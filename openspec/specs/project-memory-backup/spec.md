# project-memory-backup Specification

## Purpose
Provide independently verified native memory images that preserve complete project history while ordinary writers remain admitted, and restore supported images into absent targets with fresh authority, checked provenance and retained historical namespaces.

## Requirements

### Requirement: Native complete dataset backup

Kuru SHALL capture a project backup with the pinned native Dolt backup operation through its existing authenticated storage owner into a fresh private destination. The completed destination dataset root SHALL define the cut; an earlier source read or SQL success alone MUST NOT establish its contents. The image MUST retain the native reachable revision/ref graph and working sets, including open/resolved candidates, session catalogs and journals, private histories/summaries, schema receipts and the same-dataset operational usage ref. Ordinary independent writers SHALL remain admitted during capture and validation; no writer guard, SQL transaction, conversation-driver lease or maintenance permit SHALL span the bulk copy. Kuru MUST NOT substitute active-record export or expire history to satisfy backup bounds.

#### Scenario: Writers advance during native capture

- **WHEN** a backup runs while independent claimed sessions commit ordinary writes
- **THEN** writers remain usable and the published image restores one complete native root with its associated refs and working sets, rather than a mixture of separately read revisions

#### Scenario: Retained private and operational history

- **WHEN** a project has session-private rows, summaries, journals, candidate refs and usage history
- **THEN** offline restoration retains those rows and their original revision/provenance graph, without copying transient driver capabilities or external service credentials

### Requirement: Checked immutable backup publication

A backup SHALL contain only its bounded versioned manifest and native image. The manifest MUST bind the completed dataset root, source canonical-root/storage identity, historical scope and SQL origin, captured schema and engine provenance, actual ref/working-set observations, and a sorted complete physical inventory with exact sizes and digests. All image paths MUST remain beneath retained checked private directories without links, duplicate records or substitution. Destination parents MAY be ordinary user-selected directories; private staging and the published backup MUST retain owner privacy. Kuru SHALL publish only after independent native validation and owned cleanup, using checked absent-target publication. Failed, cancelled or uncertain attempts MUST preserve their named private stage until actual cleanup is proved and MUST NOT overwrite a target or infer success from an absent reply.

#### Scenario: Target exists or a source name changes

- **WHEN** publication finds an existing target or a retained source/destination identity is substituted
- **THEN** it refuses without overwriting the target or deleting the retained stage

#### Scenario: Cancellation or lost publication response

- **WHEN** a caller disconnects before checked publication or loses its response after the move
- **THEN** cleanup retains accepted native work until settled; an unpublished stage remains distinct from a published image, and later verification proves image validity without claiming an unobserved request outcome

### Requirement: Independent native image verification

`kuru memory verify` SHALL verify manifest structure, exact physical inventory/digests and root, then restore an independent private validation copy with the bundled engine. It MUST run pinned native integrity validation without repair/data-loss flags and validate actual refs, working sets and supported Kuru schema/receipt structure through existing native/SQL boundaries before reporting restorability. The original image MUST remain immutable. Native chunk/commit checking that skips working sets MUST NOT be represented as complete working-set validation. Validity SHALL depend on the current native reader and released schema registry accepting the actual image, not exact equality with the running binary's engine/schema provenance; future, malformed or unsupported images SHALL fail without activation or migration of the original backup.

#### Scenario: Digest-consistent corrupt native image

- **WHEN** an image's metadata and hashes are internally consistent but native graph or working-set validation fails
- **THEN** verification and restore refuse it without activating a target or repairing the original image

#### Scenario: Supported older schema

- **WHEN** the native reader accepts an image whose committed main schema is a supported released version below current
- **THEN** verification reports its captured schema and root unchanged, and restore may migrate only its unpublished copy

### Requirement: Explicit absent-target restore

