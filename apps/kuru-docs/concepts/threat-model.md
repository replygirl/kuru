# Threat model

Kuru separates project data, workspace files, provider routes, and explicit process authority. The boundaries reduce accidental access and unsafe recovery; they do not promise containment against a process running as the user.

## Workspace trust

Automatically discovered repository configuration and instructions are captured and reviewed under the exact-root workspace-trust policy before their configured authority activates. The doctor can parse configuration and inspect the captured trust manifest, but it does not activate hooks, skills, tools, or instructions. A repository-selected Responses environment-variable name is resolved and looked up only after the applicable authority is approved. The fixed ChatGPT subscription status remains independent of that repository approval.

Workspace approval is not an operating-system sandbox. An approved shell command has the user's normal filesystem and process authority. It may access resources available to that user outside the workspace. State directories are never tool roots, but a tool-root check does not confine shell.

## Local data and providers

The operating system account boundary protects the local data directory. Kuru does not claim encryption at rest or secure deletion. Chat and notes do not expire automatically; exports and backups remain under the user's control. Keep the local data directory private and use the explicit forgetting or confirmed project-purge controls with their documented limits.

The `codex` ChatGPT subscription route and `responses` API-key route remain separate. Kuru does not switch routes after a failure. A doctor status is local: it does not establish that the provider is reachable, that a key is valid, or that a current model is available. Inference requests send their selected content to the configured provider.

## Memory ownership and the bundled engine

Ordinary memory operations use the managed private per-project Dolt service. Doctor performs only bounded structural validation and, when a compatible owner is already published, a read-only local status check. It never elects or starts an owner, provisions or extracts an engine, migrates, or repairs data. A valid cold store can therefore remain unverified until an ordinary memory operation opens it.

Kuru includes a target-specific engine archive and its licenses. The doctor compares the embedded archive's length and digest with its compiled target catalog. It does not attest to the complete executable, publisher identity, filesystem, or platform supply chain. Download checksums detect corruption; choose a release source you trust.
