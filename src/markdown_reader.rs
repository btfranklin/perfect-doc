use crate::config::{AnchorStyle, Config, MarkdownDialect};
use crate::model::{
    Anchor, CodeBlock, Heading, HtmlElement, Include, Outcome, ParsedDocument, Reference,
    ReferenceKind, Requirement, RuleDefinition, Source,
};
use markdown::mdast::{AttributeContent, AttributeValue, Node};
use markdown::{MdxExpressionKind, MdxSignal, ParseOptions};
use oxc_allocator::Allocator;
use oxc_parser::Parser;
use oxc_span::SourceType;
use regex::Regex;
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use unicode_normalization::UnicodeNormalization;

static HEADING_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s*\{#([^{}]*)\}\s*$").unwrap());
static ATTRIBUTES: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r##"\s*(?:#([^\s{}]+)|([A-Za-z][A-Za-z0-9_-]*)=(?:"([^"]*)"|'([^']*)'|([^\s{}]+)))"##,
    )
    .unwrap()
});
static PYTHON_SEPARATOR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[-\s]+").unwrap());

macro_rules! rule {
    ($id:literal, $family:literal, $requirement:ident, $text:literal) => {
        RuleDefinition {
            id: $id,
            family: $family,
            requirement: Requirement::$requirement,
            description: $text,
        }
    };
}

pub const RULES: &[RuleDefinition] = &[
    rule!(
        "markdown.syntax",
        7,
        Format,
        "Parse the selected Markdown dialect."
    ),
    rule!(
        "markdown.heading-shape",
        6,
        Project,
        "Check the declared heading structure."
    ),
    rule!(
        "markdown.empty-section",
        6,
        Project,
        "Check for sections without content."
    ),
    rule!(
        "markdown.fence-closed",
        7,
        Project,
        "Require a closing code fence."
    ),
    rule!(
        "markdown.table-columns",
        7,
        Project,
        "Require equal table column counts."
    ),
    rule!(
        "markdown.definition-conflict",
        8,
        Project,
        "Check conflicting reference definitions."
    ),
    rule!(
        "markdown.undefined-reference",
        8,
        Project,
        "Check explicit undefined references."
    ),
    rule!(
        "markdown.unused-definition",
        8,
        Project,
        "Check unused definitions."
    ),
    rule!("markdown.anchor", 9, Profile, "Check explicit identifiers."),
    rule!(
        "markdown.extension",
        14,
        Project,
        "Check selected extensions."
    ),
    rule!(
        "markdown.mdx-dynamic",
        14,
        Execution,
        "Report structure that requires MDX execution."
    ),
    rule!(
        "markdown.code-info",
        15,
        Project,
        "Check code language and supported attributes."
    ),
    rule!(
        "markdown.citation",
        8,
        Project,
        "Check citations against local bibliography keys."
    ),
    rule!(
        "markdown.toc",
        13,
        Project,
        "Check table of contents order and coverage."
    ),
];

pub fn parse(source: &Source, config: &Config) -> ParsedDocument {
    let mut options = match config.markdown.dialect {
        MarkdownDialect::Commonmark => ParseOptions::default(),
        MarkdownDialect::Gfm => ParseOptions::gfm(),
        MarkdownDialect::Mdx => ParseOptions::mdx(),
    };
    if config.markdown.dialect == MarkdownDialect::Mdx {
        options.mdx_expression_parse = Some(Box::new(parse_expression));
        options.mdx_esm_parse = Some(Box::new(parse_esm));
    }
    let mut reader = Reader {
        source,
        config,
        doc: ParsedDocument::default(),
        definitions: BTreeMap::new(),
        footnotes: BTreeMap::new(),
        used: BTreeSet::new(),
        footnote_uses: BTreeMap::new(),
        slugger: github_slugger::Slugger::default(),
        ids: BTreeSet::new(),
        text_ranges: Vec::new(),
        content: Vec::new(),
        quote_depth: 0,
        jsx_parent: None,
    };
    if config.markdown.table_columns && config.markdown.dialect != MarkdownDialect::Gfm {
        reader.issue("markdown.table-columns", Requirement::Project, Outcome::Unsupported, source.body_offset, "Table column checks require the GFM dialect. CommonMark and MDX do not select the GFM table extension.");
    }
    if let Some(offset) = simple_prefix_limit(source.body()) {
        reader.doc.incomplete = true;
        reader.issue(
            "markdown.syntax",
            Requirement::Execution,
            Outcome::Unverified,
            source.body_offset + offset,
            "Markdown source exceeds the reader node or nesting limit.",
        );
        return reader.doc;
    }
    match markdown::to_mdast(source.body(), &options) {
        Ok(tree) => {
            if let Some((offset, message)) = ast_limit(&tree) {
                reader.doc.incomplete = true;
                reader.issue(
                    "markdown.syntax",
                    Requirement::Execution,
                    Outcome::Unverified,
                    source.body_offset + offset,
                    message,
                );
            } else {
                reader.collect_definitions(&tree);
                reader.walk(&tree);
                reader.finish();
            }
            drop_tree(tree);
        }
        Err(error) => {
            reader.doc.incomplete = true;
            let offset = match error.place.as_deref() {
                Some(markdown::message::Place::Point(point)) => point.offset,
                Some(markdown::message::Place::Position(position)) => position.start.offset,
                None => 0,
            } + source.body_offset;
            let limited = error.rule_id.as_str() == "mdx-resource-limit";
            reader.issue(
                "markdown.syntax",
                if limited {
                    Requirement::Execution
                } else {
                    Requirement::Format
                },
                if limited {
                    Outcome::Unverified
                } else {
                    Outcome::Invalid
                },
                offset,
                error.reason,
            );
        }
    }
    reader.doc
}

fn simple_prefix_limit(text: &str) -> Option<usize> {
    // This prefix has known structural limits. Stop at other syntax.
    let mut nodes = 1;
    let mut offset = 0;
    for chunk in text.split_inclusive(['\r', '\n']) {
        let line = chunk.trim_end_matches(['\r', '\n']);
        if !line.trim().is_empty() {
            let content = line.trim_start_matches(' ');
            if line.len() - content.len() > 3 {
                return None;
            }
            let mut quote_content = content;
            let mut quotes = 0;
            while let Some(rest) = quote_content.strip_prefix('>') {
                quotes += 1;
                if quotes > 256 {
                    return Some(offset + line.len() - quote_content.len());
                }
                quote_content = rest.strip_prefix([' ', '\t']).unwrap_or(rest);
                let trimmed = quote_content.trim_start_matches(' ');
                if quote_content.len() - trimmed.len() > 3 {
                    break;
                }
                quote_content = trimmed;
            }
            let markers = content.bytes().take_while(|byte| *byte == b'#').count();
            if !(1..=6).contains(&markers)
                || !content
                    .as_bytes()
                    .get(markers)
                    .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
            {
                return None;
            }
            let heading = content[markers..].trim();
            if !heading
                .chars()
                .all(|c| c.is_alphanumeric() || matches!(c, ' ' | '\t'))
            {
                return None;
            }
            nodes += 1 + usize::from(!heading.is_empty());
            if nodes > 100_000 {
                return Some(offset);
            }
        }
        offset += chunk.len();
    }
    None
}

fn ast_limit(tree: &Node) -> Option<(usize, &'static str)> {
    let mut stack = vec![(tree, 0)];
    let mut nodes = 0;
    while let Some((node, depth)) = stack.pop() {
        nodes += 1;
        if nodes > 100_000 || depth > 256 {
            return Some((
                node.position().map_or(0, |position| position.start.offset),
                "Markdown source exceeds the reader node or nesting limit.",
            ));
        }
        if let Some(children) = node.children() {
            stack.extend(children.iter().rev().map(|child| (child, depth + 1)));
        }
    }
    None
}

fn drop_tree(mut tree: Node) {
    // Remove child vectors before drop to keep deep tree disposal off the stack.
    let mut stack = tree.children_mut().map(std::mem::take).unwrap_or_default();
    drop(tree);
    while let Some(mut node) = stack.pop() {
        if let Some(children) = node.children_mut() {
            stack.append(children);
        }
    }
}

fn javascript_limit_signal(offset: usize) -> MdxSignal {
    MdxSignal::Error(
        "MDX JavaScript exceeds the reader nesting limit.".into(),
        offset,
        Box::new("perfect-doc".into()),
        Box::new("mdx-resource-limit".into()),
    )
}

#[derive(Clone, Copy)]
enum JsMode {
    Code,
    Template,
    JsxText,
    JsxTag(bool),
}

