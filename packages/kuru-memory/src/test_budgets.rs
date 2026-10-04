//! Product budgets that tests outside this crate derive their waits from.
//!
//! Each item forwards a product value unchanged, so a test bound written in
//! terms of it moves with the product instead of guessing a number. Nothing
//! here is a new budget.

use std::time::Duration;

/// The client's reply deadline for one service operation
/// (`service::rpc::OPERATION_TIMEOUT`).
pub const OPERATION_TIMEOUT: Duration = crate::service::rpc::OPERATION_TIMEOUT;

/// One memory statement's budget, also the pool acquire ceiling
/// (`store::QUERY_TIMEOUT`).
pub const QUERY_TIMEOUT: Duration = crate::store::QUERY_TIMEOUT;

/// Worst-case owned server close: the graceful pool drain, the lifetime
/// close, the supervisor reap allowance and the post-reap drain
/// (`server::close_budget`).
pub fn close_budget() -> Duration {
    crate::server::close_budget()
}

/// Bound for the supervisor to stop Dolt gracefully, kill it and report its
/// own exit (`server::SUPERVISOR_REAP_ALLOWANCE`).
pub const SUPERVISOR_REAP_ALLOWANCE: Duration = crate::server::SUPERVISOR_REAP_ALLOWANCE;
