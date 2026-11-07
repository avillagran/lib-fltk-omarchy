//! Async activation probe for an isolated Wayland compositor.
use fltk::prelude::*;
use fltk_omarchy::{
    native::{self, PresentOptions, PresentRequest},
    App,
};
use std::{
    cell::RefCell,
    rc::Rc,
    task::Poll,
    time::{Duration, Instant},
};

thread_local! {
    static EXIT_REQUEST: RefCell<Option<PresentRequest>> = const { RefCell::new(None) };
}

fn main() {
    let tls_exit = std::env::args().any(|arg| arg == "--tls-exit");
    if tls_exit {
        EXIT_REQUEST.with(|request| assert!(request.borrow().is_none()));
    }
    let _app = App::without_theme_watch("dev.omarchy.fltk.async-present-check");
    let root = std::path::PathBuf::from(std::env::args().nth(1).unwrap());
    if tls_exit {
        let mut window = fltk::window::DoubleWindow::new(0, 0, 200, 150, "async-tls-exit");
        window.end();
        let request = native::begin_present(&mut window, PresentOptions::default()).unwrap();
        EXIT_REQUEST.with(|slot| *slot.borrow_mut() = Some(request));
        std::fs::write(root.join("tls-request-stored"), "pending").unwrap();
        return;
    }
    let mut b = fltk::window::DoubleWindow::new(450, 80, 320, 220, "async-probe-b");
    b.end();
    b.show();
    let b = Rc::new(RefCell::new(b));
    let request: Rc<RefCell<Option<PresentRequest>>> = Rc::default();
    let mut a = fltk::window::DoubleWindow::new(60, 80, 320, 220, "async-probe-a");
    let mut button = fltk_omarchy::widgets::Button::new("Activate");
    button.resize(24, 76, 272, 36);
    let (target, pending) = (b.clone(), request.clone());
    a.handle(move |_, event| {
        if !matches!(
            event,
            fltk::enums::Event::KeyDown | fltk::enums::Event::Shortcut
        ) {
            return false;
        }
        *pending.borrow_mut() = Some(
            native::begin_present(&mut *target.borrow_mut(), PresentOptions::default()).unwrap(),
        );
        true
    });
    a.end();
    a.show();
    let _ = button.take_focus();
    let start = Instant::now();
    std::fs::write(root.join("ready"), "ready").unwrap();
    let mut count = 0;
    while start.elapsed() < Duration::from_secs(40) && !root.join("done").exists() {
        let _ = fltk::app::wait_for(0.01);
        if root.join("synthetic").exists() {
            std::fs::remove_file(root.join("synthetic")).unwrap();
            let used = fltk_omarchy::focus::press("1");
            std::fs::write(root.join("synthetic-reply"), used.to_string()).unwrap();
        }
        let result = request.borrow_mut().as_mut().and_then(|r| match r.poll() {
            Poll::Pending => None,
            Poll::Ready(r) => Some(r),
        });
        if let Some(result) = result {
            count += 1;
            std::fs::write(root.join(format!("result-{count}")), format!("{result:?}")).unwrap();
            request.borrow_mut().take();
        }
    }
    a.hide();
    b.borrow_mut().hide();
}
