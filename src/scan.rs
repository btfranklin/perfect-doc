use crate::{
    assets, collection,
    config::{self, Config},
    headings, html_reader, includes, links, markdown_reader, metadata,
    model::*,
    online,
};
use globset::{Glob, GlobSet, GlobSetBuilder};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    io::Read,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};
use unicode_normalization::UnicodeNormalization;

pub const RULES: &[RuleDefinition] = &[
    RuleDefinition {
        id: "discovery.complete",
        family: 1,
        requirement: Requirement::Execution,
        description: "Selected input discovery must complete within limits.",
    },
    RuleDefinition {
        id: "discovery.required",
        family: 1,
        requirement: Requirement::Project,
        description: "Required path patterns must select an input.",
    },
    RuleDefinition {
        id: "source.encoding",
        family: 2,
        requirement: Requirement::Format,
        description: "Source bytes must have valid encoding and no forbidden controls.",
    },
    RuleDefinition {
        id: "source.conflict",
        family: 2,
        requirement: Requirement::Project,
        description: "Source must not contain unresolved merge conflict markers outside literals.",
    },
    RuleDefinition {
        id: "source.hidden",
        family: 2,
        requirement: Requirement::Project,
        description: "Selected names and reference IDs must not contain hidden controls.",
    },
    RuleDefinition {
        id: "source.changed",
        family: 1,
        requirement: Requirement::Execution,
        description: "Source files must not change while a scan reads them.",
    },
    RuleDefinition {
        id: "name.valid",
        family: 3,
        requirement: Requirement::Project,
        description: "File names must meet the declared portability and pattern rules.",
    },
    RuleDefinition {
        id: "name.collision",
        family: 3,
        requirement: Requirement::Project,
        description: "Case and Unicode path identities must not collide.",
    },
    RuleDefinition {
        id: "config.exception",
        family: 23,
        requirement: Requirement::Project,
        description: "Exceptions must be balanced, current, and refer to supported rules.",
    },
];

#[derive(Debug)]
pub struct ScanError(pub String);
impl fmt::Display for ScanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for ScanError {}
impl From<String> for ScanError {
    fn from(message: String) -> Self {
        Self(message)
    }
}

pub fn validate(roots: &[PathBuf], config: &Config) -> Result<Report, ScanError> {
    validate_with_cancel(roots, config, &AtomicBool::new(false))
}

