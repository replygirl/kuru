# Dependencies

## Blocked by

- [x] `openai-authentication` — separate native ChatGPT and API-key Responses routes plus owned rotating credentials *(archived 2026-09-11)*
- [x] `provider-response-budgets` — absolute 60-second catalog and 600-second completion budgets plus bounded SSE handling *(archived 2026-09-12)*
- [x] `bounded-provider-diagnostics` — finite rejected-response classification and bounded redacted failed-body reading *(archived 2026-09-12)*

## Soft-blocked by

None.

## Existing foundations

The active terminal, process-observation, dream-instruction, and ordered-memory
changes neither provide nor consume connector retry or auth-rotation behavior.
