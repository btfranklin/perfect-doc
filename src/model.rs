use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

/// Diagnostic importance. Completion is recorded separately.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Warning,
    Error,
}

/// Evidence from an individual check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    Valid,
    Invalid,
    Unverified,
    Excluded,
    Unsupported,
}

/// The source of a structural requirement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Requirement {
    Format,
    Profile,
    Project,
    Execution,
}

/// One-based Unicode scalar columns and lines; byte offsets are zero-based.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Location {
    pub path: String,
    pub line: usize,
    pub column: usize,
    pub byte_offset: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diagnostic {
    pub rule: String,
    pub severity: Severity,
    pub outcome: Outcome,
    pub requirement: Requirement,
    pub location: Location,
    pub message: String,
    /// The failed reference or contract value, with URI secrets removed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// A repair action for the reported rule.
    pub help: String,
    pub required: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related: Vec<Location>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suppression: Option<String>,
}

impl Diagnostic {
    pub fn at(
        rule: &str,
        requirement: Requirement,
        outcome: Outcome,
        location: Location,
        message: impl Into<String>,
    ) -> Self {
        Self {
            rule: rule.into(),
            severity: Severity::Error,
            outcome,
            requirement,
            location,
            message: message.into(),
            target: None,
            help: repair_hint(rule).into(),
            required: true,
            related: Vec::new(),
            suppression: None,
        }
    }
    pub fn with_target(mut self, target: &str) -> Self {
        self.target = Some(safe_target(target));
        self
    }
}

/// Remove credentials, query values, and external fragments from a reported URI.
pub fn safe_target(value: &str) -> String {
    if value.to_ascii_lowercase().starts_with("data:") {
        return format!(
            "{},[redacted]",
            value
                .split([';', ','])
                .next()
                .unwrap_or("data:")
                .chars()
                .take(128)
                .collect::<String>()
        );
    }
    if let Some(relative) = value.strip_prefix("//") {
        return safe_target(&format!("https://{relative}"))
            .trim_start_matches("https:")
            .into();
    }
    if let Ok(mut url) = url::Url::parse(value) {
        let _ = url.set_username("");
        let _ = url.set_password(None);
        if url.query().is_some() {
            url.set_query(Some("[redacted]"));
        }
        if url.fragment().is_some() {
            url.set_fragment(Some("[redacted]"));
        }
        return url.into();
    }
    if value.contains("://") {
        return "[invalid URI]".into();
    }
    if let Some((path, query)) = value.split_once('?') {
        return format!(
            "{path}?[redacted]{}",
            query.find('#').map_or("", |at| &query[at..])
        );
    }
    value.into()
}

