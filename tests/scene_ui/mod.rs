//! Shared-painter parity and damage: compare every pixel, including clips,
//! rounded borders, literal '@', alpha over varied backgrounds and text.
use super::{check, png_rgb};
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
    let image = scene::Image::rgba(2, 2, vec![
        230, 110, 20, 128, 0, 0, 0, 0,
        20, 170, 240, 255, 80, 30, 100, 64,
    ]).unwrap();
    scene::clip(Rect::new(x + 170, y + 70, 55, 50), || {
        image.draw(Rect::new(x + 160, y + 60, 80, 70));
    });
    let (child, ()) = Scene::record(|| paint::fill(-10, -5, 20, 20, 4, t.palette.accent));
    let child = scene::Layer::new(Rect::new(-10, -5, 20, 20), child).unwrap();
    assert!(child.draw_at(x + 235, y + 20, 0.5));
    // Outside the previous clips: catches unbalanced replay stacks.
    paint::fill(x + 262, y + 130, 12, 12, 3, t.palette.foreground);
}

pub fn checks(run: &mut dyn FnMut(&str, Check)) {
    run("scene: isolated group opacity, translation, nested layers and live backdrop", &|shots| {
        let t = theme::current();
        let (a, b, bg, other) = (t.palette.accent, t.palette.foreground, t.palette.background, t.palette.muted);
        let (child, ()) = Scene::record(|| {
            paint::fill(30, 40, 70, 60, 8, a);
            paint::fill(60, 50, 60, 60, 0, b);
        });
        let layer = scene::Layer::new(Rect::new(30, 40, 100, 80), child).map_err(|e| e.to_string())?;
        let (nested, ()) = Scene::record(|| { assert!(layer.draw_at(-20, -10, 0.5)); });
        let nested = scene::Layer::new(Rect::new(-20, -10, 100, 80), nested).map_err(|e| e.to_string())?;
        let mode = std::rc::Rc::new(std::cell::Cell::new(0));
        let mut win = AppWindow::new("Isolated scene layers", 260, 240);
        let mut body = win.body();
        body.set_margin(0);
        let mut frame = fltk::frame::Frame::default();
        body.add(&frame);
        let (m, l, n) = (mode.clone(), layer.clone(), nested.clone());
        frame.draw(move |w| {
            let (x, y) = (w.x(), w.y());
            let base = if m.get() == 1 { other } else { bg };
            paint::fill(x, y, w.w(), w.h(), 0, base);
            // Lower half has a different backdrop, including under the
            // overlap: proves group alpha is not a flat-parent-color veil.
            paint::fill(x, y + 60, w.w(), w.h() - 60, 0, a);
            let (s, ()) = Scene::record(|| {
                if m.get() == 2 { assert!(n.draw_at(x + 20, y + 20, 0.5)); }
                else { assert!(l.draw_at(x + 20, y + 20, 0.5)); }
            });
            s.draw();
        });
        win.end();
        for i in 0..3 {
            mode.set(i);
            let p = shots.join(format!("scene-layer-{i}.png"));
            win.snapshot(&p).map_err(|e| e.to_string())?;
            let image = png_rgb(&p)?;
            let (x, y) = (frame.x(), frame.y());
            let base = if i == 1 { other } else { bg };
            let opacity = if i == 2 { (128.0 / 255.0) * (128.0 / 255.0) } else { 128.0 / 255.0 };
            check(close(image.at(x + 20, y + 20), base), "rounded transparent corner preserves current background")?;
            check(close(image.at(x + 30, y + 30), a.over(base, opacity)), "translated child over current background")?;
            check(close(image.at(x + 65, y + 45), b.over(base, opacity)), "overlap uses only top child, group alpha applied once")?;
            check(close(image.at(x + 65, y + 75), b.over(a, opacity)), "same layer over second background stripe")?;
            check(close(image.at(x + 110, y + 95), a), "transparent padding leaves background untouched")?;
        }
        nested.clear_raster();
        layer.clear_raster();
        fltk::window::DoubleWindow::delete((*win).clone());
        Ok(())
    });
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
                            assert!(s.draw_damage(Rect::new(x + xx, y + yy, 23.min(280 - xx), 17.min(150 - yy))));
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
            let expected = png_rgb(&direct)?;
            let actual = png_rgb(&p)?;
            let mut mismatches = Vec::new();
            let mut count = 0usize;
            let mut max_delta = 0u8;
            let height = expected.raw.len() / (expected.w * expected.depth + 1);
            for yy in 0..height as i32 {
                for xx in 0..expected.w as i32 {
                    let a = expected.at(xx, yy);
                    let b = actual.at(xx, yy);
                    if a != b {
                        count += 1;
                        max_delta = max_delta.max((0..3).map(|c| a[c].abs_diff(b[c])).max().unwrap());
                        if mismatches.len() < 12 {
                            mismatches.push((xx - canvas.x(), yy - canvas.y(), a, b));
                        }
                    }
                }
            }
            check(count == 0, &format!("whole image parity, mode {i}: {count} differing pixels, max channel delta {max_delta}, first local (x,y,expected,actual): {mismatches:?}"))?;
        }
        fltk::window::DoubleWindow::delete((*win).clone());
        Ok(())
    });
}

fn close(actual: [u8; 3], expected: theme::Rgb) -> bool {
    actual[0].abs_diff(expected.r()) <= 2 && actual[1].abs_diff(expected.g()) <= 2 && actual[2].abs_diff(expected.b()) <= 2
}
