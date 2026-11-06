//! Motion: Omarchy's transitions for control states (Button 120 ms color,
//! PanelActionButton and rows 60 ms, Toggle 100 ms, ToggleSwitch 120 ms
//! color and knob, PanelSlider 140 ms OutCubic), layout changes (see
//! [`crate::layout`]), overlays, tab indicators and list rows.
//!
//! One shared fltk timer redraws only the widgets that are mid-transition
//! (60 fps) and stops when none is: an idle window costs no CPU. Theme
//! changes are not animated.
//!
//! ## On or off
//!
//! Animations follow the system's motion setting, live (docs/ANIMATION.md):
//!
//! | OS | Setting read |
//! |---|---|
//! | Linux, Hyprland (Omarchy) | `animations:enabled` over Hyprland's socket, re-read on `configreloaded` and focus changes |
//! | Linux, GNOME and friends | `org.gnome.desktop.interface enable-animations` |
//! | macOS | `NSWorkspace.accessibilityDisplayShouldReduceMotion` |
//! | Windows | `SystemParametersInfo(SPI_GETCLIENTAREAANIMATION)` |
//!
//! The app can force it ([`set_enabled`]`(true / false)`, `None` follows
//! the system again), and so can the user with `OMARCHY_FLTK_ANIM=on|off`
//! (`OMARCHY_FLTK_ANIMATIONS=0` still works). The app's choice wins, then
//! the variable, then the system; with no answer animations are on.
//! Turned off, every component jumps to its final state.

use crate::theme::Rgb;
use fltk::{app, prelude::*, widget::Widget};
use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

thread_local! {
    static FORCED: Cell<Option<bool>> = const { Cell::new(None) };
    static ENV: Cell<Option<Option<bool>>> = const { Cell::new(None) };
    static TIMER: Cell<bool> = const { Cell::new(false) };
    static CLOCK: Cell<Option<Instant>> = const { Cell::new(None) };
    static LAST_KICK: Cell<Option<Instant>> = const { Cell::new(None) };
}

// The system setting: 0 unknown, 1 on, 2 off. Written by the watcher thread.
static SYSTEM: AtomicU8 = AtomicU8::new(0);
static WATCHING: AtomicBool = AtomicBool::new(false);
// Set by `init` (an fltk app exists): changes wake its event loop.
static GUI: AtomicBool = AtomicBool::new(false);
static KICK: Mutex<Option<std::sync::mpsc::Sender<()>>> = Mutex::new(None);

/// Where the current on/off answer comes from ([`source`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// [`set_enabled`] with `Some`.
    App,
    /// `OMARCHY_FLTK_ANIM` / `OMARCHY_FLTK_ANIMATIONS`.
    Env,
    /// The OS motion setting ([`system`]).
    System,
    /// Nothing answered: on.
    Default,
}

/// Are animations on? The app's [`set_enabled`] first, then
/// `OMARCHY_FLTK_ANIM=on|off`, then the system setting ([`system`]); on
/// when nothing answers.
pub fn enabled() -> bool {
    if let Some(on) = FORCED.with(|f| f.get()) {
        return on;
    }
    if let Some(on) = env() {
        return on;
    }
    system().unwrap_or(true)
}

/// What decided [`enabled`].
pub fn source() -> Source {
    if FORCED.with(|f| f.get()).is_some() {
        Source::App
    } else if env().is_some() {
        Source::Env
    } else if system().is_some() {
        Source::System
    } else {
        Source::Default
    }
}

/// Forces animations on or off for this app (`true` / `false`), or
/// follows the system again (`None`). Tests and screenshots that need
/// final states call `set_enabled(false)`.
pub fn set_enabled(on: impl Into<Option<bool>>) {
    FORCED.with(|f| f.set(on.into()));
}

/// `OMARCHY_FLTK_ANIM=on|off|1|0|true|false` (or the older
/// `OMARCHY_FLTK_ANIMATIONS`), read once.
fn env() -> Option<bool> {
    ENV.with(|e| {
        if let Some(v) = e.get() {
            return v;
        }
        let v = ["OMARCHY_FLTK_ANIM", "OMARCHY_FLTK_ANIMATIONS"]
            .iter()
            .find_map(|k| std::env::var(k).ok().and_then(|v| parse_switch(&v)));
        e.set(Some(v));
        v
    })
}

/// `on`/`off`, `1`/`0`, `true`/`false`, `yes`/`no` (any case).
pub(crate) fn parse_switch(v: &str) -> Option<bool> {
    match v.trim().to_ascii_lowercase().as_str() {
        "1" | "on" | "true" | "yes" => Some(true),
        "0" | "off" | "false" | "no" => Some(false),
        _ => None,
    }
}

