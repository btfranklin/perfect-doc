//! Read HTML source without running scripts or changing the input.
//!
//! Token errors retain source byte positions. Tree errors use the document start
//! because the tree parser does not supply positions. Element parents describe
//! source nesting, with supported omitted end tags applied.
//!
//! The source rules cover selected content models, attributes, and static
//! associations. They do not cover all HTML content models, SVG or MathML
//! vocabularies, the full ARIA role matrix, or computed accessible names.
use crate::config::{Config, HtmlMode};
use crate::model::{
    Anchor, CodeBlock, Heading, HtmlElement, Outcome, ParsedDocument, Reference, ReferenceKind,
    Requirement, RuleDefinition, Source,
};
use html5gum::{DefaultEmitter, Token, Tokenizer};
use scraper::{Html, Selector};
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

pub const RULES: &[RuleDefinition] = &[
    RuleDefinition {
        id: "html.syntax",
        family: 16,
        requirement: Requirement::Format,
        description: "HTML token and tree parse errors",
    },
    RuleDefinition {
        id: "html.structure",
        family: 16,
        requirement: Requirement::Format,
        description: "Supported HTML element content and tag requirements",
    },
    RuleDefinition {
        id: "html.attributes",
        family: 16,
        requirement: Requirement::Format,
        description: "Required attributes and supported attribute values",
    },
    RuleDefinition {
        id: "html.document",
        family: 16,
        requirement: Requirement::Profile,
        description: "Complete HTML document structure",
    },
    RuleDefinition {
        id: "html.references",
        family: 17,
        requirement: Requirement::Format,
        description: "Static form, table, map, and ARIA associations",
    },
    RuleDefinition {
        id: "html.aria",
        family: 17,
        requirement: Requirement::Format,
        description: "Supported ARIA roles and attribute values",
    },
    RuleDefinition {
        id: "html.accessibility",
        family: 18,
        requirement: Requirement::Profile,
        description: "Static image and control name sources",
    },
    RuleDefinition {
        id: "html.language",
        family: 18,
        requirement: Requirement::Project,
        description: "Document language and language identifier syntax",
    },
    RuleDefinition {
        id: "html.landmarks",
        family: 18,
        requirement: Requirement::Project,
        description: "Required main landmark",
    },
    RuleDefinition {
        id: "html.srcset",
        family: 19,
        requirement: Requirement::Format,
        description: "Image candidate URLs and descriptors",
    },
    RuleDefinition {
        id: "html.encoding",
        family: 2,
        requirement: Requirement::Format,
        description: "HTML encoding declarations",
    },
    RuleDefinition {
        id: "html.dynamic",
        family: 16,
        requirement: Requirement::Execution,
        description: "HTML template expressions that prevent static checks",
    },
];

pub fn parse(source: &Source, config: &Config) -> ParsedDocument {
    read(source, source.body(), source.body_offset, config, false)
}

/// Keep source byte positions when Markdown supplies separate HTML ranges.
pub fn parse_fragment(source: &Source, ranges: &[Range<usize>], config: &Config) -> ParsedDocument {
    let mut bytes = source.text.as_bytes().to_vec();
    for byte in &mut bytes {
        if matches!(*byte, b'<' | b'&') {
            *byte = b' ';
        }
    }
    for range in ranges {
        if range.start <= range.end
            && range.end <= bytes.len()
            && source.text.is_char_boundary(range.start)
            && source.text.is_char_boundary(range.end)
        {
            bytes[range.clone()].copy_from_slice(&source.text.as_bytes()[range.clone()]);
        }
    }
    let masked = String::from_utf8(bytes).expect("masked source is UTF-8");
    let mut parsed = read(source, &masked, 0, config, true);
    parsed.content_ranges = parsed
        .content_ranges
        .iter()
        .flat_map(|content| {
            ranges.iter().filter_map(|range| {
                let start = content.start.max(range.start);
                let end = content.end.min(range.end);
                (start < end).then_some(start..end)
            })
        })
        .collect();
    parsed
}

