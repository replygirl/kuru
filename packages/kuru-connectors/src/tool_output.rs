use std::fmt;

use anyhow::Error;
use serde_json::Value;

/// A producer result before the single ToolHost projection boundary.
pub(crate) enum ToolExecution {
    Text(String),
    Json(Value),
    ApplicationError {
        kind: ToolFailureKind,
        content: ToolContent,
    },
}

pub(crate) enum ToolContent {
    Json(Value),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ToolFailureKind {
    BuiltIn,
    McpRoute,
    McpCall,
    McpApplication,
    OutputWithheld,
}

impl fmt::Display for ToolFailureKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::BuiltIn => "tool execution failed",
            Self::McpRoute => "MCP route failed",
            Self::McpCall => "MCP tool call failed",
            Self::McpApplication => "MCP tool application error",
            Self::OutputWithheld => "tool result withheld",
        })
    }
}

pub(crate) struct ToolFailure {
    pub(crate) kind: ToolFailureKind,
    pub(crate) error: Error,
}

impl ToolFailure {
    pub(crate) fn built_in(error: Error) -> Self {
        Self {
            kind: ToolFailureKind::BuiltIn,
            error,
        }
    }
}

pub(crate) struct ProjectedToolError {
    kind: ToolFailureKind,
    detail: String,
}

impl ProjectedToolError {
    pub(crate) fn new(kind: ToolFailureKind, detail: String) -> Self {
        Self { kind, detail }
    }

    pub(crate) fn output_withheld() -> Self {
        Self::new(ToolFailureKind::OutputWithheld, String::new())
    }
}

impl fmt::Display for ProjectedToolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.kind.to_string())?;
        if !self.detail.is_empty() {
            write!(formatter, ": {}", self.detail)?;
        }
        Ok(())
    }
}

impl fmt::Debug for ProjectedToolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProjectedToolError")
            .field("kind", &self.kind)
            .field("detail", &self.detail)
            .finish()
    }
}

impl std::error::Error for ProjectedToolError {}
