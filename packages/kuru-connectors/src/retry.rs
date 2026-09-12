use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime},
};

use anyhow::{Result, ensure};
use reqwest::header::HeaderValue;

const MAX_APPLICATION_SENDS: u8 = 4;
const MAX_PROVIDER_SENDS: u8 = 3;
const MAX_REFRESH_SENDS: u8 = 2;
const MAX_DELAYS: u8 = 2;
const MAX_DELAY: Duration = Duration::from_secs(30);
const MAX_TOTAL_DELAY: Duration = Duration::from_secs(60);
const MAX_RETRY_AFTER_BYTES: usize = 128;

#[derive(Clone)]
pub(crate) struct OperationBudget {
    shared: Arc<Mutex<State>>,
    deadline: Instant,
}

struct State {
    application_sends: u8,
    provider_sends: u8,
    refresh_sends: u8,
    delays: u8,
    total_delay: Duration,
    rotation_used: bool,
    random: Option<u64>,
}

#[derive(Clone)]
pub(crate) struct RefreshAllowance {
    budget: OperationBudget,
    deadline: Instant,
}

pub(crate) enum RetryDecision {
    Delay(Duration),
    Exhausted,
}

impl OperationBudget {
    pub(crate) fn new(timeout: Duration) -> Self {
        let deadline = Instant::now()
            .checked_add(timeout)
            .unwrap_or_else(Instant::now);
        let mut bytes = [0_u8; 8];
        let random = getrandom::fill(&mut bytes)
            .ok()
            .map(|()| u64::from_ne_bytes(bytes));
        Self {
            shared: Arc::new(Mutex::new(State {
                application_sends: 0,
                provider_sends: 0,
                refresh_sends: 0,
                delays: 0,
                total_delay: Duration::ZERO,
                rotation_used: false,
                random,
            })),
            deadline,
        }
    }

    pub(crate) fn take_provider_send(&self) -> Result<()> {
        let mut state = self
            .shared
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        ensure!(
            Instant::now() < self.deadline
                && state.application_sends < MAX_APPLICATION_SENDS
                && state.provider_sends < MAX_PROVIDER_SENDS,
            "provider retry budget exhausted after {} provider attempts",
            state.provider_sends
        );
        state.application_sends += 1;
        state.provider_sends += 1;
        Ok(())
    }

    pub(crate) fn provider_attempts(&self) -> u8 {
        self.shared
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .provider_sends
    }

    pub(crate) fn begin_rotation(&self, auth_timeout: Duration) -> Result<RefreshAllowance> {
        let mut state = self
            .shared
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let remaining = MAX_APPLICATION_SENDS.saturating_sub(state.application_sends);
        ensure!(
            Instant::now() < self.deadline && !state.rotation_used && remaining >= 2,
            "provider retry budget exhausted after {} provider attempts",
            state.provider_sends
        );
        state.rotation_used = true;
        let auth_deadline = Instant::now()
            .checked_add(auth_timeout)
            .unwrap_or(self.deadline);
        Ok(RefreshAllowance {
            budget: self.clone(),
            deadline: self.deadline.min(auth_deadline),
        })
    }

    pub(crate) fn retry_delay(
        &self,
        retry_after: Option<&HeaderValue>,
        wall_now: SystemTime,
    ) -> RetryDecision {
        let mut state = self
            .shared
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if state.delays >= MAX_DELAYS
            || state.provider_sends >= MAX_PROVIDER_SENDS
            || state.application_sends >= MAX_APPLICATION_SENDS
        {
            return RetryDecision::Exhausted;
        }
        let base = if state.delays == 0 {
            Duration::from_millis(500)
        } else {
            Duration::from_secs(1)
        };
        let jitter = equal_jitter(base, &mut state.random);
        let minimum = match retry_after.map(|value| parse_retry_after(value, wall_now)) {
            Some(RetryAfter::Unfit) => return RetryDecision::Exhausted,
            Some(RetryAfter::Valid(value)) => value,
            Some(RetryAfter::Invalid) | None => Duration::ZERO,
        };
        let delay = jitter.max(minimum);
        let Some(total) = state.total_delay.checked_add(delay) else {
            return RetryDecision::Exhausted;
        };
        if delay > MAX_DELAY
            || total > MAX_TOTAL_DELAY
            || Instant::now()
                .checked_add(delay)
                .is_none_or(|then| then >= self.deadline)
        {
            return RetryDecision::Exhausted;
        }
        state.delays += 1;
        state.total_delay = total;
        RetryDecision::Delay(delay)
    }