fn issue(
    doc: &mut ParsedDocument,
    source: &Source,
    rule: &str,
    offset: usize,
    message: impl Into<String>,
) {
    let requirement = RULES
        .iter()
        .find(|r| r.id == rule)
        .map_or(Requirement::Format, |r| r.requirement);
    doc.diagnostics
        .push(source.diagnostic(rule, requirement, Outcome::Invalid, offset, message));
}
fn uncertain(doc: &mut ParsedDocument, source: &Source, offset: usize, message: impl Into<String>) {
    doc.diagnostics.push(source.diagnostic(
        "html.dynamic",
        Requirement::Execution,
        Outcome::Unverified,
        offset,
        message,
    ));
}
fn string(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}
fn void(tag: &str) -> bool {
    matches!(
        tag,
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "source"
            | "track"
            | "wbr"
    )
}
fn optional(tag: &str) -> bool {
    matches!(
        tag,
        "html"
            | "caption"
            | "head"
            | "body"
            | "li"
            | "dt"
            | "dd"
            | "p"
            | "rt"
            | "rp"
            | "optgroup"
            | "option"
            | "colgroup"
            | "thead"
            | "tbody"
            | "tfoot"
            | "tr"
            | "td"
            | "th"
    )
}
fn may_omit_at_parent_end(elements: &[HtmlElement], index: usize) -> bool {
    let e = &elements[index];
    if !optional(&e.tag) || matches!(e.tag.as_str(), "dt" | "thead") {
        return false;
    }
    if e.tag == "p"
        && e.parent.is_some_and(|i| {
            matches!(
                elements[i].tag.as_str(),
                "a" | "audio" | "del" | "ins" | "map" | "noscript" | "video"
            ) || custom_name(&elements[i].tag)
        })
    {
        return false;
    }
    true
}
fn closes(previous: &str, next: &str) -> bool {
    match previous {
        "li" => next == "li",
        "dt" | "dd" => matches!(next, "dt" | "dd"),
        "rt" | "rp" => matches!(next, "rt" | "rp"),
        "option" => matches!(next, "option" | "optgroup" | "hr"),
        "optgroup" => matches!(next, "optgroup" | "hr"),
        "thead" | "tbody" => matches!(next, "tbody" | "tfoot"),
        "caption" => matches!(
            next,
            "colgroup" | "col" | "thead" | "tbody" | "tfoot" | "tr"
        ),
        "colgroup" => !matches!(next, "col" | "template"),
        "tr" => next == "tr",
        "td" | "th" => matches!(next, "td" | "th" | "tr"),
        "head" => !matches!(
            next,
            "base"
                | "basefont"
                | "bgsound"
                | "link"
                | "meta"
                | "title"
                | "noscript"
                | "noframes"
                | "style"
                | "script"
                | "template"
        ),
        "p" => matches!(
            next,
            "address"
                | "article"
                | "aside"
                | "blockquote"
                | "details"
                | "dialog"
                | "div"
                | "dl"
                | "fieldset"
                | "figcaption"
                | "figure"
                | "footer"
                | "form"
                | "h1"
                | "h2"
                | "h3"
                | "h4"
                | "h5"
                | "h6"
                | "header"
                | "hgroup"
                | "hr"
                | "main"
                | "menu"
                | "nav"
                | "ol"
                | "p"
                | "pre"
                | "search"
                | "section"
                | "table"
                | "ul"
        ),
        _ => false,
    }
}
fn dynamic(value: &str) -> bool {
    value.contains("{{") || value.contains("{%")
}
fn read(
    source: &Source,
    input: &str,
    base: usize,
    config: &Config,
    fragment: bool,
) -> ParsedDocument {
    let mut doc = ParsedDocument::default();
    let emitter = DefaultEmitter::<usize>::new_with_span();
    let mut tokenizer = Tokenizer::new_with_emitter(input, emitter);
    let mut stack: Vec<usize> = Vec::new();
    let mut foreign: Vec<bool> = Vec::new();
    let mut doctypes = Vec::new();
    let mut content_starts = BTreeMap::new();
    while let Some(token) = tokenizer.next() {
        let token = token.unwrap();
        match token {
            Token::Error(error) => {
                if config.html.conformance
                    && !(error.value == html5gum::Error::CdataInHtmlContent
                        && foreign.last().copied().unwrap_or(false))
                {
                    issue(
                        &mut doc,
                        source,
                        "html.syntax",
                        base + error.span.start,
                        format!("HTML parse error: {}", error.value),
                    );
                }
            }
            Token::Doctype(value) => {
                doctypes.push((
                    base + value.span.start,
                    string(&value.name),
                    value.force_quirks,
                    value.public_identifier.is_some(),
                    value.system_identifier.as_ref().map(|s| string(s)),
                ));
            }
            Token::StartTag(tag) => {
                let name = string(&tag.name);
                while stack
                    .last()
                    .is_some_and(|i| closes(&doc.elements[*i].tag, &name))
                {
                    let index = stack.pop().unwrap();
                    foreign.pop();
                    doc.elements[index].end = base + tag.span.start;
                }
                // An omitted row or section end tag also closes its cells.
                if matches!(name.as_str(), "tr" | "tbody" | "tfoot") {
                    while stack.last().is_some_and(|i| {
                        matches!(doc.elements[*i].tag.as_str(), "td" | "th" | "tr")
                            && doc.elements[*i].tag != name
                    }) {
                        let index = stack.pop().unwrap();
                        foreign.pop();
                        doc.elements[index].end = base + tag.span.start;
                    }
                    if let Some(index) = stack.pop_if(|i| closes(&doc.elements[*i].tag, &name)) {
                        foreign.pop();
                        doc.elements[index].end = base + tag.span.start;
                    }
                }
                let integration = stack.last().is_some_and(|i| {
                    let parent = &doc.elements[*i];
                    matches!(parent.tag.as_str(), "foreignobject" | "desc" | "title")
                        || (matches!(parent.tag.as_str(), "mi" | "mo" | "mn" | "ms" | "mtext")
                            && !matches!(name.as_str(), "mglyph" | "malignmark"))
                        || (parent.tag == "annotation-xml"
                            && parent.attrs.get("encoding").is_some_and(|v| {
                                v.eq_ignore_ascii_case("text/html")
                                    || v.eq_ignore_ascii_case("application/xhtml+xml")
                            }))
                });
                let is_foreign = matches!(name.as_str(), "svg" | "math")
                    || (foreign.last().copied().unwrap_or(false) && !integration);
                let mut attrs = BTreeMap::new();
                let mut attr_offsets = BTreeMap::new();
                for (key, value) in &tag.attributes {
                    attrs.insert(string(key), string(value));
                    attr_offsets.insert(string(key), base + value.span.start);
                }
                let index = doc.elements.len();
                if index >= 100_000 || stack.len() >= 256 {
                    doc.incomplete = true;
                    doc.diagnostics.push(source.diagnostic(
                        "html.syntax",
                        Requirement::Execution,
                        Outcome::Unverified,
                        base + tag.span.start,
                        "HTML source exceeds the reader element or nesting limit.",
                    ));
                    return doc;
                }
                let element = HtmlElement {
                    tag: name.clone(),
                    attrs,
                    attr_offsets,
                    offset: base + tag.span.start,
                    end: base + tag.span.end,
                    parent: stack.last().copied(),
                    text: String::new(),
                };
                if config.html.conformance && tag.self_closing && !void(&name) && !is_foreign {
                    issue(
                        &mut doc,
                        source,
                        "html.structure",
                        element.offset,
                        format!("The {name} element cannot use a self-closing start tag."),
                    );
                }
                check_element(&mut doc, source, &element, &stack, is_foreign, config);
                if crate::headings::is_content_element(&name)
                    && !stack.iter().any(|i| {
                        matches!(
                            doc.elements[*i].tag.as_str(),
                            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "head" | "template"
                        )
                    })
                {
                    doc.content_ranges
                        .push(base + tag.span.start..base + tag.span.end);
                }
                doc.elements.push(element);
                content_starts.insert(index, base + tag.span.end);
                if !void(&name) && !(is_foreign && tag.self_closing) {
                    stack.push(index);
                    foreign.push(is_foreign);
                    if !is_foreign
                        && let Some(state) = html5gum::emitters::naive_next_state(&tag.name)
                    {
                        tokenizer.set_state(state);
                    }
                }
            }
            Token::EndTag(tag) => {
                let name = string(&tag.name);
                let offset = base + tag.span.start;
                if let Some(position) = stack.iter().rposition(|i| doc.elements[*i].tag == name) {
                    for index in stack.drain(position..).rev() {
                        let end_element = doc.elements[index].clone();
                        if config.html.conformance
                            && doc.elements[index].tag != name
                            && !may_omit_at_parent_end(&doc.elements, index)
                        {
                            issue(
                                &mut doc,
                                source,
                                "html.structure",
                                end_element.offset,
                                format!(
                                    "The {} element needs an end tag before </{name}>.",
                                    end_element.tag
                                ),
                            );
                        }
                        doc.elements[index].end = base + tag.span.end;
                        if matches!(doc.elements[index].tag.as_str(), "script" | "style") {
                            let start = content_starts[&index];
                            let value = source.text.get(start..offset).unwrap_or_default();
                            if doc.elements[index].tag == "script"
                                && doc.elements[index].attrs.get("type").is_some_and(|v| {
                                    v.eq_ignore_ascii_case("application/ld+json")
                                        || v.eq_ignore_ascii_case("application/json")
                                })
                            {
                                doc.code_blocks.push(CodeBlock {
                                    language: Some("json".into()),
                                    info: "embedded".into(),
                                    value: value.into(),
                                    offset: doc.elements[index].offset,
                                    content_offset: start,
                                });
                            } else if doc.elements[index].tag == "style" {
                                css_references(value, start, source, &mut doc);
                            }
                            doc.opaque_ranges.push(start..offset);
                        }
                    }
                    foreign.truncate(stack.len());
                } else if config.html.conformance {
                    issue(
                        &mut doc,
                        source,
                        "html.structure",
                        offset,
                        format!("The </{name}> tag has no open element."),
                    );
                }
            }
            Token::String(value) => {
                let text = string(&value);
                if !text.trim().is_empty()
                    && !stack.iter().any(|i| {
                        matches!(
                            doc.elements[*i].tag.as_str(),
                            "h1" | "h2"
                                | "h3"
                                | "h4"
                                | "h5"
                                | "h6"
                                | "head"
                                | "script"
                                | "style"
                                | "template"
                        )
                    })
                {
                    doc.content_ranges
                        .push(base + value.span.start..base + value.span.end);
                }
                if !stack.iter().any(|i| {
                    matches!(
                        doc.elements[*i].tag.as_str(),
                        "template" | "script" | "style"
                    )
                }) {
                    for index in &stack {
                        doc.elements[*index].text.push_str(&text);
                    }
                }
            }
            Token::Comment(_) => {}
        }
    }
    for index in stack {
        let end_element = doc.elements[index].clone();
        doc.elements[index].end = base + input.len();
        if config.html.conformance && !may_omit_at_parent_end(&doc.elements, index) {
            issue(
                &mut doc,
                source,
                "html.structure",
                end_element.offset,
                format!("The {} element needs an end tag.", end_element.tag),
            );
        }
    }
    let document = !fragment
        && match config.html.mode {
            HtmlMode::Document => true,
            HtmlMode::Fragment => false,
            HtmlMode::Auto => !doctypes.is_empty() || doc.elements.iter().any(|e| e.tag == "html"),
        };
    let tree = if document {
        Html::parse_document(input)
    } else {
        Html::parse_fragment(input)
    };
    if config.html.conformance {
        // Tree errors have no source span. Do not invent a precise token location.
        for error in &tree.errors {
            issue(
                &mut doc,
                source,
                "html.syntax",
                base,
                format!("HTML tree parse error (document start location): {error}"),
            );
        }
        if document {
            if doctypes.len() != 1 {
                issue(
                    &mut doc,
                    source,
                    "html.document",
                    base,
                    "A complete HTML page needs one HTML doctype.",
                );
            }
            for (offset, name, quirks, public, system) in doctypes {
                if name != "html"
                    || quirks
                    || public
                    || system.is_some_and(|s| s != "about:legacy-compat")
                {
                    issue(
                        &mut doc,
                        source,
                        "html.document",
                        offset,
                        "Use an HTML doctype without obsolete identifiers.",
                    );
                }
            }
            let titles: Vec<_> = tree
                .select(&Selector::parse("head > title").unwrap())
                .collect();
            if titles.len() != 1 || titles[0].text().collect::<String>().trim().is_empty() {
                issue(
                    &mut doc,
                    source,
                    "html.document",
                    base,
                    "A complete HTML page needs one nonempty title in its head.",
                );
            }
            for name in ["html", "head", "body"] {
                if doc.elements.iter().filter(|e| e.tag == name).count() > 1 {
                    issue(
                        &mut doc,
                        source,
                        "html.document",
                        base,
                        format!("A complete HTML page cannot repeat the {name} start tag."),
                    );
                }
            }
        }
    }
    if config.html.conformance {
        element_groups(source, &mut doc);
    }
    associations(source, config, &mut doc);
    if document
        && config.html.require_lang
        && !tree
            .root_element()
            .value()
            .attr("lang")
            .is_some_and(|v| !v.is_empty())
    {
        issue(
            &mut doc,
            source,
            "html.language",
            base,
            "The document html element needs a lang attribute.",
        );
    }
    if document
        && config.html.require_main
        && !doc.elements.iter().any(|e| {
            e.tag == "main"
                || e.attrs
                    .get("role")
                    .is_some_and(|v| v.split_ascii_whitespace().any(|t| t == "main"))
        })
    {
        issue(
            &mut doc,
            source,
            "html.landmarks",
            base,
            "The document needs a main landmark.",
        );
    }
    for index in 0..doc.elements.len() {
        let element = doc.elements[index].clone();
        if !ancestors(&doc.elements, index).any(|i| doc.elements[i].tag == "template") {
            extract(source, config, &element, &mut doc);
        }
        if !ancestors(&doc.elements, index).any(|i| doc.elements[i].tag == "template")
            && let Some(level) = element
                .tag
                .strip_prefix('h')
                .and_then(|s| s.parse::<u8>().ok())
                .filter(|n| (1..=6).contains(n))
        {
            doc.headings.push(Heading {
                level,
                text: element.text.trim().into(),
                id: element.attrs.get("id").cloned(),
                offset: element.offset,
                end: element.end,
            });
        }
    }
    doc
}