fn javascript_limit(value: &str) -> Option<usize> {
    let bytes = value.as_bytes();
    let mut index = 0;
    let mut stack = Vec::new();
    let mut mode = JsMode::Code;
    let mut expression = true;
    let mut control = false;
    let mut jsx_returns = Vec::new();
    let mut unary = 0;
    let mut right_associative = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        match mode {
            JsMode::Template => match byte {
                b'\\' => {
                    index += 2;
                    continue;
                }
                b'`' => {
                    mode = JsMode::Code;
                    expression = false;
                }
                b'$' if bytes.get(index + 1) == Some(&b'{') => {
                    stack.push((b'}', JsMode::Template, false));
                    mode = JsMode::Code;
                    expression = true;
                    index += 1;
                }
                _ => {}
            },
            JsMode::JsxText => match byte {
                b'<' => {
                    let closing = bytes.get(index + 1) == Some(&b'/');
                    if !closing {
                        jsx_returns.push(JsMode::JsxText);
                    }
                    mode = JsMode::JsxTag(closing);
                }
                b'{' => {
                    stack.push((b'}', JsMode::JsxText, false));
                    mode = JsMode::Code;
                    expression = true;
                }
                _ => {}
            },
            JsMode::JsxTag(closing) => match byte {
                b'\'' | b'"' => {
                    index = skip_js_string(bytes, index, byte);
                    continue;
                }
                b'{' => {
                    stack.push((b'}', mode, false));
                    mode = JsMode::Code;
                    expression = true;
                }
                b'>' => {
                    mode = if closing || index > 0 && bytes[index - 1] == b'/' {
                        expression = false;
                        jsx_returns.pop().unwrap_or(JsMode::Code)
                    } else {
                        JsMode::JsxText
                    };
                }
                _ => {}
            },
            JsMode::Code => match byte {
                b'\'' | b'"' => {
                    index = skip_js_string(bytes, index, byte);
                    expression = false;
                    unary = 0;
                    continue;
                }
                b'`' => {
                    mode = JsMode::Template;
                    unary = 0;
                }
                b'/' if bytes.get(index + 1) == Some(&b'/') => {
                    index += 2;
                    while index < bytes.len() && !matches!(bytes[index], b'\r' | b'\n') {
                        index += 1;
                    }
                    continue;
                }
                b'/' if bytes.get(index + 1) == Some(&b'*') => {
                    index += 2;
                    while index + 1 < bytes.len() && &bytes[index..index + 2] != b"*/" {
                        index += 1;
                    }
                    index = (index + 2).min(bytes.len());
                    continue;
                }
                b'/' if expression => {
                    index += 1;
                    let mut class = false;
                    while index < bytes.len() {
                        match bytes[index] {
                            b'\\' => index += 1,
                            b'[' => class = true,
                            b']' => class = false,
                            b'/' if !class => {
                                index += 1;
                                break;
                            }
                            b'\r' | b'\n' => break,
                            _ => {}
                        }
                        index += 1;
                    }
                    expression = false;
                    unary = 0;
                    continue;
                }
                b'<' if expression
                    && bytes
                        .get(index + 1)
                        .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'>') =>
                {
                    jsx_returns.push(JsMode::Code);
                    mode = JsMode::JsxTag(false);
                    unary = 0;
                }
                b'(' | b'[' | b'{' => {
                    stack.push((
                        match byte {
                            b'(' => b')',
                            b'[' => b']',
                            _ => b'}',
                        },
                        JsMode::Code,
                        control && byte == b'(',
                    ));
                    control = false;
                    expression = true;
                    unary = 0;
                }
                b')' | b']' | b'}' => {
                    if stack.last().is_some_and(|(close, _, _)| *close == byte) {
                        let (_, resume, after_control) = stack.pop().unwrap();
                        mode = resume;
                        expression = after_control;
                    } else {
                        expression = false;
                    }
                    unary = 0;
                }
                byte if byte.is_ascii_alphabetic()
                    || matches!(byte, b'_' | b'$')
                    || byte >= 0x80 =>
                {
                    let start = index;
                    while index < bytes.len() {
                        let character = value[index..].chars().next().unwrap();
                        if !character.is_ascii()
                            || character.is_ascii_alphanumeric()
                            || matches!(character, '_' | '$')
                        {
                            index += character.len_utf8();
                        } else {
                            break;
                        }
                    }
                    let word = &value[start..index];
                    control = matches!(word, "if" | "while" | "for" | "with" | "switch" | "catch");
                    expression = matches!(
                        word,
                        "return"
                            | "throw"
                            | "case"
                            | "delete"
                            | "typeof"
                            | "void"
                            | "new"
                            | "yield"
                            | "await"
                            | "in"
                            | "instanceof"
                            | "else"
                            | "do"
                    );
                    if !matches!(
                        word,
                        "delete" | "typeof" | "void" | "new" | "yield" | "await"
                    ) {
                        unary = 0;
                    } else {
                        unary += 1;
                    }
                    if unary > 256 {
                        return Some(start);
                    }
                    continue;
                }
                byte if byte.is_ascii_digit() => {
                    expression = false;
                    unary = 0;
                }
                b'+' | b'-' if bytes.get(index + 1) == Some(&byte) => {
                    if expression {
                        unary += 1;
                    }
                    index += 1;
                }
                b'!' | b'~' | b'+' | b'-' if expression => {
                    unary += 1;
                }
                b'=' if bytes.get(index + 1) != Some(&b'=')
                    && (index == 0 || !matches!(bytes[index - 1], b'=' | b'!' | b'<' | b'>')) =>
                {
                    right_associative += 1;
                    expression = true;
                }
                b'?' if !bytes
                    .get(index + 1)
                    .is_some_and(|byte| matches!(byte, b'?' | b'.'))
                    && (index == 0 || bytes[index - 1] != b'?') =>
                {
                    right_associative += 1;
                    expression = true;
                }
                b'*' if bytes.get(index + 1) == Some(&b'*') => {
                    right_associative += 1;
                    expression = true;
                    index += 1;
                }
                b',' | b';' => {
                    right_associative = 0;
                    unary = 0;
                    expression = true;
                }
                b'.' => {
                    expression = false;
                    unary = 0;
                }
                byte if byte.is_ascii_whitespace() => {}
                _ => {
                    expression = true;
                    unary = 0;
                }
            },
        }
        if stack.len() + jsx_returns.len() > 256 || unary > 256 || right_associative > 256 {
            return Some(index);
        }
        index += 1;
    }
    None
}

fn skip_js_string(bytes: &[u8], start: usize, quote: u8) -> usize {
    let mut index = start + 1;
    while index < bytes.len() {
        if bytes[index] == b'\\' {
            index += 2;
        } else if bytes[index] == quote {
            return index + 1;
        } else {
            index += 1;
        }
    }
    index
}

fn parse_expression(value: &str, kind: &MdxExpressionKind) -> MdxSignal {
    if let Some(offset) = javascript_limit(value) {
        return javascript_limit_signal(offset);
    }
    let allocator = Allocator::default();
    let value = if matches!(kind, MdxExpressionKind::AttributeExpression) {
        format!("({{{value}}})")
    } else {
        value.to_owned()
    };
    if value.trim().is_empty() {
        return MdxSignal::Ok;
    }
    let parser = Parser::new(&allocator, &value, SourceType::jsx());
    if parser.parse_expression().is_ok() {
        return MdxSignal::Ok;
    }
    // MDX accepts an empty expression that contains a comment.
    let parsed = Parser::new(&allocator, &value, SourceType::jsx()).parse();
    if parsed.diagnostics.is_empty()
        && parsed.program.body.is_empty()
        && parsed.program.directives.is_empty()
    {
        return MdxSignal::Ok;
    }
    MdxSignal::Eof(
        "Invalid JavaScript expression".into(),
        Box::new("perfect-doc".into()),
        Box::new("mdx-expression".into()),
    )
}

fn parse_esm(value: &str) -> MdxSignal {
    if let Some(offset) = javascript_limit(value) {
        return javascript_limit_signal(offset);
    }
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, value, SourceType::jsx()).parse();
    if parsed.diagnostics.is_empty() {
        // MDX permits only import and export declarations at the document level.
        if parsed
            .program
            .body
            .iter()
            .all(|statement| statement.is_module_declaration())
        {
            return MdxSignal::Ok;
        }
        return MdxSignal::Error(
            "MDX permits only import and export declarations here".into(),
            0,
            Box::new("perfect-doc".into()),
            Box::new("mdx-esm".into()),
        );
    }
    MdxSignal::Eof(
        "Invalid JavaScript module declaration".into(),
        Box::new("perfect-doc".into()),
        Box::new("mdx-esm".into()),
    )
}

struct Reader<'a> {
    source: &'a Source,
    config: &'a Config,
    doc: ParsedDocument,
    definitions: BTreeMap<String, (String, Option<String>, usize)>,
    footnotes: BTreeMap<String, usize>,
    used: BTreeSet<String>,
    footnote_uses: BTreeMap<String, usize>,
    slugger: github_slugger::Slugger,
    ids: BTreeSet<String>,
    text_ranges: Vec<Range<usize>>,
    content: Vec<Range<usize>>,
    quote_depth: usize,
    jsx_parent: Option<usize>,
}

