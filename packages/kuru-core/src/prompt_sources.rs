//! Checked, bounded prompt-source capture. Metadata discovery never reads a
//! skill body; selection reopens the same checked source before disclosure.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail, ensure};
use kuru_platform::fs::{Directory, NameRetention, Privacy, regular_file_info};
use serde::Deserialize;

pub(crate) const MAX_SKILLS: usize = 64;
pub(crate) const MAX_COMMANDS: usize = 64;
const MAX_CANDIDATES: usize = 256;
const MAX_METADATA_FILE: usize = 8 * 1024;
const MAX_METADATA_TOTAL: usize = 64 * 1024;
const MAX_COMMAND_FILE: usize = 64 * 1024;
const MAX_COMMAND_TOTAL: usize = 256 * 1024;
const MAX_MATERIAL_FILE: usize = 256 * 1024;
pub(crate) const MAX_MATERIAL_TOTAL: usize = 1024 * 1024;
pub(crate) const MAX_MATERIAL_SOURCES: usize = 128;
const MAX_NOTICES: usize = 16;

#[derive(Clone, Debug)]
pub(crate) struct PromptSource {
    pub path: PathBuf,
    pub directory_identity: [u8; 24],
    pub file_identity: [u8; 24],
    pub project: bool,
}

#[derive(Clone, Debug)]
pub struct SkillMetadata {
    pub name: String,
    pub description: String,
    pub(crate) source: PromptSource,
    pub(crate) frontmatter: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct CustomCommand {
    pub name: String,
    pub description: String,
    pub body: String,
    pub(crate) source: PromptSource,
    pub(crate) content: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum MaterialKind {
    SkillBody,
    SkillReference,
}

#[derive(Clone, Debug)]
pub(crate) struct PromptMaterial {
    pub kind: MaterialKind,
    pub name: String,
    pub source: PromptSource,
    pub content: String,
}

#[derive(Clone, Debug, Default)]
pub struct PromptCatalog {
    skills: BTreeMap<String, SkillMetadata>,
    commands: BTreeMap<String, CustomCommand>,
    notices: Vec<String>,
}

#[derive(Deserialize)]
struct Frontmatter {
    name: String,
    description: String,
}

impl PromptCatalog {
    pub(crate) fn discover(
        project: &Path,
        user_config_root: Option<&Path>,
        built_ins: &[&str],
    ) -> Result<Self> {
        let mut catalog = Self::default();
        let mut metadata_bytes = 0usize;
        let mut command_bytes = 0usize;
        let user = user_config_root.and_then(|path| {
            let canonical = path.canonicalize().ok()?;
            if canonical.starts_with(project) {
                catalog.notice("user prompt directory is inside the project and was omitted");
                None
            } else {
                Some(canonical)
            }
        });
        // Capture higher-precedence project entries first so an invalid or
        // oversized lower-precedence source cannot spend the effective budget.
        for (base, project_origin) in [(Some(project), true), (user.as_deref(), false)] {
            let Some(base) = base else { continue };
            let skill_root = if project_origin {
                base.join(".agents/skills")
            } else {
                base.join("skills")
            };
            for name in catalog.names(&skill_root, true)? {
                if catalog.skills.contains_key(&name) {
                    continue;
                }
                if catalog.skills.len() >= MAX_SKILLS {
                    catalog.notice("additional skills exceeded the 64-entry catalog limit");
                    continue;
                }
                let path = skill_root.join(&name).join("SKILL.md");
                let attempt = (|| -> Result<SkillMetadata> {
                    let (mut file, source) = open_checked(&path, base, project_origin)?;
                    let frontmatter = read_frontmatter(&mut file)?;
                    let parsed = parse_frontmatter(&frontmatter, &name)?;
                    let next_total = metadata_bytes
                        .checked_add(frontmatter.len())
                        .context("metadata total overflow")?;
                    ensure!(next_total <= MAX_METADATA_TOTAL, "metadata total limit");
                    verify(&path, &file, &source)?;
                    metadata_bytes = next_total;
                    Ok(SkillMetadata {
                        name: parsed.name,
                        description: parsed.description,
                        source,
                        frontmatter,
                    })
                })();
                match attempt {
                    Ok(skill) => {
                        catalog.skills.insert(skill.name.clone(), skill);
                    }
                    Err(_) => catalog.notice("a skill was omitted (invalid source or limit)"),
                }
            }
            let command_root = if project_origin {
                base.join(".kuru/commands")
            } else {
                base.join("commands")
            };
            for name in catalog.names(&command_root, false)? {
                if built_ins
                    .iter()
                    .any(|built_in| *built_in == format!("/{name}"))
                {
                    catalog.notice("a custom command was shadowed by a built-in");
                    continue;
                }
                if catalog.commands.contains_key(&name) {
                    continue;
                }
                if catalog.commands.len() >= MAX_COMMANDS {
                    catalog.notice("additional commands exceeded the 64-entry catalog limit");
                    continue;
                }
                let path = command_root.join(format!("{name}.md"));
                let attempt = (|| -> Result<CustomCommand> {
                    let (mut file, source) = open_checked(&path, base, project_origin)?;
                    let frontmatter = read_frontmatter(&mut file)?;
                    let parsed = parse_frontmatter(&frontmatter, &name)?;
                    let mut bytes = frontmatter.clone();
                    file.by_ref()
                        .take((MAX_COMMAND_FILE + 1 - bytes.len()) as u64)
                        .read_to_end(&mut bytes)?;
                    ensure!(bytes.len() <= MAX_COMMAND_FILE, "command file limit");
                    let next_total = command_bytes
                        .checked_add(bytes.len())
                        .context("command total overflow")?;
                    ensure!(next_total <= MAX_COMMAND_TOTAL, "command total limit");
                    verify(&path, &file, &source)?;
                    let body = String::from_utf8(bytes[frontmatter.len()..].to_vec())?;
                    ensure!(!body.trim().is_empty(), "empty command body");
                    command_bytes = next_total;
                    Ok(CustomCommand {
                        name: parsed.name,
                        description: parsed.description,
                        body,
                        source,
                        content: String::from_utf8(bytes)?,
                    })
                })();
                match attempt {
                    Ok(command) => {
                        catalog.commands.insert(command.name.clone(), command);
                    }
                    Err(_) => {
                        catalog.notice("a custom command was omitted (invalid source or limit)")
                    }
                }
            }
        }
        Ok(catalog)
    }

    pub fn skills(&self) -> impl Iterator<Item = &SkillMetadata> {
        self.skills.values()
    }

    pub fn commands(&self) -> impl Iterator<Item = &CustomCommand> {
        self.commands.values()
    }

    pub fn notices(&self) -> &[String] {
        &self.notices
    }

    pub(crate) fn skill(&self, name: &str) -> Option<&SkillMetadata> {
        self.skills.get(name)
    }

    pub(crate) fn project_skills(&self) -> impl Iterator<Item = &SkillMetadata> {
        self.skills.values().filter(|entry| entry.source.project)
    }

    pub(crate) fn project_commands(&self) -> impl Iterator<Item = &CustomCommand> {
        self.commands.values().filter(|entry| entry.source.project)
    }

    fn notice(&mut self, message: &str) {
        if self.notices.len() < MAX_NOTICES {
            self.notices.push(message.to_owned());
        } else if self.notices.len() == MAX_NOTICES {
            self.notices
                .push("Additional prompt-source omissions exceeded the notice limit".into());
        }
    }

    fn names(&mut self, root: &Path, directories: bool) -> Result<Vec<String>> {
        let metadata = match fs::symlink_metadata(root) {
            Ok(value) => value,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(_) => {
                self.notice("a prompt-source directory could not be inspected");
                return Ok(Vec::new());
            }
        };
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            self.notice("a prompt-source directory was not a checked directory");
            return Ok(Vec::new());
        }
        let entries = match fs::read_dir(root) {
            Ok(entries) => entries,
            Err(_) => {
                self.notice("a prompt-source directory could not be listed");
                return Ok(Vec::new());
            }
        };
        let mut names = BTreeSet::new();
        let mut candidates = 0usize;
        for entry in entries {
            candidates += 1;
            if candidates > MAX_CANDIDATES {
                self.notice("a prompt-source directory exceeded the candidate limit");
                return Ok(Vec::new());
            }
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => {
                    self.notice("a prompt-source entry could not be inspected");
                    continue;
                }
            };
            let Some(text) = entry.file_name().to_str().map(str::to_owned) else {
                self.notice("a non-UTF-8 prompt-source entry was omitted");
                continue;
            };
            let name = if directories {
                text
            } else {
                let Some(stem) = text.strip_suffix(".md") else {
                    continue;
                };
                stem.to_owned()
            };
            if !valid_name(&name) {
                self.notice("an invalid prompt-source name was omitted");
                continue;
            }
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(_) => {
                    self.notice("a prompt-source entry type could not be inspected");
                    continue;
                }
            };
            if (directories && !file_type.is_dir()) || (!directories && !file_type.is_file()) {
                self.notice("an unsafe prompt-source entry was omitted");
                continue;
            }
            names.insert(name);
        }
        Ok(names.into_iter().collect())
    }
}

