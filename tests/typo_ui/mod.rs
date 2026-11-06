//! UI checks of font weights and letter spacing (`paint::Weight`,
//! `paint::text_spaced`).
use super::{check, pixel, pump_until};
use fltk::prelude::*;
use fltk_omarchy::paint::{self, Weight};
use fltk_omarchy::{theme, AppWindow};
use std::cell::Cell;
use std::path::Path;
use std::rc::Rc;

type Check<'a> = &'a dyn Fn(&Path) -> Result<(), String>;

pub fn checks(run: &mut dyn FnMut(&str, Check)) {
    run("paint: font weights (semibold has at least regular's ink) and letter spacing", &|shots| {
        let mut win = AppWindow::new("Type", 420, 200);
        let mut body = win.body();
        let mut canvas = fltk::widget::Widget::default();
        body.add(&canvas);
        let widths = Rc::new(Cell::new((0, 0)));
        let wd = widths.clone();
        canvas.draw(move |w| {
            theme::with(|t| {
                let fg = t.palette.foreground;
                paint::text_weight("Working on it", w.x() + 10, w.y() + 10, 300, 30, 20, Weight::Regular, fg, fltk::enums::Align::Left);
                paint::text_weight("Working on it", w.x() + 10, w.y() + 50, 300, 30, 20, Weight::SemiBold, fg, fltk::enums::Align::Left);
                let a = paint::text_spaced("SECTION", w.x() + 10, w.y() + 90, 380, 24, 12, Weight::Medium, 0.0, fg, fltk::enums::Align::Left);
                let b = paint::text_spaced("SECTION", w.x() + 10, w.y() + 120, 380, 24, 12, Weight::Medium, 2.0, fg, fltk::enums::Align::Left);
                wd.set((a, b));
            });
        });
        win.end();
        win.show();
        let c2 = canvas.clone();
        pump_until(|| c2.w() > 0);
        let p = shots.join("typo-weights.png");
        win.clone().snapshot(&p).map_err(|e| e.to_string())?;
        let bg = theme::with(|t| t.palette.background);
        let ink = |y0: i32| -> Result<u32, String> {
            let mut n = 0;
            for y in y0..y0 + 30 {
                for x in canvas.x() + 10..canvas.x() + 310 {
                    let [r, g, b] = pixel(&p, x, canvas.y() + y)?;
                    n += (r.abs_diff(bg.r()) as u32 + g.abs_diff(bg.g()) as u32 + b.abs_diff(bg.b()) as u32) / 3;
                }
            }
            Ok(n)
        };
        let (regular, semibold) = (ink(10)?, ink(50)?);
        check(regular > 0 && semibold >= regular, &format!("ink regular {regular} semibold {semibold}"))?;
        let (a, b) = widths.get();
        check(b == a + 2 * 6 || b == a + 2 * 6 + 1, &format!("letter spacing adds 2 px between 7 letters: {a} → {b}"))?;
        fltk::window::DoubleWindow::delete((*win).clone());
        Ok(())
    });
}
