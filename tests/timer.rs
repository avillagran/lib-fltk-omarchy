//! Main-thread runtime checks of callback ownership and nested event loops.
use fltk_omarchy::{
    platform::{Os, OS},
    testkit, timer, App,
};
use std::{
    cell::Cell,
    rc::Rc,
    time::{Duration, Instant},
};
struct Guard(Rc<Cell<usize>>);
impl Drop for Guard {
    fn drop(&mut self) {
        self.0.set(self.0.get() + 1);
    }
}
struct OnDrop(Option<Box<dyn FnOnce()>>);
impl Drop for OnDrop {
    fn drop(&mut self) {
        if let Some(f) = self.0.take() {
            f();
        }
    }
}
fn pump(done: impl Fn() -> bool) {
    let end = Instant::now() + Duration::from_secs(3);
    while !done() && Instant::now() < end {
        let _ = fltk::app::wait_for(0.002);
        App::pump();
    }
    assert!(done(), "timer condition timed out");
}
fn idle() {
    let s = testkit::timer_stats();
    assert_eq!(
        (s.pending, s.queued, s.executing, s.native_armed),
        (0, 0, 0, false),
        "{s:?}"
    );
    assert_eq!(
        s.scheduled,
        s.completed + s.canceled,
        "terminal ownership counters: {s:?}"
    );
}
fn main() {
    if OS == Os::Linux
        && std::env::var_os("DISPLAY").is_none()
        && std::env::var_os("WAYLAND_DISPLAY").is_none()
    {
        println!("timer runtime skipped: no display");
        return;
    }
    let _app = App::without_theme_watch("dev.omarchy.fltk.timer-tests");
    let dropped = Rc::new(Cell::new(0));
    let fired = Rc::new(Cell::new(0));
    for _ in 0..512 {
        let (guard, calls) = (Guard(dropped.clone()), fired.clone());
        timer::after(Duration::ZERO, move || {
            let _ = &guard;
            calls.set(calls.get() + 1);
        });
    }
    pump(|| fired.get() == 512);
    assert_eq!(dropped.get(), 512);
    idle();
    println!("PASS timers: completion drops all captures after a ready-job burst");
    for _ in 0..512 {
        let guard = Guard(dropped.clone());
        let t = timer::after(Duration::from_secs(60), move || {
            let _ = &guard;
            panic!("canceled timer fired");
        });
        assert!(t.is_pending());
        assert!(t.cancel());
        assert!(!t.is_pending());
        assert!(!t.cancel());
    }
    assert_eq!(dropped.get(), 1024);
    idle();
    println!("PASS timers: cancellation releases captures immediately; stale handles are harmless");
    let count = Rc::new(Cell::new(0));
    let n = count.clone();
    let guard = Guard(dropped.clone());
    let before = testkit::timer_stats();
    timer::every(Duration::from_millis(1), move || {
        let _ = &guard;
        n.set(n.get() + 1);
        n.get() < 5
    });
    pump(|| count.get() == 5);
    assert_eq!(dropped.get(), 1025);
    idle();
    assert_eq!(testkit::timer_stats().scheduled - before.scheduled, 1);
    println!("PASS timers: repetition reuses its callback and releases it on termination");
    let slot: Rc<Cell<Option<timer::Timer>>> = Rc::default();
    let s = slot.clone();
    let guard = Guard(dropped.clone());
    let t = timer::every(Duration::from_millis(1), move || {
        let _ = &guard;
        assert!(s.get().unwrap().cancel());
        true
    });
    slot.set(Some(t));
    pump(|| !t.is_pending());
    assert_eq!(dropped.get(), 1026);
    idle();
    println!("PASS timers: canceling inside a repeat cannot resurrect it");
    let inner = Rc::new(Cell::new(false));
    let out = inner.clone();
    let nested = Rc::new(Cell::new(0));
    let depth = nested.clone();
    let outer_done = Rc::new(Cell::new(false));
    let done = outer_done.clone();
    timer::after(Duration::ZERO, move || {
        let until = Instant::now() + Duration::from_secs(1);
        while !out.get() && Instant::now() < until {
            let _ = fltk::app::wait_for(0.002);
        }
        assert!(
            out.get(),
            "preexisting timer did not run in nested event loop"
        );
        done.set(true);
    });
    timer::after(Duration::from_millis(15), move || {
        depth.set(testkit::timer_stats().executing);
        inner.set(true);
    });
    pump(|| outer_done.get());
    assert_eq!(nested.get(), 2);
    idle();
    println!(
        "PASS timers: nested event loops run other jobs without borrowing an executing callback"
    );
    let destructor_done = Rc::new(Cell::new(false));
    let d = destructor_done.clone();
    let guard = OnDrop(Some(Box::new(move || {
        timer::after(Duration::ZERO, move || d.set(true));
    })));
    let t = timer::after(Duration::from_secs(60), move || {
        let _ = &guard;
    });
    assert!(t.cancel());
    pump(|| destructor_done.get());
    idle();
    println!("PASS timers: capture destructors can schedule work without a registry borrow");
    let scoped = timer::after(Duration::from_secs(60), || panic!("guarded timer fired"));
    drop(scoped.guard());
    assert!(!scoped.is_pending());
    idle();
    let detached = Rc::new(Cell::new(false));
    let d = detached.clone();
    let t = timer::after(Duration::ZERO, move || d.set(true))
        .guard()
        .detach();
    pump(|| detached.get());
    assert!(!t.is_pending());
    idle();
    println!("PASS timers: scoped guard cancellation and explicit detachment");
    let guard = Guard(dropped.clone());
    let old = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    timer::every(Duration::from_millis(1), move || {
        let _ = &guard;
        panic!("expected timer panic");
    });
    pump(|| dropped.get() == 1027);
    std::panic::set_hook(old);
    idle();
    let alive = Rc::new(Cell::new(false));
    let out = alive.clone();
    timer::after(Duration::ZERO, move || out.set(true));
    pump(|| alive.get());
    idle();
    println!("PASS timers: callback panic releases captures and later jobs still run");
    // Canceling debounced requests must release work now, not at its
    // original (possibly long) deadline. Only the latest timer stays live.
    use std::sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    };
    struct SendGuard(Arc<AtomicUsize>);
    impl Drop for SendGuard {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    let released = Arc::new(AtomicUsize::new(0));
    let latest = fltk_omarchy::task::Latest::new(Duration::from_secs(60), |_: usize| {
        panic!("canceled Latest ran")
    });
    for value in 0..1024 {
        let guard = SendGuard(released.clone());
        latest.request(move |_| {
            let _ = &guard;
            Ok(value)
        });
        assert_eq!(testkit::timer_stats().pending, 1);
    }
    assert_eq!(released.load(Ordering::SeqCst), 1023);
    latest.cancel();
    assert_eq!(released.load(Ordering::SeqCst), 1024);
    idle();
    let guard = SendGuard(released.clone());
    latest.request(move |_| {
        let _ = &guard;
        Ok(7)
    });
    let clone = latest.clone();
    drop(latest);
    assert_eq!(released.load(Ordering::SeqCst), 1024);
    drop(clone);
    assert_eq!(released.load(Ordering::SeqCst), 1025);
    idle();
    println!("PASS Latest: replacement/cancel/final-owner drop free pending work immediately; one live deadline");
    let entered = Arc::new(AtomicBool::new(false));
    let stopped = Arc::new(AtomicBool::new(false));
    let (e, s) = (entered.clone(), stopped.clone());
    let latest = fltk_omarchy::task::Latest::new(Duration::ZERO, |_: usize| {
        panic!("dropped Latest delivered")
    });
    latest.request(move |ctx| {
        e.store(true, Ordering::SeqCst);
        loop {
            if let Err(error) = ctx.sleep(Duration::from_millis(1)) {
                s.store(true, Ordering::SeqCst);
                return Err(error.into());
            }
        }
    });
    pump(|| entered.load(Ordering::SeqCst));
    drop(latest);
    pump(|| stopped.load(Ordering::SeqCst));
    idle();
    println!("PASS Latest: dropping the final owner cancels running cooperative work");
    // Drive the frame clock directly: real native deadlines must deliver
    // widget damage without allocating an owned callback per frame.
    use fltk::prelude::*;
    let win = fltk::window::DoubleWindow::new(0, 0, 100, 100, "Timer frame clock");
    let mut widget = fltk::widget::Widget::new(0, 0, 100, 100, "");
    win.end();
    let before = testkit::timer_stats();
    for _ in 0..64 {
        widget.clear_damage();
        fltk_omarchy::anim::request_redraw_at(&widget, fltk_omarchy::anim::Pace::Display);
        assert_eq!(fltk_omarchy::anim::pending(), 1);
        pump(|| fltk_omarchy::anim::pending() == 0);
        assert!(widget.damage(), "frame clock did not deliver redraw");
    }
    assert_eq!(
        testkit::timer_stats().scheduled,
        before.scheduled,
        "animation allocated owned timer callbacks"
    );
    fltk::window::DoubleWindow::delete(win);
    idle();
    println!("PASS animation clock: 64 native frame deadlines deliver redraws, zero owned callback allocations, timer stops");
    println!("timer ownership {:?}", testkit::timer_stats());
}
