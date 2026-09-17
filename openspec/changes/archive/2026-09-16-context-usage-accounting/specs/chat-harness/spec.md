## ADDED Requirements

### Requirement: Inspectable session cost and request fit

The TUI SHALL provide `/cost` and an estimated context indicator for the relevant active request, alongside existing model, effort, mode and permission state. It SHALL distinguish actual reported token counts from incomplete components, unknown prices, assumed model windows and subscription API-equivalent estimates; it MUST NOT present an estimate as a bill, remaining subscription quota or known zero. When optional history is omitted for fit, the user SHALL see its actual count without a false claim that persisted history was deleted. Headless output SHALL keep its existing machine format and may expose these facts only through an explicit compatible structured field or command output.

#### Scenario: Partial ledger and assumed window
- **WHEN** `/cost` inspects a resumed pre-ledger session using a model without a verified window or complete price
- **THEN** known usage appears with incomplete history and price labels, and the status identifies the assumed context bound.

#### Scenario: Narrow terminal with concurrent status
- **WHEN** the TUI shows context, cost and permission status in a narrow pane while a request is active
- **THEN** the current selection and operation controls remain usable and no provisional preview or private continuation becomes durable cost text.
