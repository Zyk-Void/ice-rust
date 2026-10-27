//! Per-session admission limits and process-wide single-flight for optional JEV work.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, OnceLock};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use super::helper::{AbstainReason, HelperOutcome};

const RATE_WINDOW_MS: u64 = 60_000;
const CACHE_TTL_MS: u64 = 10 * 60_000;
const BREAKER_OPEN_MS: u64 = 5 * 60_000;
const MAX_REQUESTS_PER_WINDOW: usize = 2;

static SHARED: OnceLock<Arc<Semaphore>> = OnceLock::new();

fn shared() -> Arc<Semaphore> {
    SHARED.get_or_init(|| Arc::new(Semaphore::new(1))).clone()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SkipReason {
    Busy,
    RateLimited,
    CircuitOpen,
    BatchCached,
}

impl SkipReason {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Busy => "busy",
            Self::RateLimited => "rate_limited",
            Self::CircuitOpen => "circuit_open",
            Self::BatchCached => "batch_cached",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Admission {
    Start,
    Skip(SkipReason),
}

impl Admission {
    pub(crate) fn reason(self) -> Option<SkipReason> {
        match self {
            Self::Start => None,
            Self::Skip(reason) => Some(reason),
        }
    }
}

#[derive(Default)]
pub(crate) struct HelperGate {
    in_flight: Option<u64>,
    next_flight: u64,
    requests: VecDeque<u64>,
    batches: HashMap<String, (u64, Option<HelperOutcome>)>,
    consecutive_failures: u8,
    breaker_until: Option<u64>,
}

impl HelperGate {
    pub(crate) fn admit(
        &mut self,
        key: &str,
        now_ms: u64,
    ) -> (Admission, Option<OwnedSemaphorePermit>) {
        self.requests
            .retain(|request| now_ms.saturating_sub(*request) < RATE_WINDOW_MS);
        self.batches.retain(|_, (expiry, _)| *expiry > now_ms);
        if self.breaker_until.is_some_and(|until| until > now_ms) {
            return (Admission::Skip(SkipReason::CircuitOpen), None);
        }
        if self.breaker_until.is_some() {
            self.breaker_until = None;
            self.consecutive_failures = 0;
        }
        if let Some((_, outcome)) = self.batches.get(key) {
            return (
                Admission::Skip(if outcome.is_some() {
                    SkipReason::BatchCached
                } else {
                    SkipReason::Busy
                }),
                None,
            );
        }
        if self.in_flight.is_some() {
            return (Admission::Skip(SkipReason::Busy), None);
        }
        if self.requests.len() >= MAX_REQUESTS_PER_WINDOW {
            return (Admission::Skip(SkipReason::RateLimited), None);
        }
        let permit = match shared().try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => return (Admission::Skip(SkipReason::Busy), None),
        };
        self.next_flight = self.next_flight.wrapping_add(1).max(1);
        self.in_flight = Some(self.next_flight);
        self.requests.push_back(now_ms);
        self.batches
            .insert(key.to_owned(), (now_ms.saturating_add(CACHE_TTL_MS), None));
        (Admission::Start, Some(permit))
    }

    pub(crate) fn flight_id(&self) -> Option<u64> {
        self.in_flight
    }

    pub(crate) fn cached(&mut self, key: &str, now_ms: u64) -> Option<HelperOutcome> {
        self.batches
            .get(key)
            .filter(|(expiry, _)| *expiry > now_ms)
            .and_then(|(_, outcome)| outcome.clone())
    }

    pub(crate) fn finish(
        &mut self,
        flight_id: u64,
        key: &str,
        now_ms: u64,
        outcome: &HelperOutcome,
    ) {
        if self.in_flight != Some(flight_id) {
            return;
        }
        self.in_flight = None;
        match self.batches.get_mut(key) {
            Some((expiry, cached)) if *expiry > now_ms => *cached = Some(outcome.clone()),
            Some(_) => {
                self.batches.remove(key);
            }
            None => {}
        }
        let failure = matches!(
            outcome,
            HelperOutcome::Abstained(
                AbstainReason::Timeout | AbstainReason::Unavailable | AbstainReason::Malformed
            )
        );
        if failure {
            self.consecutive_failures = self.consecutive_failures.saturating_add(1);
            if self.consecutive_failures >= 3 {
                self.breaker_until = Some(now_ms.saturating_add(BREAKER_OPEN_MS));
                self.consecutive_failures = 0;
            }
        } else {
            self.consecutive_failures = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn timeout() -> HelperOutcome {
        HelperOutcome::Abstained(AbstainReason::Timeout)
    }

    #[test]
    fn limits_two_requests_per_rolling_minute() {
        let _lock = TEST_LOCK.lock().unwrap();
        let mut gate = HelperGate::default();
        for (key, time) in [("a", 0), ("b", 1)] {
            let (admission, permit) = gate.admit(key, time);
            assert_eq!(admission, Admission::Start);
            let id = gate.flight_id().unwrap();
            drop(permit);
            gate.finish(id, key, time, &HelperOutcome::Judged(Vec::new()));
        }
        assert_eq!(
            gate.admit("c", 2).0,
            Admission::Skip(SkipReason::RateLimited)
        );
        assert_eq!(gate.admit("c", 2 * RATE_WINDOW_MS).0, Admission::Start);
    }

    #[test]
    fn caches_abstentions_and_opens_breaker_after_three_failures() {
        let _lock = TEST_LOCK.lock().unwrap();
        let mut gate = HelperGate::default();
        for (key, time) in [("a", 0), ("b", RATE_WINDOW_MS), ("c", 2 * RATE_WINDOW_MS)] {
            let (admission, permit) = gate.admit(key, time);
            assert_eq!(admission, Admission::Start);
            let id = gate.flight_id().unwrap();
            drop(permit);
            gate.finish(id, key, time, &timeout());
        }
        assert_eq!(gate.cached("c", 2 * RATE_WINDOW_MS), Some(timeout()));
        assert_eq!(
            gate.admit("d", 4 * RATE_WINDOW_MS + 1).0,
            Admission::Skip(SkipReason::CircuitOpen)
        );
        assert_eq!(
            gate.admit("d", 5 * 60_000 + 2 * RATE_WINDOW_MS + 1).0,
            Admission::Start
        );
    }

    #[test]
    fn caches_completed_batches_for_ten_minutes() {
        let _lock = TEST_LOCK.lock().unwrap();
        let mut gate = HelperGate::default();
        let outcome = HelperOutcome::Judged(Vec::new());
        let (admission, permit) = gate.admit("same", 7 * RATE_WINDOW_MS);
        assert_eq!(admission, Admission::Start);
        let id = gate.flight_id().unwrap();
        drop(permit);
        gate.finish(id, "same", 7 * RATE_WINDOW_MS, &outcome);
        assert_eq!(gate.cached("same", 7 * RATE_WINDOW_MS + 1), Some(outcome));
        assert_eq!(
            gate.admit("same", 7 * RATE_WINDOW_MS + 1).0,
            Admission::Skip(SkipReason::BatchCached)
        );
        assert_eq!(
            gate.admit("same", 7 * RATE_WINDOW_MS + CACHE_TTL_MS).0,
            Admission::Start
        );
    }

    #[test]
    fn stale_flight_cannot_clear_a_newer_flight() {
        let _lock = TEST_LOCK.lock().unwrap();
        let mut gate = HelperGate::default();
        let (admission, permit) = gate.admit("old", 0);
        assert_eq!(admission, Admission::Start);
        let old_id = gate.flight_id().unwrap();
        drop(permit);
        gate.in_flight = None; // simulate cancellation/rebind
        let (admission, permit) = gate.admit("new", 1);
        assert_eq!(admission, Admission::Start);
        let new_id = gate.flight_id().unwrap();
        gate.finish(old_id, "old", 2, &timeout());
        assert_eq!(gate.flight_id(), Some(new_id));
        drop(permit);
        gate.finish(new_id, "new", 3, &HelperOutcome::Judged(Vec::new()));
        assert_eq!(gate.flight_id(), None);
    }

    #[test]
    fn process_gate_is_try_only_across_sessions() {
        let _lock = TEST_LOCK.lock().unwrap();
        let mut first = HelperGate::default();
        let mut second = HelperGate::default();
        let (admission, permit) = first.admit("first", 4 * RATE_WINDOW_MS);
        assert_eq!(admission, Admission::Start);
        assert_eq!(
            second.admit("second", 4 * RATE_WINDOW_MS).0,
            Admission::Skip(SkipReason::Busy)
        );
        drop(permit);
        assert_eq!(second.admit("second", 0).0, Admission::Start);
    }
}
