use std::{fs, path::Path};

use kuru_core::Config;
use serde_json::Value;
use tempfile::TempDir;

const SCHEMA: &str = include_str!("../../../apps/kuru-docs/public/configuration.v1.schema.json");

fn schema() -> jsonschema::Validator {
    jsonschema::validator_for(&serde_json::from_str(SCHEMA).unwrap()).unwrap()
}

fn json_from_toml(text: &str) -> Value {
    serde_json::to_value(toml::from_str::<toml::Value>(text).unwrap()).unwrap()
}

fn parse_config(text: &str) -> anyhow::Result<Config> {
    let dir = TempDir::new().unwrap();
    write(dir.path().join(".kuru/config.toml"), text);
    Config::load(None, dir.path(), None)
}

fn write(path: impl AsRef<Path>, text: &str) {
    let path = path.as_ref();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

#[test]
fn published_schema_accepts_defaults_and_documented_configuration() {
    let validator = schema();
    let defaults = serde_json::to_value(Config::default()).unwrap();
    assert!(validator.is_valid(&defaults));
    let example = "provider='responses'\nmodel='future-model'\neffort='future-effort'\nassumed_context_window_tokens=64000\ncontext_output_reserve_tokens=4096\n[mcp.local]\ncommand='runner'\nargs=['--stdio']\n[mcp.local.env]\nTOKEN='from-environment'\n[external_agents]\npeer='https://example.test/a2a'\n[memory]\nstartup_timeout_secs=60";
    let value = json_from_toml(example);
    assert!(validator.is_valid(&value));
    parse_config(example).unwrap();
}

#[test]
fn schema_and_parser_reject_unknown_keys_and_shared_bounds() {
    let validator = schema();
    for text in [
        "unexpected=true",
        "[memory]\nunexpected=true",
        "[mcp.local]\ncommand='runner'\nunexpected=true",
        "assumed_context_window_tokens=0",
        "assumed_context_window_tokens=2000001",
        "context_output_reserve_tokens=0",
        "context_output_reserve_tokens=2000001",
        "[memory]\nstartup_timeout_secs=0",
    ] {
        assert!(!validator.is_valid(&json_from_toml(text)), "schema: {text}");
        assert!(parse_config(text).is_err(), "parser: {text}");
    }

    let too_many_mcp = (0..65)
        .map(|index| format!("[mcp.m{index}]\ncommand='runner'"))
        .collect::<Vec<_>>()
        .join("\n");
    let too_many_agents = format!(
        "[external_agents]\n{}",
        (0..65)
            .map(|index| format!("a{index}='https://example.test/a2a'"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    for text in [too_many_mcp, too_many_agents] {
        assert!(
            !validator.is_valid(&json_from_toml(&text)),
            "schema: {text}"
        );
        assert!(parse_config(&text).is_err(), "parser: {text}");
    }
}

/// The forward-compatibility policy is documented, so the rejection has to say
/// it rather than leave the reader with a bare type error.
#[test]
fn unknown_keys_are_rejected_with_the_documented_forward_compatibility_message() {
    const POLICY: &str = "nknown keys are rejected: a configuration that needs a new key requires a newer Kuru version and never silently changes authority";
    for page in [
        include_str!("../../../docs/configuration.md"),
        include_str!("../../../apps/kuru-docs/reference/configuration.md"),
    ] {
        let flattened = page.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(flattened.contains(POLICY), "documented policy drifted");
    }

    for text in [
        "unexpected=true",
        "[memory]\nunexpected=true",
        "[mcp.local]\ncommand='runner'\nunexpected=true",
        "[[permissions]]\naction='allow'\nselector={kind='native',name='file_write'}\nextra=true",
    ] {
        let error = format!("{:#}", parse_config(text).unwrap_err());
        assert!(error.contains(POLICY), "{text}: {error}");
        assert!(!error.contains("unexpected"), "{text}: {error}");
    }

    // An in-range key with an out-of-range value is not a forward-compatibility
    // rejection and must not borrow its message.
    let error = format!(
        "{:#}",
        parse_config("assumed_context_window_tokens=0").unwrap_err()
    );
    assert!(!error.contains(POLICY), "{error}");
}

#[test]
fn native_validation_keeps_cross_field_rules_authoritative() {
    for text in [
        "[mcp.local]\ncommand='runner'\nurl='https://example.test/mcp'",
        "[mcp.local]\nurl='https://example.test/mcp'\nargs=['invalid']",
        "mode='freudian'\nmax_parts=2",
    ] {
        assert!(parse_config(text).is_err(), "{text}");
    }
}

#[test]
fn permission_schema_and_parser_agree_on_documented_and_invalid_rules() {
    let validator = schema();
    let documented = concat!(
        "allow_write=false\n",
        "[[permissions]]\naction='ask'\nselector={kind='native',name='file_write'}\npath='src/**'\n",
        "[[permissions]]\naction='deny'\nselector={kind='native',name='shell'}\n",
        "[[permissions]]\naction='ask'\nselector={kind='mcp',alias='local_service',tool='write_document'}\n",
        "[[permissions]]\naction='ask'\nselector={kind='a2a',alias='research_peer'}\n",
        "[mcp.local_service]\ncommand='runner'\n",
        "[external_agents]\nresearch_peer='https://example.test/a2a'\n"
    );
    assert!(validator.is_valid(&json_from_toml(documented)));
    assert_eq!(parse_config(documented).unwrap().permissions.len(), 4);

    for rule in [
        "action='permit'\nselector={kind='native',name='file_write'}",
        "action='allow'\nselector={kind='native',name='missing'}",
        "action='allow'\nselector={kind='native',name='file_write',description='anything'}",
        "action='allow'\nselector={kind='native',name='file_write'}\nextra=true",
        "action='allow'\nselector={kind='native',name='file_write'}\npath='/absolute'",
        "action='allow'\nselector={kind='native',name='file_write'}\npath='src/../secret'",
        "action='allow'\nselector={kind='native',name='file_write'}\npath='src/a**b'",
        "action='allow'\nselector={kind='native',name='file_write'}\npath='src/[ab]'",
        "action='allow'\nselector={kind='native',name='shell'}\npath='src/**'",
    ] {
        let text = format!("[[permissions]]\n{rule}");
        assert!(
            !validator.is_valid(&json_from_toml(&text)),
            "schema: {text}"
        );
        assert!(parse_config(&text).is_err(), "parser: {text}");
    }

    // Unicode scalar counts, rather than UTF-8 byte counts, match the schema's
    // maxLength contract. The same C0, DEL and C1 controls are forbidden.
    for pattern in ["1:report", &"é".repeat(512)] {
        let text = format!(
            "[[permissions]]\naction='ask'\nselector={{kind='native',name='file_read'}}\npath='{pattern}'"
        );
        assert!(validator.is_valid(&json_from_toml(&text)), "schema: {text}");
        parse_config(&text).unwrap();
    }
    let oversized = "é".repeat(513);
    let text = format!(
        "[[permissions]]\naction='ask'\nselector={{kind='native',name='file_read'}}\npath='{oversized}'"
    );
    assert!(!validator.is_valid(&json_from_toml(&text)));
    assert!(parse_config(&text).is_err());
    for (length, accepted) in [(256, true), (257, false)] {
        let tool = "é".repeat(length);
        let text = format!(
            "[[permissions]]\naction='ask'\nselector={{kind='mcp',alias='files',tool='{tool}'}}\n[mcp.files]\ncommand='runner'"
        );
        assert_eq!(validator.is_valid(&json_from_toml(&text)), accepted);
        assert_eq!(parse_config(&text).is_ok(), accepted);
    }
    for control in ['\u{7f}', '\u{85}'] {
        let path = format!("src/{control}");
        let value = serde_json::json!({"permissions":[{
            "action":"ask", "selector":{"kind":"native","name":"file_read"}, "path":path
        }]});
        assert!(!validator.is_valid(&value));
        let text = format!(
            "[[permissions]]\naction='ask'\nselector={{kind='native',name='file_read'}}\npath='src/{control}'"
        );
        assert!(parse_config(&text).is_err());
        let value = serde_json::json!({"permissions":[{
            "action":"ask", "selector":{"kind":"mcp","alias":"files","tool":format!("write{control}")}
        }]});
        assert!(!validator.is_valid(&value));
    }
}
