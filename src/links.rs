use crate::{config::Config, model::*};
use percent_encoding::percent_decode_str;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Component, Path, PathBuf},
};
use url::Url;

pub const RULES: &[RuleDefinition] = &[
    RuleDefinition {
        id: "uri.syntax",
        family: 11,
        requirement: Requirement::Format,
        description: "References must have valid URI syntax and allowed schemes.",
    },
    RuleDefinition {
        id: "link.exists",
        family: 10,
        requirement: Requirement::Profile,
        description: "Local link targets must exist and be readable.",
    },
    RuleDefinition {
        id: "link.case",
        family: 10,
        requirement: Requirement::Profile,
        description: "Link path case must match the target path.",
    },
    RuleDefinition {
        id: "link.boundary",
        family: 10,
        requirement: Requirement::Project,
        description: "Local links must stay within the declared root.",
    },
    RuleDefinition {
        id: "link.anchor",
        family: 9,
        requirement: Requirement::Profile,
        description: "Fragments must resolve to a target anchor.",
    },
    RuleDefinition {
        id: "link.type",
        family: 17,
        requirement: Requirement::Format,
        description: "ID associations must point to the required element type.",
    },
    RuleDefinition {
        id: "link.available",
        family: 10,
        requirement: Requirement::Execution,
        description: "Required link targets must be available to the scan.",
    },
];

#[derive(Debug, Clone)]
pub enum Target {
    External(String),
    Local {
        path: PathBuf,
        fragment: Option<String>,
    },
    Data(String),
    Other,
}

pub fn clean_path(path: &Path) -> PathBuf {
    let mut clean = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                clean.pop();
            }
            other => clean.push(other.as_os_str()),
        }
    }
    clean
}

fn validate_escapes(value: &str) -> Result<(), String> {
    let bytes = value.as_bytes();
    for i in 0..bytes.len() {
        if bytes[i] == b'%'
            && (i + 2 >= bytes.len()
                || !bytes[i + 1].is_ascii_hexdigit()
                || !bytes[i + 2].is_ascii_hexdigit())
        {
            return Err("Invalid percent escape in reference".into());
        }
    }
    Ok(())
}

pub fn decode(value: &str) -> Result<String, String> {
    validate_escapes(value)?;
    percent_decode_str(value)
        .decode_utf8()
        .map(|v| v.into_owned())
        .map_err(|_| "URI escapes do not form valid UTF-8".into())
}

pub fn resolve(
    target: &str,
    source: &Path,
    root: &Path,
    config: &Config,
    routes: &BTreeMap<String, PathBuf>,
) -> Result<Target, String> {
    resolve_with_route(target, source, root, config, routes, None)
}

