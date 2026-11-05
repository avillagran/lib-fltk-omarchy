//! Measures `NavTree` with a large model, offscreen (no window shown):
//! first rebuild, a frame, in-place patches (60 per second of a live
//! protocol) and structural changes (insert + rebuild + frame).
//!   cargo run --release --example navtree_bench -- 5000
use fltk::prelude::*;
use fltk_omarchy::layout::{self, FlexExt};
use fltk_omarchy::model::ListModel;
use fltk_omarchy::navtree::*;
use fltk_omarchy::widgets::glyph;
use fltk_omarchy::{App, AppWindow, State};
use std::time::{Duration, Instant};

fn median(mut v: Vec<Duration>) -> Duration {
    v.sort();
    v[v.len() / 2]
}

fn main() {
    let n: usize = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(5000);
    let _app = App::without_theme_watch("dev.omarchy.fltk.navtree-bench");
    fltk_omarchy::anim::set_enabled(false);
    let model = ListModel::new(|t: &NavItem| t.key.clone());
    let per = 99;
    let mut items = Vec::with_capacity(n);
    let mut p = 0;
    while items.len() < n {
        items.push(NavItem::section(&format!("h{p}"), &format!("Host {p}")).collapsible());
        for t in 0..per.min(n - items.len()) {
            let mut it = NavItem::row(
                &format!("t{p}.{t}"),
                glyph::FILE,
                &format!("Thread {p}.{t}"),
            )
            .parent(&format!("h{p}"));
            if t % 7 == 0 {
                it = it.detail("Working").badge(Badge::spinner());
            }
            items.push(it);
        }
        p += 1;
    }
    model.extend(items);
    let mut win = AppWindow::new("bench", 640, 600);
    let mut body = win.body();
    body.set_margin(0);
    let sel = State::new(String::new());
    let mut row = layout::row();
    let tree = NavTree::new(&model, &sel);
    row.fit(&*tree);
    row.end();
    win.end();
    tree.set_pinned(true);
    let out = std::path::PathBuf::from("target/navtree-bench.png");
    let t = Instant::now();
    let shown = tree.visible_keys().len();
    let rebuild = t.elapsed();
    let frame = |w: &mut AppWindow| {
        let t = Instant::now();
        let mut canvas = tree.rows_widget();
        canvas.redraw();
        w.snapshot(&out).expect("snapshot");
        t.elapsed()
    };
    // A frame here is an offscreen snapshot: drawing plus the PNG encoding.
    let first = frame(&mut win);
    let frames: Vec<Duration> = (0..10).map(|_| frame(&mut win)).collect();
    let mut patches = Vec::new();
    for i in 0..600 {
        let k = format!("t{}.{}", i % p, i % per);
        let t = Instant::now();
        model.patch(&k, |x| x.title = format!("Thread {i}"));
        patches.push(t.elapsed());
    }
    let mut structural = Vec::new();
    for i in 0..30 {
        let t = Instant::now();
        model.insert(
            1,
            NavItem::row(&format!("new{i}"), glyph::FILE, "New").parent("h0"),
        );
        let _ = tree.scroll_position();
        structural.push(t.elapsed());
    }
    println!(
        "{{\"rows\": {shown}, \"rebuild_ms\": {:.2}, \"first_frame_ms\": {:.2}, \"frame_ms\": {:.2}, \"patch_us\": {:.1}, \"insert_rebuild_ms\": {:.2}, \"rows_drawn\": {}}}",
        rebuild.as_secs_f64() * 1e3,
        first.as_secs_f64() * 1e3,
        median(frames).as_secs_f64() * 1e3,
        median(patches).as_secs_f64() * 1e6,
        median(structural).as_secs_f64() * 1e3,
        tree.rows_drawn()
    );
    let _ = std::fs::remove_file(out);
}
