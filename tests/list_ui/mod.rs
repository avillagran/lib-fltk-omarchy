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

    run("list: the wheel goes to the scroll area around it at the list's edges", &|_| {
        let count = State::new(20usize);
        let mut win = AppWindow::new("Nested", 360, 300);
        let mut body = win.body();
        let mut outer = fltk::group::Scroll::default();
        outer.set_type(fltk::group::ScrollType::Vertical);
        body.add(&outer);
        outer.begin();
        let mut list = List::new(&count, |i| ListRow::new(&format!("row {i}"))).row_height(32);
        list.resize(0, 0, 300, 150);
        let mut tail = fltk::frame::Frame::default();
        tail.resize(0, 150, 300, 600);
        outer.end();
        win.end();
        win.show();
        let o2 = outer.clone();
        pump_until(|| o2.w() > 0 && o2.h() > 100);
        // Children of a Scroll keep their positions relative to it.
        list.resize(outer.x(), outer.y(), 300, 150);
        tail.resize(outer.x(), outer.y() + 150, 300, 600);
        outer.redraw();
        fltk::app::wait_for(0.01).ok();
        let (x, y) = (list.x() + 40, list.y() + 60);
        let max = 20 * 32 - 150;
        for _ in 0..10 {
            focus::wheel_at(&*win, x, y, 1);
            if list.scroll_offset() >= max as i64 {
                break;
            }
        }
        check(list.scroll_offset() == max as i64, &format!("the list scrolls to its end first: {}", list.scroll_offset()))?;
        check(outer.yposition() == 0, &format!("the outer area has not moved: {}", outer.yposition()))?;
        focus::wheel_at(&*win, x, y, 1);
        check(outer.yposition() > 0, &format!("at the list's end the wheel scrolls the outer area: {}", outer.yposition()))?;
        // Back up: the list (now higher on screen) takes the wheel again.
        let (lx, ly) = (list.x() + 40, list.y() + 60);
        let before = list.scroll_offset();
        focus::wheel_at(&*win, lx, ly.max(outer.y() + 5), -1);
        check(list.scroll_offset() < before, &format!("wheel up scrolls the list again: {before} → {}", list.scroll_offset()))?;
        fltk::window::DoubleWindow::delete((*win).clone());
        Ok(())
    });

    run("rich text: the wheel goes to the scroll area around it at its end", &|_| {
        use fltk_omarchy::text::RichText;
        let mut win = AppWindow::new("Nested text", 360, 300);
        let mut body = win.body();
        let mut outer = fltk::group::Scroll::default();
        outer.set_type(fltk::group::ScrollType::Vertical);
        body.add(&outer);
        outer.begin();
        let mut rt = RichText::new();
        rt.resize(0, 0, 300, 150);
        let mut tail = fltk::frame::Frame::default();
        tail.resize(0, 150, 300, 600);
        outer.end();
        win.end();
        win.show();
        let o2 = outer.clone();
        pump_until(|| o2.w() > 0 && o2.h() > 100);
        rt.resize(outer.x(), outer.y(), 300, 150);
        tail.resize(outer.x(), outer.y() + 150, 300, 600);
        for k in 0..30 {
            rt.append_markdown(&format!("Paragraph {k} with enough words to take a line or two."));
        }
        outer.redraw();
        fltk::app::wait_for(0.01).ok();
        let (x, y) = (rt.x() + 40, rt.y() + 60);
        for _ in 0..80 {
            if rt.is_at_bottom() {
                break;
            }
            focus::wheel_at(&*win, x, y, 1);
        }
        check(rt.is_at_bottom() && outer.yposition() == 0, &format!("the text scrolls to its end first (outer {})", outer.yposition()))?;
        focus::wheel_at(&*win, x, y, 1);
        check(outer.yposition() > 0, "at its end the wheel scrolls the outer area")?;
        fltk::window::DoubleWindow::delete((*win).clone());
        Ok(())
    });
}
