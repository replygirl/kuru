## MODIFIED Requirements

### Requirement: Native Windows release artifact

The release workflow SHALL build `x86_64-pc-windows-msvc` from the same prepared
version commit as the other native targets in the release catalog and include
its ZIP and checksum in the complete staged candidate. The ZIP SHALL contain
exactly the flat regular members `kuru.exe`, `LICENSE` and `README.md`; the
executable SHALL include its verified full Dolt bundle. Target selection,
expected executable names and formats SHALL have one authoritative catalog. The
release SHALL retain strategy-only dispatch, conventional-commit version
selection, automatic exact-commit recovery, immutable publication, and
documentation deployment from the selected commit before the sole final
public-release job.

#### Scenario: Windows artifact is missing or invalid
- **WHEN** any required Windows package or native packaged-runtime check fails
- **THEN** candidate acceptance fails, final Pages deployment does not run, and no partial successful release is made public.

#### Scenario: Release run resumes
- **WHEN** an interrupted release is rerun after some target artifacts were prepared
- **THEN** recovery uses the existing planned version commit, verifies one core and one paired support archive for every catalog target in the candidate inventory and does not create another bump or overwrite published assets.

## ADDED Requirements

### Requirement: Intel macOS installation refusal

The release catalog SHALL contain `aarch64-apple-darwin`,
`aarch64-unknown-linux-gnu`, `x86_64-unknown-linux-gnu` and
`x86_64-pc-windows-msvc`, and SHALL NOT contain `x86_64-apple-darwin`. The
shell bootstrap MUST refuse an Intel Mac, whether detected from a `Darwin`
`x86_64` host or selected with `--target x86_64-apple-darwin`, before any
network request, with a non-zero exit and the message
`kuru: Intel Macs (x86_64-apple-darwin) are no longer supported; v0.9.0 was the last release supporting them`.
The refusal MUST leave an existing destination executable unchanged. Published
releases up to v0.9.0 SHALL keep their `x86_64-apple-darwin` assets, and the
installation documentation SHALL direct Intel Mac users to the v0.9.0 tag's own
bootstrap with an explicit `--version 0.9.0`.

#### Scenario: Intel Mac host runs the current bootstrap
- **WHEN** the shell bootstrap runs on a host whose `uname` reports `Darwin` and `x86_64`
- **THEN** it exits non-zero with the v0.9.0 refusal message, makes no download request and leaves the destination unchanged

#### Scenario: Intel target is selected explicitly
- **WHEN** the shell bootstrap is invoked with `--target x86_64-apple-darwin` on any host
- **THEN** it exits non-zero with the same refusal message before any download

#### Scenario: Release catalog excludes Intel macOS
- **WHEN** the release catalog is queried for `x86_64-apple-darwin` or for the macOS `x86_64` platform
- **THEN** no target is found, and release assembly expects no Intel macOS archive
