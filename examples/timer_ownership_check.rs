//! Callback ownership diagnostic; run on a private display.
use fltk_omarchy::App;
use std::{
    cell::Cell,
    rc::Rc,
    time::{Duration, Instant},
};
struct Payload {
    dropped: Rc<Cell<usize>>,
    _bytes: Vec<u8>,
}
impl Drop for Payload {
    fn drop(&mut self) {
        self.dropped.set(self.dropped.get() + 1);
    }
}
fn pump(done: impl Fn() -> bool) {
    let end = Instant::now() + Duration::from_secs(3);
    while !done() && Instant::now() < end {
        let _ = fltk::app::wait_for(0.002);
        App::pump();
    }
}
fn main() {
    let _app = App::without_theme_watch("dev.omarchy.fltk.timer-ownership");
    let n = 128usize;
    let fired = Rc::new(Cell::new(0));
    let dropped = Rc::new(Cell::new(0));
    for _ in 0..n {
        let guard = Payload {
            dropped: dropped.clone(),
            _bytes: vec![7; 4096],
        };
        let calls = fired.clone();
        fltk::app::add_timeout3(0.0, move |_| {
            let _ = &guard;
            calls.set(calls.get() + 1);
        });
    }
    pump(|| fired.get() == n);
    assert_eq!(fired.get(), n, "callbacks did not fire");
    let fired_drops = dropped.get();
    let canceled = Rc::new(Cell::new(0));
    for _ in 0..n {
        let guard = Payload {
            dropped: canceled.clone(),
            _bytes: vec![7; 4096],
        };
        let h = fltk::app::add_timeout3(60.0, move |_| {
            let _ = &guard;
        });
        fltk::app::remove_timeout3(h);
    }
    println!("{{\"backend\":\"fltk-1.5.23-timeout3\",\"fired\":{},\"fired_drops\":{},\"canceled\":{},\"canceled_drops\":{},\"retained_payload_bytes\":{}}}",fired.get(),fired_drops,n,canceled.get(),(n-fired_drops+n-canceled.get())*4096);
    assert_eq!(
        fired_drops, 0,
        "upstream behavior changed; re-audit the adapter"
    );
    assert_eq!(canceled.get(), 0, "upstream cancellation ownership changed");
    let fired = Rc::new(Cell::new(0));
    let dropped = Rc::new(Cell::new(0));
    let canceled = Rc::new(Cell::new(0));
    for _ in 0..n {
        let guard = Payload {
            dropped: dropped.clone(),
            _bytes: vec![7; 4096],
        };
        let calls = fired.clone();
        fltk_omarchy::timer::after(Duration::ZERO, move || {
            let _ = &guard;
            calls.set(calls.get() + 1);
        });
        let guard = Payload {
            dropped: canceled.clone(),
            _bytes: vec![7; 4096],
        };
        fltk_omarchy::timer::after(Duration::from_secs(60), move || {
            let _ = &guard;
        })
        .cancel();
    }
    pump(|| fired.get() == n);
    assert_eq!((fired.get(), dropped.get(), canceled.get()), (n, n, n));
    let stats = fltk_omarchy::testkit::timer_stats();
    assert_eq!(
        (
            stats.pending,
            stats.queued,
            stats.executing,
            stats.native_armed
        ),
        (0, 0, 0, false)
    );
    println!("{{\"backend\":\"owned-ui-timer\",\"fired\":{},\"fired_drops\":{},\"canceled\":{},\"canceled_drops\":{},\"retained_payload_bytes\":0}}",fired.get(),dropped.get(),n,canceled.get());
}
