## MODIFIED Requirements

### Requirement: Source and mise installation

The repository SHALL document mise GitHub binary installation and compiler-free
platform shell installation before source installation. Windows instructions
SHALL use native PowerShell and its matching executable archive. Source
installation SHALL use the pinned Rust toolchain through prepared app mise
tasks, include the target's embedded runtime and support an explicit destination.
Mise instructions SHALL distinguish installation from activation and explain
exact version selection.

#### Scenario: Direct mise installation
- **WHEN** a user selects a published Kuru version through mise's GitHub backend
- **THEN** mise installs the matching native executable without a checkout or compiler and can activate it for invocation.

#### Scenario: Candidate mise installation before publication
- **WHEN** native acceptance routes the real mise GitHub backend through supported URL replacements to simulated release metadata and the exact staged Windows ZIP selected for final publication
- **THEN** native selection, checksum enforcement, installation, activation, and persistent offline runtime behavior are verified without live fallback or a public download, while the package-owned published verifier remains available as a separate post-publication diagnostic.

#### Scenario: Source installation
- **WHEN** the source installer runs with an explicit writable destination
- **THEN** the resulting executable reports its version and runs the offline demo with an initially empty runtime cache.

#### Scenario: Native PowerShell installation
- **WHEN** the Windows bootstrap runs in stock PowerShell with a supported published version
- **THEN** it installs the verified native executable without Bash, a compiler, a checkout or a separate Dolt installation.

### Requirement: Native Windows release artifact

The release workflow SHALL build `x86_64-pc-windows-msvc` from the same prepared
version commit as the existing four native targets and include its ZIP and
checksum in the complete staged candidate. The ZIP SHALL contain exactly the
flat regular members `kuru.exe`, `LICENSE` and `README.md`; the executable SHALL
include its verified full Dolt bundle. Target selection, expected executable
names and formats SHALL have one authoritative catalog. The release SHALL retain
strategy-only dispatch, conventional-commit version selection, automatic
exact-commit recovery, immutable publication, and documentation deployment from
the selected commit before the sole final public-release job.

#### Scenario: Windows artifact is missing or invalid
- **WHEN** any required Windows package or native packaged-runtime check fails
- **THEN** candidate acceptance fails, final Pages deployment does not run, and no partial successful release is made public.

#### Scenario: Release run resumes
- **WHEN** an interrupted release is rerun after some target artifacts were prepared
- **THEN** recovery uses the existing planned version commit, verifies the full five-target candidate inventory and does not create another bump or overwrite published assets.
