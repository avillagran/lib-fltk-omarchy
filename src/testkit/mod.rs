//! Test and agent harness: look at a running UI as data, drive it with a
//! script, check its motion frame by frame and log what each frame costs.
//!
//! - [`tree`]: every widget of a window (plus the virtual rows of lists,
//!   trees, transcripts near the viewport) as [`Element`]s with path, kind,
//!   name, value, rectangle, visibility and focus; [`tree_json`] for agents.
//! - [`run_script`]: JSON steps (`press`, `type`, `click`, `wheel`,
//!   `wait_ms` on the frozen animation clock, `expect`, `snapshot`…)
//!   through the same event path as real input; returns a [`Report`].
//!   Any toolkit app runs a script headlessly with
//!   `OMARCHY_FLTK_SCRIPT=steps.json` (and dumps its tree with
//!   `OMARCHY_FLTK_TREE=out.json`): see [`env`](mod@env) and docs/TESTKIT.md.
//! - [`Gate`]: records rectangles of chosen widgets, transcript messages
//!   or tree rows on every animation frame and finds jumps, pops and
//!   overshoot.
//! - Frame log and blocked-UI warning: `OMARCHY_FLTK_FRAME_LOG=path`
//!   writes one JSON line per window frame; debug builds warn once per
//!   call site when a UI callback waits (wall time over CPU time by more
//!   than 50 ms). See [`frames`].
//!
//! Without the `a11y` feature the element names come from the components'
//! own descriptions, kept only while the kit is [`arm`]ed (the env
//! variables arm it; tests call [`arm`] before building windows), else
//! from labels and tooltips.
//!
//! Input never leaves the app: events go straight to its own windows,
//! which need not be shown. Never point a script at a window on the
//! user's desktop; run under `xvfb-run` or a hidden workspace.

use fltk::prelude::*;
use fltk::widget::Widget;
use std::cell::{Cell, RefCell};

pub mod env;
pub mod frames;
mod gate;
pub(crate) mod lite;
mod script;
mod tree;

pub use gate::{Finding, FindingKind, Gate, Rules, Sample};
pub use script::{run_script, run_steps, Report, StepResult};
pub use tree::{find, tree, tree_json, Element};

thread_local! {
    static ARMED: Cell<bool> = const { Cell::new(false) };
    /// Top-level windows made by `AppWindow::new` while armed (the env
    /// runner drives the last one shown or made).
    static WINDOWS: RefCell<Vec<Widget>> = const { RefCell::new(Vec::new()) };
}

/// Keeps component descriptions (names, roles, virtual rows) for [`tree`]
/// in builds without the `a11y` feature, for widgets created from now on.
/// Sticky. The env variables of [`env`](mod@env) arm the kit in `App::new`.
pub fn arm() {
    ARMED.with(|a| a.set(true));
}

/// True once [`arm`] ran (or an env variable armed the kit).
pub fn armed() -> bool {
    ARMED.with(|a| a.get())
}

/// Remembers an `AppWindow` for the env runner (only while armed).
pub(crate) fn note_window<W: WidgetExt>(w: &W) {
    if !armed() {
        return;
    }
    // SAFETY: a live widget; the copy only tracks its deletion.
    let w = unsafe { Widget::from_widget_ptr(w.as_widget_ptr()) };
    WINDOWS.with(|v| {
        let mut v = v.borrow_mut();
        v.retain(|x| !x.was_deleted());
        v.push(w);
    });
}

/// The window the env runner drives: the last noted one that is shown,
/// else the last noted one.
pub(crate) fn main_window() -> Option<fltk::window::DoubleWindow> {
    WINDOWS.with(|v| {
        let v = v.borrow();
        let live: Vec<&Widget> = v.iter().filter(|w| !w.was_deleted()).collect();
        let pick = live
            .iter()
            .rev()
            .find(|w| w.visible())
            .or_else(|| live.last())
            .copied()?;
        // SAFETY: a live window made by AppWindow::new.
        Some(unsafe { fltk::window::DoubleWindow::from_widget_ptr(pick.as_widget_ptr()) })
    })
}

/// Renders `win` offscreen at the current animation clock (what
/// [`AppWindow::snapshot`](crate::AppWindow::snapshot) draws), so state
/// that advances while painting (scroll glides, row motion) moves on.
pub(crate) fn render_frame<W: WindowExt>(win: &W) {
    // SAFETY: a live window.
    let mut w = unsafe { fltk::window::DoubleWindow::from_widget_ptr(win.as_widget_ptr()) };
    let _ = crate::window::render(&mut w);
}
/// UI timer ownership counters (UI thread only). Pending includes running
/// jobs; a canceled running callback can remain executing until it returns.
pub fn timer_stats() -> crate::timer::Stats {
    crate::timer::stats()
}
