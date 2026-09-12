## MODIFIED Requirements

### Requirement: Layered configuration and project instructions

The application SHALL merge user, ancestor project and explicit local
configuration in that order, reject unknown or invalid options, and load
ancestor AGENTS.md instructions with clear local precedence. It SHALL retain the
canonical workspace and a one-shot, side-effect-free configuration snapshot with
field-level provenance before activating automatic-ancestor authority. The
snapshot SHALL capture every CLI override, including `allow_write`,
`allow_shell`, and `no_dream`, before manifest derivation and review; no
post-snapshot mutation may add or alter authority. Explicit configuration and
CLI inputs authorize only their own effective leaves; saved preferences remain
limited to mode, model and effort and cannot add authority. Ordinary model
auto-resolution and runtime preference application MAY occur after the final
snapshot without rereading or changing authority configuration.

#### Scenario: Local override
- **WHEN** a project config changes the user default mode and explicit local config changes it again
- **THEN** the local mode is effective while unrelated inherited options remain.

#### Scenario: Ancestor authority pending approval
- **WHEN** an ancestor contributes an effective authority-bearing configuration
  leaf without a matching workspace approval
- **THEN** the parsed snapshot remains inspectable but runtime activation does
  not begin.

#### Scenario: Malformed automatic configuration
- **WHEN** an automatic or explicit configuration file has a read, TOML parse,
  type, or validation error containing fake secrets or terminal controls
- **THEN** Kuru reports only a bounded escaped source label, coarse error
  category, and parser-provided line and column when available, without raw
  parser text, excerpts, values, or control characters.
