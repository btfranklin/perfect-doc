//! Bounded HTTP checks. This module does not execute returned content.

use crate::config::NetworkConfig;
use crate::model::{
    Diagnostic, ExternalLink, Location, NetworkResult, Outcome, Requirement, RuleDefinition,
};
use reqwest::blocking::{Client, Response};
use reqwest::{Method, StatusCode, header};
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use time::OffsetDateTime;
use url::Url;

pub const RULES: &[RuleDefinition] = &[
    RuleDefinition {
        id: "external.response",
        family: 22,
        requirement: Requirement::Execution,
        description: "Check HTTP destinations with bounded requests.",
    },
    RuleDefinition {
        id: "external.fragment",
        family: 22,
        requirement: Requirement::Execution,
        description: "Check fragments in static HTTP documents.",
    },
    RuleDefinition {
        id: "external.policy",
        family: 22,
        requirement: Requirement::Project,
        description: "Apply the declared network host policy.",
    },
    RuleDefinition {
        id: "external.cache",
        family: 22,
        requirement: Requirement::Execution,
        description: "Read and write an explicit network evidence cache.",
    },
];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Evidence {
    checked_at: i64,
    page: Page,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
enum Page {
    Reachable {
        anchors: Option<BTreeSet<String>>,
        dynamic: bool,
        fragment_limit: Option<String>,
    },
    Missing {
        status: u16,
    },
    Unverified {
        reason: String,
    },
}

impl Page {
    fn unverified(reason: impl Into<String>) -> Self {
        Self::Unverified {
            reason: reason.into(),
        }
    }
    fn cacheable(&self) -> bool {
        matches!(
            self,
            Self::Missing { .. }
                | Self::Reachable {
                    fragment_limit: None,
                    ..
                }
        )
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cache {
    policy: String,
    entries: BTreeMap<String, Evidence>,
}

struct Work {
    url: Url,
    references: Vec<usize>,
    fragments: bool,
    cache_key: Option<String>,
}

struct Session<'a> {
    client: Client,
    config: &'a NetworkConfig,
    cancelled: &'a AtomicBool,
    requests: AtomicUsize,
    hosts: Mutex<BTreeMap<String, Instant>>,
}

/// Check HTTP links. Disabled network mode makes no requests.
pub fn check(
    links: &[ExternalLink],
    config: &NetworkConfig,
    cancelled: &AtomicBool,
) -> NetworkResult {
    if !config.enabled || links.is_empty() {
        return NetworkResult::default();
    }
    if config.concurrency == 0
        || config.concurrency > 64
        || config.max_requests == 0
        || config.max_body_bytes == 0
        || config.timeout_seconds == 0
        || config.retries > 5
        || config.max_redirects > 64
    {
        return NetworkResult {
            diagnostics: links
                .iter()
                .map(|link| {
                    diagnostic(
                        link,
                        "external.response",
                        Outcome::Unverified,
                        "The network limits are invalid.",
                        config,
                    )
                })
                .collect(),
            ..NetworkResult::default()
        };
    }
    let now = OffsetDateTime::now_utc().unix_timestamp();
    let mut result = NetworkResult::default();
    let mut verified_urls = BTreeSet::new();
    let mut groups = BTreeMap::<String, Work>::new();
    for (index, link) in links.iter().enumerate() {
        let Ok(mut url) = Url::parse(&link.url) else {
            result.diagnostics.push(diagnostic(
                link,
                "external.response",
                Outcome::Invalid,
                "The external URI is invalid.",
                config,
            ));
            continue;
        };
        if !url.username().is_empty() || url.password().is_some() {
            result.diagnostics.push(diagnostic(
                link,
                "external.policy",
                Outcome::Invalid,
                "URI user information is prohibited. Use network.auth_env.",
                config,
            ));
            continue;
        }
        if !matches!(url.scheme(), "http" | "https") {
            result.diagnostics.push(diagnostic(
                link,
                "external.response",
                Outcome::Unverified,
                "Online verification supports HTTP and HTTPS only.",
                config,
            ));
            continue;
        }
        if !permitted(&url, config) {
            result.diagnostics.push(diagnostic(
                link,
                "external.policy",
                Outcome::Excluded,
                "The host policy excludes this destination.",
                config,
            ));
            continue;
        }
        let fragments =
            config.check_fragments && url.fragment().is_some_and(|value| !value.is_empty());
        url.set_fragment(None);
        let key = url.as_str().to_owned();
        let cache_key = (url.query().is_none() && auth_variable(&url, config).is_none())
            .then(|| hash(key.as_bytes()));
        let work = groups.entry(key).or_insert_with(|| Work {
            url,
            references: Vec::new(),
            fragments: false,
            cache_key,
        });
        work.references.push(index);
        work.fragments |= fragments;
    }
    let mut policy_config = config.clone();
    policy_config.cache = None;
    let policy = hash(&serde_json::to_vec(&policy_config).unwrap_or_default());
    let (mut cache, cache_error) = load_cache(config.cache.as_deref(), &policy);
    if let Some(reason) = cache_error {
        result
            .diagnostics
            .push(cache_diagnostic(&links[0].location, reason, config));
    }
    cache
        .entries
        .retain(|_, evidence| fresh(evidence, now, config.cache_ttl_seconds));
    let client = match Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(config.timeout_seconds))
        .user_agent(&config.user_agent)
        .build()
    {
        Ok(client) => client,
        Err(_) => {
            for work in groups.values() {
                apply(
                    work,
                    links,
                    &Page::unverified("The HTTP client configuration is unavailable."),
                    config,
                    &mut result,
                    &mut verified_urls,
                );
            }
            return result;
        }
    };
    let session = Session {
        client,
        config,
        cancelled,
        requests: AtomicUsize::new(0),
        hosts: Mutex::new(BTreeMap::new()),
    };
    let mut pending = Vec::new();
    let mut evidence_time: Option<i64> = None;
    for (_, work) in groups {
        if !cancelled.load(Ordering::Relaxed)
            && let Some(evidence) = work
                .cache_key
                .as_ref()
                .and_then(|key| cache.entries.get(key))
            && (!work.fragments
                || matches!(
                    &evidence.page,
                    Page::Reachable {
                        anchors: Some(_),
                        ..
                    } | Page::Missing { .. }
                ))
        {
            evidence_time = Some(
                evidence_time.map_or(evidence.checked_at, |prior| prior.min(evidence.checked_at)),
            );
            apply(
                &work,
                links,
                &evidence.page,
                config,
                &mut result,
                &mut verified_urls,
            );
        } else {
            pending.push(work);
        }
    }
    let next = AtomicUsize::new(0);
    let completed = Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..config.concurrency.min(pending.len()) {
            scope.spawn(|| {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(work) = pending.get(index) else {
                        break;
                    };
                    let page = session.fetch(work);
                    completed.lock().expect("result lock").push((
                        index,
                        Evidence {
                            checked_at: OffsetDateTime::now_utc().unix_timestamp(),
                            page,
                        },
                    ));
                }
            });
        }
    });
    let mut completed = completed.into_inner().expect("result lock");
    completed.sort_by_key(|(index, _)| *index);
    for (index, evidence) in completed {
        let work = &pending[index];
        evidence_time =
            Some(evidence_time.map_or(evidence.checked_at, |prior| prior.min(evidence.checked_at)));
        apply(
            work,
            links,
            &evidence.page,
            config,
            &mut result,
            &mut verified_urls,
        );
        if evidence.page.cacheable()
            && let Some(key) = &work.cache_key
        {
            cache.entries.insert(key.clone(), evidence);
        }
    }
    result.checked_at = evidence_time.map(timestamp);
    result.verified = verified_urls.len();
    if let Some(path) = &config.cache
        && let Err(reason) = save_cache(Path::new(path), &cache)
    {
        result
            .diagnostics
            .push(cache_diagnostic(&links[0].location, reason, config));
    }
    result
}

