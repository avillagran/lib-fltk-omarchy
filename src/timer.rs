//! UI-thread timers with owned callbacks and stable cancellation handles.
//!
//! Completed/canceled timers release their captures. One native timeout
//! serves all pending deadlines; no thread or periodic idle poll is used.
//! The detached [`Timer`] handle may be dropped without canceling the job.
//! Keep it to cancel explicitly, or use [`Timer::guard`] for scoped ownership.
//! Create and operate timers on the FLTK UI thread after creating [`crate::App`].
//! Callbacks must stay short; use [`crate::task::Task`] for work.
//! Animation uses the shared [`crate::anim`] frame clock instead of timers.
use fltk::app;
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeSet, HashMap},
    time::{Duration, Instant},
};

type Callback = Box<dyn FnMut(Timer)>;
/// A stable timer identity; stale handles never refer to a newer job.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timer(u64);
impl Timer {
    /// Cancels pending work and drops its captures. If called from the
    /// running callback, captures drop after that invocation returns.
    pub fn cancel(self) -> bool {
        cancel(self)
    }
    /// True while pending or executing (unless explicitly canceled).
    pub fn is_pending(self) -> bool {
        ui();
        TIMERS.with(|t| t.borrow().jobs.contains_key(&self))
    }
    /// Cancels on guard drop. Retain the guard for the desired lifetime.
    pub fn guard(self) -> TimerGuard {
        ui();
        TimerGuard(Some(self), std::marker::PhantomData)
    }
}
/// Scoped cancellation. The handle itself is detached; this guard is not.
/// The guard stays on the UI thread, like the callback it owns.
///
/// ```compile_fail
/// let guard = fltk_omarchy::timer::after(std::time::Duration::from_secs(1), || {}).guard();
/// std::thread::spawn(move || drop(guard));
/// ```
#[must_use = "dropping the guard cancels the timer"]
pub struct TimerGuard(Option<Timer>, std::marker::PhantomData<std::rc::Rc<()>>);
impl TimerGuard {
    /// Gives back the detached handle without canceling.
    pub fn detach(mut self) -> Timer {
        self.0.take().expect("live guard")
    }
}
impl Drop for TimerGuard {
    fn drop(&mut self) {
        if let Some(timer) = self.0.take() {
            timer.cancel();
        }
    }
}
/// Schedules a one-shot UI callback. Dropping its returned handle does not
/// cancel it. Call on the FLTK UI thread after creating the application.
pub fn after(delay: Duration, f: impl FnOnce() + 'static) -> Timer {
    let mut f = Some(f);
    add(delay.as_secs_f64(), move |_| {
        if let Some(f) = f.take() {
            f();
        }
    })
}
/// Repeats while `f` returns true. A false return, explicit cancellation,
/// or panic terminates the job and releases its captures. A slow callback
/// skips missed periods instead of running a catch-up burst.
/// Call on the FLTK UI thread. A zero interval panics.
pub fn every(interval: Duration, mut f: impl FnMut() -> bool + 'static) -> Timer {
    assert!(
        !interval.is_zero(),
        "repeating UI timers need a positive interval"
    );
    let seconds = interval.as_secs_f64();
    add(seconds, move |id| {
        if f() {
            repeat(seconds, id);
        }
    })
}

