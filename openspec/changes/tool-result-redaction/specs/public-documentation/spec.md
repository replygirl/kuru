## ADDED Requirements

### Requirement: Tool-result projection reference

The curated documentation SHALL describe recognizable-secret projection for
successful and failed tool results, show the exact visible marker, enumerate the
finite detector categories and local heuristic floors, and state the known
false-positive and false-negative limits. It MUST explain that the projection
does not inspect credential stores or enumerate environment values, does not
modify tool inputs, writes or prior history, and is not a byte-exact backup or a
guarantee that arbitrary secrets are private.

#### Scenario: User interprets a projected result

- **WHEN** a user sees `[REDACTED:recognized-secret]` in file, shell or MCP output
- **THEN** the tools reference explains which returned projection changed, which original data was preserved and why unknown or transformed secrets may remain visible
