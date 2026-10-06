# Data and privacy

Kuru stores project memory and its ChatGPT sign-in separately beneath its local data directory. On macOS and Linux, the default is `~/.local/share/kuru` unless `XDG_DATA_HOME` is set. On Windows it is `%LOCALAPPDATA%\kuru`, falling back to `%USERPROFILE%\AppData\Local\kuru` when needed. `--data-dir` or `KURU_DATA_DIR` selects another location. Keep this directory private and outside the project tool root.

Each canonical project has its own managed memory beneath `memory/<project-hash>/`. The bundled Dolt runtime cache is under `tools/dolt` by default. Kuru's ChatGPT credentials are stored in its private `auth/openai` directory; this command-line integration does not import another application's credentials. The Responses route reads an API key from its configured environment variable and does not copy the key into this report.

Chat and notes do not expire automatically. `kuru memory forget` removes one selected active note while retaining older revisions. `kuru memory purge --yes` removes the selected project's managed current memory and revisions after confirmation. It retains original/shared legacy input, exports, backups, other projects, and the engine cache. Neither operation promises secure erasure from physical storage or copies outside Kuru's managed project store.

The doctor never prints credentials, account identifiers, configuration values, private paths, or memory rows. It does not contact a model provider or refresh sign-in. A non-empty Responses environment variable is only a local presence observation, not proof that the key is valid or that the service is available. A memory result from a live owner is a bounded read-only status check; cold SQL health may remain unverified.

Kuru stores local chat and memory as application data. Do not assume the data directory is encrypted at rest. Protect it with the operating system account and storage controls you rely on for private records. When a provider request is made, the request content selected for that interaction is sent to the configured provider under its service terms; local diagnostic commands do not make such a request.

See [Parts, relationships & memory](./memory) for project histories, exports, and deletion controls, and [Configuration](../reference/configuration) for precedence and route settings.