/// Return a repair action for a supported rule.
pub fn repair_hint(rule: &str) -> &'static str {
    match rule {
        "discovery.complete" => {
            "Select readable supported documents. If a limit stopped the scan, reduce the input or increase the declared file, byte, or depth limit, then run the scan again."
        }
        "discovery.required" => {
            "Create the required file or correct its path pattern and the input selection."
        }
        "source.encoding" | "html.encoding" => {
            "Save the file as UTF-8. Remove invalid bytes and forbidden control characters; make the encoding declaration match the file."
        }
        "source.conflict" => "Resolve the merge conflict and remove its conflict markers.",
        "source.hidden" => {
            "Remove the reported hidden character from the name or identifier, then update its references."
        }
        "source.changed" => "Finish writes to the source files, then run the scan again.",
        "name.valid" => {
            "Rename the file to meet the configured name and path rules, then update its references."
        }
        "name.collision" => {
            "Give the colliding files distinct names after case folding and Unicode normalization, then update their references."
        }
        "config.exception" => {
            "Correct the exception rule, path, reason, expiry date, or comment pair. Remove exceptions that no longer apply."
        }
        "frontmatter.syntax" => {
            "Correct the metadata syntax and delimiter pair. Remove duplicate keys."
        }
        "frontmatter.shape" => "Use a metadata object with the expected field types.",
        "frontmatter.contract" => {
            "Correct the reported metadata field to meet the declared contract. Add required fields and remove prohibited fields."
        }
        "schema.valid" => {
            "Correct the reported value at its instance path to meet the schema constraint."
        }
        "schema.available" => {
            "Make every schema reference available as a readable local file within the configured root and limits. Remote schema retrieval is not enabled."
        }
        "structured.syntax" => {
            "Correct the reported JSON, YAML, TOML, or XML syntax and remove duplicate object keys."
        }
        "example.syntax" => {
            "Correct the example syntax or its declared schema. Use expect-invalid only for an intentional invalid example."
        }
        "markdown.syntax" => {
            "Correct the syntax for the selected Markdown dialect. If a parser limit stopped the scan, reduce nesting or split the document."
        }
        "markdown.heading-shape" => {
            "Correct the reported heading level, order, title, or count to meet the declared heading rules."
        }
        "markdown.empty-section" => {
            "Add section content before the next heading, or remove the empty section."
        }
        "markdown.fence-closed" => {
            "Add a matching closing code fence with the same marker and at least the opening length."
        }
        "markdown.table-columns" => {
            "Make each table row match the header column count. Select the GFM dialect for this check."
        }
        "markdown.definition-conflict" => {
            "Keep one reference or footnote definition for each normalized label."
        }
        "markdown.undefined-reference" => {
            "Add the missing reference definition or correct the reference label."
        }
        "markdown.unused-definition" => "Use the reported definition or remove it.",
        "markdown.anchor" | "anchor.unique" => {
            "Give each anchor a valid unique identifier, then update its references."
        }
        "markdown.extension" => {
            "Correct the declared extension syntax and attributes, or select the profile that supports this extension."
        }
        "markdown.mdx-dynamic" | "html.dynamic" => {
            "Scan the static HTML produced by the renderer so that runtime structure can be checked."
        }
        "markdown.code-info" => {
            "Add or correct the code language and supported fence attributes; check the configured language allow list."
        }
        "markdown.citation" => {
            "Correct the citation key or add its entry to a readable local bibliography."
        }
        "markdown.toc" => {
            "Update the table of contents to match the document heading anchors and order."
        }
        "html.syntax" => {
            "Correct the reported HTML token or parse error. If a parser limit stopped the scan, reduce nesting or split the document."
        }
        "html.structure" => {
            "Move or replace the reported element so that its parent permits that element and its required children are present."
        }
        "html.attributes" => {
            "Correct the reported attribute name, value, or required attribute; remove duplicate attributes."
        }
        "html.document" => {
            "Add or correct the required doctype, html, head, title, and body structure, or select fragment mode for a fragment."
        }
        "html.references" => {
            "Correct the ID reference and give its target the required element type."
        }
        "html.aria" => "Correct the reported ARIA role, attribute, value, or ID reference.",
        "html.accessibility" => {
            "Add the required text alternative or label, and correct the reported structural association."
        }
        "html.language" => "Set a valid language tag on the required element.",
        "html.landmarks" => {
            "Add the required main landmark and remove conflicting landmark declarations."
        }
        "html.srcset" => {
            "Correct each srcset URL and descriptor. Use one descriptor type and unique positive descriptor values."
        }
        "uri.syntax" => {
            "Correct the URI syntax and percent escapes. Use an allowed scheme and a valid local path or destination."
        }
        "link.exists" => {
            "Create the missing target or correct the reference path. For a directory link, provide a configured index file."
        }
        "link.case" => "Make the reference use the exact case of each target path component.",
        "link.boundary" => {
            "Move the target inside the declared root or correct the reference to an allowed target."
        }
        "link.anchor" => {
            "Correct the fragment to an existing target identifier, or add the missing identifier to the target document."
        }
        "link.type" => {
            "Reference a target with the required element or asset type, or correct the target declaration."
        }
        "link.available" => {
            "Select and read the target document. Resolve template values or scan rendered output to verify the reference."
        }
        "metadata.unique" => {
            "Give the reported metadata field a unique value and update its references."
        }
        "route.unique" => "Give each page, alias, and redirect a unique publication route.",
        "route.target" => {
            "Correct the route or redirect target. Use valid absolute route paths and remove redirect cycles."
        }
        "navigation.structure" => {
            "Correct the navigation tree, duplicate entry, target, or required coverage reported in the message."
        }
        "collection.reachable" => {
            "Add a valid link from a declared entrypoint to the document, or correct the entrypoint contract."
        }
        "include.target" => {
            "Correct the local include path and select an existing ordered region or valid line range."
        }
        "include.cycle" => {
            "Remove the include cycle. Reduce include depth or adjust the declared depth limit."
        }
        "include.expansion" => {
            "Correct the static include target or slice. If a limit stopped expansion, reduce include depth or size, or increase the declared limit."
        }
        "anchor.preserved" => {
            "Restore the required public anchor or correct the preserved anchor contract."
        }
        "build.output" => {
            "Build the documentation, then correct the missing output, output reference, or source-to-output route contract."
        }
        "asset.syntax" => {
            "Replace or repair the malformed asset with a valid file in its declared format."
        }
        "asset.type" => {
            "Correct the asset extension, declared media type, or reference element so that they match the file format."
        }
        "asset.integrity" => {
            "Calculate the supported integrity hash from the current asset bytes and update the integrity attribute."
        }
        "asset.dimensions" => {
            "Correct the declared image dimensions or replace the image to meet the configured limits."
        }
        "asset.available" => {
            "Make the asset readable within the configured size limits and use a supported format."
        }
        "asset.orphan" => {
            "Add a reference to the asset, remove it, or exclude it from the declared orphan asset scope."
        }
        "sitemap.structure" => {
            "Correct the sitemap XML, unique absolute locations, and route targets."
        }
        "external.response" => {
            "Open the reported URL and correct a missing destination. For an unavailable response, check network access, authentication, or server status, then run the scan again."
        }
        "external.fragment" => {
            "Correct the fragment to an identifier in the destination static HTML. Use rendered output when the identifier requires scripts."
        }
        "external.policy" => {
            "Correct the network host policy or its declared limits, then run the requested check again."
        }
        "external.cache" => {
            "Remove or repair the explicitly configured evidence cache, or omit its path to make fresh requests."
        }
        _ => {
            "Correct the reported structural constraint at this source location, then run the scan again."
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct RuleDefinition {
    pub id: &'static str,
    pub family: u8,
    pub requirement: Requirement,
    pub description: &'static str,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Coverage {
    pub documents: usize,
    pub parsed: usize,
    pub excluded: usize,
    pub supporting_files: usize,
    pub references: usize,
    pub local_verified: usize,
    pub external_urls: usize,
    pub external_verified: usize,
    pub enabled_rules: Vec<String>,
    pub available_rules: Vec<String>,
    pub disabled_rules: Vec<String>,
    pub include_patterns: Vec<String>,
    pub exclude_patterns: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    pub diagnostics: Vec<Diagnostic>,
    pub coverage: Coverage,
    pub fail_on: Severity,
    pub network_enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network_checked_at: Option<String>,
}

impl Report {
    pub fn exit_code(&self) -> u8 {
        if self.diagnostics.iter().any(|d| {
            d.suppression.is_none() && d.outcome == Outcome::Invalid && d.severity >= self.fail_on
        }) {
            return 1;
        }
        if self.diagnostics.iter().any(|d| {
            d.suppression.is_none()
                && d.required
                && matches!(d.outcome, Outcome::Unverified | Outcome::Unsupported)
        }) {
            return 3;
        }
        0
    }
    pub fn is_valid(&self) -> bool {
        self.exit_code() == 0
    }
    pub fn sort(&mut self) {
        self.diagnostics.sort_by(|a, b| {
            (
                &a.location.path,
                a.location.byte_offset,
                &a.rule,
                &a.message,
            )
                .cmp(&(
                    &b.location.path,
                    b.location.byte_offset,
                    &b.rule,
                    &b.message,
                ))
        });
    }
}

#[derive(Debug, Clone)]
pub struct Source {
    pub path: PathBuf,
    pub display_path: String,
    pub text: String,
    pub body_offset: usize,
    line_starts: Vec<usize>,
    mappings: Vec<SourceMapping>,
}

#[derive(Debug, Clone)]
struct SourceMapping {
    destination: std::ops::Range<usize>,
    source: Arc<Source>,
    original: usize,
    context: Option<Location>,
}

/// A source slice used for static include expansion. The context identifies its include.
#[derive(Debug, Clone)]
pub struct SourcePart {
    pub source: Arc<Source>,
    pub range: std::ops::Range<usize>,
    pub context: Option<Location>,
}

impl Source {
    pub fn new(path: PathBuf, display_path: String, text: String) -> Self {
        let bytes = text.as_bytes();
        let mut line_starts = vec![0];
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'\r' {
                if bytes.get(i + 1) == Some(&b'\n') {
                    i += 1;
                }
                line_starts.push(i + 1);
            } else if bytes[i] == b'\n' {
                line_starts.push(i + 1);
            }
            i += 1;
        }
        Self {
            path,
            display_path,
            text,
            body_offset: 0,
            line_starts,
            mappings: Vec::new(),
        }
    }
    pub fn body(&self) -> &str {
        &self.text[self.body_offset..]
    }
    pub fn compose(
        path: PathBuf,
        display_path: String,
        parts: &[SourcePart],
    ) -> Result<Self, String> {
        let mut text = String::new();
        let mut mappings = Vec::new();
        for part in parts {
            let value = part
                .source
                .text
                .get(part.range.clone())
                .ok_or("Invalid source slice for an include")?;
            let start = text.len();
            text.push_str(value);
            mappings.push(SourceMapping {
                destination: start..text.len(),
                source: part.source.clone(),
                original: part.range.start,
                context: part.context.clone(),
            });
        }
        let mut source = Self::new(path, display_path, text);
        source.mappings = mappings;
        Ok(source)
    }
    fn mapping(&self, offset: usize) -> Option<&SourceMapping> {
        let index = self
            .mappings
            .partition_point(|m| m.destination.end <= offset);
        self.mappings
            .get(index)
            .filter(|m| m.destination.contains(&offset))
            .or_else(|| self.mappings.last().filter(|m| offset == m.destination.end))
    }
    pub fn path_at(&self, offset: usize) -> &std::path::Path {
        if let Some(mapping) = self.mapping(offset) {
            mapping
                .source
                .path_at(mapping.original + offset.saturating_sub(mapping.destination.start))
        } else {
            &self.path
        }
    }
    pub fn location(&self, offset: usize) -> Location {
        if let Some(mapping) = self.mapping(offset) {
            return mapping
                .source
                .location(mapping.original + offset.saturating_sub(mapping.destination.start));
        }
        let mut offset = offset.min(self.text.len());
        while !self.text.is_char_boundary(offset) {
            offset -= 1;
        }
        let index = self
            .line_starts
            .partition_point(|start| *start <= offset)
            .saturating_sub(1);
        Location {
            path: self.display_path.clone(),
            line: index + 1,
            column: self.text[self.line_starts[index]..offset].chars().count() + 1,
            byte_offset: offset,
        }
    }
    pub fn diagnostic(
        &self,
        rule: &str,
        requirement: Requirement,
        outcome: Outcome,
        offset: usize,
        message: impl Into<String>,
    ) -> Diagnostic {
        let mut diagnostic =
            Diagnostic::at(rule, requirement, outcome, self.location(offset), message);
        if let Some(context) = self.mapping(offset).and_then(|m| m.context.clone()) {
            diagnostic.related.push(context);
        }
        diagnostic
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Markdown,
    Html,
}

#[derive(Debug, Clone)]
pub struct Heading {
    pub level: u8,
    pub text: String,
    pub id: Option<String>,
    pub offset: usize,
    pub end: usize,
}

#[derive(Debug, Clone)]
pub struct Anchor {
    pub id: String,
    pub offset: usize,
    pub tag: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferenceKind {
    Link,
    Image,
    Asset,
    Include,
    Id,
    Schema,
    Navigation,
    Canonical,
    Alternate,
    Redirect,
}

#[derive(Debug, Clone)]
pub struct Reference {
    pub target: String,
    pub offset: usize,
    pub kind: ReferenceKind,
    pub expected_tag: Option<String>,
    pub integrity: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
}
impl Reference {
    pub fn new(target: impl Into<String>, offset: usize, kind: ReferenceKind) -> Self {
        Self {
            target: target.into(),
            offset,
            kind,
            expected_tag: None,
            integrity: None,
            width: None,
            height: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Include {
    pub target: String,
    pub offset: usize,
    pub region: Option<String>,
    pub start_line: Option<usize>,
    pub end_line: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct CodeBlock {
    pub language: Option<String>,
    pub info: String,
    pub value: String,
    pub offset: usize,
    pub content_offset: usize,
}

#[derive(Debug, Clone)]
pub struct HtmlElement {
    pub tag: String,
    pub attrs: BTreeMap<String, String>,
    pub attr_offsets: BTreeMap<String, usize>,
    pub offset: usize,
    pub end: usize,
    pub parent: Option<usize>,
    pub text: String,
}

#[derive(Debug, Clone, Default)]
pub struct ParsedDocument {
    pub incomplete: bool,
    pub headings: Vec<Heading>,
    pub anchors: Vec<Anchor>,
    pub references: Vec<Reference>,
    pub includes: Vec<Include>,
    pub code_blocks: Vec<CodeBlock>,
    pub elements: Vec<HtmlElement>,
    pub diagnostics: Vec<Diagnostic>,
    pub opaque_ranges: Vec<std::ops::Range<usize>>,
    pub html_ranges: Vec<std::ops::Range<usize>>,
    /// Static content spans, excluding headings and container markup.
    pub content_ranges: Vec<std::ops::Range<usize>>,
    pub dynamic_anchors: bool,
}

#[derive(Debug, Clone)]
pub struct Document {
    pub source: Source,
    pub format: Format,
    pub parsed: ParsedDocument,
    pub metadata: Option<serde_json::Value>,
    pub route: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ExternalLink {
    pub url: String,
    pub location: Location,
}

#[derive(Debug, Clone, Default)]
pub struct NetworkResult {
    pub diagnostics: Vec<Diagnostic>,
    pub verified: usize,
    pub checked_at: Option<String>,
}
