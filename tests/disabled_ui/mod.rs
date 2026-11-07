//! Disabled controls keep native editing state and revoke old gestures.
use super::{check, png_rgb, Rgb};
use fltk::{enums::Event, prelude::*};
use fltk_omarchy::{
    focus::{self, press},
    layout::{self, FlexExt},
    overlay::Sheet,
    widgets::{Button, TextField},
    App, AppWindow, State,
};
use std::{
    cell::{Cell, RefCell},
    path::Path,
    rc::Rc,
};

type Check<'a> = &'a dyn Fn(&Path) -> Result<(), String>;

fn picture(win: &mut AppWindow, shots: &Path, name: &str) -> Result<Rgb, String> {
    let path = shots.join(format!("{name}.png"));
    win.snapshot(&path).map_err(|error| error.to_string())?;
    png_rgb(&path)
}

fn pixels<W: WidgetExt>(image: &Rgb, widget: &W) -> Vec<[u8; 3]> {
    (widget.y()..widget.y() + widget.h())
        .flat_map(|y| (widget.x()..widget.x() + widget.w()).map(move |x| image.at(x, y)))
        .collect()
}

fn centre<W: WidgetExt>(widget: &W) -> (i32, i32) {
    (widget.x() + widget.w() / 2, widget.y() + widget.h() / 2)
}

pub fn checks(run: &mut dyn FnMut(&str, Check)) {
    run(
        "disabled controls: primary and native fields recover exact pixels and editing state",
        &|shots| {
            let mut win = AppWindow::new("Disabled controls", 440, 300);
            let mut body = win.body();
            let anchor = Button::new("Anchor");
            body.fit(&*anchor);
            let mut group = layout::column();
            let calls = Rc::new(Cell::new(0));
            let observed = calls.clone();
            let mut button = Button::new("Apply")
                .primary()
                .on_click(move || observed.set(observed.get() + 1));
            group.fit(&*button);
            let draft = State::new("abé日本%20".to_owned());
            let field = TextField::new().bind(&draft);
            group.fit(&*field);
            let mut empty = TextField::new().placeholder("Name");
            group.fit(&*empty);
            group.close_fitted();
            win.end();
            let _ = anchor.clone().take_focus();
            let mut input = field.input();
            input.set_position(4).map_err(|error| error.to_string())?;
            input.set_mark(2).map_err(|error| error.to_string())?;
            focus::hover_at(&*win, 1, 1);
            let enabled = picture(&mut win, shots, "disabled-controls-enabled")?;
            let positions = (input.position(), input.mark());
            button.deactivate();
            input.deactivate();
            empty.deactivate();
            let disabled = picture(&mut win, shots, "disabled-controls-inactive")?;
            check(
                pixels(&enabled, &*button) != pixels(&disabled, &*button),
                "primary still looks enabled",
            )?;
            check(
                pixels(&enabled, &*field) != pixels(&disabled, &*field),
                "input-only inactive chrome was unchanged",
            )?;
            check(
                pixels(&enabled, &*empty) != pixels(&disabled, &*empty),
                "parent-inactive placeholder/chrome was unchanged",
            )?;
            let (x, y) = centre(&*button);
            focus::hover_at(&*win, x, y);
            focus::click_at(&*win, x, y);
            press("Enter");
            let hot = picture(&mut win, shots, "disabled-controls-no-hover")?;
            check(
                pixels(&disabled, &*button) == pixels(&hot, &*button),
                "disabled hover or press changed pixels",
            )?;
            check(calls.get() == 0, "disabled action fired")?;
            check(
                draft.get() == "abé日本%20" && field.value() == draft.get(),
                "disabled styling changed the draft",
            )?;
            check(
                (input.position(), input.mark()) == positions,
                "disabled styling reset caret or selection",
            )?;
            button.activate();
            input.activate();
            empty.activate();
            focus::hover_at(&*win, 1, 1);
            let _ = anchor.clone().take_focus();
            let restored = picture(&mut win, shots, "disabled-controls-restored")?;
            for widget in [
                button.as_base_widget(),
                field.as_base_widget(),
                empty.as_base_widget(),
            ] {
                check(
                    pixels(&enabled, &widget) == pixels(&restored, &widget),
                    "re-enabled pixels differ from baseline",
                )?;
            }
            check(
                (input.position(), input.mark()) == positions,
                "re-enable reset caret or selection",
            )?;
            fltk::window::DoubleWindow::delete((*win).clone());
            Ok(())
        },
    );

    run(
        "disabled controls: deactivate, parent disable and warm Sheet revoke stale presses",
        &|shots| {
            let mut win = AppWindow::new("Revoked presses", 440, 300);
            let mut body = win.body();
            let anchor = Button::new("Open");
            body.fit(&*anchor);
            let mut group = layout::column();
            let calls = Rc::new(Cell::new(0));
            let observed = calls.clone();
            let mut button = Button::new("Apply")
                .primary()
                .on_click(move || observed.set(observed.get() + 1));
            group.fit(&*button);
            group.close_fitted();
            win.end();
            win.show();
            App::pump();
            picture(&mut win, shots, "disabled-controls-gesture-owner")?;
            for phase in 0..3 {
                let at = centre(&*button);
                focus::mouse_drag(&*win, at, at, false);
                match phase {
                    0 => {
                        button.deactivate();
                        button.activate();
                    }
                    1 => {
                        group.deactivate();
                        group.activate();
                    }
                    _ => {
                        group.hide();
                        group.show();
                    }
                }
                focus::mouse_up(&*win, at.0, at.1);
                check(
                    calls.get() == phase,
                    "an old press activated after a lifecycle change",
                )?;
                focus::click_at(&*win, at.0, at.1);
                check(calls.get() == phase + 1, "fresh re-enabled press was lost")?;
            }
            let _ = button.take_focus();
            group.deactivate();
            press("Enter");
            check(calls.get() == 3, "parent-inactive keyboard action fired")?;
            group.activate();
            let slot = Rc::new(RefCell::new(None::<Button>));
            let epoch = Rc::new(Cell::new(1));
            let delivered = Rc::new(RefCell::new(Vec::new()));
            let (out, current, events) = (slot.clone(), epoch.clone(), delivered.clone());
            let sheet = Sheet::new(&*anchor, "Retained action", 320, 180, move |body, _| {
                let (current, events) = (current.clone(), events.clone());
                let button = Button::new("Apply")
                    .primary()
                    .on_click(move || events.borrow_mut().push(current.get()));
                body.fit(&*button);
                *out.borrow_mut() = Some(button);
            });
            sheet.open();
            picture(&mut win, shots, "disabled-controls-sheet-open")?;
            let button = slot.borrow().clone().ok_or("sheet button missing")?;
            let at = centre(&*button);
            focus::mouse_drag(&*win, at, at, false);
            sheet.close();
            epoch.set(2);
            sheet.open();
            focus::mouse_up(&*win, at.0, at.1);
            check(
                delivered.borrow().is_empty(),
                "old Sheet press reached the current callback epoch",
            )?;
            focus::click_at(&*win, at.0, at.1);
            check(
                *delivered.borrow() == [2],
                "fresh warm-Sheet press did not reach its epoch",
            )?;
            // A retained but hidden child must also reject direct native dispatch.
            sheet.close();
            let mut hidden = button.clone();
            hidden.handle_event(Event::Released);
            check(*delivered.borrow() == [2], "hidden retained action fired")?;
            fltk::window::DoubleWindow::delete((*win).clone());
            Ok(())
        },
    );
}
