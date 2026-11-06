//! A single-instance app toggled from a second invocation.
//!
//! ```text
//! cargo run --example instance               # start (or raise the running one)
//! cargo run --example instance -- --hidden   # start hidden (autostart)
//! cargo run --example instance -- toggle     # show, or hide when focused
//! cargo run --example instance -- hide | show | quit | status
//! cargo run --example instance -- call FN [ARG..]   # any IPC function
//! cargo run --example instance -- --self-test       # scripted check
//! ```
//!
//! A plain second launch forwards its arguments to the running app
//! (`activate`) and exits. `--self-test` starts hidden, toggles itself
//! from child processes and quits through IPC; run it with a private
//! `OMARCHY_FLTK_RUNTIME_DIR` and on a display that is not the user's
//! (e.g. `xvfb-run`).
use fltk::prelude::*;
use fltk_omarchy::instance::{self, Error};
use fltk_omarchy::layout::{self, FlexExt};
use fltk_omarchy::task::{Outcome, Task};
use fltk_omarchy::widgets::*;
use fltk_omarchy::{i18n, trn, App, AppWindow, State};
use std::time::Duration;

const ID: &str = "dev.omarchy.fltk.instance-example";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(String::as_str).unwrap_or("");
    // Commands for the running instance: answer and exit without a window.
    match cmd {
        "call" if args.len() >= 2 => {
            let rest: Vec<&str> = args[2..].iter().map(String::as_str).collect();
            exit_with(instance::call_args(ID, &args[1], &rest));
        }
        "hide" | "quit" | "status" => match instance::call(ID, if cmd == "status" { "window" } else { cmd }, "") {
            Err(Error::NotRunning) if cmd == "status" => exit_with(Ok(r#"{"running":false}"#.into())),
            Err(Error::NotRunning) => return,
            r => exit_with(r),
        },
        "toggle" | "show" => match instance::call(ID, cmd, "") {
            Err(Error::NotRunning) => {} // start it, shown
            r => exit_with(r),
        },
        _ => {}
    }
    let hidden = args.iter().any(|a| a == "--hidden" || a == "--self-test");
    // A second plain launch forwards its arguments and exits here.
    let server = instance::acquire_or_forward(ID);
    if let Some(why) = server.unavailable_reason() {
        eprintln!("instance: no IPC ({why})");
    }

    let app = App::new(ID);
    app.keep_running(true); // hidden is not quit
    let mut win = AppWindow::new("Instance", 460, 240);
    let mut body = win.body();
    let count = State::new(0usize);
    let last = State::new(String::new());
    body.fit(&*Label::new("").bind(&i18n::map(&count, |n| trn!("Activated {count} times", *n))));
    body.fit(&*Label::new("").bind(&last));
    let mut row = layout::row();
    let mut w2 = win.clone();
    row.fit(&*Button::new("").bind_text(&i18n::text("Hide")).bordered().on_click(move || w2.hide()));
    row.fit(&*Button::new("").bind_text(&i18n::text("Quit")).bordered().on_click(App::quit));
    row.close_fitted();
    win.end();

    server.manage(&win);
    let (c2, l2) = (count.clone(), last.clone());
    server.on_activate(move |r| {
        c2.update(|n| *n += 1);
        l2.set(r.args.join(" "));
    });
    let c3 = count.clone();
    server.handle("count", move |_| c3.get().to_string());
    println!("instance: listening on {}", server.endpoint());

    if args.iter().any(|a| a == "--self-test") {
        self_test(&win);
    } else if !hidden {
        win.show();
    }
    app.run();
    drop(server);
}

fn exit_with(r: Result<String, Error>) -> ! {
    match r {
        Ok(v) => {
            println!("{v}");
            std::process::exit(0)
        }
        Err(e) => {
            eprintln!("instance: {e}");
            std::process::exit(1)
        }
    }
}

/// Runs this binary with `args` on a worker; its stdout reaches `done`.
fn run_self(args: &'static [&'static str], done: impl FnOnce(String) + 'static) {
    Task::new(move |_| {
        let out = std::process::Command::new(std::env::current_exe().map_err(|e| e.to_string())?)
            .args(args)
            .output()
            .map_err(|e| e.to_string())?;
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    })
    .on_done(move |o| match o {
        Outcome::Done(s) => done(s),
        o => {
            eprintln!("self-test: child failed: {o:?}");
            std::process::exit(1)
        }
    })
    .start();
}

fn check(ok: bool, what: &str) {
    println!("self-test: {} {what}", if ok { "PASS" } else { "FAIL" });
    if !ok {
        std::process::exit(1);
    }
}

/// Hidden start → `toggle` shows → `toggle` hides (it is active) →
/// plain launch raises → `call count` → `quit` ends the app.
fn self_test(win: &AppWindow) {
    let w = win.clone();
    check(!w.shown(), "starts hidden");
    run_self(&["toggle"], move |_| {
        later(move || {
            check(w.shown(), "toggle shows the hidden window");
            check(w.is_active(), "the shown window is active");
            let w = w.clone();
            run_self(&["toggle"], move |_| {
                later(move || {
                    check(!w.shown(), "toggle hides the active window");
                    let w = w.clone();
                    run_self(&["open", "this"], move |_| {
                        later(move || {
                            check(w.shown(), "a second launch raises the window");
                            run_self(&["call", "count"], move |n| {
                                check(n == "1", "the activation was counted once");
                                run_self(&["quit"], |_| println!("self-test: quit sent"));
                            });
                        })
                    });
                })
            });
        })
    });
}

/// Gives the window a moment to map before checking it.
fn later(f: impl FnOnce() + 'static) {
    let mut f = Some(f);
    fltk::app::add_timeout3(Duration::from_millis(300).as_secs_f64(), move |_| {
        if let Some(f) = f.take() {
            f()
        }
    });
}
