# Spec Delta

## ADDED Requirements

### Requirement: Bounded local doctor report

`kuru doctor` MUST report local configuration validity, workspace trust, ChatGPT subscription status, Responses API-key route status, current-project memory status, and embedded-engine asset integrity using stable fixed status codes. Human and JSON output MUST use the same observations and MUST NOT include credentials, account identifiers, configuration values, private paths, or memory content.

#### Scenario: Independent authentication routes
- **WHEN** a user runs the doctor with either authentication route configured
- **THEN** it reports ChatGPT subscription authentication separately from Responses route configuration, without provider requests, refresh, route fallback, or secret disclosure

#### Scenario: Responses environment authority is not approved
- **WHEN** pure configuration parsing identifies a repository-selected Responses environment-variable name that has not passed the applicable workspace-trust review
- **THEN** the doctor may include that literal in its captured trust manifest, but MUST NOT resolve or look up the variable or read its environment value; it reports the route as unverified while still reporting independent fixed ChatGPT account status

#### Scenario: Invalid and unreadable configuration
- **WHEN** configuration parsing finds invalid content or configuration I/O is uncertain
- **THEN** the report distinguishes invalid configuration from an unreadable or uncertain source using fixed redacted reasons

#### Scenario: Embedded payload check
- **WHEN** the doctor checks the bundled engine asset
- **THEN** it reports whether the embedded asset matches its declared bounded integrity metadata and does not claim to attest to the installed executable or package signature

### Requirement: Non-activating project memory inspection

The doctor MUST inspect only the canonical current project. It MUST NOT create a project, open a writable store, elect or start a service, provision or extract an engine, migrate, repair, or acquire lifecycle ownership. It MAY inspect validated structure and attach to an already-published compatible owner through a bounded read-only handle. If no safe existing owner can be inspected, it MUST report the deeper live database health as unverified.

#### Scenario: Fresh project
- **WHEN** no project memory has been activated
- **THEN** the doctor reports memory as absent without creating files, service state, or engine data

#### Scenario: Existing compatible owner
- **WHEN** a compatible memory owner is already published for the current project
- **THEN** the doctor performs only a bounded read-only inspection and releases its inspection handle without affecting owner lifetime

#### Scenario: Cold or uncertain memory
- **WHEN** project structure is valid but no compatible owner is safely inspectable, or ownership/health is uncertain
- **THEN** the doctor reports the relevant bounded structural result and leaves deeper SQL health unverified without provisioning, repair, or an indefinite wait

#### Scenario: Invalid activation
- **WHEN** activation metadata or canonical project structure is invalid
- **THEN** the doctor reports a fixed redacted invalid-state result and leaves all files unchanged

### Requirement: Stable diagnostic exit and output contract

The doctor MUST select behavior from the parsed typed CLI command, MUST emit machine-readable JSON on request, and MUST return stable report exits: `0` when no actionable local problem was found, `1` when workspace/data invocation cannot be resolved, `2` when a check is unverified, and `3` when an actionable local problem was found. An unselected route and absent fresh-project memory are informational; a missing key for a selected Responses route or missing ChatGPT sign-in for a selected `codex` route is actionable. The command MUST NOT execute configured tools or instructions, invoke providers, or grant workspace authority.

#### Scenario: Machine-readable report
- **WHEN** a user requests JSON output
- **THEN** the output contains the same fixed observations as human output and contains no secret values, account identifiers, private file paths, or memory rows

#### Scenario: Selected and unused authentication routes
- **WHEN** the selected route lacks its required local sign-in or key, or an authentication route is unused
- **THEN** the selected-route problem returns exit `3` with a fixed remedy, while the unused route is reported as not selected without prompting a route switch

#### Scenario: Fresh project memory
- **WHEN** a project has no activated memory store and its selected authentication route has no actionable issue
- **THEN** memory is reported as not initialized and the command returns exit `0` without creating state

#### Scenario: Untrusted repository
- **WHEN** repository configuration has not received required workspace approval
- **THEN** the doctor may parse the configuration and inspect the captured trust manifest, but reports the trust limitation without looking up that configured name in the environment or reading its value
