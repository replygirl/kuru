# skill-discovery Specification

## Purpose
Define bounded Agent Skills metadata discovery and the reviewed, on-demand disclosure of selected bodies and references to actors without granting tool authority.

## Requirements

### Requirement: Bounded progressive skill catalog

Kuru SHALL discover Agent Skills from documented user and project directories by reading only bounded YAML `name` and `description` frontmatter at startup. It SHALL validate the name against its directory, keep deterministic project-over-user precedence, capture checked project metadata provenance in the base authority manifest, and show only effective approved metadata to actors. It MUST NOT read or inject a skill's Markdown body or references during catalog discovery. Unsupported, malformed, ambiguous or over-limit entries MUST be omitted with bounded actionable notices rather than silently broadening discovery.

#### Scenario: Metadata without body
- **WHEN** a project skill has valid frontmatter and a distinctive body secret, and the actor has not selected it
- **THEN** the approved actor prompt contains its name and description but not the body secret or any reference bytes.

#### Scenario: Shadowed and invalid entries
- **WHEN** a user and project skill share a name or a candidate's name, layout, frontmatter or limit is invalid
- **THEN** the project entry wins the valid collision, invalid entries remain unavailable, and discovery reports bounded omissions without accepting their bodies.

### Requirement: Selected skill material has exact scoped authority

Kuru SHALL expose a fixed actor `skill_load` selection with a catalog name and optional direct `references/<file>.md` path. It SHALL checked-read the selected full `SKILL.md` body and requested reference only on selection, subject to per-file, aggregate and source-count bounds. Project material SHALL join a complete supplemental authority manifest with exact captured bytes, source and directory/file identities, and a canonical sorted path set; Kuru MUST obtain existing exact-root once/persist approval before promoting new project material into instructions. It SHALL revalidate the retained root, captured directories and trust generation after a foreground wait, publish only the reviewed immutable bytes, and settle the original tool call before the next inference. A missing, changed, denied, unsafe or oversized source MUST NOT yield partial instructions or a side effect. User-config skills use caller authority but retain checked capture and bounds.

#### Scenario: Body and reference are selected separately
- **WHEN** an actor selects a project skill and later requests one of its references
- **THEN** each newly active source is checked, reviewed with the complete effective manifest when required, and added to the next actor prompt without loading unselected references.

#### Scenario: Source replacement during review
- **WHEN** a selected project skill or reference directory is replaced while a persistent review is pending
- **THEN** no approval or prompt update is published for the replaced source, and a later invocation must recapture it.

#### Scenario: Headless trust requirement
- **WHEN** an actor requests unapproved project skill material without an interactive review channel or an explicit one-invocation workspace grant
- **THEN** the tool reports the existing workspace-trust requirement and does not reveal or inject that material.

### Requirement: Skills never grant tools by declaration

Skill metadata, `allowed-tools`, Markdown prose, scripts and references MUST NOT grant file, shell, MCP or protocol permissions. Kuru SHALL NOT execute skill scripts as part of selection. Existing tool permission checks SHALL still apply to any later actor call, independently of skill trust.

#### Scenario: Allowed-tools is present
- **WHEN** a selected `SKILL.md` declares `allowed-tools` or instructs a shell action
- **THEN** skill selection grants no such tool authority, and a later tool call follows the ordinary permission decision path.
