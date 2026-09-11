## 1. Keep private preparation outside restored Cargo outputs

- [x] 1.1 Set the reusable native job's bundle-directory override under runner temporary storage and document the cache ownership boundary; verify existing mise preparation and every consumer inherit the same absolute path without changing local defaults or private-object checks. The early GITHUB_ENV step leaves creation to the private preparer; independent review traced all consumers and temporary offline-test override restoration. Actual runner execution is recorded in the next task and verification.
- [x] 1.2 Run relevant workflow/tooling/docs/cospec checks and normal concurrent hooks; record actual required native CI, including a Windows Rust-cache restoration followed by preparation, offline source/build/shipping checks and complete aggregate success, then strictly validate and archive the completed change.

Final integration passed at a1547ed after an actual Windows target-cache restore,
with both preparations and subsequent consumers using the runner-temp mirror.
