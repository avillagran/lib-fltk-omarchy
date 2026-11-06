//! Shared-painter parity and damage: compare every pixel, including clips,
//! rounded borders, literal '@', alpha over varied backgrounds and text.
use super::{check, pixel};
use fltk::{enums::Align, prelude::*};
use fltk_omarchy::{paint, scene::{self, Rect, Scene}, theme, AppWindow};
use std::path::Path;

type Check<'a> = &'a dyn Fn(&Path) -> Result<(), String>;

fn content(x: i32, y: i32) {
    let t = theme::current();
    paint::fill(x, y, 280, 150, 0, t.palette.background);
    paint::fill(x, y + 60, 280, 90, 0, t.palette.accent);
    scene::clip(Rect::new(x + 12, y + 8, 246, 128), || {
        paint::surface(x + 8, y + 5, 260, 110, 18, Some(t.palette.background), Some((t.palette.foreground, 2)));
        paint::alpha_fill(x + 18, y + 58, 235, 88, t.palette.foreground, 0.25);
        scene::clip(Rect::new(x + 24, y + 16, 190, 100), || {
            paint::text("@review: café 日本語", x + 28, y + 24, 230, 28, 15, true, t.palette.foreground, Align::Left);
            paint::text_weight("Semibold card", x + 28, y + 52, 190, 24, 13, paint::Weight::SemiBold, t.palette.accent, Align::Left);
            paint::text_spaced("LAYER", x + 28, y + 82, 190, 20, 11, paint::Weight::Medium, 2.0, t.palette.foreground, Align::Left);
        });
    });
    // Outside the previous clips: catches unbalanced replay stacks.
    paint::fill(x + 262, y + 130, 12, 12, 3, t.palette.foreground);
}

pub fn checks(run: &mut dyn FnMut(&str, Check)) {
    run("scene: immediate/replay pixel parity and clipped damage", &|shots| {
        let mode = std::rc::Rc::new(std::cell::Cell::new(0));
        let mut win = AppWindow::new("Scene parity", 320, 240);
        let mut body = win.body();
        body.set_margin(0);
        let mut canvas = fltk::frame::Frame::default().with_size(280, 150);
        body.add(&canvas);
        let m = mode.clone();
        canvas.draw(move |w| {
            let (x, y) = (w.x(), w.y());
            match m.get() {
                0 => content(x, y),
                1 => Scene::record(|| content(x, y)).0.draw(),
                _ => {
                    let bg = theme::with(|t| t.palette.background);
                    paint::fill(x, y, w.w(), w.h(), 0, bg);
                    let (s, ()) = Scene::record(|| content(x, y));
                    // Tile the complete image through disjoint damage regions.
                    for yy in (0..150).step_by(17) {
                        for xx in (0..280).step_by(23) {
                            s.draw_damage(Rect::new(x + xx, y + yy, 23.min(280 - xx), 17.min(150 - yy)));
                        }
                    }
                }
            }
        });
        win.end();
        let direct = shots.join("scene-direct.png");
        win.snapshot(&direct).map_err(|e| e.to_string())?;
        for i in 1..=2 {
            mode.set(i);
            let p = shots.join(format!("scene-replay-{i}.png"));
            win.snapshot(&p).map_err(|e| e.to_string())?;
            // PNGs have deterministic encoding in this test; byte equality
            // verifies the whole rendered surface without per-pixel I/O.
            check(std::fs::read(&direct).map_err(|e| e.to_string())? == std::fs::read(&p).map_err(|e| e.to_string())?, &format!("whole image parity, mode {i}; sample {:?}", pixel(&p, canvas.x() + 40, canvas.y() + 60)?))?;
        }
        fltk::window::DoubleWindow::delete((*win).clone());
        Ok(())
    });
}
