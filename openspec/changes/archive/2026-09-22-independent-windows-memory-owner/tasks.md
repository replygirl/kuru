## 1. Windows service lifetime

- [x] 1.1 Add the service-specific safe Windows independent launch mode and keep existing modes unchanged; author native parent Job allow/deny fixtures.
- [x] 1.2 Select the independent mode only in the memory service starter and surface a precise containment error on denied breakaway.

## 2. Native regression

- [x] 2.1 Add a real starter-exit/surviving-client Windows fixture that fails under inherited kill-on-close Job behavior and passes with independent launch.
- [x] 2.2 Keep existing Windows process and memory lifecycle fixtures in the hosted native CI path, and record that Windows execution remains pending until the exact-head PR run.

## 3. Review and archive

- [x] 3.1 Run local package checks, strict Cospec validation and independent source review; update verification.md with exact results.
- [x] 3.2 Archive this focused fix before the final foundation branch commit, then hand delivery the new exact head for normal hook, PR and hosted checks.
