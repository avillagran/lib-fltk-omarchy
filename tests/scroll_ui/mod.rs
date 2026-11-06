//! UI checks of scroll blits: a RichText frame that only scrolled moves
//! the pixels on screen (fl_scroll) and paints the exposed strip; the
//! result must equal a full repaint, pixel for pixel, both ways.
use super::{check, pump_until};
use fltk::prelude::*;
use fltk_omarchy::text::RichText;
use fltk_omarchy::AppWindow;
use std::path::Path;

type Check<'a> = &'a dyn Fn(&Path) -> Result<(), String>;

fn settle() {
    for _ in 0..8 {
        fltk::app::flush();
        let _ = fltk::app::wait_for(0.03);
    }
}

fn grab(win: &mut fltk::window::DoubleWindow) -> Result<Vec<u8>, String> {
    fltk_omarchy::window::read_pixels(win).ok_or_else(|| "read_pixels".to_string())
}

pub fn checks(run: &mut dyn FnMut(&str, Check)) {
    run("scroll blit: moved pixels + exposed strip == full repaint (down and up)", &|_| {
        let mut win = AppWindow::new("Blit", 520, 400);
        let rt = RichText::new();
        win.end();
        win.show();
        for k in 0..80 {
            rt.append_markdown(&format!("Paragraph **{k}** with `code`, a [link](https://example.com/{k}) and enough words to wrap once or twice in this width."));
        }
        let r2 = rt.clone();
        pump_until(|| r2.w() > 0);
        settle();
        let mut dw = (*win).clone();
        let supported = cfg!(not(target_env = "msvc"));
        let mut blits = 0;
        for (step, top) in [(0, 300i64), (1, 340), (2, 120), (3, 900)] {
            let before = rt.scroll_blits();
            rt.scroll_to(top);
            settle();
            // A frame where something else changed too (the window got
            // focus and its header repainted) is painted in full instead.
            let blitted = rt.scroll_blits() > before;
            blits += blitted as u32;
            let a = grab(&mut dw)?;
            rt.clone().redraw();
            settle();
            let b = grab(&mut dw)?;
            check(a.len() == b.len() && !a.is_empty(), "captures")?;
            let diff = a.iter().zip(&b).filter(|(x, y)| x.abs_diff(**y) > 2).count();
            check(diff == 0, &format!("step {step} (top {top}, blit {blitted}): {diff} bytes differ from a full repaint"))?;
        }
        if supported {
            check(blits == 3, &format!("scroll-only frames moved pixels ({blits} of 3 small steps)"))?;
        }
        fltk::window::DoubleWindow::delete((*win).clone());
        Ok(())
    });
}
