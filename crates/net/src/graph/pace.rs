//! The ground rules every Graph request about a Microsoft account's mail goes by (R3 spec S14,
//! S21): one retry policy by kind of request, and one gate per mailbox.
//!
//! - **Trying again.** A read ([`Kind::Read`]) and a change that is the same however often it is
//!   made ([`Kind::Same`]: setting a property, deleting) are asked again when the server says
//!   "busy" (429, 503, 504) or can't be reached, after as long as it asked (`Retry-After`) or,
//!   when it didn't say, after a wait that doubles each time ([`Retry`]). A change that would
//!   happen twice ([`Kind::Once`]: making something, a move, sending) is never asked again by
//!   itself: whoever asked looks first.
//! - **One mailbox, a few requests at once.** Microsoft allows 4 requests at once per mailbox.
//!   [`Gate`] lets [`MAX_AT_ONCE`] through, of which work nobody is waiting for
//!   ([`Priority::Background`]) takes [`MAX_BACKGROUND`] at most, so that what the owner waits
//!   for always has room. A "busy" answer pauses every request for that mailbox for as long as
//!   the server asked ([`Gate::pause`]), whoever made it.
//!
//! A wait the server asks for that is longer than [`Retry::longest_wait`] is not waited out in a
//! run: the gate stays shut for the mailbox that long, and the run ends with the server's
//! "busy" for the host to try again after.

use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use crate::NetError;

/// Most Graph requests at once for one mailbox (Microsoft's limit).
pub const MAX_AT_ONCE: usize = 4;
/// Most of those that work nobody is waiting for may take.
pub const MAX_BACKGROUND: usize = 2;
/// Most mailboxes a process keeps a gate for; more and the unused ones are let go.
const MAX_GATES: usize = 256;

/// What kind of request it is, for whether it may be asked again by itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A read (`GET`, or a `$batch` of reads).
    Read,
    /// A change that is the same however often it is made: setting properties (`PATCH`), a
    /// deletion (`DELETE`).
    Same,
    /// A change that would happen twice: making something, a move, a copy, sending (`POST`).
    Once,
}

impl Kind {
    /// Whether a request of this kind may be asked again without looking first.
    pub fn may_repeat(self) -> bool {
        !matches!(self, Kind::Once)
    }
}

/// Who is waiting for a request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Priority {
    /// The owner: opening a message, sending, a Send/Receive they asked for.
    Owner,
    /// Nobody: fetching in the background, a run that started by itself.
    Background,
}

/// How requests are asked again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Retry {
    /// Most tries of one request, the first one counted.
    pub tries: u32,
    /// The wait before the second try when the server doesn't say how long; it doubles for each
    /// try after.
    pub first_wait: Duration,
    /// The longest wait that is waited out within a run.
    pub longest_wait: Duration,
}

impl Retry {
    /// What every Graph request about mail goes by.
    pub const STANDARD: Retry = Retry { tries: 4, first_wait: Duration::from_secs(1), longest_wait: Duration::from_secs(60) };

    /// How long to wait before try number `next` (2 for the second), when the server asked for
    /// `asked` seconds (or didn't say).
    pub fn wait_before(&self, next: u32, asked: Option<u32>) -> Duration {
        match asked {
            Some(seconds) => Duration::from_secs(u64::from(seconds)),
            None => {
                let doublings = next.saturating_sub(2).min(16);
                self.first_wait.saturating_mul(1u32 << doublings).min(self.longest_wait)
            }
        }
    }
}

/// Who may ask a mailbox's server something just now: at most [`MAX_AT_ONCE`] at once, at most
/// [`MAX_BACKGROUND`] of them for work nobody waits for, and nobody while the server asked to be
/// left alone. One per mailbox ([`Gate::for_mailbox`]), shared by every thread that asks.
#[derive(Debug, Default)]
pub struct Gate {
    state: Mutex<GateState>,
    changed: Condvar,
}

#[derive(Debug, Default)]
struct GateState {
    running: usize,
    background: usize,
    paused_until: Option<Instant>,
}

/// One request's place at the gate: given back when it is dropped.
#[derive(Debug)]
pub struct Permit<'a> {
    gate: &'a Gate,
    priority: Priority,
}