/// The OS motion setting: `Some(true)` animations on, `Some(false)` off
/// (reduce motion), `None` unknown or not read yet. A background thread
/// reads it (blocked while idle) and follows changes; see the module docs.
pub fn system() -> Option<bool> {
    start_watch();
    match SYSTEM.load(Ordering::Relaxed) {
        1 => Some(true),
        2 => Some(false),
        _ => None,
    }
}

/// Starts following the system setting and waits a little (≤ 50 ms) for
/// the first answer, so the first frames already follow it. [`crate::App`]
/// calls it; it is cheap to call again.
pub fn init() {
    GUI.store(true, Ordering::Relaxed);
    // `OMARCHY_FLTK_ANIM` decides: the system setting is not needed.
    if env().is_some() {
        return;
    }
    if WATCHING.load(Ordering::Relaxed) {
        return;
    }
    let first = start_watch();
    if let Some(rx) = first {
        let _ = rx.recv_timeout(Duration::from_millis(50));
    }
}

fn store(v: Option<bool>) -> bool {
    let n = match v {
        Some(true) => 1,
        Some(false) => 2,
        None => 0,
    };
    SYSTEM.swap(n, Ordering::Relaxed) != n
}

/// One thread for the life of the process: reads the setting, then waits
/// for a reason to read it again (a Hyprland event, or [`kick`] when an
/// animation starts). Never polls.
fn start_watch() -> Option<std::sync::mpsc::Receiver<()>> {
    if WATCHING.swap(true, Ordering::Relaxed) {
        return None;
    }
    let (kick_tx, kick_rx) = std::sync::mpsc::channel::<()>();
    let (first_tx, first_rx) = std::sync::mpsc::channel::<()>();
    if let Ok(mut k) = KICK.lock() {
        *k = Some(kick_tx.clone());
    }
    let spawned = std::thread::Builder::new().name("omarchy-motion".into()).spawn(move || {
        store(crate::platform::animations());
        let _ = first_tx.send(());
        // Hyprland: a second thread blocks on the event socket and kicks.
        crate::platform::watch_animations(kick_tx);
        while kick_rx.recv().is_ok() {
            // Several events in a row (a reload sends many): read once.
            while kick_rx.try_recv().is_ok() {}
            if store(crate::platform::animations()) && GUI.load(Ordering::Relaxed) {
                app::awake();
            }
        }
    });
    if spawned.is_err() {
        return None;
    }
    Some(first_rx)
}

/// Asks the watcher to read the system setting again, at most every 2 s
/// (called when an animation starts, so a change made while the app was
/// idle is picked up without polling).
pub(crate) fn kick() {
    if FORCED.with(|f| f.get()).is_some() || env().is_some() || !WATCHING.load(Ordering::Relaxed) {
        return;
    }
    let now = Instant::now();
    let due = LAST_KICK.with(|l| {
        let due = l.get().map_or(true, |t| now.duration_since(t) >= Duration::from_secs(2));
        if due {
            l.set(Some(now));
        }
        due
    });
    if due {
        if let Ok(k) = KICK.lock() {
            if let Some(tx) = k.as_ref() {
                let _ = tx.send(());
            }
        }
    }
}

/// Hyprland's `getoption` reply: `{"bool": true}` (0.50+) or `{"int": 1}`.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn parse_hyprland_option(json: &str) -> Option<bool> {
    let v: serde_json::Value = serde_json::from_str(json.trim()).ok()?;
    if let Some(b) = v.get("bool").and_then(|b| b.as_bool()) {
        return Some(b);
    }
    v.get("int").and_then(|i| i.as_i64()).map(|i| i != 0)
}

// ================================================================== clock

/// The animation clock: the real time, or a frozen time set by
/// [`clock::freeze`] (tests that look at intermediate frames).
pub fn now() -> Instant {
    CLOCK.with(|c| c.get()).unwrap_or_else(Instant::now)
}

/// A native transparency capture draws twice. Both passes must observe
/// one animation time; preserve an outer deterministic test clock.
#[cfg(feature="gpu")]
pub(crate) fn sample_time<T>(f: impl FnOnce() -> T) -> T {
    struct Restore(Option<Instant>);
    impl Drop for Restore {fn drop(&mut self){CLOCK.with(|c|c.set(self.0));}}
    let at=now();
    let _restore=Restore(CLOCK.with(|c|c.replace(Some(at))));
    f()
}

