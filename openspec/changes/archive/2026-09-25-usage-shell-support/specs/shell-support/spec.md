## MODIFIED Requirements

### Requirement: CLI-derived shell and manual output

Kuru SHALL generate Bash, Zsh, Fish and PowerShell completion content and a man page from its authoritative current Clap command definition through Usage. Generation MUST remain deterministic for a given executable version and MUST run without project configuration, workspace trust approval, memory ownership, provider activity, tool startup or network access. The generated man page SHALL describe the current nested commands and options rather than a separate hand-maintained list. Generated completion scripts SHALL use Kuru's pure embedded Usage-backed answer endpoint by default and MAY explicitly use an external Usage executable instead. The default mode MUST NOT require users to install Usage.

#### Scenario: Pure output without a project
- **WHEN** a release executable generates each supported shell completion or its man page from a directory with no Kuru project or private state
- **THEN** it emits bounded current-CLI content to stdout without creating state, requesting authority or contacting a provider.

#### Scenario: Command changes
- **WHEN** a CLI command or option changes in the executable used to generate a release
- **THEN** the generated support output reflects that executable's command tree without editing a separate template.

#### Scenario: Self-hosted completion answer
- **WHEN** a generated Bash, Zsh, Fish or PowerShell completion script asks Kuru for candidates
- **THEN** the hidden pure endpoint returns Usage's candidates in that shell's protocol without loading project state, and an explicit external-Usage script remains available.
