//! Busy-sheet native input probe; drive only inside a private display.
use fltk::{app, prelude::*};
use fltk_omarchy::{
    anim, focus,
    layout::FlexExt,
    overlay::{Sheet, SheetDismiss, WeakSheet},
    testkit, tr,
    widgets::{Button, TextField},
    App, AppWindow, State,
};
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
    time::{Duration, Instant},
};
struct Dropped(Rc<Cell<usize>>);
impl Drop for Dropped {
    fn drop(&mut self) {
        self.0.set(self.0.get() + 1);
    }
}
thread_local! { static EVENTS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) }; }
fn observe(event: fltk::enums::Event) -> bool {
    if matches!(
        event,
        fltk::enums::Event::KeyDown | fltk::enums::Event::Shortcut
    ) {
        EVENTS.with(|e| {
            e.borrow_mut()
                .push(format!("{event:?}:{:?}", app::event_key()))
        });
    }
    false
}
fn main() {
    let _app = App::without_theme_watch("dev.omarchy.fltk.sheet-guard-check");
    app::add_handler(observe);
    anim::set_enabled(false);
    testkit::arm();
    let root = PathBuf::from(std::env::args().nth(1).expect("private fixture directory"));
    let mut win = AppWindow::new(&tr!("Sheet guard input"), 620, 420);
    let mut body = win.body();
    let anchor = Button::new(&tr!("Anchor"));
    body.fit(&*anchor);
    win.end();
    win.set_pos(100, 100);
    let mut peer = AppWindow::new(&tr!("sheet-guard-peer"), 240, 160);
    let mut body = peer.body();
    let peer_input = TextField::new();
    body.fit(&*peer_input);
    peer.end();
    peer.set_pos(750, 100);
    let allowed = State::new(false);
    let field = Rc::new(RefCell::new(None::<TextField>));
    let finish = Rc::new(RefCell::new(None::<Button>));
    let builds = Rc::new(Cell::new(0));
    let drops = Rc::new(Cell::new(0));
    let reasons = Rc::new(RefCell::new(Vec::<String>::new()));
    let weak = Rc::new(RefCell::new(None::<WeakSheet>));
    let hide_reopen = Rc::new(Cell::new(None));
    let (f, b, n, d) = (field.clone(), finish.clone(), builds.clone(), drops.clone());
    let (r, w, h) = (reasons.clone(), weak.clone(), hide_reopen.clone());
    let sheet = Sheet::new(&*win, "Guarded form", 420, 260, move |col, owner| {
        n.set(n.get() + 1);
        let input = TextField::new();
        col.fit(&*input);
        *f.borrow_mut() = Some(input);
        let done = Button::new(&tr!("Owner finish")).on_click(move || {
            if let Some(owner) = owner.upgrade() {
                owner.close();
            }
        });
        col.fit(&*done);
        *b.borrow_mut() = Some(done);
        let capture = Dropped(d.clone());
        let mut frame = fltk::frame::Frame::default();
        frame.draw(move |_| {
            let _ = &capture;
        });
    })
    .bind_dismissible(&allowed)
    .release_after(Duration::from_millis(50))
    .on_dismiss(move |why| {
        r.borrow_mut().push(format!("{why:?}"));
        if why == SheetDismiss::OwnerHidden {
            h.set(Some(w.borrow().as_ref().unwrap().upgrade().unwrap().open()));
        }
    });
    *weak.borrow_mut() = Some(sheet.downgrade());
    win.show();
    let _ = anchor.clone().take_focus();
    sheet.open();
    let mut sheet = Some(sheet);
    let owner_position = Cell::new((win.x(), win.y()));
    let report = |sheet: Option<&Sheet>| {
        if !win.was_deleted() {
            owner_position.set((win.x(), win.y()));
        }
        let (owner_x, owner_y) = owner_position.get();
        let point = |r: (i32, i32, i32, i32)| (owner_x + r.0 + r.2 / 2, owner_y + r.1 + r.3 / 2);
        let field = field.borrow();
        let input = field.as_ref().filter(|f| !f.was_deleted());
        let finish = finish.borrow();
        let finish = finish.as_ref().filter(|f| !f.was_deleted());
        serde_json::json!({"open":sheet.is_some_and(|s|s.is_open()),"built":sheet.is_some_and(|s|s.is_built()),"allowed":allowed.get(),"focused":input.is_some_and(|f|focus::current().is_some_and(|w|w.inside(&**f))),"unhandled":EVENTS.with(|e|e.borrow().clone()),"policy":sheet.is_some_and(|s|s.is_dismissible()),"focused_nodes":if win.was_deleted(){vec![]}else{testkit::tree(&*win).into_iter().filter(|e|e.focused).map(|e|e.name).collect::<Vec<_>>()},"anchor_focused":(!anchor.was_deleted() && focus::is_focused(&*anchor)),"draft":input.map(|f|f.value()),"close_disabled":(!win.was_deleted()).then(||testkit::find(&*win,"Close")).flatten().is_some_and(|e|e.disabled),"close":sheet.and_then(|s|s.close_rect()).map(point),"finish":finish.map(|f|point((f.x(),f.y(),f.w(),f.h()))),"outside":[owner_x+2,owner_y+2],"owner_xid":if win.was_deleted() || !win.shown(){0}else{win.raw_handle() as usize},"peer_xid":if peer.shown(){peer.raw_handle() as usize}else{0},"peer_text":peer_input.value(),"reasons":*reasons.borrow(),"builds":builds.get(),"drops":drops.get(),"hide_reopen":hide_reopen.get(),"listeners":allowed.listener_count()})
    };
    let start = Instant::now();
    let mut ready = false;
    let mut last = 0;
    while start.elapsed() < Duration::from_secs(40) && !root.join("done").exists() {
        let _ = app::wait_for(0.005);
        App::pump();
        if !ready && start.elapsed() > Duration::from_millis(200) {
            std::fs::write(root.join("ready.json"), report(sheet.as_ref()).to_string()).unwrap();
            ready = true;
        }
        if let Ok(bytes) = std::fs::read(root.join("command.json")) {
            if let Ok(command) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                let seq = command["seq"].as_u64().unwrap();
                if seq <= last {
                    continue;
                }
                match command["action"].as_str().unwrap() {
                    "allow" => allowed.set(true),
                    "block" => allowed.set(false),
                    "open" => {
                        win.clone().show();
                        let _ = anchor.clone().take_focus();
                        sheet.as_ref().unwrap().open();
                    }
                    "hide" => {
                        win.clone().hide();
                        peer.clone().show();
                        let _ = peer_input.clone().take_focus();
                    }
                    "delete" => {
                        fltk::window::DoubleWindow::delete((*win).clone());
                        peer.clone().show();
                        let _ = peer_input.clone().take_focus();
                    }
                    "drop" => {
                        drop(sheet.take());
                    }
                    "report" => {}
                    _ => panic!("unknown fixture action"),
                }
                std::fs::write(
                    root.join(format!("reply-{seq}.json")),
                    report(sheet.as_ref()).to_string(),
                )
                .unwrap();
                last = seq;
            }
        }
    }
    eprintln!("unhandled keys {:?}", EVENTS.with(|e| e.borrow().clone()));
    if !win.was_deleted() {
        win.clone().hide();
    }
    peer.hide();
}
