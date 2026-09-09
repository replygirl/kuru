//! Validate the static site's public boundary, local resources, and anchors.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use scraper::{Html, Selector};
use url::Url;

const REQUIRED: &[&str] = &["index.html", "sitemap.xml", "llms.txt", "llms-full.txt"];
const PRIVATE_DIRECTORIES: &[&str] = &[
    ".git",
    ".agents",
    ".claude",
    ".codex",
    ".opencode",
    ".kuru",
    "openspec",
];

struct Page {
    anchors: BTreeSet<String>,
    links: Vec<String>,
}

impl Page {
    fn parse(content: &str) -> Self {
        let document = Html::parse_document(content);
        let mut anchors = BTreeSet::new();
        let mut links = Vec::new();
        for element in document.select(&Selector::parse("*").expect("fixed selector")) {
            if let Some(id) = element.attr("id") {
                anchors.insert(id.to_owned());
            }
            if element.value().name() == "a"
                && let Some(name) = element.attr("name")
            {
                anchors.insert(name.to_owned());
            }
            for attribute in ["href", "src"] {
                if let Some(link) = element.attr(attribute) {
                    links.push(link.to_owned());
                }
            }
        }
        Self { anchors, links }
    }
}

fn private_artifact(path: &Path) -> bool {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    path.components().any(|part| {
        PRIVATE_DIRECTORIES
            .iter()
            .any(|private| part.as_os_str() == *private)
    }) || matches!(
        name.as_ref(),
        "AGENTS.md" | "CLAUDE.md" | "auth.json" | ".env"
    ) || name.starts_with(".env.")
        || name.starts_with("memory.sqlite3")
        || ["verification", "implementation-contract"]
            .iter()
            .any(|prefix| {
                name == *prefix
                    || name.starts_with(&format!("{prefix}."))
                    || name.starts_with(&format!("{prefix}_"))
            })
}

/// Resolve existing ancestors too, so a missing file below an escaping symlink
/// is rejected before any target content is read.
fn contained(root: &Path, path: &Path) -> bool {
    contained_at_depth(root, path, 64)
}

fn contained_at_depth(root: &Path, path: &Path, remaining_links: usize) -> bool {
    if remaining_links == 0 {
        return false;
    }
    let mut existing = path;
    loop {
        match fs::canonicalize(existing) {
            Ok(resolved) => return resolved.starts_with(root),
            Err(_) => {
                if let Ok(target) = fs::read_link(existing) {
                    let target = if target.is_absolute() {
                        target
                    } else {
                        existing.parent().unwrap_or(root).join(target)
                    };
                    return contained_at_depth(root, &target, remaining_links - 1);
                }
                match existing.parent() {
                    Some(parent) => existing = parent,
                    None => return false,
                }
            }
        }
    }
}

fn collect(root: &Path, directory: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(directory).with_context(|| format!("read {}", directory.display()))? {
        let entry = entry?;
        let path = entry.path();
        files.push(path.clone());
        // Never recurse through symlinks, including cycles within the output.
        if entry.file_type()?.is_dir() && contained(root, &path) {
            collect(root, &path, files)?;
        }
    }
    Ok(())
}

fn decode(value: &str) -> Result<String> {
    let source = value.as_bytes();
    let mut bytes = Vec::with_capacity(source.len());
    let mut index = 0;
    while index < source.len() {
        if source[index] == b'%' {
            let encoded = source
                .get(index + 1..index + 3)
                .context("invalid percent escape")?;
            let high = char::from(encoded[0])
                .to_digit(16)
                .context("invalid percent escape")?;
            let low = char::from(encoded[1])
                .to_digit(16)
                .context("invalid percent escape")?;
            bytes.push((high * 16 + low) as u8);
            index += 3;
        } else {
            bytes.push(source[index]);
            index += 1;
        }
    }
    String::from_utf8(bytes).context("invalid UTF-8 in URL")
}

fn normalize(path: &str) -> Result<String> {
    if path.contains(['\0', '\\']) {
        bail!("invalid local URL path");
    }
    let mut parts = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    Ok(format!("/{}", parts.join("/")))
}