/// A controllable clock for tests and screenshots of intermediate frames.
pub mod clock {
    use super::CLOCK;
    use std::time::{Duration, Instant};

    /// Stops time where it is now: animations stay on the current frame.
    pub fn freeze() {
        CLOCK.with(|c| c.set(Some(c.get().unwrap_or_else(Instant::now))));
    }

    /// Moves the frozen clock forward (freezes it first if needed).
    pub fn advance(d: Duration) {
        freeze();
        CLOCK.with(|c| c.set(c.get().map(|t| t + d)));
    }

    /// Real time again.
    pub fn resume() {
        CLOCK.with(|c| c.set(None));
    }

    /// True while frozen.
    pub fn frozen() -> bool {
        CLOCK.with(|c| c.get().is_some())
    }
}

// ============================================================== durations

/// Omarchy and o3 durations, in milliseconds, with where they come from
/// (docs/ANIMATION.md has the full table).
pub mod ms {
    /// Layout geometry (sizes, positions, sidebar width): o3
    /// `PinnedKeyboardPanel` `Behavior on x/y/width/height`, 190 ms OutCubic.
    pub const LAYOUT: u32 = 190;
    /// A popup card appearing: Omarchy `PopupCard`, 180 ms `Style.easeOut`.
    pub const POPUP_IN: u32 = 180;
    /// A popup card going: Omarchy `PopupCard`, 120 ms.
    pub const POPUP_OUT: u32 = 120;
    /// Selection indicators and chip strips: o3 group chips and Omarchy
    /// clock panel tabs, 160 ms OutCubic.
    pub const INDICATOR: u32 = 160;
    /// A list row moving to another place: o3 `rowMoveMs` (T3RowFlight), 520 ms.
    pub const ROW_MOVE: u32 = 520;
    /// Rows making room for it: o3 `RowDisplaced`, 0.85 × 520 ms OutCubic.
    pub const ROW_DISPLACED: u32 = 442;
    /// A row arriving: o3 `T3ThreadRow.arrive`, 360 ms OutCubic.
    pub const ROW_ARRIVE: u32 = 360;
    /// A row leaving: o3 `T3ThreadRow.depart`, 300 ms InCubic.
    pub const ROW_DEPART: u32 = 300;
    /// A card growing toward new content (tree branches): Omarchy
    /// `KeyboardPanel` `drawnHeight`, 200 ms `Style.easeOut`.
    pub const REVEAL: u32 = 200;
    /// Hover intent before a hover-opened surface opens: o3 composer
    /// quick menu, 350 ms.
    pub const HOVER_OPEN: u32 = 350;
    /// Grace after the pointer leaves before it closes: o3 thread preview
    /// `closeDelay`, 140 ms.
    pub const HOVER_CLOSE: u32 = 140;
    /// Hover intent before a `PopupMenu` submenu opens (and an open sibling
    /// closes): desktop menus, close to GTK's `gtk-menu-popup-delay` (225 ms).
    pub const SUBMENU_OPEN: u32 = 200;
    /// Grace while the pointer heads for an open submenu across other
    /// entries: they do not take over until it rests there this long.
    pub const SUBMENU_AIM: u32 = 300;
}

// ================================================================= easing

/// Easing curves used by Omarchy and o3.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Ease {
    /// Qt's ColorAnimation default.
    Linear,
    /// Easing.OutCubic (positions, sizes).
    OutCubic,
    /// Easing.InCubic (leaving).
    InCubic,
    /// Easing.InOutCubic (o3 row moves, T3Reveal).
    InOutCubic,
    /// Easing.InOutQuart (o3 row flight on an arc).
    InOutQuart,
    /// Easing.OutQuint.
    OutQuint,
    /// Easing.OutBack with Qt's overshoot (1.70158 is Qt's default).
    OutBack(f64),
    /// Omarchy `Style.easeOut`: cubic Bézier (0.23, 1.0, 0.32, 1.0), "a
    /// strong ease-out, for anything appearing or moving in answer to the
    /// pointer: it starts at once and settles without a bounce".
    EaseOut,
}

impl Ease {
    pub fn apply(self, t: f64) -> f64 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Ease::Linear => t,
            Ease::OutCubic => 1.0 - (1.0 - t).powi(3),
            Ease::InCubic => t * t * t,
            Ease::InOutCubic => {
                if t < 0.5 {
                    4.0 * t * t * t
                } else {
                    1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
                }
            }
            Ease::InOutQuart => {
                if t < 0.5 {
                    8.0 * t.powi(4)
                } else {
                    1.0 - (-2.0 * t + 2.0).powi(4) / 2.0
                }
            }
            Ease::OutQuint => 1.0 - (1.0 - t).powi(5),
            Ease::OutBack(s) => {
                let u = t - 1.0;
                1.0 + u * u * ((s + 1.0) * u + s)
            }
            Ease::EaseOut => bezier(0.23, 1.0, 0.32, 1.0, t),
        }
    }
}

