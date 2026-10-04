use perfect_doc::{Config, Outcome, Report, validate};
use std::fs;

fn fixture(files: &[(&str, &str)], edit: impl FnOnce(&mut Config)) -> Report {
    let directory = tempfile::tempdir().unwrap();
    for (path, text) in files {
        let target = directory.path().join(path);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(target, text).unwrap();
    }
    let mut config = Config {
        base_dir: fs::canonicalize(directory.path()).unwrap(),
        include: vec!["host.md".into()],
        ..Config::default()
    };
    config.markdown.includes = true;
    edit(&mut config);
    validate(&[directory.path().into()], &config).unwrap()
}

fn valid(report: Report) {
    assert_eq!(report.exit_code(), 0, "{:?}", report.diagnostics);
}

#[test]
fn headings_from_unselected_includes_resolve_in_host() {
    valid(fixture(
        &[
            ("host.md", "# Host\n\n[Part](#part)\n\n!include[part.md]\n"),
            ("part.md", "## Part\n\n[Host](#host)\n"),
        ],
        |_| {},
    ));
}

#[test]
fn repeated_included_headings_get_complete_host_suffixes() {
    valid(fixture(
        &[
            (
                "host.md",
                "# Same\n\n!include[part.md]\n\n!include[part.md]\n\n[Last](#same-2)\n",
            ),
            ("part.md", "## Same\n"),
        ],
        |_| {},
    ));
}

#[test]
fn included_frontmatter_does_not_enter_host_metadata_or_headings() {
    valid(fixture(
        &[
            (
                "host.md",
                "---\ntitle: Host\n---\n# Host\n\n!include[part.md]\n\n[Part](#part)\n",
            ),
            ("part.md", "---\ntitle: Part\n---\n## Part\n"),
        ],
        |config| {
            config.frontmatter.required_fields = vec!["title".into()];
            config.frontmatter.forbidden_fields = vec!["not_allowed".into()];
        },
    ));
}

#[test]
fn retained_host_metadata_links_are_checked() {
    let report = fixture(
        &[
            (
                "host.md",
                "---\nmore: missing.md\n---\n# Host\n\n!include[part.md]\n",
            ),
            ("part.md", "## Part\n"),
        ],
        |config| config.frontmatter.link_fields = vec!["more".into()],
    );
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.rule == "link.exists"),
        "{:?}",
        report.diagnostics
    );
}

#[test]
fn raw_html_associations_resolve_across_include_boundary() {
    valid(fixture(
        &[
            (
                "host.md",
                "# Host\n\n<label for='name'>Name</label>\n\n!include[part.md]\n",
            ),
            ("part.md", "<input id='name'>\n"),
        ],
        |_| {},
    ));
}

#[test]
fn included_references_use_included_file_directory_even_with_routes() {
    valid(fixture(
        &[
            ("host.md", "# Host\n\n!include[parts/part.md]\n"),
            (
                "parts/part.md",
                "[Data](data.txt)\n\n![Picture](picture.svg)\n",
            ),
            ("parts/data.txt", "Data\n"),
            (
                "parts/picture.svg",
                "<svg xmlns='http://www.w3.org/2000/svg'/>",
            ),
        ],
        |config| config.routes.enabled = true,
    ));
}

#[test]
fn included_error_has_original_unicode_location_and_include_context() {
    let report = fixture(
        &[
            ("host.md", "# Host\n\n!include[parts/part.md]\n"),
            ("parts/part.md", "## Part\n\n雪 [Missing](absent.txt)\n"),
        ],
        |_| {},
    );
    let diagnostic = report
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.rule == "link.exists")
        .unwrap();
    assert_eq!(diagnostic.location.path, "parts/part.md");
    assert_eq!(diagnostic.location.line, 3);
    assert!(diagnostic.location.column >= 3);
    assert!(
        diagnostic
            .related
            .iter()
            .any(|location| location.path == "host.md" && location.line == 3),
        "{diagnostic:?}"
    );
}

#[test]
fn region_selection_excludes_other_content_and_markers() {
    valid(fixture(
        &[
            (
                "host.md",
                "# Host\n\n!include[part.md]{region=chosen}\n\n[Part](#part)\n",
            ),
            (
                "part.md",
                "[Missing](absent.txt)\n<!-- region chosen -->\n## Part\n<!-- endregion chosen -->\n[Missing](another.txt)\n",
            ),
        ],
        |_| {},
    ));
}

