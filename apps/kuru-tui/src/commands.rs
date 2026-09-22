//! The working built-in TUI command catalog. Modal review tokens are private
//! UI controls and never appear here.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CommandId {
    Clear,
    Cost,
    Dream,
    Effort,
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
        id: CommandId::UndoDream,
        name: "/undo-dream",
        usage: "/undo-dream",
        summary: "Undo the latest dream membership change",
    },
];

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

#[cfg(test)]
mod tests {
    use super::*;

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
            "/tools",
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
}
