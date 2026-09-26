## ADDED Requirements

### Requirement: Speaker lifecycle observation preserves mode authority

After the selected mode policy returns and the runtime validates an eligible speaker, Kuru MAY run the configured `speaker_selected` hook chain with a bounded projection of that selection. Hooks MAY observe or stop dispatch only. They MUST NOT replace the speaker, alter eligibility or reason, request another selection, mutate topology, or grant provider or tool authority. Hook output MUST be validated before any speaker event publication or provider request.

#### Scenario: Speaker dispatch is stopped

- **WHEN** a speaker hook validly stops an otherwise eligible selection
- **THEN** Kuru reports the stopped hook outcome and performs no provider request without selecting a substitute speaker

#### Scenario: Mode-invalid hook result

- **WHEN** a hook attempts to replace the selected speaker or reason
- **THEN** runtime validation rejects the result before speaker publication or dispatch and preserves the mode policy and topology
