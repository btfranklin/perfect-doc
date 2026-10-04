use crate::{
    config::Config,
    links::{self, Target},
    metadata,
    model::*,
};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    io::Read,
    path::{Path, PathBuf},
};

pub const RULES: &[RuleDefinition] = &[
    RuleDefinition {
        id: "metadata.unique",
        family: 5,
        requirement: Requirement::Project,
        description: "Selected metadata values must be unique across the collection.",
    },
    RuleDefinition {
        id: "route.unique",
        family: 12,
        requirement: Requirement::Profile,
        description: "Publication routes and aliases must be unique.",
    },
    RuleDefinition {
        id: "route.target",
        family: 12,
        requirement: Requirement::Profile,
        description: "Redirects must reach a declared target without a cycle.",
    },
    RuleDefinition {
        id: "navigation.structure",
        family: 13,
        requirement: Requirement::Project,
        description: "Navigation must have valid targets and unique parent relationships.",
    },
    RuleDefinition {
        id: "collection.reachable",
        family: 13,
        requirement: Requirement::Project,
        description: "Required pages must be reachable from declared entrypoints.",
    },
    RuleDefinition {
        id: "include.target",
        family: 14,
        requirement: Requirement::Project,
        description: "Includes must have valid targets, regions, and line ranges.",
    },
    RuleDefinition {
        id: "include.cycle",
        family: 14,
        requirement: Requirement::Project,
        description: "Includes must not recurse or exceed the configured depth.",
    },
    RuleDefinition {
        id: "anchor.unique",
        family: 9,
        requirement: Requirement::Profile,
        description: "Explicit and generated anchors must not collide.",
    },
    RuleDefinition {
        id: "anchor.preserved",
        family: 9,
        requirement: Requirement::Project,
        description: "Declared public anchors must remain available.",
    },
    RuleDefinition {
        id: "build.output",
        family: 21,
        requirement: Requirement::Project,
        description: "Supplied build output must contain required pages and source anchors.",
    },
];

/// Decode a URI route once before path normalization.
pub fn normalize_route(route: &str) -> String {
    normalize_decoded_route(&links::decode(route).unwrap_or_else(|_| route.into()))
}

/// Normalize a route whose percent escapes are already decoded.
pub fn normalize_decoded_route(route: &str) -> String {
    let path = links::clean_path(Path::new(&format!("/{}", route.trim_start_matches('/'))));
    let value = path.to_string_lossy().replace('\\', "/");
    let value = value.trim_end_matches('/');
    if value.is_empty() {
        "/".into()
    } else {
        value.into()
    }
}

fn valid_route(route: &str) -> bool {
    let lower = route.to_ascii_lowercase();
    if !route.starts_with('/')
        || route.starts_with("//")
        || route.contains(['?', '#', '\\'])
        || lower.contains("%2f")
        || lower.contains("%5c")
    {
        return false;
    }
    links::decode(route).is_ok_and(|decoded| {
        !decoded.chars().any(char::is_control)
            && !decoded.contains('\\')
            && !decoded.split('/').any(|part| matches!(part, "." | ".."))
    })
}

