use perfect_doc::{Config, Outcome, validate};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

struct Server {
    base: String,
    requests: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Server {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("local listener");
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let count = requests.clone();
        let stopped = stop.clone();
        let thread = std::thread::spawn(move || {
            while !stopped.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream.set_nonblocking(false).unwrap();
                        stream
                            .set_write_timeout(Some(Duration::from_secs(2)))
                            .unwrap();
                        stream
                            .set_read_timeout(Some(Duration::from_secs(2)))
                            .unwrap();
                        let mut bytes = Vec::new();
                        let mut chunk = [0; 1024];
                        while !bytes.windows(4).any(|value| value == b"\r\n\r\n") {
                            let Ok(size) = stream.read(&mut chunk) else {
                                break;
                            };
                            if size == 0 {
                                break;
                            }
                            bytes.extend_from_slice(&chunk[..size]);
                        }
                        let request = String::from_utf8_lossy(&bytes);
                        let mut words = request
                            .lines()
                            .next()
                            .unwrap_or_default()
                            .split_whitespace();
                        let method = words.next().unwrap_or_default();
                        let path = words.next().unwrap_or_default();
                        count.fetch_add(1, Ordering::Relaxed);
                        let status = if path.starts_with("/good") {
                            200
                        } else if path.starts_with("/blocked") {
                            403
                        } else {
                            404
                        };
                        let body = "<h1 id='ok'>Test</h1>";
                        let mut reply = format!(
                            "HTTP/1.1 {status} Fixture\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        );
                        if method != "HEAD" {
                            reply.push_str(body);
                        }
                        let _ = stream.write_all(reply.as_bytes());
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(2))
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            base,
            requests,
            stop,
            thread: Some(thread),
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.thread.take().unwrap().join().unwrap();
    }
}

fn config(directory: &tempfile::TempDir) -> Config {
    let mut config = Config {
        base_dir: directory.path().to_path_buf(),
        ..Config::default()
    };
    config.network.enabled = true;
    config.network.host_delay_ms = 0;
    config.network.retries = 0;
    config.network.check_fragments = true;
    config
}

#[test]
fn online_evidence_reaches_report_locations_coverage_and_exit_status() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("README.md"), format!("# Network\n\n[Good]({0}/good#ok)\n[Missing]({0}/missing)\n[Blocked]({0}/blocked?token=report-secret)\n[Again]({0}/good#ok)\n", server.base)).unwrap();
    let roots = [directory.path().to_path_buf()];
    let report = validate(&roots, &config(&directory)).unwrap();
    assert_eq!(report.exit_code(), 1);
    assert_eq!(report.coverage.external_urls, 3);
    assert_eq!(report.coverage.external_verified, 1);
    assert!(report.network_enabled);
    assert!(report.network_checked_at.is_some());
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.rule == "external.response"
                && diagnostic.outcome == Outcome::Invalid
                && diagnostic.location.line == 4)
    );
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.rule == "external.response"
                && diagnostic.outcome == Outcome::Unverified
                && diagnostic.required
                && diagnostic.location.line == 5)
    );
    assert!(
        !serde_json::to_string(&report)
            .unwrap()
            .contains("report-secret")
    );
    assert_eq!(server.requests.load(Ordering::Relaxed), 4);
}

#[test]
fn optional_unverified_checks_and_offline_checks_have_distinct_evidence() {
    let server = Server::new();
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("README.md"),
        format!("# Network\n\n[Blocked]({}/blocked)\n", server.base),
    )
    .unwrap();
    let roots = [directory.path().to_path_buf()];
    let mut config = config(&directory);
    let required = validate(&roots, &config).unwrap();
    assert_eq!(required.exit_code(), 3);
    config.network.required = false;
    let optional = validate(&roots, &config).unwrap();
    assert_eq!(optional.exit_code(), 0);
    assert!(
        optional
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.outcome == Outcome::Unverified && !diagnostic.required)
    );
    assert_eq!(optional.coverage.external_verified, 0);
    config.network.enabled = false;
    let requests = server.requests.load(Ordering::Relaxed);
    let offline = validate(&roots, &config).unwrap();
    assert_eq!(offline.exit_code(), 0);
    assert!(!offline.network_enabled);
    assert!(offline.network_checked_at.is_none());
    assert_eq!(offline.coverage.external_verified, 0);
    assert_eq!(server.requests.load(Ordering::Relaxed), requests);
}
