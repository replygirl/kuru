# Troubleshooting

Start with the bounded local report:

```sh
kuru doctor
kuru doctor --json
```

The command reports configuration and workspace-trust state, the selected authentication route, current-project memory, and the embedded Dolt archive. It does not contact an inference provider, refresh credentials, run tools, start or repair memory, or extract the engine. For memory, it may make one bounded read-only check of an already-published compatible local owner. If the project is cold, deeper SQL health is reported as unverified.

## Report states and exit codes

JSON contains a schema version, exit code, and fixed check IDs, states, conditions, and actions. It omits credential values, account identifiers, configuration values, private paths, memory rows, and provider error text. Human output describes the same fixed observations.

| Exit code | Meaning                                                                                                                                                                                                                                    |
| --------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `0`       | No actionable local problem was found. Unselected routes and fresh project memory can be informational.                                                                                                                                    |
| `1`       | The workspace or local data directory could not be resolved for the command.                                                                                                                                                               |
| `2`       | At least one local check is unverified, for example because required workspace authority is not approved or no running memory owner can be checked.                                                                                        |
| `3`       | At least one actionable local problem was found, such as invalid configuration, a missing key for the selected Responses route, a signed-out selected ChatGPT route, invalid memory activation metadata, or a mismatched embedded archive. |

The report does not establish provider availability, API-key validity, cold database integrity, or whole-executable authenticity. A non-empty Responses environment variable is a local presence check only.

## Common results

| Result                                                                      | Safe next step                                                                                                                                                                             |
| --------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| ChatGPT subscription is signed out while `codex` is selected                | Run `kuru login`, or use the documented device sign-in flow.                                                                                                                               |
| Responses route is selected but its environment variable is absent or empty | Configure the selected API-key environment variable; Kuru does not switch to the subscription route.                                                                                       |
| Responses route awaits workspace review                                     | Review the exact manifest with `kuru trust status`; configuration review does not test the key or contact a provider.                                                                      |
| Configuration is invalid                                                    | Check the user and project configuration layers in [Configuration](../reference/configuration). Doctor output never echoes the invalid value.                                              |
| Project memory is absent                                                    | Use the project normally when you want Kuru to activate memory. Doctor leaves fresh projects untouched.                                                                                    |
| Memory activation metadata is invalid                                       | Preserve the project data directory and consult [Memory](../concepts/memory) before making changes. Doctor does not repair or recreate it.                                                 |
| Project structure is valid but cold database health is unchecked            | Open the project normally when you are ready to use it. The doctor does not start Dolt or perform a cold SQL check.                                                                        |
| Embedded archive does not match its target catalog                          | Reinstall from a verified [Kuru release](https://github.com/replygirl/kuru/releases). The check covers the embedded archive length and digest, not a signature of the complete executable. |

For installation failures, see [Installation & updates](./installation). For memory locations, retention, and deletion boundaries, see [Data and privacy](../concepts/data-privacy).