impl Session<'_> {
    fn fetch(&self, work: &Work) -> Page {
        let auth = match auth_variable(&work.url, self.config) {
            Some(variable) => match std::env::var(variable) {
                Ok(value) if !value.trim().is_empty() => Some(value),
                _ => {
                    return Page::unverified(
                        "The configured authentication environment variable is unavailable.",
                    );
                }
            },
            None => None,
        };
        for attempt in 0..=self.config.retries {
            let response = self.request(&work.url, &work.url, Method::HEAD, auth.as_deref());
            match response {
                Ok((response, final_url)) => {
                    let status = response.status();
                    let fallback = matches!(
                        status,
                        StatusCode::METHOD_NOT_ALLOWED | StatusCode::NOT_IMPLEMENTED
                    ) || (status.is_success()
                        && (work.fragments
                            || status == StatusCode::NO_CONTENT
                            || status == StatusCode::RESET_CONTENT));
                    let response = if fallback {
                        drop(response);
                        self.request(&final_url, &work.url, Method::GET, auth.as_deref())
                    } else {
                        Ok((response, final_url))
                    };
                    match response {
                        Ok((response, _)) => {
                            let status = response.status();
                            if retry_status(status) && attempt < self.config.retries {
                                continue;
                            }
                            return self.evaluate(response, work.fragments);
                        }
                        Err(error) if error.retry && attempt < self.config.retries => continue,
                        Err(error) => return Page::unverified(error.reason),
                    }
                }
                Err(error) if error.retry && attempt < self.config.retries => continue,
                Err(error) => return Page::unverified(error.reason),
            }
        }
        Page::unverified("The retry limit stopped verification.")
    }

    fn request(
        &self,
        start: &Url,
        origin: &Url,
        mut method: Method,
        auth: Option<&str>,
    ) -> Result<(Response, Url), RequestError> {
        let mut current = start.clone();
        let mut seen = BTreeSet::new();
        let mut redirects = 0;
        loop {
            if self.cancelled.load(Ordering::Relaxed) {
                return Err(RequestError::stop("The network check was cancelled."));
            }
            if !seen.insert(current.as_str().to_owned()) {
                return Err(RequestError::stop("A redirect loop stopped verification."));
            }
            if !permitted(&current, self.config) {
                return Err(RequestError::stop(
                    "The host policy excludes a redirect destination.",
                ));
            }
            if !current.username().is_empty() || current.password().is_some() {
                return Err(RequestError::stop(
                    "A redirect contains prohibited URI user information.",
                ));
            }
            if !matches!(current.scheme(), "http" | "https") {
                return Err(RequestError::stop("A redirect uses an unsupported scheme."));
            }
            if self.requests.load(Ordering::Relaxed) >= self.config.max_requests {
                return Err(RequestError::stop(
                    "The request limit stopped verification.",
                ));
            }
            self.wait_for_host(&current)?;
            if self
                .requests
                .try_update(Ordering::Relaxed, Ordering::Relaxed, |count| {
                    (count < self.config.max_requests).then(|| count + 1)
                })
                .is_err()
            {
                return Err(RequestError::stop(
                    "The request limit stopped verification.",
                ));
            }
            let mut request = self.client.request(method.clone(), current.clone());
            if self.cancelled.load(Ordering::Relaxed) {
                return Err(RequestError::stop("The network check was cancelled."));
            }
            if current.origin() == origin.origin()
                && let Some(value) = auth
            {
                request = request.bearer_auth(value);
            }
            let response = request.send().map_err(|_| RequestError {
                reason: "The HTTP connection, TLS exchange, or timeout prevented verification.",
                retry: true,
            })?;
            let status = response.status();
            if matches!(status.as_u16(), 300 | 301 | 302 | 303 | 307 | 308) {
                if redirects >= self.config.max_redirects {
                    return Err(RequestError::stop(
                        "The redirect limit stopped verification.",
                    ));
                }
                let location = response
                    .headers()
                    .get(header::LOCATION)
                    .and_then(|value| value.to_str().ok())
                    .ok_or_else(|| RequestError::stop("The redirect location is unavailable."))?;
                current = current
                    .join(location)
                    .map_err(|_| RequestError::stop("The redirect URI is invalid."))?;
                if current
                    .fragment()
                    .is_some_and(|fragment| !fragment.is_empty())
                {
                    return Err(RequestError::stop(
                        "A redirect changes the fragment. This fragment remains unverified.",
                    ));
                }
                current.set_fragment(None);
                if status == StatusCode::SEE_OTHER && method != Method::HEAD {
                    method = Method::GET;
                }
                redirects += 1;
                continue;
            }
            return Ok((response, current));
        }
    }

    fn wait_for_host(&self, url: &Url) -> Result<(), RequestError> {
        let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
        let slot = {
            let mut hosts = self.hosts.lock().expect("host lock");
            let now = Instant::now();
            let slot = hosts.get(&host).copied().unwrap_or(now).max(now);
            let next = slot
                .checked_add(Duration::from_millis(self.config.host_delay_ms))
                .ok_or_else(|| RequestError::stop("The host delay exceeds the clock limit."))?;
            hosts.insert(host, next);
            slot
        };
        loop {
            if self.cancelled.load(Ordering::Relaxed) {
                return Err(RequestError::stop("The network check was cancelled."));
            }
            let remaining = slot.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Ok(());
            }
            std::thread::sleep(remaining.min(Duration::from_millis(25)));
        }
    }

    fn evaluate(&self, mut response: Response, fragments: bool) -> Page {
        if self.cancelled.load(Ordering::Relaxed) {
            return Page::unverified("The network check was cancelled.");
        }
        let status = response.status();
        if matches!(status, StatusCode::NOT_FOUND | StatusCode::GONE) {
            return Page::Missing {
                status: status.as_u16(),
            };
        }
        if !status.is_success() {
            return Page::unverified(format!(
                "HTTP {} did not establish destination validity.",
                status.as_u16()
            ));
        }
        if !fragments {
            return Page::Reachable {
                anchors: None,
                dynamic: false,
                fragment_limit: None,
            };
        }
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        if !matches!(
            content_type
                .split(';')
                .next()
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase()
                .as_str(),
            "text/html" | "application/xhtml+xml"
        ) {
            return Page::Reachable {
                anchors: None,
                dynamic: false,
                fragment_limit: Some(
                    "The returned resource type does not support static HTML fragment checks."
                        .into(),
                ),
            };
        }
        let mut body = Vec::new();
        let mut chunk = [0; 8192];
        loop {
            if self.cancelled.load(Ordering::Relaxed) {
                return Page::unverified("The network check was cancelled.");
            }
            let count = match response.read(&mut chunk) {
                Ok(count) => count,
                Err(_) => {
                    return Page::unverified(
                        "The HTTP body read or timeout prevented verification.",
                    );
                }
            };
            if count == 0 {
                break;
            }
            if (body.len() as u64).saturating_add(count as u64) > self.config.max_body_bytes {
                return Page::unverified("The response body limit stopped fragment verification.");
            }
            body.extend_from_slice(&chunk[..count]);
        }
        let encoding = match html_encoding(&content_type, &body) {
            Ok(encoding) => encoding,
            Err(reason) => return Page::unverified(reason),
        };
        let (text, _, errors) = encoding.decode(&body);
        if errors {
            return Page::unverified(
                "The returned HTML encoding prevents a complete fragment check.",
            );
        }
        let html = Html::parse_document(&text);
        let selector = Selector::parse("*").expect("constant selector");
        let mut anchors = BTreeSet::new();
        let mut dynamic = false;
        for element in html.select(&selector) {
            if let Some(id) = element.value().attr("id") {
                anchors.insert(id.to_owned());
            }
            if element.value().name() == "a"
                && let Some(name) = element.value().attr("name")
            {
                anchors.insert(name.to_owned());
            }
            dynamic |= matches!(element.value().name(), "script" | "iframe")
                || element
                    .value()
                    .attrs()
                    .any(|(name, _)| name.starts_with("on"));
        }
        Page::Reachable {
            anchors: Some(anchors),
            dynamic,
            fragment_limit: None,
        }
    }
}

