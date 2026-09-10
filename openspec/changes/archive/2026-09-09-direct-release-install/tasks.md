## 1. Bootstrap and initial publication

- [x] 1.1 Confirm the initial Release run published v0.1.0 and deployed Pages; record the actual release assets, commit and live site before merging this follow-up. Release 34432603265 passed all eleven jobs at aac594e; all four archives and SHA256SUMS are published, and the live homepage, linked guides and assets match the final Pages artifact.
- [x] 1.2 Add the package-owned compiler-free bootstrap and forward the existing release-install entrypoint; verify actual Bash installation, explicit/latest selection and failure preservation with Rust-driven integration tests.
- [x] 1.3 Include bootstrap tests and ShellCheck in the delivery package's mise tasks; verify package checks discover and run them through existing root aggregation.

## 2. Installation documentation and delivered behavior

- [x] 2.1 Put mise and curl installation before source in README and align both installation guides, release/contributing/security links and docs navigation label; manually verify command syntax, activation/update behavior and absence of visibility/readiness commentary.
- [x] 2.2 Install and exercise actual released assets through the bootstrap and isolated mise environment without a compiler on PATH; record version/demo/update outcomes and any external verification limits. All four published archives passed bootstrap installation; host version/demo/update and isolated mise 2026.9.3 exact-version installation passed. The initial mise credential-helper HOME assumption was corrected only in the probe, with both attempts retained.
- [x] 2.3 Run the full repository gate and archive the completed cospec change before committing; record coverage, docs and hosted platform evidence through the normal PR flow. Final full gate passed in 46.68 seconds at 97.46% line coverage; archive through cospec before the branch commit, and observe hosted checks on that committed branch before merge.