/// CSS/Qt cubic Bézier easing from (0,0) to (1,1) through (x1,y1), (x2,y2).
fn bezier(x1: f64, y1: f64, x2: f64, y2: f64, x: f64) -> f64 {
    if x <= 0.0 || x >= 1.0 {
        return x.clamp(0.0, 1.0);
    }
    let c = |a: f64, b: f64, s: f64| {
        let u = 1.0 - s;
        3.0 * u * u * s * a + 3.0 * u * s * s * b + s * s * s
    };
    let dc = |a: f64, b: f64, s: f64| {
        let u = 1.0 - s;
        3.0 * u * u * a + 6.0 * u * s * (b - a) + 3.0 * s * s * (1.0 - b)
    };
    // Newton on x(s) = x, bisection when the slope is flat.
    let mut s = x;
    for _ in 0..8 {
        let err = c(x1, x2, s) - x;
        if err.abs() < 1e-6 {
            return c(y1, y2, s);
        }
        let d = dc(x1, x2, s);
        if d.abs() < 1e-6 {
            break;
        }
        s = (s - err / d).clamp(0.0, 1.0);
    }
    let (mut lo, mut hi) = (0.0, 1.0);
    s = x;
    for _ in 0..40 {
        let v = c(x1, x2, s);
        if (v - x).abs() < 1e-7 {
            break;
        }
        if v < x {
            lo = s
        } else {
            hi = s
        }
        s = (lo + hi) / 2.0;
    }
    c(y1, y2, s)
}

/// A one-shot progress from 0 to 1: `Tween::start(ms, ease)`, then
/// [`Tween::value`] each frame until [`Tween::done`].
#[derive(Debug, Clone, Copy)]
pub struct Tween {
    start: Instant,
    ms: u32,
    ease: Ease,
}

impl Tween {
    pub fn start(ms: u32, ease: Ease) -> Tween {
        kick();
        Tween { start: now(), ms: ms.max(1), ease }
    }

    /// Linear progress 0..=1.
    pub fn t(&self) -> f64 {
        (now().saturating_duration_since(self.start).as_secs_f64() * 1000.0 / self.ms as f64).clamp(0.0, 1.0)
    }

    /// Eased progress (may overshoot 1 with OutBack).
    pub fn value(&self) -> f64 {
        self.ease.apply(self.t())
    }

    pub fn done(&self) -> bool {
        self.t() >= 1.0
    }
}

/// Values that can be interpolated.
pub trait Lerp: Copy + PartialEq {
    fn lerp(a: Self, b: Self, t: f64) -> Self;
}

impl Lerp for f64 {
    fn lerp(a: f64, b: f64, t: f64) -> f64 {
        a + (b - a) * t
    }
}

impl Lerp for i32 {
    fn lerp(a: i32, b: i32, t: f64) -> i32 {
        (a as f64 + (b - a) as f64 * t).round() as i32
    }
}

impl Lerp for Rgb {
    fn lerp(a: Rgb, b: Rgb, t: f64) -> Rgb {
        b.over(a, t)
    }
}

/// What a control background looks like: fill, border color and width.
/// "No fill" is the color under the control; "no border" is width 0.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Look {
    pub fill: Rgb,
    pub border: Rgb,
    pub border_width: i32,
}

impl Lerp for Look {
    fn lerp(a: Look, b: Look, t: f64) -> Look {
        // A border that appears or disappears fades from/to the fill color.
        let ab = if a.border_width > 0 { a.border } else { a.fill };
        let bb = if b.border_width > 0 { b.border } else { b.fill };
        Look {
            fill: Rgb::lerp(a.fill, b.fill, t),
            border: Rgb::lerp(ab, bb, t),
            border_width: if t >= 1.0 { b.border_width } else { a.border_width.max(b.border_width) },
        }
    }
}

/// One animated value of a widget. Keep it next to the widget's draw
/// state; call [`Anim::get`] with the target every draw.
#[derive(Debug, Clone)]
pub struct Anim<T: Lerp> {
    from: Option<T>,
    to: Option<T>,
    start: Option<Instant>,
    theme: u64,
}

impl<T: Lerp> Default for Anim<T> {
    fn default() -> Self {
        Anim { from: None, to: None, start: None, theme: 0 }
    }
}

