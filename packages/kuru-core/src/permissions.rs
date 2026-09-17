//! Pure, bounded decisions for already-checked external-effect invocations.
//! Physical root and protected-path validation remain the caller's job.

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

const MAX_RULES: usize = 128;
// JSON Schema maxLength counts Unicode scalar values; keep the same limits.
const MAX_PATTERN_CHARS: usize = 512;
const MAX_TARGET_CHARS: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionAction {
    Allow,
    Ask,
    Deny,
}

impl PermissionAction {
    const fn priority(self) -> u8 {
        match self {
            Self::Allow => 1,
            Self::Ask => 2,
            Self::Deny => 3,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeTool {
    FileRead,
    FileList,
    FileWrite,
    FileDelete,
    Shell,
}

impl NativeTool {
    pub const fn is_file(self) -> bool {
        !matches!(self, Self::Shell)
    }
}

/// Stable identity, independent of provider-facing MCP name hashing.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PermissionSelector {
    Native { name: NativeTool },
    Mcp { alias: String, tool: String },
    A2a { alias: String },
}

impl PermissionSelector {
    pub const fn native(name: NativeTool) -> Self {
        Self::Native { name }
    }

    pub fn mcp(alias: impl Into<String>, tool: impl Into<String>) -> Result<Self> {
        let selector = Self::Mcp {
            alias: alias.into(),
            tool: tool.into(),
        };
        selector.validate()?;
        Ok(selector)
    }

    pub fn a2a(alias: impl Into<String>) -> Result<Self> {
        let selector = Self::A2a {
            alias: alias.into(),
        };
        selector.validate()?;
        Ok(selector)
    }

    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Native { .. } => Ok(()),
            Self::Mcp { alias, tool } => {
                validate_alias(alias)?;
                ensure!(
                    !tool.is_empty()
                        && tool.chars().count() <= 256
                        && !tool.chars().any(char::is_control),
                    "MCP permission tool name must be 1–256 non-control characters"
                );
                Ok(())
            }
            Self::A2a { alias } => validate_alias(alias),
        }
    }

    pub const fn is_file(&self) -> bool {
        matches!(self, Self::Native { name } if name.is_file())
    }
}