struct RequestError {
    reason: &'static str,
    retry: bool,
}
impl RequestError {
    fn stop(reason: &'static str) -> Self {
        Self {
            reason,
            retry: false,
        }
    }
}
fn retry_status(status: StatusCode) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
}
fn permitted(url: &Url, config: &NetworkConfig) -> bool {
    let host = url.host_str().unwrap_or_default();
    !config
        .excluded_hosts
        .iter()
        .any(|value| host.eq_ignore_ascii_case(value))
        && (config.allowed_hosts.is_empty()
            || config
                .allowed_hosts
                .iter()
                .any(|value| host.eq_ignore_ascii_case(value)))
}
fn auth_variable<'a>(url: &Url, config: &'a NetworkConfig) -> Option<&'a str> {
    let host = url.host_str()?;
    config
        .auth_env
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(host))
        .map(|(_, value)| value.as_str())
}

fn charset(value: &str) -> Option<&str> {
    value.split(';').skip(1).find_map(|part| {
        let (name, value) = part.trim().split_once('=')?;
        name.trim()
            .eq_ignore_ascii_case("charset")
            .then(|| value.trim().trim_matches(['\'', '"']))
    })
}

fn html_encoding(
    content_type: &str,
    body: &[u8],
) -> Result<&'static encoding_rs::Encoding, &'static str> {
    let encoding = |label: &str| {
        encoding_rs::Encoding::for_label(label.as_bytes())
            .ok_or("The returned HTML declares an unsupported encoding.")
    };
    if let Some((encoding, _)) = encoding_rs::Encoding::for_bom(body) {
        return Ok(encoding);
    }
    if let Some(label) = charset(content_type) {
        return encoding(label);
    }
    let prefix = String::from_utf8_lossy(&body[..body.len().min(1024)]);
    let html = Html::parse_document(&prefix);
    let selector = Selector::parse("meta").expect("constant selector");
    for meta in html.select(&selector) {
        if let Some(label) = meta.value().attr("charset") {
            return encoding(label);
        }
        if meta
            .value()
            .attr("http-equiv")
            .is_some_and(|value| value.eq_ignore_ascii_case("content-type"))
            && let Some(label) = meta.value().attr("content").and_then(charset)
        {
            return encoding(label);
        }
    }
    Ok(encoding_rs::UTF_8)
}

