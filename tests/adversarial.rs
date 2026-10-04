use perfect_doc::{Config, validate};
use std::fs;

fn fixture(files: &[(&str, &str)], edit: impl FnOnce(&mut Config)) -> perfect_doc::Report {
    let directory = tempfile::tempdir().unwrap();
    for (path, text) in files {
        let path = directory.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    let mut config = Config {
        base_dir: fs::canonicalize(directory.path()).unwrap(),
        ..Config::default()
    };
    edit(&mut config);
    validate(&[directory.path().into()], &config).unwrap()
}

#[test]
fn svg_local_symbol_reference_resolves() {
    let report = fixture(
        &[
            ("page.html", "<img src='drawing.svg' alt=''>"),
            (
                "drawing.svg",
                "<svg xmlns='http://www.w3.org/2000/svg'><symbol id='mark'><path d='M0 0'/></symbol><use href='#mark'/></svg>",
            ),
        ],
        |_| {},
    );
    assert_eq!(report.exit_code(), 0, "{:?}", report.diagnostics);
}

#[test]
fn malformed_embedded_json_is_checked_without_example_policy() {
    let report = fixture(
        &[(
            "page.html",
            "<script type='application/ld+json'>{broken}</script>",
        )],
        |_| {},
    );
    assert_eq!(report.exit_code(), 1, "{:?}", report.diagnostics);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.rule == "structured.syntax")
    );
}

#[test]
fn configured_utf8_encoding_alias_is_supported() {
    let report = fixture(&[("page.html", "<p>Text</p>")], |config| {
        config.files.html_encoding = "UTF8".into()
    });
    assert_eq!(report.exit_code(), 0, "{:?}", report.diagnostics);
}

#[test]
fn percent_encoded_base64_data_is_decoded_before_base64() {
    let report = fixture(
        &[(
            "page.html",
            "<a href='data:application/json;base64,eyJ4IjoxfQ%3D%3D'>Data</a>",
        )],
        |_| {},
    );
    assert_eq!(report.exit_code(), 0, "{:?}", report.diagnostics);
}

#[test]
fn data_uri_scheme_is_case_insensitive() {
    let report = fixture(
        &[(
            "page.html",
            "<a href='DATA:application/json,%7B%22x%22:1%7D'>Data</a>",
        )],
        |_| {},
    );
    assert_eq!(report.exit_code(), 0, "{:?}", report.diagnostics);
}

#[test]
fn encoded_declared_route_resolves_its_encoded_link() {
    let report = fixture(
        &[
            ("index.md", "[Page](/snow%20hill#page)"),
            ("page.md", "---\nroute: /snow%20hill\n---\n# Page"),
        ],
        |config| {
            config.routes.enabled = true;
            config.frontmatter.route_field = Some("route".into());
        },
    );
    assert_eq!(report.exit_code(), 0, "{:?}", report.diagnostics);
}

#[test]
fn generated_collection_can_resolve_cross_page_fragments() {
    let report = fixture(
        &[
            ("index.md", "# Home\n[Page](page.md#page)"),
            ("page.md", "# Page"),
            (
                "build/index.html",
                "<!doctype html><title>Home</title><h1 id=home>Home</h1><a href='page/index.html#page'>Page</a>",
            ),
            (
                "build/page/index.html",
                "<!doctype html><title>Page</title><h1 id=page>Page</h1>",
            ),
        ],
        |config| {
            config.exclude.push("build/**".into());
            config.routes.enabled = true;
            config.routes.output_dir = Some("build".into());
            config.routes.require_output = true;
        },
    );
    assert_eq!(report.exit_code(), 0, "{:?}", report.diagnostics);
}

#[test]
fn generated_output_assets_receive_structural_checks() {
    let report = fixture(
        &[
            ("index.md", "# Home"),
            (
                "build/index.html",
                "<!doctype html><title>Home</title><h1 id=home>Home</h1><img src='invalid.png' alt=''>",
            ),
            ("build/invalid.png", "invalid image bytes"),
        ],
        |config| {
            config.exclude.push("build/**".into());
            config.routes.enabled = true;
            config.routes.output_dir = Some("build".into());
            config.routes.require_output = true;
        },
    );
    assert_eq!(report.exit_code(), 1, "{:?}", report.diagnostics);
    assert!(report.diagnostics.iter().any(|d| d.rule == "asset.syntax"));
}