fn check_element(
    doc: &mut ParsedDocument,
    source: &Source,
    e: &HtmlElement,
    stack: &[usize],
    foreign: bool,
    config: &Config,
) {
    if e.attrs.iter().any(|(name, v)| {
        dynamic(v)
            && ((config.html.conformance
                && matches!(
                    name.as_str(),
                    "id" | "type"
                        | "name"
                        | "lang"
                        | "dir"
                        | "form"
                        | "list"
                        | "for"
                        | "headers"
                        | "usemap"
                        | "scope"
                        | "rowspan"
                        | "colspan"
                        | "width"
                        | "height"
                ))
                || name.starts_with("aria-")
                || (config.html.accessibility && matches!(name.as_str(), "alt" | "title")))
    }) {
        uncertain(
            doc,
            source,
            e.offset,
            "An HTML attribute has a template expression. Its static value cannot be checked.",
        );
    }
    if e.attrs.iter().any(|(name, value)| {
        dynamic(value)
            && (matches!(name.as_str(), "srcset" | "imagesrcset")
                || (name == "style" && config.assets.validate_css))
    }) {
        uncertain(
            doc,
            source,
            e.offset,
            "A template expression prevents checks of HTML image candidates or inline CSS references.",
        );
    }
    if e.attrs.get("id").is_some_and(|v| dynamic(v)) {
        doc.dynamic_anchors = true;
    }
    if let Some(lang) = e.attrs.get("lang")
        && !lang.is_empty()
        && !dynamic(lang)
        && language_tags::LanguageTag::parse(lang).is_err()
    {
        issue(
            doc,
            source,
            "html.language",
            e.attr_offsets["lang"],
            "The lang attribute is not a valid language identifier.",
        );
    }
    if !config.html.conformance || foreign {
        return;
    }
    const ELEMENTS: &str = "a abbr address area article aside audio b base bdi bdo blockquote body br button canvas caption cite code col colgroup data datalist dd del details dfn dialog div dl dt em embed fieldset figcaption figure footer form h1 h2 h3 h4 h5 h6 head header hgroup hr html i iframe img input ins kbd label legend li link main map mark menu meta meter nav noscript object ol optgroup option output p picture pre progress q rp rt ruby s samp script search section select selectedcontent slot small source span strong style sub summary sup table tbody td template textarea tfoot th thead time title tr track u ul var video wbr svg math";
    const OBSOLETE: &str = "acronym applet basefont bgsound big blink center dir font frame frameset isindex keygen listing marquee menuitem multicol nextid nobr noembed noframes param plaintext rb rtc spacer strike tt xmp";
    let obsolete = OBSOLETE.split_ascii_whitespace().any(|t| t == e.tag);
    if config.html.obsolete_elements && obsolete {
        issue(
            doc,
            source,
            "html.structure",
            e.offset,
            format!("The {} element is obsolete.", e.tag),
        );
    }
    if !ELEMENTS.split_ascii_whitespace().any(|t| t == e.tag)
        && !obsolete
        && !config.html.allowed_elements.contains(&e.tag)
        && !(config.html.allow_custom_elements && custom_name(&e.tag))
    {
        issue(
            doc,
            source,
            "html.structure",
            e.offset,
            format!(
                "The {} name is not a supported HTML element or a valid custom element name.",
                e.tag
            ),
        );
    }
    let parent_name = e.parent.map(|i| doc.elements[i].tag.clone());
    let parent = parent_name.as_deref();
    let allowed = match e.tag.as_str() {
        "li" => parent.is_some_and(|p| matches!(p, "ul" | "ol" | "menu")),
        "dt" | "dd" => {
            parent == Some("dl")
                || (parent == Some("div")
                    && e.parent
                        .and_then(|i| doc.elements[i].parent)
                        .is_some_and(|i| doc.elements[i].tag == "dl"))
        }
        "caption" | "colgroup" | "thead" | "tbody" | "tfoot" => parent == Some("table"),
        "col" => matches!(parent, Some("colgroup") | Some("table")),
        "tr" => parent.is_some_and(|p| matches!(p, "table" | "thead" | "tbody" | "tfoot")),
        "td" | "th" => parent == Some("tr"),
        "optgroup" => parent == Some("select"),
        "option" => parent.is_some_and(|p| matches!(p, "select" | "optgroup" | "datalist")),
        "source" => parent.is_some_and(|p| matches!(p, "picture" | "audio" | "video")),
        "track" => parent.is_some_and(|p| matches!(p, "audio" | "video")),
        "summary" => parent == Some("details"),
        "legend" => parent == Some("fieldset"),
        "figcaption" => parent == Some("figure"),
        _ => true,
    };
    if !allowed {
        issue(
            doc,
            source,
            "html.structure",
            e.offset,
            format!("The {} element has an invalid parent.", e.tag),
        );
    }
    if let Some(parent) = parent {
        let allowed_child = match parent {
            "ul" | "ol" | "menu" => matches!(e.tag.as_str(), "li" | "script" | "template"),
            "dl" => matches!(e.tag.as_str(), "dt" | "dd" | "div" | "script" | "template"),
            "tr" => matches!(e.tag.as_str(), "td" | "th" | "script" | "template"),
            "thead" | "tbody" | "tfoot" => matches!(e.tag.as_str(), "tr" | "script" | "template"),
            "colgroup" => matches!(e.tag.as_str(), "col" | "template"),
            "table" => matches!(
                e.tag.as_str(),
                "caption"
                    | "colgroup"
                    | "col"
                    | "thead"
                    | "tbody"
                    | "tfoot"
                    | "tr"
                    | "script"
                    | "template"
            ),
            "optgroup" => matches!(e.tag.as_str(), "option" | "script" | "template"),
            "head" => matches!(
                e.tag.as_str(),
                "title" | "base" | "link" | "meta" | "style" | "noscript" | "script" | "template"
            ),
            _ => true,
        };
        if !allowed_child {
            issue(
                doc,
                source,
                "html.structure",
                e.offset,
                format!("The {parent} element cannot contain a {} child.", e.tag),
            );
        }
    }
    for index in stack {
        let ancestor = &doc.elements[*index];
        if (e.tag == "form" && ancestor.tag == "form")
            || (e.tag == "a" && ancestor.tag == "a")
            || (ancestor.tag == "button" && interactive(e))
            || (ancestor.tag == "a"
                && ancestor.attrs.contains_key("href")
                && (interactive(e) || e.attrs.contains_key("tabindex")))
            || (e.tag == "label" && ancestor.tag == "label")
        {
            issue(
                doc,
                source,
                "html.structure",
                e.offset,
                format!(
                    "The {} element cannot be nested in this {} element.",
                    e.tag, ancestor.tag
                ),
            );
            break;
        }
    }
    let required: &[&str] = match e.tag.as_str() {
        "track" => &["src"],
        "optgroup" => &["label"],
        "map" => &["name"],
        "bdo" => &["dir"],
        "input"
            if e.attrs
                .get("type")
                .is_some_and(|t| t.eq_ignore_ascii_case("image")) =>
        {
            &["src", "alt"]
        }
        "link" => &["href"],
        "data" => &["value"],
        "area" if e.attrs.contains_key("href") => &["alt"],
        _ => &[],
    };
    if e.tag == "meta" {
        let names = ["name", "http-equiv", "charset", "itemprop"]
            .iter()
            .filter(|key| e.attrs.contains_key(**key))
            .count();
        if names != 1 {
            issue(
                doc,
                source,
                "html.attributes",
                e.offset,
                "A meta element needs one name, http-equiv, charset, or itemprop attribute.",
            );
        }
        if !e.attrs.contains_key("charset") && !e.attrs.contains_key("content") {
            issue(
                doc,
                source,
                "html.attributes",
                e.offset,
                "This meta element needs a content attribute.",
            );
        }
    }
    for attr in required {
        if !e.attrs.contains_key(*attr) {
            issue(
                doc,
                source,
                "html.attributes",
                e.offset,
                format!("The {} element needs a {attr} attribute.", e.tag),
            );
        }
    }
    if e.tag == "img" && !e.attrs.contains_key("src") && !e.attrs.contains_key("srcset") {
        issue(
            doc,
            source,
            "html.attributes",
            e.offset,
            "The img element needs src or srcset.",
        );
    }
    if e.tag == "source"
        && !e.attrs.contains_key(if parent == Some("picture") {
            "srcset"
        } else {
            "src"
        })
    {
        issue(
            doc,
            source,
            "html.attributes",
            e.offset,
            "The source element needs its media source attribute.",
        );
    }
    for (attr, value) in &e.attrs {
        if dynamic(value) {
            continue;
        }
        let offset = e.attr_offsets[attr];
        if attr == "id" && (value.is_empty() || value.chars().any(|c| c.is_ascii_whitespace())) {
            issue(
                doc,
                source,
                "html.attributes",
                offset,
                "An HTML id must be nonempty and cannot contain ASCII whitespace.",
            );
        }
        let tokens: Option<&[&str]> = match attr.as_str() {
            "dir" => Some(&["ltr", "rtl", "auto"]),
            "contenteditable" => Some(&["", "true", "false", "plaintext-only"]),
            "draggable" | "spellcheck" => Some(&["true", "false"]),
            "translate" => Some(&["", "yes", "no"]),
            "loading" if matches!(e.tag.as_str(), "img" | "iframe") => Some(&["eager", "lazy"]),
            "decoding" if e.tag == "img" => Some(&["sync", "async", "auto"]),
            "scope" if e.tag == "th" => Some(&["row", "col", "rowgroup", "colgroup"]),
            "method" if e.tag == "form" => Some(&["get", "post", "dialog"]),
            "type" if e.tag == "button" => Some(&["button", "submit", "reset"]),
            "type" if e.tag == "input" => Some(&[
                "hidden",
                "text",
                "search",
                "tel",
                "url",
                "email",
                "password",
                "date",
                "month",
                "week",
                "time",
                "datetime-local",
                "number",
                "range",
                "color",
                "checkbox",
                "radio",
                "file",
                "submit",
                "image",
                "reset",
                "button",
            ]),
            _ => None,
        };
        if let Some(tokens) = tokens
            && !tokens.iter().any(|t| value.eq_ignore_ascii_case(t))
        {
            issue(
                doc,
                source,
                "html.attributes",
                offset,
                format!("The {attr} attribute has an invalid value."),
            );
        }
        let boolean = match attr.as_str() {
            "disabled" => matches!(
                e.tag.as_str(),
                "button" | "fieldset" | "input" | "optgroup" | "option" | "select" | "textarea"
            ),
            "checked" => e.tag == "input",
            "selected" => e.tag == "option",
            "multiple" => matches!(e.tag.as_str(), "input" | "select"),
            "required" => matches!(e.tag.as_str(), "input" | "select" | "textarea"),
            "readonly" => matches!(e.tag.as_str(), "input" | "textarea"),
            "autofocus" | "inert" | "itemscope" => true,
            "controls" | "autoplay" | "loop" | "muted" => {
                matches!(e.tag.as_str(), "audio" | "video")
            }
            "open" => matches!(e.tag.as_str(), "details" | "dialog"),
            _ => false,
        };
        if boolean && !value.is_empty() && !value.eq_ignore_ascii_case(attr) {
            issue(
                doc,
                source,
                "html.attributes",
                offset,
                format!("The boolean {attr} attribute must be empty or contain its name."),
            );
        }
        if matches!(attr.as_str(), "colspan" | "rowspan") && matches!(e.tag.as_str(), "td" | "th") {
            let valid = !value.is_empty()
                && value.bytes().all(|b| b.is_ascii_digit())
                && value.parse::<u32>().is_ok_and(|n| {
                    if attr == "colspan" {
                        (1..=1000).contains(&n)
                    } else {
                        n <= 65534
                    }
                });
            if !valid {
                issue(
                    doc,
                    source,
                    "html.attributes",
                    offset,
                    format!("The {attr} attribute is outside its integer range."),
                );
            }
        }
        if matches!(attr.as_str(), "width" | "height")
            && matches!(
                e.tag.as_str(),
                "img" | "video" | "canvas" | "iframe" | "embed" | "object" | "input"
            )
            && (value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()))
        {
            issue(
                doc,
                source,
                "html.attributes",
                offset,
                format!("The {attr} attribute needs a nonnegative integer."),
            );
        }
    }
    if e.tag == "input" {
        let kind = e
            .attrs
            .get("type")
            .map_or("text", String::as_str)
            .to_ascii_lowercase();
        for attr in ["checked", "multiple", "accept", "capture", "src", "alt"] {
            let allowed = match attr {
                "checked" => matches!(kind.as_str(), "checkbox" | "radio"),
                "multiple" => matches!(kind.as_str(), "email" | "file"),
                "accept" | "capture" => kind == "file",
                "src" | "alt" => kind == "image",
                _ => true,
            };
            if !allowed && e.attrs.contains_key(attr) {
                issue(
                    doc,
                    source,
                    "html.attributes",
                    e.attr_offsets[attr],
                    format!("The {attr} attribute does not apply to input type {kind}."),
                );
            }
        }
    }
}
fn custom_name(name: &str) -> bool {
    if !name.starts_with(|c: char| c.is_ascii_lowercase())
        || !name.contains('-')
        || matches!(
            name,
            "annotation-xml"
                | "color-profile"
                | "font-face"
                | "font-face-src"
                | "font-face-uri"
                | "font-face-format"
                | "font-face-name"
                | "missing-glyph"
        )
    {
        return false;
    }
    name.chars().all(|c| matches!(c,'-'|'.'|'_'|'0'..='9'|'a'..='z'|'\u{B7}'|'\u{C0}'..='\u{D6}'|'\u{D8}'..='\u{F6}'|'\u{F8}'..='\u{37D}'|'\u{37F}'..='\u{1FFF}'|'\u{200C}'..='\u{200D}'|'\u{203F}'..='\u{2040}'|'\u{2070}'..='\u{218F}'|'\u{2C00}'..='\u{2FEF}'|'\u{3001}'..='\u{D7FF}'|'\u{F900}'..='\u{FDCF}'|'\u{FDF0}'..='\u{FFFD}'|'\u{10000}'..='\u{EFFFF}'))
}
fn interactive(e: &HtmlElement) -> bool {
    match e.tag.as_str() {
        "button" | "details" | "embed" | "iframe" | "label" | "select" | "textarea" => true,
        "a" => e.attrs.contains_key("href"),
        "audio" | "video" => e.attrs.contains_key("controls"),
        "input" => !e
            .attrs
            .get("type")
            .is_some_and(|v| v.eq_ignore_ascii_case("hidden")),
        "img" | "object" => e.attrs.contains_key("usemap"),
        _ => false,
    }
}

