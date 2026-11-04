//! Bounded optional telemetry transport. No disk work runs under this lock.
//! Accounted bytes are bounded debug-encoded size, not an exact heap measure.
//! Producers never wait for queue space; overload is explicitly reported.

use std::collections::VecDeque;
use std::fmt::{self, Write};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use super::Command;

const MAX_COUNT: usize = 512;
const MAX_BYTES: usize = 8 * 1024 * 1024;
const MAX_EVENT_BYTES: usize = 256 * 1024;
const RESERVED_COUNT: usize = 64;
const RESERVED_BYTES: usize = 1024 * 1024;

struct State {
    pending: VecDeque<(Command, usize)>,
    bytes: usize,
    closed: bool,
    reported_gap: bool,
}

struct Shared {
    state: Mutex<State>,
    ready: Condvar,
    lost: AtomicU64,
    senders: AtomicUsize,
}

pub(super) struct Sender(Arc<Shared>);
pub(super) struct Receiver(Arc<Shared>);

pub(super) enum Received {
    Command(Command),
    Gap(u64),
}

pub(super) fn channel() -> (Sender, Receiver) {
    let shared = Arc::new(Shared {
        state: Mutex::new(State {
            pending: VecDeque::new(),
            bytes: 0,
            closed: false,
            reported_gap: false,
        }),
        ready: Condvar::new(),
        lost: AtomicU64::new(0),
        senders: AtomicUsize::new(1),
    });
    (Sender(shared.clone()), Receiver(shared))
}

impl Clone for Sender {
    fn clone(&self) -> Self {
        self.0.senders.fetch_add(1, Ordering::Relaxed);
        Self(self.0.clone())
    }
}

impl Drop for Sender {
    fn drop(&mut self) {
        if self.0.senders.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.close();
        }
    }
}

// fmt writes directly into a capped counter: no serialized payload copy. An
// over-limit value short-circuits without traversing its remaining children.
struct Counter(usize);
impl Write for Counter {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        self.0 = self.0.saturating_add(text.len());
        if self.0 > MAX_EVENT_BYTES {
            Err(fmt::Error)
        } else {
            Ok(())
        }
    }
}

fn size(command: &Command) -> Option<usize> {
    let mut counter = Counter(std::mem::size_of::<Command>());
    write!(&mut counter, "{command:?}").ok()?;
    Some(counter.0)
}

fn terminal(command: &Command) -> bool {
    use ice_agent::AgentEvent;
    matches!(
        command,
        Command::Agent(
            AgentEvent::AgentEnd { .. }
                | AgentEvent::RunSettled { .. }
                | AgentEvent::TurnEnd { .. }
                | AgentEvent::MessageEnd { .. }
                | AgentEvent::ToolExecutionEnd { .. }
        )
    ) || matches!(command, Command::Memory(observation)
        if matches!(observation.phase.as_str(), "completed" | "succeeded" | "failed" | "cancelled" | "skipped"))
}

impl Sender {
    /// Ok means the pipeline is open, not that this event was retained. Loss is
    /// reported independently so saturation cannot disconnect the live feed.
    pub(super) fn send(&self, command: Command) -> Result<(), ()> {
        let bytes = size(&command);
        // The lock covers only bounded in-memory bookkeeping, never formatting,
        // store I/O, or waiting for capacity. Brief contention is not overload.
        let mut state = self.0.state.lock().unwrap();
        if state.closed {
            return Err(());
        }
        let Some(bytes) = bytes else {
            self.0.lost.fetch_add(1, Ordering::Relaxed);
            self.0.ready.notify_one();
            return Ok(());
        };
        // Only coalesce adjacent samples: preserve event ordering otherwise.
        if matches!(command, Command::Info(_))
            && matches!(state.pending.back(), Some((Command::Info(_), _)))
        {
            let (_, old_bytes) = state.pending.pop_back().expect("known sample");
            state.bytes -= old_bytes;
        }
        let critical = terminal(&command);
        let count_limit = MAX_COUNT - if critical { 0 } else { RESERVED_COUNT };
        let byte_limit = MAX_BYTES - if critical { 0 } else { RESERVED_BYTES };
        if state.pending.len() >= count_limit || state.bytes.saturating_add(bytes) > byte_limit {
            self.0.lost.fetch_add(1, Ordering::Relaxed);
        } else {
            state.bytes += bytes;
            state.pending.push_back((command, bytes));
        }
        self.0.ready.notify_one();
        Ok(())
    }

