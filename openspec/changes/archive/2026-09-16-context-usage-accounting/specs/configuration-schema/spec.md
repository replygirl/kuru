## ADDED Requirements

### Requirement: Bounded context configuration parity

P7 SHALL reuse the existing bounded `assumed_context_window_tokens` setting and add only an optional bounded `context_output_reserve_tokens` override for fitting. The native configuration parser and published `configuration.v1.schema.json` SHALL accept the same supported fields, reject unknown keys and out-of-range values, and retain ordinary model and effort values as open strings. An override SHALL not grant tool authority or mutate the frozen provider route; an output reserve greater than the selected effective window SHALL cause a pre-dispatch fit refusal.

#### Scenario: Explicit window override
- **WHEN** a user configures a valid context window for an unfamiliar model
- **THEN** fit uses that explicit bound with its configured provenance and both parser and published schema accept the equivalent configuration.

#### Scenario: Invalid bound
- **WHEN** the window or reserve is outside documented bounds or uses an unsupported key
- **THEN** native validation and published schema both reject the configuration.
