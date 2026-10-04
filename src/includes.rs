//! Expand static Markdown includes without code execution.

use crate::{
    collection,
    config::{Config, MarkdownDialect},
    html_reader,
    links::{self, Target},
    markdown_reader, metadata,
    model::*,
};
use std::{
    collections::BTreeMap,
    ops::Range,
    path::{Path, PathBuf},
    sync::Arc,
};

pub const RULES: &[RuleDefinition] = &[RuleDefinition {
    id: "include.expansion",
    family: 14,
    requirement: Requirement::Execution,
    description: "Static includes must be complete within the configured source limits.",
}];

#[derive(Clone)]
struct Input {
    source: Arc<Source>,
    parsed: ParsedDocument,
}

struct Expansion {
    parts: Vec<SourcePart>,
    bytes: u64,
    limit: u64,
    failed: bool,
}
impl Expansion {
    fn append(
        &mut self,
        source: Arc<Source>,
        range: Range<usize>,
        context: Option<Location>,
    ) -> bool {
        let length = range.end.saturating_sub(range.start) as u64;
        if self.bytes.saturating_add(length) > self.limit {
            return false;
        }
        self.bytes += length;
        if !range.is_empty() {
            self.parts.push(SourcePart {
                source,
                range,
                context,
            });
        }
        true
    }
}

struct Composer<'a> {
    root: &'a Path,
    config: &'a Config,
    inputs: BTreeMap<PathBuf, Input>,
    read_bytes: u64,
    diagnostics: Vec<Diagnostic>,
}

/// Expand each host, then parse the complete host with its selected profile.
/// Original include records are kept for checks against the original inputs.
pub fn compose(documents: &mut [Document], root: &Path, config: &Config) -> Vec<Diagnostic> {
    let mut composer = Composer {
        root,
        config,
        inputs: BTreeMap::new(),
        read_bytes: 0,
        diagnostics: Vec::new(),
    };
    for document in documents.iter() {
        composer.read_bytes = composer
            .read_bytes
            .saturating_add(document.source.text.len() as u64);
        composer.inputs.insert(
            document.source.path.clone(),
            Input {
                source: Arc::new(document.source.clone()),
                parsed: document.parsed.clone(),
            },
        );
    }
    let mut total = composer.read_bytes;
    for document in documents.iter_mut() {
        if document.format != Format::Markdown || !config.markdown.includes {
            continue;
        }
        let original = Input {
            source: Arc::new(document.source.clone()),
            parsed: document.parsed.clone(),
        };
        if !original
            .parsed
            .includes
            .iter()
            .any(|include| token_end(&original.source, include).is_some())
        {
            continue;
        }
        let profile = config.profile_for(&document.source.display_path);
        let available = config
            .files
            .max_total_bytes
            .saturating_sub(total.saturating_sub(original.source.text.len() as u64));
        let mut expansion = Expansion {
            parts: Vec::new(),
            bytes: 0,
            limit: config.files.max_file_bytes.min(available),
            failed: false,
        };
        let mut stack = vec![original.source.path.clone()];
        let complete = composer.expand(
            &original,
            0..original.source.text.len(),
            &profile,
            &mut stack,
            None,
            &mut expansion,
        );
        if !complete {
            document.parsed.dynamic_anchors = true;
            document.parsed.incomplete = true;
            continue;
        }
        let composed = Source::compose(
            original.source.path.clone(),
            original.source.display_path.clone(),
            &expansion.parts,
        );
        let mut source = match composed {
            Ok(source) => source,
            Err(message) => {
                composer.issue(&original.source, 0, None, Outcome::Unverified, message);
                document.parsed.dynamic_anchors = true;
                document.parsed.incomplete = true;
                continue;
            }
        };
        source.body_offset = original.source.body_offset;
        let mut parsed = markdown_reader::parse(&source, &profile);
        if profile.markdown.dialect == MarkdownDialect::Mdx {
            html_reader::validate_elements(&source, &profile, &mut parsed);
        }
        if !parsed.html_ranges.is_empty() {
            let mut html = html_reader::parse_fragment(&source, &parsed.html_ranges, &profile);
            let base = parsed.elements.len();
            for element in &mut html.elements {
                element.parent = element.parent.map(|parent| parent + base);
            }
            parsed.content_ranges.extend(html.content_ranges);
            parsed.headings.extend(html.headings);
            parsed.anchors.extend(html.anchors);
            parsed.references.extend(html.references);
            parsed.code_blocks.extend(html.code_blocks);
            parsed.elements.extend(html.elements);
            parsed.diagnostics.extend(html.diagnostics);
            parsed.dynamic_anchors |= html.dynamic_anchors;
            parsed.incomplete |= html.incomplete;
            parsed.headings.sort_by_key(|heading| heading.offset);
        }
        for (offset, value) in source.text.char_indices() {
            if (value == '\0'
                || (value.is_control() && !matches!(value, '\n' | '\r' | '\t' | '\u{c}')))
                && source.path_at(offset) != original.source.path
            {
                parsed.diagnostics.push(source.diagnostic(
                    "source.encoding",
                    Requirement::Format,
                    Outcome::Invalid,
                    offset,
                    "Included source contains a forbidden control character",
                ));
            }
        }

        parsed.references.extend(
            original
                .parsed
                .references
                .iter()
                .filter(|reference| reference.offset < original.source.body_offset)
                .cloned(),
        );
        parsed.diagnostics.extend(
            original
                .parsed
                .diagnostics
                .iter()
                .filter(|diagnostic| {
                    diagnostic.rule.starts_with("frontmatter.")
                        || diagnostic.rule.starts_with("schema.")
                })
                .cloned(),
        );
        parsed.includes = original.parsed.includes;
        parsed.dynamic_anchors |= expansion.failed;
        parsed.incomplete |= expansion.failed;
        total = total
            .saturating_sub(document.source.text.len() as u64)
            .saturating_add(source.text.len() as u64);
        document.source = source;
        document.parsed = parsed;
    }
    composer.diagnostics
}

