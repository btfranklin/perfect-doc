use crate::{
    config::Config,
    model::{Document, Heading, Outcome, ReferenceKind, Requirement},
};

/// Check the complete heading list after static include expansion.
pub fn check(document: &mut Document, config: &Config) {
    document.parsed.diagnostics.retain(|diagnostic| {
        !matches!(
            diagnostic.rule.as_str(),
            "markdown.heading-shape" | "markdown.empty-section"
        )
    });
    document
        .parsed
        .headings
        .sort_by_key(|heading| heading.offset);
    let headings = document.parsed.headings.clone();
    let policy = &config.markdown;
    if policy.heading_start.is_none()
        && !policy.single_h1
        && !policy.no_heading_skips
        && !policy.nonempty_headings
        && !policy.nonempty_sections
    {
        return;
    }
    if let (Some(first), Some(level)) = (headings.first(), policy.heading_start)
        && first.level != level
    {
        issue(
            document,
            "markdown.heading-shape",
            first.offset,
            format!("The first heading must have level {level}."),
        );
    }

    let mut previous = 0;
    let mut h1_count = 0;
    let mut content = document.parsed.content_ranges.clone();
    content.retain(|range| range.end > range.start);
    content.sort_by_key(|range| range.start);
    let mut media: Vec<_> = document
        .parsed
        .references
        .iter()
        .filter(|reference| reference.kind == ReferenceKind::Image)
        .map(|reference| reference.offset)
        .collect();
    media.extend(
        document
            .parsed
            .elements
            .iter()
            .filter(|element| {
                if !is_content_element(&element.tag) {
                    return false;
                }
                let mut parent = element.parent;
                while let Some(index) = parent {
                    let ancestor = &document.parsed.elements[index];
                    if matches!(
                        ancestor.tag.as_str(),
                        "head" | "script" | "style" | "template"
                    ) {
                        return false;
                    }
                    parent = ancestor.parent;
                }
                true
            })
            .map(|element| element.offset),
    );
    media.sort_unstable();
    for (index, heading) in headings.iter().enumerate() {
        if heading.level == 1 {
            h1_count += 1;
            if policy.single_h1 && h1_count > 1 {
                issue(
                    document,
                    "markdown.heading-shape",
                    heading.offset,
                    "Only one level-one heading is allowed.",
                );
            }
        }
        if policy.no_heading_skips && previous != 0 && heading.level > previous + 1 {
            issue(
                document,
                "markdown.heading-shape",
                heading.offset,
                "This heading skips a level.",
            );
        }
        if policy.nonempty_headings && !heading_has_content(heading, &media) {
            content_issue(
                document,
                "markdown.heading-shape",
                heading.offset,
                "This heading needs content.",
            );
        }
        previous = heading.level;
        if policy.nonempty_sections {
            let end = headings
                .get(index + 1)
                .map_or(document.source.text.len(), |next| next.offset);
            let next = content.partition_point(|range| range.start < heading.end);
            if !content
                .get(next)
                .is_some_and(|range| range.start < end && range.end > range.start)
            {
                content_issue(
                    document,
                    "markdown.empty-section",
                    heading.offset,
                    "This section has no content before the next heading.",
                );
            }
        }
    }
}

fn heading_has_content(heading: &Heading, media: &[usize]) -> bool {
    let next = media.partition_point(|offset| *offset < heading.offset);
    !heading.text.trim().is_empty() || media.get(next).is_some_and(|offset| *offset < heading.end)
}

/// These static elements can contain content without a text child.
pub(crate) fn is_content_element(tag: &str) -> bool {
    matches!(
        tag,
        "img"
            | "picture"
            | "audio"
            | "video"
            | "iframe"
            | "embed"
            | "object"
            | "svg"
            | "canvas"
            | "math"
            | "input"
            | "button"
            | "select"
            | "textarea"
            | "table"
            | "pre"
            | "code"
            | "hr"
    )
}

fn issue(document: &mut Document, rule: &str, offset: usize, message: impl Into<String>) {
    document.parsed.diagnostics.push(document.source.diagnostic(
        rule,
        Requirement::Project,
        Outcome::Invalid,
        offset,
        message,
    ));
}

fn content_issue(document: &mut Document, rule: &str, offset: usize, message: &str) {
    if document.parsed.dynamic_anchors || document.parsed.incomplete {
        document.parsed.diagnostics.push(document.source.diagnostic(
            rule,
            Requirement::Execution,
            Outcome::Unverified,
            offset,
            "Document content is incomplete or has runtime expressions. Supply complete static rendered output to check empty headings and sections.",
        ));
    } else {
        issue(document, rule, offset, message);
    }
}
