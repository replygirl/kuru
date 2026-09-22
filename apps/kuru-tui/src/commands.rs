//! The working built-in TUI command catalog. Modal review tokens are private
//! UI controls and never appear here.

use std::collections::BTreeMap;

use kuru_core::PromptCatalog;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CommandId {
    Clear,
    Cost,
    Dream,
    Effort,
    FileCheckpoints,
    FileInspect,
    FilePrune,
    FileUndo,
    Focus,
    Help,
    Memory,
    MemoryHistory,
    MemoryStatus,
    Mode,
    Model,
    Notes,
    Parts,
    Permissions,
    Quit,
    Relate,
    Retry,
    Status,
    Tools,
    UndoDream,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct CommandSpec {
    pub id: CommandId,
    pub name: &'static str,
    pub usage: &'static str,
    pub summary: &'static str,
}

/// Sorted by canonical name so help and completion have stable order.
pub(crate) const BUILT_INS: &[CommandSpec] = &[
    CommandSpec {
        id: CommandId::Clear,
        name: "/clear",
        usage: "/clear",
        summary: "Clear this visible conversation view",
    },
    CommandSpec {
        id: CommandId::Cost,
        name: "/cost",
        usage: "/cost",
        summary: "Show known session usage and estimates",
    },
    CommandSpec {
        id: CommandId::Dream,
        name: "/dream",
        usage: "/dream",
        summary: "Run a dream cycle",
    },
    CommandSpec {
        id: CommandId::Effort,
        name: "/effort",
        usage: "/effort LEVEL",
        summary: "Choose reasoning effort",
    },
    CommandSpec {
        id: CommandId::FileCheckpoints,
        name: "/file-checkpoints",
        usage: "/file-checkpoints",
        summary: "List private file checkpoint metadata",
    },
    CommandSpec {
        id: CommandId::FileInspect,
        name: "/file-inspect",
        usage: "/file-inspect ID",
        summary: "Inspect one file checkpoint",
    },
    CommandSpec {
        id: CommandId::FilePrune,
        name: "/file-prune",
        usage: "/file-prune ID [--discard-uncertain]",
        summary: "Prune one inactive file checkpoint",
    },
    CommandSpec {
        id: CommandId::FileUndo,
        name: "/file-undo",
        usage: "/file-undo ID",
        summary: "Undo one applied file effect if unchanged",
    },
    CommandSpec {
        id: CommandId::Focus,
        name: "/focus",
        usage: "/focus NAME|ID|auto",
        summary: "Choose speaking focus",
    },
    CommandSpec {
        id: CommandId::Help,
        name: "/help",
        usage: "/help",
        summary: "Show working commands and keys",
    },
    CommandSpec {
        id: CommandId::Memory,
        name: "/memory",
        usage: "/memory ID",
        summary: "Inspect one part's memory",
    },
    CommandSpec {
        id: CommandId::MemoryHistory,
        name: "/memory-history",
        usage: "/memory-history",
        summary: "Inspect memory revisions",
    },
    CommandSpec {
        id: CommandId::MemoryStatus,
        name: "/memory-status",
        usage: "/memory-status",
        summary: "Inspect managed memory status",
    },
    CommandSpec {
        id: CommandId::Mode,
        name: "/mode",
        usage: "/mode ifs|polyvagal|freudian|jungian",
        summary: "Choose a framework mode",
    },
    CommandSpec {
        id: CommandId::Model,
        name: "/model",
        usage: "/model ID",
        summary: "Choose a model",
    },
    CommandSpec {
        id: CommandId::Notes,
        name: "/notes",
        usage: "/notes ID",
        summary: "Inspect selected notes",
    },
    CommandSpec {
        id: CommandId::Parts,
        name: "/parts",
        usage: "/parts",
        summary: "Show current parts",
    },
    CommandSpec {
        id: CommandId::Permissions,
        name: "/permissions",
        usage: "/permissions",
        summary: "Inspect and revoke tool grants",
    },
    CommandSpec {
        id: CommandId::Quit,
        name: "/quit",
        usage: "/quit",
        summary: "Close this session",
    },
    CommandSpec {
        id: CommandId::Relate,
        name: "/relate",
        usage: "/relate KIND ID,ID",
        summary: "Change a relationship",
    },
    CommandSpec {
        id: CommandId::Retry,
        name: "/retry",
        usage: "/retry",
        summary: "Retry the last settled turn",
    },
    CommandSpec {
        id: CommandId::Status,
        name: "/status",
        usage: "/status",
        summary: "Show this session's current status",
    },
    CommandSpec {
        id: CommandId::Tools,
        name: "/tools",
        usage: "/tools",
        summary: "Inspect filtered tools and MCP server state",
    },
    CommandSpec {
        id: CommandId::UndoDream,
        name: "/undo-dream",
        usage: "/undo-dream",
        summary: "Undo the latest dream membership change",
    },
];

pub(crate) fn built_in_names() -> Vec<&'static str> {
    BUILT_INS.iter().map(|spec| spec.name).collect()
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct CommandRequest<'a> {
    pub id: CommandId,
    pub args: &'a str,
}

pub(crate) fn parse(text: &str) -> Option<CommandRequest<'_>> {
    let (name, args) = text.split_once(' ').unwrap_or((text, ""));
    let id = BUILT_INS.iter().find(|spec| spec.name == name)?.id;
    Some(CommandRequest {
        id,
        args: args.trim(),
    })
}

pub(crate) fn names_matching(prefix: &str) -> Vec<&'static str> {
    if !prefix.starts_with('/') || prefix.chars().any(char::is_whitespace) {
        return Vec::new();
    }
    BUILT_INS
        .iter()
        .filter(|spec| spec.name.starts_with(prefix))
        .map(|spec| spec.name)
        .collect()
}

