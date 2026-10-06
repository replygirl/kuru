# Spec Delta

## ADDED Requirements

### Requirement: Personal update notice preference

Kuru SHALL expose `[update] notice` as a boolean defaulting to false. Repository-origin automatic configuration MUST NOT enable it, even after trust or a later disabling override. User defaults, project-local overrides, explicit configuration and invocation assignments MAY enable it under existing managed constraints. The preference MUST NOT grant provider, tool or installation authority.

#### Scenario: Trusted repository requests network notice
- **WHEN** an automatic ancestor repository configuration enables update.notice
- **THEN** parsing refuses that personal preference before network or configured authority, even when the workspace is trusted

#### Scenario: Personal opt-in and managed constraint
- **WHEN** a user-authority layer enables the notice
- **THEN** ordinary immutable precedence applies and a managed false constraint cannot be bypassed
