//! Temporary coverage stall drill: revert before merge.
//!
//! This never-ending test makes the `connectors-core-platform` coverage shard
//! reach its inner deadline on every OS so the hosted run uploads a stall report.

#[test]
fn coverage_stall_drill() {
    loop {
        std::thread::sleep(std::time::Duration::from_secs(60));
    }
}