#[test]
fn navigation_fragments_must_exist() {
    let report = fixture(
        &[
            ("index.md", "# Home"),
            ("nav.json", "[{\"path\":\"index.md#missing\"}]"),
        ],
        |config| {
            config.collection.navigation = vec![perfect_doc::config::NavigationFile {
                path: "nav.json".into(),
                ..Default::default()
            }];
        },
    );
    assert_ne!(report.exit_code(), 0, "{:?}", report.diagnostics);
}

#[test]
fn sitemap_locations_must_have_declared_local_routes() {
    let report = fixture(
        &[
            ("index.md", "# Home"),
            (
                "sitemap.xml",
                "<urlset xmlns='http://www.sitemaps.org/schemas/sitemap/0.9'><url><loc>https://docs.invalid/missing</loc></url></urlset>",
            ),
        ],
        |config| {
            config.routes.enabled = true;
            config.assets.sitemap = vec!["sitemap.xml".into()];
        },
    );
    assert_ne!(report.exit_code(), 0, "{:?}", report.diagnostics);
}

#[test]
fn literal_percent_route_is_not_decoded_twice() {
    let report = fixture(
        &[
            ("index.md", "[Page](/page%2520#page)"),
            ("page.md", "---\nroute: /page%2520\n---\n# Page"),
        ],
        |config| {
            config.routes.enabled = true;
            config.frontmatter.route_field = Some("route".into());
        },
    );
    assert_eq!(report.exit_code(), 0, "{:?}", report.diagnostics);
}

#[test]
fn relative_links_from_encoded_parent_routes_resolve() {
    let report = fixture(
        &[
            (
                "home.md",
                "---\nroute: /snow%20hill/\n---\n# Home\n[Page](page/#page)",
            ),
            ("page.md", "---\nroute: /snow%20hill/page/\n---\n# Page"),
        ],
        |config| {
            config.routes.enabled = true;
            config.frontmatter.route_field = Some("route".into());
        },
    );
    assert_eq!(report.exit_code(), 0, "{:?}", report.diagnostics);
}

#[test]
fn route_declarations_reject_encoded_separators_and_parent_segments() {
    for route in ["/section%2fpage", "/section%5cpage", "/%2e%2e/page"] {
        let text = format!("---\nroute: {route}\n---\n# Page");
        let report = fixture(&[("page.md", &text)], |config| {
            config.routes.enabled = true;
            config.frontmatter.route_field = Some("route".into());
        });
        assert_eq!(report.exit_code(), 1, "{route}: {:?}", report.diagnostics);
    }
}

#[test]
fn flat_output_routes_keep_the_html_file_extension() {
    let report = fixture(
        &[
            ("page.name.md", "# Page"),
            (
                "build/page.name.html",
                "<!doctype html><title>Page</title><h1 id=page>Page</h1>",
            ),
        ],
        |config| {
            config.exclude.push("build/**".into());
            config.routes.enabled = true;
            config.routes.directory_urls = false;
            config.routes.output_dir = Some("build".into());
            config.routes.require_output = true;
        },
    );
    assert_eq!(report.exit_code(), 0, "{:?}", report.diagnostics);
}

#[test]
fn code_file_references_do_not_create_include_recursion() {
    let report = fixture(
        &[
            ("a.md", "```markdown {file=b.md}\n# Example\n```"),
            ("b.md", "```markdown {file=a.md}\n# Other example\n```"),
        ],
        |_| {},
    );
    assert_eq!(report.exit_code(), 0, "{:?}", report.diagnostics);
    let missing = fixture(
        &[("a.md", "```text {file=missing.txt}\nExample\n```")],
        |_| {},
    );
    assert_eq!(missing.exit_code(), 1, "{:?}", missing.diagnostics);
}

