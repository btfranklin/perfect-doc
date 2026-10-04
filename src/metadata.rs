use crate::{config::Config, model::*};
use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Value};
use std::{
    fmt,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

pub const RULES: &[RuleDefinition] = &[
    RuleDefinition {
        id: "frontmatter.syntax",
        family: 4,
        requirement: Requirement::Format,
        description: "Front matter must have valid syntax and unique keys.",
    },
    RuleDefinition {
        id: "frontmatter.shape",
        family: 4,
        requirement: Requirement::Profile,
        description: "Front matter must be an object in the selected format.",
    },
    RuleDefinition {
        id: "frontmatter.contract",
        family: 5,
        requirement: Requirement::Project,
        description: "Metadata must meet the configured field contract.",
    },
    RuleDefinition {
        id: "schema.valid",
        family: 5,
        requirement: Requirement::Project,
        description: "Data must match its local JSON Schema.",
    },
    RuleDefinition {
        id: "schema.available",
        family: 20,
        requirement: Requirement::Execution,
        description: "Required schema references must be available within limits.",
    },
    RuleDefinition {
        id: "structured.syntax",
        family: 20,
        requirement: Requirement::Format,
        description: "Selected structured data must parse without duplicate keys.",
    },
    RuleDefinition {
        id: "example.syntax",
        family: 15,
        requirement: Requirement::Project,
        description: "Selected code examples must have valid data syntax.",
    },
];

struct StrictValue(Value);
impl<'de> Deserialize<'de> for StrictValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ValueVisitor;
        impl<'de> Visitor<'de> for ValueVisitor {
            type Value = StrictValue;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a JSON value with unique object keys")
            }
            fn visit_bool<E: de::Error>(self, v: bool) -> Result<Self::Value, E> {
                Ok(StrictValue(Value::Bool(v)))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
                Ok(StrictValue(v.into()))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
                Ok(StrictValue(v.into()))
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> Result<Self::Value, E> {
                serde_json::Number::from_f64(v)
                    .map(|n| StrictValue(Value::Number(n)))
                    .ok_or_else(|| E::custom("Non-finite number"))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                Ok(StrictValue(v.into()))
            }
            fn visit_string<E: de::Error>(self, v: String) -> Result<Self::Value, E> {
                Ok(StrictValue(v.into()))
            }
            fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(StrictValue(Value::Null))
            }
            fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(StrictValue(Value::Null))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut access: A) -> Result<Self::Value, A::Error> {
                let mut values = Vec::new();
                while let Some(value) = access.next_element::<StrictValue>()? {
                    values.push(value.0);
                }
                Ok(StrictValue(Value::Array(values)))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut access: A) -> Result<Self::Value, A::Error> {
                let mut values = Map::new();
                while let Some(key) = access.next_key::<String>()? {
                    if values.contains_key(&key) {
                        return Err(de::Error::custom(format!("Duplicate object key: {key}")));
                    }
                    values.insert(key, access.next_value::<StrictValue>()?.0);
                }
                Ok(StrictValue(Value::Object(values)))
            }
        }
        deserializer.deserialize_any(ValueVisitor)
    }
}

pub fn parse_data(text: &str, format: &str) -> Result<Value, String> {
    match format.to_ascii_lowercase().as_str() {
        "json" | "jsonld" | "json-ld" => serde_json::from_str::<StrictValue>(text)
            .map(|v| v.0)
            .map_err(|e| e.to_string()),
        "yaml" | "yml" => serde_saphyr::from_str_with_options::<Value>(
            text,
            serde_saphyr::options! { strict_booleans: true },
        )
        .map_err(|e| e.to_string()),
        "toml" => toml::from_str::<toml::Value>(text)
            .map_err(|e| e.to_string())
            .and_then(|v| serde_json::to_value(v).map_err(|e| e.to_string())),
        "xml" => roxmltree::Document::parse(text)
            .map(|_| Value::Null)
            .map_err(|e| e.to_string()),
        _ => Err(format!("Unsupported structured format: {format}")),
    }
}