impl Reader<'_> {
    fn issue(
        &mut self,
        rule: &str,
        requirement: Requirement,
        outcome: Outcome,
        offset: usize,
        message: impl Into<String>,
    ) {
        self.doc.diagnostics.push(self.source.diagnostic(
            rule,
            requirement,
            outcome,
            offset,
            message,
        ));
    }

    fn span(&self, node: &Node) -> Range<usize> {
        node.position()
            .map_or(self.source.body_offset..self.source.body_offset, |p| {
                p.start.offset + self.source.body_offset..p.end.offset + self.source.body_offset
            })
    }

    fn node_label(&self, node: &Node, fallback: &str) -> String {
        let span = self.span(node);
        let raw = &self.source.text[span];
        let first = raw.find('[').and_then(|start| bracket(raw, start));
        if let Some((first, end)) = first {
            let selected = if matches!(node, Node::LinkReference(_) | Node::ImageReference(_)) {
                bracket(raw, end + 1).map_or(
                    first,
                    |(second, _)| if second.is_empty() { first } else { second },
                )
            } else {
                first.strip_prefix('^').unwrap_or(first)
            };
            label(selected)
        } else {
            label(fallback)
        }
    }

    fn collect_definitions(&mut self, node: &Node) {
        let offset = self.span(node).start;
        match node {
            Node::Definition(value) => {
                let id = self.node_label(node, &value.identifier);
                if let Some((url, title, original)) = self.definitions.get(&id) {
                    if url != &value.url || title != &value.title {
                        let original = *original;
                        self.issue("markdown.definition-conflict", Requirement::Project, Outcome::Invalid, offset, format!("Reference {id:?} has conflicting definitions. The first definition applies."));
                        self.doc
                            .diagnostics
                            .last_mut()
                            .unwrap()
                            .related
                            .push(self.source.location(original));
                    }
                } else {
                    self.definitions
                        .insert(id, (value.url.clone(), value.title.clone(), offset));
                }
            }
            Node::FootnoteDefinition(value) => {
                let id = self.node_label(node, &value.identifier);
                if let Some(original) = self.footnotes.get(&id).copied() {
                    self.issue(
                        "markdown.definition-conflict",
                        Requirement::Project,
                        Outcome::Invalid,
                        offset,
                        format!("Footnote {id:?} has more than one definition."),
                    );
                    self.doc
                        .diagnostics
                        .last_mut()
                        .unwrap()
                        .related
                        .push(self.source.location(original));
                } else {
                    self.footnotes.insert(id, offset);
                }
            }
            _ => {}
        }
        if let Some(children) = node.children() {
            for child in children {
                self.collect_definitions(child);
            }
        }
    }

    fn walk(&mut self, node: &Node) {
        if self
            .jsx_parent
            .is_some_and(|index| self.doc.elements[index].tag == "template")
        {
            return;
        }
        let span = self.span(node);
        let offset = span.start;
        let jsx_parent = self.jsx_parent;
        let mut parent = jsx_parent;
        let mut static_content = true;
        while let Some(index) = parent {
            let element = &self.doc.elements[index];
            if matches!(
                element.tag.as_str(),
                "head" | "script" | "style" | "template"
            ) {
                static_content = false;
                break;
            }
            parent = element.parent;
        }
        if static_content
            && match node {
                Node::Text(value) => !value.value.trim().is_empty(),
                Node::InlineCode(value) => !value.value.trim().is_empty(),
                Node::Image(_)
                | Node::ImageReference(_)
                | Node::Code(_)
                | Node::Table(_)
                | Node::ThematicBreak(_) => true,
                _ => false,
            }
        {
            self.doc.content_ranges.push(span.clone());
        }
        match node {
            Node::Heading(value) => {
                let mut text = plain(node);
                let explicit_ids = self.config.markdown.explicit_heading_ids
                    || matches!(
                        self.config.markdown.anchors,
                        AnchorStyle::Kramdown | AnchorStyle::ExplicitOnly
                    );
                let id = if let Some(capture) = HEADING_ID.captures(&text).filter(|_| explicit_ids)
                {
                    let id = capture[1].to_owned();
                    text.truncate(capture.get(0).unwrap().start());
                    if id.is_empty()
                        || id.chars().any(char::is_whitespace)
                        || id.chars().any(char::is_control)
                    {
                        self.issue("markdown.anchor", Requirement::Profile, Outcome::Invalid, offset, "An explicit heading ID must be nonempty and have no spaces or control characters.");
                    }
                    Some(id)
                } else {
                    match self.config.markdown.anchors {
                        AnchorStyle::Github => Some(self.slugger.slug(&text)),
                        AnchorStyle::PythonMarkdown => {
                            Some(self.unique_id(python_slug(&text), "_"))
                        }
                        AnchorStyle::Kramdown => Some(self.unique_id(kramdown_slug(&text), "-")),
                        AnchorStyle::ExplicitOnly => None,
                    }
                };
                if let Some(id) = &id {
                    self.doc.anchors.push(Anchor {
                        id: id.clone(),
                        offset,
                        tag: format!("h{}", value.depth),
                    });
                    self.ids.insert(id.clone());
                }
                self.doc.headings.push(Heading {
                    level: value.depth,
                    text,
                    id,
                    offset,
                    end: span.end,
                });
            }
            Node::Link(value) => {
                self.doc
                    .references
                    .push(Reference::new(&value.url, offset, ReferenceKind::Link))
            }
            Node::Image(value) => {
                self.doc
                    .references
                    .push(Reference::new(&value.url, offset, ReferenceKind::Image))
            }
            Node::LinkReference(value) => self.reference(
                &self.node_label(node, &value.identifier),
                offset,
                ReferenceKind::Link,
            ),
            Node::ImageReference(value) => self.reference(
                &self.node_label(node, &value.identifier),
                offset,
                ReferenceKind::Image,
            ),
            Node::Definition(_) => {
                self.doc.opaque_ranges.push(span);
                return;
            }
            Node::FootnoteDefinition(_) => {}
            Node::FootnoteReference(value) => {
                let id = self.node_label(node, &value.identifier);
                let count = self.footnote_uses.entry(id.clone()).or_default();
                *count += 1;
                let count = *count;
                let (target, reference) = footnote_ids(self.config.markdown.anchors, &id, count);
                self.doc.anchors.push(Anchor {
                    id: reference,
                    offset,
                    tag: "a".into(),
                });
                self.doc.references.push(Reference::new(
                    format!("#{target}"),
                    offset,
                    ReferenceKind::Link,
                ));
                self.used.insert(format!("^{id}"));
            }
            Node::Code(value) => {
                self.code(value, span.clone());
                self.content.push(span.clone());
                self.doc.opaque_ranges.push(span);
                return;
            }
            Node::InlineCode(_) => {
                self.doc.opaque_ranges.push(span);
                return;
            }
            Node::Html(_) => {
                self.doc.html_ranges.push(span.clone());
                self.doc.opaque_ranges.push(span.clone());
                self.content.push(span);
                return;
            }
            Node::Text(_) => self.text_ranges.push(span.clone()),
            Node::Paragraph(_)
            | Node::ThematicBreak(_)
            | Node::Table(_)
            | Node::List(_)
            | Node::Blockquote(_) => self.content.push(span.clone()),
            Node::MdxFlowExpression(value) => {
                if !empty_js(&value.value) {
                    self.dynamic(offset, "MDX expressions can change document structure. Supply static rendered output to check the result.");
                }
                self.doc.opaque_ranges.push(span);
                return;
            }
            Node::MdxTextExpression(value) => {
                if !empty_js(&value.value) {
                    self.dynamic(offset, "MDX expressions can change document structure. Supply static rendered output to check the result.");
                }
                self.doc.opaque_ranges.push(span);
                return;
            }
            Node::MdxjsEsm(_) => {
                self.doc.opaque_ranges.push(span);
                return;
            }
            Node::MdxJsxFlowElement(value) => {
                self.jsx_parent = self
                    .jsx(value.name.as_deref(), &value.attributes, node)
                    .or(jsx_parent);
            }
            Node::MdxJsxTextElement(value) => {
                self.jsx_parent = self
                    .jsx(value.name.as_deref(), &value.attributes, node)
                    .or(jsx_parent);
            }
            _ => {}
        }
        if let Node::Table(table) = node
            && self.config.markdown.table_columns
        {
            let width = table
                .children
                .first()
                .and_then(Node::children)
                .map_or(0, Vec::len);
            for row in &table.children {
                if row.children().map_or(0, Vec::len) != width {
                    self.issue(
                        "markdown.table-columns",
                        Requirement::Project,
                        Outcome::Invalid,
                        self.span(row).start,
                        "Table rows must have the same number of columns as the header.",
                    );
                }
            }
        }
        let quote = matches!(node, Node::Blockquote(_));
        if quote {
            self.quote_depth += 1;
        }
        if let Some(children) = node.children() {
            for child in children {
                self.walk(child);
            }
        }
        if quote {
            self.quote_depth -= 1;
        }
        self.jsx_parent = jsx_parent;
    }

    fn dynamic(&mut self, offset: usize, message: &str) {
        self.doc.dynamic_anchors = true;
        self.issue(
            "markdown.mdx-dynamic",
            Requirement::Execution,
            Outcome::Unverified,
            offset,
            message,
        );
    }

    fn jsx(
        &mut self,
        name: Option<&str>,
        attributes: &[AttributeContent],
        node: &Node,
    ) -> Option<usize> {
        let span = self.span(node);
        let offset = span.start;
        let component = name.is_some_and(|name| {
            name.chars().next().is_some_and(char::is_uppercase) || name.contains('.')
        });
        if component {
            self.dynamic(offset, "An MDX component can add or replace anchors and references. Supply static rendered output.");
        }
        let mut attrs = BTreeMap::new();
        let offsets = jsx_attribute_offsets(&self.source.text[span.clone()], offset);
        let mut attr_offsets = BTreeMap::new();
        for attribute in attributes {
            match attribute {
                AttributeContent::Expression(_) => self.dynamic(
                    offset,
                    "MDX spread attributes need execution. Supply static rendered output.",
                ),
                AttributeContent::Property(attribute) => {
                    let key = match attribute.name.as_str() {
                        "className" => "class".to_owned(),
                        "htmlFor" => "for".to_owned(),
                        key => key.to_ascii_lowercase(),
                    };
                    let attr_offset = offsets.get(&attribute.name).copied().unwrap_or(offset);
                    attr_offsets.insert(key.clone(), attr_offset);
                    let value = match &attribute.value {
                        Some(AttributeValue::Literal(value)) => value.clone(),
                        Some(AttributeValue::Expression(_)) => {
                            self.dynamic(attr_offset, "An MDX attribute expression cannot be resolved from static source.");
                            "{{mdx}}".into()
                        }
                        None => String::new(),
                    };
                    if attrs.insert(key.clone(), value.clone()).is_some() {
                        self.issue(
                            "markdown.syntax",
                            Requirement::Format,
                            Outcome::Invalid,
                            attr_offset,
                            format!("JSX attribute {:?} occurs more than once.", attribute.name),
                        );
                    }
                    if !component
                        && !matches!(&attribute.value, Some(AttributeValue::Expression(_)))
                    {
                        match key.as_str() {
                            "id" => self.doc.anchors.push(Anchor {
                                id: value,
                                offset: attr_offset,
                                tag: name.unwrap_or("").into(),
                            }),
                            "href" => self.doc.references.push(Reference::new(
                                value,
                                attr_offset,
                                ReferenceKind::Link,
                            )),
                            "src" | "poster" => self.doc.references.push(Reference::new(
                                value,
                                attr_offset,
                                ReferenceKind::Asset,
                            )),
                            _ => {}
                        }
                    }
                }
            }
        }
        let name = name.filter(|_| !component)?;
        let text = plain(node);
        let mut parent = self.jsx_parent;
        let mut visible = !matches!(name, "head" | "script" | "style" | "template");
        while let Some(index) = parent {
            let element = &self.doc.elements[index];
            visible &= !matches!(
                element.tag.as_str(),
                "head" | "script" | "style" | "template"
            );
            parent = element.parent;
        }
        if visible && crate::headings::is_content_element(name) {
            self.doc
                .content_ranges
                .push(offset..(offset + 1 + name.len()).min(span.end));
        }
        if let Some(level) = name
            .strip_prefix('h')
            .and_then(|s| s.parse::<u8>().ok())
            .filter(|n| (1..=6).contains(n))
        {
            self.doc.headings.push(Heading {
                level,
                text: text.clone(),
                id: attrs
                    .get("id")
                    .filter(|value| !value.starts_with("{{"))
                    .cloned(),
                offset,
                end: span.end,
            });
        } else {
            self.content.push(span.clone());
        }
        let index = self.doc.elements.len();
        self.doc.elements.push(HtmlElement {
            tag: name.to_owned(),
            attrs,
            attr_offsets,
            offset,
            end: span.end,
            parent: self.jsx_parent,
            text,
        });
        Some(index)
    }

    fn unique_id(&self, base: String, separator: &str) -> String {
        let mut result = base.clone();
        let mut count = 1;
        while self.ids.contains(&result) || result.is_empty() {
            result = format!("{base}{separator}{count}");
            count += 1;
        }
        result
    }

    fn reference(&mut self, identifier: &str, offset: usize, kind: ReferenceKind) {
        let id = label(identifier);
        self.used.insert(id.clone());
        if let Some((target, _, _)) = self.definitions.get(&id) {
            self.doc
                .references
                .push(Reference::new(target, offset, kind));
        } else if self.config.markdown.undefined_references {
            self.issue(
                "markdown.undefined-reference",
                Requirement::Project,
                Outcome::Invalid,
                offset,
                format!("Reference {id:?} has no definition."),
            );
        }
    }

    fn code(&mut self, value: &markdown::mdast::Code, span: Range<usize>) {
        let raw = &self.source.text[span.clone()];
        let first = raw.split(['\r', '\n']).next().unwrap_or("").trim_start();
        let fence_char = first.chars().next().filter(|c| matches!(c, '`' | '~'));
        let info = [value.lang.as_deref(), value.meta.as_deref()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" ");
        let mut content_offset = if fence_char.is_some() {
            span.start
                + raw.find(['\r', '\n']).map_or(raw.len(), |n| {
                    n + if raw.as_bytes().get(n..n + 2) == Some(b"\r\n") {
                        2
                    } else {
                        1
                    }
                })
        } else {
            span.start
        };
        if fence_char.is_some() && !value.value.is_empty() {
            let first_value = value.value.split(['\r', '\n']).next().unwrap_or("");
            let line = self.source.text[content_offset..span.end]
                .split(['\r', '\n'])
                .next()
                .unwrap_or("");
            if !first_value.is_empty()
                && let Some(index) = line.find(first_value)
            {
                content_offset += index;
            }
        }
        self.doc.code_blocks.push(CodeBlock {
            language: value.lang.clone(),
            info,
            value: value.value.clone(),
            offset: span.start,
            content_offset,
        });
        if self.config.markdown.require_language && value.lang.as_ref().is_none_or(|s| s.is_empty())
        {
            self.issue(
                "markdown.code-info",
                Requirement::Project,
                Outcome::Invalid,
                span.start,
                "This code block needs a language name.",
            );
        }
        if let Some(language) = &value.lang
            && !self.config.markdown.allowed_languages.is_empty()
            && !self
                .config
                .markdown
                .allowed_languages
                .iter()
                .any(|allowed| language_alias(allowed) == language_alias(language))
        {
            self.issue(
                "markdown.code-info",
                Requirement::Project,
                Outcome::Invalid,
                span.start,
                format!("Code language {language:?} is not allowed."),
            );
        }
        if let Some(character) = fence_char {
            let size = first.chars().take_while(|c| *c == character).count();
            if self.config.markdown.closed_fences {
                let mut last = raw
                    .trim_end_matches(['\r', '\n'])
                    .rsplit(['\r', '\n'])
                    .next()
                    .unwrap_or("")
                    .trim();
                for _ in 0..self.quote_depth {
                    last = last.strip_prefix('>').unwrap_or(last).trim_start();
                }
                let count = last.chars().take_while(|c| *c == character).count();
                if !raw.contains(['\r', '\n']) || count < size || !last[count..].trim().is_empty() {
                    self.issue(
                        "markdown.fence-closed",
                        Requirement::Project,
                        Outcome::Invalid,
                        span.start,
                        "This code fence needs an explicit closing fence.",
                    );
                }
            }
        }
        if let Some(meta) = &value.meta {
            self.code_meta(meta, span.start);
        }
    }

    fn code_meta(&mut self, meta: &str, offset: usize) {
        let meta = meta.trim();
        if matches!(meta, "invalid" | "expect-invalid") {
            return;
        }
        if !meta.starts_with('{') || !meta.ends_with('}') {
            // CommonMark permits any remaining fence info as literal metadata.
            return;
        }
        let Some(attrs) = self.attributes(&meta[1..meta.len() - 1], offset) else {
            return;
        };
        for (key, value) in &attrs {
            match key.as_str() {
                "id" => self.doc.anchors.push(Anchor {
                    id: value.clone(),
                    offset,
                    tag: "pre".into(),
                }),
                "file" => {}
                "start" | "end" => {
                    if value.parse::<usize>().ok().is_none_or(|n| n == 0) {
                        self.issue(
                            "markdown.code-info",
                            Requirement::Project,
                            Outcome::Invalid,
                            offset,
                            "Code line ranges need positive line numbers.",
                        );
                    }
                }
                "invalid" => {}
                _ => self.issue(
                    "markdown.code-info",
                    Requirement::Project,
                    Outcome::Unsupported,
                    offset,
                    format!("Code attribute {key:?} is not supported."),
                ),
            }
        }
        if let (Some(start), Some(end)) = (
            attrs.get("start").and_then(|v| v.parse::<usize>().ok()),
            attrs.get("end").and_then(|v| v.parse::<usize>().ok()),
        ) && start > end
        {
            self.issue(
                "markdown.code-info",
                Requirement::Project,
                Outcome::Invalid,
                offset,
                "The code range start must not exceed its end.",
            );
        }
        if let Some(target) = attrs.get("file") {
            if target.trim().is_empty() {
                self.issue(
                    "markdown.code-info",
                    Requirement::Project,
                    Outcome::Invalid,
                    offset,
                    "The example file path must not be empty.",
                );
            } else {
                self.doc.includes.push(Include {
                    target: target.clone(),
                    offset,
                    region: None,
                    start_line: attrs
                        .get("start")
                        .and_then(|value| value.parse::<usize>().ok()),
                    end_line: attrs
                        .get("end")
                        .and_then(|value| value.parse::<usize>().ok()),
                });
            }
        } else if attrs.contains_key("start") || attrs.contains_key("end") {
            self.issue(
                "markdown.code-info",
                Requirement::Project,
                Outcome::Invalid,
                offset,
                "A code line range needs a file attribute.",
            );
        }
    }

    fn attributes(&mut self, text: &str, offset: usize) -> Option<BTreeMap<String, String>> {
        let mut attrs = BTreeMap::new();
        let mut cursor = 0;
        for capture in ATTRIBUTES.captures_iter(text) {
            let matched = capture.get(0).unwrap();
            if !text[cursor..matched.start()].trim().is_empty() {
                self.issue(
                    "markdown.extension",
                    Requirement::Project,
                    Outcome::Invalid,
                    offset,
                    "Invalid extension attribute syntax.",
                );
                return None;
            }
            let (key, value) = if let Some(id) = capture.get(1) {
                ("id", id.as_str())
            } else {
                (
                    capture.get(2).unwrap().as_str(),
                    capture
                        .get(3)
                        .or(capture.get(4))
                        .or(capture.get(5))
                        .unwrap()
                        .as_str(),
                )
            };
            if attrs.insert(key.to_owned(), value.to_owned()).is_some() {
                self.issue(
                    "markdown.extension",
                    Requirement::Project,
                    Outcome::Invalid,
                    offset,
                    format!("Attribute {key:?} occurs more than once."),
                );
            }
            cursor = matched.end();
        }
        if !text[cursor..].trim().is_empty() {
            self.issue(
                "markdown.extension",
                Requirement::Project,
                Outcome::Invalid,
                offset,
                "Invalid extension attribute syntax.",
            );
            return None;
        }
        Some(attrs)
    }

    fn finish(&mut self) {
        self.undefined();
        for (id, offset) in self.footnotes.clone() {
            if self.config.markdown.anchors != AnchorStyle::PythonMarkdown
                && !self.footnote_uses.contains_key(&id)
            {
                continue;
            }
            let (target, _) = footnote_ids(self.config.markdown.anchors, &id, 1);
            self.doc.anchors.push(Anchor {
                id: target,
                offset,
                tag: "li".into(),
            });
        }
        if self.config.markdown.unused_definitions {
            for (id, (_, _, offset)) in self.definitions.clone() {
                if !self.used.contains(&id) {
                    self.issue(
                        "markdown.unused-definition",
                        Requirement::Project,
                        Outcome::Invalid,
                        offset,
                        format!("Reference definition {id:?} is not used."),
                    );
                }
            }
            for (id, offset) in self.footnotes.clone() {
                if !self.used.contains(&format!("^{id}")) {
                    self.issue(
                        "markdown.unused-definition",
                        Requirement::Project,
                        Outcome::Invalid,
                        offset,
                        format!("Footnote definition {id:?} is not used."),
                    );
                }
            }
        }
        self.heading_policy();
        self.extensions();
        self.citations();
        self.toc();
    }

    fn heading_policy(&mut self) {
        let headings = self.doc.headings.clone();
        if let (Some(first), Some(expected)) =
            (headings.first(), self.config.markdown.heading_start)
            && first.level != expected
        {
            self.issue(
                "markdown.heading-shape",
                Requirement::Project,
                Outcome::Invalid,
                first.offset,
                format!("The first heading must have level {expected}."),
            );
        }
        let mut h1 = 0;
        let mut previous = 0;
        for (index, heading) in headings.iter().enumerate() {
            if heading.level == 1 {
                h1 += 1;
            }
            if self.config.markdown.single_h1 && h1 > 1 && heading.level == 1 {
                self.issue(
                    "markdown.heading-shape",
                    Requirement::Project,
                    Outcome::Invalid,
                    heading.offset,
                    "Only one level-one heading is allowed.",
                );
            }
            if self.config.markdown.no_heading_skips
                && previous != 0
                && heading.level > previous + 1
            {
                self.issue(
                    "markdown.heading-shape",
                    Requirement::Project,
                    Outcome::Invalid,
                    heading.offset,
                    "This heading skips a level.",
                );
            }
            if self.config.markdown.nonempty_headings && heading.text.trim().is_empty() {
                self.issue(
                    "markdown.heading-shape",
                    Requirement::Project,
                    Outcome::Invalid,
                    heading.offset,
                    "This heading needs content.",
                );
            }
            previous = heading.level;
            if self.config.markdown.nonempty_sections {
                let end = headings
                    .get(index + 1)
                    .map_or(self.source.text.len(), |next| next.offset);
                if !self
                    .content
                    .iter()
                    .any(|range| range.start >= heading.end && range.start < end)
                {
                    self.issue(
                        "markdown.empty-section",
                        Requirement::Project,
                        Outcome::Invalid,
                        heading.offset,
                        "This section has no content before the next heading.",
                    );
                }
            }
        }
    }

    fn masked(&self) -> String {
        let mut bytes = self.source.text.as_bytes().to_vec();
        for range in &self.doc.opaque_ranges {
            for byte in &mut bytes[range.clone()] {
                if !matches!(*byte, b'\n' | b'\r') {
                    *byte = b' ';
                }
            }
        }
        String::from_utf8(bytes).expect("masked source is UTF-8")
    }

    fn undefined(&mut self) {
        // Only explicit full and collapsed forms are a reference contract.
        // A lone bracket label can be ordinary prose.
        let regex =
            Regex::new(r"!?\[([^\[\]\r\n]+)\]\[([^\[\]\r\n]*)\]|\[\^([^\[\]\r\n]+)\]").unwrap();
        for range in self.text_ranges.clone() {
            let raw = &self.source.text[range.clone()];
            for capture in regex.captures_iter(raw) {
                let start = capture.get(0).unwrap().start();
                if escaped(raw, start) {
                    continue;
                }
                let (id, footnote) = if let Some(id) = capture.get(3) {
                    (label(id.as_str()), true)
                } else {
                    let id = capture.get(2).unwrap().as_str();
                    (
                        label(if id.is_empty() {
                            capture.get(1).unwrap().as_str()
                        } else {
                            id
                        }),
                        false,
                    )
                };
                if footnote && self.config.markdown.dialect == MarkdownDialect::Commonmark {
                    continue;
                }
                if (footnote && !self.footnotes.contains_key(&id))
                    || (!footnote && !self.definitions.contains_key(&id))
                {
                    if self.config.markdown.undefined_references {
                        self.issue(
                            "markdown.undefined-reference",
                            Requirement::Project,
                            Outcome::Invalid,
                            range.start + start,
                            format!("Reference {id:?} has no definition."),
                        );
                    }
                } else if !footnote {
                    let kind = if capture.get(0).unwrap().as_str().starts_with('!') {
                        ReferenceKind::Image
                    } else {
                        ReferenceKind::Link
                    };
                    self.reference(&id, range.start + start, kind);
                }
            }
        }
    }

    fn extensions(&mut self) {
        let masked = self.masked();
        let mut boundaries = Vec::new();
        if self.config.markdown.wiki_links {
            let regex = Regex::new(r"\[\[([^\[\]\r\n]+)\]\]").unwrap();
            for capture in regex.captures_iter(&masked[self.source.body_offset..]) {
                let offset = capture.get(0).unwrap().start() + self.source.body_offset;
                if escaped(&masked, offset) {
                    continue;
                }
                let target = capture[1].split('|').next().unwrap().trim();
                self.doc
                    .references
                    .push(Reference::new(target, offset, ReferenceKind::Link));
            }
        }
        if self.config.markdown.includes {
            let regex = Regex::new(r"!include\[([^\]\r\n]*)\](?:\{([^}\r\n]*)\})?").unwrap();
            for capture in regex.captures_iter(&masked[self.source.body_offset..]) {
                let offset = capture.get(0).unwrap().start() + self.source.body_offset;
                if escaped(&masked, offset) {
                    continue;
                }
                boundaries.push(offset..offset + capture.get(0).unwrap().len());
                let Some(attrs) =
                    self.attributes(capture.get(2).map_or("", |v| v.as_str()), offset)
                else {
                    continue;
                };
                let target = capture[1].trim().to_owned();
                if target.is_empty() {
                    self.issue(
                        "markdown.extension",
                        Requirement::Project,
                        Outcome::Invalid,
                        offset,
                        "An include needs a target path.",
                    );
                }
                for key in attrs.keys() {
                    if !matches!(key.as_str(), "region" | "start" | "end") {
                        self.issue(
                            "markdown.extension",
                            Requirement::Project,
                            Outcome::Unsupported,
                            offset,
                            format!("Include attribute {key:?} is not supported."),
                        );
                    }
                }
                let mut number = |name: &str| -> Option<usize> {
                    attrs
                        .get(name)
                        .and_then(|value| match value.parse::<usize>() {
                            Ok(n) if n > 0 => Some(n),
                            _ => {
                                self.issue(
                                    "markdown.extension",
                                    Requirement::Project,
                                    Outcome::Invalid,
                                    offset,
                                    "Include line numbers must be positive integers.",
                                );
                                None
                            }
                        })
                };
                let start_line = number("start");
                let end_line = number("end");
                if start_line
                    .zip(end_line)
                    .is_some_and(|(start, end)| start > end)
                {
                    self.issue(
                        "markdown.extension",
                        Requirement::Project,
                        Outcome::Invalid,
                        offset,
                        "The include start must not exceed its end.",
                    );
                }
                if attrs.contains_key("region") && (start_line.is_some() || end_line.is_some()) {
                    self.issue(
                        "markdown.extension",
                        Requirement::Project,
                        Outcome::Invalid,
                        offset,
                        "Select an include region or line range, not both.",
                    );
                }
                self.doc.includes.push(Include {
                    target,
                    offset,
                    region: attrs.get("region").cloned(),
                    start_line,
                    end_line,
                });
            }
        }
        if self.config.markdown.directives {
            let regex = Regex::new(r"^(:{3,})([A-Za-z][A-Za-z0-9_-]*)?(?:\{(.*)\})?\s*$").unwrap();
            let mut stack = Vec::new();
            let mut offset = 0;
            for line in masked.split_inclusive(['\r', '\n']) {
                if offset >= self.source.body_offset {
                    if let Some(capture) = regex.captures(line.trim_end_matches(['\r', '\n'])) {
                        boundaries.push(offset..offset + line.len());
                        let size = capture[1].len();
                        if let Some(name) = capture.get(2) {
                            if stack.last().is_some_and(|(previous, _)| *previous >= size) {
                                self.issue(
                                    "markdown.extension",
                                    Requirement::Project,
                                    Outcome::Invalid,
                                    offset,
                                    "A nested directive needs a longer colon fence.",
                                );
                            }
                            let Some(attrs) =
                                self.attributes(capture.get(3).map_or("", |v| v.as_str()), offset)
                            else {
                                offset += line.len();
                                continue;
                            };
                            for (key, value) in attrs {
                                match key.as_str() {
                                    "id" => self.doc.anchors.push(Anchor {
                                        id: value,
                                        offset,
                                        tag: "div".into(),
                                    }),
                                    "title" | "class" => {}
                                    _ => self.issue(
                                        "markdown.extension",
                                        Requirement::Project,
                                        Outcome::Unsupported,
                                        offset,
                                        format!("Directive attribute {key:?} is not supported."),
                                    ),
                                }
                            }
                            if !matches!(
                                name.as_str(),
                                "note"
                                    | "tip"
                                    | "warning"
                                    | "important"
                                    | "caution"
                                    | "details"
                                    | "container"
                            ) {
                                self.issue(
                                    "markdown.extension",
                                    Requirement::Project,
                                    Outcome::Unsupported,
                                    offset,
                                    format!("Directive {:?} is not supported.", name.as_str()),
                                );
                            }
                            stack.push((size, offset));
                        } else if stack.last().is_some_and(|(previous, _)| *previous == size) {
                            stack.pop();
                        } else {
                            self.issue(
                                "markdown.extension",
                                Requirement::Project,
                                Outcome::Invalid,
                                offset,
                                "This directive fence has no matching open fence.",
                            );
                        }
                    } else if line.starts_with(":::") {
                        self.issue(
                            "markdown.extension",
                            Requirement::Project,
                            Outcome::Invalid,
                            offset,
                            "Invalid directive boundary or attributes.",
                        );
                    }
                }
                offset += line.len();
            }
            for (_, offset) in stack {
                self.issue(
                    "markdown.extension",
                    Requirement::Project,
                    Outcome::Invalid,
                    offset,
                    "This directive needs a closing colon fence.",
                );
            }
        }
        if boundaries.is_empty() {
            return;
        }
        boundaries.sort_by_key(|range| range.start);
        let mut merged: Vec<Range<usize>> = Vec::new();
        for boundary in boundaries {
            if let Some(previous) = merged.last_mut()
                && boundary.start <= previous.end
            {
                previous.end = previous.end.max(boundary.end);
            } else {
                merged.push(boundary);
            }
        }
        let content = std::mem::take(&mut self.doc.content_ranges);
        for range in content {
            let mut start = range.start;
            let first = merged.partition_point(|boundary| boundary.end <= start);
            for boundary in &merged[first..] {
                if boundary.start >= range.end {
                    break;
                }
                if boundary.start > start {
                    let part = start..boundary.start.min(range.end);
                    if !self.source.text[part.clone()].trim().is_empty() {
                        self.doc.content_ranges.push(part);
                    }
                }
                start = start.max(boundary.end.min(range.end));
            }
            if start < range.end {
                let part = start..range.end;
                if !self.source.text[part.clone()].trim().is_empty() {
                    self.doc.content_ranges.push(part);
                }
            }
        }
    }

    fn citations(&mut self) {
        if self.config.markdown.bibliography.is_empty() {
            return;
        }
        let mut keys = BTreeMap::new();
        let mut complete = true;
        for path in &self.config.markdown.bibliography {
            let path = self.config.resolve(path);
            match read_bibliography(&path, self.config) {
                Ok(text) => match biblatex::Bibliography::parse(&text) {
                    Ok(bibliography) => {
                        for entry in bibliography.iter() {
                            if keys.insert(entry.key.clone(), path.clone()).is_some() {
                                self.issue(
                                    "markdown.citation",
                                    Requirement::Project,
                                    Outcome::Invalid,
                                    self.source.body_offset,
                                    format!(
                                        "Bibliography key {:?} occurs in more than one file.",
                                        entry.key
                                    ),
                                );
                            }
                        }
                    }
                    Err(error) => {
                        complete = false;
                        self.issue(
                            "markdown.citation",
                            Requirement::Format,
                            Outcome::Invalid,
                            self.source.body_offset,
                            format!("Invalid bibliography {}: {error}", path.display()),
                        );
                    }
                },
                Err((requirement, outcome, message)) => {
                    complete = false;
                    self.issue(
                        "markdown.citation",
                        requirement,
                        outcome,
                        self.source.body_offset,
                        message,
                    );
                }
            }
        }
        let masked = self.masked();
        let citation = Regex::new(r"\[(@[^\]\r\n]*)\]").unwrap();
        let key_pattern = Regex::new(r"^@([A-Za-z0-9_:.+/-]+)$").unwrap();
        let mut used = BTreeSet::new();
        for capture in citation.captures_iter(&masked[self.source.body_offset..]) {
            let offset = capture.get(0).unwrap().start() + self.source.body_offset;
            if escaped(&masked, offset) {
                continue;
            }
            for part in capture[1].split(';') {
                if let Some(key) = key_pattern.captures(part.trim()) {
                    let key = key[1].to_owned();
                    if complete && !keys.contains_key(&key) {
                        self.issue(
                            "markdown.citation",
                            Requirement::Project,
                            Outcome::Invalid,
                            offset,
                            format!("Citation key {key:?} is not in the bibliography."),
                        );
                    }
                    used.insert(key);
                } else {
                    self.issue("markdown.citation", Requirement::Project, Outcome::Unsupported, offset, "Use citations in the form [@key; @other]. Citation locators are not supported.");
                }
            }
        }
        if self.config.markdown.unused_definitions {
            for key in keys.keys() {
                if !used.contains(key) {
                    self.issue(
                        "markdown.unused-definition",
                        Requirement::Project,
                        Outcome::Invalid,
                        self.source.body_offset,
                        format!("Bibliography key {key:?} is not used in this document."),
                    );
                }
            }
        }
    }

    fn toc(&mut self) {
        if !self.config.markdown.toc {
            return;
        }
        let body = self.source.body();
        let open = "<!-- toc -->";
        let close = "<!-- /toc -->";
        let spans: Vec<_> = self
            .doc
            .html_ranges
            .iter()
            .filter_map(|range| {
                let value = self.source.text[range.clone()].trim();
                if value == open || value == close {
                    Some((range.start, value.to_owned()))
                } else {
                    None
                }
            })
            .collect();
        if spans.len() != 2 || spans[0].1 != open || spans[1].1 != close {
            self.issue(
                "markdown.toc",
                Requirement::Project,
                Outcome::Invalid,
                self.source.body_offset,
                "The table of contents needs one <!-- toc --> and one <!-- /toc --> marker.",
            );
            return;
        }
        let start = spans[0].0;
        let end = spans[1].0;
        let actual: Vec<_> = self
            .doc
            .references
            .iter()
            .filter(|r| r.offset > start && r.offset < end && r.kind == ReferenceKind::Link)
            .map(|r| r.target.clone())
            .collect();
        let expected: Vec<_> = self
            .doc
            .headings
            .iter()
            .filter_map(|heading| heading.id.as_ref().map(|id| format!("#{id}")))
            .collect();
        if actual != expected {
            self.issue(
                "markdown.toc",
                Requirement::Project,
                Outcome::Invalid,
                start,
                "Table of contents links must match all heading anchors in source order.",
            );
        }
        if body.matches(open).count() > 1 || body.matches(close).count() > 1 {
            self.issue(
                "markdown.toc",
                Requirement::Project,
                Outcome::Invalid,
                start,
                "The table of contents markers must occur only once.",
            );
        }
    }
}