fn resolve_with_route(
    target: &str,
    source: &Path,
    root: &Path,
    config: &Config,
    routes: &BTreeMap<String, PathBuf>,
    primary_route: Option<&str>,
) -> Result<Target, String> {
    if target.chars().any(|c| c.is_control()) {
        return Err("Reference contains a control character".into());
    }
    let (plain, fragment) = target
        .split_once('#')
        .map_or((target, None), |(path, fragment)| (path, Some(fragment)));
    let fragment = fragment.map(decode).transpose()?;
    let plain = plain.split_once('?').map_or(plain, |(path, _)| path);
    validate_escapes(target)?;
    let candidate = if target.starts_with("//") {
        format!("https:{target}")
    } else {
        target.to_owned()
    };
    let scheme = candidate.split_once(':').filter(|(prefix, _)| {
        !prefix.is_empty()
            && prefix.bytes().enumerate().all(|(i, b)| {
                b.is_ascii_alphabetic()
                    || (i > 0 && (b.is_ascii_digit() || matches!(b, b'+' | b'-' | b'.')))
            })
    });
    if let Some((scheme, _)) = scheme {
        let scheme = scheme.to_ascii_lowercase();
        if !config.links.allowed_schemes.contains(&scheme) {
            return Err(format!("URI scheme is not allowed: {scheme}"));
        }
        let url = Url::parse(&candidate).map_err(|_| "Invalid absolute URI".to_owned())?;
        if matches!(scheme.as_str(), "http" | "https" | "ftp") {
            if url.host_str().is_none() || !url.username().is_empty() || url.password().is_some() {
                return Err("Network URI needs a host and must not contain credentials".into());
            }
            return Ok(Target::External(url.to_string()));
        }
        if scheme == "data" {
            return Ok(Target::Data(url.to_string()));
        }
        if scheme == "mailto" {
            let address = decode(url.path())?;
            if address.is_empty()
                || address.split(',').any(|value| {
                    let pair = value.rsplit_once('@');
                    pair.is_none_or(|(local, host)| {
                        local.is_empty()
                            || host.is_empty()
                            || host.contains(' ')
                            || local.chars().any(|c| c.is_control())
                    })
                })
            {
                return Err("Invalid mailto address structure".into());
            }
        }
        if scheme == "tel" {
            let number = decode(url.path())?;
            let number = number.split(';').next().unwrap_or("");
            if !number.chars().any(|c| c.is_ascii_digit())
                || number.chars().any(|c| {
                    !c.is_ascii_digit() && !matches!(c, '+' | '-' | '.' | '(' | ')' | '*' | '#')
                })
            {
                return Err("Invalid telephone URI structure".into());
            }
        }
        return Ok(Target::Other);
    }
    if target.contains('\\') {
        return Err("Use forward slashes in a document URI".into());
    }
    let plain = decode(plain)?;
    let path = if plain.is_empty() {
        source.to_path_buf()
    } else if config.routes.enabled {
        let route = if plain.starts_with('/') {
            crate::collection::normalize_decoded_route(&plain)
        } else {
            let source_route = primary_route.or_else(|| {
                routes
                    .iter()
                    .find_map(|(route, path)| (path == source).then_some(route.as_str()))
            });
            if let Some(source_route) = source_route {
                let source_route =
                    if config.routes.directory_urls && !source_route.ends_with(".html") {
                        format!("{source_route}/")
                    } else {
                        source_route.to_owned()
                    };
                let mut base = Url::parse("https://perfect-doc.invalid/")
                    .map_err(|_| "Invalid source route")?;
                base.set_path(&source_route.replace('%', "%25"));
                let raw_path = target.split(['?', '#']).next().unwrap_or("");
                crate::collection::normalize_route(
                    base.join(raw_path)
                        .map_err(|_| "Invalid relative route")?
                        .path(),
                )
            } else {
                String::new()
            }
        };
        routes.get(&route).cloned().unwrap_or_else(|| {
            if plain.starts_with('/') {
                root.join(plain.trim_start_matches('/'))
            } else {
                source.parent().unwrap_or(root).join(&plain)
            }
        })
    } else if plain.starts_with('/') {
        root.join(plain.trim_start_matches('/'))
    } else {
        source.parent().unwrap_or(root).join(&plain)
    };
    let mut path = clean_path(&path);
    if path.is_dir() {
        let matches: Vec<_> = config
            .links
            .index_files
            .iter()
            .map(|index| path.join(index))
            .filter(|p| p.is_file())
            .collect();
        if matches.len() > 1 {
            return Err("Directory link has more than one configured index file".into());
        }
        if let Some(index) = matches.into_iter().next() {
            path = index;
        }
    }
    Ok(Target::Local { path, fragment })
}

#[derive(Default)]
pub struct LinkResult {
    pub diagnostics: Vec<Diagnostic>,
    pub external: Vec<ExternalLink>,
    pub edges: Vec<(usize, usize)>,
    pub assets: Vec<(usize, Reference, PathBuf)>,
    pub verified: usize,
    pub referenced_paths: BTreeSet<PathBuf>,
}