pub fn read_frontmatter(
    source: &mut Source,
    config: &Config,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<Value> {
    if config.frontmatter.format == "none" {
        return None;
    }
    let start = source.body_offset;
    let body = source.body();
    let first = body.lines().next().unwrap_or("").trim_end();
    let detected = match first {
        "---" => Some(("---", "yaml")),
        "+++" => Some(("+++", "toml")),
        ";;;" => Some((";;;", "json")),
        _ => None,
    };
    let Some((delimiter, format)) = detected else {
        if config.frontmatter.required {
            diagnostics.push(source.diagnostic(
                "frontmatter.contract",
                Requirement::Project,
                Outcome::Invalid,
                start,
                "Required front matter is missing",
            ));
        }
        return None;
    };
    let mut offset = first.len();
    if body.as_bytes().get(offset) == Some(&b'\r') {
        offset += 1;
    }
    if body.as_bytes().get(offset) == Some(&b'\n') {
        offset += 1;
    }
    let content_start = offset;
    let mut close = None;
    for line in body[offset..].split_inclusive('\n') {
        if line.trim_end_matches(['\r', '\n']) == delimiter
            || (format == "yaml" && line.trim_end_matches(['\r', '\n']) == "...")
        {
            close = Some((offset, offset + line.len()));
            break;
        }
        offset += line.len();
    }
    let Some((end, after)) = close else {
        // An opening thematic break alone is valid Markdown in automatic mode.
        if config.frontmatter.format == "auto"
            && !config.frontmatter.required
            && config.frontmatter.schema.is_none()
            && config.frontmatter.inline_schema.is_none()
            && delimiter == "---"
        {
            return None;
        }
        diagnostics.push(source.diagnostic(
            "frontmatter.syntax",
            Requirement::Format,
            Outcome::Invalid,
            start,
            "Front matter has no closing delimiter",
        ));
        source.body_offset = source.text.len();
        return None;
    };
    if config.frontmatter.format != "auto" && config.frontmatter.format != format {
        diagnostics.push(source.diagnostic(
            "frontmatter.shape",
            Requirement::Profile,
            Outcome::Invalid,
            start,
            format!(
                "Expected {} front matter, found {format}",
                config.frontmatter.format
            ),
        ));
    }
    let value = parse_data(&body[content_start..end], format);
    source.body_offset = start + after;
    match value {
        Ok(value) if value.is_object() => Some(value),
        Ok(_) => {
            diagnostics.push(source.diagnostic(
                "frontmatter.shape",
                Requirement::Profile,
                Outcome::Invalid,
                start,
                "Front matter must contain an object",
            ));
            None
        }
        Err(error) => {
            diagnostics.push(source.diagnostic(
                "frontmatter.syntax",
                Requirement::Format,
                Outcome::Invalid,
                start + content_start,
                error,
            ));
            None
        }
    }
}

/// A field is either a top-level key or an RFC 6901 JSON pointer.
pub fn field<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    if key.starts_with('/') {
        value.pointer(key)
    } else {
        value.get(key)
    }
}

