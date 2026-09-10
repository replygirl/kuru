## ADDED Requirements

### Requirement: Package-owned memory runtime verification

The memory package SHALL own runtime provisioning and real Dolt integration
fixtures through native mise tasks. Required tests MUST fail when Dolt cannot be
provisioned or started. CI SHALL exercise the supported native release platforms;
the existing workspace coverage threshold and release archive contract SHALL remain.

#### Scenario: Missing test runtime
- **WHEN** a required integration check cannot obtain its pinned Dolt executable
- **THEN** the check fails with actionable diagnostics instead of skipping or substituting SQLite.