impl Composer<'_> {
    fn issue(
        &mut self,
        source: &Source,
        offset: usize,
        context: Option<&Location>,
        outcome: Outcome,
        message: impl Into<String>,
    ) {
        let mut diagnostic = source.diagnostic(
            "include.expansion",
            Requirement::Execution,
            outcome,
            offset,
            message,
        );
        if let Some(include) = self.inputs.get(&source.path).and_then(|input| {
            input
                .parsed
                .includes
                .iter()
                .filter(|include| include.offset <= offset)
                .max_by_key(|include| include.offset)
        }) {
            diagnostic = diagnostic.with_target(&include.target);
        }
        if let Some(context) = context
            && !diagnostic.related.contains(context)
        {
            diagnostic.related.push(context.clone());
        }
        self.diagnostics.push(diagnostic);
    }

    fn load(&mut self, path: &Path, profile: &Config) -> Result<Input, String> {
        if let Some(input) = self.inputs.get(path) {
            let mut input = input.clone();
            if self.config.format_for(path).is_none() {
                input.parsed = markdown_reader::parse(&input.source, profile);
            }
            return Ok(input);
        }
        if self.inputs.len() >= self.config.files.max_files {
            return Err("Include source file limit reached".into());
        }
        let mut source = collection::read_supporting_source(path, self.config)?;
        if self.read_bytes.saturating_add(source.text.len() as u64)
            > self.config.files.max_total_bytes
        {
            return Err("Include source collection byte limit reached".into());
        }
        self.read_bytes += source.text.len() as u64;
        source.display_path = path
            .strip_prefix(self.root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        if source.text.starts_with('\u{feff}') {
            source.body_offset = '\u{feff}'.len_utf8();
        }
        let mut input_config = if self.config.format_for(path).is_some() {
            self.config.profile_for(&source.display_path)
        } else {
            profile.clone()
        };
        if self.config.format_for(path) == Some(Format::Markdown) {
            // Included metadata is not part of the host metadata contract.
            input_config.frontmatter.required = false;
            let mut diagnostics = Vec::new();
            metadata::read_frontmatter(&mut source, &input_config, &mut diagnostics);
            self.diagnostics.extend(diagnostics);
        }
        let parsed = markdown_reader::parse(&source, &input_config);
        let input = Input {
            source: Arc::new(source),
            parsed,
        };
        self.inputs.insert(path.to_path_buf(), input.clone());
        Ok(input)
    }

    fn target(
        &self,
        include: &Include,
        source: &Source,
        profile: &Config,
    ) -> Result<PathBuf, String> {
        if include.target.contains(['?', '#']) {
            return Err(
                "An include target must be a physical file without a query or fragment".into(),
            );
        }
        let mut config = profile.clone();
        config.routes.enabled = false;
        let target = links::resolve(
            &include.target,
            &source.path,
            self.root,
            &config,
            &BTreeMap::new(),
        )?;
        let Target::Local {
            path,
            fragment: None,
        } = target
        else {
            return Err("An include target must be a local file".into());
        };
        let canonical = std::fs::canonicalize(&path)
            .map_err(|error| format!("Include target is not available: {error}"))?;
        if !canonical.starts_with(self.root) {
            return Err("Include target escapes the input root".into());
        }
        if !canonical.is_file() {
            return Err("Include target is not a regular file".into());
        }
        Ok(canonical)
    }

    fn expand(
        &mut self,
        input: &Input,
        range: Range<usize>,
        profile: &Config,
        stack: &mut Vec<PathBuf>,
        context: Option<Location>,
        output: &mut Expansion,
    ) -> bool {
        let mut cursor = range.start;
        let mut includes: Vec<_> = input
            .parsed
            .includes
            .iter()
            .filter(|include| range.contains(&include.offset))
            .collect();
        includes.sort_by_key(|include| include.offset);
        for include in includes {
            let literal = input
                .parsed
                .code_blocks
                .iter()
                .any(|code| code.offset == include.offset);
            let end = if literal {
                include.offset
            } else {
                let Some(end) = token_end(&input.source, include) else {
                    continue;
                };
                if end > range.end || include.offset < cursor {
                    continue;
                }
                end
            };
            if !literal
                && !output.append(
                    input.source.clone(),
                    cursor..include.offset,
                    context.clone(),
                )
            {
                self.issue(
                    &input.source,
                    include.offset,
                    context.as_ref(),
                    Outcome::Unverified,
                    "Composed document byte limit reached",
                );
                return false;
            }
            let at = input.source.location(include.offset);
            let malformed = input.parsed.diagnostics.iter().any(|diagnostic| {
                diagnostic.rule == "markdown.extension"
                    && diagnostic.location.byte_offset == include.offset
                    && matches!(diagnostic.outcome, Outcome::Invalid | Outcome::Unsupported)
            });
            let target = if malformed {
                Err("Include attributes are invalid or unsupported".into())
            } else {
                self.target(include, &input.source, profile)
            };
            let expanded = match target {
                Err(message) => {
                    self.issue(
                        &input.source,
                        include.offset,
                        context.as_ref(),
                        Outcome::Invalid,
                        message,
                    );
                    false
                }
                Ok(path) => {
                    if !literal && stack.contains(&path) {
                        self.issue(
                            &input.source,
                            include.offset,
                            context.as_ref(),
                            Outcome::Invalid,
                            "Include cycle detected",
                        );
                        false
                    } else if !literal
                        && stack.len() > self.config.collection.include_max_depth.min(128)
                    {
                        self.issue(
                            &input.source,
                            include.offset,
                            context.as_ref(),
                            Outcome::Unverified,
                            "Include nesting limit reached",
                        );
                        false
                    } else {
                        match self.load(&path, profile) {
                            Err(message) => {
                                let outcome = if message == "Supporting file is not UTF-8" {
                                    Outcome::Invalid
                                } else {
                                    Outcome::Unverified
                                };
                                self.issue(
                                    &input.source,
                                    include.offset,
                                    context.as_ref(),
                                    outcome,
                                    message,
                                );
                                false
                            }
                            Ok(child) => match selected_range(&child.source, include) {
                                Err(message) => {
                                    self.issue(
                                        &input.source,
                                        include.offset,
                                        context.as_ref(),
                                        Outcome::Invalid,
                                        message,
                                    );
                                    false
                                }
                                Ok(_) if literal => true,
                                Ok(selected) => {
                                    stack.push(path);
                                    let complete = self.expand(
                                        &child,
                                        selected,
                                        profile,
                                        stack,
                                        Some(at),
                                        output,
                                    );
                                    stack.pop();
                                    if !complete {
                                        return false;
                                    }
                                    true
                                }
                            },
                        }
                    }
                }
            };
            if !expanded {
                output.failed = true;
                if !literal
                    && !output.append(input.source.clone(), include.offset..end, context.clone())
                {
                    self.issue(
                        &input.source,
                        include.offset,
                        context.as_ref(),
                        Outcome::Unverified,
                        "Composed document byte limit reached",
                    );
                    return false;
                }
            }
            if !literal {
                cursor = end;
            }
        }
        if !output.append(input.source.clone(), cursor..range.end, context.clone()) {
            self.issue(
                &input.source,
                cursor,
                context.as_ref(),
                Outcome::Unverified,
                "Composed document byte limit reached",
            );
            return false;
        }
        true
    }
}

