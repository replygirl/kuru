## 1. Independent quality checks

- [x] 1.1 Split mise/hk check scheduling and remove duplicate ordinary suites before coverage; verify actual concurrent independent checks and correct failure propagation while retaining instrumented fixtures and 90% gates.
- [x] 1.2 Fan out CI and exact-SHA release validation into reusable granular jobs; verify actionlint, every required aggregate dependency and unchanged version/publication/Pages ordering.
- [ ] 1.3 Update canonical contributor instructions and command documentation; verify documented task names, managed drift and repository invariants.
- [ ] 1.4 Run the new workflow on GitHub's native runners, record observed jobs and remaining application failures separately, validate and archive this CI change after its scheduling acceptance passes.
