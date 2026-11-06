//! Driving any toolkit app from the environment, for agents and CI:
//!
//! ```sh
//! env -u WAYLAND_DISPLAY xvfb-run -a \
//!     OMARCHY_FLTK_SCRIPT=steps.json OMARCHY_FLTK_TREE=tree.json \
//!     OMARCHY_FLTK_REPORT=report.json ./my-app
//! ```
//!
//! With [`SCRIPT`] or [`TREE`] set, `App::new` arms the kit and the app's
//! windows are not mapped (headless: they render offscreen and take input
//! straight from the script). When the app calls `App::run`, instead of the
//! event loop the script runs against the last `AppWindow` made (or the one
//! whose title the script names: `{"window": "Title", "steps": [..]}`),
//! then the tree is written to [`TREE`], the report to [`REPORT`] (else a
//! summary to stderr) and the app exits: status 0 when every step passed,
//! 1 otherwise. [`TREE`] alone dumps the tree of the started app.
//! [`SHOW`]`=1` maps the windows as usual (only under `xvfb-run` or on a
//! hidden workspace: never on the user's desktop).

use std::cell::Cell;
use std::path::PathBuf;

/// Path of a JSON script ([`run_script`](super::run_script) format).
pub const SCRIPT: &str = "OMARCHY_FLTK_SCRIPT";
/// Where the element tree goes after the script ([`tree_json`](super::tree_json)).
pub const TREE: &str = "OMARCHY_FLTK_TREE";
/// Where the JSON report goes ([`Report::to_json`](super::Report::to_json)).
pub const REPORT: &str = "OMARCHY_FLTK_REPORT";
/// `1`: map the windows during a scripted run.
pub const SHOW: &str = "OMARCHY_FLTK_SCRIPT_SHOW";

thread_local! {
    static ACTIVE: Cell<Option<bool>> = const { Cell::new(None) };
}

fn var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// A scripted or tree-dump run was asked for in the environment.
pub fn active() -> bool {
    ACTIVE.with(|a| {
        if let Some(v) = a.get() {
            return v;
        }
        let v = var(SCRIPT).is_some() || var(TREE).is_some();
        a.set(Some(v));
        v
    })
}

/// Called by `App::new`: arms the kit for scripted runs and opens the
/// frame log.
pub(crate) fn init() {
    if active() {
        super::arm();
    }
    super::frames::init_from_env();
}

/// Windows stay unmapped during a scripted run (unless [`SHOW`]`=1`).
pub(crate) fn headless() -> bool {
    active() && var(SHOW).as_deref() != Some("1")
}

/// Runs the script from the environment (called by `App::run`); None when
/// no scripted run was asked for, else the exit status.
pub(crate) fn run_if_requested() -> Option<i32> {
    if !active() {
        return None;
    }
    ACTIVE.with(|a| a.set(Some(false)));
    Some(run())
}

fn run() -> i32 {
    use fltk::prelude::*;
    // Startup work posted by the app (tasks, first layouts).
    for _ in 0..3 {
        super::script::pump();
    }
    let script = match var(SCRIPT).map(PathBuf::from) {
        Some(p) => match std::fs::read_to_string(&p) {
            Ok(s) => match serde_json::from_str::<serde_json::Value>(&s) {
                Ok(v) => v,
                Err(e) => return fail(&format!("{}: {e}", p.display())),
            },
            Err(e) => return fail(&format!("{}: {e}", p.display())),
        },
        None => serde_json::Value::Array(Vec::new()),
    };
    let wanted = script.get("window").and_then(|w| w.as_str()).map(str::to_string);
    let win = match &wanted {
        Some(t) => super::WINDOWS.with(|v| {
            v.borrow()
                .iter()
                .rev()
                .find(|w| !w.was_deleted() && w.label() == *t)
                // SAFETY: a live window made by AppWindow::new.
                .map(|w| unsafe { fltk::window::DoubleWindow::from_widget_ptr(w.as_widget_ptr()) })
        }),
        None => super::main_window(),
    };
    let Some(win) = win else {
        return fail(&match wanted {
            Some(t) => format!("no AppWindow titled {t:?}"),
            None => "no AppWindow was made before App::run".into(),
        });
    };
    if win.shown() {
        // Mapped (OMARCHY_FLTK_SCRIPT_SHOW=1): let the first frames land.
        for _ in 0..10 {
            let _ = fltk::app::wait_for(0.02);
            super::script::pump();
        }
    }
    super::render_frame(&win);
    let steps = super::script::steps_of(&script);
    let report = super::run_steps(&win, &steps);
    if let Some(p) = var(TREE) {
        if let Err(e) = std::fs::write(&p, super::tree_json(&win)) {
            eprintln!("fltk-omarchy testkit: {p}: {e}");
        }
    }
    match var(REPORT) {
        Some(p) => {
            if let Err(e) = std::fs::write(&p, report.to_json()) {
                eprintln!("fltk-omarchy testkit: {p}: {e}");
            }
        }
        None => eprintln!("fltk-omarchy testkit: {}", report.summary()),
    }
    if report.ok() {
        0
    } else {
        1
    }
}

fn fail(msg: &str) -> i32 {
    eprintln!("fltk-omarchy testkit: {msg}");
    if let Some(p) = var(REPORT) {
        let v = serde_json::json!({ "ok": false, "passed": 0, "failed": 1, "error": msg, "steps": [] });
        let _ = std::fs::write(p, serde_json::to_string_pretty(&v).unwrap_or_default());
    }
    2
}