fn associations(source: &Source, config: &Config, doc: &mut ParsedDocument) {
    let elements = doc.elements.clone();
    let mut ids = BTreeMap::new();
    let mut form_names = BTreeSet::new();
    let mut map_names = BTreeSet::new();
    for (index, e) in elements.iter().enumerate() {
        if let Some(id) = e.attrs.get("id")
            && !dynamic(id)
            && !ancestors(&elements, index).any(|i| elements[i].tag == "template")
            && let Some(previous) = ids.insert(id.clone(), index)
        {
            issue(
                doc,
                source,
                "html.references",
                e.attr_offsets["id"],
                format!("The id {id:?} is used more than once."),
            );
            doc.diagnostics
                .last_mut()
                .unwrap()
                .related
                .push(source.location(elements[previous].attr_offsets["id"]));
        }
        if let Some(name) = e.attrs.get("name")
            && matches!(e.tag.as_str(), "form" | "map")
            && !dynamic(name)
        {
            let names = if e.tag == "form" {
                &mut form_names
            } else {
                &mut map_names
            };
            if name.is_empty()
                || (e.tag == "map" && name.chars().any(|c| c.is_ascii_whitespace()))
                || !names.insert(name.clone())
            {
                issue(
                    doc,
                    source,
                    "html.references",
                    e.attr_offsets["name"],
                    format!(
                        "The {} name must be nonempty, contain no spaces, and be unique.",
                        e.tag
                    ),
                );
            }
            if e.tag == "map" && e.attrs.get("id").is_some_and(|id| id != name) {
                issue(
                    doc,
                    source,
                    "html.references",
                    e.attr_offsets["name"],
                    "A map id and name must match.",
                );
            }
        }
    }
    for (index, e) in elements.iter().enumerate() {
        if ancestors(&elements, index).any(|i| elements[i].tag == "template") {
            continue;
        }
        for (attr, expected, list) in [
            (
                "for",
                if e.tag == "output" {
                    None
                } else {
                    Some("labelable")
                },
                e.tag == "output",
            ),
            ("form", Some("form"), false),
            ("list", Some("datalist"), false),
            ("headers", Some("th"), true),
            ("aria-activedescendant", None, false),
            ("aria-controls", None, true),
            ("aria-describedby", None, true),
            ("aria-details", None, false),
            ("aria-errormessage", None, false),
            ("aria-flowto", None, true),
            ("aria-labelledby", None, true),
            ("aria-owns", None, true),
        ] {
            let applies = match attr {
                "for" => matches!(e.tag.as_str(), "label" | "output"),
                "form" => matches!(
                    e.tag.as_str(),
                    "button" | "fieldset" | "input" | "object" | "output" | "select" | "textarea"
                ),
                "list" => e.tag == "input",
                "headers" => matches!(e.tag.as_str(), "td" | "th"),
                _ => true,
            };
            if !applies {
                continue;
            }
            if let Some(value) = e.attrs.get(attr)
                && !dynamic(value)
            {
                let values: Vec<_> = if list {
                    value.split_ascii_whitespace().collect()
                } else {
                    vec![value.as_str()]
                };
                if values.is_empty() && attr.starts_with("aria-") {
                    issue(
                        doc,
                        source,
                        "html.aria",
                        e.attr_offsets[attr],
                        format!("The {attr} attribute needs an identifier."),
                    );
                }
                for value in values {
                    let mut reference = Reference::new(
                        format!("#{value}"),
                        e.attr_offsets[attr],
                        ReferenceKind::Id,
                    );
                    reference.expected_tag = expected.map(str::to_owned);
                    doc.references.push(reference);
                    if attr == "headers"
                        && let Some(target) = ids.get(value)
                        && table_owner(&elements, index) != table_owner(&elements, *target)
                    {
                        issue(
                            doc,
                            source,
                            "html.references",
                            e.attr_offsets[attr],
                            "A headers reference must target a header in the same table.",
                        );
                    }
                }
            }
        }
        if let Some(value) = e.attrs.get("usemap")
            && matches!(e.tag.as_str(), "img" | "object")
            && !dynamic(value)
        {
            if !value.starts_with('#') || value.len() == 1 {
                issue(
                    doc,
                    source,
                    "html.references",
                    e.attr_offsets["usemap"],
                    "The usemap attribute needs a fragment that names a map.",
                );
            } else {
                let mut reference =
                    Reference::new(value, e.attr_offsets["usemap"], ReferenceKind::Id);
                reference.expected_tag = Some("map".into());
                doc.references.push(reference);
            }
        }
        if config.html.conformance {
            aria(e, source, doc);
        }
        if config.html.accessibility {
            if e.tag == "img" && !e.attrs.contains_key("alt") {
                issue(
                    doc,
                    source,
                    "html.accessibility",
                    e.offset,
                    "The img element needs an alt attribute. Use an empty value for a decorative image.",
                );
            }
            let input_type = e
                .attrs
                .get("type")
                .map_or("text", String::as_str)
                .to_ascii_lowercase();
            let labelable = matches!(
                e.tag.as_str(),
                "button" | "meter" | "output" | "progress" | "select" | "textarea"
            ) || (e.tag == "input" && input_type != "hidden");
            if labelable {
                let explicit_label = e.attrs.get("id").is_some_and(|id| {
                    elements.iter().any(|label| {
                        label.tag == "label"
                            && label.attrs.get("for") == Some(id)
                            && !label.text.trim().is_empty()
                    })
                });
                let ancestor_label = ancestors(&elements, index).any(|i| {
                    elements[i].tag == "label"
                        && (!elements[i].attrs.contains_key("for")
                            || elements[i].attrs.get("for") == e.attrs.get("id"))
                        && !elements[i].text.trim().is_empty()
                });
                let own_text = e.tag == "button" && !e.text.trim().is_empty();
                let input_value = e.tag == "input"
                    && matches!(input_type.as_str(), "submit" | "reset" | "button")
                    && (input_type != "button"
                        || e.attrs.get("value").is_some_and(|v| !v.trim().is_empty()));
                let image_alt = e.tag == "input"
                    && input_type == "image"
                    && e.attrs.get("alt").is_some_and(|v| !v.trim().is_empty());
                let attr_name = ["aria-label", "aria-labelledby", "title"]
                    .iter()
                    .any(|attr| e.attrs.get(*attr).is_some_and(|v| !v.trim().is_empty()));
                let nested_name = e.tag == "button"
                    && elements.iter().enumerate().any(|(child, c)| {
                        ancestors(&elements, child).any(|i| i == index)
                            && c.attrs.get("alt").is_some_and(|v| !v.trim().is_empty())
                    });
                if !(explicit_label
                    || ancestor_label
                    || own_text
                    || input_value
                    || image_alt
                    || attr_name
                    || nested_name)
                {
                    issue(
                        doc,
                        source,
                        "html.accessibility",
                        e.offset,
                        format!("The {} control has no supported static name source.", e.tag),
                    );
                }
            }
        }
    }
}
fn ancestors(elements: &[HtmlElement], index: usize) -> impl Iterator<Item = usize> + '_ {
    let mut next = elements[index].parent;
    std::iter::from_fn(move || {
        let current = next?;
        next = elements[current].parent;
        Some(current)
    })
}
fn table_owner(elements: &[HtmlElement], index: usize) -> Option<usize> {
    ancestors(elements, index).find(|i| elements[*i].tag == "table")
}
fn aria(e: &HtmlElement, source: &Source, doc: &mut ParsedDocument) {
    const ROLES: &str = "alert alertdialog application article banner blockquote button caption cell checkbox code columnheader combobox complementary contentinfo definition deletion dialog directory document emphasis feed figure form generic grid gridcell group heading img insertion link list listbox listitem log main marquee math menu menubar menuitem menuitemcheckbox menuitemradio meter navigation none note option paragraph presentation progressbar radio radiogroup region row rowgroup rowheader scrollbar search searchbox separator slider spinbutton status strong subscript suggestion superscript switch tab table tablist tabpanel term textbox time timer toolbar tooltip tree treegrid treeitem";
    const ATTRS: &str = "aria-activedescendant aria-atomic aria-autocomplete aria-braillelabel aria-brailleroledescription aria-busy aria-checked aria-colcount aria-colindex aria-colindextext aria-colspan aria-controls aria-current aria-describedby aria-description aria-details aria-disabled aria-dropeffect aria-errormessage aria-expanded aria-flowto aria-grabbed aria-haspopup aria-hidden aria-invalid aria-keyshortcuts aria-label aria-labelledby aria-level aria-live aria-modal aria-multiline aria-multiselectable aria-orientation aria-owns aria-placeholder aria-posinset aria-pressed aria-readonly aria-relevant aria-required aria-roledescription aria-rowcount aria-rowindex aria-rowindextext aria-rowspan aria-selected aria-setsize aria-sort aria-valuemax aria-valuemin aria-valuenow aria-valuetext";
    if let Some(value) = e.attrs.get("role")
        && !dynamic(value)
        && (value.split_ascii_whitespace().next().is_none()
            || !value
                .split_ascii_whitespace()
                .any(|r| ROLES.split_ascii_whitespace().any(|known| r == known)))
    {
        issue(
            doc,
            source,
            "html.aria",
            e.attr_offsets["role"],
            "The role attribute has no supported concrete role.",
        );
    }
    let role = e.attrs.get("role").and_then(|value| {
        value
            .split_ascii_whitespace()
            .find(|r| ROLES.split_ascii_whitespace().any(|known| *r == known))
    });
    if let Some(role) = role {
        let required: &[&str] = match role {
            "checkbox" | "menuitemcheckbox" | "menuitemradio" | "radio" | "switch" => {
                &["aria-checked"]
            }
            "slider" | "scrollbar" => &["aria-valuenow"],
            "heading" if !matches!(e.tag.as_str(), "h1" | "h2" | "h3" | "h4" | "h5" | "h6") => {
                &["aria-level"]
            }
            "combobox" => &["aria-expanded"],
            _ => &[],
        };
        for attr in required {
            let native = match *attr {
                "aria-checked" => {
                    e.tag == "input"
                        && e.attrs
                            .get("type")
                            .is_some_and(|t| matches!(t.as_str(), "checkbox" | "radio"))
                }
                "aria-valuenow" => {
                    e.tag == "input" && e.attrs.get("type").is_some_and(|t| t == "range")
                }
                "aria-expanded" => e.tag == "select",
                _ => false,
            };
            if !native && !e.attrs.contains_key(*attr) {
                issue(
                    doc,
                    source,
                    "html.aria",
                    e.offset,
                    format!("The {role} role needs the {attr} attribute."),
                );
            }
        }
        for (attr, roles) in [
            (
                "aria-checked",
                &[
                    "checkbox",
                    "menuitemcheckbox",
                    "menuitemradio",
                    "radio",
                    "switch",
                ] as &[&str],
            ),
            (
                "aria-selected",
                &["gridcell", "option", "row", "tab", "treeitem"],
            ),
            ("aria-pressed", &["button"]),
        ] {
            if e.attrs.contains_key(attr) && !roles.contains(&role) {
                issue(
                    doc,
                    source,
                    "html.aria",
                    e.attr_offsets[attr],
                    format!("The {attr} attribute does not apply to the {role} role."),
                );
            }
        }
    }
    for (attr, value) in &e.attrs {
        if !attr.starts_with("aria-") || dynamic(value) {
            continue;
        }
        if !ATTRS.split_ascii_whitespace().any(|known| known == attr) {
            issue(
                doc,
                source,
                "html.aria",
                e.attr_offsets[attr],
                format!("The {attr} attribute is not a supported ARIA attribute."),
            );
            continue;
        }
        let choices: Option<&[&str]> = match attr.as_str() {
            "aria-atomic"
            | "aria-busy"
            | "aria-disabled"
            | "aria-modal"
            | "aria-multiline"
            | "aria-multiselectable"
            | "aria-readonly"
            | "aria-required" => Some(&["true", "false"]),
            "aria-hidden" | "aria-expanded" | "aria-selected" | "aria-grabbed" => {
                Some(&["true", "false", "undefined"])
            }
            "aria-checked" | "aria-pressed" => Some(&["true", "false", "mixed", "undefined"]),
            "aria-autocomplete" => Some(&["none", "inline", "list", "both"]),
            "aria-live" => Some(&["off", "polite", "assertive"]),
            "aria-orientation" => Some(&["horizontal", "vertical", "undefined"]),
            "aria-sort" => Some(&["none", "ascending", "descending", "other"]),
            "aria-invalid" => Some(&["false", "true", "grammar", "spelling"]),
            "aria-haspopup" => {
                Some(&["false", "true", "menu", "listbox", "tree", "grid", "dialog"])
            }
            _ => None,
        };
        if let Some(choices) = choices
            && !choices.contains(&value.as_str())
        {
            issue(
                doc,
                source,
                "html.aria",
                e.attr_offsets[attr],
                format!("The {attr} attribute has an invalid token value."),
            );
        }
        let integer = match attr.as_str() {
            "aria-colcount" | "aria-rowcount" | "aria-setsize" => Some(-1),
            "aria-colindex" | "aria-rowindex" | "aria-colspan" | "aria-rowspan" | "aria-level"
            | "aria-posinset" => Some(1),
            _ => None,
        };
        if let Some(minimum) = integer
            && !value.parse::<i64>().is_ok_and(|n| n >= minimum)
        {
            issue(
                doc,
                source,
                "html.aria",
                e.attr_offsets[attr],
                format!("The {attr} attribute needs an integer in its allowed range."),
            );
        }
        if matches!(
            attr.as_str(),
            "aria-valuenow" | "aria-valuemin" | "aria-valuemax"
        ) && !value.parse::<f64>().is_ok_and(f64::is_finite)
        {
            issue(
                doc,
                source,
                "html.aria",
                e.attr_offsets[attr],
                format!("The {attr} attribute needs a finite number."),
            );
        }
        if attr == "aria-relevant"
            && (value.split_ascii_whitespace().next().is_none()
                || !value
                    .split_ascii_whitespace()
                    .all(|t| matches!(t, "additions" | "removals" | "text" | "all")))
        {
            issue(
                doc,
                source,
                "html.aria",
                e.attr_offsets[attr],
                "The aria-relevant attribute has invalid tokens.",
            );
        }
    }
}

