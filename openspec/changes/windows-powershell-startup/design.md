## Context

The original native timeout discarded the child state. Repeated processes on one runner have passed, including an unchanged-script control, but those processes share a Windows instance and its system caches. The original cause remains unconfirmed.

## Decisions

Temporarily fan out the existing Windows platform job to 16 fresh runners. Each runs the original single direct/configured launch comparison and unchanged assertions, with failure-only state capture and awaited cleanup. This tests fresh runner variation without adding unrelated behavioral probes. Preserve each coverage artifact under a distinct matrix name.

Remove the temporary matrix and restore the ordinary artifact name before merge. Keep the release workflow untouched. A passing reproduction is evidence only of that run, not proof of a root-cause correction.

## Operational surface

Use the existing native `windows-2025` GitHub runners, not containers. Retain the existing exact Rust, mise and action pins and `x86_64-pc-windows-msvc` target. No new secrets, application dependencies, services, bind addresses or network listeners are introduced. Each job retains its existing 30-minute maximum and package-owned coverage task. GitHub's existing runner concurrency limits apply; diagnostic polling stays near five-minute intervals.

## Risks / Trade-offs

The temporary matrix increases runner work. It is bounded to 16 instances and must be removed before merge. Failure capture must preserve the original assertion failure; neither retries nor successful siblings can turn a failed instance green.