fn read_bibliography(
    path: &Path,
    config: &Config,
) -> Result<String, (Requirement, Outcome, String)> {
    let unreadable = |error: String| {
        (
            Requirement::Execution,
            Outcome::Unverified,
            format!("Cannot read bibliography {}: {error}", path.display()),
        )
    };
    let root =
        std::fs::canonicalize(&config.base_dir).map_err(|error| unreadable(error.to_string()))?;
    let canonical: PathBuf =
        std::fs::canonicalize(path).map_err(|error| unreadable(error.to_string()))?;
    if !config.links.allow_outside_root && !canonical.starts_with(root) {
        return Err((
            Requirement::Project,
            Outcome::Invalid,
            format!(
                "Bibliography {} leaves the configuration root",
                path.display()
            ),
        ));
    }
    let file = std::fs::File::open(&canonical).map_err(|error| unreadable(error.to_string()))?;
    let metadata = file
        .metadata()
        .map_err(|error| unreadable(error.to_string()))?;
    if !metadata.is_file() {
        return Err(unreadable("The target is not a regular file".into()));
    }
    if metadata.len() > config.files.max_file_bytes {
        return Err(unreadable("The file size limit was exceeded".into()));
    }
    let mut bytes = Vec::new();
    file.take(config.files.max_file_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| unreadable(error.to_string()))?;
    if bytes.len() as u64 > config.files.max_file_bytes {
        return Err(unreadable("The file size limit was exceeded".into()));
    }
    String::from_utf8(bytes).map_err(|error| {
        (
            Requirement::Format,
            Outcome::Invalid,
            format!("Bibliography {} is not UTF-8: {error}", path.display()),
        )
    })
}

