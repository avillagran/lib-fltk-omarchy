//! What frames and UI callbacks cost.
//!
//! **Frame log**: with `OMARCHY_FLTK_FRAME_LOG=path` (or [`log_to`]) every
//! frame an `AppWindow` paints adds one JSON line to `path`:
//!
//! ```json
//! {"t":1.204,"window":"Chat","frame":42,"paint_ms":1.8,"cpu_ms":1.7,"wall_ms":16.7,"ui_cpu_ms":2.1,"damage":128,"size":[900,700]}
//! ```
//!
//! `t`: seconds since the log opened; `paint_ms` / `cpu_ms`: wall and
//! UI-thread CPU time of the paint; `wall_ms` / `ui_cpu_ms`: since this
//! window's previous frame started (the frame interval, and all UI-thread
//! work in it: events, callbacks, layout, paint); `damage`: FLTK's damage
//! bits (`128` = full redraw, `4` = scroll, `1` = child). CPU times are
//! `null` where the OS does not report thread CPU time.
//!
//! **Blocked UI thread**: debug builds time the callbacks of buttons,
//! window shortcuts and [`task::post`](crate::task::post). When one takes
//! more than 50 ms longer in wall time than in CPU time (it waited: file
//! or network I/O, a lock, a sleep), stderr gets one warning naming where
//! the callback was set, once per call site and at most once a second.
//! `OMARCHY_FLTK_BLOCK_WARN=0` silences it, `=1` turns it on in release
//! builds. Move such work into a [`Task`](crate::task::Task).

use crate::platform::thread_cpu_time;
use fltk::prelude::*;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::panic::Location;
use std::path::Path;
use std::time::{Duration, Instant};

/// The frame log's environment variable.
pub const FRAME_LOG: &str = "OMARCHY_FLTK_FRAME_LOG";
/// `0` silences the blocked-UI warning, `1` turns it on in release builds.
pub const BLOCK_WARN: &str = "OMARCHY_FLTK_BLOCK_WARN";
/// Wall time over CPU time that counts as blocked.
pub const BLOCK_SLACK: Duration = Duration::from_millis(50);

struct Log {
    out: std::io::BufWriter<std::fs::File>,
    start: Instant,
    /// Per window: frames so far, start and UI CPU of the previous frame.
    last: HashMap<usize, (u64, Instant, Option<Duration>)>,
}

