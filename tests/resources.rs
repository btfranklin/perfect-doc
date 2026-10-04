use base64::{Engine, engine::general_purpose::STANDARD};
use perfect_doc::{Config, Outcome, Report, validate};
use std::{fs, io::Cursor};

fn fixture(files: &[(&str, &[u8])], edit: impl FnOnce(&mut Config)) -> Report {
    let directory = tempfile::tempdir().unwrap();
    for (path, bytes) in files {
        let path = directory.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }
    let mut config = Config {
        base_dir: fs::canonicalize(directory.path()).unwrap(),
        include: vec!["page.html".into()],
        ..Config::default()
    };
    edit(&mut config);
    validate(&[directory.path().into()], &config).unwrap()
}

fn png(width: u32, height: u32) -> Vec<u8> {
    let mut output = Cursor::new(Vec::new());
    image::DynamicImage::new_rgba8(width, height)
        .write_to(&mut output, image::ImageFormat::Png)
        .unwrap();
    output.into_inner()
}

fn image_uri(mime: &str, bytes: &[u8]) -> String {
    format!("data:{mime};base64,{}", STANDARD.encode(bytes))
}

fn incomplete(report: Report, outcome: Outcome) {
    assert_eq!(report.exit_code(), 3, "{:?}", report.diagnostics);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.outcome == outcome
                && diagnostic.required
                && diagnostic.suppression.is_none()),
        "{:?}",
        report.diagnostics
    );
    assert!(
        !report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.outcome == Outcome::Invalid),
        "{:?}",
        report.diagnostics
    );
}

#[test]
fn image_pixel_limit_is_unverified_for_file_and_data_uri() {
    let bytes = png(2, 2);
    for data in [false, true] {
        let target = if data {
            image_uri("image/png", &bytes)
        } else {
            "picture.png".into()
        };
        let html = format!("<img alt='' src='{target}'>");
        incomplete(
            fixture(
                &[("page.html", html.as_bytes()), ("picture.png", &bytes)],
                |config| config.assets.max_pixels = 3,
            ),
            Outcome::Unverified,
        );
    }
}

#[test]
fn unsupported_image_decoder_cannot_pass_for_file_or_data_uri() {
    // This is a local AVIF brand box. This build has no AVIF decoder.
    let bytes = b"\x00\x00\x00\x18ftypavif\x00\x00\x00\x00avifmif1";
    for data in [false, true] {
        let target = if data {
            image_uri("image/avif", bytes)
        } else {
            "picture.avif".into()
        };
        let html = format!("<img alt='' src='{target}'>");
        incomplete(
            fixture(
                &[("page.html", html.as_bytes()), ("picture.avif", bytes)],
                |_| {},
            ),
            Outcome::Unsupported,
        );
    }
}

#[test]
fn malformed_data_payload_is_invalid_and_payload_is_redacted() {
    let secret = "PRIVATE_PAYLOAD";
    let html = format!("<img alt='' src='data:image/png;base64,{secret}'>");
    let report = fixture(&[("page.html", html.as_bytes())], |_| {});
    assert_eq!(report.exit_code(), 1, "{:?}", report.diagnostics);
    assert!(report.diagnostics.iter().any(
        |diagnostic| diagnostic.rule == "uri.syntax" && diagnostic.outcome == Outcome::Invalid
    ));
    let output = serde_json::to_string(&report).unwrap();
    assert!(!output.contains(secret), "{output}");
    assert!(output.contains("[redacted]"));
}

#[test]
fn malformed_image_bytes_stay_invalid() {
    let html = format!(
        "<img alt='' src='{}'>",
        image_uri("image/png", b"not an image")
    );
    let report = fixture(&[("page.html", html.as_bytes())], |_| {});
    assert_eq!(report.exit_code(), 1, "{:?}", report.diagnostics);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.outcome == Outcome::Invalid)
    );
}

#[test]
fn css_parser_nesting_limit_is_unverified() {
    let css = format!("{}{}", "a{".repeat(140), "}".repeat(140));
    incomplete(
        fixture(
            &[
                ("page.html", b"<link rel='stylesheet' href='style.css'>"),
                ("style.css", css.as_bytes()),
            ],
            |_| {},
        ),
        Outcome::Unverified,
    );
}

#[test]
fn malformed_css_url_stays_invalid() {
    let report = fixture(
        &[
            ("page.html", b"<link rel='stylesheet' href='style.css'>"),
            ("style.css", b"a { background: url(\"bad\nurl\"); }"),
        ],
        |_| {},
    );
    assert_eq!(report.exit_code(), 1, "{:?}", report.diagnostics);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.rule == "asset.syntax"
                && diagnostic.outcome == Outcome::Invalid)
    );
}

#[test]
fn css_image_file_and_data_uri_obey_image_limits() {
    let bytes = png(2, 2);
    for data in [false, true] {
        let target = if data {
            image_uri("image/png", &bytes)
        } else {
            "picture.png".into()
        };
        let css = format!("a {{ background: url('{target}'); }}");
        incomplete(
            fixture(
                &[
                    ("page.html", b"<link rel='stylesheet' href='style.css'>"),
                    ("style.css", css.as_bytes()),
                    ("picture.png", &bytes),
                ],
                |config| config.assets.max_pixels = 3,
            ),
            Outcome::Unverified,
        );
    }
}

#[test]
fn json_data_uri_recursion_limit_is_unverified() {
    let data = format!("{}0{}", "[".repeat(200), "]".repeat(200));
    let html = format!("<a href='data:application/json,{data}'>Data</a>");
    incomplete(
        fixture(&[("page.html", html.as_bytes())], |_| {}),
        Outcome::Unverified,
    );
}

#[test]
fn asset_reads_obey_remaining_total_byte_budget() {
    incomplete(
        fixture(
            &[
                ("page.html", b"<link rel='stylesheet' href='one.css'>"),
                (
                    "one.css",
                    b"@import 'two.css'; /* A local source fixture */",
                ),
                (
                    "two.css",
                    b"/* This second stylesheet has more bytes than the remaining read budget. */",
                ),
            ],
            |config| config.files.max_total_bytes = 100,
        ),
        Outcome::Unverified,
    );
}

#[test]
fn oversized_asset_cannot_pass() {
    let bytes = png(2, 2);
    incomplete(
        fixture(
            &[
                ("page.html", b"<img alt='' src='picture.png'>"),
                ("picture.png", &bytes),
            ],
            |config| config.files.max_file_bytes = 40,
        ),
        Outcome::Unverified,
    );
}

#[test]
fn disabled_svg_structure_policy_also_applies_to_data_uri() {
    let html = format!(
        "<img alt='' src='{}'>",
        image_uri("image/svg+xml", b"not xml")
    );
    let report = fixture(&[("page.html", html.as_bytes())], |config| {
        config.assets.validate_svg = false
    });
    assert_eq!(report.exit_code(), 0, "{:?}", report.diagnostics);
}