pub(crate) fn help_text() -> String {
    let mut help = String::from(
        "Enter send · Alt+Enter newline · F2 models · F3 effort · F4 mode · F5 permissions · Esc cancel\nTab completes a leading slash command.\n",
    );
    for spec in BUILT_INS {
        help.push_str(spec.usage);
        help.push_str(" · ");
        help.push_str(spec.summary);
        help.push('\n');
    }
    help.push_str("Approval: Alt+1 Once · Alt+2 Session · Alt+3 Always · Alt+4 Deny.\nModel, effort and mode selections are remembered for this project.");
    help
}

/// An invocation-local catalog built only after workspace preflight. Built-in
/// names remain the single typed source for reserved commands.
#[derive(Clone, Debug, Default)]
pub(crate) struct Registry {
    custom: BTreeMap<String, kuru_core::CustomCommand>,
}

impl Registry {
    pub(crate) fn from_catalog(catalog: &PromptCatalog) -> Self {
        let custom = catalog
            .commands()
            .filter(|entry| parse(&format!("/{}", entry.name)).is_none())
            .map(|entry| (entry.name.clone(), entry.clone()))
            .collect();
        Self { custom }
    }

    pub(crate) fn custom<'a>(
        &'a self,
        text: &'a str,
    ) -> Option<(&'a kuru_core::CustomCommand, &'a str)> {
        let (name, args) = text.split_once(' ').unwrap_or((text, ""));
        if parse(name).is_some() {
            return None;
        }
        self.custom
            .get(name.strip_prefix('/')?)
            .map(|entry| (entry, args.trim()))
    }

    pub(crate) fn names_matching(&self, prefix: &str) -> Vec<String> {
        let mut names = names_matching(prefix)
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        if prefix.starts_with('/') && !prefix.chars().any(char::is_whitespace) {
            names.extend(
                self.custom
                    .keys()
                    .map(|name| format!("/{name}"))
                    .filter(|name| name.starts_with(prefix)),
            );
        }
        names.sort();
        names
    }

    pub(crate) fn help_text(&self) -> String {
        let mut help = help_text();
        for entry in self.custom.values() {
            help.push('\n');
            help.push('/');
            help.push_str(&entry.name);
            help.push_str(" [arguments] · ");
            help.push_str(&entry.description);
        }
        help
    }
}

pub(crate) fn expand_custom(entry: &kuru_core::CustomCommand, args: &str) -> String {
    if args.is_empty() {
        return entry.body.clone();
    }
    format!("{}\n\nArguments (literal text):\n{}", entry.body, args)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kuru_core::{ConfigSnapshot, InvocationOverrides};

    #[test]
    fn catalog_names_are_unique_sorted_and_parse_to_their_dispatch_ids() {
        let names = BUILT_INS.iter().map(|spec| spec.name).collect::<Vec<_>>();
        assert!(names.windows(2).all(|pair| pair[0] < pair[1]));
        let help = help_text();
        for spec in BUILT_INS {
            assert_eq!(parse(spec.name).map(|request| request.id), Some(spec.id));
            assert!(help.contains(spec.usage));
            assert!(names_matching(spec.name).contains(&spec.name));
        }
        for unavailable in [
            "/new",
            "/sessions",
            "/resume",
            "/config",
            "/export",
            "/compact",
            "/instruction-once",
            "/approval-always",
        ] {
            assert!(parse(unavailable).is_none());
            assert!(!names_matching(unavailable).contains(&unavailable));
        }
    }

    #[test]
    fn completion_is_bounded_and_prefix_order_is_stable() {
        assert_eq!(
            names_matching("/mem"),
            vec!["/memory", "/memory-history", "/memory-status"]
        );
        assert_eq!(names_matching("/cle"), vec!["/clear"]);
        assert!(names_matching("ordinary").is_empty());
        assert!(names_matching("/memory ").is_empty());
        assert!(
            parse("/model demo")
                .is_some_and(|request| request.id == CommandId::Model && request.args == "demo")
        );
    }

    #[test]
    fn effective_custom_catalog_drives_help_completion_and_literal_prompt_expansion() {
        let project = tempfile::tempdir().unwrap();
        let commands = project.path().join(".kuru/commands");
        std::fs::create_dir_all(&commands).unwrap();
        std::fs::write(
            commands.join("review.md"),
            "---\nname: review\ndescription: Review this change\n---\nExplain the risks.\n",
        )
        .unwrap();
        std::fs::write(
            commands.join("help.md"),
            "---\nname: help\ndescription: Fake help\n---\nWrong command.\n",
        )
        .unwrap();
        let snapshot = ConfigSnapshot::parse_with_sources(
            None,
            None,
            project.path(),
            None,
            None,
            None,
            &built_in_names(),
            InvocationOverrides::default(),
        )
        .unwrap();
        let registry = Registry::from_catalog(snapshot.prompt_catalog());
        assert_eq!(registry.names_matching("/rev"), ["/review"]);
        assert!(
            registry
                .help_text()
                .contains("/review [arguments] · Review this change")
        );
        assert!(!registry.help_text().contains("Fake help"));
        assert!(registry.custom("/help").is_none());
        let (entry, args) = registry.custom("/review `literal` $HOME").unwrap();
        assert_eq!(args, "`literal` $HOME");
        assert_eq!(
            expand_custom(entry, args),
            "Explain the risks.\n\n\nArguments (literal text):\n`literal` $HOME"
        );
    }
}