pub(crate) fn select_material(
    skill: &SkillMetadata,
    reference: Option<&str>,
    project: &Path,
    user_config_root: Option<&Path>,
) -> Result<Vec<PromptMaterial>> {
    let base = if skill.source.project {
        project
    } else {
        user_config_root.context("user skill root is unavailable")?
    };
    let (mut file, source) = open_checked(&skill.source.path, base, skill.source.project)?;
    ensure!(
        source.directory_identity == skill.source.directory_identity
            && source.file_identity == skill.source.file_identity,
        "skill source changed after catalog capture"
    );
    let frontmatter = read_frontmatter(&mut file)?;
    ensure!(
        frontmatter == skill.frontmatter,
        "skill metadata changed after catalog capture"
    );
    let mut body = Vec::new();
    file.by_ref()
        .take((MAX_MATERIAL_FILE + 1) as u64)
        .read_to_end(&mut body)?;
    ensure!(
        body.len() <= MAX_MATERIAL_FILE,
        "skill body exceeds file limit"
    );
    verify(&skill.source.path, &file, &source)?;
    let body = String::from_utf8(body)?;
    ensure!(!body.trim().is_empty(), "selected skill has an empty body");
    let mut selected = vec![PromptMaterial {
        kind: MaterialKind::SkillBody,
        name: skill.name.clone(),
        source,
        content: body,
    }];
    if let Some(reference) = reference {
        let path = reference_path(&skill.source.path, reference)?;
        let (mut file, source) = open_checked(&path, base, skill.source.project)?;
        let mut bytes = Vec::new();
        file.by_ref()
            .take((MAX_MATERIAL_FILE + 1) as u64)
            .read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() <= MAX_MATERIAL_FILE,
            "skill reference exceeds file limit"
        );
        verify(&path, &file, &source)?;
        selected.push(PromptMaterial {
            kind: MaterialKind::SkillReference,
            name: skill.name.clone(),
            source,
            content: String::from_utf8(bytes)?,
        });
    }
    Ok(selected)
}

