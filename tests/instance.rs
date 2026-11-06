//! Single instance across processes: this test binary starts copies of
//! itself as second launches against a primary in this process. Every
//! process uses a private runtime directory under the system temp dir
//! (never the user's runtime dir) and test-only app ids. No window, no
//! display needed: requests reach handlers through the real UI-thread
//! queue (`task::post`), drained here.

use fltk_omarchy::instance::{self, Instance, Options};
use fltk_omarchy::task;
use std::cell::RefCell;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::time::{Duration, Instant};

const APP: &str = "dev.omarchy.fltk.test-instance";
const CHILD: &str = "FLTK_OMARCHY_INSTANCE_TEST_CHILD";

/// Second launch, run in a child process (`CHILD` = mode). In the normal
/// test run (no `CHILD`) it does nothing.
#[test]
fn child_entry() {
    let Ok(mode) = std::env::var(CHILD) else { return };
    match mode.as_str() {
        // Explicit client: acquire must find the primary, then call it.
        "client" => {
            let runtime = std::env::var("OMARCHY_FLTK_RUNTIME_DIR").unwrap();
            let c = match Options::new(APP).runtime_dir(runtime).acquire() {
                Instance::Secondary(c) => c,
                Instance::Primary(_) => {
                    println!("RESULT primary");
                    return;
                }
            };
            println!("RESULT echo={}", c.call("echo", "from child").unwrap());
            println!("RESULT args={}", c.call_args("join", &["a", "b c"]).unwrap());
            println!("RESULT state={}", instance::call(APP, "state", "").unwrap());
        }
        // The default second launch: forwards argv ("activate") and exits 0.
        "forward" => {
            let _server = instance::acquire_or_forward(APP);
            println!("RESULT not forwarded");
        }
        _ => {}
    }
}

/// A private runtime dir; Drop removes only the files the instance layer
/// makes and then the empty directories.
struct TempRuntime(PathBuf);

impl TempRuntime {
    fn new() -> TempRuntime {
        let p = std::env::temp_dir().join(format!("fo-ipc-it-{}", std::process::id()));
        std::fs::create_dir(&p).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        TempRuntime(p)
    }
}

impl Drop for TempRuntime {
    fn drop(&mut self) {
        let d = self.0.join(APP);
        if std::fs::symlink_metadata(&d).is_ok_and(|m| m.file_type().is_dir()) {
            let _ = std::fs::remove_file(d.join("instance.sock"));
            let _ = std::fs::remove_file(d.join("instance.lock"));
            let _ = std::fs::remove_dir(&d);
        }
        let _ = std::fs::remove_dir(&self.0);
    }
}

/// Runs a copy of this binary in `mode` while this thread drains the UI
/// queue; returns (exit code, stdout).
fn run_child(rt: &TempRuntime, mode: &str, extra: &[&str]) -> (i32, String) {
    let exe = std::env::current_exe().unwrap();
    let mut child = Command::new(exe)
        .args(["--exact", "child_entry", "--nocapture", "--test-threads=1"])
        .args(extra)
        .env(CHILD, mode)
        .env("OMARCHY_FLTK_RUNTIME_DIR", &rt.0)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let end = Instant::now() + Duration::from_secs(30);
    let status = loop {
        task::drain();
        if let Some(s) = child.try_wait().unwrap() {
            break s;
        }
        if Instant::now() > end {
            let _ = child.kill();
            panic!("child {mode} did not finish");
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    let mut out = String::new();
    std::io::Read::read_to_string(child.stdout.as_mut().unwrap(), &mut out).unwrap();
    (status.code().unwrap_or(-1), out)
}

#[test]
fn second_process_reaches_the_primary() {
    let rt = TempRuntime::new();
    let server = match Options::new(APP).runtime_dir(&rt.0).acquire() {
        Instance::Primary(s) => s,
        Instance::Secondary(_) => panic!("a stale primary answered in a fresh runtime dir"),
    };
    assert!(server.is_listening(), "{:?}", server.unavailable_reason());
    let activations: Rc<RefCell<Vec<Vec<String>>>> = Rc::default();
    server.handle("echo", |a| format!("echo:{a}"));
    server.handle_request("join", |r| Ok(r.args.join("|")));
    server.handle("state", |_| r#"{"running":true}"#.into());
    let acts = activations.clone();
    server.on_activate(move |r| acts.borrow_mut().push(r.args.clone()));

    let (code, out) = run_child(&rt, "client", &[]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("RESULT echo=echo:from child"), "{out}");
    assert!(out.contains("RESULT args=a|b c"), "{out}");
    assert!(out.contains(r#"RESULT state={"running":true}"#), "{out}");

    // The default second launch forwards its argv and exits before its
    // own code runs.
    let (code, out) = run_child(&rt, "forward", &["--skip", "open-me.txt"]);
    assert_eq!(code, 0, "{out}");
    assert!(!out.contains("RESULT not forwarded"), "{out}");
    let acts = activations.borrow();
    assert_eq!(acts.len(), 1);
    assert!(acts[0].iter().any(|a| a == "open-me.txt"), "{acts:?}");

    // Once the primary stops, a launch becomes the primary itself.
    drop(acts);
    drop(server);
    assert!(matches!(Options::new(APP).runtime_dir(&rt.0).client().call("ping", ""), Err(instance::Error::NotRunning)));
    let (code, out) = run_child(&rt, "forward", &[]);
    assert_eq!(code, 0);
    assert!(out.contains("RESULT not forwarded"), "{out}");
}