    #[cfg(test)]
    fn with_seed(timeout: Duration, seed: Option<u64>) -> Self {
        let budget = Self::new(timeout);
        budget
            .shared
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .random = seed;
        budget
    }
}

impl RefreshAllowance {
    pub(crate) fn deadline(&self) -> Instant {
        self.deadline
    }

    pub(crate) fn check_dispatch(&self, caller_closed: bool) -> Result<()> {
        ensure!(
            !caller_closed && Instant::now() < self.deadline,
            "authentication refresh was cancelled before dispatch"
        );
        Ok(())
    }

    pub(crate) fn take_refresh_send(&self, reserve_provider_replay: bool) -> Result<()> {
        let mut state = self
            .budget
            .shared
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let required_after = u8::from(reserve_provider_replay);
        ensure!(
            Instant::now() < self.deadline
                && state.refresh_sends < MAX_REFRESH_SENDS
                && state.application_sends < MAX_APPLICATION_SENDS
                && MAX_APPLICATION_SENDS - state.application_sends > required_after,
            "ChatGPT refresh retry budget exhausted"
        );
        state.application_sends += 1;
        state.refresh_sends += 1;
        Ok(())
    }

    pub(crate) fn can_repeat_refresh(&self) -> bool {
        let state = self
            .budget
            .shared
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        Instant::now() < self.deadline
            && state.refresh_sends < MAX_REFRESH_SENDS
            && MAX_APPLICATION_SENDS.saturating_sub(state.application_sends) >= 2
    }

    pub(crate) fn retry_delay(&self) -> RetryDecision {
        self.budget.retry_delay(None, SystemTime::now())
    }
}

enum RetryAfter {
    Valid(Duration),
    Invalid,
    Unfit,
}

fn parse_retry_after(value: &HeaderValue, now: SystemTime) -> RetryAfter {
    let bytes = value.as_bytes();
    if bytes.len() > MAX_RETRY_AFTER_BYTES {
        return RetryAfter::Invalid;
    }
    if !bytes.is_empty() && bytes.iter().all(u8::is_ascii_digit) {
        let Ok(text) = std::str::from_utf8(bytes) else {
            return RetryAfter::Invalid;
        };
        return match text.parse::<u64>() {
            Ok(seconds) => RetryAfter::Valid(Duration::from_secs(seconds)),
            Err(_) => RetryAfter::Unfit,
        };
    }
    let Ok(text) = std::str::from_utf8(bytes) else {
        return RetryAfter::Invalid;
    };
    let Ok(date) = httpdate::parse_http_date(text) else {
        return RetryAfter::Invalid;
    };
    if httpdate::fmt_http_date(date) != text {
        return RetryAfter::Invalid;
    }
    match date.duration_since(now) {
        Ok(delay) => RetryAfter::Valid(round_up_seconds(delay)),
        Err(_) => RetryAfter::Valid(Duration::ZERO),
    }
}

fn round_up_seconds(delay: Duration) -> Duration {
    if delay.subsec_nanos() == 0 {
        delay
    } else {
        Duration::from_secs(delay.as_secs().saturating_add(1))
    }
}

fn equal_jitter(base: Duration, random: &mut Option<u64>) -> Duration {
    let half = base / 2;
    let span = base - half;
    let Some(mut value) = *random else {
        return base;
    };
    value ^= value << 13;
    value ^= value >> 7;
    value ^= value << 17;
    *random = Some(value);
    let span_nanos = span.as_nanos() as u64;
    half + Duration::from_nanos(value % (span_nanos + 1))
}

pub(crate) fn exhausted(operation: impl std::fmt::Display, attempts: u8) -> anyhow::Error {
    anyhow::anyhow!("{operation} retry budget exhausted after {attempts} provider attempts")
}

