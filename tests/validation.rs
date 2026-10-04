use perfect_doc::{Config, Outcome, model::Severity, validate};
use std::{collections::BTreeMap, path::Path};
use tempfile::TempDir;

fn write(root: &Path, path: &str, text: &str) {
    let path = root.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}
fn project(files: &[(&str, &str)]) -> (TempDir, Config) {
    let dir = TempDir::new().unwrap();
    for (path, text) in files {
        write(dir.path(), path, text);
    }
    let cfg = Config {
        base_dir: dir.path().to_owned(),
        ..Config::default()
    };
    (dir, cfg)
}
fn scan(dir: &TempDir, cfg: &Config) -> perfect_doc::Report {
    validate(&[dir.path().to_owned()], cfg).unwrap()
}
fn has(report: &perfect_doc::Report, rule: &str) -> bool {
    report
        .diagnostics
        .iter()
        .any(|d| d.rule == rule && d.suppression.is_none())
}

#[test]
fn complete_collection_checks_forward_links_and_okf() {
    let (dir, cfg) = project(&[
        ("start.md", "# Start\n[Next](nested/next.okf#next)\n"),
        ("nested/next.okf", "# Next\n[Back](../start.md#start)"),
    ]);
    let report = scan(&dir, &cfg);
    assert_eq!(report.exit_code(), 0, "{:?}", report.diagnostics);
    assert_eq!(report.coverage.documents, 2);
    assert_eq!(report.coverage.local_verified, 2);
}
#[test]
fn missing_file_and_anchor_have_original_unicode_locations() {
    let (dir, cfg) = project(&[
        ("a.md", "# α\n雪 [target](b.md#lost)\n[missing](gone.md)\n"),
        ("b.md", "# Present"),
    ]);
    let report = scan(&dir, &cfg);
    assert_eq!(report.exit_code(), 1);
    let d = report
        .diagnostics
        .iter()
        .find(|d| d.rule == "link.anchor")
        .unwrap();
    assert_eq!(d.location.line, 2);
    assert_eq!(d.location.column, 3);
    assert!(has(&report, "link.exists"));
}
#[test]
fn code_literals_do_not_create_links_or_conflicts() {
    let (dir, cfg) = project(&[(
        "a.md",
        "# Text\n`[fake](absent.md)`\n```md\n<<<<<<< branch\n[bad](absent.md)\n<div><img src=bad>\n```\n",
    )]);
    assert_eq!(scan(&dir, &cfg).exit_code(), 0);
}
#[test]
fn root_relative_and_encoded_paths_resolve() {
    let (dir, cfg) = project(&[
        ("a.md", "[go](/folder/snow%20%E9%9B%AA.md#snow)"),
        ("folder/snow 雪.md", "# Snow"),
    ]);
    assert_eq!(scan(&dir, &cfg).exit_code(), 0);
}
#[test]
fn query_does_not_change_local_target() {
    let (dir, cfg) = project(&[("a.md", "[go](b.md?display=full#b)"), ("b.md", "# B")]);
    assert_eq!(scan(&dir, &cfg).exit_code(), 0);
}
#[test]
fn directory_index_ambiguity_is_a_failure() {
    let (dir, cfg) = project(&[
        ("a.md", "[go](folder/)"),
        ("folder/index.md", "# A"),
        ("folder/index.html", "<p>B</p>"),
    ]);
    assert_eq!(scan(&dir, &cfg).exit_code(), 1);
}
#[test]
fn empty_inputs_and_unselected_fragment_targets_are_incomplete() {
    let (dir, mut cfg) = project(&[]);
    assert_eq!(scan(&dir, &cfg).exit_code(), 3);
    cfg.files.allow_empty = true;
    assert_eq!(scan(&dir, &cfg).exit_code(), 0);
    write(dir.path(), "a.md", "[go](b.md#b)");
    write(dir.path(), "b.md", "# B");
    cfg.include = vec!["a.md".into()];
    assert_eq!(scan(&dir, &cfg).exit_code(), 3);
}
#[test]
fn missing_input_is_an_invocation_failure() {
    let (dir, cfg) = project(&[]);
    assert!(validate(&[dir.path().join("missing")], &cfg).is_err());
}
#[test]
fn required_input_patterns_cannot_silently_miss() {
    let (dir, mut cfg) = project(&[("a.md", "# A")]);
    cfg.required_paths = vec!["handbook/*.md".into()];
    assert!(has(&scan(&dir, &cfg), "discovery.required"));
}
#[test]
fn limits_do_not_claim_success() {
    let (dir, mut cfg) = project(&[("a.md", "# A\nThis is a document")]);
    cfg.files.max_file_bytes = 5;
    assert_eq!(scan(&dir, &cfg).exit_code(), 3);
    cfg.files.max_file_bytes = 100;
    write(dir.path(), "b.md", "# B");
    cfg.files.max_files = 1;
    assert_eq!(scan(&dir, &cfg).exit_code(), 3);
}
#[test]
fn invalid_encoding_and_nul_are_invalid() {
    let (dir, cfg) = project(&[("a.md", "# A\0")]);
    assert!(has(&scan(&dir, &cfg), "source.encoding"));
    std::fs::write(dir.path().join("a.md"), [0xff]).unwrap();
    assert_eq!(scan(&dir, &cfg).exit_code(), 1);
}
#[test]
fn byte_order_mark_does_not_shift_body_locations() {
    let (dir, cfg) = project(&[("a.md", "\u{feff}# A\r\n[bad](missing.md)\r\n")]);
    let report = scan(&dir, &cfg);
    let d = report
        .diagnostics
        .iter()
        .find(|d| d.rule == "link.exists")
        .unwrap();
    assert_eq!(d.location.line, 2);
    assert_eq!(d.location.column, 1);
    assert_eq!(d.location.byte_offset, 8);
}
#[test]
fn frontmatter_yaml_toml_json_and_duplicate_keys() {
    for (delim, metadata) in [
        ("---", "id: page"),
        ("+++", "id = \"page\""),
        (";;;", "{\"id\":\"page\"}"),
    ] {
        let text = format!("{delim}\n{metadata}\n{delim}\n# Heading\n[bad](lost.md)");
        let (dir, mut cfg) = project(&[("a.md", &text)]);
        cfg.frontmatter.required_fields = vec!["id".into()];
        let report = scan(&dir, &cfg);
        assert!(!has(&report, "frontmatter.syntax"));
        assert_eq!(
            report
                .diagnostics
                .iter()
                .find(|d| d.rule == "link.exists")
                .unwrap()
                .location
                .line,
            5
        );
    }
    for (delim, metadata) in [
        ("---", "id: one\nid: two"),
        ("+++", "id = 1\nid = 2"),
        (";;;", "{\"id\":1,\"id\":2}"),
    ] {
        let text = format!("{delim}\n{metadata}\n{delim}\n# A");
        let (dir, cfg) = project(&[("a.md", &text)]);
        assert!(has(&scan(&dir, &cfg), "frontmatter.syntax"));
    }
}
#[test]
fn metadata_contracts_and_schema_conditions() {
    let (dir, mut cfg) = project(&[
        (
            "a.md",
            "---\nid: same\nkind: child\nstart: 2026-10-03\nend: 2026-10-01\n---\n# A",
        ),
        ("b.md", "---\nid: same\n---\n# B"),
    ]);
    cfg.frontmatter.id_field = Some("id".into());
    cfg.frontmatter.required_fields = vec!["id".into()];
    cfg.frontmatter.date_order = vec![perfect_doc::config::DateOrder {
        before: "start".into(),
        after: "end".into(),
    }];
    cfg.frontmatter.inline_schema = Some(
        serde_json::json!({"type":"object","if":{"properties":{"kind":{"const":"child"}},"required":["kind"]},"then":{"required":["parent"]}}),
    );
    let report = scan(&dir, &cfg);
    assert!(has(&report, "metadata.unique"));
    assert!(has(&report, "frontmatter.contract"));
    assert!(has(&report, "schema.valid"));
}
#[test]
fn local_schema_references_work_and_remote_refs_stay_incomplete() {
    let (dir, mut cfg) = project(&[
        ("a.md", "---\nvalue: 3\n---\n# A"),
        ("contract.json", "{\"$ref\":\"types.json\"}"),
        (
            "types.json",
            "{\"type\":\"object\",\"properties\":{\"value\":{\"minimum\":4}}}",
        ),
    ]);
    cfg.frontmatter.schema = Some("contract.json".into());
    assert!(has(&scan(&dir, &cfg), "schema.valid"));
    write(
        dir.path(),
        "contract.json",
        "{\"$ref\":\"https://example.invalid/schema.json\"}",
    );
    assert_eq!(scan(&dir, &cfg).exit_code(), 3);
}
#[test]
fn required_metadata_and_shape_are_enforced() {
    let (dir, mut cfg) = project(&[("a.md", "# A")]);
    cfg.frontmatter.required = true;
    assert!(has(&scan(&dir, &cfg), "frontmatter.contract"));
    write(dir.path(), "a.md", "---\n- value\n---\n# A");
    assert!(has(&scan(&dir, &cfg), "frontmatter.shape"));
}
#[test]
fn structured_examples_are_opt_in_and_never_executed() {
    let (dir, mut cfg) = project(&[(
        "a.md",
        "```json\n{broken}\n```\n```sh\ntouch never-created\n```\n",
    )]);
    assert_eq!(scan(&dir, &cfg).exit_code(), 0);
    cfg.markdown.structured_examples = true;
    assert!(has(&scan(&dir, &cfg), "example.syntax"));
    assert!(!dir.path().join("never-created").exists());
    write(
        dir.path(),
        "a.md",
        "```json expect-invalid\n{broken}\n```\n",
    );
    assert_eq!(scan(&dir, &cfg).exit_code(), 0);
}
#[test]
fn structured_collection_inputs_and_json_ld_are_checked() {
    let (dir, mut cfg) = project(&[
        (
            "a.html",
            "<script type=\"application/ld+json\">{\"same\":1,\"same\":2}</script>",
        ),
        ("data.toml", "value = 1\nvalue = 2"),
    ]);
    cfg.structured = vec![perfect_doc::config::StructuredFile {
        glob: "*.toml".into(),
        schema: None,
    }];
    let report = scan(&dir, &cfg);
    assert!(has(&report, "structured.syntax"));
    assert_eq!(report.exit_code(), 1);
}
#[test]
fn metadata_links_use_the_same_resolver() {
    let (dir, mut cfg) = project(&[("a.md", "---\nrelated: [missing.md]\n---\n# A")]);
    cfg.frontmatter.link_fields = vec!["related".into()];
    assert!(has(&scan(&dir, &cfg), "link.exists"));
}
#[test]
fn routes_aliases_redirects_and_relative_links() {
    let (dir, mut cfg) = project(&[
        (
            "a.md",
            "---\nroute: /guide/\naliases: [/old/]\n---\n# Guide\n[next](next/#next)",
        ),
        ("next.md", "---\nroute: /guide/next/\n---\n# Next"),
        (
            "index.md",
            "[guide](/old/#guide)\n[redirect](/redirect/#next)",
        ),
    ]);
    cfg.routes.enabled = true;
    cfg.frontmatter.route_field = Some("route".into());
    cfg.frontmatter.aliases_field = Some("aliases".into());
    cfg.routes.redirects = BTreeMap::from([("/redirect/".into(), "/guide/next/".into())]);
    let report = scan(&dir, &cfg);
    assert_eq!(report.exit_code(), 0, "{:?}", report.diagnostics);
}
#[test]
fn route_collisions_and_redirect_cycles_fail() {
    let (dir, mut cfg) = project(&[
        ("a.md", "---\nroute: /same/\n---\n# A"),
        ("b.md", "---\nroute: /same/\n---\n# B"),
    ]);
    cfg.routes.enabled = true;
    cfg.frontmatter.route_field = Some("route".into());
    cfg.routes.redirects = BTreeMap::from([
        ("/one".into(), "/two".into()),
        ("/two".into(), "/one".into()),
    ]);
    let report = scan(&dir, &cfg);
    assert!(has(&report, "route.unique"));
    assert!(has(&report, "route.target"));
}
#[test]
fn ordinary_link_cycles_pass_but_include_cycles_fail() {
    let (dir, mut cfg) = project(&[
        ("a.md", "[B](b.md)\n!include[b.md]"),
        ("b.md", "[A](a.md)\n!include[a.md]"),
    ]);
    assert_eq!(scan(&dir, &cfg).exit_code(), 0);
    cfg.markdown.includes = true;
    assert!(has(&scan(&dir, &cfg), "include.cycle"));
}
#[test]
fn include_regions_and_ranges_are_checked() {
    let (dir, mut cfg) = project(&[
        ("a.md", "!include[snippet.txt]{region=piece}"),
        (
            "snippet.txt",
            "<!-- region piece -->\ntext\n<!-- endregion piece -->",
        ),
    ]);
    cfg.markdown.includes = true;
    assert_eq!(scan(&dir, &cfg).exit_code(), 0);
    write(
        dir.path(),
        "a.md",
        "!include[snippet.txt]{region=lost start=1 end=99}",
    );
    assert!(has(&scan(&dir, &cfg), "include.target"));
}
#[test]
fn navigation_coverage_and_reachability() {
    let (dir, mut cfg) = project(&[
        ("index.md", "# Index\n[child](child.md)"),
        ("child.md", "# Child"),
        ("orphan.md", "# Orphan"),
        (
            "nav.json",
            "[{\"path\":\"index.md\",\"children\":[{\"path\":\"child.md\"}]}]",
        ),
    ]);
    cfg.collection.navigation = vec![perfect_doc::config::NavigationFile {
        path: "nav.json".into(),
        ..Default::default()
    }];
    cfg.collection.require_navigation_coverage = true;
    cfg.collection.require_reachable = true;
    cfg.collection.entrypoints = vec!["index.md".into()];
    let report = scan(&dir, &cfg);
    assert!(has(&report, "navigation.structure"));
    assert!(has(&report, "collection.reachable"));
}
#[test]
fn metadata_parent_cycles_are_separate_from_link_cycles() {
    let (dir, mut cfg) = project(&[
        ("a.md", "---\nid: a\nparent: b\n---\n# A"),
        ("b.md", "---\nid: b\nparent: a\n---\n# B"),
    ]);
    cfg.frontmatter.id_field = Some("id".into());
    cfg.frontmatter.parent_field = Some("parent".into());
    assert!(has(&scan(&dir, &cfg), "navigation.structure"));
}
#[test]
fn mixed_markdown_html_id_associations_and_duplicate_anchors() {
    let (dir, mut cfg) = project(&[(
        "a.md",
        "# Label {#field}\n<label for=\"field\">Name</label>\n<input id=\"field\" aria-label=\"Name\">\n",
    )]);
    cfg.markdown.explicit_heading_ids = true;
    let report = scan(&dir, &cfg);
    assert!(has(&report, "anchor.unique"));
}
#[test]
fn html_base_applies_to_assets_and_links() {
    let (dir, cfg) = project(&[
        (
            "a.html",
            "<base href=\"folder/\"><a href=\"b.html#b\">go</a>",
        ),
        ("folder/b.html", "<p id=\"b\">B</p>"),
    ]);
    let report = scan(&dir, &cfg);
    assert_eq!(report.exit_code(), 0, "{:?}", report.diagnostics);
}
#[test]
fn static_id_associations_check_target_type() {
    let (dir, cfg) = project(&[(
        "a.html",
        "<label for=\"p\">Name</label><p id=\"p\">Text</p>",
    )]);
    assert!(has(&scan(&dir, &cfg), "link.type"));
}
#[test]
fn svg_id_references_and_css_imports_are_checked() {
    let (dir, cfg) = project(&[
        (
            "a.html",
            "<img alt=\"\" src=\"a.svg#shape\"><link rel=\"stylesheet\" href=\"style.css\">",
        ),
        (
            "a.svg",
            "<svg xmlns=\"http://www.w3.org/2000/svg\"><rect id=\"shape\"/></svg>",
        ),
        (
            "style.css",
            "@import 'more.css'; body { background: url(missing.png); }",
        ),
        ("more.css", "p {color: red;}"),
    ]);
    let report = scan(&dir, &cfg);
    assert!(!has(&report, "link.anchor"));
    assert!(has(&report, "link.exists"));
}
#[test]
fn asset_image_bytes_and_integrity_are_checked() {
    let (dir, cfg) = project(&[
        (
            "a.html",
            "<img alt=\"\" src=\"bad.png\"><script src=\"code.js\" integrity=\"sha256-YQ==\"></script>",
        ),
        ("bad.png", "not an image"),
        ("code.js", "// literal; never run"),
    ]);
    let report = scan(&dir, &cfg);
    assert!(has(&report, "asset.syntax"));
    assert!(has(&report, "asset.integrity"));
}
#[test]
fn image_dimension_policy_allows_scaling_by_default() {
    let (dir, mut cfg) = project(&[("a.html", "<img alt=\"\" width=\"4\" src=\"a.png\">")]);
    image::RgbaImage::new(1, 1)
        .save(dir.path().join("a.png"))
        .unwrap();
    assert_eq!(scan(&dir, &cfg).exit_code(), 0);
    cfg.assets.image_dimensions = true;
    assert!(has(&scan(&dir, &cfg), "asset.dimensions"));
}
#[test]
fn data_uris_decode_and_do_not_use_the_file_system() {
    let (dir, cfg) = project(&[(
        "a.html",
        "<img alt=\"\" src=\"data:image/svg+xml,%3Csvg%20xmlns%3D%22http%3A%2F%2Fwww.w3.org%2F2000%2Fsvg%22%2F%3E\">",
    )]);
    assert_eq!(scan(&dir, &cfg).exit_code(), 0);
    write(
        dir.path(),
        "a.html",
        "<img alt=\"\" src=\"data:image/png;base64,not@base64\">",
    );
    assert!(has(&scan(&dir, &cfg), "uri.syntax"));
}
#[test]
fn output_contract_checks_generated_anchor_preservation() {
    let (dir, mut cfg) = project(&[
        ("index.md", "# Welcome"),
        (
            "build/index.html",
            "<!doctype html><html lang=\"en\"><head><title>Page</title></head><body><h1 id=\"welcome\">Welcome</h1></body></html>",
        ),
    ]);
    cfg.exclude.push("build/**".into());
    cfg.routes.enabled = true;
    cfg.routes.output_dir = Some("build".into());
    cfg.routes.require_output = true;
    assert_eq!(scan(&dir, &cfg).exit_code(), 0);
    write(
        dir.path(),
        "build/index.html",
        "<!doctype html><html><head><title>Page</title></head><body><h1>Welcome</h1></body></html>",
    );
    assert!(has(&scan(&dir, &cfg), "build.output"));
}
#[test]
fn explicit_public_anchor_contract_catches_removals() {
    let (dir, mut cfg) = project(&[("a.md", "# Existing")]);
    cfg.routes.preserved_anchors = BTreeMap::from([("a.md".into(), vec!["previous".into()])]);
    assert!(has(&scan(&dir, &cfg), "anchor.preserved"));
}
#[test]
fn scoped_exceptions_keep_evidence_visible() {
    let (dir, mut cfg) = project(&[("a.md", "[bad](missing.md)"), ("b.md", "[bad](missing.md)")]);
    cfg.exceptions = vec![perfect_doc::config::Exception {
        rule: "link.exists".into(),
        path: "a.md".into(),
        reason: "fixture needs this broken reference".into(),
        expires: None,
    }];
    let report = scan(&dir, &cfg);
    assert_eq!(report.exit_code(), 1);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.location.path == "a.md" && d.suppression.is_some())
    );
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.location.path == "b.md" && d.suppression.is_none())
    );
}
#[test]
fn expired_and_unbalanced_exceptions_cannot_pass() {
    let (dir, mut cfg) = project(&[(
        "a.md",
        "<!-- perfect-doc-disable link.exists : deliberate sample -->\n[bad](missing.md)",
    )]);
    cfg.exceptions = vec![perfect_doc::config::Exception {
        rule: "link.exists".into(),
        path: "a.md".into(),
        reason: "old exception".into(),
        expires: Some("2000-01-01".into()),
    }];
    let report = scan(&dir, &cfg);
    assert!(has(&report, "config.exception"));
    assert_eq!(report.exit_code(), 1);
}
#[test]
fn balanced_inline_suppressions_do_not_hide_other_rules() {
    let (dir, cfg) = project(&[(
        "a.md",
        "<!-- perfect-doc-disable link.exists : deliberate example -->\n[bad](missing.md)\n<!-- perfect-doc-enable link.exists -->\n[other](other.md)",
    )]);
    let report = scan(&dir, &cfg);
    assert!(!has(&report, "config.exception"));
    assert_eq!(
        report
            .diagnostics
            .iter()
            .filter(|d| d.rule == "link.exists" && d.suppression.is_some())
            .count(),
        1
    );
    assert_eq!(
        report
            .diagnostics
            .iter()
            .filter(|d| d.rule == "link.exists" && d.suppression.is_none())
            .count(),
        1
    );
}
#[test]
fn strict_configuration_rejects_typos_and_unknown_rules() {
    let (dir, mut cfg) = project(&[("a.md", "# A")]);
    write(dir.path(), "bad.toml", "[markdown]\nheading_strat = 1");
    assert!(Config::from_file(&dir.path().join("bad.toml")).is_err());
    cfg.rules.disable = vec!["invented.rule".into()];
    assert!(validate(&[dir.path().into()], &cfg).is_err());
}
#[test]
fn naming_policies_and_case_collisions_are_explicit() {
    let (dir, mut cfg) = project(&[("CON.md", "# A"), ("a.md", "# A"), ("A.md", "# B")]);
    cfg.files.portable_names = true;
    let report = scan(&dir, &cfg);
    assert!(has(&report, "name.valid"));
    if dir.path().read_dir().unwrap().count() == 3 {
        assert!(has(&report, "name.collision"));
    }
}
#[test]
fn hidden_uri_controls_and_invalid_escapes_fail() {
    let (dir, mut cfg) = project(&[("a.md", "[bad](other%XZ.md)\n[hidden](other\u{200b}.md)")]);
    cfg.files.hidden_characters = true;
    let report = scan(&dir, &cfg);
    assert!(has(&report, "uri.syntax"));
    assert!(has(&report, "source.hidden"));
}
#[test]
fn severity_does_not_make_required_incomplete_checks_pass() {
    let (dir, mut cfg) = project(&[("a.md", "[unknown](a.bin#id)"), ("a.bin", "data")]);
    cfg.rules.severity = BTreeMap::from([("link.available".into(), Severity::Info)]);
    let report = scan(&dir, &cfg);
    assert_eq!(report.exit_code(), 3);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.outcome == Outcome::Unverified)
    );
}
#[test]
fn cancelled_scans_are_incomplete() {
    let (dir, cfg) = project(&[("a.md", "# A")]);
    let token = std::sync::atomic::AtomicBool::new(true);
    let report = perfect_doc::validate_with_cancel(&[dir.path().into()], &cfg, &token).unwrap();
    assert_eq!(report.exit_code(), 3);
}
#[test]
fn report_output_is_deterministic_offline() {
    let (dir, cfg) = project(&[("b.md", "[bad](missing.md)"), ("a.md", "[bad](missing.md)")]);
    let first = perfect_doc::report::json_report(&scan(&dir, &cfg)).unwrap();
    let second = perfect_doc::report::json_report(&scan(&dir, &cfg)).unwrap();
    assert_eq!(first, second);
}
#[test]
fn junit_and_sarif_retain_failures_and_incomplete_evidence() {
    let (dir, cfg) = project(&[
        ("a.md", "[bad](missing.md)\n[unknown](a.bin#id)"),
        ("a.bin", "data"),
    ]);
    let report = scan(&dir, &cfg);
    let junit = perfect_doc::report::junit(&report);
    let xml = roxmltree::Document::parse(&junit).unwrap();
    assert!(xml.descendants().any(|n| n.has_tag_name("failure")));
    assert!(xml.descendants().any(|n| n.has_tag_name("error")));
    let sarif: serde_json::Value =
        serde_json::from_str(&perfect_doc::report::sarif(&report).unwrap()).unwrap();
    assert_eq!(sarif["runs"][0]["columnKind"], "unicodeCodePoints");
    assert_eq!(
        sarif["runs"][0]["invocations"][0]["executionSuccessful"],
        false
    );
}
#[cfg(unix)]
#[test]
fn links_cannot_escape_via_symlinks_or_parent_paths() {
    use std::os::unix::fs::symlink;
    let outside = TempDir::new().unwrap();
    write(outside.path(), "target.md", "# Target");
    let (dir, cfg) = project(&[("a.md", "[outside](linked.md)")]);
    symlink(
        outside.path().join("target.md"),
        dir.path().join("linked.md"),
    )
    .unwrap();
    assert!(has(&scan(&dir, &cfg), "link.boundary"));
}

