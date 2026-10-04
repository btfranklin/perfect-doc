use std::process::Command;
use tempfile::TempDir;
fn run(args: &[&str]) -> std::process::Output {
    let cwd = TempDir::new().unwrap();
    Command::new(env!("CARGO_BIN_EXE_perfect-doc"))
        .args(args)
        .current_dir(cwd.path())
        .output()
        .unwrap()
}
#[test]
fn help_and_version_are_available() {
    assert!(run(&["--help"]).status.success());
    let output = run(&["--version"]);
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        format!("perfect-doc {}", env!("CARGO_PKG_VERSION"))
    );
}
#[test]
fn invalid_invocation_and_missing_input_have_exit_two() {
    assert_eq!(run(&["unknown"]).status.code(), Some(2));
    assert_eq!(
        run(&["check", "/perfect-doc-missing-input"]).status.code(),
        Some(2)
    );
}
#[test]
fn machine_output_is_one_clean_report() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("a.md"), "[bad](missing.md)").unwrap();
    let output = run(&["check", dir.path().to_str().unwrap(), "--format", "json"]);
    assert_eq!(output.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["coverage"]["documents"], 1);
    assert!(output.stderr.is_empty());
}
#[test]
fn report_file_and_clean_scan_work() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("a.okf"), "# A").unwrap();
    let report = dir.path().join("result.xml");
    let output = run(&[
        "check",
        dir.path().to_str().unwrap(),
        "--format",
        "junit",
        "--output",
        report.to_str().unwrap(),
    ]);
    assert!(output.status.success(), "{:?}", output);
    assert!(output.stdout.is_empty());
    assert!(roxmltree::Document::parse(&std::fs::read_to_string(report).unwrap()).is_ok());
}
#[test]
fn init_is_round_trip_valid_and_protects_existing_files() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("contract.toml");
    assert!(run(&["init", path.to_str().unwrap()]).status.success());
    let config = perfect_doc::Config::from_file(&path).unwrap();
    assert!(!config.network.enabled);
    assert_eq!(
        run(&["init", path.to_str().unwrap()]).status.code(),
        Some(2)
    );
}
#[test]
fn effective_config_and_rule_catalog_are_inspectable() {
    let output = run(&["check", "--show-config"]);
    assert!(output.status.success());
    let _: perfect_doc::Config = toml::from_str(&String::from_utf8_lossy(&output.stdout)).unwrap();
    let output = run(&["rules", "--json"]);
    let rules: Vec<serde_json::Value> = serde_json::from_slice(&output.stdout).unwrap();
    assert!(rules.iter().any(|r| r["id"] == "link.exists"));
}
#[test]
fn conflicting_network_modes_fail() {
    assert_eq!(
        run(&["check", "--online", "--offline"]).status.code(),
        Some(2)
    );
}
#[test]
fn empty_scan_is_incomplete() {
    let dir = TempDir::new().unwrap();
    assert_eq!(
        run(&["check", dir.path().to_str().unwrap()]).status.code(),
        Some(3)
    );
}
#[cfg(unix)]
#[test]
fn non_utf8_unknown_command_is_an_invocation_error() {
    use std::os::unix::ffi::OsStrExt;
    let output = Command::new(env!("CARGO_BIN_EXE_perfect-doc"))
        .arg(std::ffi::OsStr::from_bytes(b"\xff"))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn automatic_relative_configuration_is_loaded_from_the_current_directory() {
    let dir = TempDir::new().unwrap();
    std::fs::write(
        dir.path().join("perfect-doc.toml"),
        "[markdown]\nno_heading_skips = true\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("page.md"), "# Page\n### Skipped\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_perfect-doc"))
        .args(["check", ".", "--format", "json"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{:?}", output);
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        report["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["rule"] == "markdown.heading-shape")
    );
}

#[test]
fn human_output_has_a_banner_that_can_be_disabled() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("page.md"), "# Page\n").unwrap();
    let normal = run(&["check", dir.path().to_str().unwrap()]);
    let plain = run(&["check", dir.path().to_str().unwrap(), "--no-banner"]);
    assert!(normal.status.success());
    assert!(plain.status.success());
    let banner = include_str!("../src/banner.txt");
    assert!(banner.is_ascii());
    assert_eq!(
        String::from_utf8(normal.stdout).unwrap(),
        format!(
            "{banner}\n{}",
            String::from_utf8(plain.stdout.clone()).unwrap()
        )
    );
    let path = dir.path().join("report.txt");
    let saved = run(&[
        "check",
        dir.path().to_str().unwrap(),
        "--output",
        path.to_str().unwrap(),
    ]);
    assert!(saved.status.success());
    assert!(saved.stdout.is_empty());
    assert_eq!(std::fs::read(path).unwrap(), plain.stdout);
}

#[test]
fn help_has_a_banner_that_can_be_disabled() {
    for command in [vec!["--help"], vec!["check", "--help"]] {
        let normal = run(&command);
        let mut plain_command = vec!["--no-banner"];
        plain_command.extend(command);
        let plain = run(&plain_command);
        assert!(normal.status.success());
        assert!(plain.status.success());
        let banner = include_str!("../src/banner.txt");
        assert_eq!(
            String::from_utf8(normal.stdout).unwrap(),
            format!("{banner}\n{}", String::from_utf8(plain.stdout).unwrap())
        );
    }
}

#[test]
fn banners_do_not_enter_machine_reports_or_configuration() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("page.md"), "# Page\n").unwrap();
    for format in ["json", "junit", "sarif"] {
        let normal = run(&["check", dir.path().to_str().unwrap(), "--format", format]);
        let plain = run(&[
            "--no-banner",
            "check",
            dir.path().to_str().unwrap(),
            "--format",
            format,
        ]);
        assert!(normal.status.success());
        assert_eq!(normal.stdout, plain.stdout);
        match format {
            "junit" => {
                roxmltree::Document::parse(std::str::from_utf8(&normal.stdout).unwrap()).unwrap();
            }
            _ => {
                serde_json::from_slice::<serde_json::Value>(&normal.stdout).unwrap();
            }
        }
    }
    let normal = run(&["check", "--show-config"]);
    let plain = run(&["check", "--show-config", "--no-banner"]);
    assert!(normal.status.success());
    assert_eq!(normal.stdout, plain.stdout);
    toml::from_str::<perfect_doc::Config>(std::str::from_utf8(&normal.stdout).unwrap()).unwrap();
}