impl Drop for Permit<'_> {
    fn drop(&mut self) {
        let mut state = self.gate.lock();
        state.running = state.running.saturating_sub(1);
        if self.priority == Priority::Background {
            state.background = state.background.saturating_sub(1);
        }
        drop(state);
        self.gate.changed.notify_all();
    }
}

impl Gate {
    pub fn new() -> Gate {
        Gate::default()
    }

    /// The gate of the mailbox at `address`, the same one for everybody in this process.
    pub fn for_mailbox(address: &str) -> Arc<Gate> {
        static GATES: OnceLock<Mutex<HashMap<String, Arc<Gate>>>> = OnceLock::new();
        let key = address.trim().to_ascii_lowercase();
        let mut gates = GATES.get_or_init(Mutex::default).lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if gates.len() >= MAX_GATES && !gates.contains_key(&key) {
            gates.retain(|_, gate| Arc::strong_count(gate) > 1);
        }
        gates.entry(key).or_default().clone()
    }

    fn lock(&self) -> MutexGuard<'_, GateState> {
        self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Wait for a place, for at most `patience`. While the server asked to be left alone for
    /// longer than that, the answer is its "busy" at once, with how long is left.
    pub fn enter(&self, priority: Priority, patience: Duration) -> Result<Permit<'_>, NetError> {
        let deadline = Instant::now() + patience;
        let mut state = self.lock();
        loop {
            let now = Instant::now();
            if let Some(until) = state.paused_until.filter(|until| *until > now) {
                if until > deadline {
                    let left = until.saturating_duration_since(now);
                    return Err(busy(left));
                }
                state = self.changed.wait_timeout(state, until.saturating_duration_since(now)).unwrap_or_else(|poisoned| poisoned.into_inner()).0;
                continue;
            }
            let room = state.running < MAX_AT_ONCE && (priority == Priority::Owner || state.background < MAX_BACKGROUND);
            if room {
                state.running += 1;
                if priority == Priority::Background {
                    state.background += 1;
                }
                return Ok(Permit { gate: self, priority });
            }
            if now >= deadline {
                return Err(busy(Duration::ZERO));
            }
            state = self.changed.wait_timeout(state, deadline.saturating_duration_since(now)).unwrap_or_else(|poisoned| poisoned.into_inner()).0;
        }
    }

    /// Nobody asks the mailbox's server anything for `wait` (a pause that ends later stays).
    pub fn pause(&self, wait: Duration) {
        let until = Instant::now() + wait;
        let mut state = self.lock();
        if state.paused_until.is_none_or(|now| now < until) {
            state.paused_until = Some(until);
        }
        drop(state);
        self.changed.notify_all();
    }

    /// How long the gate stays shut yet; `None` when it is open.
    pub fn paused_for(&self) -> Option<Duration> {
        let now = Instant::now();
        self.lock().paused_until.filter(|until| *until > now).map(|until| until.saturating_duration_since(now))
    }

    /// How many requests are under way, and how many of them are background work.
    pub fn running(&self) -> (usize, usize) {
        let state = self.lock();
        (state.running, state.background)
    }
}

