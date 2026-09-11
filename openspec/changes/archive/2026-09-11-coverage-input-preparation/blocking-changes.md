# Dependencies

## Blocked by

None.

## Soft-blocked by

None.

## Scope review

Reviewed active change names and the overlapping native-windows and dependency
refresh proposals. This CI change uses the existing bundle-preparation task and
instrumented supervisor selection; it introduces no native API, dependency pin,
provider bundle or product behavior. Native-windows acceptance remains tracked
independently on its actual candidate. The current hosted run must finish before
the next push so this preparation change does not cancel its evidence collection.

The provider-client bundle remains blocked by native-windows and is outside
this change. The archived parallel-quality-gates record supplies the existing
concurrent job and hook topology, which this change preserves.
