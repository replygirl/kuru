## ADDED Requirements

### Requirement: Verified shell-support installation with legacy core compatibility

Current direct installers and the native updater SHALL verify and stage the selected release's target-paired shell-support envelope within the explicit installation root before changing the installed executable. They SHALL publish only its exact versioned support directory before the binary, then update a stable owned Unix man file by checked regular-file publication only after confirmed executable replacement. They SHALL preserve the existing executable replacement receipt and report failed, uncertain or partial support publication without claiming a complete new installation. They MUST NOT modify shell profiles or unselected filesystem roots. The existing three-member platform executable archive and its strict validation SHALL remain unchanged, so previously published clients can still upgrade the executable. New clients SHALL parse one bounded shell-support-format marker in the core README only after verifying the core archive; a verified unmarked historical README retains the legacy executable-only path, while a marked release requires its selected target's paired sidecar and malformed, duplicate or unsupported markers are rejected. Current release packaging and verification MUST require the supported marker in every new core archive.

#### Scenario: Support validation fails before binary publication
- **WHEN** the selected support envelope is absent where required, corrupt, oversized, or contains an unsafe member
- **THEN** the installer leaves the existing executable and support directories unchanged and reports the failed installation.

#### Scenario: Executable publication fails after support staging
- **WHEN** support files were verified and published but existing executable replacement fails or remains uncertain
- **THEN** the executable outcome follows its existing receipt, only exactly owned unreferenced support files may be cleaned, and the installer never reports a complete new installation.

#### Scenario: Stable man publication fails after binary success
- **WHEN** the new executable is confirmed but checked publication of the selected Unix `share/man/man1/kuru.1` fails
- **THEN** the installer reports a partial result and an explicit retry revalidates the executable and versioned snapshot before repairing that file, without claiming cross-file atomicity.

#### Scenario: A marked new release omits support assets
- **WHEN** a selected core archive has the supported README marker but its checksum manifest omits its target-paired shell-support envelope
- **THEN** the installer rejects the release before changing the installed executable rather than treating it as legacy.

#### Scenario: A new core has an invalid capability marker
- **WHEN** a release candidate's README lacks the required new-format marker, or a verified core has a malformed, duplicated or unsupported marker
- **THEN** release assembly or installation refuses it without weakening the three-member core archive inventory.

#### Scenario: Upgrade from an older strict reader
- **WHEN** an already-published v0.4.1 or v0.4.2 updater selects a new release
- **THEN** its unchanged three-member core archive remains acceptable; missing shell assets are not misrepresented as installed, and a current verified installer or the new binary's pure local generation offers explicit repair.

#### Scenario: New direct installation
- **WHEN** a current Unix or Windows bootstrap installs a release with support envelopes into an explicit destination
- **THEN** the matching verified versioned support files and executable are present, and a cold offline Kuru conversation still works without a separate engine download.
