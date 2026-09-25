## 1. Bound the fixture input independently

- [x] 1.1 Confirm the selected instrumented artifact fails the old fixture input bound before packaging, and a distinct privately stripped copy remains within the unchanged shipping bound.
- [x] 1.2 Replace the selected debug/instrumented input limit with an explicit finite 192 MiB fixture bound while preserving source identity, bounded copy, final 128 MiB archive and profile checks.

## 2. Verify actual packaged behavior

- [x] 2.1 Run the exact instrumented embedded-runtime acceptance against the corrected fixture and observe offline install, self-update, and nonempty profile collection.
- [x] 2.2 Run strict Cospec validation and apply, and record normal final-head push hooks and native CI as deferred before-merge gates.