fn empty_js(value: &str) -> bool {
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, value, SourceType::jsx()).parse();
    parsed.diagnostics.is_empty()
        && parsed.program.body.is_empty()
        && parsed.program.directives.is_empty()
}

fn footnote_ids(style: AnchorStyle, id: &str, count: usize) -> (String, String) {
    match style {
        AnchorStyle::Github | AnchorStyle::ExplicitOnly => {
            let suffix = if count > 1 {
                format!("-{count}")
            } else {
                String::new()
            };
            (
                format!("user-content-fn-{id}"),
                format!("user-content-fnref-{id}{suffix}"),
            )
        }
        AnchorStyle::PythonMarkdown => {
            let number = if count > 1 {
                count.to_string()
            } else {
                String::new()
            };
            (format!("fn:{id}"), format!("fnref{number}:{id}"))
        }
        AnchorStyle::Kramdown => {
            let suffix = if count > 1 {
                format!(":{}", count - 1)
            } else {
                String::new()
            };
            (format!("fn:{id}"), format!("fnref:{id}{suffix}"))
        }
    }
}

fn jsx_attribute_offsets(text: &str, base: usize) -> BTreeMap<String, usize> {
    let mut offsets = BTreeMap::new();
    let bytes = text.as_bytes();
    let mut cursor = 1;
    while cursor < bytes.len()
        && !bytes[cursor].is_ascii_whitespace()
        && !matches!(bytes[cursor], b'>' | b'/')
    {
        cursor += 1;
    }
    while cursor < bytes.len() {
        while cursor < bytes.len() && (bytes[cursor].is_ascii_whitespace() || bytes[cursor] == b'/')
        {
            cursor += 1;
        }
        if cursor == bytes.len() || bytes[cursor] == b'>' {
            break;
        }
        let start = cursor;
        while cursor < bytes.len()
            && (bytes[cursor].is_ascii_alphanumeric()
                || matches!(bytes[cursor], b'_' | b':' | b'.' | b'-'))
        {
            cursor += 1;
        }
        if cursor > start {
            offsets.insert(text[start..cursor].into(), base + start);
        }
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if bytes.get(cursor) == Some(&b'=') {
            cursor += 1;
            while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
                cursor += 1;
            }
        }
        match bytes.get(cursor).copied() {
            Some(quote @ (b'\'' | b'"')) => {
                cursor += 1;
                while cursor < bytes.len() && bytes[cursor] != quote {
                    if bytes[cursor] == b'\\' {
                        cursor += 1;
                    }
                    cursor += 1;
                }
                cursor += usize::from(cursor < bytes.len());
            }
            Some(b'{') => {
                let mut depth = 1;
                cursor += 1;
                let mut quote = None;
                while cursor < bytes.len() && depth > 0 {
                    let byte = bytes[cursor];
                    if let Some(active) = quote {
                        if byte == b'\\' {
                            cursor += 1;
                        } else if byte == active {
                            quote = None;
                        }
                    } else {
                        match byte {
                            b'\'' | b'"' | b'`' => quote = Some(byte),
                            b'{' => depth += 1,
                            b'}' => depth -= 1,
                            _ => {}
                        }
                    }
                    cursor += 1;
                }
            }
            _ if cursor == start => cursor += 1,
            _ => {}
        }
    }
    offsets
}

