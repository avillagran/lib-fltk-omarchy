//! Native presentation probe. Run only on a private test display: it
//! intentionally requests activation of its own two windows.
use fltk::prelude::*;
use fltk_omarchy::{
    native::{self, PresentOptions},
    widgets::Button,
    App,
};
use serde_json::{json, Value};
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
    time::{Duration, Instant},
};
fn report(result: Result<native::Presentation, native::PresentError>) -> Value {
    match result {
        Ok(r) => json!({"backend":format!("{:?}",r.backend),"status":format!("{:?}",r.status)}),
        Err(e) => json!({"error":format!("{e:?}")}),
    }
}
fn main() {
    let _app = App::without_theme_watch("dev.omarchy.fltk.present-check");
    let root = PathBuf::from(std::env::args().nth(1).expect("private output directory"));
    std::fs::create_dir_all(&root).unwrap();
    let mut b = fltk::window::DoubleWindow::new(450, 80, 320, 220, "present-probe-b");
    b.end();
    b.show();
    let b = Rc::new(RefCell::new(b));
    let mut a = fltk::window::DoubleWindow::new(60, 80, 320, 220, "present-probe-a");
    let events: Rc<RefCell<Vec<Value>>> = Rc::default();
    let count = Rc::new(Cell::new(0));
    let (target, out, n) = (b.clone(), events.clone(), count.clone());
    let mut button =
        Button::new(&fltk_omarchy::tr!("Activate the other own window")).on_click(move || {
            let r = report(native::present(
                &mut *target.borrow_mut(),
                PresentOptions::default(),
            ));
            n.set(n.get() + 1);
            out.borrow_mut().push(json!({"kind":"button","result":r}));
        });
    button.resize(24, 76, 272, 36);
    // Accept any real native key in this diagnostic: virtual keyboards may
    // use compact keymaps that FLTK 1.4 normalizes to number-row keysyms.
    let (target, out, n) = (b.clone(), events.clone(), count.clone());
    a.handle(move |_, event| {
        if !matches!(
            event,
            fltk::enums::Event::KeyDown | fltk::enums::Event::Shortcut
        ) {
            return false;
        }
        let r = report(native::present(
            &mut *target.borrow_mut(),
            PresentOptions::default(),
        ));
        n.set(n.get() + 1);
        out.borrow_mut().push(json!({"kind":"key","result":r}));
        true
    });
    a.end();
    a.show();
    let _ = button.take_focus();
    let start = Instant::now();
    let mut ready = false;
    let mut written_count = None;
    let mut last = 0u64;
    let baseline = (
        std::env::var_os("DESKTOP_STARTUP_ID"),
        std::env::var_os("XDG_ACTIVATION_TOKEN"),
    );
    while start.elapsed() < Duration::from_secs(45) && !root.join("done").exists() {
        let _ = fltk::app::wait_for(0.01);
        if !ready && start.elapsed() > Duration::from_millis(200) {
            // Probe backend after initial mapping. Result is intentionally
            // not treated as activation acknowledgement by the fixture.
            let initial = native::present(&mut a, PresentOptions::default());
            let x11 = initial
                .as_ref()
                .is_ok_and(|r| r.backend == native::PresentBackend::X11);
            std::fs::write(root.join("ready.json"),json!({"pid":std::process::id(),"a":if x11{a.raw_handle() as usize}else{0},"b":if x11{b.borrow().raw_handle() as usize}else{0},"initial":report(initial)}).to_string()).unwrap();
            ready = true;
        }
        if let Ok(bytes) = std::fs::read(root.join("command.json")) {
            if let Ok(cmd) = serde_json::from_slice::<Value>(&bytes) {
                let seq = cmd["seq"].as_u64().unwrap_or(0);
                if seq > last {
                    let options = PresentOptions {
                        activation_token: cmd["activation_token"].as_str(),
                        startup_id: cmd["startup_id"].as_str(),
                        timestamp: cmd["timestamp"].as_u64().map(|n| n as u32),
                    };
                    let value = if cmd["synthetic"].as_bool() == Some(true) {
                        json!({"synthetic_used": fltk_omarchy::focus::press("1")})
                    } else {
                        report(match cmd["window"].as_str() {
                            Some("a") => native::present(&mut a, options),
                            _ => native::present(&mut *b.borrow_mut(), options),
                        })
                    };
                    let unchanged = baseline
                        == (
                            std::env::var_os("DESKTOP_STARTUP_ID"),
                            std::env::var_os("XDG_ACTIVATION_TOKEN"),
                        );
                    std::fs::write(
                        root.join(format!("reply-{seq}.json")),
                        json!({"result":value,"env_unchanged":unchanged}).to_string(),
                    )
                    .unwrap();
                    last = seq;
                }
            }
        }
        if written_count != Some(count.get()) {
            std::fs::write(
                root.join("events.json"),
                json!({"count":count.get(),"events":*events.borrow()}).to_string(),
            )
            .unwrap();
            written_count = Some(count.get());
        }
    }
    a.hide();
    b.borrow_mut().hide();
}