pub fn index_routes(
    documents: &mut [Document],
    root: &Path,
    config: &Config,
    diagnostics: &mut Vec<Diagnostic>,
) -> BTreeMap<String, PathBuf> {
    let mut routes = BTreeMap::new();
    let mut unique: BTreeMap<(String, String), Location> = BTreeMap::new();
    let mut fields = config.frontmatter.unique_fields.clone();
    if let Some(field) = &config.frontmatter.id_field
        && !fields.contains(field)
    {
        fields.push(field.clone());
    }
    for document in documents {
        if let Some(metadata) = &document.metadata {
            for key in &fields {
                if let Some(value) = metadata::field(metadata, key).filter(|v| !v.is_null()) {
                    let key = (key.clone(), value.to_string());
                    if let Some(previous) = unique.insert(key.clone(), document.source.location(0))
                    {
                        let mut d = document.source.diagnostic(
                            "metadata.unique",
                            Requirement::Project,
                            Outcome::Invalid,
                            0,
                            format!("Metadata field is not unique: {}", key.0),
                        );
                        d.related.push(previous);
                        diagnostics.push(d);
                    }
                }
            }
        }
        if !config.routes.enabled {
            continue;
        }
        let declared = document
            .metadata
            .as_ref()
            .and_then(|v| {
                config
                    .frontmatter
                    .route_field
                    .as_ref()
                    .and_then(|key| metadata::field(v, key))
            })
            .and_then(Value::as_str);
        let route = if let Some(declared) = declared {
            if !valid_route(declared) {
                diagnostics.push(document.source.diagnostic(
                    "route.target",
                    Requirement::Profile,
                    Outcome::Invalid,
                    0,
                    "Declared route must be an absolute URL path without a query or fragment",
                ));
                continue;
            }
            normalize_route(declared)
        } else {
            let relative = document
                .source
                .path
                .strip_prefix(root)
                .unwrap_or(&document.source.path);
            let mut path = relative.with_extension("");
            if path
                .file_name()
                .is_some_and(|s| s == "index" || s == "README")
            {
                path.pop();
            }
            let mut route = normalize_decoded_route(&format!(
                "{}/{}",
                normalize_route(&config.routes.base).trim_end_matches('/'),
                path.to_string_lossy().replace('\\', "/")
            ));
            if config.routes.html_extension && route != "/" {
                route.push_str(".html");
            }
            route
        };
        document.route = Some(route.clone());
        let mut aliases = vec![route];
        if let Some(values) = document.metadata.as_ref().and_then(|v| {
            config
                .frontmatter
                .aliases_field
                .as_ref()
                .and_then(|key| metadata::field(v, key))
        }) {
            if let Some(array) = values.as_array() {
                for value in array {
                    if let Some(alias) = value.as_str().filter(|s| valid_route(s)) {
                        aliases.push(normalize_route(alias));
                    } else {
                        diagnostics.push(document.source.diagnostic(
                            "route.target",
                            Requirement::Profile,
                            Outcome::Invalid,
                            0,
                            "Route aliases must be absolute URL path strings",
                        ));
                    }
                }
            } else {
                diagnostics.push(document.source.diagnostic(
                    "route.target",
                    Requirement::Profile,
                    Outcome::Invalid,
                    0,
                    "Route aliases must be an array",
                ));
            }
        }
        for alias in aliases {
            if let Some(previous) = routes.insert(alias.clone(), document.source.path.clone()) {
                diagnostics.push(document.source.diagnostic(
                    "route.unique",
                    Requirement::Profile,
                    Outcome::Invalid,
                    0,
                    format!(
                        "Route is used more than once: {alias} (also {})",
                        previous.strip_prefix(root).unwrap_or(&previous).display()
                    ),
                ));
            }
        }
    }
    let mut redirect_map = BTreeMap::new();
    for (from, to) in &config.routes.redirects {
        let location = Location {
            path: "perfect-doc.toml".into(),
            line: 1,
            column: 1,
            byte_offset: 0,
        };
        if !valid_route(from) || !valid_route(to) {
            diagnostics.push(Diagnostic::at("route.target",Requirement::Profile,Outcome::Invalid,location,"Redirects require absolute route paths with valid escapes and no encoded separators or traversal"));
            continue;
        }
        let from = normalize_route(from);
        if redirect_map
            .insert(from.clone(), normalize_route(to))
            .is_some()
        {
            diagnostics.push(Diagnostic::at(
                "route.unique",
                Requirement::Profile,
                Outcome::Invalid,
                location,
                format!("Redirect source is used more than once: {from}"),
            ));
        }
    }
    for (from, target) in &redirect_map {
        let mut seen = BTreeSet::from([from.clone()]);
        let mut to = target;
        while let Some(next) = redirect_map.get(to) {
            if !seen.insert(to.clone()) {
                break;
            }
            to = next;
        }
        let location = Location {
            path: "perfect-doc.toml".into(),
            line: 1,
            column: 1,
            byte_offset: 0,
        };
        if seen.contains(to) {
            diagnostics.push(Diagnostic::at(
                "route.target",
                Requirement::Profile,
                Outcome::Invalid,
                location,
                format!("Redirect cycle from {from}"),
            ));
        } else if let Some(path) = routes.get(to).cloned() {
            if routes.insert(from.clone(), path).is_some() {
                diagnostics.push(Diagnostic::at(
                    "route.unique",
                    Requirement::Profile,
                    Outcome::Invalid,
                    location,
                    format!("Redirect replaces a page route: {from}"),
                ));
            }
        } else {
            diagnostics.push(Diagnostic::at(
                "route.target",
                Requirement::Profile,
                Outcome::Invalid,
                location,
                format!("Redirect target does not exist: {target}"),
            ));
        }
    }
    routes
}

