//! UI checks of reorderable lists: drag past the threshold with a drop
//! line, a plain click does not move, Alt+Up / Alt+Down.
use super::{check, pixel, pump_until};
use fltk::prelude::*;
use fltk_omarchy::focus::{self, press};
use fltk_omarchy::layout::FlexExt;
use fltk_omarchy::list::{List, ListRow};
use fltk_omarchy::{theme, AppWindow, State};
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

type Check<'a> = &'a dyn Fn(&Path) -> Result<(), String>;

pub fn checks(run: &mut dyn FnMut(&str, Check)) {
    run("list: reorder by dragging (threshold, drop line) and Alt+Up/Down", &|shots| {
        let items = Rc::new(RefCell::new(vec!["a", "b", "c", "d", "e"]));
        let count = State::new(5usize);
        let mut win = AppWindow::new("Reorder", 360, 300);
        let mut body = win.body();
        let it = items.clone();
        let it2 = items.clone();
        let mut list = List::new(&count, move |i| ListRow::new(it.borrow()[i]))
            .row_height(32)
            .reorderable(move |from, to| {
                let mut v = it2.borrow_mut();
                let x = v.remove(from);
                v.insert(to, x);
            });
        list.set_size(300, 200);
        body.fit(&*list);
        win.end();
        win.show();
        let l2 = list.clone();
        pump_until(|| l2.w() > 0 && l2.h() > 100);
        let row_y = |i: i32| list.y() + i * 32 + 16;
        let x = list.x() + 40;
        // A click with less than 5 px of travel selects, does not move.
        focus::mouse_drag(&*win, (x, row_y(0)), (x, row_y(0) + 3), true);
        check(*items.borrow() == ["a", "b", "c", "d", "e"], "a small drag does not reorder")?;
        // Drag "a" below "c": shown as a line at gap 3 while dragging.
        focus::mouse_drag(&*win, (x, row_y(0)), (x, list.y() + 3 * 32 + 2), false);
        check(list.drop_point() == Some(3), &format!("drop point while dragging: {:?}", list.drop_point()))?;
        let p = shots.join("list-reorder-drag.png");
        win.clone().snapshot(&p).map_err(|e| e.to_string())?;
        let accent = theme::with(|t| t.palette.accent);
        let [r, g, b] = pixel(&p, x, list.y() + 3 * 32)?;
        let near = |a: u8, b: u8| a.abs_diff(b) <= 3;
        check(near(r, accent.r()) && near(g, accent.g()) && near(b, accent.b()), &format!("drop line in the accent: {:?}", (r, g, b)))?;
        focus::mouse_up(&*win, x, list.y() + 3 * 32 + 2);
        check(*items.borrow() == ["b", "c", "a", "d", "e"], &format!("dropped below c: {:?}", items.borrow()))?;
        check(list.selected() == Some(2) && list.drop_point().is_none(), "the moved row stays selected; no drop line after")?;
        // Upward: "e" to the top.
        focus::mouse_drag(&*win, (x, row_y(4)), (x, list.y() + 2), true);
        check(*items.borrow() == ["e", "b", "c", "a", "d"], &format!("dragged to the top: {:?}", items.borrow()))?;
        // Keyboard: Alt+Down twice, Alt+Up once on the selected row (index 0).
        let _ = list.take_focus();
        list.select(0);
        press("Alt+Down");
        press("Alt+Down");
        press("Alt+Up");
        check(*items.borrow() == ["b", "e", "c", "a", "d"], &format!("Alt+arrows: {:?}", items.borrow()))?;
        check(list.selected() == Some(1), "selection follows the item")?;
        press("Down");
        check(list.selected() == Some(2) && *items.borrow() == ["b", "e", "c", "a", "d"], "plain Down still moves the selection only")?;
        fltk::window::DoubleWindow::delete((*win).clone());
        Ok(())
    });
}