pub fn check_frontmatter(document: &mut Document, config: &Config) -> Result<(), String> {
    let Some(value) = &document.metadata else {
        return Ok(());
    };
    let policy = &config.frontmatter;
    let mut messages = Vec::new();
    for key in &policy.required_fields {
        if field(value, key).is_none() {
            messages.push(format!("Required metadata field is missing: {key}"));
        }
    }
    for key in &policy.forbidden_fields {
        if field(value, key).is_some() {
            messages.push(format!("Metadata field is prohibited: {key}"));
        }
    }
    if !policy.allowed_fields.is_empty() {
        for key in value.as_object().expect("metadata object").keys() {
            if !policy.allowed_fields.contains(key) {
                messages.push(format!("Metadata field is not allowed: {key}"));
            }
        }
    }
    for key in [
        &policy.id_field,
        &policy.route_field,
        &policy.parent_field,
        &policy.language_field,
        &policy.name_field,
    ]
    .into_iter()
    .flatten()
    {
        if field(value, key).is_some_and(|v| v.as_str().is_none_or(|s| s.trim().is_empty())) {
            messages.push(format!(
                "Metadata field must contain a nonempty string: {key}"
            ));
        }
    }
    if let Some(key) = &policy.language_field
        && let Some(lang) = field(value, key).and_then(Value::as_str)
        && language_tags::LanguageTag::parse(lang).is_err()
    {
        messages.push(format!("Invalid language tag in metadata field: {key}"));
    }
    if let Some(key) = &policy.name_field
        && let Some(name) = field(value, key).and_then(Value::as_str)
        && document
            .source
            .path
            .file_stem()
            .is_none_or(|stem| stem != name)
    {
        messages.push(format!("Metadata field {key} must match the file stem"));
    }
    for order in &policy.date_order {
        let parse = |key: &str| {
            field(value, key).map(|v| {
                let s = v
                    .as_str()
                    .ok_or_else(|| format!("Date field must be a string: {key}"))?;
                if let Ok(date) = crate::config::parse_date(s) {
                    return Ok(date.with_time(time::Time::MIDNIGHT).assume_utc());
                }
                time::OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339)
                    .map_err(|_| format!("Invalid ISO date or timestamp: {key}"))
            })
        };
        match (parse(&order.before), parse(&order.after)) {
            (Some(Ok(a)), Some(Ok(b))) if a > b => messages.push(format!(
                "{} must not be after {}",
                order.before, order.after
            )),
            (Some(Err(e)), _) | (_, Some(Err(e))) => messages.push(e),
            _ => {}
        }
    }
    for key in &policy.link_fields {
        if let Some(data) = field(value, key) {
            let items = data
                .as_array()
                .map(|v| v.as_slice())
                .unwrap_or_else(|| std::slice::from_ref(data));
            for item in items {
                if let Some(target) = item.as_str() {
                    document
                        .parsed
                        .references
                        .push(Reference::new(target, 0, ReferenceKind::Link));
                } else {
                    messages.push(format!("Reference field must contain strings: {key}"));
                }
            }
        }
    }
    for message in messages {
        document.parsed.diagnostics.push(document.source.diagnostic(
            "frontmatter.contract",
            Requirement::Project,
            Outcome::Invalid,
            0,
            message,
        ));
    }
    if let Some(schema) = policy.inline_schema.clone() {
        check_schema(
            &schema,
            value,
            &document.source,
            0,
            config,
            None,
            &mut document.parsed.diagnostics,
        )?;
    }
    if let Some(path) = &policy.schema {
        let (schema, path) = load_schema(path, config)?;
        check_schema(
            &schema,
            value,
            &document.source,
            0,
            config,
            Some(&path),
            &mut document.parsed.diagnostics,
        )?;
    }
    Ok(())
}

struct LocalRetriever {
    root: PathBuf,
    max_bytes: u64,
    requests: Arc<AtomicUsize>,
}
impl jsonschema::Retrieve for LocalRetriever {
    fn retrieve(
        &self,
        uri: &jsonschema::Uri<String>,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        if self.requests.fetch_add(1, Ordering::Relaxed) >= 128 {
            return Err("Schema reference limit reached".into());
        }
        let url = url::Url::parse(uri.as_str())?;
        if url.scheme() != "file" {
            return Err("Remote schema references are unavailable in a local scan".into());
        }
        let path = url.to_file_path().map_err(|_| "Invalid local schema URI")?;
        let path = std::fs::canonicalize(path)?;
        if !path.starts_with(&self.root) {
            return Err("Schema reference leaves the configuration directory".into());
        }
        let mut file = std::fs::File::open(path)?;
        let mut text = String::new();
        std::io::Read::read_to_string(
            &mut std::io::Read::take(&mut file, self.max_bytes + 1),
            &mut text,
        )?;
        if text.len() as u64 > self.max_bytes {
            return Err("Schema file limit reached".into());
        }
        Ok(parse_data(&text, "json")?)
    }
}

pub fn load_schema(path: &str, config: &Config) -> Result<(Value, PathBuf), String> {
    let path = config.resolve(path);
    let text = crate::collection::read_supporting_source(&path, config)?.text;
    Ok((
        parse_data(&text, "json").map_err(|e| format!("Invalid schema syntax: {e}"))?,
        path,
    ))
}

