//! Render a completed report without changing its evidence.

use crate::model::{Diagnostic, Outcome, Report, Severity};
use serde_json::json;
use std::fmt::Write;

pub fn human(report: &Report) -> String {
    let mut text = String::new();
    for diagnostic in &report.diagnostics {
        let suppressed = diagnostic
            .suppression
            .as_ref()
            .map(|reason| format!(" [suppressed: {reason}]"))
            .unwrap_or_default();
        let _ = writeln!(
            text,
            "{}:{}:{}: {:?} {} ({:?}): {}{}",
            diagnostic.location.path,
            diagnostic.location.line,
            diagnostic.location.column,
            diagnostic.severity,
            diagnostic.rule,
            diagnostic.outcome,
            diagnostic.message,
            suppressed
        );
        if let Some(target) = &diagnostic.target {
            let _ = writeln!(text, "  Target: {target}");
        }
        let _ = writeln!(text, "  Repair: {}", diagnostic.help);
        for related in &diagnostic.related {
            let _ = writeln!(
                text,
                "  Related: {}:{}:{}",
                related.path, related.line, related.column
            );
        }
    }
    let status = match report.exit_code() {
        0 => "Passed",
        1 => "Failed",
        _ => "Incomplete",
    };
    let _ = writeln!(
        text,
        "{status}: {} documents selected, {} parsed, {} references; {} local references verified.",
        report.coverage.documents,
        report.coverage.parsed,
        report.coverage.references,
        report.coverage.local_verified
    );
    if report.network_enabled {
        let _ = writeln!(
            text,
            "External URLs: {} selected, {} verified.",
            report.coverage.external_urls, report.coverage.external_verified
        );
    } else {
        let _ = writeln!(text, "External reachability was not requested.");
    }
    text
}

pub fn json_report(report: &Report) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(report)
}

fn xml(value: &str) -> String {
    value
        .chars()
        .map(|c| match c {
            '&' => "&amp;".into(),
            '<' => "&lt;".into(),
            '>' => "&gt;".into(),
            '"' => "&quot;".into(),
            '\'' => "&apos;".into(),
            '\n' => "&#10;".into(),
            '\r' => "&#13;".into(),
            '\t' => "&#9;".into(),
            c if c == '\u{fffe}' || c == '\u{ffff}' || c < ' ' => "\u{fffd}".into(),
            c => c.to_string(),
        })
        .collect()
}

fn failure(d: &Diagnostic, report: &Report) -> bool {
    d.suppression.is_none() && d.outcome == Outcome::Invalid && d.severity >= report.fail_on
}
fn incomplete(d: &Diagnostic) -> bool {
    d.suppression.is_none()
        && d.required
        && matches!(d.outcome, Outcome::Unverified | Outcome::Unsupported)
}

fn detail(d: &Diagnostic) -> String {
    let mut text = format!(
        "{}:{}:{}: {}",
        d.location.path, d.location.line, d.location.column, d.message
    );
    if let Some(target) = &d.target {
        let _ = write!(text, "\nTarget: {target}");
    }
    let _ = write!(text, "\nRepair: {}", d.help);
    for related in &d.related {
        let _ = write!(
            text,
            "\nRelated: {}:{}:{}",
            related.path, related.line, related.column
        );
    }
    text
}

pub fn junit(report: &Report) -> String {
    let failures = report
        .diagnostics
        .iter()
        .filter(|d| failure(d, report))
        .count();
    let errors = report.diagnostics.iter().filter(|d| incomplete(d)).count();
    let tests = report.diagnostics.len() + 1;
    let mut text = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<testsuites><testsuite name=\"Perfect Doc\" tests=\"{tests}\" failures=\"{failures}\" errors=\"{errors}\"><properties><property name=\"documents\" value=\"{}\"/><property name=\"parsed\" value=\"{}\"/><property name=\"network_enabled\" value=\"{}\"/></properties><testcase name=\"scan coverage\" classname=\"perfect-doc\"/>\n",
        report.coverage.documents, report.coverage.parsed, report.network_enabled
    );
    for diagnostic in &report.diagnostics {
        let _ = write!(
            text,
            "<testcase name=\"{}:{}:{} {}\" classname=\"{}\">",
            xml(&diagnostic.location.path),
            diagnostic.location.line,
            diagnostic.location.column,
            xml(&diagnostic.rule),
            xml(&diagnostic.rule)
        );
        if failure(diagnostic, report) {
            let _ = write!(
                text,
                "<failure message=\"{}\" type=\"{}\">{}</failure>",
                xml(&diagnostic.message),
                xml(&diagnostic.rule),
                xml(&detail(diagnostic))
            );
        } else if incomplete(diagnostic) {
            let _ = write!(
                text,
                "<error message=\"{}\" type=\"incomplete\">{}</error>",
                xml(&diagnostic.message),
                xml(&detail(diagnostic))
            );
        } else if let Some(reason) = &diagnostic.suppression {
            let _ = write!(text, "<skipped message=\"{}\"/>", xml(reason));
        } else {
            let _ = write!(
                text,
                "<system-out>{}</system-out>",
                xml(&detail(diagnostic))
            );
        }
        text.push_str("</testcase>\n");
    }
    text.push_str("</testsuite></testsuites>\n");
    text
}

fn uri_path(path: &str) -> String {
    let mut url = url::Url::parse("https://perfect-doc.invalid/").expect("constant URL");
    url.set_path(&path.replace('%', "%25"));
    url.path().trim_start_matches('/').into()
}

pub fn sarif(report: &Report) -> Result<String, serde_json::Error> {
    let rules = crate::rules();
    let results:Vec<_>=report.diagnostics.iter().map(|d|{
        let level=if incomplete(d){"error"}else{match d.severity{Severity::Error=>"error",Severity::Warning=>"warning",Severity::Info=>"note"}};
        let mut value=json!({"ruleId":d.rule,"level":level,"message":{"text":detail(d)},"locations":[{"physicalLocation":{"artifactLocation":{"uri":uri_path(&d.location.path)},"region":{"startLine":d.location.line,"startColumn":d.location.column}}}],"properties":{"outcome":d.outcome,"requirement":d.requirement,"required":d.required,"byte_offset":d.location.byte_offset,"target":d.target,"help":d.help}});
        if let Some(reason)=&d.suppression{value["suppressions"]=json!([{"kind":"external","status":"accepted","justification":reason}]);}
        if !d.related.is_empty(){value["relatedLocations"]=json!(d.related.iter().enumerate().map(|(i,l)|json!({"id":i+1,"physicalLocation":{"artifactLocation":{"uri":uri_path(&l.path)},"region":{"startLine":l.line,"startColumn":l.column}}})).collect::<Vec<_>>());}
        value
    }).collect();
    serde_json::to_string_pretty(
        &json!({"$schema":"https://json.schemastore.org/sarif-2.1.0.json","version":"2.1.0","runs":[{"tool":{"driver":{"name":"Perfect Doc","version":env!("CARGO_PKG_VERSION"),"rules":rules.iter().map(|r|json!({"id":r.id,"shortDescription":{"text":r.description},"properties":{"family":r.family,"requirement":r.requirement}})).collect::<Vec<_>>()}},"columnKind":"unicodeCodePoints","results":results,"invocations":[{"executionSuccessful":!report.diagnostics.iter().any(incomplete)}],"properties":{"coverage":report.coverage,"exit_code":report.exit_code(),"network_enabled":report.network_enabled,"network_checked_at":report.network_checked_at}}]}),
    )
}