fn bracket(text: &str, start: usize) -> Option<(&str, usize)> {
    if text.as_bytes().get(start) != Some(&b'[') {
        return None;
    }
    let mut depth = 1;
    for index in start + 1..text.len() {
        match text.as_bytes()[index] {
            b'[' if !escaped(text, index) => depth += 1,
            b']' if !escaped(text, index) => {
                depth -= 1;
                if depth == 0 {
                    return Some((&text[start + 1..index], index));
                }
            }
            _ => {}
        }
    }
    None
}

fn plain(node: &Node) -> String {
    match node {
        Node::Text(value) => value.value.clone(),
        Node::InlineCode(value) => value.value.clone(),
        Node::Image(value) => value.alt.clone(),
        Node::ImageReference(value) => value.alt.clone(),
        Node::Html(_) | Node::MdxFlowExpression(_) | Node::MdxTextExpression(_) => String::new(),
        Node::MdxJsxFlowElement(value)
            if value
                .name
                .as_deref()
                .is_some_and(|name| matches!(name, "head" | "script" | "style" | "template")) =>
        {
            String::new()
        }
        Node::MdxJsxTextElement(value)
            if value
                .name
                .as_deref()
                .is_some_and(|name| matches!(name, "head" | "script" | "style" | "template")) =>
        {
            String::new()
        }
        Node::Break(_) => " ".into(),
        _ => node.children().map_or(String::new(), |children| {
            children.iter().map(plain).collect()
        }),
    }
}

