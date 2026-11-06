//! Persistent notices with app-owned actions; --snapshot PATH is offscreen.
use fltk_omarchy::{
    anim, i18n,
    layout::FlexExt,
    overlay::{Notice, NoticeAction, NoticeHandle},
    tr, trn,
    widgets::*,
    App, AppWindow, State,
};
use std::{cell::RefCell, rc::Rc};

fn open(win: &AppWindow, count: &State<u64>) -> Option<NoticeHandle> {
    let count = count.clone();
    Notice::new("", "")
        .icon(glyph::BELL)
        .bind_summary(&i18n::text("Operation interrupted"))
        .bind_body(&i18n::text(
            "Retry when ready. This notice stays until you close it.",
        ))
        .action(NoticeAction::new("retry", "").bind_text(&i18n::text("Retry")))
        .action(NoticeAction::new("close", "").bind_text(&i18n::text("Close")))
        .on_action(move |notice, id| match id {
            "retry" => count.update(|n| *n += 1),
            "close" => notice.close(),
            _ => {}
        })
        .show(&**win)
}

fn main() {
    let app = App::new("dev.omarchy.fltk.notices");
    i18n::load(include_str!("support/notices.json")).expect("example catalog");
    let mut win = AppWindow::new(&tr!("Persistent notices"), 640, 360);
    win.bind_title(&i18n::text("Persistent notices"));
    let mut body = win.body();
    let count = State::new(0u64);
    body.fit(&*Label::new("").bind(&i18n::map(&count, |n| trn!("{count} retries", *n))));
    let current: Rc<RefCell<Option<NoticeHandle>>> = Rc::default();
    let (owner, w, n) = (current.clone(), win.clone(), count.clone());
    body.fit(
        &*Button::new("")
            .left_aligned()
            .bind_text(&i18n::text("Show notification"))
            .on_click(move || {
                // The application bounds its own notice queue.
                if let Some(old) = owner.borrow_mut().take() {
                    old.close();
                }
                *owner.borrow_mut() = open(&w, &n);
            }),
    );
    win.end();
    *current.borrow_mut() = open(&win, &count);
    let args: Vec<_> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--snapshot") {
        anim::set_enabled(false);
        win.snapshot(args.get(i + 1).expect("snapshot path"))
            .expect("notice snapshot");
    } else {
        win.show();
        app.run();
    }
}
