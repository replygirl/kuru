//! Repository ownership and exact dependency-pin invariants.

use std::{
    collections::BTreeSet,
    fs,
    path::{Component, Path},
};

use anyhow::{Context, Result};
use toml::Value;

fn read_toml(path: &Path) -> Result<Value> {
    let content = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    toml::from_str(&content).with_context(|| format!("parse {}", path.display()))
}

fn exact_version(version: &str) -> bool {
    let Some(version) = version.strip_prefix('=') else {
        return false;
    };
    if version
        .chars()
        .any(|character| character.is_whitespace() || "^~*<>=,|".contains(character))
    {
        return false;
    }
    let release = version.split(['-', '+']).next().unwrap_or_default();
    let parts: Vec<_> = release.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()))
}

/// A mise tool pin is a version string or a table whose `version` carries it.
fn tool_version(tool: &Value) -> Option<&str> {
    tool.as_str()
        .or_else(|| tool.get("version").and_then(Value::as_str))
}

fn owned_member(member: &str) -> bool {
    let parts: Vec<_> = Path::new(member).components().collect();
    matches!(parts.as_slice(), [Component::Normal(root), Component::Normal(_)] if *root == "apps" || *root == "packages")
        && member.split('/').count() == 2
        && !member.split('/').any(|part| part.starts_with('.'))
        && !member.contains(['*', '?', '\\'])
}

fn inherited_dependencies(manifest: &Value, member: &str, errors: &mut BTreeSet<String>) {
    for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
        if let Some(dependencies) = manifest.get(section).and_then(Value::as_table) {
            for (name, dependency) in dependencies {
                if dependency.get("workspace").and_then(Value::as_bool) != Some(true) {
                    errors.insert(format!(
                        "{member}: dependency {name} must inherit the Cargo workspace pin"
                    ));
                }
            }
        }
    }
    if let Some(targets) = manifest.get("target").and_then(Value::as_table) {
        for target in targets.values() {
            inherited_dependencies(target, member, errors);
        }
    }
}