/// The server's "busy", for a wait of `left` that won't be waited out here.
fn busy(left: Duration) -> NetError {
    let seconds = u32::try_from(left.as_secs().saturating_add(u64::from(left.subsec_nanos() > 0))).unwrap_or(u32::MAX);
    let text = match seconds {
        0 => "Microsoft Graph is busy; try again later".to_string(),
        1 => "Microsoft Graph is busy; try again in a second".to_string(),
        n => format!("Microsoft Graph is busy; try again in {n} seconds"),
    };
    NetError::Busy { text, retry_after: (seconds > 0).then_some(seconds) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn at_most_four_at_once_and_two_of_them_in_the_background() {
        let gate = Gate::new();
        let short = Duration::from_millis(20);
        let a = gate.enter(Priority::Background, short).unwrap();
        let b = gate.enter(Priority::Background, short).unwrap();
        // A third piece of background work waits; the owner doesn't.
        assert!(matches!(gate.enter(Priority::Background, short), Err(NetError::Busy { .. })));
        let c = gate.enter(Priority::Owner, short).unwrap();
        let d = gate.enter(Priority::Owner, short).unwrap();
        assert_eq!(gate.running(), (4, 2));
        // Four at once is all there is, for anybody.
        assert!(gate.enter(Priority::Owner, short).is_err());
        drop(c);
        assert!(gate.enter(Priority::Background, short).is_err(), "two background ones are under way");
        let e = gate.enter(Priority::Owner, short).unwrap();
        drop((a, b, d, e));
        assert_eq!(gate.running(), (0, 0));
    }

    #[test]
    fn threads_never_go_past_the_limits() {
        let gate = Arc::new(Gate::new());
        let (now, most, most_background) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let background_now = Arc::new(AtomicUsize::new(0));
        let threads: Vec<_> = (0..12)
            .map(|n| {
                let (gate, now, most, most_background, background_now) =
                    (gate.clone(), now.clone(), most.clone(), most_background.clone(), background_now.clone());
                std::thread::spawn(move || {
                    let priority = if n % 2 == 0 { Priority::Background } else { Priority::Owner };
                    for _ in 0..5 {
                        let _permit = gate.enter(priority, Duration::from_secs(10)).unwrap();
                        most.fetch_max(now.fetch_add(1, Ordering::SeqCst) + 1, Ordering::SeqCst);
                        if priority == Priority::Background {
                            most_background.fetch_max(background_now.fetch_add(1, Ordering::SeqCst) + 1, Ordering::SeqCst);
                        }
                        std::thread::sleep(Duration::from_millis(2));
                        if priority == Priority::Background {
                            background_now.fetch_sub(1, Ordering::SeqCst);
                        }
                        now.fetch_sub(1, Ordering::SeqCst);
                    }
                })
            })
            .collect();
        threads.into_iter().for_each(|t| t.join().unwrap());
        assert!(most.load(Ordering::SeqCst) <= MAX_AT_ONCE);
        assert!(most_background.load(Ordering::SeqCst) <= MAX_BACKGROUND);
    }

    #[test]
    fn a_pause_holds_everybody_and_then_lets_them_through() {
        let gate = Gate::new();
        gate.pause(Duration::from_millis(150));
        assert!(gate.paused_for().is_some());
        // Somebody who can't wait that long is told at once, with how long it is.
        let started = Instant::now();
        match gate.enter(Priority::Owner, Duration::from_millis(10)) {
            Err(NetError::Busy { retry_after, .. }) => assert_eq!(retry_after, Some(1)),
            other => panic!("{other:?}"),
        }
        assert!(started.elapsed() < Duration::from_millis(100));
        // Somebody who can waits it out.
        let permit = gate.enter(Priority::Owner, Duration::from_secs(5)).unwrap();
        assert!(started.elapsed() >= Duration::from_millis(140));
        assert!(gate.paused_for().is_none());
        drop(permit);
        // A shorter pause doesn't cut a longer one short.
        gate.pause(Duration::from_secs(30));
        gate.pause(Duration::from_millis(1));
        assert!(gate.paused_for().is_some_and(|left| left > Duration::from_secs(20)));
    }

    #[test]
    fn one_gate_per_mailbox() {
        let a = Gate::for_mailbox("Ada@Example.org ");
        let b = Gate::for_mailbox("ada@example.org");
        assert!(Arc::ptr_eq(&a, &b));
        assert!(!Arc::ptr_eq(&a, &Gate::for_mailbox("noor@example.org")));
    }

    #[test]
    fn waits_double_and_the_server_is_believed() {
        let retry = Retry::STANDARD;
        assert_eq!([2, 3, 4, 5].map(|n| retry.wait_before(n, None)), [1, 2, 4, 8].map(Duration::from_secs), "doubling from a second");
        assert_eq!(retry.wait_before(40, None), retry.longest_wait);
        assert_eq!(retry.wait_before(2, Some(7)), Duration::from_secs(7));
        assert!(Kind::Read.may_repeat() && Kind::Same.may_repeat() && !Kind::Once.may_repeat());
    }
}
