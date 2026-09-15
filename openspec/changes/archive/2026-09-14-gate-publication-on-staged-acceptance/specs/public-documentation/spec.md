## MODIFIED Requirements

### Requirement: Published release verification documentation

Maintainer documentation SHALL describe the same-run automated staged Windows
gate, its relationship to documentation deployment and final public release
promotion, and recovery by rerunning the existing Release workflow. It SHALL
state that the loopback fixture consumes the exact staged Windows ZIP without
claiming an actual public download, and distinguish the separately invokable
published-package verifier. Curated installation documentation SHALL describe
the compiler-free binary verification path without exposing repository-only
evidence.

#### Scenario: Maintainer recovers post-publication verification
- **WHEN** staged Windows acceptance, documentation build, or documentation deployment fails or is interrupted
- **THEN** the release guide directs the maintainer to rerun the failed work in the same Release run and explains that no public release is promoted until every dependency of the final publication job succeeds

#### Scenario: Visitor reads installation guidance
- **WHEN** a visitor selects the mise installation path
- **THEN** the guide accurately distinguishes pre-publication native acceptance of the exact staged ZIP from an actual public download while preserving the compiler-free installation instructions