fn target(
    root: &Path,
    base: &str,
    pages: &BTreeMap<PathBuf, Page>,
    source: &Path,
    link: &str,
    sitemap: bool,
) -> Result<Option<String>> {
    let absolute_link = Url::parse(link);
    if absolute_link
        .as_ref()
        .is_ok_and(|url| url.scheme() == "file")
    {
        return Ok(Some(format!(
            "{source}: {link:?} escapes output directory",
            source = source.display()
        )));
    }
    if !sitemap && (absolute_link.is_ok() || link.starts_with("//")) {
        return Ok(None);
    }
    let current = Url::parse(&format!(
        "https://docs.invalid{base}{}",
        source.to_string_lossy()
    ))?;
    let absolute = current.join(link)?;
    let normalized = normalize(&decode(absolute.path())?)?;
    if base != "/" && normalized != base.trim_end_matches('/') && !normalized.starts_with(base) {
        return Ok(Some(format!(
            "{}: {link:?} escapes site base {base}",
            source.display()
        )));
    }
    let relative = normalized[base.trim_end_matches('/').len()..].trim_start_matches('/');
    let path = root.join(relative);
    let mut candidates = if path == root {
        vec![root.join("index.html")]
    } else {
        vec![path.clone()]
    };
    if path != root && path.extension().is_none() {
        candidates.extend([path.with_extension("html"), path.join("index.html")]);
    }
    let mut existing = None;
    for candidate in candidates {
        if !contained(root, &candidate) {
            return Ok(Some(format!(
                "{}: {link:?} escapes output directory",
                source.display()
            )));
        }
        if candidate.is_file() {
            existing = Some(candidate.canonicalize()?);
            break;
        }
    }
    let Some(existing) = existing else {
        return Ok(Some(format!(
            "{}: missing local target {link:?}",
            source.display()
        )));
    };
    let fragment = decode(absolute.fragment().unwrap_or_default())?;
    let fragment = fragment.split(":~:text=").next().unwrap_or_default();
    if !fragment.is_empty()
        && pages
            .get(&existing)
            .is_some_and(|page| !page.anchors.contains(fragment))
    {
        return Ok(Some(format!(
            "{}: missing HTML anchor in {link:?}",
            source.display()
        )));
    }
    Ok(None)
}

pub fn check(root: &Path, base: &str) -> Result<Vec<String>> {
    if !base.starts_with('/')
        || !base.ends_with('/')
        || base.contains("//")
        || base.contains(['?', '#', '%', '\\'])
        || base.chars().any(char::is_control)
        || base.split('/').any(|part| matches!(part, "." | ".."))
    {
        return Ok(vec![
            "base must be an absolute URL path with a trailing slash".into(),
        ]);
    }
    if !root.is_dir() {
        return Ok(vec![format!(
            "docs output directory does not exist: {}",
            root.display()
        )]);
    }
    let root = root.canonicalize()?;
    let mut errors = BTreeSet::new();
    let mut pages = BTreeMap::new();
    let mut artifacts = Vec::new();
    collect(&root, &root, &mut artifacts)?;
    for path in artifacts {
        let relative = path.strip_prefix(&root)?;
        if !contained(&root, &path) {
            errors.insert(format!(
                "{}: artifact escapes output directory",
                relative.display()
            ));
            continue;
        }
        if private_artifact(relative) {
            errors.insert(format!(
                "{}: repository-only or private artifact",
                relative.display()
            ));
        }
        if path.is_file()
            && path
                .extension()
                .is_some_and(|extension| extension == "html")
        {
            match fs::read_to_string(&path) {
                Ok(content) => {
                    pages.insert(path.canonicalize()?, Page::parse(&content));
                }
                Err(error) => {
                    errors.insert(format!("{}: cannot read HTML: {error}", relative.display()));
                }
            }
        }
    }
    for name in REQUIRED {
        let path = root.join(name);
        if contained(&root, &path) && (!path.is_file() || fs::metadata(path)?.len() == 0) {
            errors.insert(format!(
                "{name}: required public docs output is missing or empty"
            ));
        }
    }
    let mut links = Vec::new();
    for (path, page) in &pages {
        for link in &page.links {
            links.push((path.strip_prefix(&root)?.to_path_buf(), link.clone(), false));
        }
    }
    let sitemap = root.join("sitemap.xml");
    if sitemap.is_file() && contained(&root, &sitemap) {
        let content = fs::read_to_string(sitemap)?;
        match roxmltree::Document::parse(&content) {
            Ok(document) => {
                let locations: Vec<_> = document
                    .descendants()
                    .filter(|node| node.is_element() && node.tag_name().name() == "loc")
                    .collect();
                if locations.is_empty() {
                    errors.insert("sitemap.xml: no page locations".into());
                }
                for location in locations {
                    links.push((
                        PathBuf::from("sitemap.xml"),
                        location.text().unwrap_or_default().into(),
                        true,
                    ));
                }
            }
            Err(error) => {
                errors.insert(format!("sitemap.xml: invalid XML: {error}"));
            }
        }
    }
    for (source, link, sitemap) in links {
        match target(&root, base, &pages, &source, &link, sitemap) {
            Ok(Some(error)) => {
                errors.insert(error);
            }
            Ok(None) => {}
            Err(error) => {
                errors.insert(format!(
                    "{}: invalid link {link:?}: {error}",
                    source.display()
                ));
            }
        }
    }
    Ok(errors.into_iter().collect())
}
