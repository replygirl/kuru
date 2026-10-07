# Design

## Context

The strict source scanner treats every quoted `-m` as a destructive branch flag;
the new native restore uses that spelling for a different Dolt procedure. The
preferences fixture also crosses a canonical-project boundary through one view.

## Decisions

Use the existing long commit-message spelling and retain the scanner unchanged.
Keep the bound-store refusal and exercise default preferences through the other
project's own view, explicitly closing both owners.

## Integration contract

`CALL DOLT_COMMIT` retains the same query, argument order and author. Its staged
message alias changes to `--message`; working `-Am` and captured root validation
are untouched. Memory project binding precedes historical namespace lookup;
another project's defaults never authorize reuse of the first project's view.
No protocol, schema, API, route, dependency or installation contract changes.

## Risks / Trade-offs

The existing dirty remapped restore verifies actual staged/working root equality.
The existing preferences flow verifies refusal, separate defaults and retained
reopen/resume behavior. Native platform execution remains fresh full CI evidence.
