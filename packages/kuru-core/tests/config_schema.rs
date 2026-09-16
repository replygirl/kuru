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
    let example = "provider='responses'\nmodel='future-model'\neffort='future-effort'\nassumed_context_window_tokens=64000\n[mcp.local]\ncommand='runner'\nargs=['--stdio']\n[mcp.local.env]\nTOKEN='from-environment'\n[external_agents]\npeer='https://example.test/a2a'\n[memory]\nstartup_timeout_secs=60";
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
