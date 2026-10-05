# Spec Delta

## ADDED Requirements

### Requirement: Attachment-owned immutable state cut handles

The private service SHALL bind each state cut to its attachment, owner generation
and captured live/candidate identity. Its read and close operations MUST use that
exact handle and attachment, carry no mutation receipt and reject foreign,
closed or restarted handles and crossed cursors. Disconnect SHALL release owned
cuts, and retained cuts SHALL prevent idle retirement. Protocol minor and exact
golden pin MUST advance together. Conditional publication SHALL keep its existing
typed stale refusal and exact receipt/recovery classification while validating
the full encoded request against the unchanged service envelope.

#### Scenario: Foreign or stale cut
- **WHEN** another attachment presents a handle, a cursor crosses cuts, or the owner restarts
- **THEN** the request refuses without silently reopening a new revision or granting candidate access

#### Scenario: Explicit close or disconnect
- **WHEN** the owning attachment closes a cut or disconnects
- **THEN** its retained cut resources release and later handle reads refuse while unrelated attachments remain usable

#### Scenario: Exact conditional request envelope
- **WHEN** an escaped conditional payload crosses the complete service request limit
- **THEN** local and remote preflight refuse before mutation or receipt, while the last fitting payload retains ordinary atomic publication