impl<T: Lerp> Anim<T> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Current value moving toward `target` over `ms` milliseconds. While a
    /// transition runs, `widget` is redrawn on the next frame.
    pub fn get<W: WidgetExt>(&mut self, target: T, ms: u32, ease: Ease, widget: &W) -> T {
        let v = self.value(target, ms, ease);
        if self.start.is_some() {
            request_redraw(widget);
        }
        v
    }

    /// Like [`Anim::get`] without scheduling a redraw (the caller does).
    pub fn value(&mut self, target: T, ms: u32, ease: Ease) -> T {
        let gen = crate::theme::generation();
        let (Some(to), true, true) = (self.to, enabled(), self.theme == gen) else {
            // First draw, transitions off or a new theme: jump.
            self.from = Some(target);
            self.to = Some(target);
            self.start = None;
            self.theme = gen;
            return target;
        };
        let now = now();
        let current = self.current(now, ms, ease).unwrap_or(to);
        if target != to {
            self.from = Some(current);
            self.to = Some(target);
            self.start = Some(now);
            kick();
        }
        self.current(now, ms, ease).unwrap_or(target)
    }

    fn current(&mut self, now: Instant, ms: u32, ease: Ease) -> Option<T> {
        let (from, to, start) = (self.from?, self.to?, self.start?);
        let t = now.saturating_duration_since(start).as_secs_f64() * 1000.0 / ms.max(1) as f64;
        if t >= 1.0 {
            self.start = None;
            self.from = Some(to);
            return Some(to);
        }
        Some(T::lerp(from, to, ease.apply(t)))
    }

    /// Sets the value with no transition (e.g. while dragging).
    pub fn jump(&mut self, v: T) -> T {
        self.from = Some(v);
        self.to = Some(v);
        self.start = None;
        self.theme = crate::theme::generation();
        v
    }

    /// True while a transition runs.
    pub fn running(&self) -> bool {
        self.start.is_some()
    }

    /// The value it is heading to (None before the first [`Anim::get`]).
    pub fn target(&self) -> Option<T> {
        self.to
    }
}

/// How often an animation wants frames (o3-code's `T3Ticker` paces). The
/// shared timer runs at the fastest pace any widget asked for; a widget
/// with a slower pace is redrawn only when its own interval has passed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Pace {
    /// 8 fps: ambient, decorative motion (a slow glow, an idle indicator).
    Saver,
    /// 30 fps: busy indicators, small effects.
    Low,
    /// 60 fps: control and layout transitions (the default).
    Normal,
    /// The monitor's refresh rate (Hyprland reports it; 60 elsewhere):
    /// scrolling and motion that should feel as smooth as the display.
    Display,
}

/// A global limit on [`Pace`]s, for battery or for show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaceMode {
    /// Every pace at most [`Pace::Low`] (30 fps).
    Saver,
    /// Paces as asked.
    Normal,
    /// Every pace at [`Pace::Display`].
    Max,
}

impl Pace {
    /// This pace under `mode`.
    pub fn under(self, mode: PaceMode) -> Pace {
        match mode {
            PaceMode::Saver => self.min(Pace::Low),
            PaceMode::Normal => self,
            PaceMode::Max => Pace::Display,
        }
    }

    /// Time between frames at this pace on a `display_hz` monitor.
    pub fn interval(self, display_hz: f64) -> Duration {
        let fps = match self {
            Pace::Saver => 8.0,
            Pace::Low => 30.0,
            Pace::Normal => 60.0,
            Pace::Display => display_hz.clamp(30.0, 500.0),
        };
        Duration::from_secs_f64(1.0 / fps)
    }
}

/// Frame scheduling for paced widgets (pure, so it can be tested).
#[derive(Debug, Default)]
pub struct Schedule {
    /// (widget id, pace, last frame drawn by the clock or, for a new
    /// request, when it was asked)
    waiting: Vec<(usize, Pace, Instant)>,
    last: Vec<(usize, Instant)>,
}

impl Schedule {
    /// Asks for a frame for `id` at `pace` (a faster pace wins over a
    /// pending slower one).
    pub fn request(&mut self, id: usize, pace: Pace, now: Instant) {
        let base = self.last.iter().find(|(i, _)| *i == id).map_or(now, |(_, t)| *t);
        match self.waiting.iter_mut().find(|(i, _, _)| *i == id) {
            Some(w) => w.1 = w.1.max(pace),
            None => self.waiting.push((id, pace, base)),
        }
    }

    /// How long until the next widget is due, or None when nothing waits.
    pub fn next_wait(&self, now: Instant, hz: f64) -> Option<Duration> {
        self.waiting
            .iter()
            .map(|(_, p, base)| (*base + p.interval(hz)).saturating_duration_since(now))
            .min()
    }