fn extract(source: &Source, config: &Config, e: &HtmlElement, doc: &mut ParsedDocument) {
    for attr in ["id", "name"] {
        if attr == "name" && !matches!(e.tag.as_str(), "a" | "map") {
            continue;
        }
        if let Some(id) = e.attrs.get(attr)
            && !id.is_empty()
            && !dynamic(id)
            && !(attr == "name" && e.attrs.get("id") == Some(id))
        {
            doc.anchors.push(Anchor {
                id: id.clone(),
                offset: e.attr_offsets[attr],
                tag: e.tag.clone(),
            });
        }
    }
    if e.tag == "meta" {
        let charset = e.attrs.get("charset").cloned().or_else(|| {
            if !e
                .attrs
                .get("http-equiv")
                .is_some_and(|v| v.eq_ignore_ascii_case("content-type"))
            {
                return None;
            }
            e.attrs.get("content").and_then(|v| {
                v.split(';').find_map(|part| {
                    let (key, value) = part.trim().split_once('=')?;
                    key.trim()
                        .eq_ignore_ascii_case("charset")
                        .then(|| value.trim().trim_matches(['\'', '"']).into())
                })
            })
        });
        if let Some(charset) = charset
            && !dynamic(&charset)
        {
            let declared = encoding_rs::Encoding::for_label(charset.as_bytes());
            let actual = encoding_rs::Encoding::for_label(config.files.html_encoding.as_bytes());
            if declared.is_none() || declared != actual {
                issue(
                    doc,
                    source,
                    "html.encoding",
                    e.offset,
                    format!(
                        "The HTML encoding declaration {charset:?} does not match the configured input encoding."
                    ),
                );
            }
        }
    }
    for attr in [
        "href",
        "xlink:href",
        "src",
        "poster",
        "data",
        "action",
        "formaction",
        "cite",
        "longdesc",
    ] {
        let applies = match attr {
            "href" => matches!(
                e.tag.as_str(),
                "a" | "area" | "link" | "base" | "use" | "image" | "feimage" | "textpath" | "mpath"
            ),
            "xlink:href" => matches!(
                e.tag.as_str(),
                "use" | "image" | "feimage" | "a" | "textpath" | "mpath"
            ),
            "src" => matches!(
                e.tag.as_str(),
                "img"
                    | "script"
                    | "iframe"
                    | "embed"
                    | "audio"
                    | "video"
                    | "source"
                    | "track"
                    | "input"
            ),
            "poster" => e.tag == "video",
            "data" => e.tag == "object",
            "action" => e.tag == "form",
            "formaction" => matches!(e.tag.as_str(), "button" | "input"),
            "cite" => matches!(e.tag.as_str(), "blockquote" | "q" | "del" | "ins"),
            "longdesc" => matches!(e.tag.as_str(), "img" | "iframe"),
            _ => false,
        };
        if !applies {
            continue;
        }
        if let Some(target) = e.attrs.get(attr) {
            if e.tag == "base" {
                continue;
            }
            let image = matches!(e.tag.as_str(), "img" | "image" | "feimage")
                || attr == "poster"
                || (e.tag == "input"
                    && e.attrs
                        .get("type")
                        .is_some_and(|v| v.eq_ignore_ascii_case("image")));
            let rel = e.attrs.get("rel").map_or("", String::as_str);
            let asset = image
                || attr == "src"
                || attr == "data"
                || attr == "xlink:href"
                || e.tag == "use"
                || (e.tag == "link"
                    && rel.split_ascii_whitespace().any(|t| {
                        matches!(
                            t.to_ascii_lowercase().as_str(),
                            "stylesheet" | "icon" | "preload" | "modulepreload"
                        )
                    }));
            let kind = if image {
                ReferenceKind::Image
            } else if asset {
                ReferenceKind::Asset
            } else if e.tag == "link"
                && rel
                    .split_ascii_whitespace()
                    .any(|t| t.eq_ignore_ascii_case("canonical"))
            {
                ReferenceKind::Canonical
            } else if e.tag == "link"
                && rel
                    .split_ascii_whitespace()
                    .any(|t| t.eq_ignore_ascii_case("alternate"))
            {
                ReferenceKind::Alternate
            } else {
                ReferenceKind::Link
            };
            let mut reference = Reference::new(target, e.attr_offsets[attr], kind);
            reference.integrity = e.attrs.get("integrity").cloned();
            reference.expected_tag = if image {
                Some("image")
            } else if e.tag == "link"
                && rel
                    .split_ascii_whitespace()
                    .any(|t| t.eq_ignore_ascii_case("stylesheet"))
            {
                Some("stylesheet")
            } else {
                match e.tag.as_str() {
                    "script" => Some("script"),
                    "audio" => Some("audio"),
                    "video" => Some("video"),
                    "track" => Some("caption"),
                    "source" => e.parent.and_then(|i| match doc.elements[i].tag.as_str() {
                        "picture" => Some("image"),
                        "audio" => Some("audio"),
                        "video" => Some("video"),
                        _ => None,
                    }),
                    _ => None,
                }
            }
            .map(str::to_owned);
            if image {
                reference.width = e.attrs.get("width").and_then(|s| s.parse().ok());
                reference.height = e.attrs.get("height").and_then(|s| s.parse().ok());
            }
            doc.references.push(reference);
        }
    }
    if matches!(e.tag.as_str(), "img" | "source" | "link") {
        let attr = if e.tag == "link" {
            "imagesrcset"
        } else {
            "srcset"
        };
        if let Some(value) = e.attrs.get(attr)
            && !dynamic(value)
        {
            srcset(value, e.attr_offsets[attr], source, doc);
        }
    }
    if let Some(value) = e.attrs.get("style")
        && !dynamic(value)
    {
        css_references(value, e.attr_offsets["style"], source, doc);
    }
    if e.tag == "a" && e.attrs.get("ping").is_some_and(|v| !dynamic(v)) {
        for url in e.attrs["ping"].split_ascii_whitespace() {
            doc.references.push(Reference::new(
                url,
                e.attr_offsets["ping"],
                ReferenceKind::Link,
            ));
        }
    }
}

