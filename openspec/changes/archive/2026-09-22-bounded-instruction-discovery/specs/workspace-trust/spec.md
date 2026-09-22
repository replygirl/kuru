## ADDED Requirements

### Requirement: Imported instruction authority is captured before review

Kuru SHALL derive the applicable project-instruction claim from the rendered encounter order of all active `AGENTS.md`, `CLAUDE.md` and imported sources, binding each full path digest, checked containing-directory and file identity, and exact captured bytes. An omitted over-cap source SHALL NOT activate prompt authority; a subsequently fitting or changed source SHALL produce a newly reviewed manifest. Kuru SHALL inject only the captured projection after the complete applicable workspace manifest passes the existing exact-root once or persistent approval preflight. It MUST NOT reopen an instruction source after that review to build the same invocation's prompt.

#### Scenario: Imported bytes change after approval
- **WHEN** an imported source changes after approval but before dispatch in the current invocation
- **THEN** the current invocation uses only its captured reviewed bytes, and a new snapshot sees a different manifest requiring fresh review before the changed bytes can activate.

#### Scenario: Explicit local override does not bless import
- **WHEN** a user-owned local config overrides a repository setting and a repository `CLAUDE.md` imports another instruction file
- **THEN** the effective override keeps user provenance while the imported prompt bytes remain a separate repository-origin claim requiring workspace review.