pub fn check(root: &Path) -> Result<Vec<String>> {
    let canonical_root = root.canonicalize()?;
    let manifest = read_toml(&root.join("Cargo.toml"))?;
    let config = read_toml(&root.join("mise.toml"))?;
    let toolchain = read_toml(&root.join("rust-toolchain.toml"))?;
    let mut errors = BTreeSet::new();
    let tools = config.get("tools").and_then(Value::as_table);
    let rust = tools
        .and_then(|tools| tools.get("rust"))
        .and_then(tool_version);
    let channel = toolchain
        .get("toolchain")
        .and_then(|toolchain| toolchain.get("channel"))
        .and_then(Value::as_str);
    if rust.is_none() || rust != channel {
        errors.insert("Rust pins differ between mise.toml and rust-toolchain.toml".into());
    }
    for (name, tool) in tools.into_iter().flatten() {
        if !tool_version(tool).is_some_and(|version| exact_version(&format!("={version}"))) {
            errors.insert(format!("mise tool {name} must be exactly pinned"));
        }
    }
    if config.get("monorepo_root").and_then(Value::as_bool) != Some(true) {
        errors.insert("mise.toml must declare monorepo_root = true".into());
    }
    let roots = config
        .get("monorepo")
        .and_then(|monorepo| monorepo.get("config_roots"))
        .and_then(Value::as_array);
    for pattern in ["apps/*", "packages/*"] {
        if !roots.is_some_and(|roots| roots.iter().any(|root| root.as_str() == Some(pattern))) {
            errors.insert(format!("mise monorepo config_roots must include {pattern}"));
        }
    }
    let workspace = manifest
        .get("workspace")
        .context("Cargo.toml lacks workspace")?;
    let dependencies = workspace
        .get("dependencies")
        .and_then(Value::as_table)
        .context("Cargo workspace lacks dependencies")?;
    for (name, dependency) in dependencies {
        let version = dependency
            .as_str()
            .or_else(|| dependency.get("version").and_then(Value::as_str));
        if version.is_some_and(|version| !exact_version(version)) {
            errors.insert(format!(
                "workspace dependency {name} must be exactly pinned"
            ));
        }
        if dependency.get("git").is_some() {
            errors.insert(format!(
                "workspace dependency {name} must use an exact registry pin or owned path"
            ));
        }
        if version.is_none() && dependency.get("path").is_none() {
            errors.insert(format!(
                "workspace dependency {name} needs an exact version or owned path"
            ));
        }
        if let Some(path) = dependency.get("path").and_then(Value::as_str)
            && !owned_member(path)
        {
            errors.insert(format!(
                "workspace dependency {name} path must belong to apps/ or packages/"
            ));
        }
    }
    if !fs::read_to_string(root.join("AGENTS.md"))?.starts_with("# Kuru\n") {
        errors.insert("AGENTS.md must contain the canonical repository instructions".into());
    }
    if fs::read_to_string(root.join("CLAUDE.md"))?.trim() != "@AGENTS.md" {
        errors.insert("CLAUDE.md must import the canonical instructions with @AGENTS.md".into());
    }
    for filename in [
        "package.json",
        "package-lock.json",
        "bun.lock",
        "bun.lockb",
        "pyproject.toml",
        "uv.lock",
    ] {
        if root.join(filename).exists() {
            errors.insert(format!(
                "{filename}: language tooling must belong to an app or package"
            ));
        }
    }
    if root.join("scripts").is_dir() {
        for entry in fs::read_dir(root.join("scripts"))? {
            let entry = entry?;
            let name = entry.file_name();
            if !entry.file_type()?.is_file()
                || !matches!(
                    name.to_str(),
                    Some("check-commit.sh" | "install.sh" | "install.ps1")
                )
            {
                errors.insert(format!(
                    "scripts/{}: helper implementation must belong to an app or package",
                    name.to_string_lossy()
                ));
            }
        }
    }
    if let Some(tools) = config.get("tools").and_then(Value::as_table) {
        for name in ["bun", "python", "uv"] {
            if tools.contains_key(name) {
                errors.insert(format!(
                    "mise tool {name} is not required by native repository tooling"
                ));
            }
        }
    }
    let members = workspace
        .get("members")
        .and_then(Value::as_array)
        .context("Cargo workspace lacks members")?;
    let mut declared = BTreeSet::new();
    for value in members {
        let member = value
            .as_str()
            .context("Cargo workspace member must be a path string")?;
        declared.insert(member.to_owned());
        if !owned_member(member) {
            errors.insert(format!(
                "workspace member must belong directly to apps/ or packages/: {member}"
            ));
            continue;
        }
        let directory = root.join(member);
        if directory.exists() && !directory.canonicalize()?.starts_with(&canonical_root) {
            errors.insert(format!("workspace member escapes repository: {member}"));
            continue;
        }
        if !directory.join("Cargo.toml").is_file() {
            errors.insert(format!("workspace member missing: {member}"));
        } else {
            inherited_dependencies(
                &read_toml(&directory.join("Cargo.toml"))?,
                member,
                &mut errors,
            );
        }
        if !directory.join("mise.toml").is_file() {
            errors.insert(format!("{member}: package-owned mise.toml is missing"));
        }
    }
    for category in ["apps", "packages"] {
        let directory = root.join(category);
        if !directory.is_dir() {
            continue;
        }
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let path = entry.path();
            let member = format!("{category}/{}", entry.file_name().to_string_lossy());
            let rust_package = path.join("Cargo.toml").is_file();
            let node_package = path.join("package.json").is_file();
            if rust_package && !declared.contains(&member) {
                errors.insert(format!(
                    "Rust package missing from Cargo workspace: {member}"
                ));
            }
            if (rust_package || node_package) && !path.join("mise.toml").is_file() {
                errors.insert(format!("{member}: package-owned mise.toml is missing"));
            }
        }
    }
    Ok(errors.into_iter().collect())
}