pub(crate) fn revalidate_material(
    source: &PromptSource,
    project: &Path,
    user: Option<&Path>,
) -> Result<()> {
    let base = if source.project {
        project
    } else {
        user.context("user skill root is unavailable")?
    };
    let (file, current) = open_checked(&source.path, base, source.project)?;
    ensure!(
        current.directory_identity == source.directory_identity
            && current.file_identity == source.file_identity,
        "selected prompt source changed after capture"
    );
    verify(&source.path, &file, &current)?;
    Ok(())
}

fn reference_path(skill_file: &Path, reference: &str) -> Result<PathBuf> {
    let mut parts = reference.split('/');
    ensure!(
        parts.next() == Some("references"),
        "reference must be under references/"
    );
    let name = parts.next().context("reference name is missing")?;
    ensure!(
        parts.next().is_none(),
        "nested reference paths are unsupported"
    );
    ensure!(valid_reference_name(name), "invalid reference name");
    Ok(skill_file
        .parent()
        .context("skill has no directory")?
        .join(reference))
}

fn valid_reference_name(name: &str) -> bool {
    name.ends_with(".md")
        && !name.starts_with('.')
        && !name.contains(['\\', ':'])
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('-')
        && !name.ends_with('-')
        && !name.contains("--")
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn open_checked(path: &Path, base: &Path, project: bool) -> Result<(File, PromptSource)> {
    let guard = Directory::open(base, Privacy::Inherited, NameRetention::Pinned)
        .context("prompt source root is unavailable")?;
    let parent = path.parent().context("prompt source lacks parent")?;
    let directory = Directory::open(parent, Privacy::Inherited, NameRetention::Pinned)
        .context("prompt source parent is unsafe")?;
    ensure!(
        directory.is_within(&guard)?,
        "prompt source escapes its root"
    );
    let name = path.file_name().context("prompt source lacks name")?;
    let file = directory
        .read(name)
        .context("prompt source is unavailable")?;
    let info = regular_file_info(&file)?;
    ensure!(info.links == 1, "linked prompt sources are unsupported");
    directory.verify(name, &file)?;
    Ok((
        file,
        PromptSource {
            path: path.to_owned(),
            directory_identity: directory.identity().to_bytes(),
            file_identity: info.identity.to_bytes(),
            project,
        },
    ))
}

fn verify(path: &Path, file: &File, source: &PromptSource) -> Result<()> {
    let parent = path.parent().context("prompt source lacks parent")?;
    let directory = Directory::open(parent, Privacy::Inherited, NameRetention::Pinned)?;
    ensure!(
        directory.identity().to_bytes() == source.directory_identity,
        "prompt source directory changed during read"
    );
    directory.verify(path.file_name().context("prompt source lacks name")?, file)?;
    ensure!(
        regular_file_info(file)?.identity.to_bytes() == source.file_identity,
        "prompt source changed during read"
    );
    Ok(())
}

fn read_frontmatter(file: &mut File) -> Result<Vec<u8>> {
    file.seek(SeekFrom::Start(0))?;
    let mut bytes = Vec::new();
    let mut lines = 0usize;
    let mut line = Vec::new();
    while bytes.len() < MAX_METADATA_FILE {
        let mut one = [0u8; 1];
        ensure!(
            file.read(&mut one)? == 1,
            "missing YAML frontmatter closing fence"
        );
        bytes.push(one[0]);
        line.push(one[0]);
        if one[0] == b'\n' {
            lines += 1;
            let trimmed = line.strip_suffix(b"\n").unwrap_or(&line);
            let trimmed = trimmed.strip_suffix(b"\r").unwrap_or(trimmed);
            ensure!(
                lines != 1 || trimmed == b"---",
                "missing YAML frontmatter opening fence"
            );
            if lines > 1 && trimmed == b"---" {
                return Ok(bytes);
            }
            line.clear();
        }
    }
    bail!("skill frontmatter exceeds 8 KiB")
}

fn parse_frontmatter(bytes: &[u8], expected_name: &str) -> Result<Frontmatter> {
    let text = std::str::from_utf8(bytes)?;
    let body = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))
        .context("invalid YAML opening fence")?;
    let body = body
        .strip_suffix("---\n")
        .or_else(|| body.strip_suffix("---\r\n"))
        .context("invalid YAML closing fence")?;
    let parsed: Frontmatter = serde_yaml_ng::from_str(body).context("invalid YAML frontmatter")?;
    ensure!(
        parsed.name == expected_name && valid_name(&parsed.name),
        "invalid source name"
    );
    ensure!(
        !parsed.description.trim().is_empty()
            && parsed.description.chars().count() <= 1024
            && !parsed.description.chars().any(char::is_control),
        "invalid source description"
    );
    Ok(parsed)
}
