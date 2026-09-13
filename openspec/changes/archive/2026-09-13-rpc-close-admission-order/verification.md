## 1. Close publication ordering [critical]

- [x] 1.1 @regression (agent) register a waker on a real EOF-backed RPC close reply before sending close and observe admission plus completion at first wake -> isolated old-order run failed with `Some((false, true))`; the corrected run passed with `Some((true, true))`
- [x] 1.2 @integration (agent) close the same real EOF-backed RPC twice after the first reply wakes the caller -> corrected isolated regression passed, including the second public close and one completed peer transcript

## 2. Existing cleanup contract

- [x] 2.1 @integration (agent) run the focused RPC test suite -> isolated `rpc::tests::` run passed 5 tests with 152 filtered out, including protocol bounds, parent-runtime loss retention and double close
- [x] 2.2 @unit (agent) typecheck the connector package -> package-owned connector typecheck passed with the isolated target
