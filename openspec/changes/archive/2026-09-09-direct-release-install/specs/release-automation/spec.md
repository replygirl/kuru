## MODIFIED Requirements

### Requirement: Accurate release notes and instructions

Release notes SHALL use supported Communiqué configuration and include initial implementation context for the first release. Documentation SHALL describe required release credentials and direct binary installation, including explicit versions and local release directories, without repository-visibility commentary.

#### Scenario: Initial commit contains the harness
- **WHEN** the first release has no previous version tag
- **THEN** notes generation receives the root commit inventory in addition to later conventional history

#### Scenario: Installation from a downloaded release
- **WHEN** a user has the matching native archive and checksum manifest in a local directory
- **THEN** the documented installer can use that directory and explicit version without a compiler or a network request
