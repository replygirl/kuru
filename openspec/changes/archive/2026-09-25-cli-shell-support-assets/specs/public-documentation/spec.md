## ADDED Requirements

### Requirement: Accurate shell-support installation guidance

Installation documentation SHALL name the supported completion shells, stable Unix `share/man/man1/kuru.1` location, direct-install versioned support directory, and stock-shell activation commands using the fixed installed executable's pure generator rather than a profile path pinned to one versioned support directory, without modifying a user's profile automatically. It SHALL show one-time MANPATH setup for ordinary `man kuru`, distinguish shipped assets from activated completions, say that a running shell needs a reload after a settled update, and avoid promising synchronized activation during an unresolved replacement. Older already-published updaters can upgrade the compatible core binary but cannot retroactively install a new support envelope; rerunning the current verified installer or using the new binary's pure output commands SHALL provide a concrete repair path. Homebrew tap/formula installation SHALL be documented only when the separate P24b integration is actually delivered.

#### Scenario: Direct installer completed
- **WHEN** a user has installed Kuru into an explicit directory
- **THEN** the docs show how each stock shell and Unix man use pure output from the fixed installed executable, and separately where the exact versioned files live, without assuming an edited profile or leaving a persistent path pinned to an obsolete version.

#### Scenario: Legacy updater completed
- **WHEN** an older client upgraded only the unchanged executable archive
- **THEN** the docs show how to rerun the current installer to obtain managed support or generate and place matching user-managed shell/man bytes with the newly installed binary, without claiming that the older updater installed them.
