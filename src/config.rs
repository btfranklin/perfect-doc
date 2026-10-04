use crate::model::{Format, Severity};
use globset::Glob;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub extensions: BTreeMap<String, String>,
    pub required_paths: Vec<String>,
    pub files: FilesConfig,
    pub markdown: MarkdownConfig,
    pub html: HtmlConfig,
    pub frontmatter: FrontmatterConfig,
    pub links: LinksConfig,
    pub routes: RoutesConfig,
    pub collection: CollectionConfig,
    pub assets: AssetsConfig,
    pub network: NetworkConfig,
    pub rules: RulesConfig,
    pub exceptions: Vec<Exception>,
    pub structured: Vec<StructuredFile>,
    pub profiles: Vec<Profile>,
    /// Directory that owns relative configuration paths. It is not an input key.
    #[serde(skip)]
    pub base_dir: PathBuf,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            include: vec!["**/*".into()],
            exclude: vec![
                "**/.git/**".into(),
                "**/node_modules/**".into(),
                "**/target/**".into(),
                "**/.venv/**".into(),
                "**/.pytest_cache/**".into(),
                "**/dist/**".into(),
            ],
            extensions: BTreeMap::from([
                ("md".into(), "markdown".into()),
                ("markdown".into(), "markdown".into()),
                ("okf".into(), "markdown".into()),
                ("html".into(), "html".into()),
                ("htm".into(), "html".into()),
                ("mdx".into(), "mdx".into()),
            ]),
            required_paths: Vec::new(),
            files: FilesConfig::default(),
            markdown: MarkdownConfig::default(),
            html: HtmlConfig::default(),
            frontmatter: FrontmatterConfig::default(),
            links: LinksConfig::default(),
            routes: RoutesConfig::default(),
            collection: CollectionConfig::default(),
            assets: AssetsConfig::default(),
            network: NetworkConfig::default(),
            rules: RulesConfig::default(),
            exceptions: Vec::new(),
            structured: Vec::new(),
            profiles: Vec::new(),
            base_dir: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        }
    }
}

macro_rules! settings {
    ($name:ident { $($field:ident: $type:ty = $value:expr),* $(,)? }) => {
        #[derive(Debug, Clone, Serialize, Deserialize)]
        #[serde(default, deny_unknown_fields)]
        pub struct $name { $(pub $field: $type),* }
        impl Default for $name { fn default() -> Self { Self { $($field: $value),* } } }
    };
}