#[test]
fn line_selection_keeps_original_source_line() {
    let report = fixture(
        &[
            ("host.md", "# Host\n\n!include[part.txt]{start=2 end=3}\n"),
            (
                "part.txt",
                "[Ignored](absent.txt)\r\n## Part\r\n[Missing](missing.txt)\r\n",
            ),
        ],
        |_| {},
    );
    let missing: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.rule == "link.exists")
        .collect();
    assert_eq!(missing.len(), 1, "{:?}", report.diagnostics);
    assert_eq!(missing[0].location.path, "part.txt");
    assert_eq!(missing[0].location.line, 3);
}

#[test]
fn nested_unselected_includes_resolve_anchors_and_origin() {
    valid(fixture(
        &[
            (
                "host.md",
                "# Host\n\n!include[parts/one.md]\n\n[Two](#two)\n",
            ),
            ("parts/one.md", "## One\n\n!include[nested/two.md]\n"),
            ("parts/nested/two.md", "### Two\n\n[Data](data.txt)\n"),
            ("parts/nested/data.txt", "Data\n"),
        ],
        |_| {},
    ));
}

#[test]
fn nested_cycle_cannot_pass() {
    let report = fixture(
        &[
            ("host.md", "# Host\n\n!include[one.md]\n"),
            ("one.md", "## One\n\n!include[host.md]\n"),
        ],
        |_| {},
    );
    assert_eq!(report.exit_code(), 1, "{:?}", report.diagnostics);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.rule == "include.expansion"
                && diagnostic.outcome == Outcome::Invalid)
    );
}

#[test]
fn depth_limit_is_required_unverified() {
    let report = fixture(
        &[
            ("host.md", "# Host\n\n!include[one.md]\n"),
            ("one.md", "## One\n\n!include[two.md]\n"),
            ("two.md", "### Two\n"),
        ],
        |config| config.collection.include_max_depth = 1,
    );
    assert_eq!(report.exit_code(), 3, "{:?}", report.diagnostics);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.rule == "include.expansion"
                && diagnostic.outcome == Outcome::Unverified
                && diagnostic.required)
    );
}

#[test]
fn repeated_includes_obey_composed_file_byte_limit() {
    let report = fixture(
        &[
            ("host.md", "!include[p.txt]\n!include[p.txt]\n"),
            ("p.txt", "abcdefghijklmnopqrstuvwxyz\n"),
        ],
        |config| config.files.max_file_bytes = 40,
    );
    assert_eq!(report.exit_code(), 3, "{:?}", report.diagnostics);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.rule == "include.expansion"
                && diagnostic.outcome == Outcome::Unverified)
    );
}

#[test]
fn literal_code_and_code_file_records_are_not_expanded() {
    valid(fixture(
        &[
            (
                "host.md",
                "# Host\n\n`!include[absent.md]`\n\n```text {file=literal.txt start=1 end=1}\n!include[missing.md]\n```\n\n!include[part.md]\n",
            ),
            ("literal.txt", "!include[missing.md]\n"),
            ("part.md", "## Part\n"),
        ],
        |_| {},
    ));
}

#[test]
fn included_structured_examples_are_checked_after_composition() {
    let report = fixture(
        &[
            ("host.md", "# Host\n\n!include[part.txt]\n"),
            ("part.txt", "```json\n{broken}\n```\n"),
        ],
        |config| config.markdown.structured_examples = true,
    );
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.rule == "example.syntax"
                && diagnostic.location.path == "part.txt"),
        "{:?}",
        report.diagnostics
    );
}

#[test]
fn invalid_region_and_unknown_attributes_cannot_pass() {
    for directive in [
        "!include[part.md]{region=absent}",
        "!include[part.md]{start=bad}",
        "!include[part.md]{other=value}",
    ] {
        let report = fixture(&[("host.md", directive), ("part.md", "# Part\n")], |_| {});
        assert_ne!(
            report.exit_code(),
            0,
            "{directive}: {:?}",
            report.diagnostics
        );
    }
}

#[test]
fn total_composed_bytes_and_new_source_count_are_bounded() {
    let files = [
        (
            "host.md",
            "!include[p.txt]\n!include[p.txt]\n!include[p.txt]\n",
        ),
        ("p.txt", "abcdefghijklmnopqrstuvwxyz\n"),
    ];
    for edit in [(75, 100usize), (1024, 1usize)] {
        let report = fixture(&files, |config| {
            config.files.max_total_bytes = edit.0;
            config.files.max_files = edit.1;
            config.exclude.push("p.txt".into());
        });
        assert_eq!(report.exit_code(), 3, "{:?}", report.diagnostics);
        assert!(
            report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.rule == "include.expansion"
                    && diagnostic.outcome == Outcome::Unverified)
        );
    }
}