    /// The widgets due at `now` (removed from the queue, remembered as drawn).
    pub fn take_due(&mut self, now: Instant, hz: f64) -> Vec<usize> {
        let mut due = Vec::new();
        self.waiting.retain(|(id, p, base)| {
            // A little early is fine: timers wake a bit late or early.
            let ready = now + Duration::from_millis(2) >= *base + p.interval(hz);
            if ready {
                due.push(*id);
            }
            !ready
        });
        for id in &due {
            match self.last.iter_mut().find(|(i, _)| i == id) {
                Some(l) => l.1 = now,
                None => self.last.push((*id, now)),
            }
        }
        // Forget widgets idle for a while (their next frame starts fresh).
        self.last.retain(|(_, t)| now.saturating_duration_since(*t) < Duration::from_secs(2));
        due
    }

    /// Drops a widget (deleted).
    pub fn forget(&mut self, id: usize) {
        self.waiting.retain(|(i, _, _)| *i != id);
        self.last.retain(|(i, _)| *i != id);
    }

    pub fn len(&self) -> usize {
        self.waiting.len()
    }

    pub fn is_empty(&self) -> bool {
        self.waiting.is_empty()
    }
}

static PACE_MODE: AtomicU8 = AtomicU8::new(1);

/// Sets the global [`PaceMode`] (default `Normal`).
pub fn set_pace_mode(mode: PaceMode) {
    PACE_MODE.store(
        match mode {
            PaceMode::Saver => 0,
            PaceMode::Normal => 1,
            PaceMode::Max => 2,
        },
        Ordering::Relaxed,
    );
}

/// The global [`PaceMode`].
pub fn pace_mode() -> PaceMode {
    match PACE_MODE.load(Ordering::Relaxed) {
        0 => PaceMode::Saver,
        2 => PaceMode::Max,
        _ => PaceMode::Normal,
    }
}

thread_local! {
    static SCHEDULE: RefCell<Schedule> = RefCell::new(Schedule::default());
    /// Widgets with a pending frame, with their id (a deleted widget has no
    /// pointer to ask for).
    static WIDGETS: RefCell<Vec<(usize, Widget, bool)>> = const { RefCell::new(Vec::new()) };
    static DISPLAY_HZ: Cell<(f64, Option<Instant>)> = const { Cell::new((60.0, None)) };
}

/// The monitor refresh rate used by [`Pace::Display`] (re-read every 3 s).
fn display_hz() -> f64 {
    let (hz, at) = DISPLAY_HZ.with(|d| d.get());
    if at.is_some_and(|t| t.elapsed() < Duration::from_secs(3)) {
        return hz;
    }
    let hz = crate::hyprland::refresh_hz_for(None).unwrap_or(60.0);
    DISPLAY_HZ.with(|d| d.set((hz, Some(Instant::now()))));
    hz
}

/// Redraws `w` on the next animation frame (one shared timer, 60 fps).
pub fn request_redraw<W: WidgetExt>(w: &W) {
    request_redraw_at(w, Pace::Normal);
}

/// Redraws `w` on its next frame at `pace` (under the global
/// [`PaceMode`]). One shared timer serves every pace and stops when no
/// widget waits.
pub fn request_redraw_at<W: WidgetExt>(w: &W, pace: Pace) {
    request(w, pace, false);
}

/// Like [`request_redraw`], but the frame only reports a scroll
/// (`Damage::Scroll`): a view that can move its pixels (`fl_scroll`) and
/// paint the exposed strip does so. Any full redraw of the widget wins.
pub fn request_scroll_frame<W: WidgetExt>(w: &W) {
    request(w, Pace::Normal, true);
}

/// [`request_scroll_frame`] at `pace` (a streaming view paced at 30 fps).
pub fn request_scroll_frame_at<W: WidgetExt>(w: &W, pace: Pace) {
    request(w, pace, true);
}

fn request<W: WidgetExt>(w: &W, pace: Pace, scroll_only: bool) {
    // SAFETY: `w` is a live widget; the copy only tracks its deletion.
    let w = unsafe { <Widget as WidgetBase>::from_widget_ptr(w.as_widget_ptr()) };
    let id = w.as_widget_ptr() as usize;
    WIDGETS.with(|list| {
        let mut list = list.borrow_mut();
        // Widgets deleted mid-transition (a closed window) leave the list first.
        list.retain(|(i, x, _)| {
            let keep = !x.was_deleted();
            if !keep {
                SCHEDULE.with(|s| s.borrow_mut().forget(*i));
            }
            keep
        });
        match list.iter_mut().find(|(i, _, _)| *i == id) {
            // A full redraw request wins over a scroll-only one.
            Some(e) => e.2 = e.2 && scroll_only,
            None => list.push((id, w, scroll_only)),
        }
    });
    SCHEDULE.with(|s| s.borrow_mut().request(id, pace.under(pace_mode()), Instant::now()));
    arm();
}

