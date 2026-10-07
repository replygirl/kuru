# Verification

## 1. Explicit catalog avoids memory activation [critical]

- [x] 1.1 @regression (agent) invoke an explicit provider/model catalog with a deliberately present isolated legacy sentinel and unapproved memory executable -> 2026-10-06 owning app test with exact explicit_models_skip_legacy_memory_and_unrelated_authority selection failed before the fix (exit 101, memory-authority refusal); corrected explicit_models selection passed 3/3 with unchanged sentinel and absent memory, tools and trust paths
- [x] 1.2 @integration (agent) invoke the explicit provider catalog against an isolated HTTP fixture with a synthetic API key -> corrected owning explicit_models selection exited 0; one numeric-loopback catalog request preserved future-catalog-model and advertised future-capability=true with no memory or engine files
- [x] 1.3 @e2e (agent) observe the fresh CLI process catalog output and terminal child exit in the isolated HTTP fixture -> explicit_models_http_catalog_preserves_capabilities_without_memory observed successful CLI exit and parsed JSON with the fixture model/capability; all three selected cases passed, all unrelated cases were filtered

## 2. Relevant authority and compatibility [critical]

- [x] 2.1 @integration (agent) invoke explicit Responses catalog with an unapproved automatic endpoint -> explicit_models_preserve_responses_route_review_before_activation passed; refusal named only Responses route and left sentinel unchanged without memory/tools/trust creation
- [x] 2.2 @regression (agent) invoke default Models with unapproved memory authority and a legacy sentinel -> explicit_models_skip_legacy_memory_and_unrelated_authority passed; both default Models and provider-only Models retained the memory-executable authority refusal
- [x] 2.3 @integration (agent) run owning static, documentation and managed-artifact checks -> 2026-10-06 owning app host lint, Windows-target lint and all-target/all-feature typecheck exited 0; Rust format and cospec managed checks exited 0; docs:check exited 0 including build, lint, formatting and public artifact/link/anchor checks; strict fix validation exited 0 with 0 errors and 0 warnings

## Proof limits

The HTTP fixture covers the explicit Responses route. Ordinary native ChatGPT
Models has no injected endpoint hook: KURU_TEST_CHATGPT_BASE applies only to
canary. An initial fixture assumption therefore failed with authentication HTTP
401 using synthetic credentials; it was replaced with the numeric-loopback
fixture above. No real account catalog availability or inference pass is claimed.
Native Windows behavior and full workspace coverage remain CI checks; this local
change runs three focused cases on macOS and host/Windows static checks.
