## 1. Selection and continuity

- [x] 1.1 Add a regression through the production selection path proving unchanged equal activations do not rotate with turn count; observe failure before the correction and success afterward.
- [x] 1.2 Implement stable maximum-activation selection with previous-completed-speaker tie preference, preserving existing target and focus behavior; verify all decisive cases and reasons.
- [x] 1.3 Persist the optional continuity value only with completed session state; verify legacy records, real Dolt close/reopen and failure before completion publication.

## 2. Documentation and verification

- [x] 2.1 Document the concrete selection rule and additive session/event metadata without introducing new framework policy or commands; run documentation checks.
- [ ] 2.2 Run scoped format, lint, typecheck and combined coverage, recording native platform evidence separately; complete strict validation and archive after required checks pass.
