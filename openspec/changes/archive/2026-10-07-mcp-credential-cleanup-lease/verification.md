# Verification

## 1. Isolated shared-issuer cleanup [critical]

- [~] 1.1 @regression (agent) execute the shared-issuer fixture natively on Windows -> defer: baseline job 112865645143 failed mcp.rs:2706 with native credential changed; corrected native Windows execution requires fresh PR CI and the historical backend cause remains unproved
- [x] 1.2 @integration (agent) inspect fixture cleanup and vault operation labels -> Root reviewed retained lease, both-alias cleanup, original functional error priority, nonsecret labels and unchanged guards; source hashes f8f552a9 and d4ffa2b0
- [x] 1.3 @integration (agent) execute the existing shared-issuer and affected Vault cases locally -> owning task 71285 EXIT0 shared-issuer 1/1 in 0.68s; task84451 four Vault cases passed and one outer-string assertion failed; corrected exact-root-cause/full-chain-privacy assertion passed task49338 EXIT0 1/1 in 0.01s

## 2. Static and delivery verification

- [x] 2.1 @integration (agent) run scoped host and Windows lint, typecheck, formatting and managed checks -> final host94639, Windows60377, type88263 EXIT0; format79199 and managed13172 EXIT0; final test assertion formatted by owning rust formatter55978 EXIT0; no provider/auth/platform changes
- [~] 2.2 @e2e (agent) observe complete PR and exact-main CI -> defer: corrected commit must be published first; PR256 remains held by required ci-gate and O3 auto remains OFF until merged exact-main full CI succeeds
