## 1. Native task-shell boundary [critical]

- [~] 1.1 @regression (agent) invoke the real `verify:published-windows` mise task on native Windows with one required verifier input deliberately absent -> defer: the regression is implemented with a bounded owned child, but native Windows execution is unavailable locally and remains required hosted acceptance
- [~] 1.2 @runtime (agent) run the corrected pull-request head in the required GitHub `windows-2025` delivery shard -> defer: the native runner executes only after the completed record is archived and pushed and remains a required merge gate
