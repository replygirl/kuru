## ADDED Requirements

### Requirement: Published release verification documentation

Maintainer documentation SHALL describe the same-run automated published
Windows gate, its bounded receipt, its relationship to final documentation
publication, and recovery by rerunning the existing Release workflow job. Curated
installation documentation SHALL describe published binary verification without
exposing repository-only evidence or suggesting that ordinary installation
requires a compiler.

#### Scenario: Maintainer recovers post-publication verification
- **WHEN** publication succeeded but the published Windows verification job failed or was interrupted
- **THEN** the release guide directs the maintainer to rerun that job in the same Release run and explains that documentation remains blocked until it succeeds

#### Scenario: Visitor reads installation guidance
- **WHEN** a visitor selects the mise installation path
- **THEN** the guide accurately states that the published Windows package is checked through the native same-run release gate while preserving the compiler-free installation instructions
