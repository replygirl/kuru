## 1. Retained private owner transition [critical]

- [~] 1.1 @regression (agent) on hosted Windows with distinct `TokenOwner` and `TokenUser`, copy an ordinary source access policy to a retained private stage -> defer: pre-fix job `107047743474` proved the post-assignment validator observed a distinct supported owner without effective OWNER RIGHTS suppression; the corrected exact owner/DACL/private-publication behavior requires Delivery's hosted Windows run
- [~] 1.2 @regression (agent) assign an unsupported owner or present a broad/null DACL -> defer: source preserves the pre-mutation unsupported-owner rejection and strict pre-repair stage validation; native unchanged-bytes/identity execution requires Delivery's hosted Windows run
- [~] 1.3 @regression (agent) mutate fixture ACLs through the retained exact object -> defer: pre-fix job `107047743474` failed four cases at fixture `SetSecurityInfo` with error 5; corrected weak/null/source-change execution requires Delivery's hosted Windows run

## 2. Static and cross-target checks

- [x] 2.1 @integration (agent) compile all `kuru-platform` targets for Windows MSVC -> `cargo check -p kuru-platform --all-targets --target x86_64-pc-windows-msvc` passed; one pre-existing `filesystem.rs` unused-import warning remains
- [x] 2.2 @equivalence (agent) run repository formatting and diff checks -> `cargo fmt --all -- --check`, `git diff --check`, strict Cospec validation, and the apply gate passed
