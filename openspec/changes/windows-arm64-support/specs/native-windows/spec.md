## MODIFIED Requirements

### Requirement: Native Windows application

Kuru SHALL support x64 Windows 10 version 1809 or newer and Arm64 Windows 11 or
newer through native MSVC executables for `x86_64-pc-windows-msvc` and
`aarch64-pc-windows-msvc`. Chat, framework selection, sessions, preferences,
memory, dreams, undo, tools, authentication and connectors MUST retain their
existing behavior on both targets without requiring WSL, a Unix shell, a
compiler, an MSVC redistributable installation or a separately managed database.
Shipping builds MUST link the appropriate static CRT for their target without
changing host build-script/proc-macro settings. Fresh bootstrap installation
MUST select the target from the host's native machine architecture, never from
the architecture reported by an emulated shell's or parent process's
environment, and MUST fail closed on any other machine. Update MUST keep the
target of the installed executable, so an x64 executable running under
emulation on Arm64 continues to update to x64; moving such an installation to
Arm64 is outside this requirement. Recovery of an interrupted update MUST accept
the recorded helper's own Windows target even when it differs from the native
machine. A Windows target SHALL be documented as supported only after native
memory, process cleanup, terminal, installation and update checks have passed
on that architecture.

#### Scenario: Fresh native installation
- **WHEN** a user launches the installed executable in an isolated Windows user environment on either supported architecture
- **THEN** version and help work, an offline demo persists and reopens its conversation, and no external Dolt executable or runtime download is required.

#### Scenario: Native executable dependencies
- **WHEN** native CI inspects the shipped Kuru and embedded Dolt PE imports for a Windows target
- **THEN** only operating-system DLL dependencies are present and no separately installed MSVC redistributable is required.

#### Scenario: Bootstrap ignores an x64 architecture environment
- **WHEN** the PowerShell bootstrap runs on an Arm64 machine with `PROCESSOR_ARCHITECTURE` reporting `AMD64`
- **THEN** it determines the native machine through the operating system rather than the environment, selects the `aarch64-pc-windows-msvc` archive and requires an ARM64 PE32+ executable, or fails closed when asked for a target that does not match the native machine.

#### Scenario: Emulated x64 installation updates
- **WHEN** an installed x64 `kuru.exe` running under emulation on an Arm64 machine updates, or the bootstrap recovers its interrupted update
- **THEN** the update installs the x64 target and recovery accepts the recorded x64 helper, without replacing the executable with one of a different machine type.

### Requirement: Required native Windows verification

CI SHALL require native Windows MSVC tests on every supported Windows
architecture, currently x64 and Arm64, for platform, database, runtime, CLI,
connector, terminal, installation and update behavior. A missing required
prerequisite MUST fail rather than skip. A Windows runner label that the
aggregation gate does not recognize MUST fail the gate rather than fall into a
Unix or x64 branch. Platform-scoped Unix tests SHALL retain their coverage and
have actual Windows counterparts for shared behavior. Native Windows coverage and
its limits MUST be reported honestly per architecture alongside the unchanged
workspace quality threshold, including any test artifact excluded by name on one
architecture and the recorded reason.

#### Scenario: Windows-specific regression
- **WHEN** a native test, required prerequisite or Windows coverage check fails on the candidate commit on either Windows architecture
- **THEN** the required Windows job for that architecture and the aggregate CI gate fail even if every other job passes.

#### Scenario: Unknown Windows runner label
- **WHEN** a native-tests call names a Windows runner label the gate does not list
- **THEN** the gate fails and reports the label instead of accepting Unix-path results for it.

### Requirement: Staged Windows release acceptance

Each release SHALL exercise its exact staged ZIP for every Windows target in the
release catalog on a native runner of that architecture before public GitHub
release promotion. The acceptance check MUST route the real mise GitHub backend
through an isolated loopback fixture serving simulated release metadata and those
exact candidate bytes, start with empty Kuru data and engine caches, run an
offline demo conversation, resume the same session, inspect memory status and
history, list sessions, and verify the extracted embedded engine and licenses
against the checked-out authoritative asset manifest for that target. The
staged check MUST also run the previous-release updater rule for that target:
when a published stable release carries the target's archive, the greatest such
release's own updater MUST install the staged candidate; when none does, the
check MUST record "no predecessor for <target>" with the releases it inspected
and pass. This staged check MUST NOT be claimed as an actual public download; the
package-owned published verifier remains available as a separate
post-publication diagnostic per target.

#### Scenario: Cold staged package persists and reopens
- **WHEN** the ordinary mise installation path consumes the exact staged release ZIP for a Windows target with the demo provider and an empty offline engine cache
- **THEN** it extracts the bundled engine, completes and resumes one durable session, reports the resulting memory state and history, and lists both turns without downloading another engine

#### Scenario: Staged native cleanup is uncertain
- **WHEN** any owned fixture or application process cannot be confirmed stopped before isolated state cleanup
- **THEN** native acceptance fails and public release promotion remains blocked

#### Scenario: Staged bytes differ from the candidate
- **WHEN** the ZIP served to mise or its checksum differs from the complete candidate artifact selected for publication
- **THEN** acceptance fails rather than substituting rebuilt or public bytes

#### Scenario: First release for a target
- **WHEN** no published stable release lists the target's core archive and no published `SHA256SUMS` names it
- **THEN** staged acceptance prints "no predecessor for <target>" with the inspected versions, skips no other check, and passes

#### Scenario: Listing and manifest disagree
- **WHEN** a published release's asset listing lacks the target's archive but its `SHA256SUMS` names it, or the reverse
- **THEN** staged acceptance fails rather than treating that release as having no predecessor
