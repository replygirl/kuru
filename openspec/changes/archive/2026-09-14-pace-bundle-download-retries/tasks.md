## 1. Retry pacing

- [x] 1.1 Set the existing production retry pauses to five and fifteen seconds through one private explicit-policy core, retaining every attempt, deadline, error, staging, and publication rule
- [x] 1.2 Keep bounded real-HTTP tests fast with private injected delay schedules and add one production-shaped 500-to-success case that observes the real five-second pause, identical GETs, and verified publication
- [x] 1.3 Replace the timing in `docs/development.md` while preserving its three-attempt, total-deadline, `Retry-After`, integrity, and publication statements

## 2. Verification

- [x] 2.1 Pass focused bundle recovery tests, delivery formatting, all-target/all-feature typecheck, strict Cospec validation, and diff checks. *(Observed: the focused bundle suite passed 13/13 in 6.31 seconds; delivery typecheck, repository formatting, the documentation build/content/link checks, and diff checks passed.)*
- [x] 2.2 Record the two pre-fix Intel 504 failures and preserve post-push required GitHub checks as the merge gate. *(Observed: Intel jobs `104013546597` and `104032117607` ended in HTTP 504 after roughly 13 seconds; the latter identified attempt 3/3 with no `Retry-After`. Required GitHub checks remain pending after archive and push.)*
