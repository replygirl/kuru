## ADDED Requirements

### Requirement: Lifecycle-hook documentation states authority and settlement limits

User documentation SHALL describe every hook event, declaration order, typed input/output, supported decision, bound, trust requirement, permission revalidation, annotation visibility, failure and cancellation behavior, and platform support. It SHALL state that hook commands have process authority and are not an OS sandbox; pre hooks cannot override denies or roots; post hooks cannot change settled effects, answers, usage, or records; speaker hooks cannot substitute a peer; annotations may be omitted by context fitting; and no generic plugin runtime is provided.

#### Scenario: User evaluates a repository hook

- **WHEN** a user reviews the hook configuration and workspace-trust documentation
- **THEN** they can determine which command will run, at which event, with what data and limits, which final checks still apply, and how denial, failure, cancellation, or annotation omission appears
