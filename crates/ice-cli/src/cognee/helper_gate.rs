//! Bounded admission, cache, and breaker state for optional helper work.
use super::helper::{AbstainReason, HelperOutcome};
use std::collections::{HashMap, VecDeque};
#[cfg(not(test))]
use std::sync::OnceLock;
use std::sync::{Arc, Mutex};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

const RATE_WINDOW_MS: u64 = 60_000;
const POSITIVE_TTL_MS: u64 = 10 * 60_000;
const NEGATIVE_TTL_MS: u64 = 60_000;
const BREAKER_OPEN_MS: u64 = 5 * 60_000;
const MAX_CACHE_ENTRIES: usize = 128;
const MAX_PROVIDERS: usize = 32;
const MAX_REQUESTS_PER_PROVIDER: usize = 6;
const MAX_REQUESTS_GLOBAL: usize = 12;
#[cfg(not(test))]
static SHARED: OnceLock<Arc<SharedHelperBudget>> = OnceLock::new();
#[cfg(not(test))]
fn shared() -> Arc<SharedHelperBudget> {
    SHARED
        .get_or_init(|| Arc::new(SharedHelperBudget::default()))
        .clone()
}

/// Process-wide try-only budgets. Provider IDs passed here must be nonsecret hashes.
pub(crate) struct SharedHelperBudget {
    state: Mutex<BudgetState>,
    global: Arc<Semaphore>,
}
#[derive(Default)]
struct BudgetState {
    global_times: VecDeque<u64>,
    providers: HashMap<String, ProviderState>,
}
struct ProviderState {
    times: VecDeque<u64>,
    semaphore: Arc<Semaphore>,
}
impl Default for SharedHelperBudget {
    fn default() -> Self {
        Self {
            state: Mutex::new(BudgetState::default()),
            global: Arc::new(Semaphore::new(2)),
        }
    }
}
impl SharedHelperBudget {
    #[cfg(test)]
    pub(crate) fn isolated() -> Arc<Self> {
        Arc::new(Self::default())
    }
    fn admit(self: &Arc<Self>, provider: &str, now: u64) -> Result<BudgetPermit, SkipReason> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state
            .global_times
            .retain(|t| now.saturating_sub(*t) < RATE_WINDOW_MS);
        for provider_state in state.providers.values_mut() {
            provider_state
                .times
                .retain(|t| now.saturating_sub(*t) < RATE_WINDOW_MS);
        }
        if !state.providers.contains_key(provider) {
            if state.providers.len() >= MAX_PROVIDERS {
                let evict = state
                    .providers
                    .iter()
                    .find(|(_, p)| p.times.is_empty() && p.semaphore.available_permits() == 1)
                    .map(|(k, _)| k.clone());
                if let Some(key) = evict {
                    state.providers.remove(&key);
                } else {
                    return Err(SkipReason::RateLimited);
                }
            }
            state.providers.insert(
                provider.to_owned(),
                ProviderState {
                    times: VecDeque::new(),
                    semaphore: Arc::new(Semaphore::new(1)),
                },
            );
        }
        let p = &state.providers[provider];
        if state.global_times.len() >= MAX_REQUESTS_GLOBAL
            || p.times.len() >= MAX_REQUESTS_PER_PROVIDER
        {
            return Err(SkipReason::RateLimited);
        }
        let global = self
            .global
            .clone()
            .try_acquire_owned()
            .map_err(|_| SkipReason::Busy)?;
        let provider_permit = state.providers[provider]
            .semaphore
            .clone()
            .try_acquire_owned()
            .map_err(|_| SkipReason::Busy)?;
        state.global_times.push_back(now);
        state
            .providers
            .get_mut(provider)
            .unwrap()
            .times
            .push_back(now);
        Ok(BudgetPermit {
            _global: global,
            _provider: provider_permit,
        })
    }
}
pub(crate) struct BudgetPermit {
    _global: OwnedSemaphorePermit,
    _provider: OwnedSemaphorePermit,
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
        if let Self::Skip(reason) = self {
            Some(reason)
        } else {
            None
        }
    }
}
pub(crate) struct HelperGate {
    in_flight: Option<u64>,
    next_flight: u64,
    requests: VecDeque<u64>,
    batches: VecDeque<(String, u64, Option<HelperOutcome>)>,
    consecutive_failures: u8,
    breaker_until: Option<u64>,
    budget: Arc<SharedHelperBudget>,
}
impl Default for HelperGate {
    fn default() -> Self {
        #[cfg(test)]
        {
            Self::with_budget(SharedHelperBudget::isolated())
        }
        #[cfg(not(test))]
        {
            Self::with_budget(shared())
        }
    }
}
impl HelperGate {
    pub(crate) fn with_budget(budget: Arc<SharedHelperBudget>) -> Self {
        Self {
            in_flight: None,
            next_flight: 0,
            requests: VecDeque::new(),
            batches: VecDeque::new(),
            consecutive_failures: 0,
            breaker_until: None,
            budget,
        }
    }
    pub(crate) fn admit(
        &mut self,
        key: &str,
        provider: &str,
        now: u64,
    ) -> (Admission, Option<BudgetPermit>) {
        self.requests
            .retain(|t| now.saturating_sub(*t) < RATE_WINDOW_MS);
        self.batches.retain(|(k, e, _)| *e > now || k == key);
        if self.breaker_until.is_some_and(|until| until > now) {
            return (Admission::Skip(SkipReason::CircuitOpen), None);
        }
        if self.breaker_until.is_some() {
            self.breaker_until = None;
            self.consecutive_failures = 0;
        }
        if let Some((_, _, outcome)) = self.batches.iter().find(|(k, _, _)| k == key) {
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
        if self.requests.len() >= 2 {
            return (Admission::Skip(SkipReason::RateLimited), None);
        }
        let permit = match self.budget.admit(provider, now) {
            Ok(p) => p,
            Err(reason) => return (Admission::Skip(reason), None),
        };
        self.next_flight = self.next_flight.wrapping_add(1).max(1);
        self.in_flight = Some(self.next_flight);
        self.requests.push_back(now);
        if self.batches.len() >= MAX_CACHE_ENTRIES {
            self.batches.pop_front();
        }
        self.batches
            .push_back((key.to_owned(), now.saturating_add(POSITIVE_TTL_MS), None));
        (Admission::Start, Some(permit))
    }
    pub(crate) fn flight_id(&self) -> Option<u64> {
        self.in_flight
    }
    pub(crate) fn cached(&mut self, key: &str, now: u64) -> Option<HelperOutcome> {
        self.batches
            .iter()
            .find(|(k, e, _)| k == key && *e > now)
            .and_then(|(_, _, o)| o.clone())
    }
    pub(crate) fn finish(&mut self, id: u64, key: &str, now: u64, outcome: &HelperOutcome) {
        if self.in_flight != Some(id) {
            return;
        }
        self.in_flight = None;
        let ttl = if matches!(outcome, HelperOutcome::Judged(_)) {
            POSITIVE_TTL_MS
        } else {
            NEGATIVE_TTL_MS
        };
        if let Some((_, expiry, cache)) = self.batches.iter_mut().find(|(k, _, _)| k == key) {
            *expiry = now.saturating_add(ttl);
            *cache = Some(outcome.clone())
        }
        if matches!(
            outcome,
            HelperOutcome::Abstained(
                AbstainReason::Timeout | AbstainReason::Unavailable | AbstainReason::Malformed
            )
        ) {
            self.consecutive_failures = self.consecutive_failures.saturating_add(1);
            if self.consecutive_failures >= 3 {
                self.breaker_until = Some(now.saturating_add(BREAKER_OPEN_MS));
                self.consecutive_failures = 0
            }
        } else {
            self.consecutive_failures = 0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn judged() -> HelperOutcome {
        HelperOutcome::Judged(Vec::new())
    }
    fn start(g: &mut HelperGate, key: &str, provider: &str, now: u64) {
        let (a, p) = g.admit(key, provider, now);
        assert_eq!(a, Admission::Start);
        let id = g.flight_id().unwrap();
        drop(p);
        g.finish(id, key, now, &judged());
    }
    #[test]
    fn independent_sessions_share_provider_rolling_quota() {
        let b = SharedHelperBudget::isolated();
        let mut gates = (0..7)
            .map(|_| HelperGate::with_budget(b.clone()))
            .collect::<Vec<_>>();
        for (i, gate) in gates.iter_mut().enumerate().take(6) {
            start(gate, &format!("k{i}"), "same", i as u64);
        }
        assert_eq!(
            gates[6].admit("last", "same", 7).0,
            Admission::Skip(SkipReason::RateLimited)
        );
    }
    #[test]
    fn providers_share_global_rolling_quota() {
        let b = SharedHelperBudget::isolated();
        let mut gates = (0..13)
            .map(|_| HelperGate::with_budget(b.clone()))
            .collect::<Vec<_>>();
        for (i, gate) in gates.iter_mut().enumerate().take(12) {
            start(gate, &format!("k{i}"), &format!("p{i}"), i as u64);
        }
        assert_eq!(
            gates[12].admit("last", "other", 13).0,
            Admission::Skip(SkipReason::RateLimited)
        );
    }
    #[test]
    fn cache_is_fifo_bounded_and_negative_ttl_is_short() {
        let mut g = HelperGate::with_budget(SharedHelperBudget::isolated());
        for i in 0..129 {
            g.requests.clear();
            let key = format!("{i}");
            let provider = format!("p{i}");
            let now = i as u64 * RATE_WINDOW_MS;
            let (a, p) = g.admit(&key, &provider, now);
            assert_eq!(a, Admission::Start);
            let id = g.flight_id().unwrap();
            drop(p);
            g.finish(id, &key, now, &judged());
        }
        assert!(g.cached("0", 129 * RATE_WINDOW_MS).is_none());
        assert!(g.cached("128", 129 * RATE_WINDOW_MS).is_some());
        assert!(g.cached("128", 130 * RATE_WINDOW_MS).is_some());
        let (admission, permit) = g.admit("negative", "negative-provider", 130 * RATE_WINDOW_MS);
        assert_eq!(admission, Admission::Start);
        let id = g.flight_id().unwrap();
        drop(permit);
        g.finish(
            id,
            "negative",
            130 * RATE_WINDOW_MS,
            &HelperOutcome::Abstained(AbstainReason::Timeout),
        );
        assert!(g
            .cached("negative", 130 * RATE_WINDOW_MS + NEGATIVE_TTL_MS - 1)
            .is_some());
        assert!(g
            .cached("negative", 130 * RATE_WINDOW_MS + NEGATIVE_TTL_MS)
            .is_none());
    }
    #[test]
    fn old_flight_cannot_clear_rebound_guard() {
        let b = SharedHelperBudget::isolated();
        let mut g = HelperGate::with_budget(b);
        let (a, p) = g.admit("old", "p", 0);
        assert_eq!(a, Admission::Start);
        let old = g.flight_id().unwrap();
        drop(p);
        g.in_flight = None;
        let (a, p) = g.admit("new", "p", 1);
        assert_eq!(a, Admission::Start);
        let new = g.flight_id().unwrap();
        g.finish(old, "old", 2, &judged());
        assert_eq!(g.flight_id(), Some(new));
        drop(p);
    }
}
