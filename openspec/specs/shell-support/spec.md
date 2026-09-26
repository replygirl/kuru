# shell-support Specification

## Purpose
Define executable-derived shell completions and manual output, their exact native release envelopes, and the checks that keep installed support paired with its executable.

## Requirements

### Requirement: CLI-derived shell and manual output

Kuru SHALL generate Bash, Zsh, Fish and PowerShell completion content and a man page from its authoritative current Clap command definition. Generation MUST remain deterministic for a given executable version and MUST run without project configuration, workspace trust approval, memory ownership, provider activity, tool startup or network access. The generated man page SHALL describe the current nested commands and options rather than a separate hand-maintained list. Generated completion scripts SHALL use Kuru's pure embedded answer endpoint by default and MAY explicitly use a separately installed completion-answering executable instead. The default mode MUST NOT require installing that separate executable.

#### Scenario: Pure output without a project
- **WHEN** a release executable generates each supported shell completion or its man page from a directory with no Kuru project or private state
- **THEN** it emits bounded current-CLI content to stdout without creating state, requesting authority or contacting a provider.

#### Scenario: Command changes
- **WHEN** a CLI command or option changes in the executable used to generate a release
- **THEN** the generated support output reflects that executable's command tree without editing a separate template.

#### Scenario: Self-hosted completion answer
- **WHEN** a generated Bash, Zsh, Fish or PowerShell completion script asks Kuru for candidates
- **THEN** the hidden pure endpoint returns candidates in that shell's protocol without loading project state, and an explicit external-executable script remains available.

### Requirement: Exact shell-support archive content

Each native release target SHALL pair its executable archive with one bounded target-specific Unix tar.gz or Windows ZIP shell-support envelope containing exactly five regular files: `completions/kuru.bash`, `completions/_kuru`, `completions/kuru.fish`, `completions/kuru.ps1` and `man/kuru.1`. Each envelope MUST reject missing, extra, duplicated, linked, traversing, oversized or corrupt members. Its uncompressed files SHALL match generation from its own selected target executable; different targets need not produce equal bytes.

#### Scenario: Target-paired output agreement
- **WHEN** the release candidate collects generated support files from its supported native builds
- **THEN** it rejects a sidecar that differs from its paired target executable and publishes exactly one checked five-file envelope per target without requiring other targets' bytes to match.

#### Scenario: Unsafe support member
- **WHEN** a support envelope contains an unexpected or unsafe member, or fails its release checksum
- **THEN** installation refuses it before executable replacement or support publication.
