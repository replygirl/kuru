# Spec Delta

## ADDED Requirements

### Requirement: Doctor and local-data guidance

Curated public documentation SHALL explain `kuru doctor`'s local scope, separate authentication routes, fixed report/exit meanings, and limits. It SHALL provide actionable troubleshooting, FAQ, data-location and privacy, bundled-download-size, and threat-model guidance matching current behavior. It MUST describe data as non-expiring unless explicitly removed, distinguish selected-note forgetting from project purge and secure erasure, identify the bundled engine as target-dependent and included in the application, and state that shell commands have process authority and are not an operating-system sandbox. It MUST NOT promise recovery commands or integrity guarantees the implementation does not provide.

#### Scenario: User interprets a doctor result
- **WHEN** a user follows a doctor status or exit code to troubleshooting guidance
- **THEN** the relevant page explains what was checked, what remained unverified, and an available safe next step without implying network access, repair, or authority activation

#### Scenario: User evaluates data location and retention
- **WHEN** a user reads the data and privacy guidance
- **THEN** they can locate current project data and distinguish ordinary retention, selected-note forgetting, project purge, backups, and the limits of secure deletion

#### Scenario: User evaluates installation size and shell authority
- **WHEN** a user reads installation or threat-model guidance
- **THEN** they understand that a target-specific compressed engine archive is included in the app download, that its size is not the full installer size, and that explicit shell execution is process authority rather than a sandbox