fn label(value: &str) -> String {
    value
        .split([' ', '\t', '\r', '\n'])
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
        .to_uppercase()
        .to_lowercase()
}
fn escaped(text: &str, index: usize) -> bool {
    text.as_bytes()[..index]
        .iter()
        .rev()
        .take_while(|byte| **byte == b'\\')
        .count()
        % 2
        == 1
}
fn language_alias(value: &str) -> &str {
    match value {
        "js" => "javascript",
        "ts" => "typescript",
        "py" => "python",
        "yml" => "yaml",
        "sh" | "shell" => "bash",
        "rs" => "rust",
        "txt" => "text",
        _ => value,
    }
}
fn python_slug(value: &str) -> String {
    let ascii: String = value.nfkd().filter(char::is_ascii).collect();
    let text: String = ascii
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace() || matches!(c, '_' | '-'))
        .collect();
    PYTHON_SEPARATOR.replace_all(text.trim(), "-").into_owned()
}
fn kramdown_slug(value: &str) -> String {
    let text = value.trim_start_matches(|c: char| !c.is_ascii_alphabetic());
    let text: String = text
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '-'))
        .map(|c| {
            if c == ' ' {
                '-'
            } else {
                c.to_ascii_lowercase()
            }
        })
        .collect();
    if text.is_empty() {
        "section".into()
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn read(text: &str, config: &Config) -> ParsedDocument {
        parse(
            &Source::new(PathBuf::from("test.md"), "test.md".into(), text.into()),
            config,
        )
    }

    fn has(doc: &ParsedDocument, rule: &str) -> bool {
        doc.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.rule == rule)
    }

    #[test]
    fn commonmark_literals_and_open_fences_are_valid() {
        let mut config = Config::default();
        config.markdown.dialect = MarkdownDialect::Commonmark;
        let doc = read(
            "# Same\n\n# Same\n\n[ambiguous] * literal\n\n```text arbitrary metadata\n[link](missing.md)\n",
            &config,
        );
        assert_eq!(
            doc.anchors
                .iter()
                .map(|a| a.id.as_str())
                .collect::<Vec<_>>(),
            ["same", "same-1"]
        );
        assert!(doc.references.is_empty());
        assert!(!has(&doc, "markdown.fence-closed"));
        assert!(!has(&doc, "markdown.syntax"));
    }

    #[test]
    fn links_ignore_code_and_preserve_raw_html_boundaries() {
        let doc = read(
            "# *Bold* `code` &amp; 😀\n\n[x](target.md) ![alt](image.png) ` [no](bad) `\n\n<div id=\"raw\">\n[opaque](bad)\n</div>\n\n```md\n[hidden](bad)\n```\n",
            &Config::default(),
        );
        assert_eq!(doc.references.len(), 2);
        assert_eq!(doc.headings[0].text, "Bold code & 😀");
        assert_eq!(doc.html_ranges.len(), 1);
        assert_eq!(doc.code_blocks[0].value, "[hidden](bad)");
    }

    #[test]
    fn locations_are_original_unicode_and_crlf_bytes() {
        let text = "---\r\nx: true\r\n---\r\n# Café\r\n\r\né [x](target.md)\r\n";
        let mut source = Source::new(PathBuf::from("test.md"), "test.md".into(), text.into());
        source.body_offset = text.find("# Café").unwrap();
        let doc = parse(&source, &Config::default());
        assert_eq!(doc.headings[0].offset, source.body_offset);
        let location = source.location(doc.references[0].offset);
        assert_eq!((location.line, location.column), (6, 3));
        assert_eq!(doc.references[0].offset, text.find("[x]").unwrap());
    }

    #[test]
    fn references_use_first_definition_and_normalized_labels() {
        let mut config = Config::default();
        config.markdown.unused_definitions = true;
        let doc = read(
            "[Full][ A  B ] [A B][] ![A B]\n\n[A B]: first.md\n[a b]: second.md\n[unused]: unused.md\n",
            &config,
        );
        assert_eq!(doc.references.len(), 3);
        assert!(doc.references.iter().all(|r| r.target == "first.md"));
        assert!(has(&doc, "markdown.definition-conflict"));
        assert!(has(&doc, "markdown.unused-definition"));
        let same = read("[x]: one.md\n[X]: one.md\n\n[x]\n", &Config::default());
        assert!(!has(&same, "markdown.definition-conflict"));
    }

    #[test]
    fn undefined_reference_policy_keeps_bracket_prose_literal() {
        let mut config = Config::default();
        config.markdown.undefined_references = true;
        let doc = read(
            "[ordinary] [x][missing] [empty][] \\[escaped][missing] ` [code][missing] `\n",
            &config,
        );
        assert_eq!(
            doc.diagnostics
                .iter()
                .filter(|d| d.rule == "markdown.undefined-reference")
                .count(),
            2
        );
        let default = read("[x][missing] [^missing]\n", &Config::default());
        assert!(default.diagnostics.is_empty());
    }

    #[test]
    fn profiles_generate_renderer_ids() {
        let mut config = Config::default();
        config.markdown.anchors = AnchorStyle::PythonMarkdown;
        config.markdown.explicit_heading_ids = true;
        let doc = read("# Café\n# Café\n# Thing {#chosen}\n# 中文\n", &config);
        assert_eq!(
            doc.anchors
                .iter()
                .map(|a| a.id.as_str())
                .collect::<Vec<_>>(),
            ["cafe", "cafe_1", "chosen", "_1"]
        );
        config.markdown.anchors = AnchorStyle::Kramdown;
        let doc = read("# 123 Hello! world_\n# 123 Hello! world_\n# 123\n", &config);
        assert_eq!(
            doc.anchors
                .iter()
                .map(|a| a.id.as_str())
                .collect::<Vec<_>>(),
            ["hello-world", "hello-world-1", "section"]
        );
        config.markdown.anchors = AnchorStyle::ExplicitOnly;
        let doc = read("# None\n# Yes {#id}\n", &config);
        assert_eq!(doc.anchors.len(), 1);
        assert_eq!(doc.anchors[0].id, "id");
    }

    #[test]
    fn opt_in_heading_table_and_fence_rules() {
        let mut config = Config::default();
        config.markdown.single_h1 = true;
        config.markdown.no_heading_skips = true;
        config.markdown.nonempty_headings = true;
        config.markdown.nonempty_sections = true;
        config.markdown.table_columns = true;
        config.markdown.closed_fences = true;
        let doc = read(
            "# A\n### B\n#\n\n| A | B |\n| - | - |\n| only |\n\n```text\nvalue\n",
            &config,
        );
        assert!(has(&doc, "markdown.heading-shape"));
        assert!(has(&doc, "markdown.empty-section"));
        assert!(has(&doc, "markdown.table-columns"));
        assert!(has(&doc, "markdown.fence-closed"));
    }

    #[test]
    fn gfm_footnotes_and_task_lists_are_parsed() {
        let doc = read(
            "- [x] task\n- [ ] task\n\nA[^note] again[^note]\n\n[^note]: Text with [link](target.md).\n",
            &Config::default(),
        );
        assert!(doc.diagnostics.is_empty());
        assert!(doc.anchors.iter().any(|a| a.id == "user-content-fn-note"));
        assert!(
            doc.anchors
                .iter()
                .any(|a| a.id == "user-content-fnref-note-2")
        );
        assert!(doc.references.iter().any(|r| r.target == "target.md"));
    }

    #[test]
    fn mdx_uses_javascript_grammar_and_static_attributes() {
        let mut config = Config::default();
        config.markdown.dialect = MarkdownDialect::Mdx;
        let doc = read(
            "import X from './x.js'\n\n# Heading\n\n<a id=\"static\" href=\"target.md\">Text</a>\n\n{1 + 2}\n\n<X {...props} />\n",
            &config,
        );
        assert!(!has(&doc, "markdown.syntax"), "{:?}", doc.diagnostics);
        assert!(doc.anchors.iter().any(|a| a.id == "static"));
        assert!(doc.references.iter().any(|r| r.target == "target.md"));
        assert!(doc.dynamic_anchors);
        assert!(
            doc.diagnostics
                .iter()
                .any(|d| d.outcome == Outcome::Unverified)
        );
        for value in [
            "{1 + }",
            "{\"a\"; \"b\"}",
            "export const x = ;\n",
            "<div><span></div>",
        ] {
            assert!(has(&read(value, &config), "markdown.syntax"), "{value}");
        }
        assert!(!has(&read("{/* comment */}\n", &config), "markdown.syntax"));
    }

    #[test]
    fn declared_extensions_keep_literal_ranges_opaque() {
        let mut config = Config::default();
        config.markdown.wiki_links = true;
        config.markdown.includes = true;
        config.markdown.directives = true;
        let doc = read(
            "[[page.md#anchor|Text]]\n\n!include[part.md]{region=sample}\n\n:::note{#notice title=\"A title\"}\nText\n::::tip\nMore\n::::\n:::\n\n`[[hidden]]`\n\n```text\n!include[hidden.md]\n```\n",
            &config,
        );
        assert!(doc.diagnostics.is_empty(), "{:?}", doc.diagnostics);
        assert_eq!(doc.includes.len(), 1);
        assert_eq!(doc.includes[0].region.as_deref(), Some("sample"));
        assert_eq!(doc.references.len(), 1);
        assert_eq!(doc.references[0].target, "page.md#anchor");
        assert!(doc.anchors.iter().any(|a| a.id == "notice"));
        let bad = read(
            ":::note{title=x title=y}\n!include[a.md]{start=9 end=1}\n",
            &config,
        );
        assert!(has(&bad, "markdown.extension"));
    }

    #[test]
    fn table_of_contents_checks_declared_order() {
        let mut config = Config::default();
        config.markdown.toc = true;
        let good = read(
            "# A\n\n<!-- toc -->\n- [A](#a)\n- [B](#b)\n<!-- /toc -->\n\n## B\nText\n",
            &config,
        );
        assert!(!has(&good, "markdown.toc"), "{:?}", good.diagnostics);
        let bad = read(
            "# A\n\n<!-- toc -->\n- [missing](#b)\n<!-- /toc -->\n",
            &config,
        );
        assert!(has(&bad, "markdown.toc"));
    }

    #[test]
    fn reference_label_spaces_and_unicode_do_not_merge_distinct_keys() {
        let mut config = Config::default();
        config.markdown.undefined_references = true;
        let doc = read(
            "[spaced][ A  B ] [joined][ab] [fold][SS]\n\n[A B]: spaced.md\n[ab]: joined.md\n[ẞ]: folded.md\n",
            &config,
        );
        assert_eq!(
            doc.references
                .iter()
                .map(|reference| reference.target.as_str())
                .collect::<BTreeSet<_>>(),
            BTreeSet::from(["spaced.md", "joined.md", "folded.md"])
        );
        assert!(!has(&doc, "markdown.definition-conflict"));
        assert!(!has(&doc, "markdown.undefined-reference"));
    }

    #[test]
    fn fences_in_containers_have_original_locations_and_valid_closure() {
        let mut config = Config::default();
        config.markdown.closed_fences = true;
        let text = "> ```json\n> {\"x\": 1}\n> ```\n\n- item\n\n  ```text\n  body\n  ```\n";
        let doc = read(text, &config);
        assert!(!has(&doc, "markdown.fence-closed"), "{:?}", doc.diagnostics);
        assert_eq!(doc.code_blocks.len(), 2);
        assert_eq!(doc.code_blocks[0].offset, 2);
        assert_eq!(doc.code_blocks[0].value, "{\"x\": 1}");
    }

    #[test]
    fn table_extra_cells_and_escaped_delimiters_are_checked() {
        let mut config = Config::default();
        config.markdown.table_columns = true;
        let good = read("| A | B |\n| - | - |\n| a\\|b | `c\\|d` |\n", &config);
        assert!(!has(&good, "markdown.table-columns"));
        let bad = read("| A | B |\n| - | - |\n| a | b | c |\n", &config);
        assert!(has(&bad, "markdown.table-columns"));
    }

    #[test]
    fn mdx_static_elements_keep_parents_text_and_attribute_offsets() {
        let mut config = Config::default();
        config.markdown.dialect = MarkdownDialect::Mdx;
        let text = "<div><h1 id=\"title\">Title</h1><button title=\"id\" id=\"go\">Go</button><template><a href=\"missing\">Hidden</a></template></div>\n\n{/* comment */}\n";
        let doc = read(text, &config);
        assert!(doc.diagnostics.is_empty(), "{:?}", doc.diagnostics);
        assert_eq!(doc.elements.len(), 4);
        assert_eq!(doc.elements[2].parent, Some(0));
        assert_eq!(doc.elements[2].text, "Go");
        assert_eq!(
            doc.elements[2].attr_offsets["id"],
            text.find("id=\"go\"").unwrap()
        );
        assert_eq!(doc.headings[0].text, "Title");
        assert!(doc.references.is_empty());
    }

    #[test]
    fn unselected_table_extension_cannot_report_a_complete_check() {
        let mut config = Config::default();
        config.markdown.dialect = MarkdownDialect::Commonmark;
        config.markdown.table_columns = true;
        let doc = read("| A | B |\n| - | - |\n| only |\n", &config);
        assert!(
            doc.diagnostics
                .iter()
                .any(|d| d.rule == "markdown.table-columns"
                    && d.outcome == Outcome::Unsupported
                    && d.required)
        );
    }

    #[test]
    fn standalone_cr_fence_locations_and_closure_are_supported() {
        let mut config = Config::default();
        config.markdown.closed_fences = true;
        let doc = read("```json\r{\"x\":1}\r```\r", &config);
        assert!(!has(&doc, "markdown.fence-closed"));
        assert_eq!(doc.code_blocks[0].content_offset, 8);
    }

    #[test]
    fn heading_suffix_ids_follow_the_selected_extension() {
        let mut config = Config::default();
        let text = "# Heading {#Chosen}\n";
        let default = read(text, &config);
        assert_eq!(default.headings[0].text, "Heading {#Chosen}");
        assert_eq!(default.anchors[0].id, "heading-chosen");
        config.markdown.explicit_heading_ids = true;
        let selected = read(text, &config);
        assert_eq!(selected.headings[0].text, "Heading");
        assert_eq!(selected.anchors[0].id, "Chosen");
        config.markdown.explicit_heading_ids = false;
        config.markdown.anchors = AnchorStyle::PythonMarkdown;
        assert_eq!(read(text, &config).anchors[0].id, "heading-chosen");
        config.markdown.anchors = AnchorStyle::Kramdown;
        assert_eq!(read(text, &config).anchors[0].id, "Chosen");
        config.markdown.anchors = AnchorStyle::ExplicitOnly;
        assert_eq!(read(text, &config).anchors[0].id, "Chosen");
    }

    #[test]
    fn external_code_files_use_include_records_for_line_bounds() {
        let doc = read(
            "```rust {file=example.rs start=2 end=5}\nvalue\n```\n",
            &Config::default(),
        );
        assert!(doc.diagnostics.is_empty(), "{:?}", doc.diagnostics);
        assert_eq!(doc.includes.len(), 1);
        let include = &doc.includes[0];
        assert_eq!(include.target, "example.rs");
        assert_eq!((include.start_line, include.end_line), (Some(2), Some(5)));
        assert_eq!(include.offset, doc.code_blocks[0].offset);
        let no_file = read("```rust {start=1 end=2}\nvalue\n```\n", &Config::default());
        assert!(
            no_file
                .diagnostics
                .iter()
                .any(|d| d.rule == "markdown.code-info" && d.outcome == Outcome::Invalid)
        );
        assert!(no_file.includes.is_empty());
        for attrs in [
            "file=example.rs start=0",
            "file=example.rs start=5 end=2",
            "file=example.rs end=invalid",
            "file=\"\" start=1",
        ] {
            let text = format!("```rust {{{attrs}}}\nvalue\n```\n");
            assert!(
                has(&read(&text, &Config::default()), "markdown.code-info"),
                "{attrs}"
            );
        }
    }

    #[test]
    fn external_code_ranges_are_checked_against_the_local_target() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("page.md");
        std::fs::write(dir.path().join("example.rs"), "first\nsecond\n").unwrap();
        let config = Config {
            base_dir: dir.path().into(),
            ..Config::default()
        };
        std::fs::write(
            &source,
            "```rust {file=example.rs start=1 end=2}\nvalue\n```\n",
        )
        .unwrap();
        let valid = crate::validate(&[dir.path().into()], &config).unwrap();
        assert!(valid.is_valid(), "{:?}", valid.diagnostics);
        std::fs::write(
            &source,
            "```rust {file=example.rs start=1 end=3}\nvalue\n```\n",
        )
        .unwrap();
        let invalid = crate::validate(&[dir.path().into()], &config).unwrap();
        assert!(
            invalid
                .diagnostics
                .iter()
                .any(|d| d.rule == "include.target" && d.outcome == Outcome::Invalid)
        );
        assert_eq!(invalid.exit_code(), 1);
    }

    #[test]
    fn bibliography_read_limits_are_required_incomplete_outcomes() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("refs.bib"),
            "@article{known, title = {Title}}\n",
        )
        .unwrap();
        let mut config = Config {
            base_dir: dir.path().into(),
            ..Config::default()
        };
        config.files.max_file_bytes = 4;
        config.markdown.bibliography.push("refs.bib".into());
        let doc = read("[@known]", &config);
        assert!(doc.diagnostics.iter().any(|d| d.rule == "markdown.citation"
            && d.outcome == Outcome::Unverified
            && d.required));
        assert!(
            !doc.diagnostics
                .iter()
                .any(|d| d.outcome == Outcome::Invalid)
        );
        config.files.max_file_bytes = 1024;
        config.markdown.bibliography[0] = "missing.bib".into();
        let doc = read("[@known]", &config);
        assert!(
            doc.diagnostics
                .iter()
                .any(|d| d.outcome == Outcome::Unverified)
        );
        assert!(
            !doc.diagnostics
                .iter()
                .any(|d| d.outcome == Outcome::Invalid)
        );
    }

    #[test]
    fn bibliography_syntax_and_utf8_errors_are_invalid() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("refs.bib");
        let mut config = Config {
            base_dir: dir.path().into(),
            ..Config::default()
        };
        config.markdown.bibliography.push("refs.bib".into());
        for contents in [b"@article{broken".as_slice(), b"\xff\xfe".as_slice()] {
            std::fs::write(&path, contents).unwrap();
            let doc = read("[@known]", &config);
            assert!(doc.diagnostics.iter().any(|d| d.rule == "markdown.citation"
                && d.outcome == Outcome::Invalid
                && d.requirement == Requirement::Format));
            assert!(
                !doc.diagnostics
                    .iter()
                    .any(|d| d.outcome == Outcome::Unverified)
            );
        }
    }

    #[test]
    fn bibliography_canonical_root_policy_can_be_explicitly_relaxed() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let bibliography = outside.path().join("refs.bib");
        std::fs::write(&bibliography, "@article{known, title={Title}}\n").unwrap();
        let mut config = Config {
            base_dir: root.path().into(),
            ..Config::default()
        };
        config
            .markdown
            .bibliography
            .push(bibliography.to_string_lossy().into_owned());
        let rejected = read("[@known]", &config);
        assert!(
            rejected
                .diagnostics
                .iter()
                .any(|d| d.rule == "markdown.citation"
                    && d.outcome == Outcome::Invalid
                    && d.requirement == Requirement::Project)
        );
        config.links.allow_outside_root = true;
        assert!(read("[@known]", &config).diagnostics.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn bibliography_symlinks_cannot_bypass_the_canonical_root() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let bibliography = outside.path().join("refs.bib");
        std::fs::write(&bibliography, "@article{known, title={Title}}\n").unwrap();
        std::os::unix::fs::symlink(&bibliography, root.path().join("linked.bib")).unwrap();
        let mut config = Config {
            base_dir: root.path().into(),
            ..Config::default()
        };
        config.markdown.bibliography.push("linked.bib".into());
        assert!(
            read("[@known]", &config)
                .diagnostics
                .iter()
                .any(|d| d.outcome == Outcome::Invalid && d.requirement == Requirement::Project)
        );
    }

    #[test]
    fn deep_markdown_and_mdx_trees_stop_before_recursive_reader_walks() {
        let mut config = Config::default();
        for text in [
            format!("{}Text\n", ">".repeat(300)),
            (0..140)
                .map(|level| format!("{}- item\n", "  ".repeat(level)))
                .collect(),
        ] {
            let doc = read(&text, &config);
            assert!(
                doc.diagnostics.iter().any(|d| d.rule == "markdown.syntax"
                    && d.outcome == Outcome::Unverified
                    && d.requirement == Requirement::Execution
                    && d.required),
                "{:?}",
                doc.diagnostics
            );
            assert!(doc.headings.is_empty());
        }
        config.markdown.dialect = MarkdownDialect::Mdx;
        let text = format!("{}Text{}\n", "<div>".repeat(300), "</div>".repeat(300));
        assert!(
            read(&text, &config)
                .diagnostics
                .iter()
                .any(|d| d.outcome == Outcome::Unverified && d.required)
        );
    }

    #[test]
    fn markdown_node_budget_stops_large_structural_documents() {
        let text = "# Heading\n".repeat(50_000);
        let doc = read(&text, &Config::default());
        assert!(doc.diagnostics.iter().any(|d| d.rule == "markdown.syntax"
            && d.outcome == Outcome::Unverified
            && d.required));
    }

    #[test]
    fn disabling_syntax_diagnostics_does_not_hide_incomplete_reader_coverage() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("page.md"),
            format!("{}Text\n", ">".repeat(300)),
        )
        .unwrap();
        let mut config = Config {
            base_dir: dir.path().into(),
            ..Config::default()
        };
        config.rules.disable.push("markdown.syntax".into());
        let report = crate::validate(&[dir.path().into()], &config).unwrap();
        assert_eq!(report.exit_code(), 3);
        assert_eq!(report.coverage.parsed, 0);
        assert!(
            report
                .diagnostics
                .iter()
                .any(|d| d.rule == "discovery.complete"
                    && d.outcome == Outcome::Unverified
                    && d.required
                    && d.suppression.is_none())
        );
    }

    #[test]
    fn long_unicode_prose_and_code_literals_do_not_consume_a_false_nesting_budget() {
        let prose = "日本語 é 🦀 [ordinary brackets] ".repeat(10_000);
        let code = format!(
            "{}{}{}",
            ">".repeat(1000),
            "[x](missing.md) ".repeat(10_000),
            "<div>".repeat(1000)
        );
        let text = format!("# Heading\n\n{prose}\n\n```text\n{code}\n```\n");
        let doc = read(&text, &Config::default());
        assert!(doc.diagnostics.is_empty(), "{:?}", doc.diagnostics);
        assert!(doc.references.is_empty());
        assert_eq!(doc.code_blocks.len(), 1);
        assert_eq!(doc.code_blocks[0].value, code);
    }

    #[test]
    fn mdx_javascript_nesting_limits_stop_before_the_recursive_parser() {
        let mut config = Config::default();
        config.markdown.dialect = MarkdownDialect::Mdx;
        for expression in [
            format!("{}0{}", "[".repeat(300), "]".repeat(300)),
            format!("日本語 / {}0{}", "[".repeat(300), "]".repeat(300)),
            format!(
                "<div>{{<span/> + {}0{}}}</div>",
                "[".repeat(300),
                "]".repeat(300)
            ),
            format!("`prefix${{{}0{}}}`", "[".repeat(300), "]".repeat(300)),
            format!("{}0", "!".repeat(300)),
            format!("{}0", "x = ".repeat(300)),
            format!("{}0{}", "f(".repeat(300), ")".repeat(300)),
            format!("{}Text{}", "<div>".repeat(300), "</div>".repeat(300)),
        ] {
            let doc = read(&format!("{{{expression}}}\n"), &config);
            assert!(doc.incomplete);
            assert!(
                doc.diagnostics.iter().any(|d| d.rule == "markdown.syntax"
                    && d.outcome == Outcome::Unverified
                    && d.requirement == Requirement::Execution
                    && d.required),
                "{:?}",
                doc.diagnostics
            );
        }
        let esm = format!(
            "export const x = {}0{};\n",
            "[".repeat(300),
            "]".repeat(300)
        );
        assert!(read(&esm, &config).incomplete);
    }

    #[test]
    fn mdx_lexical_literals_do_not_create_a_false_javascript_nesting_limit() {
        let mut config = Config::default();
        config.markdown.dialect = MarkdownDialect::Mdx;
        let brackets = "[".repeat(1000);
        for expression in [
            format!("\"{brackets}\""),
            format!("/* {brackets} */ 0"),
            format!("`{brackets}`"),
            format!("/{brackets}]/.test('x')"),
            format!("<div>{brackets}</div>"),
            format!("<div>{{<span/> + \"{brackets}\"}}</div>"),
        ] {
            let doc = read(&format!("{{{expression}}}\n"), &config);
            assert!(!doc.incomplete, "{:?}", doc.diagnostics);
            assert!(!has(&doc, "markdown.syntax"), "{:?}", doc.diagnostics);
        }
    }

    #[test]
    fn citation_keys_require_a_valid_local_bibliography() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("refs.bib"),
            "@article{known, title = {Title}, author = {A. Name}, year = {2020}}\n",
        )
        .unwrap();
        let mut config = Config {
            base_dir: dir.path().into(),
            ..Config::default()
        };
        config.markdown.bibliography.push("refs.bib".into());
        assert!(!has(
            &read("Known [@known]\n", &config),
            "markdown.citation"
        ));
        assert!(has(
            &read("Missing [@missing]\n", &config),
            "markdown.citation"
        ));
        let code = read("`[@missing]`\n", &config);
        assert!(!has(&code, "markdown.citation"));
    }
}
