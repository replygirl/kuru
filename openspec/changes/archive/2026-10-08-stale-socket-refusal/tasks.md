# Tasks

## 1. Native stale endpoint

- [x] 1.1 Correct the existing fixture to bind without listening; retain exact refusal, occupied-name rejection and native inode/adjacent-data assertions, with no lock across await.
- [x] 1.2 Run the focused native test, platform lint and independent review; state explicitly that Linux coverage acceptance remains for CI.

Observed before implementation: Ubuntu job113612420206 fails the original exact ConnectionRefused assertion with ConnectionReset. Source review confirms UnixSocket bind and listen are separate native operations. The fixture's established contract is a non-listening retained socket, not a listening peer that might close after a connection. The five-second connection bound remains.

Observed after correction: focused native macOS test1/1 passes, platform format check and all-target/all-feature lint pass, independent diff review clears the short lock scope and exact identity assertions. Linux is unavailable locally; corrected instrumented Linux acceptance remains required in final PR CI before merge. No Linux pass is claimed here.