fn token_end(source: &Source, include: &Include) -> Option<usize> {
    let rest = source.text.get(include.offset..)?;
    let rest = rest.strip_prefix("!include[")?;
    let end = rest.find(']')?;
    let target = rest.get(..end)?;
    if target.contains(['\r', '\n']) || target.trim() != include.target {
        return None;
    }
    let mut length = "!include[".len() + end + 1;
    let after = source.text.get(include.offset + length..)?;
    if let Some(attrs) = after.strip_prefix('{')
        && let Some(close) = attrs.find('}')
        && !attrs[..close].contains(['\r', '\n'])
    {
        length += close + 2;
    }
    Some(include.offset + length)
}

fn selected_range(source: &Source, include: &Include) -> Result<Range<usize>, String> {
    if include.region.is_some() && (include.start_line.is_some() || include.end_line.is_some()) {
        return Err("Select an include region or line range, not both".into());
    }
    if let Some(region) = &include.region {
        let start = format!("<!-- region {region} -->");
        let end = format!("<!-- endregion {region} -->");
        let starts: Vec<_> = source
            .text
            .match_indices(&start)
            .map(|(offset, _)| offset + start.len())
            .collect();
        let ends: Vec<_> = source
            .text
            .match_indices(&end)
            .map(|(offset, _)| offset)
            .collect();
        if starts.len() != 1 || ends.len() != 1 || starts[0] > ends[0] {
            return Err("Include region needs one ordered marker pair".into());
        }
        return Ok(starts[0]..ends[0]);
    }
    if include.start_line.is_some() || include.end_line.is_some() {
        let mut lines = Vec::new();
        let bytes = source.text.as_bytes();
        let mut start = 0;
        let mut cursor = 0;
        while cursor < bytes.len() {
            if matches!(bytes[cursor], b'\r' | b'\n') {
                if bytes[cursor] == b'\r' && bytes.get(cursor + 1) == Some(&b'\n') {
                    cursor += 1;
                }
                lines.push(start..cursor + 1);
                start = cursor + 1;
            }
            cursor += 1;
        }
        if start < bytes.len() {
            lines.push(start..bytes.len());
        }
        let start = include.start_line.unwrap_or(1);
        let end = include.end_line.unwrap_or(lines.len());
        if start == 0 || start > end || end > lines.len() {
            return Err("Include line range is outside the target file".into());
        }
        return Ok(lines[start - 1].start..lines[end - 1].end);
    }
    Ok(source.body_offset..source.text.len())
}