pub fn validate_with_cancel(
    roots: &[PathBuf],
    config: &Config,
    cancelled: &AtomicBool,
) -> Result<Report, ScanError> {
    let mut canonical_config = config.clone();
    canonical_config.base_dir = std::fs::canonicalize(&config.base_dir)
        .map_err(|e| ScanError(format!("Cannot open configuration directory: {e}")))?;
    let config = &canonical_config;
    config.validate()?;
    metadata::preflight(config)?;
    if roots.is_empty() {
        return Err(ScanError("At least one input root is required".into()));
    }
    let roots: Vec<_> = roots
        .iter()
        .map(|p| {
            std::fs::canonicalize(p)
                .map_err(|e| ScanError(format!("Cannot open input root {}: {e}", p.display())))
        })
        .collect::<Result<_, _>>()?;
    let root = if let Some(path) = &config.links.root {
        std::fs::canonicalize(config.resolve(path))
            .map_err(|e| ScanError(format!("Cannot open link root: {e}")))?
    } else {
        common_root(&roots)
    };
    let include = globs(&config.include)?;
    let exclude = globs(&config.exclude)?;
    let mut report = Report {
        diagnostics: Vec::new(),
        coverage: Coverage::default(),
        fail_on: config.rules.fail_on,
        network_enabled: config.network.enabled,
        network_checked_at: None,
    };
    let mut inventory = BTreeSet::new();
    let mut selected = BTreeSet::new();
    let at_root = |message: &str| {
        Diagnostic::at(
            "discovery.complete",
            Requirement::Execution,
            Outcome::Unverified,
            Location {
                path: display(&root, &root),
                line: 1,
                column: 1,
                byte_offset: 0,
            },
            message.to_owned(),
        )
    };
    'roots: for input in &roots {
        if !input.starts_with(&root) && !config.links.allow_outside_root {
            return Err(ScanError(
                "Input roots must be inside the declared link root".into(),
            ));
        }
        if input.is_file() {
            if !inventory.contains(input) && inventory.len() >= config.files.max_files {
                report.diagnostics.push(at_root("Input file limit reached"));
                break;
            }
            let display = display(input, &root);
            if exclude.is_match(&display) || !include.is_match(&display) {
                report.coverage.excluded += 1;
                continue;
            }
            inventory.insert(input.clone());
            selected.insert(input.clone());
            continue;
        }
        let walker = walkdir::WalkDir::new(input)
            .follow_links(config.files.follow_symlinks)
            .max_depth(config.files.max_depth)
            .into_iter()
            .filter_entry(|entry| {
                let relative = display(entry.path(), &root);
                !exclude.is_match(&relative)
                    && !(entry.file_type().is_dir()
                        && exclude.is_match(format!("{relative}/__entry__")))
            });
        for entry in walker {
            if cancelled.load(Ordering::Relaxed) {
                report.diagnostics.push(at_root("Scan was cancelled"));
                break 'roots;
            }
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    report
                        .diagnostics
                        .push(at_root(&format!("Input discovery failed: {e}")));
                    continue;
                }
            };
            if entry.file_type().is_dir() {
                if entry.depth() == config.files.max_depth
                    && std::fs::read_dir(entry.path()).is_ok_and(|mut d| d.next().is_some())
                {
                    report.diagnostics.push(at_root(&format!(
                        "Directory depth limit reached at {}",
                        display(entry.path(), &root)
                    )));
                }
                continue;
            }
            let relative = display(entry.path(), &root);
            if entry.file_type().is_symlink() && !config.files.follow_symlinks {
                if config.format_for(entry.path()).is_some() && include.is_match(&relative) {
                    let mut d = at_root("A selected symbolic link was excluded by the link policy");
                    d.location.path = relative;
                    report.diagnostics.push(d);
                }
                continue;
            }
            if !entry.file_type().is_file() {
                continue;
            }
            if inventory.len() >= config.files.max_files {
                report.diagnostics.push(at_root("Input file limit reached"));
                break 'roots;
            }
            let canonical = match std::fs::canonicalize(entry.path()) {
                Ok(p) => p,
                Err(_) => {
                    report
                        .diagnostics
                        .push(at_root("Cannot resolve an input path"));
                    continue;
                }
            };
            if !canonical.starts_with(&root) && !config.links.allow_outside_root {
                report
                    .diagnostics
                    .push(at_root("A followed symbolic link leaves the declared root"));
                continue;
            }
            inventory.insert(canonical.clone());
            if config.format_for(entry.path()).is_some() && include.is_match(&relative) {
                selected.insert(canonical);
            } else {
                report.coverage.supporting_files += 1;
            }
        }
    }
    for pattern in &config.required_paths {
        let matcher = Glob::new(pattern)
            .expect("validated glob")
            .compile_matcher();
        if !selected.iter().any(|p| matcher.is_match(display(p, &root))) {
            report.diagnostics.push(Diagnostic::at(
                "discovery.required",
                Requirement::Project,
                Outcome::Invalid,
                Location {
                    path: pattern.clone(),
                    line: 1,
                    column: 1,
                    byte_offset: 0,
                },
                "Required pattern did not select a document",
            ));
        }
    }
    if selected.is_empty() && !config.files.allow_empty {
        report
            .diagnostics
            .push(at_root("No documents were selected"));
    }
    check_names(&selected, &root, config, &mut report.diagnostics);
    let mut documents = Vec::new();
    report.coverage.documents = selected.len();
    let mut total_bytes = 0u64;
    for path in &selected {
        if cancelled.load(Ordering::Relaxed) {
            report
                .diagnostics
                .push(at_root("Scan was cancelled before parsing completed"));
            break;
        }
        let Some(format) = config.format_for(path) else {
            let mut d = at_root("Selected source format is not supported");
            d.outcome = Outcome::Unsupported;
            d.location.path = display(path, &root);
            report.diagnostics.push(d);
            continue;
        };
        let profile = config.profile_for(&display(path, &root));
        let Some(mut source) = read_source(path, &root, format, &profile, &mut report.diagnostics)
        else {
            continue;
        };
        total_bytes = total_bytes.saturating_add(source.text.len() as u64);
        if total_bytes > config.files.max_total_bytes {
            report
                .diagnostics
                .push(at_root("Source collection byte limit reached"));
            break;
        }
        let metadata = if format == Format::Markdown {
            metadata::read_frontmatter(&mut source, &profile, &mut report.diagnostics)
        } else {
            None
        };
        let mut parsed = match format {
            Format::Markdown => markdown_reader::parse(&source, &profile),
            Format::Html => html_reader::parse(&source, &profile),
        };
        if format == Format::Markdown
            && profile.markdown.dialect == crate::config::MarkdownDialect::Mdx
        {
            html_reader::validate_elements(&source, &profile, &mut parsed);
        }
        if format == Format::Markdown && !parsed.html_ranges.is_empty() {
            let mut html = html_reader::parse_fragment(&source, &parsed.html_ranges, &profile);
            let base = parsed.elements.len();
            for element in &mut html.elements {
                element.parent = element.parent.map(|parent| parent + base);
            }
            parsed.headings.extend(html.headings);
            parsed.content_ranges.extend(html.content_ranges);
            parsed.anchors.extend(html.anchors);
            parsed.references.extend(html.references);
            parsed.code_blocks.extend(html.code_blocks);
            parsed.elements.extend(html.elements);
            parsed.diagnostics.extend(html.diagnostics);
            parsed.dynamic_anchors |= html.dynamic_anchors;
            parsed.incomplete |= html.incomplete;
            parsed.headings.sort_by_key(|h| h.offset);
        }
        let mut document = Document {
            source,
            format,
            parsed,
            metadata,
            route: None,
        };
        metadata::check_frontmatter(&mut document, &profile)?;
        documents.push(document);
    }
    let routes = collection::index_routes(&mut documents, &root, config, &mut report.diagnostics);
    let include_edges =
        collection::check_includes(&documents, &root, config, &mut report.diagnostics);
    report
        .diagnostics
        .extend(includes::compose(&mut documents, &root, config));
    for document in &mut documents {
        let profile = config.profile_for(&document.source.display_path);
        headings::check(document, &profile);
        metadata::check_examples(document, &profile)?;
        check_source(document, &profile, &mut report.diagnostics);
        if !document.parsed.incomplete {
            report.coverage.parsed += 1;
        }
        report.coverage.references += document.parsed.references.len();
    }
    let local = links::check(&documents, &root, config, &routes);
    report.coverage.local_verified = local.verified;
    report.diagnostics.extend(local.diagnostics);
    collection::check(
        &documents,
        &root,
        config,
        &routes,
        &local.edges,
        &include_edges,
        &mut report.diagnostics,
    );
    let mut external = local.external;
    external.extend(assets::check(
        &documents,
        &local.assets,
        &assets::AssetContext {
            root: &root,
            config,
            inventory: &inventory,
            routes: &routes,
            incoming: &local.referenced_paths,
        },
        &mut report.diagnostics,
    ));
    check_structured(&inventory, &root, config, &mut report.diagnostics)?;
    report.coverage.external_urls = external
        .iter()
        .map(|l| &l.url)
        .collect::<BTreeSet<_>>()
        .len();
    let mut network = config.network.clone();
    if let Some(path) = &network.cache {
        network.cache = Some(config.resolve(path).to_string_lossy().into_owned());
    }
    let result = online::check(&external, &network, cancelled);
    report.coverage.external_verified = result.verified;
    report.network_checked_at = result.checked_at;
    report.diagnostics.extend(result.diagnostics);
    for document in &documents {
        report
            .diagnostics
            .extend(document.parsed.diagnostics.clone());
    }
    apply_policy(&documents, config, &mut report)?;
    for diagnostic in &mut report.diagnostics {
        if diagnostic.rule == "discovery.complete"
            && matches!(
                diagnostic.outcome,
                Outcome::Unverified | Outcome::Unsupported
            )
        {
            diagnostic.suppression = None;
        }
    }
    if report.coverage.parsed < report.coverage.documents {
        report.diagnostics.push(at_root(
            "Selected documents could not all be parsed; completion cannot be suppressed",
        ));
    }
    report.coverage.include_patterns = config.include.clone();
    report.coverage.exclude_patterns = config.exclude.clone();
    report.sort();
    Ok(report)
}