/// Read candidates with the HTML state rules. A data URL can contain commas.
fn srcset(value: &str, offset: usize, source: &Source, doc: &mut ParsedDocument) {
    let bytes = value.as_bytes();
    let mut pos = 0;
    let mut descriptors = BTreeSet::new();
    let mut kinds = BTreeSet::new();
    while pos < bytes.len() {
        while pos < bytes.len() && (bytes[pos].is_ascii_whitespace() || bytes[pos] == b',') {
            pos += 1;
        }
        if pos == bytes.len() {
            break;
        }
        let start = pos;
        while pos < bytes.len() && !bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        let mut end = pos;
        while end > start && bytes[end - 1] == b',' {
            end -= 1;
        }
        let url = &value[start..end];
        let mut parts = Vec::new();
        if end == pos {
            let mut part_start = None;
            let mut parentheses = 0;
            while pos < bytes.len() {
                let byte = bytes[pos];
                if byte == b'(' {
                    parentheses += 1;
                }
                if byte == b')' && parentheses > 0 {
                    parentheses -= 1;
                }
                if parentheses == 0 && (byte.is_ascii_whitespace() || byte == b',') {
                    if let Some(begin) = part_start.take() {
                        parts.push(&value[begin..pos]);
                    }
                    pos += 1;
                    if byte == b',' {
                        break;
                    }
                } else {
                    part_start.get_or_insert(pos);
                    pos += 1;
                }
            }
            if let Some(begin) = part_start {
                parts.push(&value[begin..pos]);
            }
        }
        let mut width = None;
        let mut density = None;
        let mut height = None;
        let mut valid = true;
        for part in parts {
            if let Some(number) = part.strip_suffix('w') {
                if width.is_some()
                    || density.is_some()
                    || number.is_empty()
                    || !number.bytes().all(|b| b.is_ascii_digit())
                    || !number.parse::<u64>().is_ok_and(|n| n > 0)
                {
                    valid = false;
                } else {
                    width = number.parse::<u64>().ok();
                }
            } else if let Some(number) = part.strip_suffix('x') {
                if density.is_some()
                    || width.is_some()
                    || height.is_some()
                    || number.starts_with('+')
                    || !number
                        .parse::<f64>()
                        .is_ok_and(|n| n.is_finite() && n > 0.0)
                {
                    valid = false;
                } else {
                    density = number.parse::<f64>().ok();
                }
            } else if let Some(number) = part.strip_suffix('h') {
                if height.is_some()
                    || density.is_some()
                    || number.is_empty()
                    || !number.bytes().all(|b| b.is_ascii_digit())
                    || !number.parse::<u64>().is_ok_and(|n| n > 0)
                {
                    valid = false;
                } else {
                    height = number.parse::<u64>().ok();
                }
            } else {
                valid = false;
            }
        }
        if height.is_some() {
            valid = false;
        }
        let key = if let Some(width) = width {
            kinds.insert('w');
            format!("{width}w")
        } else {
            kinds.insert('x');
            format!("{}x", density.unwrap_or(1.0))
        };
        if !descriptors.insert(key) {
            valid = false;
        }
        if !valid {
            issue(
                doc,
                source,
                "html.srcset",
                offset + start,
                "The srcset candidate has invalid or repeated descriptors.",
            );
        }
        if !url.is_empty() {
            doc.references
                .push(Reference::new(url, offset + start, ReferenceKind::Image));
        }
    }
    if kinds.len() > 1 {
        issue(
            doc,
            source,
            "html.srcset",
            offset,
            "The srcset attribute mixes width and density descriptors.",
        );
    }
}

