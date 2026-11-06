//! Native partial redraws with the documented background-owning host.
use super::*;
use fltk_omarchy::{anim, paint, window};

fn wait_draw(draws: &Cell<usize>, before: usize) -> Result<(), String> {
    super::super::pump_until(|| draws.get() > before);
    check(draws.get() > before, "native host paints the pending frame")
}
fn compare(win: &mut fltk::window::DoubleWindow, shots: &Path, name: &str) -> Result<(), String> {
    let actual = window::read_pixels(win).ok_or("native window readback")?;
    let path = shots.join(format!("{name}-fresh.png"));
    window::snapshot(win, &path).map_err(|e| e.to_string())?;
    let expected = super::super::png_rgb(&path)?;
    fltk_omarchy::png::write(
        &shots.join(format!("{name}-native.png")),
        win.w() as u32,
        win.h() as u32,
        3,
        &actual,
    )
    .map_err(|e| e.to_string())?;
    let mut delta = 0;
    for y in 0..win.h() {
        for x in 0..win.w() {
            let i = ((y * win.w() + x) * 3) as usize;
            for (c, v) in expected.at(x, y).iter().enumerate() {
                delta = delta.max(v.abs_diff(actual[i + c]));
            }
        }
    }
    println!("{name}: native/fresh max_rgb_delta={delta}");
    check(
        delta <= 2,
        &format!("{name}: native partial frame versus fresh background max RGB delta {delta}"),
    )
}
pub fn checks(run: &mut dyn FnMut(&str, &dyn Fn(&Path) -> Result<(), String>)) {
    run(
        "tabs keyed: plain FLTK background host clears hover and every animated native frame",
        &|shots| {
            struct Restore;
            impl Drop for Restore {
                fn drop(&mut self) {
                    anim::clock::resume();
                    anim::set_enabled(false);
                }
            }
            let _restore = Restore;
            anim::set_enabled(false);
            let selected = State::new(0usize);
            let mut win = fltk::window::DoubleWindow::new(100, 100, 620, 150, "Plain tabs");
            win.set_color(theme::with(|t| t.palette.background.fl()));
            let mut host = fltk::group::Group::new(8, 8, 300, 32, None);
            host.set_frame(fltk::enums::FrameType::NoBox);
            let mut bar = TabBar::new(&["First", "Second", "Third"], &selected);
            bar.resize(8, 8, 300, 32);
            host.end();
            let draws = Rc::new(Cell::new(0));
            let count = draws.clone();
            host.draw(move |g| {
                count.set(count.get() + 1);
                theme::with(|t| paint::fill(g.x(), g.y(), g.w(), g.h(), 0, t.palette.background));
                for i in 0..g.children() {
                    if let Some(mut child) = g.child(i) {
                        if child.visible() {
                            g.draw_child(&mut child);
                        }
                    }
                }
            });
            host.super_draw(false);
            let mut sibling = fltk::frame::Frame::new(330, 8, 280, 120, None);
            sibling.set_frame(fltk::enums::FrameType::FlatBox);
            sibling.set_color(theme::with(|t| t.palette.accent.fl()));
            win.end();
            win.show();
            wait_draw(&draws, 0)?;
            compare(&mut win, shots, "tabs-host-initial")?;
            let (x, width) = bar.tab_rect(1).ok_or("second tab")?;
            let before = draws.get();
            focus::hover_at(&win, x + width / 2, bar.y() + bar.h() / 2);
            // First motion enters the child; the next delivers FL_MOVE.
            focus::hover_at(&win, x + width / 2, bar.y() + bar.h() / 2);
            wait_draw(&draws, before)?;
            compare(&mut win, shots, "tabs-host-hover")?;
            let before = draws.get();
            focus::hover_at(&win, 500, 110);
            wait_draw(&draws, before)?;
            compare(&mut win, shots, "tabs-host-hover-cleared")?;
            anim::set_enabled(true);
            anim::clock::freeze();
            let before = draws.get();
            selected.set(2);
            wait_draw(&draws, before)?;
            compare(&mut win, shots, "tabs-host-motion-start")?;
            for (ms, name) in [
                (60, "tabs-host-motion-middle"),
                (250, "tabs-host-motion-end"),
            ] {
                let before = draws.get();
                anim::clock::advance(Duration::from_millis(ms));
                wait_draw(&draws, before)?;
                compare(&mut win, shots, name)?;
            }
            let before = draws.get();
            let end = Instant::now() + Duration::from_millis(80);
            while Instant::now() < end {
                App::pump();
                std::thread::sleep(Duration::from_millis(5));
            }
            check(
                draws.get() == before,
                "settled tabs stop scheduling native frames",
            )?;
            fltk::window::DoubleWindow::delete(win);
            Ok(())
        },
    );
}