pub fn check(
    documents: &[Document],
    root: &Path,
    config: &Config,
    routes: &BTreeMap<String, PathBuf>,
) -> LinkResult {
    let indices: BTreeMap<_, _> = documents
        .iter()
        .enumerate()
        .map(|(i, d)| (d.source.path.clone(), i))
        .collect();
    let mut result = LinkResult::default();
    for (index, document) in documents.iter().enumerate() {
        let profile = config.profile_for(&document.source.display_path);
        let mut base_path = document.source.path.clone();
        let html_base = document
            .parsed
            .elements
            .iter()
            .find(|e| e.tag == "base")
            .and_then(|e| e.attrs.get("href"));
        let external_base = html_base
            .and_then(|s| Url::parse(s).ok())
            .filter(|u| matches!(u.scheme(), "http" | "https"));
        if let Some(base) = html_base.filter(|_| external_base.is_none())
            && let Ok(Target::Local { path, .. }) =
                resolve(base, &base_path, root, &profile, routes)
        {
            base_path = if base.ends_with('/') {
                path.join("__document__.html")
            } else {
                path
            };
        }
        for reference in &document.parsed.references {
            if reference.kind == ReferenceKind::Include {
                continue;
            }
            let target = if reference.kind == ReferenceKind::Id {
                reference.target.clone()
            } else if let Some(base) = &external_base {
                base.join(&reference.target)
                    .map(|u| u.to_string())
                    .unwrap_or_else(|_| reference.target.clone())
            } else {
                reference.target.clone()
            };
            let at = |rule, requirement, outcome, message: String| {
                document
                    .source
                    .diagnostic(rule, requirement, outcome, reference.offset, message)
                    .with_target(&target)
            };
            if profile.links.template_references && target.contains(['{', '}']) {
                result.diagnostics.push(at(
                    "link.available",
                    Requirement::Execution,
                    Outcome::Unverified,
                    "Reference contains an unresolved template or expression".into(),
                ));
                continue;
            }
            match resolve_with_route(
                &target,
                if reference.kind == ReferenceKind::Id {
                    &document.source.path
                } else if html_base.is_some() || target.starts_with('#') || target.is_empty() {
                    &base_path
                } else {
                    document.source.path_at(reference.offset)
                },
                root,
                &profile,
                routes,
                if reference.kind == ReferenceKind::Id
                    || html_base.is_some()
                    || (document.source.path_at(reference.offset) != document.source.path
                        && !target.starts_with('#')
                        && !target.is_empty())
                {
                    None
                } else {
                    document.route.as_deref()
                },
            ) {
                Err(error) => result.diagnostics.push(at(
                    "uri.syntax",
                    Requirement::Format,
                    Outcome::Invalid,
                    error,
                )),
                Ok(Target::External(url)) => {
                    if url.starts_with("http:") || url.starts_with("https:") {
                        result.external.push(ExternalLink {
                            url,
                            location: document.source.location(reference.offset),
                        });
                    }
                }
                Ok(Target::Data(data)) => {
                    if let Err((outcome, message)) =
                        crate::assets::validate_data(&data, reference.kind, &profile)
                    {
                        result.diagnostics.push(at(
                            if outcome == Outcome::Invalid {
                                "uri.syntax"
                            } else {
                                "asset.available"
                            },
                            if outcome == Outcome::Invalid {
                                Requirement::Format
                            } else {
                                Requirement::Execution
                            },
                            outcome,
                            message,
                        ));
                    } else {
                        result.verified += 1;
                    }
                }
                Ok(Target::Other) => result.verified += 1,
                Ok(Target::Local { path, fragment }) => {
                    if !profile.links.allow_outside_root && !path.starts_with(root) {
                        result.diagnostics.push(at(
                            "link.boundary",
                            Requirement::Project,
                            Outcome::Invalid,
                            "Link leaves the declared root".into(),
                        ));
                        continue;
                    }
                    if !path.exists() {
                        if profile.links.strict_case && wrong_case(&path, root).is_some() {
                            result.diagnostics.push(at(
                                "link.case",
                                Requirement::Profile,
                                Outcome::Invalid,
                                "Link path case does not match the target".into(),
                            ));
                        } else {
                            result.diagnostics.push(at(
                                "link.exists",
                                Requirement::Profile,
                                Outcome::Invalid,
                                format!(
                                    "Local target does not exist: {}",
                                    path.strip_prefix(root).unwrap_or(&path).display()
                                ),
                            ));
                        }
                        continue;
                    }
                    if !path.is_file() {
                        result.diagnostics.push(at(
                            "link.exists",
                            Requirement::Profile,
                            Outcome::Invalid,
                            "Link target is not a file and has no configured index".into(),
                        ));
                        continue;
                    }
                    let canonical = match std::fs::canonicalize(&path) {
                        Ok(p) => p,
                        Err(_) => {
                            result.diagnostics.push(at(
                                "link.available",
                                Requirement::Execution,
                                Outcome::Unverified,
                                "Cannot resolve the local target".into(),
                            ));
                            continue;
                        }
                    };
                    if !profile.links.allow_outside_root && !canonical.starts_with(root) {
                        result.diagnostics.push(at(
                            "link.boundary",
                            Requirement::Project,
                            Outcome::Invalid,
                            "Symbolic link target leaves the declared root".into(),
                        ));
                        continue;
                    }
                    if profile.links.strict_case && wrong_case(&path, root).is_some() {
                        result.diagnostics.push(at(
                            "link.case",
                            Requirement::Profile,
                            Outcome::Invalid,
                            "Link path case does not match the target".into(),
                        ));
                        continue;
                    }
                    result.referenced_paths.insert(canonical.clone());
                    let target_index = indices.get(&canonical).copied();
                    if let Some(target_index) = target_index {
                        result.edges.push((index, target_index));
                    }
                    if matches!(reference.kind, ReferenceKind::Image | ReferenceKind::Asset)
                        || reference.integrity.is_some()
                    {
                        result
                            .assets
                            .push((index, reference.clone(), canonical.clone()));
                    }
                    if let Some(fragment) = fragment.filter(|f| !f.is_empty()) {
                        let fragment = fragment.split(":~:text=").next().unwrap_or("");
                        if fragment.is_empty() {
                            result.verified += 1;
                            continue;
                        }
                        if let Some(target_index) = target_index {
                            let target_doc = &documents[target_index];
                            if let Some(anchor) =
                                target_doc.parsed.anchors.iter().find(|a| a.id == fragment)
                            {
                                if let Some(expected) = &reference.expected_tag {
                                    let correct = if expected == "labelable" {
                                        matches!(
                                            anchor.tag.as_str(),
                                            "button"
                                                | "input"
                                                | "meter"
                                                | "output"
                                                | "progress"
                                                | "select"
                                                | "textarea"
                                        ) && !target_doc.parsed.elements.iter().any(|e| {
                                            e.offset == anchor.offset
                                                && e.attrs
                                                    .get("type")
                                                    .is_some_and(|t| t == "hidden")
                                        })
                                    } else {
                                        expected.split('|').any(|tag| tag == anchor.tag)
                                    };
                                    if !correct {
                                        result.diagnostics.push(at(
                                            "link.type",
                                            Requirement::Format,
                                            Outcome::Invalid,
                                            format!(
                                                "ID reference requires {expected}, found {}",
                                                anchor.tag
                                            ),
                                        ));
                                        continue;
                                    }
                                }
                            } else if target_doc.parsed.dynamic_anchors {
                                result.diagnostics.push(at(
                                    "link.available",
                                    Requirement::Execution,
                                    Outcome::Unverified,
                                    "The target can create anchors at run time".into(),
                                ));
                                continue;
                            } else {
                                result.diagnostics.push(at(
                                    "link.anchor",
                                    Requirement::Profile,
                                    Outcome::Invalid,
                                    format!("Target anchor does not exist: #{fragment}"),
                                ));
                                continue;
                            }
                        } else if canonical
                            .extension()
                            .is_some_and(|e| e.eq_ignore_ascii_case("svg"))
                        {
                            match std::fs::read_to_string(&canonical).ok().and_then(|text| {
                                roxmltree::Document::parse(&text).ok().map(|xml| {
                                    xml.descendants()
                                        .any(|n| n.attribute("id") == Some(fragment))
                                })
                            }) {
                                Some(true) => {}
                                Some(false) => {
                                    result.diagnostics.push(at(
                                        "link.anchor",
                                        Requirement::Profile,
                                        Outcome::Invalid,
                                        format!("SVG anchor does not exist: #{fragment}"),
                                    ));
                                    continue;
                                }
                                None => {
                                    result.diagnostics.push(at(
                                        "link.available",
                                        Requirement::Execution,
                                        Outcome::Unverified,
                                        "Cannot parse the SVG target".into(),
                                    ));
                                    continue;
                                }
                            }
                        } else {
                            result.diagnostics.push(at(
                                "link.available",
                                Requirement::Execution,
                                Outcome::Unverified,
                                "The target was not parsed; its fragment cannot be checked".into(),
                            ));
                            continue;
                        }
                    }
                    if std::fs::File::open(&canonical).is_err() {
                        result.diagnostics.push(at(
                            "link.available",
                            Requirement::Execution,
                            Outcome::Unverified,
                            "Cannot read the local target".into(),
                        ));
                    } else {
                        result.verified += 1;
                    }
                }
            }
        }
    }
    result
}

fn wrong_case(path: &Path, root: &Path) -> Option<PathBuf> {
    let relative = path.strip_prefix(root).ok()?;
    let mut current = root.to_path_buf();
    let mut differs = false;
    for component in relative.components() {
        let expected = component.as_os_str();
        let entries: Vec<_> = std::fs::read_dir(&current)
            .ok()?
            .filter_map(Result::ok)
            .collect();
        if let Some(exact) = entries.iter().find(|e| e.file_name() == expected) {
            current = exact.path();
        } else {
            let found = entries.iter().find(|e| {
                e.file_name().to_string_lossy().to_lowercase()
                    == expected.to_string_lossy().to_lowercase()
            })?;
            current = found.path();
            differs = true;
        }
    }
    differs.then_some(current)
}