fn globs(patterns: &[String]) -> Result<GlobSet, String> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        builder.add(Glob::new(pattern).map_err(|e| e.to_string())?);
    }
    builder.build().map_err(|e| e.to_string())
}
fn common_root(paths: &[PathBuf]) -> PathBuf {
    let mut root = if paths[0].is_dir() {
        paths[0].clone()
    } else {
        paths[0].parent().unwrap_or(Path::new("/")).to_path_buf()
    };
    for path in &paths[1..] {
        let target = if path.is_dir() {
            path.as_path()
        } else {
            path.parent().unwrap_or(Path::new("/"))
        };
        while !target.starts_with(&root) {
            if !root.pop() {
                break;
            }
        }
    }
    root
}
fn display(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn read_source(
    path: &Path,
    root: &Path,
    format: Format,
    config: &Config,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<Source> {
    let location = Location {
        path: display(path, root),
        line: 1,
        column: 1,
        byte_offset: 0,
    };
    let at = |rule, outcome, message| {
        Diagnostic::at(
            rule,
            Requirement::Execution,
            outcome,
            location.clone(),
            message,
        )
    };
    let before = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(_) => {
            diagnostics.push(at(
                "discovery.complete",
                Outcome::Unverified,
                "Cannot read source metadata".to_owned(),
            ));
            return None;
        }
    };
    if before.len() > config.files.max_file_bytes {
        diagnostics.push(at(
            "discovery.complete",
            Outcome::Unverified,
            "Source exceeds the file size limit".into(),
        ));
        return None;
    }
    let mut bytes = Vec::new();
    if std::fs::File::open(path)
        .and_then(|file| {
            file.take(config.files.max_file_bytes + 1)
                .read_to_end(&mut bytes)
        })
        .is_err()
    {
        diagnostics.push(at(
            "discovery.complete",
            Outcome::Unverified,
            "Cannot read source".into(),
        ));
        return None;
    }
    if bytes.len() as u64 > config.files.max_file_bytes {
        diagnostics.push(at(
            "discovery.complete",
            Outcome::Unverified,
            "Source grew beyond the file size limit".into(),
        ));
        return None;
    }
    if std::fs::metadata(path).map_or(true, |after| {
        after.len() != before.len() || after.modified().ok() != before.modified().ok()
    }) {
        diagnostics.push(at(
            "source.changed",
            Outcome::Unverified,
            "Source changed while it was read".into(),
        ));
    }
    if format == Format::Html
        && encoding_rs::Encoding::for_label(config.files.html_encoding.as_bytes())
            != Some(encoding_rs::UTF_8)
    {
        diagnostics.push(at("source.encoding",Outcome::Unsupported,"This build supports UTF-8 source mapping; the selected HTML encoding needs another source map".into()));
        return None;
    }
    let text = match String::from_utf8(bytes) {
        Ok(text) => text,
        Err(error) => {
            let mut d = at(
                "source.encoding",
                Outcome::Invalid,
                "Source is not valid UTF-8".into(),
            );
            d.requirement = Requirement::Format;
            d.location.byte_offset = error.utf8_error().valid_up_to();
            diagnostics.push(d);
            return None;
        }
    };
    let mut source = Source::new(path.to_path_buf(), display(path, root), text);
    if source.text.starts_with('\u{feff}') {
        source.body_offset = 3;
    }
    for (offset, c) in source.text.char_indices() {
        if c == '\0' || (c.is_control() && !matches!(c, '\n' | '\r' | '\t' | '\u{c}')) {
            diagnostics.push(source.diagnostic(
                "source.encoding",
                Requirement::Format,
                Outcome::Invalid,
                offset,
                "Source contains a forbidden control character",
            ));
        }
    }
    Some(source)
}

fn check_names(
    paths: &BTreeSet<PathBuf>,
    root: &Path,
    config: &Config,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut identities: BTreeMap<String, String> = BTreeMap::new();
    let name_pattern = config
        .files
        .name_pattern
        .as_ref()
        .map(|p| regex::Regex::new(p).expect("validated pattern"));
    let path_pattern = config
        .files
        .path_pattern
        .as_ref()
        .map(|p| regex::Regex::new(p).expect("validated pattern"));
    for path in paths {
        let relative = display(path, root);
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let mut problems = Vec::new();
        if name_pattern.as_ref().is_some_and(|p| !p.is_match(&name)) {
            problems.push("File name does not match its pattern");
        }
        if path_pattern
            .as_ref()
            .is_some_and(|p| !p.is_match(&relative))
        {
            problems.push("File path does not match its pattern");
        }
        if config
            .files
            .max_path_bytes
            .is_some_and(|limit| relative.len() > limit)
        {
            problems.push("File path exceeds its byte limit");
        }
        if config.files.portable_names {
            for part in Path::new(&relative).components() {
                let part = part.as_os_str().to_string_lossy();
                let stem = part.split('.').next().unwrap_or("").to_ascii_uppercase();
                if part.ends_with(['.', ' '])
                    || part.contains(['<', '>', ':', '"', '\\', '|', '?', '*'])
                    || part.chars().any(|c| c.is_control())
                    || matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
                    || (stem.len() == 4
                        && (stem.starts_with("COM") || stem.starts_with("LPT"))
                        && stem.as_bytes()[3].is_ascii_digit()
                        && stem.as_bytes()[3] != b'0')
                {
                    problems.push("Path has a nonportable file name");
                    break;
                }
            }
        }
        for message in problems {
            diagnostics.push(Diagnostic::at(
                "name.valid",
                Requirement::Project,
                Outcome::Invalid,
                Location {
                    path: relative.clone(),
                    line: 1,
                    column: 1,
                    byte_offset: 0,
                },
                message,
            ));
        }
        if config.files.hidden_characters && relative.chars().any(hidden) {
            diagnostics.push(Diagnostic::at(
                "source.hidden",
                Requirement::Project,
                Outcome::Invalid,
                Location {
                    path: relative.clone(),
                    line: 1,
                    column: 1,
                    byte_offset: 0,
                },
                "Path contains a hidden control character",
            ));
        }
        let identity = if config.files.detect_unicode_collisions {
            relative.nfc().collect::<String>()
        } else {
            relative.clone()
        };
        let identity = if config.files.detect_case_collisions {
            identity.to_lowercase()
        } else {
            identity
        };
        if let Some(previous) = identities.insert(identity, relative.clone())
            && previous != relative
        {
            diagnostics.push(Diagnostic::at(
                "name.collision",
                Requirement::Project,
                Outcome::Invalid,
                Location {
                    path: relative,
                    line: 1,
                    column: 1,
                    byte_offset: 0,
                },
                format!("Path identity collides with {previous}"),
            ));
        }
    }
}
fn hidden(c: char) -> bool {
    matches!(c,'\u{200b}'|'\u{200c}'|'\u{200d}'|'\u{2060}'|'\u{feff}'|'\u{202a}'..='\u{202e}'|'\u{2066}'..='\u{2069}')
}

fn check_source(document: &Document, config: &Config, diagnostics: &mut Vec<Diagnostic>) {
    if config.files.merge_conflicts {
        let mut offset = 0;
        for line in document.source.text.split_inclusive('\n') {
            if !document
                .parsed
                .opaque_ranges
                .iter()
                .any(|range| range.contains(&offset))
                && (line.starts_with("<<<<<<< ") || line.starts_with(">>>>>>> "))
            {
                diagnostics.push(document.source.diagnostic(
                    "source.conflict",
                    Requirement::Project,
                    Outcome::Invalid,
                    offset,
                    "Unresolved merge conflict marker",
                ));
            }
            offset += line.len();
        }
    }
    if config.files.hidden_characters {
        for anchor in &document.parsed.anchors {
            if anchor.id.chars().any(hidden) {
                diagnostics.push(document.source.diagnostic(
                    "source.hidden",
                    Requirement::Project,
                    Outcome::Invalid,
                    anchor.offset,
                    "Anchor contains a hidden control character",
                ));
            }
        }
        for reference in &document.parsed.references {
            if reference.target.chars().any(hidden) {
                diagnostics.push(document.source.diagnostic(
                    "source.hidden",
                    Requirement::Project,
                    Outcome::Invalid,
                    reference.offset,
                    "Reference contains a hidden control character",
                ));
            }
        }
    }
}

fn check_structured(
    inventory: &BTreeSet<PathBuf>,
    root: &Path,
    config: &Config,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<(), String> {
    for selected in &config.structured {
        let matcher = Glob::new(&selected.glob)
            .expect("validated glob")
            .compile_matcher();
        let mut matched = false;
        for path in inventory
            .iter()
            .filter(|p| matcher.is_match(display(p, root)))
        {
            matched = true;
            let source = match collection::read_supporting_source(path, config) {
                Ok(s) => s,
                Err(e) => {
                    diagnostics.push(Diagnostic::at(
                        "structured.syntax",
                        Requirement::Execution,
                        Outcome::Unverified,
                        Location {
                            path: display(path, root),
                            line: 1,
                            column: 1,
                            byte_offset: 0,
                        },
                        e,
                    ));
                    continue;
                }
            };
            let format = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            match metadata::parse_data(&source.text, format) {
                Ok(value) => {
                    if let Some(schema) = &selected.schema {
                        if format.eq_ignore_ascii_case("xml") {
                            diagnostics.push(metadata::unsupported_xml_schema(&source, 0, schema));
                            continue;
                        }
                        let (schema, schema_path) = metadata::load_schema(schema, config)?;
                        metadata::check_schema(
                            &schema,
                            &value,
                            &source,
                            0,
                            config,
                            Some(&schema_path),
                            diagnostics,
                        )?;
                    }
                }
                Err(error) => diagnostics.push(source.diagnostic(
                    "structured.syntax",
                    Requirement::Format,
                    if matches!(format, "json" | "yaml" | "yml" | "toml" | "xml") {
                        Outcome::Invalid
                    } else {
                        Outcome::Unsupported
                    },
                    0,
                    error,
                )),
            }
        }
        if !matched {
            diagnostics.push(Diagnostic::at(
                "discovery.required",
                Requirement::Project,
                Outcome::Invalid,
                Location {
                    path: selected.glob.clone(),
                    line: 1,
                    column: 1,
                    byte_offset: 0,
                },
                "Structured file pattern matched no files",
            ));
        }
    }
    Ok(())
}

fn rule_enabled(rule: &str, config: &Config) -> bool {
    let md = &config.markdown;
    let fm = &config.frontmatter;
    match rule {
        "source.conflict" => config.files.merge_conflicts,
        "source.hidden" => config.files.hidden_characters,
        "name.valid" => {
            config.files.portable_names
                || config.files.name_pattern.is_some()
                || config.files.path_pattern.is_some()
                || config.files.max_path_bytes.is_some()
        }
        "name.collision" => {
            config.files.detect_case_collisions || config.files.detect_unicode_collisions
        }
        "discovery.required" => !config.required_paths.is_empty() || !config.structured.is_empty(),
        "schema.valid" | "schema.available" => {
            fm.schema.is_some()
                || fm.inline_schema.is_some()
                || !md.example_schemas.is_empty()
                || config.structured.iter().any(|file| file.schema.is_some())
        }
        "example.syntax" => md.structured_examples || !md.example_schemas.is_empty(),
        "markdown.heading-shape" => {
            md.heading_start.is_some()
                || md.single_h1
                || md.no_heading_skips
                || md.nonempty_headings
        }
        "markdown.empty-section" => md.nonempty_sections,
        "markdown.fence-closed" => md.closed_fences,
        "markdown.table-columns" => md.table_columns,
        "markdown.undefined-reference" => md.undefined_references,
        "markdown.unused-definition" => md.unused_definitions,
        "markdown.citation" => !md.bibliography.is_empty(),
        "markdown.toc" => md.toc,
        "include.expansion" | "include.cycle" => md.includes,
        "metadata.unique" => fm.id_field.is_some() || !fm.unique_fields.is_empty(),
        "route.unique" | "route.target" => config.routes.enabled,
        "anchor.preserved" => !config.routes.preserved_anchors.is_empty(),
        "build.output" => config.routes.require_output,
        "navigation.structure" => !config.collection.navigation.is_empty(),
        "collection.reachable" => config.collection.require_reachable,
        "html.syntax" | "html.structure" | "html.attributes" | "html.document" | "html.aria" => {
            config.html.conformance
        }
        "html.accessibility" => config.html.accessibility,
        "html.language" => config.html.require_lang || config.html.conformance,
        "html.landmarks" => config.html.require_main,
        "asset.integrity" => config.assets.integrity,
        "asset.dimensions" => config.assets.image_dimensions,
        "asset.orphan" => config.collection.orphan_assets,
        "sitemap.structure" => !config.assets.sitemap.is_empty(),
        "external.response" | "external.policy" => config.network.enabled,
        "external.fragment" => config.network.enabled && config.network.check_fragments,
        "external.cache" => config.network.enabled && config.network.cache.is_some(),
        _ => true,
    }
}

fn apply_policy(
    documents: &[Document],
    config: &Config,
    report: &mut Report,
) -> Result<(), String> {
    let known: BTreeSet<_> = crate::rules().iter().map(|r| r.id.to_string()).collect();
    report.coverage.available_rules = known.iter().cloned().collect();
    report.coverage.disabled_rules = config.rules.disable.clone();
    let mut enabled: BTreeSet<_> = known
        .iter()
        .filter(|id| rule_enabled(id, config) && !config.rules.disable.contains(id))
        .cloned()
        .collect();
    for document in documents {
        let profile = config.profile_for(&document.source.display_path);
        enabled.extend(
            known
                .iter()
                .filter(|id| rule_enabled(id, &profile) && !config.rules.disable.contains(id))
                .cloned(),
        );
    }
    enabled.extend(
        report
            .diagnostics
            .iter()
            .filter(|d| !config.rules.disable.contains(&d.rule))
            .map(|d| d.rule.clone()),
    );
    report.coverage.enabled_rules = enabled.into_iter().collect();
    for diagnostic in &mut report.diagnostics {
        if let Some(severity) = config.rules.severity.get(&diagnostic.rule) {
            diagnostic.severity = *severity;
        }
        if config.rules.disable.contains(&diagnostic.rule) {
            diagnostic.suppression = Some("Disabled by configuration".into());
        }
    }
    let today = time::OffsetDateTime::now_utc().date();
    for exception in &config.exceptions {
        let matcher = Glob::new(&exception.path)
            .expect("validated glob")
            .compile_matcher();
        if exception
            .expires
            .as_ref()
            .is_some_and(|value| config::parse_date(value).is_ok_and(|date| date < today))
        {
            report.diagnostics.push(Diagnostic::at(
                "config.exception",
                Requirement::Project,
                Outcome::Invalid,
                Location {
                    path: "perfect-doc.toml".into(),
                    line: 1,
                    column: 1,
                    byte_offset: 0,
                },
                format!(
                    "Exception has expired for {} at {}",
                    exception.rule, exception.path
                ),
            ));
            continue;
        }
        let mut used = false;
        for diagnostic in &mut report.diagnostics {
            if diagnostic.rule == exception.rule && matcher.is_match(&diagnostic.location.path) {
                diagnostic.suppression = Some(exception.reason.clone());
                used = true;
            }
        }
        if !used && config.rules.unused_exceptions {
            report.diagnostics.push(Diagnostic::at(
                "config.exception",
                Requirement::Project,
                Outcome::Invalid,
                Location {
                    path: "perfect-doc.toml".into(),
                    line: 1,
                    column: 1,
                    byte_offset: 0,
                },
                format!(
                    "Exception did not suppress a finding: {} at {}",
                    exception.rule, exception.path
                ),
            ));
        }
    }
    if config.rules.inline_suppressions {
        let marker=regex::Regex::new(r"<!--\s*perfect-doc-(disable|enable|ignore)\s+([a-zA-Z0-9., -]+?)(?:\s*:\s*(.*?))?\s*-->").expect("constant pattern");
        for document in documents {
            let mut active: BTreeMap<String, (usize, String)> = BTreeMap::new();
            for found in marker.captures_iter(&document.source.text) {
                let whole = found.get(0).expect("whole match");
                if document
                    .parsed
                    .opaque_ranges
                    .iter()
                    .any(|range| range.contains(&whole.start()))
                    && !document
                        .parsed
                        .html_ranges
                        .iter()
                        .any(|range| range.contains(&whole.start()))
                {
                    continue;
                }
                if document.parsed.elements.iter().any(|e| {
                    matches!(e.tag.as_str(), "script" | "style" | "pre" | "code")
                        && (e.offset..e.end).contains(&whole.start())
                }) {
                    continue;
                }
                let action = &found[1];
                let ids: Vec<_> = found[2]
                    .split([',', ' '])
                    .filter(|v| !v.is_empty())
                    .collect();
                let reason = found.get(3).map_or("", |v| v.as_str()).trim();
                for id in ids {
                    let mut error = None;
                    if !known.contains(id) {
                        error = Some(format!("Unknown suppression rule: {id}"));
                    } else if action == "disable" {
                        if reason.is_empty() {
                            error = Some("A suppression needs a reason after ':'".into());
                        } else if active
                            .insert(id.into(), (whole.end(), reason.into()))
                            .is_some()
                        {
                            error = Some(format!("Suppression is already active: {id}"));
                        }
                    } else if action == "enable" {
                        if let Some((start, reason)) = active.remove(id) {
                            for d in &mut report.diagnostics {
                                if d.location.path == document.source.display_path
                                    && d.rule == id
                                    && (start..whole.start()).contains(&d.location.byte_offset)
                                {
                                    d.suppression = Some(reason.clone());
                                }
                            }
                        } else {
                            error = Some(format!("Suppression has no matching disable: {id}"));
                        }
                    } else if reason.is_empty() {
                        error = Some("A suppression needs a reason after ':'".into());
                    } else {
                        let next_line = document.source.location(whole.end()).line + 1;
                        for d in &mut report.diagnostics {
                            if d.location.path == document.source.display_path
                                && d.rule == id
                                && d.location.line == next_line
                            {
                                d.suppression = Some(reason.into());
                            }
                        }
                    }
                    if let Some(error) = error {
                        report.diagnostics.push(document.source.diagnostic(
                            "config.exception",
                            Requirement::Project,
                            Outcome::Invalid,
                            whole.start(),
                            error,
                        ));
                    }
                }
            }
            for (id, (offset, _)) in active {
                report.diagnostics.push(document.source.diagnostic(
                    "config.exception",
                    Requirement::Project,
                    Outcome::Invalid,
                    offset,
                    format!("Suppression has no closing enable: {id}"),
                ));
            }
        }
    }
    Ok(())
}