pub(crate) fn retry_after(headers: &reqwest::header::HeaderMap) -> Option<HeaderValue> {
    let mut values = headers.get_all(reqwest::header::RETRY_AFTER).iter();
    let value = values.next()?.clone();
    values.next().is_none().then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_after_accepts_only_bounded_canonical_values() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        assert!(matches!(
            parse_retry_after(&HeaderValue::from_static("0002"), now),
            RetryAfter::Valid(value) if value == Duration::from_secs(2)
        ));
        assert!(matches!(
            parse_retry_after(&HeaderValue::from_static("999999999999999999999999"), now),
            RetryAfter::Unfit
        ));
        let date = httpdate::fmt_http_date(now + Duration::from_secs(3));
        assert!(matches!(
            parse_retry_after(&HeaderValue::from_str(&date).unwrap(), now),
            RetryAfter::Valid(value) if value == Duration::from_secs(3)
        ));
        assert!(matches!(
            parse_retry_after(
                &HeaderValue::from_static("Sunday, 06-Nov-94 08:49:37 GMT"),
                now
            ),
            RetryAfter::Invalid
        ));
        assert!(matches!(
            parse_retry_after(&HeaderValue::from_str(&"1".repeat(129)).unwrap(), now),
            RetryAfter::Invalid
        ));
        let fractional_now = now + Duration::from_millis(250);
        assert!(matches!(
            parse_retry_after(&HeaderValue::from_str(&date).unwrap(), fractional_now),
            RetryAfter::Valid(value) if value == Duration::from_secs(3)
        ));
    }

    #[test]
    fn equal_jitter_is_seeded_and_entropy_failure_uses_upper_bound() {
        let base = Duration::from_secs(1);
        assert_eq!(equal_jitter(base, &mut None), base);
        let first = equal_jitter(base, &mut Some(7));
        let second = equal_jitter(base, &mut Some(7));
        assert_eq!(first, second);
        assert!(first >= base / 2 && first <= base);
    }

    #[test]
    fn retry_delay_rejects_unfit_minimums_and_uses_bounded_fallback() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let budget = OperationBudget::with_seed(Duration::from_secs(10), Some(11));
        assert!(matches!(
            budget.retry_delay(Some(&HeaderValue::from_static("31")), now),
            RetryDecision::Exhausted
        ));

        let budget = OperationBudget::with_seed(Duration::from_secs(10), Some(11));
        assert!(matches!(
            budget.retry_delay(
                Some(&HeaderValue::from_static("999999999999999999999999")),
                now
            ),
            RetryDecision::Exhausted
        ));

        let budget = OperationBudget::with_seed(Duration::from_secs(10), Some(11));
        assert!(matches!(
            budget.retry_delay(Some(&HeaderValue::from_static("invalid")), now),
            RetryDecision::Delay(value)
                if value >= Duration::from_millis(250) && value <= Duration::from_millis(500)
        ));
        assert!(matches!(
            budget.retry_delay(None, now),
            RetryDecision::Delay(value)
                if value >= Duration::from_millis(500) && value <= Duration::from_secs(1)
        ));
        assert!(matches!(
            budget.retry_delay(None, now),
            RetryDecision::Exhausted
        ));

        let budget = OperationBudget::with_seed(Duration::from_millis(100), Some(11));
        assert!(matches!(
            budget.retry_delay(None, now),
            RetryDecision::Exhausted
        ));
    }

    #[test]
    fn retry_after_uses_two_exact_thirty_second_delays_then_exhausts() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let budget = OperationBudget::with_seed(Duration::from_secs(300), Some(11));
        let retry_after = HeaderValue::from_static("30");

        let first = match budget.retry_delay(Some(&retry_after), now) {
            RetryDecision::Delay(delay) => delay,
            RetryDecision::Exhausted => panic!("first 30-second minimum must fit"),
        };
        let second = match budget.retry_delay(Some(&retry_after), now) {
            RetryDecision::Delay(delay) => delay,
            RetryDecision::Exhausted => panic!("second 30-second minimum must fit"),
        };

        assert_eq!(first, Duration::from_secs(30));
        assert_eq!(second, Duration::from_secs(30));
        assert_eq!(first + second, Duration::from_secs(60));
        assert!(matches!(
            budget.retry_delay(Some(&retry_after), now),
            RetryDecision::Exhausted
        ));
    }

    #[test]
    fn duplicate_retry_after_is_not_treated_as_a_server_minimum() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.append(reqwest::header::RETRY_AFTER, HeaderValue::from_static("1"));
        headers.append(reqwest::header::RETRY_AFTER, HeaderValue::from_static("2"));
        assert!(retry_after(&headers).is_none());
    }
}