pub fn check(
    documents: &[Document],
    root: &Path,
    config: &Config,
    routes: &BTreeMap<String, PathBuf>,
    link_edges: &[(usize, usize)],
    include_edges: &[(usize, usize)],
    diagnostics: &mut Vec<Diagnostic>,
) {
    let indices: BTreeMap<_, _> = documents
        .iter()
        .enumerate()
        .map(|(i, d)| (d.source.path.clone(), i))
        .collect();
    for document in documents {
        let mut anchors = BTreeMap::new();
        for anchor in &document.parsed.anchors {
            if let Some(previous) = anchors.insert(&anchor.id, anchor)
                && (previous.tag != anchor.tag
                    || (document.format == Format::Markdown
                        && (previous.tag.starts_with('h') || anchor.tag.starts_with('h'))))
            {
                diagnostics.push(document.source.diagnostic(
                    "anchor.unique",
                    Requirement::Profile,
                    Outcome::Invalid,
                    anchor.offset,
                    format!("Anchor is used more than once: {}", anchor.id),
                ));
            }
        }
    }
    for (target, ids) in &config.routes.preserved_anchors {
        let context = root.join("__contract__.md");
        let target_path = links::resolve(target, &context, root, config, routes)
            .ok()
            .and_then(|t| {
                if let Target::Local { path, .. } = t {
                    std::fs::canonicalize(path).ok()
                } else {
                    None
                }
            });
        let document = target_path
            .as_ref()
            .and_then(|p| indices.get(p))
            .map(|i| &documents[*i]);
        for id in ids {
            if document.is_none_or(|d| !d.parsed.anchors.iter().any(|a| &a.id == id)) {
                diagnostics.push(Diagnostic::at(
                    "anchor.preserved",
                    Requirement::Project,
                    Outcome::Invalid,
                    document.map_or(
                        Location {
                            path: target.clone(),
                            line: 1,
                            column: 1,
                            byte_offset: 0,
                        },
                        |d| d.source.location(0),
                    ),
                    format!("Required public anchor is missing: #{id}"),
                ));
            }
        }
    }
    if let (Some(id_field), Some(parent_field)) = (
        &config.frontmatter.id_field,
        &config.frontmatter.parent_field,
    ) {
        let ids: BTreeMap<_, _> = documents
            .iter()
            .enumerate()
            .filter_map(|(i, d)| {
                d.metadata
                    .as_ref()
                    .and_then(|v| metadata::field(v, id_field))
                    .and_then(Value::as_str)
                    .map(|id| (id, i))
            })
            .collect();
        let mut parents = Vec::new();
        for (index, document) in documents.iter().enumerate() {
            if let Some(parent) = document
                .metadata
                .as_ref()
                .and_then(|v| metadata::field(v, parent_field))
                .and_then(Value::as_str)
            {
                if let Some(parent) = ids.get(parent) {
                    parents.push((index, *parent));
                } else {
                    diagnostics.push(document.source.diagnostic(
                        "navigation.structure",
                        Requirement::Project,
                        Outcome::Invalid,
                        0,
                        "Metadata parent ID does not exist",
                    ));
                }
            }
        }
        for (index, problem) in graph_problems(documents.len(), &parents, documents.len() + 1) {
            diagnostics.push(documents[index].source.diagnostic(
                "navigation.structure",
                Requirement::Project,
                Outcome::Invalid,
                0,
                problem,
            ));
        }
    }
    let mut navigation_targets = BTreeSet::new();
    for navigation in &config.collection.navigation {
        let path = config.resolve(&navigation.path);
        let source = read_supporting_source(&path, config);
        match source {
            Ok(source) => {
                let format = path.extension().and_then(|s| s.to_str()).unwrap_or("");
                match metadata::parse_data(&source.text, format) {
                    Ok(value) => {
                        let mut targets = Vec::new();
                        navigation_nodes(
                            &value,
                            &navigation.target_key,
                            &navigation.children_key,
                            &mut targets,
                            &source,
                            diagnostics,
                            0,
                        );
                        let mut context = documents.to_vec();
                        for document in &mut context {
                            document.parsed.references.clear();
                        }
                        context.push(Document {
                            source: source.clone(),
                            format: Format::Html,
                            parsed: ParsedDocument {
                                references: targets
                                    .iter()
                                    .map(|target| {
                                        Reference::new(target, 0, ReferenceKind::Navigation)
                                    })
                                    .collect(),
                                ..ParsedDocument::default()
                            },
                            metadata: None,
                            route: None,
                        });
                        diagnostics
                            .extend(links::check(&context, root, config, routes).diagnostics);
                        for target in targets {
                            match links::resolve(&target, &source.path, root, config, routes) {
                                Ok(Target::Local { path, .. }) => {
                                    if let Ok(path) = std::fs::canonicalize(path) {
                                        if let Some(index) = indices.get(&path) {
                                            if !navigation_targets.insert(*index) {
                                                diagnostics.push(source.diagnostic(
                                                    "navigation.structure",
                                                    Requirement::Project,
                                                    Outcome::Invalid,
                                                    0,
                                                    format!(
                                                        "Navigation target is repeated: {target}"
                                                    ),
                                                ));
                                            }
                                        } else {
                                            diagnostics.push(source.diagnostic("navigation.structure",Requirement::Project,Outcome::Invalid,0,format!("Navigation target is not a selected document: {target}")));
                                        }
                                    } else {
                                        diagnostics.push(source.diagnostic(
                                            "navigation.structure",
                                            Requirement::Project,
                                            Outcome::Invalid,
                                            0,
                                            format!("Navigation target does not exist: {target}"),
                                        ));
                                    }
                                }
                                Ok(Target::External(_)) => {}
                                _ => diagnostics.push(source.diagnostic(
                                    "navigation.structure",
                                    Requirement::Project,
                                    Outcome::Invalid,
                                    0,
                                    "Invalid navigation target",
                                )),
                            }
                        }
                    }
                    Err(error) => diagnostics.push(source.diagnostic(
                        "navigation.structure",
                        Requirement::Project,
                        Outcome::Invalid,
                        0,
                        error,
                    )),
                }
            }
            Err(error) => diagnostics.push(Diagnostic::at(
                "navigation.structure",
                Requirement::Project,
                Outcome::Unverified,
                Location {
                    path: navigation.path.clone(),
                    line: 1,
                    column: 1,
                    byte_offset: 0,
                },
                error,
            )),
        }
    }
    if config.collection.require_navigation_coverage {
        for (index, document) in documents.iter().enumerate() {
            if !navigation_targets.contains(&index) {
                diagnostics.push(document.source.diagnostic(
                    "navigation.structure",
                    Requirement::Project,
                    Outcome::Invalid,
                    0,
                    "Document is absent from the navigation manifests",
                ));
            }
        }
    }
    if config.collection.require_reachable {
        let mut queue = VecDeque::new();
        for target in &config.collection.entrypoints {
            if let Ok(Target::Local { path, .. }) =
                links::resolve(target, &root.join("__entry__.md"), root, config, routes)
                && let Some(index) = std::fs::canonicalize(path)
                    .ok()
                    .and_then(|p| indices.get(&p).copied())
            {
                queue.push_back(index);
                continue;
            }
            diagnostics.push(Diagnostic::at(
                "collection.reachable",
                Requirement::Project,
                Outcome::Invalid,
                Location {
                    path: target.clone(),
                    line: 1,
                    column: 1,
                    byte_offset: 0,
                },
                "Entrypoint is not a selected document",
            ));
        }
        let mut seen = BTreeSet::new();
        let mut outgoing = vec![Vec::new(); documents.len()];
        for &(a, b) in link_edges.iter().chain(include_edges) {
            outgoing[a].push(b);
        }
        while let Some(index) = queue.pop_front() {
            if seen.insert(index) {
                queue.extend(&outgoing[index]);
            }
        }
        for (index, document) in documents.iter().enumerate() {
            if !seen.contains(&index) {
                diagnostics.push(document.source.diagnostic(
                    "collection.reachable",
                    Requirement::Project,
                    Outcome::Invalid,
                    0,
                    "Document is not reachable from the declared entrypoints",
                ));
            }
        }
    }
    if config.routes.require_output {
        check_output(documents, root, config, diagnostics);
    }
}

