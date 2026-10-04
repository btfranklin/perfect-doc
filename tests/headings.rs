use perfect_doc::{
    Config, Outcome, Report,
    config::{HtmlMode, MarkdownDialect},
    validate,
};
use tempfile::TempDir;

fn project(path: &str, text: &str) -> (TempDir, Config) {
    let directory = TempDir::new().unwrap();
    std::fs::write(directory.path().join(path), text).unwrap();
    let config = Config {
        base_dir: directory.path().to_owned(),
        ..Config::default()
    };
    (directory, config)
}

fn check(directory: &TempDir, config: &Config) -> Report {
    validate(&[directory.path().to_owned()], config).unwrap()
}

fn heading_errors<'a>(report: &'a Report, rule: &str) -> Vec<&'a perfect_doc::Diagnostic> {
    report
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.rule == rule && diagnostic.suppression.is_none())
        .collect()
}

#[test]
fn raw_html_first_heading_precedes_markdown_for_start_policy() {
    let (directory, mut config) = project("page.md", "<h1>Start</h1>\n\n## Next\nText.\n");
    config.markdown.heading_start = Some(1);
    config.markdown.no_heading_skips = true;
    let report = check(&directory, &config);
    assert_eq!(report.exit_code(), 0, "{:?}", report.diagnostics);
}

#[test]
fn raw_html_after_markdown_participates_in_skip_policy() {
    let (directory, mut config) = project("page.md", "# 雪\r\n\r\n<h3>Next</h3>\r\n");
    config.markdown.no_heading_skips = true;
    let report = check(&directory, &config);
    let errors = heading_errors(&report, "markdown.heading-shape");
    assert_eq!(errors.len(), 1, "{:?}", report.diagnostics);
    assert_eq!(errors[0].outcome, Outcome::Invalid);
    assert_eq!(errors[0].location.path, "page.md");
    assert_eq!(errors[0].location.line, 3);
    assert_eq!(errors[0].location.column, 1);
    assert_eq!(errors[0].location.byte_offset, 9);
    assert_eq!(report.exit_code(), 1);
}

#[test]
fn standalone_html_obeys_all_heading_shape_policies() {
    let (directory, mut config) = project(
        "page.html",
        "<h2>Start</h2>\n<h4>Skipped</h4>\n<h1> </h1>\n<h1>Again</h1>\n",
    );
    config.html.mode = HtmlMode::Fragment;
    config.markdown.heading_start = Some(1);
    config.markdown.no_heading_skips = true;
    config.markdown.single_h1 = true;
    config.markdown.nonempty_headings = true;
    let report = check(&directory, &config);
    assert_eq!(heading_errors(&report, "markdown.heading-shape").len(), 4);
    assert_eq!(report.exit_code(), 1, "{:?}", report.diagnostics);
}

#[test]
fn empty_html_section_ignores_comments_end_tags_and_empty_wrappers() {
    let (directory, mut config) = project(
        "page.html",
        "<section><h1>Start</h1><!-- note --><div> \n </div></section>\n<h2>Next</h2><p>Text.</p>",
    );
    config.markdown.nonempty_sections = true;
    let report = check(&directory, &config);
    let errors = heading_errors(&report, "markdown.empty-section");
    assert_eq!(errors.len(), 1, "{:?}", report.diagnostics);
    assert_eq!(errors[0].location.line, 1);
    assert_eq!(errors[0].location.column, 10);
}

#[test]
fn markdown_section_ignores_raw_html_comment_and_empty_wrapper() {
    let (directory, mut config) = project(
        "page.md",
        "# Start\n\n<!-- note -->\n\n<div></div>\n\n## Next\nText.\n",
    );
    config.markdown.nonempty_sections = true;
    let report = check(&directory, &config);
    let errors = heading_errors(&report, "markdown.empty-section");
    assert_eq!(errors.len(), 1, "{:?}", report.diagnostics);
    assert_eq!(errors[0].location.line, 1);
}

#[test]
fn selected_directive_boundaries_are_not_section_content() {
    for (body, empty) in [(":::note\n:::", true), (":::note\nText.\n:::", false)] {
        let text = format!("# Start\n\n{body}\n");
        let (directory, mut config) = project("page.md", &text);
        config.markdown.directives = true;
        config.markdown.nonempty_sections = true;
        let report = check(&directory, &config);
        assert_eq!(
            !heading_errors(&report, "markdown.empty-section").is_empty(),
            empty,
            "{:?}",
            report.diagnostics
        );
    }
}

#[test]
fn static_media_controls_tables_and_code_count_as_section_content() {
    for content in [
        "<video></video>",
        "<input aria-label=\"Name\">",
        "<table></table>",
        "<code></code>",
        "<hr>",
        "<p>&amp;</p>",
    ] {
        let text = format!("<h1>Start</h1>{content}");
        let (directory, mut config) = project("page.html", &text);
        config.html.conformance = false;
        config.html.accessibility = false;
        config.markdown.nonempty_sections = true;
        let report = check(&directory, &config);
        assert!(
            heading_errors(&report, "markdown.empty-section").is_empty(),
            "{content}: {:?}",
            report.diagnostics
        );
    }
    for content in ["```text\n```", "`code`", "---", "|A|\n|-|\n| |"] {
        let text = format!("# Start\n\n{content}\n");
        let (directory, mut config) = project("page.md", &text);
        config.markdown.nonempty_sections = true;
        let report = check(&directory, &config);
        assert_eq!(report.exit_code(), 0, "{content}: {:?}", report.diagnostics);
    }
}

