# Dependencies

## Blocked by

None.

## Soft-blocked by

None.

## Coordination

The active `native-windows` and `bundled-provider-client` changes also touch
shared manifests. Their names and affected surfaces were checked. This chore
refreshes the already integrated baseline and supplies no missing runtime
implementation; serialize shared-file edits and leave the Codex pin with its
provider change. Native Windows acceptance remains independent and required.
