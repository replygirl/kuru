## MODIFIED Requirements

### Requirement: Native Windows release artifact

The release workflow SHALL build every Windows target in the release catalog,
currently `x86_64-pc-windows-msvc` and `aarch64-pc-windows-msvc`, from the same
prepared version commit as the native Unix targets and include each ZIP and
checksum in the complete staged candidate. Each ZIP SHALL contain exactly the
flat regular members `kuru.exe`, `LICENSE` and `README.md`; the executable SHALL
include its verified full Dolt bundle for that target. Target selection, expected
executable names and formats SHALL have one authoritative catalog, and adding a
Windows target to that catalog SHALL be the same change that adds its release
build, staged acceptance and published verification jobs. The release SHALL
retain strategy-only dispatch, conventional-commit version selection, automatic
exact-commit recovery, immutable publication, and documentation deployment from
the selected commit before the sole final public-release job.

#### Scenario: Windows artifact is missing or invalid
- **WHEN** any required Windows package or native packaged-runtime check fails for any catalog Windows target
- **THEN** candidate acceptance fails, final Pages deployment does not run, and no partial successful release is made public.

#### Scenario: Release run resumes
- **WHEN** an interrupted release is rerun after some target artifacts were prepared
- **THEN** recovery uses the existing planned version commit, verifies the complete catalog candidate inventory and does not create another bump or overwrite published assets.

### Requirement: Package-owned published Windows verifier

The delivery package SHALL provide one native Windows verifier and mise task for
an exact published Kuru version and expected commit, parametrized by the Windows
target it runs on. The verifier MUST use the ordinary
`github:replygirl/kuru@VERSION` backend in isolated mise and user roots, with no
inherited release token, provider credential, proxy, OAuth store, lock, or
endpoint replacement. It MUST independently verify the published tag and commit,
complete release asset inventory, checksum manifest, the selected target's
Windows archive, selected installed executable, embedded Dolt executable, and
bundled licenses, and MUST require that the installed executable's PE machine
matches the target it verifies.

#### Scenario: Ordinary published installation
- **WHEN** the verifier runs with an exact published version, expected commit, checked-out asset manifest, native mise executable and a target matching the runner's native machine
- **THEN** it selects, installs, locates, and executes that version through mise without a custom endpoint, a compiler-built Kuru executable, or a separately installed Dolt

#### Scenario: Publication identity disagrees
- **WHEN** the tag, resolved commit, asset inventory, checksums, archive, installed executable, engine, or licenses disagree with the exact expected release for the selected target
- **THEN** verification fails rather than accepting mise installation alone as provenance evidence

### Requirement: Bounded published verification receipt

Published Windows verification SHALL bound command execution and output, await
owned cleanup, and emit a bounded JSON evidence receipt containing fixed safe
metadata only after isolated state has been removed. The receipt and its uploaded
artifact name MUST identify the verified target so receipts from different
Windows architectures in one release run never collide. It MUST parse Kuru
machine results from stdout without requiring stderr to be empty.

#### Scenario: Verification completes
- **WHEN** every release, installation, runtime, persistence, and cleanup check succeeds
- **THEN** the receipt records exact version, commit identity and target, digests, command milestones, durable session observations, bundled-runtime observations, and confirmed cleanup without raw child output or credentials

#### Scenario: Child emits informational stderr
- **WHEN** an otherwise successful Kuru command emits an informational first-run notice on stderr while retaining its JSON stdout contract
- **THEN** verification accepts the machine result and keeps only bounded failure diagnostics if a later check fails
