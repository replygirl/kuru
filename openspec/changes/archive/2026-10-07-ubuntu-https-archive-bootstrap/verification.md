# Verification

## 1. Ubuntu native CI bootstrap [critical]

- [x] 1.1 @integration (agent) run `incident_apt_update_over_every_source_is_rejected` for the repository's Ubuntu apt bootstrap checks -> passed: 1 test passed, 27 filtered out.
- [~] 1.2 @runtime (agent) observe the reviewed PR's full CI run on the GitHub-hosted Ubuntu native-test runner; confirm the Ubuntu archive setup step and required checks complete successfully -> defer: the patch has not been pushed, so no real-runner result exists yet.