    /// Out-of-band close cannot be dropped by a saturated queue. Already
    /// accepted commands and the gap signal drain before the receiver exits.
    pub(super) fn close(&self) {
        self.0.state.lock().unwrap().closed = true;
        self.0.ready.notify_one();
    }

    /// Deadline fallback: discard queued optional telemetry, never replay it
    /// after shutdown was acknowledged. An already-dequeued write may finish.
    pub(super) fn abandon(&self) {
        let mut state = self.0.state.lock().unwrap();
        state.closed = true;
        state.pending.clear();
        state.bytes = 0;
        self.0.lost.store(0, Ordering::Relaxed);
        self.0.ready.notify_one();
    }
}

impl Receiver {
    pub(super) fn recv(&self) -> Option<Received> {
        let mut state = self.0.state.lock().unwrap();
        loop {
            // Alternate diagnostics and accepted events under sustained loss;
            // a busy producer must not starve the events already admitted.
            if !state.reported_gap || state.pending.is_empty() {
                let lost = self.0.lost.swap(0, Ordering::Relaxed);
                if lost != 0 {
                    state.reported_gap = true;
                    return Some(Received::Gap(lost));
                }
            }
            if let Some((command, bytes)) = state.pending.pop_front() {
                state.bytes -= bytes;
                state.reported_gap = false;
                return Some(Received::Command(command));
            }
            if state.closed {
                return None;
            }
            // Periodic wake also observes an oversized-event loss whose notify
            // raced this wait; diagnostics must not depend on another event.
            state = self
                .0
                .ready
                .wait_timeout(state, std::time::Duration::from_millis(100))
                .unwrap()
                .0;
        }
    }
}