fn apply(
    work: &Work,
    links: &[ExternalLink],
    page: &Page,
    config: &NetworkConfig,
    result: &mut NetworkResult,
    verified_urls: &mut BTreeSet<String>,
) {
    for &index in &work.references {
        let link = &links[index];
        let (rule, outcome, reason) = match page {
            Page::Missing { status } => (
                "external.response",
                Outcome::Invalid,
                format!("HTTP {status} reports a missing destination."),
            ),
            Page::Unverified { reason } => {
                ("external.response", Outcome::Unverified, reason.clone())
            }
            Page::Reachable {
                anchors,
                dynamic,
                fragment_limit,
            } => {
                let url = Url::parse(&link.url).expect("validated URL");
                let Some(fragment) = url
                    .fragment()
                    .filter(|fragment| config.check_fragments && !fragment.is_empty())
                else {
                    verified_urls.insert(link.url.clone());
                    continue;
                };
                let fragment = match percent_encoding::percent_decode_str(fragment).decode_utf8() {
                    Ok(fragment) => fragment,
                    Err(_) => {
                        result.diagnostics.push(diagnostic(
                            link,
                            "external.fragment",
                            Outcome::Unverified,
                            "The fragment encoding prevents verification.",
                            config,
                        ));
                        continue;
                    }
                };
                if let Some(reason) = fragment_limit {
                    ("external.fragment", Outcome::Unverified, reason.clone())
                } else if fragment.contains(":~:") {
                    (
                        "external.fragment",
                        Outcome::Unverified,
                        "Text fragment directives require a browser and remain unverified.".into(),
                    )
                } else if anchors
                    .as_ref()
                    .is_some_and(|anchors| anchors.contains(fragment.as_ref()))
                {
                    verified_urls.insert(link.url.clone());
                    continue;
                } else if *dynamic {
                    ("external.fragment", Outcome::Unverified, "The fragment is absent from static HTML. Script or frame content can change the page.".into())
                } else if anchors.is_none() {
                    (
                        "external.fragment",
                        Outcome::Unverified,
                        "Static fragment evidence is unavailable.".into(),
                    )
                } else {
                    (
                        "external.fragment",
                        Outcome::Invalid,
                        "The fragment is absent from the static HTML document.".into(),
                    )
                }
            }
        };
        result
            .diagnostics
            .push(diagnostic(link, rule, outcome, reason, config));
    }
}

fn diagnostic(
    link: &ExternalLink,
    rule: &str,
    outcome: Outcome,
    reason: impl AsRef<str>,
    config: &NetworkConfig,
) -> Diagnostic {
    let requirement = if rule == "external.policy" {
        Requirement::Project
    } else {
        Requirement::Execution
    };
    let mut diagnostic = Diagnostic::at(
        rule,
        requirement,
        outcome,
        link.location.clone(),
        format!(
            "{} Destination: {}",
            reason.as_ref(),
            redacted_url(&link.url)
        ),
    );
    diagnostic.required = config.required && outcome != Outcome::Excluded;
    diagnostic.target = Some(redacted_url(&link.url));
    diagnostic
}