thread_local! {
    static LOG: RefCell<Option<Log>> = const { RefCell::new(None) };
    static ON: Cell<bool> = const { Cell::new(false) };
    static WARN: Cell<Option<bool>> = const { Cell::new(None) };
    static WARNED: RefCell<HashSet<(&'static str, u32, u32)>> = RefCell::new(HashSet::new());
    static LAST_WARN: Cell<Option<Instant>> = const { Cell::new(None) };
    static REPORTS: RefCell<Vec<Blocked>> = const { RefCell::new(Vec::new()) };
}

pub(crate) fn init_from_env() {
    if let Some(p) = std::env::var_os(FRAME_LOG).filter(|p| !p.is_empty()) {
        if let Err(e) = log_to(Some(Path::new(&p))) {
            eprintln!("fltk-omarchy: {FRAME_LOG}={}: {e}", Path::new(&p).display());
        }
    }
}

/// Starts writing the frame log to `path` (truncated), or stops it with
/// `None`. `OMARCHY_FLTK_FRAME_LOG` does the same at `App::new`.
pub fn log_to(path: Option<&Path>) -> std::io::Result<()> {
    let log = match path {
        Some(p) => Some(Log {
            out: std::io::BufWriter::new(std::fs::File::create(p)?),
            start: Instant::now(),
            last: HashMap::new(),
        }),
        None => None,
    };
    ON.with(|o| o.set(log.is_some()));
    LOG.with(|l| *l.borrow_mut() = log);
    Ok(())
}

/// A frame being painted; logged when dropped (the paint ended).
pub(crate) struct Frame {
    win: fltk::window::DoubleWindow,
    damage: u8,
    t0: Instant,
    c0: Option<Duration>,
}

impl Frame {
    /// None (nothing to do) unless the frame log is on.
    #[inline]
    pub(crate) fn begin(win: &fltk::window::DoubleWindow) -> Option<Frame> {
        if !ON.with(|o| o.get()) {
            return None;
        }
        Some(Frame { win: win.clone(), damage: win.damage_type().bits(), t0: Instant::now(), c0: thread_cpu_time() })
    }
}

impl Drop for Frame {
    fn drop(&mut self) {
        let paint = self.t0.elapsed();
        let c1 = thread_cpu_time();
        let cpu = match (self.c0, c1) {
            (Some(a), Some(b)) => Some(b.saturating_sub(a)),
            _ => None,
        };
        let id = self.win.as_widget_ptr() as usize;
        let label = self.win.label();
        let size = (self.win.w(), self.win.h());
        let (t0, c0, damage) = (self.t0, self.c0, self.damage);
        LOG.with(|l| {
            let mut l = l.borrow_mut();
            let Some(log) = l.as_mut() else { return };
            let prev = log.last.get(&id).copied();
            let n = prev.map_or(0, |p| p.0 + 1);
            let wall = prev.map(|p| t0.saturating_duration_since(p.1));
            let ui_cpu = match (prev.and_then(|p| p.2), c0) {
                (Some(a), Some(b)) => Some(b.saturating_sub(a)),
                _ => None,
            };
            log.last.insert(id, (n, t0, c0));
            let line = line(t0.saturating_duration_since(log.start), &label, n, paint, cpu, wall, ui_cpu, damage, size);
            let _ = writeln!(log.out, "{line}").and_then(|()| log.out.flush());
        });
    }
}

fn ms(d: Option<Duration>) -> serde_json::Value {
    d.map_or(serde_json::Value::Null, |d| serde_json::json!((d.as_secs_f64() * 1e5).round() / 100.0))
}

#[allow(clippy::too_many_arguments)]
fn line(
    t: Duration,
    window: &str,
    frame: u64,
    paint: Duration,
    cpu: Option<Duration>,
    wall: Option<Duration>,
    ui_cpu: Option<Duration>,
    damage: u8,
    size: (i32, i32),
) -> String {
    serde_json::json!({
        "t": (t.as_secs_f64() * 1e3).round() / 1e3,
        "window": window,
        "frame": frame,
        "paint_ms": ms(Some(paint)),
        "cpu_ms": ms(cpu),
        "wall_ms": ms(wall),
        "ui_cpu_ms": ms(ui_cpu),
        "damage": damage,
        "size": [size.0, size.1],
    })
    .to_string()
}

// ------------------------------------------------------------ blocked UI

/// One blocked-UI warning: where the callback was set and how long it ran.
#[derive(Debug, Clone, PartialEq)]
pub struct Blocked {
    /// `file:line:column` of the code that set the callback.
    pub site: String,
    pub wall: Duration,
    pub cpu: Duration,
}

fn warn_on() -> bool {
    WARN.with(|w| {
        if let Some(v) = w.get() {
            return v;
        }
        let v = match std::env::var(BLOCK_WARN).ok().as_deref() {
            Some("0") => false,
            Some("1") => true,
            _ => cfg!(debug_assertions),
        };
        w.set(Some(v));
        v
    })
}

/// Turns the blocked-UI warning on or off for this thread (tests), over
/// the build and `OMARCHY_FLTK_BLOCK_WARN` defaults.
pub fn set_block_warnings(on: bool) {
    WARN.with(|w| w.set(Some(on)));
}

/// Warnings issued so far on this thread (tests).
pub fn blocked() -> Vec<Blocked> {
    REPORTS.with(|r| r.borrow().clone())
}

thread_local! {
    static NESTED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Marks the running callback as one that runs a nested event loop (a
/// modal dialog that keeps the UI responsive), so it is not reported as
/// blocked.
pub(crate) fn nested_loop() {
    NESTED.with(|n| n.set(true));
}

/// Runs a UI-thread callback set at `site`, warning when it blocked.
#[inline]
pub(crate) fn watch<R>(site: &'static Location<'static>, f: impl FnOnce() -> R) -> R {
    if !warn_on() {
        return f();
    }
    let (t0, c0) = (Instant::now(), thread_cpu_time());
    let outer = NESTED.with(|n| n.replace(false));
    let r = f();
    let nested = NESTED.with(|n| n.replace(outer));
    if nested {
        // The callback ran a nested event loop (a file dialog): the UI kept
        // going, and the callbacks around this one ran one too.
        NESTED.with(|n| n.set(true));
        return r;
    }
    let wall = t0.elapsed();
    if wall > BLOCK_SLACK {
        if let (Some(a), Some(b)) = (c0, thread_cpu_time()) {
            let cpu = b.saturating_sub(a);
            if wall > cpu + BLOCK_SLACK {
                report(site, wall, cpu);
            }
        }
    }
    r
}

fn report(site: &'static Location<'static>, wall: Duration, cpu: Duration) {
    let key = (site.file(), site.line(), site.column());
    if WARNED.with(|w| w.borrow().contains(&key)) {
        return;
    }
    let now = Instant::now();
    if LAST_WARN.with(|l| l.get()).is_some_and(|t| now.duration_since(t) < Duration::from_secs(1)) {
        return; // rate limit: this site may warn later
    }
    LAST_WARN.with(|l| l.set(Some(now)));
    WARNED.with(|w| w.borrow_mut().insert(key));
    let b = Blocked { site: format!("{}:{}:{}", key.0, key.1, key.2), wall, cpu };
    eprintln!(
        "fltk-omarchy: the UI thread waited {:.0} ms ({:.0} ms CPU) in a callback set at {}: move blocking work into a Task",
        wall.as_secs_f64() * 1e3,
        cpu.as_secs_f64() * 1e3,
        b.site
    );
    REPORTS.with(|r| r.borrow_mut().push(b));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_line_fields() {
        let l = line(
            Duration::from_millis(1204),
            "Chat",
            3,
            Duration::from_micros(1834),
            Some(Duration::from_micros(1700)),
            None,
            None,
            128,
            (900, 700),
        );
        let v: serde_json::Value = serde_json::from_str(&l).unwrap();
        assert_eq!(v["t"], 1.204);
        assert_eq!(v["paint_ms"], 1.83);
        assert_eq!(v["cpu_ms"], 1.7);
        assert!(v["wall_ms"].is_null());
        assert_eq!(v["damage"], 128);
        assert_eq!(v["window"], "Chat");
    }

    #[test]
    fn a_sleeping_callback_warns_once_per_site() {
        set_block_warnings(true);
        let site = Location::caller();
        let busy = |ms: u64| {
            let t = Instant::now();
            while t.elapsed() < Duration::from_millis(ms) {
                std::hint::black_box(0u64);
            }
        };
        watch(site, || busy(80)); // CPU-bound: no warning
        assert!(blocked().is_empty() || thread_cpu_time().is_none());
        watch(site, || std::thread::sleep(Duration::from_millis(80)));
        watch(site, || std::thread::sleep(Duration::from_millis(80)));
        if thread_cpu_time().is_some() {
            let b = blocked();
            assert_eq!(b.len(), 1, "{b:?}");
            assert!(b[0].site.contains("frames.rs") && b[0].wall >= Duration::from_millis(80));
        }
    }
}