#[test]
fn missing_target_and_physical_parent_escape_cannot_pass() {
    for directive in [
        "!include[missing.txt]",
        "!include[../outside.md]",
        "!include[https://example.invalid/part.md]",
    ] {
        let report = fixture(&[("host.md", directive)], |config| {
            config.links.allow_outside_root = true
        });
        assert_eq!(
            report.exit_code(),
            1,
            "{directive}: {:?}",
            report.diagnostics
        );
        assert!(
            report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.rule == "include.expansion"
                    && diagnostic.outcome == Outcome::Invalid)
        );
    }
}

#[test]
fn nested_code_file_directive_stays_literal_but_its_missing_target_is_checked() {
    let report = fixture(
        &[
            ("host.md", "# Host\n\n!include[part.md]\n"),
            ("part.md", "```text {file=missing.txt}\nLiteral text\n```\n"),
        ],
        |_| {},
    );
    assert_eq!(report.exit_code(), 1, "{:?}", report.diagnostics);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.rule == "include.expansion"
                && diagnostic.location.path == "part.md")
    );
}

#[test]
fn selected_include_source_has_the_same_complete_anchor_semantics() {
    valid(fixture(
        &[
            ("host.md", "# Host\n\n!include[part.md]\n\n[Part](#part)\n"),
            ("part.md", "## Part\n"),
        ],
        |config| config.include = vec!["**/*.md".into()],
    ));
}

#[test]
fn unicode_byte_offset_is_from_original_included_text() {
    let part = "## Part\n\n雪 é [Missing](absent.txt)\n";
    let report = fixture(
        &[("host.md", "!include[part.md]\n"), ("part.md", part)],
        |_| {},
    );
    let diagnostic = report
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.rule == "link.exists")
        .unwrap();
    let prefix = &part[..diagnostic.location.byte_offset];
    assert_eq!(
        diagnostic.location.column,
        prefix.rsplit('\n').next().unwrap().chars().count() + 1
    );
    assert_eq!(diagnostic.location.path, "part.md");
}

#[test]
fn static_mdx_elements_are_checked_after_include_composition() {
    valid(fixture(
        &[
            (
                "host.md",
                "# Host\n\n<label htmlFor=\"name\">Name</label>\n\n!include[part.mdx]\n",
            ),
            ("part.mdx", "<input id=\"name\" />\n"),
        ],
        |config| config.markdown.dialect = perfect_doc::config::MarkdownDialect::Mdx,
    ));
}

#[test]
fn included_wrong_type_id_target_is_invalid() {
    let report = fixture(
        &[
            (
                "host.md",
                "# Host\n\n<label for='name'>Name</label>\n\n!include[part.md]\n",
            ),
            ("part.md", "<div id='name'>Text</div>\n"),
        ],
        |_| {},
    );
    assert_eq!(report.exit_code(), 1, "{:?}", report.diagnostics);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.rule == "link.type")
    );
}

#[test]
fn selected_file_and_include_region_have_separate_source_contracts() {
    let report = fixture(
        &[
            ("host.md", "# Host\n\n!include[part.md]{region=chosen}\n"),
            (
                "part.md",
                "[Missing](missing.txt)\n\n<!-- region chosen -->\n## Part\n<!-- endregion chosen -->\n",
            ),
        ],
        |config| config.include = vec!["**/*.md".into()],
    );
    let missing: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.rule == "link.exists")
        .collect();
    assert_eq!(missing.len(), 1, "{:?}", report.diagnostics);
    assert_eq!(missing[0].location.path, "part.md");
    assert!(missing[0].related.is_empty());
}

#[test]
fn included_bom_does_not_hide_frontmatter_or_create_a_body_heading() {
    valid(fixture(
        &[
            ("host.md", "# Host\n\n!include[part.md]\n\n[Part](#part)\n"),
            ("part.md", "\u{feff}---\ntitle: Part\n---\n## Part\n"),
        ],
        |config| config.markdown.no_heading_skips = true,
    ));
}

#[test]
fn inserted_forbidden_control_is_invalid_at_its_original_location() {
    let report = fixture(
        &[
            ("host.md", "# Host\n\n!include[part.txt]\n"),
            ("part.txt", "雪\nBad\0text\n"),
        ],
        |_| {},
    );
    assert_eq!(report.exit_code(), 1, "{:?}", report.diagnostics);
    let error = report
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.rule == "source.encoding")
        .unwrap();
    assert_eq!(error.location.path, "part.txt");
    assert_eq!(error.location.line, 2);
    assert_eq!(error.location.column, 4);
    assert_eq!(error.location.byte_offset, "雪\nBad".len());
    assert!(
        error
            .related
            .iter()
            .any(|location| location.path == "host.md")
    );
}
