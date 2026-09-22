## ADDED Requirements

### Requirement: Runtime uses the reviewed instruction projection

The chat harness SHALL receive the immutable composed instruction projection captured and approved for its invocation, including bounded omission notices and excluding resolved import directives and omitted source bodies. It SHALL NOT reread instruction files after construction or reinterpret imports itself.

#### Scenario: Source replacement after capture
- **WHEN** an applicable source is replaced after snapshot capture and before an actor request
- **THEN** that request contains the reviewed captured projection and not bytes from the replacement.
