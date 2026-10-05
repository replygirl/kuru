# Spec Delta

## ADDED Requirements

### Requirement: Confirmed compaction has a reliable private-safe notice

Each confirmed successful automatic or manual per-actor compaction SHALL produce a visible metadata-only notice identifying its actor, exclusive/inclusive covered source sequence bounds, summary identity and retained original records. A candidate checkpoint SHALL be labeled as candidate work without implying live promotion. The notice MUST NOT disclose summary text, private source rows or private reasoning sidecars, and MUST NOT turn a summary into a public transcript message or belief. Automatic and manual notices SHALL describe the same accepted checkpoint truth; manual no-op/refusal behavior and stable identity order SHALL remain intact.

#### Scenario: Automatic checkpoint succeeds before ordinary completion
- **WHEN** threshold compaction commits for a part or relationship before its ordinary request settles
- **THEN** that actor's metadata notice is observable in the TUI and headless stderr without private bodies or any change to stdout's answer/JSON contract.

#### Scenario: Candidate compaction succeeds
- **WHEN** a dream actor accepts a compaction checkpoint on its selected candidate
- **THEN** the visible notice explicitly describes a candidate checkpoint, contains only metadata and retained-source truth, and makes no claim that the candidate was promoted.

### Requirement: Notice acceptance and delivery survive operation settlement

Kuru SHALL retain only the current Harness's compaction attempt and notice metadata until its acceptance and presentation are resolved. An acknowledged checkpoint MUST be registered before a later cancellation check can discard its outcome. An aborted or uncertain attempt MUST be confirmed by its exact stored summary identity on the selected view after existing exact write-fence recovery; a generic reconciliation boolean MUST NOT be treated as proof for a particular compaction. Refused, empty, stale, failed or definitely absent attempts MUST NOT emit successful notices. Existing typed events and operation-settlement drains SHALL deliver confirmed notices once even if live activity delivery lagged or the turn failed/cancelled. This MUST NOT add a durable notice outbox, alter checkpoint/receipt/journal/TurnOutput schemas, or replay inference.

#### Scenario: Cancellation follows accepted checkpoint
- **WHEN** cancellation is observed after the compaction checkpoint was accepted
- **THEN** settlement still displays that exact checkpoint's metadata notice once and preserves every original row and private boundary.

#### Scenario: Accepted reply is lost and its cursor is superseded
- **WHEN** the exact checkpoint commits but its reply is lost or its caller aborts, and a later accepted summary supersedes the current cursor
- **THEN** existing receipt recovery and exact stored identity confirmation establish the earlier checkpoint's notice without another provider request or a duplicate summary row.

#### Scenario: Attempt did not commit
- **WHEN** provider refusal/failure, typed stale publication, or exact recovery proves the attempted checkpoint absent
- **THEN** no successful notice is produced for that attempt and the existing usable summary/history remains unchanged.

#### Scenario: Activity receiver lagged
- **WHEN** the TUI missed the live event before operation settlement
- **THEN** settlement drains the retained accepted metadata into visible presentation and deduplicates it against any already observed live notice.

#### Scenario: Headless Ctrl-C follows checkpoint acceptance
- **WHEN** Ctrl-C interrupts a headless Run after its automatic checkpoint accepted
- **THEN** the existing controlled future is cancelled and awaited before shutdown, its confirmed notice is drained to stderr, and stdout retains the current answer/JSON contract without inventing a completed answer; a result already ready for completion wins over interruption.