proptest::proptest! {
    #[test]
    fn unicode_source_columns_are_scalar_counts(prefix in "[^\\p{Cc}\\r\\n]{0,40}") {
        let text=format!("{prefix}x");let source=perfect_doc::model::Source::new("a.md".into(),"a.md".into(),text);
        let at=source.location(prefix.len());proptest::prop_assert_eq!(at.column,prefix.chars().count()+1);proptest::prop_assert_eq!(at.byte_offset,prefix.len());
    }
    #[test]
    fn local_link_identity_survives_file_name_encoding(name in "[a-z]{1,20}") {
        let target=format!("{name} 雪.md");let url=format!("{name}%20%E9%9B%AA.md#target");let text=format!("[go]({url})");
        let(dir,cfg)=project(&[("index.md",&text),(&target,"# Target")]);proptest::prop_assert_eq!(scan(&dir,&cfg).exit_code(),0);
    }
}

#[test]
fn xml_cannot_pass_a_json_schema_contract_as_a_null_value() {
    let (dir, mut config) = project(&[
        ("page.md", "# Page"),
        ("record.xml", "<record><id>item</id></record>"),
        ("schema.json", "{\"type\":\"null\"}"),
    ]);
    config.structured.push(perfect_doc::config::StructuredFile {
        glob: "record.xml".into(),
        schema: Some("schema.json".into()),
    });
    let report = scan(&dir, &config);
    assert_eq!(report.exit_code(), 3);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.rule == "schema.available"
                && d.outcome == Outcome::Unsupported
                && d.target.as_deref() == Some("schema.json"))
    );
}

#[test]
fn xml_example_schema_is_unsupported_instead_of_a_false_data_failure() {
    let (dir, mut config) = project(&[
        ("page.md", "# Page\n```xml\n<record/>\n```"),
        ("schema.json", "{\"type\":\"object\"}"),
    ]);
    config
        .markdown
        .example_schemas
        .insert("xml".into(), "schema.json".into());
    let report = scan(&dir, &config);
    assert_eq!(report.exit_code(), 3);
    assert!(has(&report, "schema.available"));
    assert!(!has(&report, "schema.valid"));
}
