## 1. Fresh Git-for-Windows clone validation [critical]

- [x] 1.1 @regression (agent) exercise the advisory local-config validator with `core.symlinks=false` and `core.symlinks=true` -> false was accepted and true rejected by `fresh_owned_setup_has_only_allowlisted_config_and_checked_values`; the retained unknown-key rejection and two related advisory cases also passed, 4/4 in `/private/tmp/kuru-native-advisory-delivery-unit.log`. The preceding PR13 Windows job was the before-fix failure of this validator against a stock fresh clone.
- [~] 1.2 @integration (agent) execute the delivery advisory unit suite on native Windows -> defer: requires hosted Windows execution after this correction; the preceding PR13 Windows job supplied the before-fix rejection.

## 2. Native path assertions

- [x] 2.1 @integration (agent) run the hostile-CARGO_HOME advisory CLI fixture -> it passed 1/1 in `/private/tmp/kuru-native-advisory-delivery-path.log` after comparing the fixture directory and package root by canonical identity.
- [x] 2.2 @integration (agent) run the core automatic-provenance fixture -> it passed 1/1 in `/private/tmp/kuru-native-advisory-core-provenance.log` with the exact escaped canonical source expectation.

## 3. Scoped validation

- [x] 3.1 @integration (agent) run the delivery and core granular test, lint, and typecheck tasks -> all six named commands exited 0: delivery unit/path filters and core provenance filter above; delivery/core typechecks in `/private/tmp/kuru-native-advisory-delivery-typecheck.log` and `/private/tmp/kuru-native-advisory-core-typecheck.log`; delivery/core lints in `/private/tmp/kuru-native-advisory-delivery-lint.log` and `/private/tmp/kuru-native-advisory-core-lint.log`. No broad suite or coverage ran.
