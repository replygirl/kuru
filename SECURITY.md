# Security policy

Kuru runs locally and coordinates model providers, isolated peer memories, tools,
MCP servers and A2A peers. There is no hosted Kuru service. Report issues against
the latest release.

## Reporting

Contact the [maintainer](https://github.com/replygirl) privately to arrange
vulnerability disclosure. Do not post vulnerabilities in public issues or include
live credentials, private conversations or memory databases. Include reproduction steps,
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