settings!(FilesConfig {
    max_file_bytes: u64 = 16 * 1024 * 1024,
    max_total_bytes: u64 = 256 * 1024 * 1024,
    max_files: usize = 100_000,
    max_depth: usize = 64,
    follow_symlinks: bool = false,
    allow_empty: bool = false,
    portable_names: bool = false,
    name_pattern: Option<String> = None,
    path_pattern: Option<String> = None,
    max_path_bytes: Option<usize> = None,
    detect_case_collisions: bool = true,
    detect_unicode_collisions: bool = true,
    hidden_characters: bool = false,
    merge_conflicts: bool = true,
    html_encoding: String = "utf-8".into(),
});

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MarkdownDialect {
    Commonmark,
    Gfm,
    Mdx,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AnchorStyle {
    Github,
    PythonMarkdown,
    Kramdown,
    ExplicitOnly,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HtmlMode {
    Auto,
    Document,
    Fragment,
}

settings!(MarkdownConfig {
    dialect: MarkdownDialect = MarkdownDialect::Gfm,
    anchors: AnchorStyle = AnchorStyle::Github,
    explicit_heading_ids: bool = false,
    heading_start: Option<u8> = None,
    single_h1: bool = false,
    no_heading_skips: bool = false,
    nonempty_headings: bool = false,
    nonempty_sections: bool = false,
    closed_fences: bool = false,
    table_columns: bool = false,
    undefined_references: bool = false,
    unused_definitions: bool = false,
    wiki_links: bool = false,
    includes: bool = false,
    directives: bool = false,
    allowed_languages: Vec<String> = Vec::new(),
    require_language: bool = false,
    structured_examples: bool = false,
    example_schemas: BTreeMap<String,String> = BTreeMap::new(),
    bibliography: Vec<String> = Vec::new(),
    toc: bool = false,
});

settings!(HtmlConfig {
    mode: HtmlMode = HtmlMode::Auto,
    conformance: bool = true,
    accessibility: bool = true,
    require_lang: bool = false,
    require_main: bool = false,
    allow_custom_elements: bool = true,
    allowed_elements: Vec<String> = Vec::new(),
    obsolete_elements: bool = false,
});

settings!(FrontmatterConfig {
    required: bool = false,
    format: String = "auto".into(),
    schema: Option<String> = None,
    inline_schema: Option<serde_json::Value> = None,
    required_fields: Vec<String> = Vec::new(),
    allowed_fields: Vec<String> = Vec::new(),
    forbidden_fields: Vec<String> = Vec::new(),
    unique_fields: Vec<String> = Vec::new(),
    link_fields: Vec<String> = Vec::new(),
    id_field: Option<String> = None,
    route_field: Option<String> = None,
    aliases_field: Option<String> = None,
    parent_field: Option<String> = None,
    language_field: Option<String> = None,
    date_order: Vec<DateOrder> = Vec::new(),
    name_field: Option<String> = None,
});
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DateOrder {
    pub before: String,
    pub after: String,
}

settings!(LinksConfig {
    root: Option<String> = None,
    index_files: Vec<String> = vec!["index.md".into(),"README.md".into(),"index.html".into(),"index.okf".into()],
    allowed_schemes: Vec<String> = vec!["http".into(),"https".into(),"mailto".into(),"tel".into(),"data".into(),"ftp".into()],
    allow_outside_root: bool = false,
    strict_case: bool = true,
    template_references: bool = true,
});

settings!(RoutesConfig {
    enabled: bool = false,
    base: String = "/".into(),
    directory_urls: bool = true,
    html_extension: bool = false,
    redirects: BTreeMap<String,String> = BTreeMap::new(),
    preserved_anchors: BTreeMap<String,Vec<String>> = BTreeMap::new(),
    output_dir: Option<String> = None,
    require_output: bool = false,
});

settings!(CollectionConfig {
    entrypoints: Vec<String> = Vec::new(),
    require_reachable: bool = false,
    navigation: Vec<NavigationFile> = Vec::new(),
    require_navigation_coverage: bool = false,
    include_max_depth: usize = 32,
    orphan_assets: bool = false,
});
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NavigationFile {
    pub path: String,
    pub target_key: String,
    pub children_key: String,
}
impl Default for NavigationFile {
    fn default() -> Self {
        Self {
            path: String::new(),
            target_key: "path".into(),
            children_key: "children".into(),
        }
    }
}

settings!(AssetsConfig {
    validate_images: bool = true,
    validate_svg: bool = true,
    validate_css: bool = true,
    integrity: bool = true,
    image_dimensions: bool = false,
    max_pixels: u64 = 100_000_000,
    sitemap: Vec<String> = Vec::new(),
});

settings!(NetworkConfig {
    enabled: bool = false,
    required: bool = true,
    timeout_seconds: u64 = 15,
    concurrency: usize = 4,
    max_requests: usize = 2000,
    max_redirects: usize = 8,
    retries: usize = 1,
    host_delay_ms: u64 = 100,
    max_body_bytes: u64 = 4 * 1024 * 1024,
    check_fragments: bool = false,
    allowed_hosts: Vec<String> = Vec::new(),
    excluded_hosts: Vec<String> = Vec::new(),
    cache: Option<String> = None,
    cache_ttl_seconds: u64 = 3600,
    auth_env: BTreeMap<String,String> = BTreeMap::new(),
    user_agent: String = format!("PerfectDoc/{}", env!("CARGO_PKG_VERSION")),
});

settings!(RulesConfig {
    disable: Vec<String> = Vec::new(),
    severity: BTreeMap<String,Severity> = BTreeMap::new(),
    fail_on: Severity = Severity::Error,
    unused_exceptions: bool = false,
    inline_suppressions: bool = true,
});
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Exception {
    pub rule: String,
    pub path: String,
    pub reason: String,
    pub expires: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructuredFile {
    pub glob: String,
    pub schema: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub glob: String,
    pub dialect: Option<MarkdownDialect>,
    pub anchors: Option<AnchorStyle>,
    pub html_mode: Option<HtmlMode>,
}

impl Config {
    pub fn from_file(path: &Path) -> Result<Self, String> {
        let text =
            std::fs::read_to_string(path).map_err(|e| format!("Cannot read configuration: {e}"))?;
        let mut config: Self =
            toml::from_str(&text).map_err(|e| format!("Invalid configuration: {e}"))?;
        config.base_dir = std::fs::canonicalize(
            path.parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new(".")),
        )
        .map_err(|e| e.to_string())?;
        config.validate()?;
        Ok(config)
    }
    pub fn resolve(&self, path: &str) -> PathBuf {
        self.base_dir.join(path)
    }
    pub fn format_for(&self, path: &Path) -> Option<Format> {
        let extension = path.extension()?.to_str()?.to_ascii_lowercase();
        self.extensions.get(&extension).map(|format| {
            if format == "html" {
                Format::Html
            } else {
                Format::Markdown
            }
        })
    }
    pub fn profile_for(&self, path: &str) -> Self {
        let mut config = self.clone();
        if Path::new(path).extension().is_some_and(|e| {
            self.extensions
                .get(&e.to_string_lossy().to_ascii_lowercase())
                .is_some_and(|f| f == "mdx")
        }) {
            config.markdown.dialect = MarkdownDialect::Mdx;
        }
        for profile in &self.profiles {
            if Glob::new(&profile.glob)
                .expect("validated glob")
                .compile_matcher()
                .is_match(path)
            {
                if let Some(value) = profile.dialect {
                    config.markdown.dialect = value;
                }
                if let Some(value) = profile.anchors {
                    config.markdown.anchors = value;
                }
                if let Some(value) = profile.html_mode {
                    config.html.mode = value;
                }
            }
        }
        config
    }
    pub fn validate(&self) -> Result<(), String> {
        let rules = crate::rules();
        let known: std::collections::BTreeSet<_> = rules.iter().map(|r| r.id).collect();
        for id in self
            .rules
            .disable
            .iter()
            .chain(self.rules.severity.keys())
            .chain(self.exceptions.iter().map(|e| &e.rule))
        {
            if !known.contains(id.as_str()) {
                return Err(format!("Unknown rule: {id}"));
            }
        }
        for pattern in self
            .include
            .iter()
            .chain(&self.exclude)
            .chain(&self.required_paths)
            .chain(self.profiles.iter().map(|p| &p.glob))
            .chain(self.structured.iter().map(|p| &p.glob))
            .chain(self.exceptions.iter().map(|p| &p.path))
        {
            Glob::new(pattern).map_err(|e| format!("Invalid path pattern {pattern:?}: {e}"))?;
        }
        for pattern in self
            .files
            .name_pattern
            .iter()
            .chain(&self.files.path_pattern)
        {
            regex::Regex::new(pattern).map_err(|e| format!("Invalid name pattern: {e}"))?;
        }
        if self.include.is_empty() {
            return Err("At least one include pattern is required".into());
        }
        if self.files.max_file_bytes == 0
            || self.files.max_total_bytes == 0
            || self.files.max_files == 0
            || self.files.max_depth == 0
            || self.collection.include_max_depth == 0
            || self.assets.max_pixels == 0
        {
            return Err("Resource limits must be greater than zero".into());
        }
        if !matches!(
            self.frontmatter.format.as_str(),
            "auto" | "yaml" | "toml" | "json" | "none"
        ) {
            return Err("Unknown front matter format".into());
        }
        if self.frontmatter.format == "none" && self.frontmatter.required {
            return Err("Required front matter cannot be disabled".into());
        }
        if self.frontmatter.schema.is_some() && self.frontmatter.inline_schema.is_some() {
            return Err("Use one front matter schema source".into());
        }
        if self
            .markdown
            .heading_start
            .is_some_and(|n| !(1..=6).contains(&n))
        {
            return Err("heading_start must be from 1 to 6".into());
        }
        for (extension, format) in &self.extensions {
            if extension.is_empty()
                || extension.contains(['.', '/', '\\'])
                || !matches!(format.as_str(), "markdown" | "html" | "mdx")
            {
                return Err(format!("Invalid extension mapping: {extension} = {format}"));
            }
        }
        if encoding_rs::Encoding::for_label(self.files.html_encoding.as_bytes()).is_none() {
            return Err("Unknown HTML encoding".into());
        }
        for item in &self.exceptions {
            if item.reason.trim().is_empty() {
                return Err("Exceptions need a reason".into());
            }
            if let Some(value) = &item.expires {
                parse_date(value)?;
            }
        }
        for navigation in &self.collection.navigation {
            if navigation.path.is_empty()
                || navigation.target_key.is_empty()
                || navigation.children_key.is_empty()
            {
                return Err("Navigation paths and keys cannot be empty".into());
            }
        }
        if self.collection.require_reachable && self.collection.entrypoints.is_empty() {
            return Err("Reachability requires entrypoints".into());
        }
        if self.collection.require_navigation_coverage && self.collection.navigation.is_empty() {
            return Err("Navigation coverage requires a manifest".into());
        }
        if self.routes.require_output && (!self.routes.enabled || self.routes.output_dir.is_none())
        {
            return Err("Output validation requires enabled routes and output_dir".into());
        }
        if !self.routes.base.starts_with('/') || self.routes.base.contains(['?', '#']) {
            return Err("Route base must be an absolute URL path".into());
        }
        if self.network.timeout_seconds == 0
            || self.network.concurrency == 0
            || self.network.concurrency > 64
            || self.network.max_requests == 0
            || self.network.max_body_bytes == 0
            || self.network.retries > 5
            || self.network.max_redirects > 64
        {
            return Err("Invalid network limits".into());
        }
        if self.network.user_agent.contains(['\r', '\n']) {
            return Err("Invalid network user agent".into());
        }
        for (host, variable) in &self.network.auth_env {
            if host.contains(['/', ':', '@']) || host.is_empty() || variable.is_empty() {
                return Err(
                    "Authentication requires a host and an environment variable name".into(),
                );
            }
        }
        Ok(())
    }
}

pub fn parse_date(value: &str) -> Result<time::Date, String> {
    time::Date::parse(
        value,
        time::macros::format_description!("[year]-[month]-[day]"),
    )
    .map_err(|_| format!("Invalid ISO date: {value}"))
}
