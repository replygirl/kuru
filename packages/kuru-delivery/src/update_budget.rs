//! The Windows update handoff's waits, kept outside the Windows-only updater so
//! maintainer fixtures on every host can bound an updater launch by them.

use std::time::Duration;

/// Connection establishment, each control frame already in flight and each
/// pipe close.
pub const STARTUP: Duration = Duration::from_secs(10);
/// The authenticated helper verifies and durably copies full embedded images
/// before acknowledging publication. That work has its own finite allowance;
/// connection establishment and a frame already in flight keep their short limit.
pub const PUBLICATION: Duration = Duration::from_secs(120);
/// Draining the helper's diagnostics and waiting for its exit after a failure.
pub const CLEANUP: Duration = Duration::from_secs(10);

/// The longest the updating parent can wait on its trusted helper, in series:
/// accepting the connection and sending the request (`STARTUP` each), the
/// publication acknowledgment (`PUBLICATION`; its frame deadline is clamped
/// inside it), closing the pipe (`STARTUP`), then on a failed handoff the
/// helper's exit (`STARTUP + CLEANUP`) and its drained diagnostics
/// (`CLEANUP`). Verification before the handoff has no bound of its own.
pub const fn handoff() -> Duration {
    STARTUP
        .saturating_mul(4)
        .saturating_add(PUBLICATION)
        .saturating_add(CLEANUP.saturating_mul(2))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handoff_sums_the_parent_waits_in_series() {
        let waits = [
            STARTUP,           // accept
            STARTUP,           // send the request
            PUBLICATION,       // acknowledgment, frame clamped inside it
            STARTUP,           // close
            STARTUP + CLEANUP, // failed handoff: helper exit
            CLEANUP,           // failed handoff: drained diagnostics
        ];
        assert_eq!(handoff(), waits.iter().sum::<Duration>());
    }
}
