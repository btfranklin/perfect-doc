use crate::{collection::read_supporting_source, config::Config, links, metadata, model::*};
use base64::{Engine, engine::general_purpose::STANDARD};
use image::ImageDecoder;
use sha2::{Digest, Sha256, Sha384, Sha512};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    io::{Cursor, Read},
    path::{Path, PathBuf},
    sync::Arc,
};

pub const RULES: &[RuleDefinition] = &[
    RuleDefinition {
        id: "asset.syntax",
        family: 19,
        requirement: Requirement::Format,
        description: "Selected images, SVG, CSS, and data URIs must have valid structure.",
    },
    RuleDefinition {
        id: "asset.type",
        family: 19,
        requirement: Requirement::Profile,
        description: "Asset format must match its declared use.",
    },
    RuleDefinition {
        id: "asset.integrity",
        family: 19,
        requirement: Requirement::Format,
        description: "Local asset bytes must match declared integrity hashes.",
    },
    RuleDefinition {
        id: "asset.dimensions",
        family: 19,
        requirement: Requirement::Project,
        description: "Selected image dimensions must match the asset.",
    },
    RuleDefinition {
        id: "asset.available",
        family: 19,
        requirement: Requirement::Execution,
        description: "Assets must be readable within resource limits.",
    },
    RuleDefinition {
        id: "asset.orphan",
        family: 13,
        requirement: Requirement::Project,
        description: "Selected asset files must have an incoming reference.",
    },
    RuleDefinition {
        id: "sitemap.structure",
        family: 20,
        requirement: Requirement::Project,
        description: "Supplied sitemaps must have valid XML and known local routes.",
    },
];

pub fn validate_data(
    value: &str,
    kind: ReferenceKind,
    config: &Config,
) -> Result<(), (Outcome, String)> {
    let invalid = |message: &str| (Outcome::Invalid, message.to_owned());
    let (header, data) = value
        .get(..5)
        .filter(|prefix| prefix.eq_ignore_ascii_case("data:"))
        .and_then(|_| value[5..].split_once(','))
        .ok_or_else(|| invalid("Data URI needs a comma"))?;
    // Base64 needs four bytes per three input bytes. Each byte can be escaped.
    let encoded_limit = config.files.max_file_bytes.div_ceil(3).saturating_mul(12);
    if data.len() as u64 > encoded_limit {
        return Err((
            Outcome::Unverified,
            "Data URI exceeds the encoded byte limit".into(),
        ));
    }
    let raw = value.as_bytes();
    for (offset, byte) in raw.iter().enumerate() {
        if *byte == b'%'
            && (raw.get(offset + 1).is_none_or(|b| !b.is_ascii_hexdigit())
                || raw.get(offset + 2).is_none_or(|b| !b.is_ascii_hexdigit()))
        {
            return Err(invalid("Invalid percent escape in data URI"));
        }
    }
    let bytes = if header
        .split(';')
        .any(|value| value.eq_ignore_ascii_case("base64"))
    {
        STANDARD
            .decode(percent_encoding::percent_decode_str(data).collect::<Vec<u8>>())
            .map_err(|_| invalid("Invalid base64 data URI"))?
    } else {
        percent_encoding::percent_decode_str(data).collect::<Vec<u8>>()
    };
    if bytes.len() as u64 > config.files.max_file_bytes {
        return Err((
            Outcome::Unverified,
            "Decoded data URI exceeds the byte limit".into(),
        ));
    }
    let mime = header.split(';').next().unwrap_or("").to_ascii_lowercase();
    if kind == ReferenceKind::Image && !mime.starts_with("image/") {
        return Err(invalid("Image data URI needs an image media type"));
    }
    if mime == "image/svg+xml" {
        if config.assets.validate_svg {
            check_svg(&String::from_utf8(bytes).map_err(|_| invalid("SVG data is not UTF-8"))?)
                .map_err(|message| (Outcome::Invalid, message))?;
        }
    } else if mime.starts_with("image/") && config.assets.validate_images {
        image_structure(&bytes, config)?;
    } else if mime == "application/json" {
        metadata::parse_data(
            &String::from_utf8(bytes).map_err(|_| invalid("JSON data is not UTF-8"))?,
            "json",
        )
        .map_err(|message| {
            let outcome = if message.starts_with("recursion limit exceeded") {
                Outcome::Unverified
            } else {
                Outcome::Invalid
            };
            (outcome, message)
        })?;
    }
    Ok(())
}

