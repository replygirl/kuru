# Verification

## 1. Login reveals current models and preserves authentication [critical]

- [x] 1.1 @integration (agent) synthetic native login followed by real HTTP catalog discovery -> browser/device success and failure matrix passed in the 329-test connector suite; persisted native session precedes exactly one GET /models?client_version=0.154.0, no inference or API-key fallback
- [x] 1.2 @integration (agent) failed catalog after synthetic login -> session/generation retained in real HTTP fixture; all five app authentication tests passed, including success before lazy catalog polling, safe listing failure, no polling after failed login, and escaped output

## 2. New models and future catalogs remain usable [critical]

- [x] 2.1 @integration (agent) real HTTP catalog fixtures with changing IDs and unknown efforts -> native and Responses successive-listing fixtures passed; future-model-2099 and unknown native efforts/capabilities survive, no completion sent
- [x] 2.2 @eval (agent) exact route metadata enrichment contract tests -> all 104 core tests passed, including both exact new IDs, documented limits/prices/cache tiers, labelled subscription prices, live precedence, no custom-route inheritance, and absent undocumented tokenizer mappings
- [x] 2.3 @e2e (agent) Kuru native `models` command using existing Kuru login -> October 8 public command succeeded without inference; seven models advertised: gpt-6-astra, gpt-reserve, gpt-5.6-sol, gpt-5.6-terra, gpt-5.6-luna, gpt-5.5, codex-auto-review. Requested new IDs were not advertised by this account/service response; no availability or paid inference claim. Advertised ultra effort survives. No credential store inspected.

## 3. Repository checks

- [x] 3.1 @regression (agent) affected package tests, formatting, lint, typecheck, docs and normal hooks -> core 104/104, connectors 329/329, app authentication 5/5; public docs build/content/link checks passed; normal hk pre-push checks passed all seven categories after correcting one fixture cmp_owned lint. No paid inference. The known invalid default Dolt test cache was preserved; package-owned preparation used the existing healthy isolated cache.
