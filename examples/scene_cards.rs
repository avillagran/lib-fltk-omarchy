//! Static layered-card reference for the window-scene painter, not a lava
//! shader demo. No animation timer. All colors follow the live theme.
//!
//! cargo run --example scene_cards -- --snapshot target/cards.png
//! cargo run --release --example scene_cards -- --bench
use fltk::{enums::Align, prelude::*};
use fltk_omarchy::{i18n, paint, scene::{self, Rect, Scene}, theme, tr, App, AppWindow};
use std::{cell::RefCell, rc::Rc, time::Instant};

type Cache = Option<((i32, i32, i32, i32), Scene)>;

fn cards(x: i32, y: i32, w: i32, h: i32) {
    let t = theme::current();
    let p = &t.palette;
    paint::fill(x, y, w, h, 0, p.darker_background);
    scene::clip(Rect::new(x, y, w, h), || {
        let horizon = y + h * 4 / 5;
        // Broad, static glow in strips: verifies alpha over a varying
        // background without a per-pixel shader or a raster allocation.
        for j in (0..100).rev() {
            let alpha = 0.002 + (100 - j) as f64 * 0.00015;
            paint::alpha_fill(x, horizon - j, w, j * 2 + 1, p.accent, alpha);
        }
        paint::fill(x, horizon, w, 2, 0, p.accent);
        paint::text(&tr!("Today"), x + 30, y + 20, w - 60, 35, 25, true, p.foreground, Align::Left);
        paint::text(&tr!("Layered cards · CPU scene"), x + 30, y + 58, w - 60, 22, 12, false, p.dim, Align::Left);
        let titles = [tr!("Ship the release notes"), tr!("Review the layout cache"), tr!("Reply to the legal thread"), tr!("Redesign the empty state"), tr!("Book flights for the offsite")];
        let tags = [tr!("Engineering"), tr!("Engineering"), tr!("Operations"), tr!("Design"), tr!("Personal")];
        for (i, (title, tag)) in titles.iter().zip(&tags).enumerate() {
            let cy = y + 106 + i as i32 * 70;
            let cw = w - 60;
            let fill = p.lighter_background.over(p.background, 0.8);
            paint::surface(x + 30, cy, cw, 58, t.radius, Some(fill), Some((p.foreground.over(fill, 0.12), 1)));
            paint::stroke(x + 45, cy + 22, 14, 14, 7, 1, p.dim);
            paint::text(title, x + 72, cy + 6, cw - 216, 25, 14, true, p.foreground, Align::Left);
            paint::text(&tr!("Ready for review"), x + 72, cy + 31, cw - 216, 18, 11, false, p.dim, Align::Left);
            let color = [p.blue, p.blue, p.yellow, p.magenta, p.green][i];
            paint::fill(x + w - 158, cy + 19, 108, 22, 9, color.over(fill, 0.18));
            paint::text(tag, x + w - 158, cy + 19, 108, 22, 10, true, color, Align::Center);
        }
    });
}

fn main() {
    let app = App::new("dev.omarchy.fltk.scene-cards");
    let args: Vec<_> = std::env::args().collect();
    if args.iter().any(|s| s == "--bench") {
        let surf = fltk::surface::ImageSurface::new(720, 640, false);
        fltk::surface::ImageSurface::push_current(&surf);
        let (s, ()) = Scene::record(|| cards(0, 0, 720, 640));
        // Warm caches before timing. No PNG encoding in the measured work.
        s.draw();
        let mut results = Vec::new();
        for mode in 0..3 {
            let start = Instant::now();
            for _ in 0..200 {
                match mode {
                    0 => cards(0, 0, 720, 640),
                    1 => Scene::record(|| cards(0, 0, 720, 640)).0.draw(),
                    _ => s.draw(),
                }
            }
            results.push(start.elapsed().as_secs_f64() * 5.0);
        }
        fltk::surface::ImageSurface::pop_current();
        println!("{}", serde_json::json!({"commands": s.commands().len(), "ms_per_frame": {"immediate": results[0], "record_and_replay": results[1], "retained_replay": results[2]}, "size": [720, 640], "backend": "cpu", "offscreen": true}));
        return;
    }
    let mut win = AppWindow::new(&tr!("Scene cards"), 720, 680);
    win.bind_title(&i18n::text("Scene cards"));
    let mut body = win.body();
    body.set_margin(0);
    let mut frame = fltk::frame::Frame::default();
    body.add(&frame);
    let cache: Rc<RefCell<Cache>> = Rc::new(RefCell::new(None));
    let c = cache.clone();
    frame.draw(move |f| {
        let key = (f.x(), f.y(), f.w(), f.h());
        let mut cached = c.borrow_mut();
        if cached.as_ref().map(|(k, _)| *k) != Some(key) {
            *cached = Some((key, Scene::record(|| cards(key.0, key.1, key.2, key.3)).0));
        }
        cached.as_ref().unwrap().1.draw();
    });
    let c = cache.clone();
    let mut f = frame.clone();
    theme::on_change_while(&frame, move |_| { *c.borrow_mut() = None; f.redraw(); });
    let mut f = frame.clone();
    i18n::bind(&frame, move || { *cache.borrow_mut() = None; f.redraw(); });
    win.end();
    if let Some(i) = args.iter().position(|s| s == "--snapshot") {
        fltk_omarchy::anim::set_enabled(false);
        win.snapshot(args.get(i + 1).expect("snapshot path")).expect("snapshot");
    } else {
        win.show();
        app.run();
    }
}