fn validate_alias(alias: &str) -> Result<()> {
    ensure!(
        !alias.is_empty()
            && alias.len() <= 64
            && alias
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte)),
        "permission aliases must use 1–64 ASCII letters, digits, underscores, or hyphens"
    );
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PermissionRule {
    pub action: PermissionAction,
    pub selector: PermissionSelector,
    /// Anchored slash-separated project-relative glob, only for native files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

impl PermissionRule {
    pub fn validate(&self) -> Result<()> {
        self.selector.validate()?;
        if let Some(pattern) = &self.path {
            ensure!(
                self.selector.is_file(),
                "permission path patterns require a native file selector"
            );
            validate_pattern(pattern)?;
        }
        Ok(())
    }

    fn matches(
        &self,
        selector: &PermissionSelector,
        target: Option<&ProjectRelativeTarget>,
    ) -> bool {
        self.selector == *selector
            && self.path.as_ref().is_none_or(|pattern| {
                target.is_some_and(|target| glob_matches(pattern, target.as_str()))
            })
    }
}

/// Lexically normalized spelling of a target already validated by platform
/// root/protected-path checks. `.` denotes the checked project root itself.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ProjectRelativeTarget(String);

impl ProjectRelativeTarget {
    pub fn parse(path: impl Into<String>) -> Result<Self> {
        let path = path.into();
        validate_relative(&path, MAX_TARGET_CHARS)?;
        Ok(Self(path))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn validate_relative(path: &str, max_chars: usize) -> Result<()> {
    ensure!(
        !path.is_empty()
            && path.chars().count() <= max_chars
            && !path.chars().any(char::is_control),
        "permission path must be nonempty, bounded and contain no controls"
    );
    if path == "." {
        return Ok(());
    }
    ensure!(
        !path.starts_with('/') && !path.contains('\\') && !path.ends_with('/'),
        "permission path must be anchored project-relative slash spelling"
    );
    ensure!(
        !(path.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
            && path.as_bytes().get(1) == Some(&b':')),
        "permission path cannot use a drive prefix"
    );
    ensure!(
        path.split('/')
            .all(|segment| !matches!(segment, "" | "." | "..")),
        "permission path must not contain empty, dot or traversal segments"
    );
    Ok(())
}

fn validate_pattern(pattern: &str) -> Result<()> {
    validate_relative(pattern, MAX_PATTERN_CHARS)?;
    if pattern == "." {
        return Ok(());
    }
    ensure!(
        !pattern
            .chars()
            .any(|character| matches!(character, '[' | ']' | '{' | '}')),
        "permission path pattern supports only *, ? and whole-segment **"
    );
    ensure!(
        pattern
            .split('/')
            .all(|segment| !segment.contains("**") || segment == "**"),
        "permission ** wildcard must occupy a complete segment"
    );
    Ok(())
}

/// An anchored, bounded glob. Single `*` and `?` never cross a slash;
/// whole-segment `**` spans zero or more path segments.
fn glob_matches(pattern: &str, target: &str) -> bool {
    if pattern == "." {
        return target == ".";
    }
    let patterns = pattern.split('/').collect::<Vec<_>>();
    let targets = if target == "." {
        vec![]
    } else {
        target.split('/').collect::<Vec<_>>()
    };
    let mut previous = vec![false; targets.len() + 1];
    previous[0] = true;
    for pattern in patterns {
        let mut next = vec![false; targets.len() + 1];
        if pattern == "**" {
            next[0] = previous[0];
            for index in 1..next.len() {
                next[index] = previous[index] || next[index - 1];
            }
        } else {
            for index in 1..next.len() {
                next[index] = previous[index - 1] && segment_matches(pattern, targets[index - 1]);
            }
        }
        previous = next;
    }
    previous[targets.len()]
}

fn segment_matches(pattern: &str, target: &str) -> bool {
    let mut previous = vec![false; target.chars().count() + 1];
    previous[0] = true;
    let characters = target.chars().collect::<Vec<_>>();
    for pattern_char in pattern.chars() {
        let mut next = vec![false; previous.len()];
        if pattern_char == '*' {
            next[0] = previous[0];
        }
        for index in 1..next.len() {
            next[index] = match pattern_char {
                '*' => previous[index] || next[index - 1],
                '?' => previous[index - 1],
                ordinary => previous[index - 1] && ordinary == characters[index - 1],
            };
        }
        previous = next;
    }
    previous[characters.len()]
}

pub fn validate_rules(rules: &[PermissionRule]) -> Result<()> {
    ensure!(
        rules.len() <= MAX_RULES,
        "at most 128 permission rules may be configured"
    );
    for rule in rules {
        rule.validate()?;
    }
    Ok(())
}

pub fn decide(
    rules: &[PermissionRule],
    selector: &PermissionSelector,
    target: Option<&ProjectRelativeTarget>,
    allow_write: bool,
    allow_shell: bool,
) -> PermissionAction {
    if validate_rules(rules).is_err()
        || selector.validate().is_err()
        || selector.is_file() != target.is_some()
    {
        return PermissionAction::Deny;
    }
    let matched = rules
        .iter()
        .filter(|rule| rule.matches(selector, target))
        .map(|rule| rule.action)
        .max_by_key(|action| action.priority());
    matched.unwrap_or(match selector {
        PermissionSelector::Native {
            name: NativeTool::FileWrite | NativeTool::FileDelete,
        } => {
            if allow_write {
                PermissionAction::Allow
            } else {
                PermissionAction::Ask
            }
        }
        PermissionSelector::Native {
            name: NativeTool::Shell,
        } => {
            if allow_shell {
                PermissionAction::Allow
            } else {
                PermissionAction::Ask
            }
        }
        _ => PermissionAction::Allow,
    })
}
