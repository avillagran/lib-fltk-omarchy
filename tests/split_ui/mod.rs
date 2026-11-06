//! UI checks of `split::Split`: mouse drag with clamping, keys in screen
//! directions (pane on either side, right-to-left rows, columns), reset,
//! the two-way State and the focus ring pixels.
use super::{check, pixel, pump_until};
use fltk::prelude::*;
use fltk_omarchy::focus::{self, press};
use fltk_omarchy::layout::{Direction, FlexExt};
use fltk_omarchy::split::Split;
use fltk_omarchy::{theme, AppWindow, State};
use std::path::Path;

type Check<'a> = &'a dyn Fn(&Path) -> Result<(), String>;

struct Fixture {
    win: AppWindow,
    pane: fltk::frame::Frame,
    split: Split,
    size: State<i32>,
}

/// A 600×300 window: `pane | grip | rest` (or `rest | grip | pane`), in a
/// row or a column.
fn fixture(pane_first: bool, column: bool, dir: Option<Direction>) -> Fixture {
    let mut win = AppWindow::new("Split", 600, 300);
    let mut body = win.body();
    body.set_margin(0);
    // A container that fills the body (not a fitted row of natural height).
    let mut flex = fltk::group::Flex::default();
    if column {
        flex.set_type(fltk::group::FlexType::Column);
    } else {
        flex.set_type(fltk::group::FlexType::Row);
    }
    body.add(&flex);
    flex.begin();
    if let Some(d) = dir {
        flex.direction(d);
    }
    flex.set_margin(0);
    flex.set_pad(0);
    let size = State::new(200);
    let mk = || {
        let mut f = fltk::frame::Frame::default();
        f.set_frame(fltk::enums::FrameType::FlatBox);
        f.set_color(theme::with(|t| t.palette.lighter_background).fl());
        f
    };
    let (pane, split) = if pane_first {
        let pane = mk();
        flex.add(&pane);
        let split = Split::new(&mut flex, &pane).range(100, 400).initial(200).bind(&size);
        flex.add(&mk());
        (pane, split)
    } else {
        flex.add(&mk());
        let pane = mk();
        let split = Split::new(&mut flex, &pane).range(100, 400).initial(200).bind(&size);
        flex.add(&pane);
        (pane, split)
    };
    flex.end();
    body.layout();
    win.end();
    win.show();
    let (sp, pn) = ((*split).clone(), pane.clone());
    pump_until(|| sp.w() > 0 && sp.h() > 100 && pn.w() > 0);
    Fixture { win, pane, split, size }
}

fn close(f: Fixture) {
    fltk::window::DoubleWindow::delete((*f.win).clone());
}

fn along(f: &Fixture, column: bool) -> i32 {
    if column { f.pane.h() } else { f.pane.w() }
}

fn centre(w: &impl WidgetExt) -> (i32, i32) {
    (w.x() + w.w() / 2, w.y() + w.h() / 2)
}

pub fn checks(run: &mut dyn FnMut(&str, Check)) {
    run("split: drag resizes the pane, clamps to the range, two-way State", &|_| {
        let f = fixture(true, false, None);
        check(along(&f, false) == 200, &format!("initial width {}", along(&f, false)))?;
        check(f.split.h() > 100, &format!("grip spans the body: {}", f.split.h()))?;
        let (x, y) = centre(&*f.split);
        focus::mouse_drag(&*f.win, (x, y), (x + 60, y), true);
        check(along(&f, false) == 260 && f.size.get() == 260, &format!("after +60: pane {} state {}", along(&f, false), f.size.get()))?;
        let (x, y) = centre(&*f.split);
        focus::mouse_drag(&*f.win, (x, y), (x + 500, y), true);
        check(f.split.size() == 400, &format!("clamped to max: {}", f.split.size()))?;
        f.size.set(150);
        fltk::app::wait_for(0.01).ok();
        check(along(&f, false) == 150, &format!("State set → pane {}", along(&f, false)))?;
        f.size.set(20);
        for _ in 0..3 {
            fltk::app::wait_for(0.01).ok();
        }
        check(f.split.size() == 100 && f.size.get() == 100, &format!("State below min clamps: {} / {}", f.split.size(), f.size.get()))?;
        close(f);
        Ok(())
    });

    run("split: keys move the grip on the screen; Home/End/Enter", &|_| {
        let f = fixture(true, false, None);
        let mut g = (*f.split).clone();
        let _ = g.take_focus();
        press("Right");
        check(f.split.size() == 210, &format!("Right grows a left pane: {}", f.split.size()))?;
        press("Left");
        press("Left");
        check(f.split.size() == 190, &format!("Left shrinks: {}", f.split.size()))?;
        press("End");
        check(f.split.size() == 400, "End → max")?;
        press("Home");
        check(f.split.size() == 100, "Home → min")?;
        press("Enter");
        check(f.split.size() == 200, "Enter → initial")?;
        close(f);
        Ok(())
    });

    run("split: a pane after the grip, a right-to-left row and a column", &|_| {
        // Pane on the right: dragging left and Left grow it.
        let f = fixture(false, false, None);
        let (x, y) = centre(&*f.split);
        focus::mouse_drag(&*f.win, (x, y), (x - 40, y), true);
        check(f.split.size() == 240, &format!("drag left grows a right pane: {}", f.split.size()))?;
        let _ = (*f.split).clone().take_focus();
        press("Left");
        check(f.split.size() == 250, &format!("Left grows it: {}", f.split.size()))?;
        close(f);

        // Right to left: the first pane is drawn on the right.
        let f = fixture(true, false, Some(Direction::Rtl));
        check(f.pane.x() > f.split.x(), "mirrored: pane right of the grip")?;
        let _ = (*f.split).clone().take_focus();
        press("Left");
        check(f.split.size() == 210, &format!("RTL: Left (towards the end) grows it: {}", f.split.size()))?;
        close(f);

        // Column: Down grows the top pane; dragging up shrinks it.
        let f = fixture(true, true, None);
        let _ = (*f.split).clone().take_focus();
        press("Down");
        check(along(&f, true) == 210, &format!("column Down: {}", along(&f, true)))?;
        let (x, y) = centre(&*f.split);
        focus::mouse_drag(&*f.win, (x, y), (x, y - 70), true);
        check(along(&f, true) == 140, &format!("column drag up: {}", along(&f, true)))?;
        close(f);
        Ok(())
    });

    run("split: the focus ring draws the grip in the accent", &|shots| {
        let f = fixture(true, false, None);
        let accent = theme::with(|t| t.palette.accent);
        let p = shots.join("split-idle.png");
        f.win.clone().snapshot(&p).map_err(|e| e.to_string())?;
        let (x, y) = centre(&*f.split);
        let at = |path: &Path| -> Result<(u8, u8, u8), String> {
            let [r, g, b] = pixel(path, x, y)?;
            Ok((r, g, b))
        };
        let idle = at(&p)?;
        check(idle != (accent.r(), accent.g(), accent.b()), "idle grip is not accent")?;
        let _ = (*f.split).clone().take_focus();
        press("Tab");
        press("Shift+Tab");
        check(focus::is_focused(&*f.split), "grip is a Tab stop")?;
        let p = shots.join("split-focus.png");
        f.win.clone().snapshot(&p).map_err(|e| e.to_string())?;
        let ring = at(&p)?;
        check(ring == (accent.r(), accent.g(), accent.b()), &format!("focused grip {ring:?} vs accent"))?;
        close(f);
        Ok(())
    });
}
