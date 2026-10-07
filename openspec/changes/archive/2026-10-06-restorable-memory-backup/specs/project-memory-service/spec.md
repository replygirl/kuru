# Spec Delta

## ADDED Requirements

### Requirement: Owner-served native backup

The authenticated project memory service SHALL serve one typed backup request through a dedicated attached exchange, checking canonical project/store/generation authority and request bounds before starting native capture. Accepted work MUST retain owner lifetime while ordinary claimed writers remain admitted. Backup MUST NOT expose arbitrary SQL, acquire a project maintenance permit, transfer candidate/driver capabilities or hold the ordinary mutation guard through copying or validation. Client EOF SHALL withdraw unpublished destination publication authority and settle accepted native work through its retained cleanup boundary; a reply lost after checked publication remains an uncertain caller outcome rather than authorizing replay. Protocol version, operation classification and exact wire pin MUST advance together.

#### Scenario: Concurrent attached writers

- **WHEN** a backup client captures while two ordinary clients drive distinct sessions
- **THEN** their authenticated writes continue through the same owner, whose accepted backup keeps it alive until native capture/validation cleanup settles

#### Scenario: Backup attachment disappears

- **WHEN** the dedicated backup connection reaches EOF during accepted work
- **THEN** the owner settles or retains its exact native operation and stage without deleting history, retiring a peer or publishing from cancelled authority
