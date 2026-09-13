## ADDED Requirements

### Requirement: Complete one-shot Unix child pipes

The retained Unix fresh-process-group owner SHALL permit a safe caller to take each configured standard input, output, and error pipe exactly once without exposing the standard child or its numeric process identity. Taking pipes MUST NOT transfer root observation, signal, reap, or post-reap authority away from that owner.

#### Scenario: Session takes all configured pipes

- **WHEN** a connector creates a retained Unix child with piped stdin, stdout, and stderr
- **THEN** it can take each pipe once for asynchronous framing and draining while the original owner remains solely responsible for ordered process-group cleanup