pub fn check_includes(
    documents: &[Document],
    root: &Path,
    config: &Config,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<(usize, usize)> {
    let indices: BTreeMap<_, _> = documents
        .iter()
        .enumerate()
        .map(|(i, d)| (d.source.path.clone(), i))
        .collect();
    let mut physical = config.clone();
    physical.routes.enabled = false;
    let mut include_edges = Vec::new();
    for (index, document) in documents.iter().enumerate() {
        for include in &document.parsed.includes {
            let literal = document
                .parsed
                .code_blocks
                .iter()
                .any(|code| code.offset == include.offset);
            let at = |outcome, message| {
                document
                    .source
                    .diagnostic(
                        "include.target",
                        Requirement::Project,
                        outcome,
                        include.offset,
                        message,
                    )
                    .with_target(&include.target)
            };
            let target = links::resolve(
                &include.target,
                &document.source.path,
                root,
                &physical,
                &BTreeMap::new(),
            );
            let Ok(Target::Local { path, .. }) = target else {
                diagnostics.push(at(
                    Outcome::Invalid,
                    "Includes require a local file target".to_owned(),
                ));
                continue;
            };
            let Ok(path) = std::fs::canonicalize(path) else {
                diagnostics.push(at(Outcome::Invalid, "Include target does not exist".into()));
                continue;
            };
            if !config.links.allow_outside_root && !path.starts_with(root) {
                diagnostics.push(at(
                    Outcome::Invalid,
                    "Include leaves the declared root".into(),
                ));
                continue;
            }
            let text = match read_supporting_source(&path, config) {
                Ok(source) => source.text,
                Err(error) => {
                    diagnostics.push(at(
                        Outcome::Unverified,
                        format!("Include target is unavailable: {error}"),
                    ));
                    continue;
                }
            };
            if !literal && let Some(child) = indices.get(&path) {
                include_edges.push((index, *child));
            }
            let lines = text.lines().count();
            if include.start_line.is_some_and(|n| n == 0 || n > lines)
                || include.end_line.is_some_and(|n| {
                    n == 0 || n > lines || include.start_line.is_some_and(|start| start > n)
                })
            {
                diagnostics.push(at(
                    Outcome::Invalid,
                    "Include line range is outside the target".into(),
                ));
            }
            if let Some(region) = &include.region {
                let start = format!("<!-- region {region} -->");
                let end = format!("<!-- endregion {region} -->");
                let starts: Vec<_> = text.match_indices(&start).map(|(i, _)| i).collect();
                let ends: Vec<_> = text.match_indices(&end).map(|(i, _)| i).collect();
                if starts.len() != 1 || ends.len() != 1 || starts[0] >= ends[0] {
                    diagnostics.push(at(
                        Outcome::Invalid,
                        format!("Include region needs one ordered marker pair: {region}"),
                    ));
                }
            }
        }
    }
    for (index, problem) in graph_problems(
        documents.len(),
        &include_edges,
        config.collection.include_max_depth,
    ) {
        diagnostics.push(documents[index].source.diagnostic(
            "include.cycle",
            Requirement::Project,
            Outcome::Invalid,
            0,
            problem,
        ));
    }
    include_edges
}

pub fn read_supporting_source(path: &Path, config: &Config) -> Result<Source, String> {
    let before = std::fs::metadata(path).map_err(|error| error.to_string())?;
    if !before.is_file() {
        return Err("Supporting input is not a regular file".into());
    }
    if before.len() > config.files.max_file_bytes {
        return Err("Supporting file exceeds the size limit".into());
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|error| error.to_string())?
        .take(config.files.max_file_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > config.files.max_file_bytes {
        return Err("Supporting file grew beyond the size limit".into());
    }
    let after = std::fs::metadata(path).map_err(|error| error.to_string())?;
    if after.len() != before.len() || after.modified().ok() != before.modified().ok() {
        return Err("Supporting file changed while it was read".into());
    }
    let text = String::from_utf8(bytes).map_err(|_| "Supporting file is not UTF-8")?;
    Ok(Source::new(
        path.to_path_buf(),
        path.to_string_lossy().into_owned(),
        text,
    ))
}

fn navigation_nodes(
    value: &Value,
    target_key: &str,
    children_key: &str,
    targets: &mut Vec<String>,
    source: &Source,
    diagnostics: &mut Vec<Diagnostic>,
    depth: usize,
) {
    if depth > 64 {
        diagnostics.push(source.diagnostic(
            "navigation.structure",
            Requirement::Execution,
            Outcome::Unverified,
            0,
            "Navigation nesting limit reached",
        ));
        return;
    }
    match value {
        Value::String(target) => targets.push(target.clone()),
        Value::Array(array) => {
            for value in array {
                navigation_nodes(
                    value,
                    target_key,
                    children_key,
                    targets,
                    source,
                    diagnostics,
                    depth + 1,
                );
            }
        }
        Value::Object(object) => {
            if let Some(target) = object.get(target_key) {
                if let Some(target) = target.as_str() {
                    targets.push(target.into());
                } else {
                    diagnostics.push(source.diagnostic(
                        "navigation.structure",
                        Requirement::Project,
                        Outcome::Invalid,
                        0,
                        "Navigation target must be a string",
                    ));
                }
            }
            if let Some(children) = object.get(children_key) {
                if !children.is_array() {
                    diagnostics.push(source.diagnostic(
                        "navigation.structure",
                        Requirement::Project,
                        Outcome::Invalid,
                        0,
                        "Navigation children must be an array",
                    ));
                } else {
                    navigation_nodes(
                        children,
                        target_key,
                        children_key,
                        targets,
                        source,
                        diagnostics,
                        depth + 1,
                    );
                }
            }
            if !object.contains_key(target_key) && !object.contains_key(children_key) {
                diagnostics.push(source.diagnostic(
                    "navigation.structure",
                    Requirement::Project,
                    Outcome::Invalid,
                    0,
                    "Navigation node has no target or children",
                ));
            }
        }
        _ => diagnostics.push(source.diagnostic(
            "navigation.structure",
            Requirement::Project,
            Outcome::Invalid,
            0,
            "Invalid navigation node",
        )),
    }
}

fn graph_problems(
    count: usize,
    edges: &[(usize, usize)],
    max_depth: usize,
) -> Vec<(usize, String)> {
    let mut outgoing = vec![Vec::new(); count];
    for &(a, b) in edges {
        outgoing[a].push(b);
    }
    let mut problems = BTreeSet::new();
    for start in 0..count {
        let mut stack = vec![(start, 0usize, false)];
        let mut active = BTreeSet::new();
        let mut best_depth = BTreeMap::new();
        while let Some((node, depth, leave)) = stack.pop() {
            if leave {
                active.remove(&node);
                continue;
            }
            if active.contains(&node) {
                problems.insert((node, "Structural recursion cycle".into()));
                continue;
            }
            if depth > max_depth {
                problems.insert((start, "Structural recursion exceeds the depth limit".into()));
                break;
            }
            if best_depth
                .get(&node)
                .is_some_and(|previous| *previous >= depth)
            {
                continue;
            }
            best_depth.insert(node, depth);
            active.insert(node);
            stack.push((node, depth, true));
            for child in &outgoing[node] {
                stack.push((*child, depth + 1, false));
            }
        }
    }
    problems.into_iter().collect()
}

fn check_output(
    documents: &[Document],
    root: &Path,
    config: &Config,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let output_root = config.resolve(
        config
            .routes
            .output_dir
            .as_deref()
            .expect("validated output directory"),
    );
    let output_root = std::fs::canonicalize(&output_root).unwrap_or(output_root);
    let mut output_config = config.clone();
    output_config.include = vec!["**/*.html".into(), "**/*.htm".into()];
    output_config.exclude.clear();
    output_config.extensions = BTreeMap::from([
        ("html".into(), "html".into()),
        ("htm".into(), "html".into()),
    ]);
    output_config.required_paths.clear();
    output_config.exceptions.clear();
    output_config.rules.unused_exceptions = false;
    output_config.profiles.clear();
    output_config.frontmatter = crate::config::FrontmatterConfig::default();
    output_config.structured.clear();
    output_config.collection.navigation.clear();
    output_config.collection.require_navigation_coverage = false;
    output_config.collection.require_reachable = false;
    output_config.collection.entrypoints.clear();
    output_config.routes.enabled = false;
    output_config.routes.require_output = false;
    output_config.routes.redirects.clear();
    output_config.routes.preserved_anchors.clear();
    output_config.assets.sitemap.clear();
    output_config.network.enabled = false;
    output_config.files.name_pattern = None;
    output_config.files.path_pattern = None;
    output_config.html.mode = crate::config::HtmlMode::Document;
    output_config.links.root = Some(output_root.to_string_lossy().into_owned());
    output_config.links.allow_outside_root = false;
    let base = normalize_route(&config.routes.base);
    let mut output_bytes = 0u64;
    for document in documents {
        let Some(route) = &document.route else {
            diagnostics.push(document.source.diagnostic(
                "build.output",
                Requirement::Project,
                Outcome::Unverified,
                0,
                "Output validation requires a publication route",
            ));
            continue;
        };
        let relative = if route == &base {
            ""
        } else {
            route
                .strip_prefix(&format!("{}/", base.trim_end_matches('/')))
                .unwrap_or(route)
                .trim_start_matches('/')
        };
        let output = if relative.is_empty() {
            output_root.join("index.html")
        } else if config.routes.directory_urls && !config.routes.html_extension {
            output_root.join(relative).join("index.html")
        } else {
            let path = output_root.join(relative);
            if path.extension().is_some_and(|ext| {
                ext.eq_ignore_ascii_case("html") || ext.eq_ignore_ascii_case("htm")
            }) {
                path
            } else {
                {
                    let mut filename = path.into_os_string();
                    filename.push(".html");
                    PathBuf::from(filename)
                }
            }
        };
        if let Ok(canonical) = std::fs::canonicalize(&output)
            && !canonical.starts_with(&output_root)
        {
            diagnostics.push(document.source.diagnostic(
                "build.output",
                Requirement::Project,
                Outcome::Invalid,
                0,
                "Required generated file leaves the output root through a symbolic link",
            ));
            continue;
        }
        if let Ok(metadata) = std::fs::metadata(&output) {
            output_bytes = output_bytes.saturating_add(metadata.len());
            if output_bytes > config.files.max_total_bytes {
                diagnostics.push(document.source.diagnostic(
                    "build.output",
                    Requirement::Execution,
                    Outcome::Unverified,
                    0,
                    "Generated anchor checks exceed the total byte limit",
                ));
                break;
            }
        }
        let source = match read_supporting_source(&output, config) {
            Ok(source) => source,
            Err(error) => {
                let outcome = if output.exists() {
                    Outcome::Unverified
                } else {
                    Outcome::Invalid
                };
                diagnostics.push(document.source.diagnostic(
                    "build.output",
                    Requirement::Project,
                    outcome,
                    0,
                    format!("Required generated HTML file is unavailable: {error}"),
                ));
                continue;
            }
        };
        let parsed = crate::html_reader::parse(&source, &output_config);
        if parsed.diagnostics.iter().any(|diagnostic| {
            matches!(
                diagnostic.outcome,
                Outcome::Unverified | Outcome::Unsupported
            )
        }) {
            diagnostics.push(document.source.diagnostic(
                "build.output",
                Requirement::Execution,
                Outcome::Unverified,
                0,
                "Generated anchor checks could not complete for this page",
            ));
            continue;
        }
        for anchor in &document.parsed.anchors {
            if !parsed
                .anchors
                .iter()
                .any(|candidate| candidate.id == anchor.id)
            {
                diagnostics.push(document.source.diagnostic(
                    "build.output",
                    Requirement::Project,
                    Outcome::Invalid,
                    anchor.offset,
                    format!("Generated output lost anchor: {}", anchor.id),
                ));
            }
        }
    }
    match crate::scan::validate(std::slice::from_ref(&output_root), &output_config) {
        Ok(mut report) => {
            let map_location = |location: &mut Location| {
                let path = Path::new(&location.path);
                let path = if path.is_absolute() {
                    path.to_path_buf()
                } else {
                    output_root.join(path)
                };
                location.path = path
                    .strip_prefix(root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/");
            };
            for diagnostic in &mut report.diagnostics {
                map_location(&mut diagnostic.location);
                for location in &mut diagnostic.related {
                    map_location(location);
                }
            }
            diagnostics.extend(report.diagnostics);
        }
        Err(error) => diagnostics.push(Diagnostic::at(
            "build.output",
            Requirement::Execution,
            Outcome::Unverified,
            Location {
                path: config.routes.output_dir.clone().unwrap_or_default(),
                line: 1,
                column: 1,
                byte_offset: 0,
            },
            format!("Generated collection could not be checked: {error}"),
        )),
    }
}