/// Starts the shared timer for the next due widget (if not running).
fn arm() {
    if TIMER.with(|t| t.get()) {
        return;
    }
    let hz = display_hz();
    let Some(wait) = SCHEDULE.with(|s| s.borrow().next_wait(Instant::now(), hz)) else { return };
    TIMER.with(|t| t.set(true));
    app::add_timeout3(wait.as_secs_f64().max(0.001), |_| {
        TIMER.with(|t| t.set(false));
        let hz = display_hz();
        let due = SCHEDULE.with(|s| s.borrow_mut().take_due(Instant::now(), hz));
        let list: Vec<(Widget, bool)> = WIDGETS.with(|l| {
            let mut l = l.borrow_mut();
            let picked = l.iter().filter(|(i, _, _)| due.contains(i)).map(|(_, w, s)| (w.clone(), *s)).collect();
            let waiting: Vec<usize> = SCHEDULE.with(|s| s.borrow().waiting.iter().map(|w| w.0).collect());
            l.retain(|(i, _, _)| waiting.contains(i));
            picked
        });
        for (mut w, scroll_only) in list {
            if !w.was_deleted() {
                if scroll_only {
                    w.set_damage_type(w.damage_type() | fltk::enums::Damage::Scroll);
                } else {
                    w.redraw();
                }
            }
        }
        arm();
    });
}

