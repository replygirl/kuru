## ADDED Requirements

### Requirement: Sourced tokenizer eligibility

Kuru SHALL select a local tokenizer only for a catalog-supported route and model whose encoding mapping has recorded primary-source provenance and an exact dependency/asset version. A documented provider family mapping MAY justify the encoding for a real catalog-supported family member but SHALL NOT establish model availability, price, context limit or an encoding for an arbitrary matching string. Unknown IDs, custom Responses bases and models without supported mapping SHALL remain usable with a visibly labelled fallback estimate.

#### Scenario: Supported GPT-5 family model
- **WHEN** a real catalog-supported GPT-5.6 model is selected on a matching native route and the pinned upstream GPT-5 family mapping is validated
- **THEN** its local sizing may use the pinned `o200k_base` tokenizer, labelled as an estimate.

#### Scenario: Unmapped or custom model
- **WHEN** a GPT-6 Astra model without mapping, an unknown model ID, or a custom Responses base is selected
- **THEN** Kuru retains the selected model and effort, uses the conservative fallback for context sizing and does not infer official tokenizer facts from the name.