#[test]
fn generated_scan_resets_source_selection_and_contracts() {
    let report = fixture(
        &[
            ("page.md", "---\nid: page\n---\n# Page"),
            (
                "build/page/index.html",
                "<!doctype html><title>Page</title><h1 id=page>Page</h1>",
            ),
        ],
        |config| {
            config.include = vec!["*.md".into()];
            config.exclude.push("build/**".into());
            config.frontmatter.required = true;
            config.frontmatter.required_fields = vec!["id".into()];
            config.required_paths = vec!["*.md".into()];
            config.routes.enabled = true;
            config.routes.output_dir = Some("build".into());
            config.routes.require_output = true;
        },
    );
    assert_eq!(report.exit_code(), 0, "{:?}", report.diagnostics);
}

#[test]
fn generated_scan_file_size_limit_is_incomplete() {
    let report = fixture(
        &[
            ("index.md", "# Home"),
            (
                "build/index.html",
                "<!doctype html><title>Home</title><h1 id=home>Home</h1>",
            ),
        ],
        |config| {
            config.exclude.push("build/**".into());
            config.files.max_file_bytes = 12;
            config.routes.enabled = true;
            config.routes.output_dir = Some("build".into());
            config.routes.require_output = true;
        },
    );
    assert_eq!(report.exit_code(), 3, "{:?}", report.diagnostics);
}

#[test]
fn generated_base_url_resolves_assets_inside_complete_output() {
    let report = fixture(
        &[
            ("index.md", "# Home"),
            (
                "build/index.html",
                "<!doctype html><title>Home</title><base href='shared/'><h1 id=home>Home</h1><img src='drawing.svg' alt=''>",
            ),
            (
                "build/shared/drawing.svg",
                "<svg xmlns='http://www.w3.org/2000/svg'/>",
            ),
        ],
        |config| {
            config.exclude.push("build/**".into());
            config.routes.enabled = true;
            config.routes.output_dir = Some("build".into());
            config.routes.require_output = true;
        },
    );
    assert_eq!(report.exit_code(), 0, "{:?}", report.diagnostics);
}

#[test]
fn generated_total_byte_budget_is_incomplete() {
    let report = fixture(
        &[
            ("a.md", "# A"),
            ("b.md", "# B"),
            (
                "build/a/index.html",
                "<!doctype html><title>A</title><h1 id=a>A</h1>",
            ),
            (
                "build/b/index.html",
                "<!doctype html><title>B</title><h1 id=b>B</h1>",
            ),
        ],
        |config| {
            config.exclude.push("build/**".into());
            config.files.max_total_bytes = 60;
            config.routes.enabled = true;
            config.routes.output_dir = Some("build".into());
            config.routes.require_output = true;
        },
    );
    assert_eq!(report.exit_code(), 3, "{:?}", report.diagnostics);
}

#[test]
fn relative_links_keep_literal_percent_in_unicode_parent_routes() {
    let report = fixture(
        &[
            (
                "home.md",
                "---\nroute: /雪%2520/\n---\n# Home\n[Page](page/#page)",
            ),
            ("page.md", "---\nroute: /雪%2520/page/\n---\n# Page"),
        ],
        |config| {
            config.routes.enabled = true;
            config.frontmatter.route_field = Some("route".into());
        },
    );
    assert_eq!(report.exit_code(), 0, "{:?}", report.diagnostics);
}

#[test]
fn generated_diagnostics_keep_the_outer_path_namespace() {
    let report = fixture(
        &[
            ("index.md", "# Home"),
            (
                "build/index.html",
                "<!doctype html><title>Home</title><h1 id=home>Home</h1><a href='missing.html'>Missing</a>",
            ),
        ],
        |config| {
            config.exclude.push("build/**".into());
            config.routes.enabled = true;
            config.routes.output_dir = Some("build".into());
            config.routes.require_output = true;
            config.exceptions = vec![perfect_doc::config::Exception {
                rule: "link.exists".into(),
                path: "build/index.html".into(),
                reason: "A test with a deliberate missing link".into(),
                expires: None,
            }];
            config.rules.unused_exceptions = true;
        },
    );
    assert_eq!(report.exit_code(), 0, "{:?}", report.diagnostics);
    let finding = report
        .diagnostics
        .iter()
        .find(|d| d.rule == "link.exists")
        .unwrap();
    assert_eq!(finding.location.path, "build/index.html");
    assert!(finding.suppression.is_some());
}