#[test]
fn head_script_style_and_template_content_do_not_fill_sections() {
    for content in [
        "<script>const value = 1;</script>",
        "<style>p { color: red; }</style>",
        "<template><p>Text.</p><input></template>",
    ] {
        let text = format!("<h1>Start</h1>{content}");
        let (directory, mut config) = project("page.html", &text);
        config.markdown.nonempty_sections = true;
        let report = check(&directory, &config);
        assert_eq!(
            heading_errors(&report, "markdown.empty-section").len(),
            1,
            "{content}: {:?}",
            report.diagnostics
        );
    }
}

#[test]
fn static_mdx_uses_combined_heading_order_and_content() {
    let (directory, mut config) = project(
        "page.mdx",
        "<div>\n<h1>Start</h1>\n<input aria-label=\"Name\" />\n</div>\n\n## Next\nText.\n",
    );
    config.markdown.dialect = MarkdownDialect::Mdx;
    config.markdown.heading_start = Some(1);
    config.markdown.no_heading_skips = true;
    config.markdown.nonempty_sections = true;
    let report = check(&directory, &config);
    assert_eq!(report.exit_code(), 0, "{:?}", report.diagnostics);
}

#[test]
fn static_media_can_fill_a_heading_without_text() {
    for (path, text) in [
        ("page.html", "<h1><svg></svg></h1>"),
        ("page.md", "# ![](data:image/png;base64,iVBORw0KGgo=)"),
        ("page.mdx", "<h1><svg /></h1>"),
    ] {
        let (directory, mut config) = project(path, text);
        config.html.accessibility = false;
        config.markdown.nonempty_headings = true;
        if path.ends_with("mdx") {
            config.markdown.dialect = MarkdownDialect::Mdx;
        }
        let report = check(&directory, &config);
        assert!(
            heading_errors(&report, "markdown.heading-shape").is_empty(),
            "{path}: {:?}",
            report.diagnostics
        );
    }
}

#[test]
fn heading_policies_are_disabled_by_default() {
    for (path, text) in [
        ("page.html", "<h2></h2><h4>Next</h4><h1>A</h1><h1>B</h1>"),
        ("page.md", "##\n\n<h4>Next</h4>\n\n# A\n\n# B\n"),
    ] {
        let (directory, config) = project(path, text);
        let report = check(&directory, &config);
        assert_eq!(report.exit_code(), 0, "{:?}", report.diagnostics);
    }
}

#[test]
fn dynamic_mdx_content_does_not_prove_a_heading_or_section_empty() {
    let (directory, mut config) = project("page.mdx", "<h1>{title}</h1>\n\n{body}\n");
    config.markdown.dialect = MarkdownDialect::Mdx;
    config.markdown.nonempty_headings = true;
    config.markdown.nonempty_sections = true;
    let report = check(&directory, &config);
    for rule in ["markdown.heading-shape", "markdown.empty-section"] {
        let errors = heading_errors(&report, rule);
        assert_eq!(errors.len(), 1, "{:?}", report.diagnostics);
        assert_eq!(errors[0].outcome, Outcome::Unverified);
        assert!(errors[0].required);
    }
    assert_eq!(report.exit_code(), 3, "{:?}", report.diagnostics);
}

#[test]
fn inert_template_text_and_media_do_not_fill_a_heading() {
    for (path, text) in [
        (
            "page.html",
            "<h1><template>Text.<svg></svg></template></h1>",
        ),
        ("page.mdx", "<h1><template>Text.<svg /></template></h1>"),
    ] {
        let (directory, mut config) = project(path, text);
        config.markdown.nonempty_headings = true;
        if path.ends_with("mdx") {
            config.markdown.dialect = MarkdownDialect::Mdx;
        }
        let report = check(&directory, &config);
        assert_eq!(
            heading_errors(&report, "markdown.heading-shape").len(),
            1,
            "{path}: {:?}",
            report.diagnostics
        );
    }
}

#[test]
fn composed_heading_error_keeps_fragment_location_and_include_context() {
    let (directory, mut config) = project("host.md", "# Start\n\n!include[part.md]\n");
    std::fs::write(directory.path().join("part.md"), "### 雪\r\nText.\r\n").unwrap();
    config.include = vec!["host.md".into()];
    config.markdown.includes = true;
    config.markdown.no_heading_skips = true;
    let report = check(&directory, &config);
    let errors = heading_errors(&report, "markdown.heading-shape");
    assert_eq!(errors.len(), 1, "{:?}", report.diagnostics);
    assert_eq!(errors[0].location.path, "part.md");
    assert_eq!(errors[0].location.line, 1);
    assert_eq!(errors[0].location.column, 1);
    assert!(errors[0].related.iter().any(|location| {
        location.path == "host.md" && location.line == 3 && location.column == 1
    }));
}
