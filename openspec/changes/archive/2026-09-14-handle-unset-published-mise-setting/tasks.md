## 1. Optional mise setting

- [x] 1.1 In the delivery-owned published-Windows verifier, query scoped `url_replacements` settings as JSON, accept the absent field as an empty map, validate the expected map shape, and preserve failures for insecure replacements, command execution, timeout, JSON, and schema errors
- [x] 1.2 Add a real isolated-mise unset-setting preflight plus configured-field and command-failure coverage, then pass focused verifier tests, delivery formatting and typecheck, strict Cospec validation, and diff checks

## Evidence

- `mise run //packages/kuru-delivery:test -- optional_url_replacements_require_a_json_object_of_safe_strings` passed 1/1, including absent, configured, malformed, unknown-field, non-string, and insecure replacement cases.
- `mise run //:format:rust`, `mise run //packages/kuru-delivery:typecheck`, and `git diff --check` passed on the frozen source.
- Independent source review cleared the optional wrapper, genuine command-failure path, and retained URL-replacement policy.
- The actual isolated Windows mise preflight and nonzero-command regression are implemented; native execution remains pending in required CI.
