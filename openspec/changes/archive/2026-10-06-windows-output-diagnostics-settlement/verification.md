# Verification

## 1. Settle diagnostics ownership [critical]

- [x] 1.1 @equivalence (agent) inspect the Windows settled-output success and failure paths -> frozen one-line diff after abort awaits the same JoinHandle before captured dispatch; root independent review clear; primary output/error/Job code unchanged
- [~] 1.2 @regression (agent) run existing native Windows command ownership/error fixtures and exact previous-release update acceptance -> defer: native Windows unavailable locally; original OS32 root retirement failed after the updater body passed, but actual locker and sampler causality are unproved; fresh full CI must observe corrected final head

## 2. Preserve package contracts

- [x] 2.1 @integration (agent) run existing delivery host/Windows lint, typecheck, format and managed checks -> host25860, Windows8814, type35572, format65841 and managed checks all exited0; no native Windows execution claim