fn css_references(value: &str, offset: usize, source: &Source, doc: &mut ParsedDocument) {
    let (mut references, errors) = crate::assets::css_references(value);
    for reference in &mut references {
        reference.offset += offset;
    }
    doc.references.extend(references);
    for (position, outcome, message) in errors {
        let rule = if outcome == Outcome::Invalid {
            "html.attributes"
        } else {
            "asset.available"
        };
        let requirement = if outcome == Outcome::Invalid {
            Requirement::Format
        } else {
            Requirement::Execution
        };
        doc.diagnostics.push(source.diagnostic(
            rule,
            requirement,
            outcome,
            offset + position,
            message,
        ));
        doc.incomplete |= outcome == Outcome::Unverified;
    }
}

fn element_groups(source: &Source, doc: &mut ParsedDocument) {
    let elements = doc.elements.clone();
    for (index, e) in elements.iter().enumerate() {
        let children: Vec<_> = elements
            .iter()
            .enumerate()
            .filter(|(_, child)| {
                child.parent == Some(index) && !matches!(child.tag.as_str(), "script" | "template")
            })
            .collect();
        match e.tag.as_str() {
            "label" => {
                let controls: Vec<_> = elements
                    .iter()
                    .enumerate()
                    .filter(|(i, c)| {
                        ancestors(&elements, *i).any(|ancestor| ancestor == index)
                            && (matches!(
                                c.tag.as_str(),
                                "button" | "meter" | "output" | "progress" | "select" | "textarea"
                            ) || (c.tag == "input"
                                && !c
                                    .attrs
                                    .get("type")
                                    .is_some_and(|t| t.eq_ignore_ascii_case("hidden"))))
                    })
                    .collect();
                if controls.len() > 1
                    || controls.first().is_some_and(|(_, control)| {
                        e.attrs.contains_key("for") && e.attrs.get("for") != control.attrs.get("id")
                    })
                {
                    issue(
                        doc,
                        source,
                        "html.structure",
                        e.offset,
                        "A label can contain only the control that it labels.",
                    );
                }
            }
            "details" => {
                let summaries = children.iter().filter(|(_, c)| c.tag == "summary").count();
                if summaries > 1
                    || (summaries == 1 && children.first().is_some_and(|(_, c)| c.tag != "summary"))
                {
                    issue(
                        doc,
                        source,
                        "html.structure",
                        e.offset,
                        "A details element can have one summary, as its first element child.",
                    );
                }
            }
            "figure" => {
                let captions: Vec<_> = children
                    .iter()
                    .enumerate()
                    .filter(|(_, (_, c))| c.tag == "figcaption")
                    .collect();
                if captions.len() > 1
                    || captions.first().is_some_and(|(position, _)| {
                        *position != 0 && *position + 1 != children.len()
                    })
                {
                    issue(
                        doc,
                        source,
                        "html.structure",
                        e.offset,
                        "A figure can have one figcaption, as its first or last element child.",
                    );
                }
            }
            "table" => {
                let mut phase = 0;
                let mut seen = BTreeSet::new();
                for (_, child) in &children {
                    let order = match child.tag.as_str() {
                        "caption" => 0,
                        "col" | "colgroup" => 1,
                        "thead" => 2,
                        "tbody" | "tr" => 3,
                        "tfoot" => 4,
                        _ => continue,
                    };
                    if order < phase
                        || (matches!(child.tag.as_str(), "caption" | "thead" | "tfoot")
                            && !seen.insert(child.tag.as_str()))
                    {
                        issue(
                            doc,
                            source,
                            "html.structure",
                            child.offset,
                            "The table child order or occurrence is invalid.",
                        );
                    }
                    phase = phase.max(order);
                }
                table_grid(source, &elements, index, doc);
            }
            "dl" => {
                let mut has_term = false;
                let mut has_description = false;
                for (_, child) in children {
                    let group: Vec<_> = if child.tag == "div" {
                        elements
                            .iter()
                            .filter(|c| {
                                c.parent.is_some_and(|i| elements[i].offset == child.offset)
                            })
                            .collect()
                    } else {
                        vec![child]
                    };
                    for child in group {
                        if child.tag == "dt" {
                            has_term = true;
                            has_description = false;
                        } else if child.tag == "dd" {
                            if !has_term {
                                issue(
                                    doc,
                                    source,
                                    "html.structure",
                                    child.offset,
                                    "A definition description needs a preceding term in its group.",
                                );
                            }
                            has_description = true;
                        }
                    }
                }
                if has_term && !has_description {
                    issue(
                        doc,
                        source,
                        "html.structure",
                        e.offset,
                        "The final definition term group needs a description.",
                    );
                }
            }
            _ => {}
        }
    }
}
fn table_grid(source: &Source, elements: &[HtmlElement], table: usize, doc: &mut ParsedDocument) {
    let rows: Vec<_> = elements
        .iter()
        .enumerate()
        .filter(|(i, e)| e.tag == "tr" && table_owner(elements, *i) == Some(table))
        .collect();
    let mut occupied: BTreeMap<usize, usize> = BTreeMap::new();
    let mut last_group = None;
    for (row_index, (index, row)) in rows.iter().enumerate() {
        let group = row.parent;
        if group != last_group {
            occupied.clear();
            last_group = group;
        }
        occupied.retain(|_, end| *end > row_index);
        let mut column = 0usize;
        for (_, cell) in elements
            .iter()
            .enumerate()
            .filter(|(_, e)| e.parent == Some(*index) && matches!(e.tag.as_str(), "td" | "th"))
        {
            if ["rowspan", "colspan"]
                .iter()
                .any(|a| cell.attrs.get(*a).is_some_and(|v| dynamic(v)))
            {
                continue;
            }
            while occupied.contains_key(&column) {
                column += 1;
            }
            let width = cell
                .attrs
                .get("colspan")
                .and_then(|v| v.parse::<usize>().ok())
                .filter(|n| (1..=1000).contains(n))
                .unwrap_or(1);
            let span = cell
                .attrs
                .get("rowspan")
                .and_then(|v| v.parse::<usize>().ok())
                .filter(|n| *n <= 65534)
                .unwrap_or(1);
            let end = if span == 0 {
                usize::MAX
            } else {
                row_index + span
            };
            for col in column..column + width {
                if occupied.insert(col, end).is_some() {
                    issue(
                        doc,
                        source,
                        "html.attributes",
                        cell.offset,
                        "A table cell overlaps an earlier row span.",
                    );
                    break;
                }
            }
            column += width;
            if column > 100_000 {
                doc.incomplete = true;
                doc.diagnostics.push(source.diagnostic(
                    "html.attributes",
                    Requirement::Execution,
                    Outcome::Unverified,
                    cell.offset,
                    "The table grid exceeds the reader column limit.",
                ));
                return;
            }
        }
    }
}

/// Check static element records from a reader that supplies its own syntax rules.
pub fn validate_elements(source: &Source, config: &Config, doc: &mut ParsedDocument) {
    let elements = doc.elements.clone();
    for (index, e) in elements.iter().enumerate() {
        let mut stack: Vec<_> = ancestors(&elements, index).collect();
        stack.reverse();
        let foreign = matches!(e.tag.as_str(), "svg" | "math")
            || stack
                .iter()
                .rev()
                .take_while(|i| {
                    !matches!(
                        elements[**i].tag.as_str(),
                        "foreignobject" | "annotation-xml"
                    )
                })
                .any(|i| matches!(elements[*i].tag.as_str(), "svg" | "math"));
        check_element(doc, source, e, &stack, foreign, config);
    }
    if config.html.conformance {
        element_groups(source, doc);
    }
    associations(source, config, doc);
}