impl Drop for Receiver {
    fn drop(&mut self) {
        let mut state = self.0.state.lock().unwrap();
        state.closed = true;
        state.pending.clear();
        state.bytes = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ice_agent::AgentEvent;

    #[test]
    fn saturated_queue_reserves_terminal_space_and_always_closes() {
        let (tx, rx) = channel();
        for _ in 0..10_000 {
            tx.send(Command::Agent(AgentEvent::TurnStart)).unwrap();
        }
        tx.send(Command::Agent(AgentEvent::AgentEnd { messages: vec![] }))
            .unwrap();
        {
            let state = tx.0.state.lock().unwrap();
            assert_eq!(state.pending.len(), MAX_COUNT - RESERVED_COUNT + 1);
            assert!(state.bytes <= MAX_BYTES);
        }
        tx.close();
        let mut lost = 0;
        let mut terminal_count = 0;
        while let Some(event) = rx.recv() {
            match event {
                Received::Gap(n) => lost += n,
                Received::Command(Command::Agent(AgentEvent::AgentEnd { .. })) => {
                    terminal_count += 1
                }
                _ => {}
            }
        }
        assert_eq!(lost, 10_000 - (MAX_COUNT - RESERVED_COUNT) as u64);
        assert_eq!(terminal_count, 1);
    }

    #[test]
    fn abandon_discards_accepted_events_and_loss_without_reopening() {
        let (tx, rx) = channel();
        for _ in 0..MAX_COUNT * 2 {
            tx.send(Command::Agent(AgentEvent::TurnStart)).unwrap();
        }
        assert!(tx.0.lost.load(Ordering::Relaxed) > 0);
        tx.abandon();
        assert!(rx.recv().is_none());
        assert_eq!(tx.0.state.lock().unwrap().bytes, 0);
        assert!(tx.send(Command::Agent(AgentEvent::TurnStart)).is_err());
        assert_eq!(tx.0.lost.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn payload_budget_rejects_oversize_and_recovers_after_drain() {
        let (tx, rx) = channel();
        tx.send(Command::CustomEntry {
            custom_type: "fixture".into(),
            data: serde_json::json!("x".repeat(MAX_EVENT_BYTES + 1)),
        })
        .unwrap();
        assert!(matches!(rx.recv(), Some(Received::Gap(1))));
        for _ in 0..1000 {
            tx.send(Command::CustomEntry {
                custom_type: "fixture".into(),
                data: serde_json::json!("😀".repeat(16_000)),
            })
            .unwrap();
        }
        assert!(tx.0.state.lock().unwrap().bytes <= MAX_BYTES - RESERVED_BYTES);
        assert!(tx.0.lost.load(Ordering::Relaxed) > 0);
        while !tx.0.state.lock().unwrap().pending.is_empty() {
            rx.recv();
        }
        tx.send(Command::Agent(AgentEvent::TurnStart)).unwrap();
        assert!(matches!(rx.recv(), Some(Received::Command(_))));
        drop(tx);
        assert!(rx.recv().is_none());
    }

    #[test]
    fn closed_queue_rejects_oversized_events_without_reporting_more_loss() {
        for receiver_dropped in [false, true] {
            let (tx, rx) = channel();
            if receiver_dropped {
                drop(rx);
            } else {
                tx.close();
                assert!(rx.recv().is_none());
            }
            assert!(tx
                .send(Command::CustomEntry {
                    custom_type: "fixture".into(),
                    data: serde_json::json!("x".repeat(MAX_EVENT_BYTES + 1)),
                })
                .is_err());
            assert_eq!(tx.0.lost.load(Ordering::Relaxed), 0);
        }
    }

    #[test]
    fn concurrent_producers_deliver_under_capacity_and_account_for_overload() {
        for each in [16, 1024] {
            let (tx, rx) = channel();
            std::thread::scope(|scope| {
                for _ in 0..8 {
                    let sender = tx.clone();
                    scope.spawn(move || {
                        for _ in 0..each {
                            sender.send(Command::Agent(AgentEvent::TurnStart)).unwrap();
                        }
                    });
                }
            });
            tx.close();
            let (mut accepted, mut lost) = (0, 0);
            while let Some(received) = rx.recv() {
                match received {
                    Received::Command(_) => accepted += 1,
                    Received::Gap(count) => lost += count,
                }
            }
            assert_eq!(accepted + lost, 8 * each);
            assert_eq!(
                accepted,
                (8 * each).min((MAX_COUNT - RESERVED_COUNT) as u64)
            );
        }
    }

    #[test]
    fn adjacent_info_is_coalesced_and_critical_loss_is_explicit() {
        let (tx, rx) = channel();
        for _ in 0..1000 {
            tx.send(Command::Info(Default::default())).unwrap();
        }
        assert_eq!(tx.0.state.lock().unwrap().pending.len(), 1);
        for _ in 0..MAX_COUNT {
            tx.send(Command::Agent(AgentEvent::RunSettled {
                run_id: "fixture".into(),
            }))
            .unwrap();
        }
        assert!(matches!(rx.recv(), Some(Received::Gap(1))));
        // More drops after the gap must not starve the accepted queue.
        tx.send(Command::Agent(AgentEvent::RunSettled {
            run_id: "lost".into(),
        }))
        .unwrap();
        assert!(matches!(rx.recv(), Some(Received::Command(_))));
        assert!(matches!(rx.recv(), Some(Received::Gap(1))));
        tx.close();
    }
}