`kuru memory restore` SHALL restore only into an absent managed target selected by the command's canonical project and explicit data path. A different canonical root MUST require explicit remap authorization recorded in restore provenance. The operation MUST hold the existing exclusive checked maintenance/startup/lifecycle authority through native restoration, all owned cleanup, directory publication and recovery. Live attachments or surviving native driver cleanup MUST refuse maintenance before durable restore intent. Restore SHALL mint fresh live store/service identity and secrets, preserve the original SQL-origin row and historical namespace/receipts, and exclude credentials, trust grants, transient claims, endpoints and runtime caches from the image. Reusing a namespace string MUST NOT share attachment or driving authority with the source store.

#### Scenario: Active or draining driver

- **WHEN** restore meets live clients or a surviving driver's actual native cleanup barrier
- **THEN** it refuses before native restore/target intent and preserves those clients, the source image and target absence

#### Scenario: Explicit root remap

- **WHEN** an absent target has a different canonical root and the user explicitly authorizes remap
- **THEN** activation records source/target provenance, old session/history namespaces remain usable, and the new service accepts only its own canonical project, fresh instance/generation and checked claims

### Requirement: Compatible staged restore activation

Before target activation, Kuru SHALL retain the captured root and source image unchanged, validate the restored native graph/refs and each applicable supported schema/receipt structure, and apply missing main-schema migrations through the existing isolated migration runner in private staging. It MUST preserve retained historical candidates and same-dataset usage history rather than rewrite them to current schema or reset receipts. Dirty main and usage roots SHALL be prepared only after original-image validation, preserving distinct staged and working snapshots as checked native commits with original heads reachable and original coordinates bound to restore provenance. The final preparation root MUST equal captured WORKING before released migrations; current-schema dirty main/usage also receives this explicit preparation, while other refs/working sets remain unchanged. Ordinary clean-state, authority and usage-content validation MUST remain unchanged rather than accepting future dirty state from initial restore provenance. Activation SHALL occur only after the stage reaches a validated ready state and its owned native children have been reaped. Interrupted stages SHALL use the existing checked staging/recovery discipline; unknown schemas, inconsistent refs/receipts or failed cleanup MUST remain unactivated and preserved. A later ordinary open SHALL validate the new target identity and retained origin before permitting runtime work.

#### Scenario: Dirty main or usage preparation

- **WHEN** a valid supported image has distinct main or usage committed, staged and working roots
- **THEN** private restore appends checked snapshot commits preserving those roots and original ancestry before any required migration, activates through unchanged clean validation, and preserves the original image and other refs without claiming active dirty coordinates are unchanged

#### Scenario: Historical restore and later ordinary open

- **WHEN** a supported older image is restored offline and then opened by an ordinary conversation
- **THEN** required migrations precede activation, the original backup/root remain unchanged, historical candidate/usage data survives, and old sessions resume under the fresh target's checked driver authority

#### Scenario: Migration or cleanup fails

- **WHEN** a staged migration, native validation or checked native shutdown fails
- **THEN** no partial target is selected or activated and the exact unactivated stage remains available under retained ownership/recovery rules

### Requirement: Bounded native operation ownership

Backup/validation/restore SHALL use retained native or SQL ownership and existing operational budgets. Native stdout/stderr MUST drain concurrently through EOF with bounded retained diagnostics, including progress exceeding that retained bound. Operational deadlines MUST NOT establish copy completion, child reap or directory safety. Every error, cancellation and dropped-caller path MUST either await checked cleanup or retain the actual child/session, stage and required lifecycle/maintenance exclusion until cleanup is observed; accepted or uncertain work MUST NOT be replayed from missing evidence. User output MUST exclude credentials and private record bodies.

#### Scenario: Large progress output

- **WHEN** native validation or restoration emits more progress bytes than Kuru retains
- **THEN** both streams continue draining, completion depends on actual native exit and EOF, and user diagnostics remain bounded without including private bodies

#### Scenario: Cleanup exceeds caller budget

- **WHEN** a deadline or cleanup refusal leaves native work unsettled
- **THEN** Kuru reports uncertainty and retains concrete ownership/exclusion rather than releasing a live stage or treating elapsed time as completion