#[cfg(test)]
mod tests {
    use super::*;
    fn read_html(text: &str) -> ParsedDocument {
        parse(
            &Source::new("test.html".into(), "test.html".into(), text.into()),
            &Config::default(),
        )
    }
    fn errors(text: &str) -> Vec<String> {
        read_html(text)
            .diagnostics
            .into_iter()
            .map(|d| format!("{}: {}", d.rule, d.message))
            .collect()
    }
    #[test]
    fn optional_tags_and_raw_text() {
        let source = "<!doctype html><title>Example</title><ul><li>One<li>Two</ul><p>Three<p>Four<script>let x = '<img src=missing>';</script>";
        assert!(errors(source).is_empty(), "{:?}", errors(source));
        assert!(read_html(source).references.is_empty());
        assert!(
            errors("<div><span>Text</div>")
                .iter()
                .any(|e| e.contains("end tag"))
        );
        assert!(errors("<div/>").iter().any(|e| e.contains("self-closing")));
    }
    #[test]
    fn token_errors_and_unicode_locations() {
        let source = Source::new(
            "test.html".into(),
            "test.html".into(),
            "é\n<a href='a&amp;b' href=other>link</a>".into(),
        );
        let doc = parse(&source, &Config::default());
        assert_eq!(doc.references[0].target, "a&b");
        let duplicate = doc
            .diagnostics
            .iter()
            .find(|d| d.message.contains("duplicate-attribute"))
            .unwrap();
        assert_eq!(duplicate.location.line, 2);
        assert!(source.text.is_char_boundary(duplicate.location.byte_offset));
        assert_eq!(doc.elements[0].offset, 3);
    }
    #[test]
    fn recovered_nesting_is_not_valid() {
        for source in [
            "<form><form></form></form>",
            "<button><a href='/'>Bad</a></button>",
            "<ul><div>Bad</div></ul>",
            "<table><div>Bad</div></table>",
            "<p><div>Bad</div></p>",
        ] {
            assert!(!errors(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn associations_and_decorative_images() {
        let doc = read_html(
            "<label for='n'>Name</label><input id=n><form id=f></form><input aria-label='Other' form=f list=items><datalist id=items><option value=a></datalist><img src=x alt=''>",
        );
        assert!(doc.diagnostics.is_empty(), "{:?}", doc.diagnostics);
        assert!(
            doc.references
                .iter()
                .any(|r| r.target == "#n" && r.expected_tag.as_deref() == Some("labelable"))
        );
        assert!(
            errors("<img src=x>")
                .iter()
                .any(|e| e.contains("alt attribute"))
        );
        assert!(
            errors("<div id=x></div><span id=x></span>")
                .iter()
                .any(|e| e.contains("more than once"))
        );
        assert!(
            errors("<input type=hidden checked>")
                .iter()
                .any(|e| e.contains("does not apply"))
        );
    }
    #[test]
    fn namespace_ids_and_custom_names() {
        let doc = read_html(
            "<svg><symbol id='icon'><path d='M0 0'/></symbol><use href='#icon'/></svg><x-café></x-café>",
        );
        assert!(doc.diagnostics.is_empty(), "{:?}", doc.diagnostics);
        assert!(
            doc.anchors
                .iter()
                .any(|a| a.id == "icon" && a.tag == "symbol")
        );
        assert!(
            errors("<font-face></font-face>")
                .iter()
                .any(|e| e.contains("name is not"))
        );
    }
    #[test]
    fn srcset_data_and_descriptor_rules() {
        let doc = read_html("<img alt='' srcset='data:image/png;base64,AAAA 1x, large.png 2x'>");
        assert!(doc.diagnostics.is_empty(), "{:?}", doc.diagnostics);
        assert_eq!(doc.references[0].target, "data:image/png;base64,AAAA");
        for value in [
            "a.png 1x 2x",
            "a.png 0w",
            "a.png 1w, b.png 2x",
            "a.png 2x, b.png 2x",
        ] {
            assert!(
                read_html(&format!("<img alt='' srcset='{value}'>"))
                    .diagnostics
                    .iter()
                    .any(|d| d.rule == "html.srcset"),
                "{value}"
            );
        }
    }
    #[test]
    fn fragments_keep_original_offsets_and_ignore_code() {
        let text = "`<img src=bad>`\n<div id=good>\nText\n</div>\n";
        let source = Source::new("test.md".into(), "test.md".into(), text.into());
        let begin = text.find("<div").unwrap();
        let end = text.find("</div>").unwrap() + 6;
        let range = begin..end;
        let doc = parse_fragment(&source, std::slice::from_ref(&range), &Config::default());
        assert!(doc.diagnostics.is_empty(), "{:?}", doc.diagnostics);
        assert_eq!(doc.elements[0].offset, begin);
        assert_eq!(doc.elements[0].text.trim(), "Text");
        assert!(doc.references.is_empty());
    }
    #[test]
    fn embedded_json_and_css_have_content_spans() {
        let text = "<script type='application/ld+json'>\n{\"x\":1}\n</script><style>@import 'theme.css';x{background:url(\"a.png\")}</style>";
        let doc = read_html(text);
        assert!(doc.diagnostics.is_empty(), "{:?}", doc.diagnostics);
        assert_eq!(doc.code_blocks[0].value, "\n{\"x\":1}\n");
        assert_eq!(&text[doc.code_blocks[0].content_offset..][..1], "\n");
        assert_eq!(
            doc.references
                .iter()
                .map(|r| r.target.as_str())
                .collect::<Vec<_>>(),
            ["theme.css", "a.png"]
        );
    }
    #[test]
    fn document_and_policy_checks() {
        let source = Source::new(
            "test.html".into(),
            "test.html".into(),
            "<html><head></head><body></body></html>".into(),
        );
        let mut config = Config::default();
        config.html.require_lang = true;
        config.html.require_main = true;
        let doc = parse(&source, &config);
        for rule in ["html.document", "html.language", "html.landmarks"] {
            assert!(doc.diagnostics.iter().any(|d| d.rule == rule), "{rule}");
        }
        assert!(
            errors("<meta charset=windows-1252>")
                .iter()
                .any(|e| e.contains("html.encoding"))
        );
        assert!(
            errors("<button aria-checked=perhaps>x</button>")
                .iter()
                .any(|e| e.contains("html.aria"))
        );
        assert!(
            read_html("<a id='{{ name }}'>Link</a>")
                .diagnostics
                .iter()
                .any(|d| d.outcome == Outcome::Unverified)
        );
    }
    #[test]
    fn optional_end_tags_have_conditions() {
        for valid in [
            "<dl><dt>T<dd>D</dl>",
            "<table><caption>C<colgroup><col><thead><tr><th>H<tbody><tr><td>D</table>",
            "<!doctype html><html><head><title>T</title><p>Text",
            "<select aria-label='Choice'><optgroup label='A'><option>A<hr><option>B</select>",
        ] {
            assert!(errors(valid).is_empty(), "{valid}: {:?}", errors(valid));
        }
        for invalid in [
            "<dl><dt>T</dl>",
            "<table><thead><tr><th>H</table>",
            "<x-panel><p>T</x-panel>",
        ] {
            assert!(
                errors(invalid).iter().any(|e| e.contains("end tag")),
                "{invalid}"
            );
        }
    }
    #[test]
    fn inline_fragment_names_and_code_literals() {
        let text = "`<img src=missing>`\n<button>Save</button> and <h2 id=title>Title</h2>";
        let source = Source::new("test.md".into(), "test.md".into(), text.into());
        let ranges = ["<button>", "</button>", "<h2 id=title>", "</h2>"].map(|tag| {
            let start = text.find(tag).unwrap();
            start..start + tag.len()
        });
        let doc = parse_fragment(&source, &ranges, &Config::default());
        assert!(doc.diagnostics.is_empty(), "{:?}", doc.diagnostics);
        assert!(doc.references.is_empty());
        assert_eq!(doc.elements[0].text, "Save");
        assert_eq!(doc.headings[0].text, "Title");
    }
    #[test]
    fn static_table_and_group_boundaries() {
        let overlap = "<table><tr><td>A<td rowspan=2>B<tr><td colspan=2>C</table>";
        assert!(errors(overlap).iter().any(|e| e.contains("overlaps")));
        for invalid in [
            "<table><tbody><tr><td>A</tbody><thead><tr><th>H</thead></table>",
            "<dl><dd>D</dd></dl>",
            "<details><p>T</p><summary>S</summary></details>",
            "<figure><figcaption>A</figcaption><figcaption>B</figcaption></figure>",
        ] {
            assert!(!errors(invalid).is_empty(), "{invalid}");
        }
        assert!(errors("<table><tr><td rowspan=0>A<td>B<tr><td>C</table>").is_empty());
    }
    #[test]
    fn inert_template_ids_are_not_active_anchors() {
        let doc = read_html("<template><div id=x></div></template><div id=x></div>");
        assert!(doc.diagnostics.is_empty(), "{:?}", doc.diagnostics);
        assert_eq!(doc.anchors.len(), 1);
    }
    #[test]
    fn css_strings_are_not_urls() {
        let doc = read_html(
            "<style>/* url(bad) */p::before{content:'url(missing)';background:url(real.png)}</style>",
        );
        assert_eq!(
            doc.references
                .iter()
                .map(|r| r.target.as_str())
                .collect::<Vec<_>>(),
            ["real.png"]
        );
    }
    #[test]
    fn policies_can_disable_name_checks() {
        let source = Source::new("test.html".into(), "test.html".into(), "<input>".into());
        let mut config = Config::default();
        config.html.accessibility = false;
        assert!(parse(&source, &config).diagnostics.is_empty());
        assert!(
            read_html("<div class='{{ class }}'></div>")
                .diagnostics
                .is_empty()
        );
    }
}
