use perfect_doc::{Config, Outcome, validate};
use std::fs;

fn check(text: &str) -> perfect_doc::Report {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("page.html"), text).unwrap();
    let config = Config {
        base_dir: directory.path().into(),
        ..Config::default()
    };
    validate(&[directory.path().into()], &config).unwrap()
}

#[test]
fn legal_optional_tags_and_script_literals_complete() {
    let report = check(
        "<!doctype html><title>Example</title><ul><li>One<li>Two</ul><script>const text = '<img src=missing>'; </script>",
    );
    assert_eq!(report.exit_code(), 0, "{:?}", report.diagnostics);
}

#[test]
fn wrong_association_target_types_fail() {
    for text in [
        "<label for=x>Name</label><div id=x></div>",
        "<input aria-label=Name form=x><div id=x></div>",
        "<input aria-label=Name list=x><div id=x></div>",
        "<table><tr><td id=x>A</td><td headers=x>B</td></tr></table>",
    ] {
        let report = check(text);
        assert_eq!(report.exit_code(), 1, "{text}: {:?}", report.diagnostics);
        assert!(
            report
                .diagnostics
                .iter()
                .any(|d| d.outcome == Outcome::Invalid && d.rule == "link.type"),
            "{:?}",
            report.diagnostics
        );
    }
}

#[test]
fn valid_static_labels_and_forward_references_complete() {
    let report = check(
        "<label for=n>Name</label><input id=n form=f list=d><form id=f></form><datalist id=d><option value=one></datalist>",
    );
    assert_eq!(report.exit_code(), 0, "{:?}", report.diagnostics);
}

#[test]
fn malformed_source_and_duplicate_ids_fail() {
    for text in [
        "<div id=x id=y></div>",
        "<div id=x></div><span id=x></span>",
        "<div><span>Text</div>",
    ] {
        let report = check(text);
        assert_eq!(report.exit_code(), 1, "{text}: {:?}", report.diagnostics);
    }
}

#[test]
fn dynamic_identifier_remains_unverified() {
    let report = check("<div id='{{ name }}'></div>");
    assert_eq!(report.exit_code(), 3, "{:?}", report.diagnostics);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.rule == "html.dynamic" && d.outcome == Outcome::Unverified)
    );
}

#[test]
fn unicode_diagnostic_locations_retain_source_bytes() {
    let report = check("é\n<div id=x id=y></div>");
    let diagnostic = report
        .diagnostics
        .iter()
        .find(|d| d.message.contains("duplicate-attribute"))
        .unwrap();
    assert_eq!(diagnostic.location.line, 2);
    assert!(diagnostic.location.byte_offset >= 3);
}

#[test]
fn aria_relationships_need_real_identifiers() {
    let report = check("<button aria-labelledby=missing>Save</button>");
    assert_eq!(report.exit_code(), 1, "{:?}", report.diagnostics);
}