#[derive(Clone, Copy)]
pub struct AssetContext<'a> {
    pub root: &'a Path,
    pub config: &'a Config,
    pub inventory: &'a BTreeSet<PathBuf>,
    pub routes: &'a BTreeMap<String, PathBuf>,
    pub incoming: &'a BTreeSet<PathBuf>,
}

pub fn check(
    documents: &[Document],
    assets: &[(usize, Reference, PathBuf)],
    context: &AssetContext<'_>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<ExternalLink> {
    let AssetContext {
        root,
        config,
        inventory,
        routes,
        incoming,
    } = *context;
    let owners: Vec<_> = documents
        .iter()
        .map(|d| Arc::new(d.source.clone()))
        .collect();
    let mut total_bytes = 0u64;
    let mut queue: VecDeque<_> = assets
        .iter()
        .map(|(index, reference, path)| (owners[*index].clone(), reference.clone(), path.clone()))
        .collect();
    let mut seen = BTreeSet::new();
    let mut external = Vec::new();
    let mut referenced: BTreeSet<_> = incoming.clone();
    referenced.extend(assets.iter().map(|(_, _, p)| p.clone()));
    let mut processed = 0;
    while let Some((owner, reference, path)) = queue.pop_front() {
        let at = |rule, outcome, message| {
            owner
                .diagnostic(
                    rule,
                    if rule == "asset.available" {
                        Requirement::Execution
                    } else if rule == "asset.dimensions" {
                        Requirement::Project
                    } else {
                        Requirement::Format
                    },
                    outcome,
                    reference.offset,
                    message,
                )
                .with_target(&reference.target)
        };
        let read_limit = config
            .files
            .max_file_bytes
            .min(config.files.max_total_bytes.saturating_sub(total_bytes));
        let bytes = match std::fs::metadata(&path) {
            Ok(info) if info.len() <= read_limit => {
                match std::fs::File::open(&path).and_then(|f| {
                    let mut b = Vec::new();
                    f.take(read_limit.saturating_add(1)).read_to_end(&mut b)?;
                    Ok(b)
                }) {
                    Ok(b) => b,
                    Err(_) => {
                        diagnostics.push(at(
                            "asset.available",
                            Outcome::Unverified,
                            "Cannot read the asset".to_owned(),
                        ));
                        continue;
                    }
                }
            }
            _ => {
                diagnostics.push(at(
                    "asset.available",
                    Outcome::Unverified,
                    "Asset cannot be read within the file size limit".into(),
                ));
                continue;
            }
        };
        total_bytes = total_bytes.saturating_add(bytes.len() as u64);
        if bytes.len() as u64 > config.files.max_file_bytes
            || total_bytes > config.files.max_total_bytes
        {
            diagnostics.push(at(
                "asset.available",
                Outcome::Unverified,
                "Asset read budget reached".into(),
            ));
            break;
        }
        let extension = path
            .extension()
            .and_then(|v| v.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if config.assets.integrity
            && let Some(integrity) = &reference.integrity
            && !integrity_matches(&bytes, integrity)
        {
            diagnostics.push(at(
                "asset.integrity",
                Outcome::Invalid,
                "Asset does not match its integrity metadata".into(),
            ));
        }
        if let Some(expected) = &reference.expected_tag
            && reference.kind != ReferenceKind::Id
            && !type_matches(&extension, expected)
        {
            diagnostics.push(at(
                "asset.type",
                Outcome::Invalid,
                format!("Asset extension does not match its use as {expected}"),
            ));
        }
        if (reference.kind == ReferenceKind::Image || type_matches(&extension, "image"))
            && extension != "svg"
            && config.assets.validate_images
        {
            match image_structure(&bytes, config) {
                Ok((width, height)) => {
                    if config.assets.image_dimensions
                        && (reference.width.is_some_and(|v| v != width)
                            || reference.height.is_some_and(|v| v != height))
                    {
                        diagnostics.push(at(
                            "asset.dimensions",
                            Outcome::Invalid,
                            format!("Declared dimensions do not match {width} by {height} pixels"),
                        ));
                    }
                }
                Err((outcome, error)) => diagnostics.push(at("asset.syntax", outcome, error)),
            }
        }
        if !seen.insert(path.clone()) {
            continue;
        }
        processed += 1;
        if processed > config.files.max_files {
            diagnostics.push(at(
                "asset.available",
                Outcome::Unverified,
                "Asset dependency limit reached".into(),
            ));
            break;
        }
        let mut supporting = None;
        if extension == "css" && config.assets.validate_css {
            match String::from_utf8(bytes) {
                Ok(text) => {
                    let source = Source::new(path.clone(), display(&path, root), text);
                    let (references, errors) = css_references(&source.text);
                    for (offset, outcome, error) in errors {
                        diagnostics.push(source.diagnostic(
                            "asset.syntax",
                            Requirement::Format,
                            outcome,
                            offset,
                            error,
                        ));
                    }
                    supporting = Some(Document {
                        source,
                        format: Format::Html,
                        parsed: ParsedDocument {
                            references,
                            ..ParsedDocument::default()
                        },
                        metadata: None,
                        route: None,
                    });
                }
                Err(_) => diagnostics.push(at(
                    "asset.syntax",
                    Outcome::Invalid,
                    "CSS source must be UTF-8".into(),
                )),
            }
        } else if extension == "svg" && config.assets.validate_svg {
            match String::from_utf8(bytes) {
                Ok(text) => {
                    let source = Source::new(path.clone(), display(&path, root), text);
                    if let Err(error) = check_svg(&source.text) {
                        diagnostics.push(source.diagnostic(
                            "asset.syntax",
                            Requirement::Format,
                            Outcome::Invalid,
                            0,
                            error,
                        ));
                    } else if let Ok(xml) = roxmltree::Document::parse(&source.text) {
                        let mut references = Vec::new();
                        let anchors = xml
                            .descendants()
                            .filter_map(|node| {
                                node.attribute("id").map(|id| Anchor {
                                    id: id.into(),
                                    offset: node.range().start,
                                    tag: node.tag_name().name().into(),
                                })
                            })
                            .collect();
                        for node in xml.descendants().filter(|n| n.is_element()) {
                            for attribute in node.attributes() {
                                if matches!(attribute.name(), "href" | "src") {
                                    references.push(Reference::new(
                                        attribute.value(),
                                        node.range().start,
                                        ReferenceKind::Asset,
                                    ));
                                }
                            }
                        }
                        supporting = Some(Document {
                            source: source.clone(),
                            format: Format::Html,
                            parsed: ParsedDocument {
                                references,
                                anchors,
                                ..ParsedDocument::default()
                            },
                            metadata: None,
                            route: None,
                        });
                    }
                }
                Err(_) => diagnostics.push(at(
                    "asset.syntax",
                    Outcome::Invalid,
                    "SVG source must be UTF-8".into(),
                )),
            }
        } else if extension == "vtt" {
            if std::str::from_utf8(&bytes).map_or(true, |s| {
                !s.trim_start_matches('\u{feff}').starts_with("WEBVTT")
            }) {
                diagnostics.push(at(
                    "asset.syntax",
                    Outcome::Invalid,
                    "Caption asset has no WEBVTT signature".into(),
                ));
            }
        } else if matches!(extension.as_str(), "woff" | "woff2" | "ttf" | "otf")
            && !font_header(&bytes, &extension)
        {
            diagnostics.push(at(
                "asset.syntax",
                Outcome::Invalid,
                "Invalid font header or declared file length".into(),
            ));
        } else if extension == "wav"
            && (bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE")
        {
            diagnostics.push(at(
                "asset.syntax",
                Outcome::Invalid,
                "Invalid WAV header".into(),
            ));
        }
        if let Some(document) = supporting {
            let mut asset_config = config.clone();
            asset_config.routes.enabled = false;
            let result = links::check(
                std::slice::from_ref(&document),
                root,
                &asset_config,
                &BTreeMap::new(),
            );
            diagnostics.extend(result.diagnostics);
            external.extend(result.external);
            let owner = Arc::new(document.source.clone());
            for (_, reference, path) in result.assets {
                referenced.insert(path.clone());
                queue.push_back((owner.clone(), reference, path));
            }
        }
    }
    if config.collection.orphan_assets {
        for path in inventory {
            if path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| type_matches(&e.to_ascii_lowercase(), "any"))
                && !referenced.contains(path)
            {
                diagnostics.push(Diagnostic::at(
                    "asset.orphan",
                    Requirement::Project,
                    Outcome::Invalid,
                    Location {
                        path: display(path, root),
                        line: 1,
                        column: 1,
                        byte_offset: 0,
                    },
                    "Asset has no incoming reference",
                ));
            }
        }
    }
    for path in &config.assets.sitemap {
        match read_supporting_source(&config.resolve(path), config) {
            Ok(source) => {
                match roxmltree::Document::parse(&source.text) {
                    Ok(xml) => {
                        if !matches!(
                            xml.root_element().tag_name().name(),
                            "urlset" | "sitemapindex"
                        ) {
                            diagnostics.push(source.diagnostic(
                                "sitemap.structure",
                                Requirement::Project,
                                Outcome::Invalid,
                                0,
                                "Sitemap root must be urlset or sitemapindex",
                            ));
                        }
                        let mut seen = BTreeSet::new();
                        for node in xml.descendants().filter(|n| n.tag_name().name() == "loc") {
                            if let Some(text) = node.text() {
                                match url::Url::parse(text) {
                                    Ok(url) if matches!(url.scheme(), "http" | "https") => {
                                        if xml.root_element().tag_name().name() == "urlset"
                                            && config.routes.enabled
                                            && !routes.contains_key(
                                                &crate::collection::normalize_route(url.path()),
                                            )
                                        {
                                            diagnostics.push(source.diagnostic("sitemap.structure",Requirement::Project,Outcome::Invalid,node.range().start,"Sitemap location does not have a declared local route"));
                                        }
                                        if !seen.insert(url.to_string()) {
                                            diagnostics.push(source.diagnostic(
                                                "sitemap.structure",
                                                Requirement::Project,
                                                Outcome::Invalid,
                                                node.range().start,
                                                "Sitemap location is repeated",
                                            ));
                                        }
                                    }
                                    _ => diagnostics.push(source.diagnostic(
                                        "sitemap.structure",
                                        Requirement::Project,
                                        Outcome::Invalid,
                                        node.range().start,
                                        "Sitemap location must be an absolute HTTP URL",
                                    )),
                                }
                            }
                        }
                    }
                    Err(e) => diagnostics.push(source.diagnostic(
                        "sitemap.structure",
                        Requirement::Project,
                        Outcome::Invalid,
                        0,
                        e.to_string(),
                    )),
                }
            }
            Err(e) => diagnostics.push(Diagnostic::at(
                "sitemap.structure",
                Requirement::Project,
                Outcome::Unverified,
                Location {
                    path: path.clone(),
                    line: 1,
                    column: 1,
                    byte_offset: 0,
                },
                e,
            )),
        }
    }
    external
}

fn display(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn type_matches(extension: &str, expected: &str) -> bool {
    match expected {
        "image" => matches!(
            extension,
            "png"
                | "jpg"
                | "jpeg"
                | "gif"
                | "webp"
                | "bmp"
                | "ico"
                | "tiff"
                | "tif"
                | "svg"
                | "avif"
                | "apng"
        ),
        "stylesheet" => extension == "css",
        "script" => matches!(extension, "js" | "mjs" | "cjs"),
        "audio" => matches!(extension, "wav" | "mp3" | "ogg" | "flac" | "m4a" | "aac"),
        "video" => matches!(extension, "mp4" | "webm" | "ogv" | "mov"),
        "caption" => extension == "vtt",
        "any" => {
            ["image", "stylesheet", "script", "audio", "video", "caption"]
                .iter()
                .any(|kind| type_matches(extension, kind))
                || matches!(extension, "woff" | "woff2" | "ttf" | "otf")
        }
        _ => true,
    }
}

fn image_structure(bytes: &[u8], config: &Config) -> Result<(u32, u32), (Outcome, String)> {
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| (Outcome::Invalid, e.to_string()))?;
    if reader.format().is_none() {
        return Err((
            Outcome::Invalid,
            "Asset has no recognized image format".into(),
        ));
    }
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(config.assets.max_pixels.saturating_mul(16));
    reader.limits(limits);
    let decoder = reader.into_decoder().map_err(image_error)?;
    let (width, height) = decoder.dimensions();
    if u64::from(width) * u64::from(height) > config.assets.max_pixels {
        return Err((Outcome::Unverified, "Image exceeds the pixel limit".into()));
    }
    image::DynamicImage::from_decoder(decoder).map_err(image_error)?;
    Ok((width, height))
}

fn image_error(error: image::ImageError) -> (Outcome, String) {
    match error {
        image::ImageError::Unsupported(_) => (
            Outcome::Unsupported,
            "Image format is not supported by this build".into(),
        ),
        image::ImageError::Limits(_) => (
            Outcome::Unverified,
            "Image decoder resource limit reached".into(),
        ),
        image::ImageError::IoError(error) if error.kind() == std::io::ErrorKind::OutOfMemory => (
            Outcome::Unverified,
            "Image decoder memory is not available".into(),
        ),
        error => (Outcome::Invalid, format!("Invalid image data: {error}")),
    }
}

fn check_svg(text: &str) -> Result<(), String> {
    let xml = roxmltree::Document::parse(text).map_err(|e| e.to_string())?;
    if xml.root_element().tag_name().name() != "svg" {
        return Err("SVG asset must have an svg root".into());
    }
    let mut ids = BTreeSet::new();
    for node in xml.descendants() {
        if let Some(id) = node.attribute("id")
            && (id.is_empty() || !ids.insert(id))
        {
            return Err("SVG ID is empty or repeated".into());
        }
    }
    Ok(())
}

fn font_header(bytes: &[u8], extension: &str) -> bool {
    if bytes.len() < 12 {
        return false;
    }
    match extension {
        "woff" | "woff2" => {
            bytes.starts_with(if extension == "woff" {
                b"wOFF"
            } else {
                b"wOF2"
            }) && u32::from_be_bytes(bytes[8..12].try_into().expect("four bytes")) as usize
                == bytes.len()
        }
        "ttf" => bytes.starts_with(&[0, 1, 0, 0]) || bytes.starts_with(b"true"),
        "otf" => bytes.starts_with(b"OTTO"),
        _ => true,
    }
}

fn integrity_matches(bytes: &[u8], integrity: &str) -> bool {
    let mut strongest = 0;
    let mut matches = false;
    for token in integrity.split_whitespace() {
        let Some((algorithm, encoded)) = token.split_once('-') else {
            continue;
        };
        let (strength, digest) = match algorithm {
            "sha256" => (1, Sha256::digest(bytes).to_vec()),
            "sha384" => (2, Sha384::digest(bytes).to_vec()),
            "sha512" => (3, Sha512::digest(bytes).to_vec()),
            _ => continue,
        };
        if strength > strongest {
            strongest = strength;
            matches = false;
        }
        if strength == strongest
            && STANDARD
                .decode(encoded.split('?').next().unwrap_or(encoded))
                .is_ok_and(|expected| expected == digest)
        {
            matches = true;
        }
    }
    strongest > 0 && matches
}

pub fn css_references(text: &str) -> (Vec<Reference>, Vec<(usize, Outcome, String)>) {
    fn walk(
        parser: &mut cssparser::Parser<'_>,
        references: &mut Vec<Reference>,
        errors: &mut Vec<(usize, Outcome, String)>,
        depth: usize,
    ) -> bool {
        if depth > 128 {
            errors.push((
                parser.position().byte_index(),
                Outcome::Unverified,
                "CSS nesting limit reached".into(),
            ));
            return false;
        }
        let mut import = false;
        while !parser.is_exhausted() {
            let offset = parser.position().byte_index();
            let Ok(token) = parser.next().cloned() else {
                break;
            };
            use cssparser::Token;
            match token {
                Token::UnquotedUrl(value) => references.push(Reference::new(
                    value.to_string(),
                    offset,
                    ReferenceKind::Asset,
                )),
                Token::QuotedString(value) if import => {
                    references.push(Reference::new(
                        value.to_string(),
                        offset,
                        ReferenceKind::Asset,
                    ));
                    import = false;
                }
                Token::AtKeyword(value) => import = value.eq_ignore_ascii_case("import"),
                Token::Function(name) if name.eq_ignore_ascii_case("url") => {
                    let result: Result<String, cssparser::ParseError<()>> = parser
                        .parse_nested_block(|input| {
                            let value = input.expect_string()?.to_string();
                            input.expect_exhausted()?;
                            Ok(value)
                        });
                    match result {
                        Ok(value) => {
                            references.push(Reference::new(value, offset, ReferenceKind::Asset))
                        }
                        Err(error)
                            if matches!(
                                error.kind,
                                cssparser::ParseErrorKind::Basic(
                                    cssparser::BasicParseErrorKind::TooManyNestedBlocks
                                )
                            ) =>
                        {
                            errors.push((
                                offset,
                                Outcome::Unverified,
                                "CSS nesting limit reached".into(),
                            ));
                            return false;
                        }
                        Err(_) => errors.push((
                            offset,
                            Outcome::Invalid,
                            "Invalid CSS URL function".into(),
                        )),
                    }
                }
                Token::Function(_)
                | Token::ParenthesisBlock
                | Token::SquareBracketBlock
                | Token::CurlyBracketBlock => {
                    let mut complete = true;
                    let result: Result<(), cssparser::ParseError<()>> =
                        parser.parse_nested_block(|inner| {
                            complete = walk(inner, references, errors, depth + 1);
                            Ok(())
                        });
                    if !complete {
                        return false;
                    }
                    if let Err(error) = result {
                        if matches!(
                            error.kind,
                            cssparser::ParseErrorKind::Basic(
                                cssparser::BasicParseErrorKind::TooManyNestedBlocks
                            )
                        ) {
                            errors.push((
                                offset,
                                Outcome::Unverified,
                                "CSS nesting limit reached".into(),
                            ));
                            return false;
                        }
                        errors.push((offset, Outcome::Invalid, "Invalid CSS block".into()));
                    }
                }
                Token::BadUrl(_)
                | Token::BadString(_)
                | Token::CloseParenthesis
                | Token::CloseSquareBracket
                | Token::CloseCurlyBracket => {
                    errors.push((offset, Outcome::Invalid, "Invalid CSS token".into()))
                }
                Token::Semicolon => import = false,
                _ => {}
            }
        }
        true
    }
    let mut references = Vec::new();
    let mut errors = Vec::new();
    walk(
        &mut cssparser::Parser::new(text),
        &mut references,
        &mut errors,
        0,
    );
    (references, errors)
}
