## Context

Git commands may automatically launch detached maintenance. The hosted failures
report only .git/objects/maintenance.lock, not ref/index/config corruption:
/tmp/kuru-pr9-macos-failed.log and /tmp/kuru-pr9-linux-failed.log.
Git documents maintenance.auto as the command-triggered maintenance control:
https://git-scm.com/docs/git-maintenance.

## Decisions

Set maintenance controls on disposable setup commands before initialization and
the first commit. Observe actual setup subprocesses through Git's trace events;
the trace lives outside the snapshotted caller. Retain whole-repository equality
and child-only environment poisoning. Do not add retries, sleeps or ignored locks.

## Risks / Trade-offs

The trace proves process launch policy without relying on a race's timing. Keep
this policy confined to sentinel setup so actual production behavior is tested
unchanged and no user's maintenance configuration is altered.

## Integration contract

Real Git initializes and commits the disposable sentinel without detached setup
work outliving its baseline. The unchanged real release and Communiqué scenarios
must still preserve every caller repository byte after child execution.