/// Widgets waiting for an animation frame (tests).
pub fn pending() -> usize {
    SCHEDULE.with(|s| s.borrow().len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature="gpu")]
    #[test]
    fn sampled_time_restores_nested_and_unwinding_test_clocks() {
        clock::freeze();let at=now();
        sample_time(|| {
            assert!(clock::frozen());
            sample_time(||assert_eq!(now(),at));
            assert_eq!(now(),at);
        });
        assert_eq!(now(),at);
        let _=std::panic::catch_unwind(||sample_time(|| {
            clock::advance(Duration::from_millis(10));
            panic!("restore sampled time");
        }));
        assert_eq!(now(),at);
        clock::resume();
        sample_time(||assert!(clock::frozen()));
        assert!(!clock::frozen());
    }

    #[test]
    fn paced_schedule() {
        let t0 = Instant::now();
        let ms = |n: u64| t0 + Duration::from_millis(n);
        let mut s = Schedule::default();
        assert!(s.next_wait(t0, 60.0).is_none(), "nothing waits");
        // A 60 fps widget (1) and an 8 fps one (2), both drawn at t0.
        s.request(1, Pace::Normal, t0);
        s.request(2, Pace::Saver, t0);
        assert_eq!(s.take_due(ms(17), 60.0), vec![1], "first frame of the fast one");
        s.request(1, Pace::Normal, ms(17));
        let mut drawn2 = 0;
        let mut drawn1 = 1;
        let mut t = 17;
        while t < 1000 {
            t += 17;
            let due = s.take_due(ms(t), 60.0);
            drawn1 += due.contains(&1) as u32;
            drawn2 += due.contains(&2) as u32;
            for id in due {
                s.request(id, if id == 1 { Pace::Normal } else { Pace::Saver }, ms(t));
            }
        }
        assert!((55..=61).contains(&drawn1), "about 60 frames a second: {drawn1}");
        assert!((7..=9).contains(&drawn2), "about 8 frames a second: {drawn2}");
        // A faster request wins over a pending slower one.
        let mut s = Schedule::default();
        s.request(3, Pace::Saver, t0);
        s.request(3, Pace::Display, t0);
        assert!(s.next_wait(t0, 120.0).unwrap() <= Duration::from_millis(9));
        assert_eq!(s.len(), 1);
        s.forget(3);
        assert!(s.is_empty());
        // Modes.
        assert_eq!(Pace::Display.under(PaceMode::Saver), Pace::Low);
        assert_eq!(Pace::Saver.under(PaceMode::Saver), Pace::Saver);
        assert_eq!(Pace::Saver.under(PaceMode::Max), Pace::Display);
        assert_eq!(Pace::Normal.under(PaceMode::Normal), Pace::Normal);
        assert_eq!(Pace::Display.interval(144.0), Duration::from_secs_f64(1.0 / 144.0));
    }

    #[test]
    fn easings_start_at_0_and_end_at_1() {
        for e in [
            Ease::Linear,
            Ease::OutCubic,
            Ease::InCubic,
            Ease::InOutCubic,
            Ease::InOutQuart,
            Ease::OutQuint,
            Ease::OutBack(1.70158),
            Ease::EaseOut,
        ] {
            assert!(e.apply(0.0).abs() < 1e-6, "{e:?} at 0");
            assert!((e.apply(1.0) - 1.0).abs() < 1e-6, "{e:?} at 1");
            // Out curves are ahead of linear halfway, In curves behind.
            let h = e.apply(0.5);
            match e {
                Ease::OutCubic | Ease::OutQuint | Ease::OutBack(_) | Ease::EaseOut => assert!(h > 0.5, "{e:?} {h}"),
                Ease::InCubic => assert!(h < 0.5),
                _ => assert!((h - 0.5).abs() < 1e-9, "{e:?} symmetric"),
            }
        }
        assert!((Ease::OutCubic.apply(0.5) - 0.875).abs() < 1e-9);
        // OutBack overshoots before settling.
        assert!((0..100).any(|i| Ease::OutBack(1.70158).apply(i as f64 / 100.0) > 1.0));
    }

    #[test]
    fn omarchy_ease_out_matches_its_bezier() {
        // Strong start: the Omarchy curve is past 60 % at a fifth of the time.
        let e = Ease::EaseOut;
        assert!(e.apply(0.2) > 0.6, "{}", e.apply(0.2));
        let mut prev = 0.0;
        for i in 1..=100 {
            let v = e.apply(i as f64 / 100.0);
            assert!(v >= prev - 1e-9 && v <= 1.0 + 1e-9, "monotone and bounded at {i}: {v}");
            prev = v;
        }
    }

    #[test]
    fn tween_and_anim_follow_the_clock_and_retarget_mid_way() {
        set_enabled(true);
        clock::freeze();
        let tw = Tween::start(200, Ease::Linear);
        clock::advance(Duration::from_millis(50));
        assert!((tw.t() - 0.25).abs() < 1e-6);
        clock::advance(Duration::from_millis(200));
        assert!(tw.done());

        let mut a: Anim<f64> = Anim::new();
        assert_eq!(a.value(0.0, 100, Ease::Linear), 0.0, "first value jumps");
        assert_eq!(a.value(100.0, 100, Ease::Linear), 0.0, "starts at the old value");
        clock::advance(Duration::from_millis(50));
        let mid = a.value(100.0, 100, Ease::Linear);
        assert!((mid - 50.0).abs() < 1e-6, "{mid}");
        // Interrupted: goes back from where it is, not from the old target.
        let back = a.value(0.0, 100, Ease::Linear);
        assert!((back - 50.0).abs() < 1e-6, "{back}");
        clock::advance(Duration::from_millis(50));
        assert!((a.value(0.0, 100, Ease::Linear) - 25.0).abs() < 1e-6);
        clock::advance(Duration::from_millis(60));
        assert_eq!(a.value(0.0, 100, Ease::Linear), 0.0);
        assert!(!a.running());

        // Off: jumps at once.
        set_enabled(false);
        assert_eq!(a.value(10.0, 100, Ease::Linear), 10.0);
        set_enabled(None);
        clock::resume();
    }

    #[test]
    fn system_setting_parsing() {
        assert_eq!(parse_hyprland_option(r#"{"option": "animations:enabled", "bool": true, "set": true }"#), Some(true));
        assert_eq!(parse_hyprland_option(r#"{"option": "animations:enabled", "bool": false, "set": true }"#), Some(false));
        assert_eq!(parse_hyprland_option(r#"{"option":"animations:enabled","int":0,"set":false}"#), Some(false));
        assert_eq!(parse_hyprland_option(r#"{"option":"animations:enabled","int":1}"#), Some(true));
        assert_eq!(parse_hyprland_option("no such option"), None);
        assert_eq!(parse_switch("OFF"), Some(false));
        assert_eq!(parse_switch(" on "), Some(true));
        assert_eq!(parse_switch("0"), Some(false));
        assert_eq!(parse_switch("maybe"), None);
    }

    #[test]
    fn app_choice_wins_and_none_follows_the_system_again() {
        set_enabled(false);
        assert!(!enabled());
        assert_eq!(source(), Source::App);
        set_enabled(Some(true));
        assert!(enabled());
        set_enabled(None);
        assert_ne!(source(), Source::App);
    }
}
