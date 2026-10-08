# Tasks

## 1. Preserve owned cleanup order

- [x] 1.1 Retain the returned store until the existing bounded proxy session-end
      observation and proxy shutdown finish; preserve cause and quiescence checks.
- [x] 1.2 Run the existing successful lost-commit and deliberate early-error
      acceptance cases through the memory package task, then relevant static checks.

## Observed evidence

PR #263 CI37753398260, Ubuntu coverage partition 7 job113232200655, failed the
existing early-error case at the proxy shutdown assertion. Source review shows
cleanup closes the returned store before the proxy's independent session-end
query settles; the successful path already awaits that observation. The original
proxy SQL/socket error is not included in the panic and remains unobserved.

After correction, the memory package's native task ran both named cases: 2 passed,
0 failed, 0 ignored. Both preserve the existing assertion and error contracts.
The package's all-target/all-feature Clippy task passed with warnings denied.
Independent source review confirmed the observation → proxy retirement → store
closure order. Full PR CI and exact-main acceptance remain pending; no CI pass
is claimed from these local checks.