/// Read-only ownership counters, exposed through `testkit::timer_stats`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// Live jobs, including executing jobs which have not been canceled.
    pub pending: usize,
    /// Deadlines waiting for dispatch; executing jobs are not queued.
    pub queued: usize,
    /// Callbacks on the stack, including canceled callbacks until return.
    pub executing: usize,
    /// Maximum simultaneous live jobs since process startup.
    pub peak_pending: usize,
    /// Created jobs; repeating a job does not increment this counter.
    pub scheduled: u64,
    /// Callback invocations, including repetitions and panics.
    pub invoked: u64,
    /// Jobs terminated by return or panic, excluding explicit cancellation.
    pub completed: u64,
    /// Successful explicit cancellations, including guard drops.
    pub canceled: u64,
    /// Whether the shared native deadline callback is armed.
    pub native_armed: bool,
}
struct Job {
    callback: Option<Callback>,
    /// During execution this is a requested next deadline, not yet queued.
    when: Option<Instant>,
    expired: Instant,
}
#[derive(Default)]
struct Timers {
    next: u64,
    jobs: HashMap<Timer, Job>,
    due: BTreeSet<(Instant, Timer)>,
    stats: Stats,
}
impl Timers {
    fn compact(&mut self) {
        if self.jobs.capacity() > 256 && self.jobs.len().saturating_mul(4) < self.jobs.capacity() {
            self.jobs.shrink_to(self.jobs.len().max(64));
        }
    }
    fn first(&self) -> Option<(Instant, Timer)> {
        self.due.first().copied()
    }
}
thread_local! {
    static TIMERS: RefCell<Timers> = RefCell::new(Timers::default());
    static ARMED: Cell<Option<(Instant, Timer)>> = const { Cell::new(None) };
}
fn ui() {
    assert!(app::is_ui_thread(), "UI timers require the FLTK UI thread");
}
fn deadline(seconds: f64, anchor: Instant) -> Instant {
    let delay = Duration::try_from_secs_f64(seconds.max(0.0)).expect("invalid UI timer delay");
    assert!(seconds.is_finite(), "invalid UI timer delay");
    anchor
        .checked_add(delay)
        .expect("UI timer deadline overflow")
}
/// Compatibility entry point for internal callbacks which select their
/// next delay explicitly. Unlike FLTK timeout3, the callback is owned.
pub(crate) fn add(seconds: f64, f: impl FnMut(Timer) + 'static) -> Timer {
    ui();
    let when = deadline(seconds, Instant::now());
    let id = TIMERS.with(|t| {
        let mut t = t.borrow_mut();
        t.next = t
            .next
            .checked_add(1)
            .expect("UI timer identities exhausted");
        let id = Timer(t.next);
        t.jobs.insert(
            id,
            Job {
                callback: Some(Box::new(f)),
                when: Some(when),
                expired: when,
            },
        );
        t.due.insert((when, id));
        t.stats.scheduled += 1;
        t.stats.peak_pending = t.stats.peak_pending.max(t.jobs.len());
        id
    });
    arm(false);
    id
}
/// Called by a running callback. No queue entry exists for that callback
/// until it returns, so a nested event loop can run other timers safely.
pub(crate) fn repeat(seconds: f64, id: Timer) -> bool {
    ui();
    let updated = TIMERS.with(|t| {
        let mut t = t.borrow_mut();
        let Some(job) = t.jobs.get_mut(&id).filter(|j| j.callback.is_none()) else {
            return false;
        };
        let now = Instant::now();
        let next = deadline(seconds, job.expired);
        job.when = Some(if next <= now {
            now + Duration::from_millis(1)
        } else {
            next
        });
        true
    });
    updated
}
pub(crate) fn cancel(id: Timer) -> bool {
    ui();
    // Extract before dropping: capture destructors may schedule/cancel jobs.
    let removed = TIMERS.with(|t| {
        let mut t = t.borrow_mut();
        let job = t.jobs.remove(&id)?;
        if job.callback.is_some() {
            if let Some(when) = job.when {
                t.due.remove(&(when, id));
            }
        }
        t.stats.canceled += 1;
        t.compact();
        Some(job)
    });
    let found = removed.is_some();
    arm(false);
    drop(removed);
    found
}
pub(crate) fn stats() -> Stats {
    ui();
    TIMERS.with(|t| {
        let t = t.borrow();
        Stats {
            pending: t.jobs.len(),
            queued: t.due.len(),
            native_armed: ARMED.with(|a| a.get().is_some()),
            ..t.stats
        }
    })
}
fn arm(yield_turn: bool) {
    let first = TIMERS.with(|t| t.borrow().first());
    if !yield_turn && ARMED.with(|a| a.get()) == first {
        return;
    }
    app::remove_raw_timeout(tick, std::ptr::null_mut());
    ARMED.with(|a| a.set(first));
    if let Some((when, _)) = first {
        let wait = when.saturating_duration_since(Instant::now()).as_secs_f64();
        app::add_raw_timeout(
            if yield_turn { wait.max(0.001) } else { wait },
            tick,
            std::ptr::null_mut(),
        );
    }
}
fn tick(_: *mut ()) {
    // The raw FLTK bridge does not catch Rust panics. Application callback
    // panics are handled per job; this boundary also protects the C ABI.
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(run));
}
fn run() {
    ARMED.with(|a| a.set(None));
    for _ in 0..64 {
        let next = TIMERS.with(|t| {
            let mut t = t.borrow_mut();
            let (when, id) = t.first()?;
            if when > Instant::now() {
                return None;
            }
            t.due.remove(&(when, id));
            let job = t.jobs.get_mut(&id).expect("queued timer exists");
            job.when = None;
            job.expired = when;
            let callback = job.callback.take().expect("queued callback is not running");
            t.stats.executing += 1;
            t.stats.invoked += 1;
            Some((id, callback))
        });
        let Some((id, mut callback)) = next else {
            arm(false);
            return;
        };
        // Existing future deadlines must remain armed while application
        // code runs a nested event loop (e.g. a native modal picker).
        arm(false);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| callback(id)));
        let mut returned = Some(callback);
        TIMERS.with(|t| {
            let mut t = t.borrow_mut();
            t.stats.executing -= 1;
            // Cancellation from the callback already removed its identity.
            let Some(job) = t.jobs.get_mut(&id) else {
                return;
            };
            if let (true, Some(when)) = (result.is_ok(), job.when) {
                job.callback = returned.take();
                t.due.insert((when, id));
            } else {
                t.jobs.remove(&id);
                t.stats.completed += 1;
                t.compact();
            }
        });
        drop(returned); // no registry borrow while captures are destroyed
    }
    arm(true); // a ready-job burst must yield to input/paint
}
