//! CPU-only native input fixture for script/tabs-input-check (private Xvfb).
use fltk::{app, prelude::*};
use fltk_omarchy::{
    anim, i18n, layout::FlexExt, model::ListModel, tr, widgets::*, App, AppWindow, State,
};
use std::{
    cell::RefCell,
    path::PathBuf,
    rc::Rc,
    time::{Duration, Instant},
};
#[derive(Clone, PartialEq)]
struct Entry {
    id: u64,
    title: String,
}
fn main() {
    let root = PathBuf::from(std::env::args().nth(1).expect("output directory"));
    std::fs::create_dir_all(&root).unwrap();
    let _app = App::without_theme_watch("dev.omarchy.fltk.tab-input-check");
    anim::set_enabled(false);
    i18n::set_locale("en");
    let model = ListModel::new(|t: &Entry| t.id);
    for (id, title) in [(10, "A"), (20, "B"), (30, "C")] {
        model.push(Entry {
            id,
            title: title.into(),
        });
    }
    let selected = State::new(Some(10u64));
    let events: Rc<RefCell<Vec<serde_json::Value>>> = Rc::default();
    let e = events.clone();
    selected
        .subscribe(move |key| {
            e.borrow_mut()
                .push(serde_json::json!({"kind":"select","key":key}))
        })
        .forever();
    let mut win = AppWindow::new(&tr!("Tab input check"), 560, 180);
    let mut body = win.body();
    let (e, s) = (events.clone(), selected.clone());
    let (e2, s2) = (events.clone(), selected.clone());
    let (e3, s3, m) = (events.clone(), selected.clone(), model.clone());
    let bar=TabBar::from_model(&model,&selected,|t|t.title.clone())
        .on_close(move|key|e.borrow_mut().push(serde_json::json!({"kind":"close","key":key,"selected":s.get(),"button":app::event_button()})))
        .on_context(move|r|e2.borrow_mut().push(serde_json::json!({"kind":"context","key":r.key,"selected":s2.get(),"button":app::event_button()})))
        .reorderable(move|r|{r.apply_to(&m);e3.borrow_mut().push(serde_json::json!({"kind":"reorder","key":r.key,"before":r.before,"selected":s3.get(),"order":(0..m.len()).filter_map(|i|m.key_at(i)).collect::<Vec<_>>()}));});
    body.fit(&*bar);
    body.fit(
        &*Label::new("").bind(&selected.map(|k| tr!("Active pane: {key}", key = format!("{k:?}")))),
    );
    win.end();
    win.set_pos(100, 100);
    win.show();
    let start = Instant::now();
    let mut ready = false;
    let mut reveal_ready = false;
    while start.elapsed() < Duration::from_secs(10) && win.shown() {
        let _ = app::wait_for(0.01);
        if !ready && start.elapsed() > Duration::from_millis(200) {
            let rects:Vec<_>=[10,20,30].iter().map(|key|{let(x,w)=bar.rect_for_key(key).unwrap();let(cx,cy,cw,ch)=bar.close_rect_for_key(key).unwrap();serde_json::json!({"key":key,"x":win.x()+x,"y":win.y()+bar.y()+bar.h()/2,"w":w,"close_x":win.x()+cx+cw/2,"close_y":win.y()+cy+ch/2})}).collect();
            std::fs::write(
                root.join("ready.json"),
                serde_json::json!({"tabs":rects,"window_x":win.x(),"window_y":win.y()}).to_string(),
            )
            .unwrap();
            ready = true;
        }
        if !reveal_ready && root.join("reveal-request").exists() {
            // Keep the active key (30) while moving it to the first slot.
            // Cursor-only focus scrolls it away; real Home must reveal it
            // without another selection notification.
            model.move_key(&30, 0);
            for id in 40..56 {
                model.push(Entry {
                    id,
                    title: format!("Overflow tab {id}"),
                });
            }
            assert!(bar.focus_key(&55));
            std::fs::write(root.join("reveal-ready.json"), serde_json::json!({"selected":selected.get(),"first_visible":bar.rect_for_key(&30).is_some(),"offset":bar.scroll_offset()}).to_string()).unwrap();
            reveal_ready = true;
        }
        if root.join("done").exists() {
            break;
        }
    }
    std::fs::write(root.join("report.json"),serde_json::json!({"events":*events.borrow(),"selected":selected.get(),"order":(0..model.len()).filter_map(|i|model.key_at(i)).collect::<Vec<_>>(),"backend":"CPU","reveal":{"first_visible":bar.rect_for_key(&30).is_some(),"offset":bar.scroll_offset(),"focused":bar.focused_key()}}).to_string()).unwrap();
    win.hide();
}
