//! Strict parsing for successful native updater stderr. These fixed diagnostic
//! records are allowed; no other stderr is discarded or treated as harmless.

#[derive(Debug, Eq, PartialEq)]
pub struct Record<'a> {
    pub phase: &'a str,
    pub elapsed_ms: u128,
}

pub fn parse(bytes: &[u8]) -> Result<Vec<Record<'_>>, &'static str> {
    if bytes.is_empty() || bytes.len() > 64 * 1024 {
        return Err("missing or oversized helper trace");
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "helper trace is not UTF-8")?;
    let complete = text
        .strip_suffix('\n')
        .ok_or("incomplete helper trace record")?;
    let mut records = Vec::new();
    let mut previous = 0;
    for line in complete.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        let record = line
            .strip_prefix("trusted update helper: phase=")
            .ok_or("unexpected stderr outside helper trace")?;
        let (phase, decimal) = record
            .split_once(" elapsed_ms=")
            .ok_or("malformed helper trace fields")?;
        if !matches!(
            phase,
            "started"
                | "connect_parent"
                | "receive_request"
                | "open_installation_state"
                | "acquire_installation_lock"
                | "installation_lock_acquired"
                | "acquire_loaded_helper_guard"
                | "hash_loaded_helper"
                | "verify_loaded_helper_record"
                | "loaded_helper_verified"
                | "prepare_parent_and_previous_receipt"
                | "verify_original"
                | "verify_recorded_helper"
                | "verify_candidate_source"
                | "copy_candidate"
                | "copy_rollback"
                | "save_prepared_receipt"
                | "prepared_receipt_saved"
                | "publish"
                | "old_moved_before_receipt"
                | "candidate_moved_before_receipt"
                | "reconcile_publication_error"
                | "send_verified_publication_acknowledgment"
                | "verified_publication_acknowledgment_sent"
                | "wait_parent_exit"
                | "parent_still_alive_cleanup_pending"
                | "cleanup_after_parent_exit"
                | "complete"
        ) {
            return Err("unknown helper trace phase");
        }
        if decimal.is_empty() || !decimal.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err("helper elapsed time is not unsigned decimal");
        }
        let elapsed_ms = decimal
            .parse::<u128>()
            .map_err(|_| "helper elapsed time overflow")?;
        if decimal != elapsed_ms.to_string() || elapsed_ms < previous {
            return Err("noncanonical or decreasing helper elapsed time");
        }
        if records.is_empty() != (phase == "started") {
            return Err("helper trace must start exactly once");
        }
        records.push(Record { phase, elapsed_ms });
        previous = elapsed_ms;
    }
    Ok(records)
}
