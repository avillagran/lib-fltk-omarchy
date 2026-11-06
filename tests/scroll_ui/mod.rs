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
            // X11 matches exactly; Cairo on Wayland can antialias a glyph cut
            // by the strip's edge a level or two differently (a handful of
            // pixels). A seam or a shifted line would be thousands of bytes.
            let diff = a.iter().zip(&b).filter(|(x, y)| x.abs_diff(**y) > 2).count();
            check(diff <= 96, &format!("step {step} (top {top}, blit {blitted}): {diff} bytes differ from a full repaint"))?;
        }
        if supported {
            check(blits == 3, &format!("scroll-only frames moved pixels ({blits} of 3 small steps)"))?;
        }
        fltk::window::DoubleWindow::delete((*win).clone());
        Ok(())
    });

    run("stream blit: growing the last message while following moves pixels == full repaint", &|_| {
        let mut win = AppWindow::new("Stream blit", 520, 400);
        let rt = RichText::new().stick_to_bottom(true);
        win.end();
        win.show();
        for k in 0..60 {
            rt.append_markdown(&format!("Paragraph **{k}** with enough words to wrap once or twice in this width."));
        }
        rt.append_markdown("Reply: ");
        let r2 = rt.clone();
        pump_until(|| r2.w() > 0 && r2.is_at_bottom());
        settle();
        let mut dw = (*win).clone();
        let supported = cfg!(not(target_env = "msvc"));
        let before = rt.scroll_blits();
        let chunks = ["the snapshot ", "is now a diff ", "keyed by id, ", "and the view ", "follows the end ", "while it grows.\n\nA new paragraph ", "starts here ", "and wraps onto a second line soon enough."];
        for (step, c) in chunks.iter().enumerate() {
            rt.push_markdown(c);
            settle();
            check(rt.is_at_bottom(), &format!("step {step}: still at the end"))?;
            let a = grab(&mut dw)?;
            rt.clone().redraw();
            settle();
            let b = grab(&mut dw)?;
            let diff = a.iter().zip(&b).filter(|(x, y)| x.abs_diff(**y) > 2).count();
            check(diff <= 96, &format!("step {step} ({c:?}): {diff} bytes differ from a full repaint"))?;
        }
        let blits = rt.scroll_blits() - before;
        if supported {
            check(blits as usize >= chunks.len() - 1, &format!("streamed frames moved pixels: {blits} of {}", chunks.len()))?;
        }
        fltk::window::DoubleWindow::delete((*win).clone());
        Ok(())
    });

    run("stream pace: pushes between paced frames are painted together, content exact", &|_| {
        use fltk_omarchy::anim::Pace;
        let mut win = AppWindow::new("Stream pace", 520, 400);
        let rt = RichText::new().stick_to_bottom(true).stream_pace(Pace::Low);
        win.end();
        win.show();
        for k in 0..40 {
            rt.append_markdown(&format!("Paragraph {k} with enough words to wrap once or twice."));
        }
        rt.append_markdown("Reply: ");
        let r2 = rt.clone();
        pump_until(|| r2.w() > 0 && r2.is_at_bottom());
        settle();
        let frames0 = rt.paint_stats().0;
        for w in ["one ", "two ", "three ", "four ", "five ", "six "] {
            rt.push_markdown(w);
        }
        settle();
        let frames = rt.paint_stats().0 - frames0;
        check(rt.plain_text().trim_end().ends_with("one two three four five six"), "all pushed text is shown")?;
        check(frames <= 2, &format!("six pushes, {frames} frames"))?;
        check(rt.is_at_bottom(), "still following")?;
        fltk::window::DoubleWindow::delete((*win).clone());
        Ok(())
    });
}