pub fn check_schema(
    schema: &Value,
    value: &Value,
    source: &Source,
    offset: usize,
    config: &Config,
    path: Option<&Path>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<(), String> {
    jsonschema::meta::validate(schema)
        .map_err(|e| format!("Invalid schema at {}", e.instance_path()))?;
    let base = path
        .map(Path::to_path_buf)
        .unwrap_or_else(|| config.base_dir.join("inline-schema.json"));
    let base =
        url::Url::from_file_path(&base).map_err(|_| "Schema base must be an absolute path")?;
    let retriever = LocalRetriever {
        root: std::fs::canonicalize(&config.base_dir).map_err(|e| e.to_string())?,
        max_bytes: config.files.max_file_bytes,
        requests: Arc::new(AtomicUsize::new(0)),
    };
    let validator = jsonschema::options()
        .should_validate_formats(true)
        .with_base_uri(base.as_str())
        .with_retriever(retriever)
        .build(schema);
    match validator {
        Ok(validator) => {
            for error in validator.iter_errors(value).take(1000) {
                diagnostics.push(
                    source
                        .diagnostic(
                            "schema.valid",
                            Requirement::Project,
                            Outcome::Invalid,
                            offset,
                            format!(
                                "Data does not meet the schema at {} ({}): {:?}",
                                error.instance_path(),
                                error.schema_path(),
                                error.kind()
                            ),
                        )
                        .with_target(&format!(
                            "{}#{}",
                            path.map_or("inline schema".into(), |p| p.display().to_string()),
                            error.schema_path()
                        )),
                );
            }
        }
        Err(_) => diagnostics.push(source.diagnostic(
            "schema.available",
            Requirement::Execution,
            Outcome::Unverified,
            offset,
            "The schema could not be resolved or compiled within the local scan limits",
        )),
    }
    Ok(())
}

pub fn unsupported_xml_schema(source: &Source, offset: usize, path: &str) -> Diagnostic {
    let mut diagnostic = source
        .diagnostic(
            "schema.available",
            Requirement::Execution,
            Outcome::Unsupported,
            offset,
            "XML syntax was checked, but JSON Schema cannot validate an XML document in this build",
        )
        .with_target(path);
    diagnostic.help = "Use JSON, YAML, or TOML for a JSON Schema contract. Use an XML schema validator for the XML contract.".into();
    diagnostic
}

pub fn check_examples(document: &mut Document, config: &Config) -> Result<(), String> {
    for code in &document.parsed.code_blocks {
        let Some(language) = code.language.as_deref() else {
            continue;
        };
        let embedded = code.info == "embedded";
        if !embedded
            && !config.markdown.structured_examples
            && !config.markdown.example_schemas.contains_key(language)
        {
            continue;
        }
        if !matches!(
            language,
            "json" | "jsonld" | "json-ld" | "yaml" | "yml" | "toml" | "xml"
        ) {
            continue;
        }
        // The explicit flag is for examples that demonstrate invalid input.
        if code.info.split_whitespace().any(|v| v == "expect-invalid") {
            continue;
        }
        match parse_data(&code.value, language) {
            Ok(value) => {
                if let Some(path) = config.markdown.example_schemas.get(language) {
                    if language == "xml" {
                        document.parsed.diagnostics.push(unsupported_xml_schema(
                            &document.source,
                            code.content_offset,
                            path,
                        ));
                        continue;
                    }
                    let (schema, path) = load_schema(path, config)?;
                    check_schema(
                        &schema,
                        &value,
                        &document.source,
                        code.content_offset,
                        config,
                        Some(&path),
                        &mut document.parsed.diagnostics,
                    )?;
                }
            }
            Err(error) => document.parsed.diagnostics.push(document.source.diagnostic(
                if embedded {
                    "structured.syntax"
                } else {
                    "example.syntax"
                },
                if embedded {
                    Requirement::Format
                } else {
                    Requirement::Project
                },
                Outcome::Invalid,
                code.content_offset,
                error,
            )),
        }
    }
    Ok(())
}

/// Reject invalid declared schemas before a document can pass without using them.
pub fn preflight(config: &Config) -> Result<(), String> {
    let paths = config
        .frontmatter
        .schema
        .iter()
        .chain(config.markdown.example_schemas.values())
        .chain(config.structured.iter().filter_map(|s| s.schema.as_ref()));
    for path in paths {
        let (schema, _) = load_schema(path, config)?;
        jsonschema::meta::validate(&schema)
            .map_err(|e| format!("Invalid schema at {}", e.instance_path()))?;
    }
    if let Some(schema) = &config.frontmatter.inline_schema {
        jsonschema::meta::validate(schema)
            .map_err(|e| format!("Invalid schema at {}", e.instance_path()))?;
    }
    Ok(())
}
