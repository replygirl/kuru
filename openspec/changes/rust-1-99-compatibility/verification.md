# Verification

## 1. Supported atomic API [critical]

- [ ] 1.1 @regression (agent) run warnings-denied Rust 1.99 lint at the unchanged atomic admission call -> before: deprecated fetch_update rejected at parallel.rs:152 in tmp/dependency-update/static-checks.log; after: the same compiler gate passes with try_update
- [ ] 1.2 @equivalence (agent) existing admission-limit and activity-write failure tests exercise atomic updates -> handle caps, overflow refusal, released permits and failure counter behavior retain their existing expectations
- [ ] 1.3 @integration (agent) host and Windows static gates compile the corrected call sites -> both warnings-denied target checks pass