fn redacted_url(value: &str) -> String {
    let Ok(mut url) = Url::parse(value) else {
        return "[invalid URI]".into();
    };
    let _ = url.set_username("");
    let _ = url.set_password(None);
    if url.query().is_some() {
        url.set_query(Some("[redacted]"));
    }
    // Fragments can contain access tokens as well as anchor names.
    if url.fragment().is_some() {
        url.set_fragment(Some("[redacted]"));
    }
    url.to_string()
}

fn cache_diagnostic(location: &Location, reason: String, config: &NetworkConfig) -> Diagnostic {
    let mut diagnostic = Diagnostic::at(
        "external.cache",
        Requirement::Execution,
        Outcome::Unverified,
        location.clone(),
        reason,
    );
    diagnostic.required = config.required;
    diagnostic
}
fn hash(value: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut output = String::with_capacity(64);
    for byte in Sha256::digest(value) {
        write!(&mut output, "{byte:02x}").expect("string write");
    }
    output
}
fn timestamp(value: i64) -> String {
    OffsetDateTime::from_unix_timestamp(value)
        .unwrap_or(OffsetDateTime::UNIX_EPOCH)
        .format(&time::format_description::well_known::Rfc3339)
        .expect("UTC timestamp")
}
fn fresh(evidence: &Evidence, now: i64, ttl: u64) -> bool {
    now.checked_sub(evidence.checked_at)
        .is_some_and(|age| age >= 0 && (age as u64) < ttl)
}
fn load_cache(path: Option<&str>, policy: &str) -> (Cache, Option<String>) {
    let empty = || Cache {
        policy: policy.into(),
        ..Cache::default()
    };
    let Some(path) = path else {
        return (empty(), None);
    };
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return (empty(), None),
        Err(_) => {
            return (
                empty(),
                Some("The network evidence cache cannot be read.".into()),
            );
        }
    };
    let mut bytes = Vec::new();
    if file
        .take(32 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .is_err()
        || bytes.len() > 32 * 1024 * 1024
    {
        return (
            empty(),
            Some("The network evidence cache read exceeds its limit or failed.".into()),
        );
    }
    match serde_json::from_slice::<Cache>(&bytes) {
        Ok(cache) if cache.policy == policy => (cache, None),
        Ok(_) => (empty(), None),
        Err(_) => (
            empty(),
            Some("The network evidence cache is invalid. Fresh requests are required.".into()),
        ),
    }
}
fn save_cache(path: &Path, cache: &Cache) -> Result<(), String> {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let Some(parent) = path.parent() else {
        return Err("The network cache path has no parent directory.".into());
    };
    let Some(name) = path.file_name() else {
        return Err("The network cache file name is unavailable.".into());
    };
    let temporary = parent.join(format!(
        ".{}.{}.{}.tmp",
        name.to_string_lossy(),
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let write = || -> Result<(), std::io::Error> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        serde_json::to_writer(&mut file, cache)?;
        file.flush()?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)
    };
    let result = write();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result.map_err(|_| "The network evidence cache cannot be written atomically.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{TcpListener, TcpStream};
    use std::sync::Arc;

    #[derive(Clone)]
    struct Request {
        method: String,
        path: String,
        authorization: Option<String>,
    }
    struct Reply {
        status: u16,
        headers: Vec<(String, String)>,
        body: String,
    }
    impl Reply {
        fn status(status: u16) -> Self {
            Self {
                status,
                headers: Vec::new(),
                body: String::new(),
            }
        }
        fn html(body: &str) -> Self {
            Self {
                status: 200,
                headers: vec![("Content-Type".into(), "text/html; charset=utf-8".into())],
                body: body.into(),
            }
        }
        fn redirect(url: &str) -> Self {
            Self {
                status: 302,
                headers: vec![("Location".into(), url.into())],
                body: String::new(),
            }
        }
    }
    struct Server {
        base: String,
        requests: Arc<Mutex<Vec<Request>>>,
        stop: Arc<AtomicBool>,
        thread: Option<std::thread::JoinHandle<()>>,
    }
    impl Server {
        fn new(handler: impl Fn(&Request) -> Reply + Send + Sync + 'static) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("local listener");
            listener.set_nonblocking(true).unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let stop = Arc::new(AtomicBool::new(false));
            let requests = Arc::new(Mutex::new(Vec::new()));
            let handler = Arc::new(handler);
            let thread = {
                let stop = stop.clone();
                let requests = requests.clone();
                std::thread::spawn(move || {
                    let mut workers = Vec::new();
                    while !stop.load(Ordering::Relaxed) {
                        match listener.accept() {
                            Ok((mut stream, _)) => {
                                stream.set_nonblocking(false).unwrap();
                                stream
                                    .set_write_timeout(Some(Duration::from_secs(2)))
                                    .unwrap();
                                let handler = handler.clone();
                                let requests = requests.clone();
                                workers.push(std::thread::spawn(move || {
                                    let Some(request) = read_request(&mut stream) else { return; };
                                    requests.lock().unwrap().push(request.clone());
                                    let reply = handler(&request);
                                    let mut text = format!("HTTP/1.1 {} Fixture\r\nContent-Length: {}\r\nConnection: close\r\n", reply.status, reply.body.len());
                                    for (name, value) in reply.headers { text.push_str(&format!("{name}: {value}\r\n")); }
                                    text.push_str("\r\n");
                                    if request.method != "HEAD" { text.push_str(&reply.body); }
                                    let _ = stream.write_all(text.as_bytes());
                                }));
                            }
                            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                                std::thread::sleep(Duration::from_millis(2))
                            }
                            Err(_) => break,
                        }
                    }
                    for worker in workers {
                        worker.join().unwrap();
                    }
                })
            };
            Self {
                base,
                requests,
                stop,
                thread: Some(thread),
            }
        }
        fn count(&self) -> usize {
            self.requests.lock().unwrap().len()
        }
        fn link(&self, path: &str, line: usize) -> ExternalLink {
            ExternalLink {
                url: format!("{}{path}", self.base),
                location: Location {
                    path: "fixture.md".into(),
                    line,
                    column: 1,
                    byte_offset: line,
                },
            }
        }
    }
    impl Drop for Server {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            self.thread.take().unwrap().join().unwrap();
        }
    }
    fn read_request(stream: &mut TcpStream) -> Option<Request> {
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut bytes = Vec::new();
        let mut chunk = [0; 1024];
        while !bytes.windows(4).any(|value| value == b"\r\n\r\n") {
            let count = stream.read(&mut chunk).ok()?;
            if count == 0 || bytes.len() > 64 * 1024 {
                return None;
            }
            bytes.extend_from_slice(&chunk[..count]);
        }
        let text = String::from_utf8(bytes).ok()?;
        let mut words = text.lines().next()?.split_whitespace();
        let method = words.next()?.into();
        let path = words.next()?.into();
        let authorization = text.lines().skip(1).find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("authorization")
                .then(|| value.trim().into())
        });
        Some(Request {
            method,
            path,
            authorization,
        })
    }
    fn config() -> NetworkConfig {
        NetworkConfig {
            enabled: true,
            host_delay_ms: 0,
            retries: 0,
            ..NetworkConfig::default()
        }
    }
    fn run(server: &Server, paths: &[&str], config: &NetworkConfig) -> NetworkResult {
        let links: Vec<_> = paths
            .iter()
            .enumerate()
            .map(|(index, path)| server.link(path, index + 1))
            .collect();
        check(&links, config, &AtomicBool::new(false))
    }

    #[test]
    fn offline_and_cancelled_checks_make_no_requests() {
        let server = Server::new(|_| Reply::status(200));
        let offline = run(&server, &["/"], &NetworkConfig::default());
        assert_eq!(offline.verified, 0);
        assert!(offline.checked_at.is_none());
        assert!(offline.diagnostics.is_empty());
        let cancelled = check(&[server.link("/", 1)], &config(), &AtomicBool::new(true));
        assert_eq!(cancelled.diagnostics[0].outcome, Outcome::Unverified);
        assert_eq!(server.count(), 0);
    }

    #[test]
    fn cancellation_during_a_request_stops_verification_and_pending_work() {
        let cancelled = Arc::new(AtomicBool::new(false));
        let stop = cancelled.clone();
        let server = Server::new(move |_| {
            stop.store(true, Ordering::Relaxed);
            Reply::status(200)
        });
        let result = check(
            &[server.link("/a", 1), server.link("/b", 2)],
            &NetworkConfig {
                concurrency: 1,
                ..config()
            },
            &cancelled,
        );
        assert_eq!(result.verified, 0);
        assert_eq!(result.diagnostics.len(), 2);
        assert!(
            result
                .diagnostics
                .iter()
                .all(|diagnostic| diagnostic.outcome == Outcome::Unverified)
        );
        assert_eq!(server.count(), 1);
    }

    #[test]
    fn statuses_preserve_missing_and_unverified_locations() {
        let server = Server::new(|request| Reply::status(request.path[1..].parse().unwrap()));
        let mut config = config();
        config.required = false;
        let result = run(
            &server,
            &["/200", "/404", "/410", "/401", "/403", "/429", "/503"],
            &config,
        );
        assert_eq!(result.verified, 1);
        assert_eq!(result.diagnostics.len(), 6);
        assert_eq!(
            result
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.outcome == Outcome::Invalid)
                .count(),
            2
        );
        assert!(
            result
                .diagnostics
                .iter()
                .all(|diagnostic| !diagnostic.required)
        );
        assert_eq!(
            result
                .diagnostics
                .iter()
                .map(|diagnostic| diagnostic.location.line)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([2, 3, 4, 5, 6, 7])
        );
    }

    #[test]
    fn unsupported_head_uses_get_and_budget_counts_both() {
        for status in [405, 501] {
            let server = Server::new(move |request| {
                Reply::status(if request.method == "HEAD" {
                    status
                } else {
                    200
                })
            });
            assert_eq!(run(&server, &["/"], &config()).verified, 1);
            assert_eq!(server.count(), 2);
            let result = run(
                &server,
                &["/"],
                &NetworkConfig {
                    max_requests: 1,
                    ..config()
                },
            );
            assert_eq!(result.verified, 0);
            assert!(result.diagnostics[0].message.contains("request limit"));
            assert_eq!(server.count(), 3);
        }
    }

    #[test]
    fn redirects_detect_loops_limits_and_host_policy() {
        let server = Server::new(|request| {
            if request.path == "/loop" {
                Reply::redirect("/loop")
            } else if request.path == "/start" {
                Reply::redirect("/ok")
            } else {
                Reply::status(200)
            }
        });
        let result = run(&server, &["/loop", "/start"], &config());
        assert_eq!(result.verified, 1);
        assert!(result.diagnostics[0].message.contains("redirect loop"));
        let limited = run(
            &server,
            &["/start"],
            &NetworkConfig {
                max_redirects: 0,
                ..config()
            },
        );
        assert_eq!(limited.diagnostics[0].outcome, Outcome::Unverified);
        let count = server.count();
        let excluded = run(
            &server,
            &["/start"],
            &NetworkConfig {
                excluded_hosts: vec!["127.0.0.1".into()],
                ..config()
            },
        );
        assert_eq!(excluded.diagnostics[0].outcome, Outcome::Excluded);
        assert!(!excluded.diagnostics[0].required);
        assert_eq!(server.count(), count);
    }

    #[test]
    fn redirect_destinations_cannot_escape_the_allowed_host_list() {
        let destination = Server::new(|_| Reply::status(200));
        let target = destination.base.replace("127.0.0.1", "localhost");
        let source = Server::new(move |_| Reply::redirect(&target));
        let result = run(
            &source,
            &["/"],
            &NetworkConfig {
                allowed_hosts: vec!["127.0.0.1".into()],
                ..config()
            },
        );
        assert_eq!(result.verified, 0);
        assert_eq!(result.diagnostics[0].outcome, Outcome::Unverified);
        assert_eq!(source.count(), 1);
        assert_eq!(destination.count(), 0);
    }

    #[test]
    fn auth_is_scoped_to_origin_and_secrets_are_redacted() {
        let destination = Server::new(|_| Reply::status(404));
        let target = format!("{}/missing?token=redirect-secret", destination.base);
        let source = Server::new(move |_| Reply::redirect(&target));
        let mut config = config();
        // Use an existing, non-secret environment value. Rust forbids mutation of
        // the process environment while test threads run.
        config.auth_env.insert("127.0.0.1".into(), "PATH".into());
        let result = run(
            &source,
            &["/start?token=source-secret#fragment-secret"],
            &config,
        );
        assert!(source.requests.lock().unwrap()[0].authorization.is_some());
        assert!(
            destination.requests.lock().unwrap()[0]
                .authorization
                .is_none()
        );
        let output = serde_json::to_string(&result.diagnostics).unwrap();
        for value in ["source-secret", "fragment-secret", "redirect-secret"] {
            assert!(!output.contains(value));
        }
        let bad = ExternalLink {
            url: format!(
                "http://name:user-secret@{}/",
                source.base.trim_start_matches("http://")
            ),
            location: source.link("/", 1).location,
        };
        let count = source.count();
        let result = check(&[bad], &config, &AtomicBool::new(false));
        assert_eq!(result.diagnostics[0].outcome, Outcome::Invalid);
        assert!(!result.diagnostics[0].message.contains("user-secret"));
        assert_eq!(source.count(), count);
    }

    #[test]
    fn fragments_deduplicate_pages_and_retain_each_source() {
        let server = Server::new(|_| Reply::html("<h1 id='a'>A</h1><a name='é'></a>"));
        let result = run(
            &server,
            &["/#a", "/#%C3%A9", "/#missing", "/#missing", "/", "/#a"],
            &NetworkConfig {
                check_fragments: true,
                ..config()
            },
        );
        assert_eq!(server.count(), 2);
        assert_eq!(result.verified, 3);
        assert_eq!(result.diagnostics.len(), 2);
        assert!(
            result
                .diagnostics
                .iter()
                .all(|diagnostic| diagnostic.rule == "external.fragment"
                    && diagnostic.outcome == Outcome::Invalid)
        );
        assert_eq!(result.diagnostics[0].location.line, 3);
        assert_eq!(result.diagnostics[1].location.line, 4);
    }

    #[test]
    fn dynamic_fragments_types_and_body_limits_remain_unverified() {
        let dynamic = Server::new(|_| Reply::html("<script>document.body.id = 'later';</script>"));
        let config = NetworkConfig {
            check_fragments: true,
            ..config()
        };
        assert_eq!(
            run(&dynamic, &["/#later"], &config).diagnostics[0].outcome,
            Outcome::Unverified
        );
        let body = Server::new(|_| Reply::html("<h1 id='present'>Long text</h1>"));
        let result = run(
            &body,
            &["/#present"],
            &NetworkConfig {
                max_body_bytes: 4,
                ..config.clone()
            },
        );
        assert_eq!(result.verified, 0);
        assert!(result.diagnostics[0].message.contains("body limit"));
        let unsupported = Server::new(|_| Reply::status(200));
        assert_eq!(
            run(&unsupported, &["/#anchor"], &config).diagnostics[0].outcome,
            Outcome::Unverified
        );
    }

    #[test]
    fn fragment_encoding_uses_declared_html_encoding_and_rejects_unknown_labels() {
        assert_eq!(
            html_encoding("text/html; charset=unknown", b"<h1 id='x'>x</h1>").unwrap_err(),
            "The returned HTML declares an unsupported encoding."
        );
        assert_eq!(
            html_encoding("text/html", b"<meta charset='windows-1252'>").unwrap(),
            encoding_rs::WINDOWS_1252
        );
        assert_eq!(
            html_encoding(
                "text/html",
                b"<meta http-equiv='Content-Type' content='text/html; charset=iso-8859-1'>"
            )
            .unwrap(),
            encoding_rs::WINDOWS_1252
        );
        let server = Server::new(|_| Reply::html("<h1 id='ok'>Text</h1>"));
        let result = run(
            &server,
            &["/#:~:text=Text"],
            &NetworkConfig {
                check_fragments: true,
                ..config()
            },
        );
        assert_eq!(result.diagnostics[0].outcome, Outcome::Unverified);
    }

    #[test]
    fn retry_budget_is_shared_with_redirects_and_fallbacks() {
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        let server = Server::new(move |_| {
            Reply::status(if count.fetch_add(1, Ordering::Relaxed) == 0 {
                503
            } else {
                200
            })
        });
        let result = run(
            &server,
            &["/"],
            &NetworkConfig {
                retries: 1,
                ..config()
            },
        );
        assert_eq!(result.verified, 1);
        assert_eq!(calls.load(Ordering::Relaxed), 2);
        let failing = Server::new(|_| Reply::status(503));
        let limited = run(
            &failing,
            &["/"],
            &NetworkConfig {
                retries: 3,
                max_requests: 2,
                ..config()
            },
        );
        assert_eq!(failing.count(), 2);
        assert_eq!(limited.diagnostics[0].outcome, Outcome::Unverified);
    }

    #[test]
    fn concurrency_and_request_limits_hold_under_contention() {
        let active = Arc::new(AtomicUsize::new(0));
        let maximum = Arc::new(AtomicUsize::new(0));
        let current = active.clone();
        let peak = maximum.clone();
        let server = Server::new(move |_| {
            let count = current.fetch_add(1, Ordering::SeqCst) + 1;
            peak.fetch_max(count, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(80));
            current.fetch_sub(1, Ordering::SeqCst);
            Reply::status(200)
        });
        let result = run(
            &server,
            &["/a", "/b", "/c", "/d", "/e", "/f", "/g", "/h"],
            &NetworkConfig {
                concurrency: 3,
                max_requests: 5,
                ..config()
            },
        );
        assert_eq!(server.count(), 5);
        assert_eq!(result.verified, 5);
        assert_eq!(result.diagnostics.len(), 3);
        assert!(maximum.load(Ordering::SeqCst) > 1);
        assert!(maximum.load(Ordering::SeqCst) <= 3);
    }

    #[test]
    fn cache_expires_and_policy_changes_require_new_evidence() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("evidence.json");
        let config = NetworkConfig {
            cache: Some(path.to_string_lossy().into()),
            ..config()
        };
        let server = Server::new(|_| Reply::status(200));
        assert_eq!(run(&server, &["/"], &config).verified, 1);
        assert_eq!(run(&server, &["/"], &config).verified, 1);
        assert_eq!(server.count(), 1);
        let mut cache: Cache = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        let prior = OffsetDateTime::now_utc().unix_timestamp() - 30;
        for evidence in cache.entries.values_mut() {
            evidence.checked_at = prior;
        }
        fs::write(&path, serde_json::to_vec(&cache).unwrap()).unwrap();
        assert_eq!(
            run(&server, &["/"], &config).checked_at,
            Some(timestamp(prior))
        );
        assert_eq!(server.count(), 1);
        for evidence in cache.entries.values_mut() {
            evidence.checked_at = 0;
        }
        fs::write(&path, serde_json::to_vec(&cache).unwrap()).unwrap();
        assert_eq!(run(&server, &["/"], &config).verified, 1);
        assert_eq!(server.count(), 2);
        assert_eq!(
            run(
                &server,
                &["/"],
                &NetworkConfig {
                    max_body_bytes: 1,
                    ..config.clone()
                }
            )
            .verified,
            1
        );
        assert_eq!(server.count(), 3);
        let bytes = fs::read_to_string(path).unwrap();
        assert!(!bytes.contains(&server.base));
    }

    #[test]
    fn query_and_auth_evidence_are_not_cached_and_corrupt_cache_is_reported() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("evidence.json");
        let mut config = NetworkConfig {
            cache: Some(path.to_string_lossy().into()),
            ..config()
        };
        let server = Server::new(|_| Reply::status(200));
        for _ in 0..2 {
            assert_eq!(run(&server, &["/?token=secret"], &config).verified, 1);
        }
        assert_eq!(server.count(), 2);
        assert!(!fs::read_to_string(&path).unwrap().contains("secret"));
        config.auth_env.insert("127.0.0.1".into(), "PATH".into());
        for _ in 0..2 {
            assert_eq!(run(&server, &["/auth"], &config).verified, 1);
        }
        assert_eq!(server.count(), 4);
        fs::write(&path, "broken cache").unwrap();
        let result = run(&server, &["/"], &config);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.rule == "external.cache"
                    && diagnostic.outcome == Outcome::Unverified)
        );
    }

    #[test]
    fn timeouts_are_unverified_and_host_requests_are_paced() {
        let stalled = Server::new(|_| {
            std::thread::sleep(Duration::from_millis(1100));
            Reply::status(200)
        });
        let result = run(
            &stalled,
            &["/"],
            &NetworkConfig {
                timeout_seconds: 1,
                ..config()
            },
        );
        assert_eq!(result.diagnostics[0].outcome, Outcome::Unverified);
        let server = Server::new(|_| Reply::status(200));
        let start = Instant::now();
        assert_eq!(
            run(
                &server,
                &["/a", "/b", "/c"],
                &NetworkConfig {
                    host_delay_ms: 100,
                    concurrency: 3,
                    ..config()
                }
            )
            .verified,
            3
        );
        // Server worker scheduling can change the spacing between callbacks.
        // Three requests must still wait through two 100 ms host slots.
        assert!(start.elapsed() >= Duration::from_millis(200));
    }
}
