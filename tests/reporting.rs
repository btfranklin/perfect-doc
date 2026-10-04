use perfect_doc::{Config, Outcome, Report, report, validate};
use std::fs;

fn scan(files: &[(&str, &str)], edit: impl FnOnce(&mut Config)) -> Report {
    let directory = tempfile::tempdir().unwrap();
    for (path, text) in files {
        fs::write(directory.path().join(path), text).unwrap();
    }
    let mut config = Config {
        base_dir: directory.path().into(),
        ..Config::default()
    };
    edit(&mut config);
    validate(&[directory.path().into()], &config).unwrap()
}

#[test]
fn every_report_format_has_source_target_and_repair_details() {
    let result = scan(
        &[
            ("a.md", "# A\n[Missing](gone.md)\n[Anchor](b.md#lost)"),
            ("b.md", "# Present"),
        ],
        |_| {},
    );
    let missing = result
        .diagnostics
        .iter()
        .find(|d| d.rule == "link.exists")
        .unwrap();
    assert_eq!(missing.location.path, "a.md");
    assert_eq!(missing.location.line, 2);
    assert_eq!(missing.target.as_deref(), Some("gone.md"));
    assert!(missing.help.contains("Create the missing target"));
    let anchor = result
        .diagnostics
        .iter()
        .find(|d| d.rule == "link.anchor")
        .unwrap();
    assert_eq!(anchor.target.as_deref(), Some("b.md#lost"));
    assert!(anchor.help.contains("identifier"));
    let human = report::human(&result);
    assert!(human.contains("a.md:2:1"));
    assert!(human.contains("Target: gone.md"));
    assert!(human.contains("Repair: Create the missing target"));
    let json: serde_json::Value =
        serde_json::from_str(&report::json_report(&result).unwrap()).unwrap();
    assert_eq!(json["diagnostics"][0]["target"], "gone.md");
    assert!(
        json["diagnostics"][0]["help"]
            .as_str()
            .unwrap()
            .contains("Create")
    );
    let junit = report::junit(&result);
    let junit = roxmltree::Document::parse(&junit).unwrap();
    let failure = junit
        .descendants()
        .find(|node| node.has_tag_name("failure"))
        .unwrap();
    assert!(failure.text().unwrap().contains("Target: gone.md"));
    assert!(failure.text().unwrap().contains("Repair:"));
    let sarif: serde_json::Value = serde_json::from_str(&report::sarif(&result).unwrap()).unwrap();
    let failure = &sarif["runs"][0]["results"][0];
    assert_eq!(failure["properties"]["target"], "gone.md");
    assert!(
        failure["message"]["text"]
            .as_str()
            .unwrap()
            .contains("Repair:")
    );
}

#[test]
fn schema_report_identifies_the_field_and_failed_constraint() {
    let result = scan(&[("page.md", "---\ncount: 0\n---\n# Page")], |config| {
        config.frontmatter.inline_schema = Some(
            serde_json::json!({"type":"object", "properties":{"count":{"type":"integer","minimum":1}}}),
        );
    });
    let finding = result
        .diagnostics
        .iter()
        .find(|d| d.rule == "schema.valid")
        .unwrap();
    assert!(finding.message.contains("/count"));
    assert!(finding.message.contains("Minimum"));
    assert!(finding.message.contains('1'));
    assert!(
        finding
            .target
            .as_deref()
            .unwrap()
            .contains("/properties/count/minimum")
    );
    assert!(finding.help.contains("instance path"));
}

#[test]
fn sarif_file_paths_escape_literal_percent_and_unicode() {
    let result = scan(&[("snow%20雪.md", "[Missing](gone.md)")], |_| {});
    let sarif: serde_json::Value = serde_json::from_str(&report::sarif(&result).unwrap()).unwrap();
    let uri = sarif["runs"][0]["results"][0]["locations"][0]["physicalLocation"]["artifactLocation"]["uri"].as_str().unwrap();
    assert!(uri.contains("%2520"));
    assert!(uri.contains("%E9%9B%AA"));
    assert_eq!(
        percent_encoding::percent_decode_str(uri)
            .decode_utf8()
            .unwrap(),
        "snow%20雪.md"
    );
}

#[test]
fn coverage_distinguishes_available_rules_from_enabled_policies() {
    let result = scan(&[("page.md", "# Page")], |config| {
        config.markdown.no_heading_skips = true
    });
    assert!(
        result
            .coverage
            .available_rules
            .iter()
            .any(|id| id == "external.response")
    );
    assert!(
        !result
            .coverage
            .enabled_rules
            .iter()
            .any(|id| id == "external.response")
    );
    assert!(
        !result
            .coverage
            .enabled_rules
            .iter()
            .any(|id| id == "external.cache")
    );
    assert!(
        result
            .coverage
            .enabled_rules
            .iter()
            .any(|id| id == "markdown.heading-shape")
    );
}

#[test]
fn mandatory_discovery_failure_cannot_be_suppressed() {
    let result = scan(&[], |config| {
        config.rules.disable.push("discovery.complete".into())
    });
    assert_eq!(result.exit_code(), 3);
    assert!(
        result
            .diagnostics
            .iter()
            .any(|d| d.rule == "discovery.complete"
                && d.outcome == Outcome::Unverified
                && d.suppression.is_none())
    );
}
