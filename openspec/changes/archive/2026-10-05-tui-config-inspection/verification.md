# Verification

## 1. Interactive configuration inspection [critical]

- [x] 1.1 @unit verify `/config` is registered, sorted, parsed without arguments, present in help, and completed from a leading prefix -> `catalog_names_are_unique_sorted_and_parse_to_their_dispatch_ids` passed with `/config` in the canonical registry.
- [x] 1.2 @integration build a captured `ConfigSnapshot` from multiple config layers and saved preferences, then inspect its projection -> all 9 `config_display_projection_tests` passed; each effective leaf uses its winning source and saved preference provenance appears only when that preference wins.
- [x] 1.3 @e2e submit `/config` through a real PTY after the TUI enters its first stable frame -> `tui_config_inspects_captured_redacted_configuration_in_a_synchronized_pty_frame` passed; captured sources and separate current runtime selections rendered without a provider turn.
- [x] 1.4 @e2e change a source file after launch and submit `/config` in the reviewed TUI -> the same synchronized PTY test passed; the projection stayed bound to launch-time bytes, without reopening trust review.

## 2. Secret-safe projection [critical]

- [x] 2.1 @unit project fake values in MCP environment entries, static header values, endpoint URL userinfo, and sensitive query values -> the core projection tests passed with no fake secret leakage; named environment references remain visible without resolving them, and ordinary capability names are retained.
- [x] 2.2 @integration inspect the final structured projection for authority-claim and ordinary leaves -> core projection tests passed; source labels are escaped/bounded and effective leaves use captured origins or the explicit built-in-default label.
- [x] 2.3 @unit inspect `kuru config`'s serialized snapshot with fake secret values -> `cli_snapshot_toml_redacts_secret_values_and_preserves_parseable_shape` passed; output remains parseable TOML with secret values redacted, references retained, and saved preferences omitted.

## 3. Live selection and output bounds [critical]

- [x] 3.1 @e2e use the real PTY to select a valid mode and then an invalid mode, submitting `/config` after each settled operation; use the real command-dispatch fixture for model and effort success/refusal -> PTY `mode=freudian` after success and unchanged `mode=freudian` after refusal; `slash_commands_change_real_runtime_state_and_validate_errors` passed for live mode/model/effort operations; the TestBackend frame renders the typed live selection.
- [x] 3.2 @integration project an oversized valid configuration and render it through the deterministic TestBackend -> core bounds and renderer tests passed; output stays within configured bounds and displays omission/truncation counts. The real PTY covers ordinary-size redaction and navigation, not oversized output.

## 4. Documentation

- [x] 4.1 @integration run the docs package content/link checks after updating the command reference -> `mise run //apps/kuru-docs:check` passed, including format, docs build, and public artifact link/anchor checks.
