# Security policy

Kuru runs locally and coordinates model providers, isolated peer memories, tools,
MCP servers and A2A peers. There is no hosted Kuru service. During initial development,
the current `main` branch is the supported version; after releases begin, report
issues against the latest release.

## Reporting

While this repository is private, authorized collaborators can report issues
privately in [replygirl/kuru](https://github.com/replygirl/kuru/issues) or through
an existing private channel with the maintainer. Do not include live credentials,
private conversations or memory databases in reports.

Before public availability, the maintainer will enable GitHub private vulnerability
reporting. Once enabled, use [a private security advisory](https://github.com/replygirl/kuru/security/advisories/new)
for vulnerabilities instead of a public issue. Include reproduction steps,
affected versions and the impact you observed. Responses depend on maintainer
availability; there is no response-time guarantee.

## Relevant boundaries

Report cross-project or cross-part memory exposure, bypasses of file-tool
capabilities, protocol authentication failures and unsafe installer updates here.
Report vulnerabilities in external providers or dependencies to their maintainers.
Shell execution, when explicitly enabled, uses the user's process permissions;
it is not an operating-system sandbox. See [protocols](docs/protocols.md) and
[configuration](docs/configuration.md) for supported behavior.

The project is provided under the [MIT license](LICENSE).
