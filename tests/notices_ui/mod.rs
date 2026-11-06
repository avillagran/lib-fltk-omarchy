//! Persistent notice ownership, actions, accessibility and stack geometry.
use super::*;
use fltk_omarchy::overlay::{Notice, NoticeAction, NoticeDismiss};

type Check<'a> = &'a dyn Fn(&Path) -> Result<(), String>;
fn centre((x, y, w, h): (i32, i32, i32, i32)) -> (i32, i32) {
    (x + w / 2, y + h / 2)
}

pub fn checks(run: &mut dyn FnMut(&str, Check)) {
    run(
        "notices: action IDs, primary release ownership, keyboard and exact focus restoration",
        &|shots| {
            let mut win = AppWindow::new("Notice actions", 620, 400);
            let mut body = win.body();
            let mut pane = TextField::new();
            body.fit(&*pane);
            win.end();
            let _ = pane.take_focus();
            let previous = focus::current().map(|w| w.as_widget_ptr());
            let events = Rc::new(RefCell::new(Vec::new()));
            let out = events.clone();
            let reason = Rc::new(Cell::new(None));
            let done = reason.clone();
            let notice = Notice::new("Copy failed", "Try again or undo the operation.")
                .icon(glyph::BELL)
                .action(NoticeAction::new("retry", "Retry"))
                .action(NoticeAction::new("blocked", "Unavailable").disabled())
                .action(NoticeAction::new("undo", "Undo"))
                .on_action(move |n, id| {
                    out.borrow_mut().push(id.to_owned());
                    if id == "undo" {
                        n.close();
                    }
                })
                .on_dismiss(move |why| done.set(Some(why)))
                .show(&*pane)
                .ok_or("notice")?;
            win.snapshot(shots.join("notice-actions.png"))
                .map_err(|e| e.to_string())?;
            check(
                focus::current().map(|w| w.as_widget_ptr()) == previous,
                "show never steals pane focus",
            )?;
            let retry = centre(notice.action_rect("retry").ok_or("retry rect")?);
            let close = centre(notice.close_rect().ok_or("close rect")?);
            for button in [2, 3, 8, 9] {
                focus::click_button_at(&*win, retry.0, retry.1, button);
                focus::click_button_at(&*win, close.0, close.1, button);
            }
            check(
                notice.is_open()
                    && events.borrow().is_empty()
                    && focus::current().map(|w| w.as_widget_ptr()) == previous,
                "auxiliary action/close hits preserve actions and pane focus",
            )?;
            focus::mouse_drag(&*win, retry, (retry.0, retry.1 + 80), true);
            check(
                events.borrow().is_empty(),
                "release outside cancels primary action",
            )?;
            focus::click_at(&*win, retry.0, retry.1);
            check(
                events.borrow().as_slice() == ["retry"] && notice.is_open(),
                "one primary release requests action without implicit dismissal",
            )?;
            check(
                !notice.activate("blocked") && !notice.activate("missing"),
                "disabled and unknown actions ignored",
            )?;
            check(notice.focus_action("retry"), "focus action")?;
            focus::press("Right");
            focus::press("Enter");
            check(
                events.borrow().as_slice() == ["retry", "undo"] && !notice.is_open(),
                "arrows skip disabled; Enter dispatches stable ID; callback owns close",
            )?;
            check(
                reason.get() == Some(NoticeDismiss::Closed)
                    && focus::current().map(|w| w.as_widget_ptr()) == previous,
                "close once and exact pane focus returns",
            )?;
            notice.close();
            check(
                !notice.activate("retry") && events.borrow().len() == 2,
                "closed handles cannot run callbacks",
            )?;
            let why = reason.clone();
            let dismissible = Notice::new("Close explicitly", "")
                .on_dismiss(move |r| why.set(Some(r)))
                .show(&*win)
                .ok_or("close-only notice")?;
            let p = centre(dismissible.close_rect().ok_or("close-only hit")?);
            focus::click_at(&*win, p.0, p.1);
            check(
                !dismissible.is_open() && reason.get() == Some(NoticeDismiss::CloseButton),
                "primary close hit dismisses explicitly",
            )?;
            let why = reason.clone();
            let dismissible = Notice::new("Keyboard close", "")
                .on_dismiss(move |r| why.set(Some(r)))
                .show(&*win)
                .ok_or("keyboard notice")?;
            let _ = pane.take_focus();
            focus::press("Tab");
            focus::press("Escape");
            check(
                !dismissible.is_open()
                    && reason.get() == Some(NoticeDismiss::Keyboard)
                    && focus::current().map(|w| w.as_widget_ptr()) == previous,
                "Tab enters close-only card; Escape dismisses and restores pane focus",
            )?;
            fltk::window::DoubleWindow::delete((*win).clone());
            Ok(())
        },
    );
    run(
        "notices: stacks belong to a window, close gaps, coexist with legacy notifications",
        &|shots| {
            let mut a = AppWindow::new("A", 560, 420);
            a.end();
            let mut b = AppWindow::new("B", 460, 420);
            b.end();
            let a1 = Notice::new("A first", "").show(&*a).ok_or("A1")?;
            let a2 = Notice::new("A second", "")
                .action(NoticeAction::new("open", "Open"))
                .show(&*a)
                .ok_or("A2")?;
            let b1 = Notice::new("B first", "").show(&*b).ok_or("B1")?;
            let margin = theme::with(|t| t.gaps_out.max(8));
            let ar = a1.rect().ok_or("A first rect")?;
            check(
                ar.1 == margin && b1.rect().ok_or("B first rect")?.1 == margin,
                "each containing window starts its own stack",
            )?;
            check(
                a2.rect().ok_or("A second rect")?.1 == ar.1 + ar.3 + margin,
                "second A card follows only A first",
            )?;
            Notification::show(&*b, "", "Legacy B", "", Urgency::Critical);
            let legacy = b.child(b.children() - 1).ok_or("legacy layer")?;
            check(
                legacy.y() == b1.rect().unwrap().1 + b1.rect().unwrap().3 + margin,
                "legacy/new cards share window stack",
            )?;
            for button in [2, 3] {
                focus::click_button_at(&*b, legacy.x() + 10, legacy.y() + 10, button);
            }
            check(
                !legacy.was_deleted() && legacy.parent().is_some(),
                "auxiliary buttons never dismiss legacy cards",
            )?;
            a.snapshot(shots.join("notice-stack-a.png"))
                .map_err(|e| e.to_string())?;
            b.snapshot(shots.join("notice-stack-b.png"))
                .map_err(|e| e.to_string())?;
            a1.close();
            check(
                a2.rect().unwrap().1 == margin,
                "closing a card removes its stack gap",
            )?;
            check(
                b1.rect().unwrap().1 == margin,
                "A close does not move B stack",
            )?;
            let count = Notification::count();
            fltk::window::DoubleWindow::delete((*a).clone());
            check(
                !a2.is_open() && Notification::count() + 1 == count && b1.is_open(),
                "deleting A prunes only its notices",
            )?;
            b1.close();
            check(
                legacy.y() == margin,
                "legacy card moves up after persistent card closes",
            )?;
            fltk::window::DoubleWindow::delete((*b).clone());
            Ok(())
        },
    );
    run(
        "notices: live labels, action wrapping, resize and RTL use actual geometry",
        &|shots| {
            struct Restore(theme::Theme);
            impl Drop for Restore {
                fn drop(&mut self) {
                    i18n::set_locale("en");
                    theme::set(self.0.clone());
                }
            }
            let _restore = Restore(theme::current());
            let mut win = AppWindow::new("Live notice", 520, 420);
            win.end();
            let summary = State::new("A persistent notice".to_owned());
            let body = State::new(String::new());
            let label = State::new("Open".to_owned());
            let events = Rc::new(RefCell::new(Vec::new()));
            let out = events.clone();
            let notice = Notice::new("", "")
                .bind_summary(&summary)
                .bind_body(&body)
                .action(NoticeAction::new("open", "").bind_text(&label))
                .action(NoticeAction::new("second", "Show more information"))
                .on_action(move |_, id| out.borrow_mut().push(id.to_owned()))
                .show(&*win)
                .ok_or("notice")?;
            let old = notice.rect().unwrap();
            let second = Notice::new("Next", "").show(&*win).ok_or("next")?;
            body.set(
                "A long operation can be retried without changing the selected conversation."
                    .into(),
            );
            check(
                notice.rect().unwrap().3 > old.3 && second.rect().unwrap().1 > old.1 + old.3,
                "body updates relayout following cards",
            )?;
            label.set("Open the containing folder and inspect the result".into());
            let first = notice.action_rect("open").unwrap();
            let next = notice.action_rect("second").unwrap();
            check(next.1 >= first.1 + first.3, "long labels wrap action rows")?;
            win.snapshot(shots.join("notice-live-ltr.png"))
                .map_err(|e| e.to_string())?;
            let ltrclose = notice.close_rect().unwrap();
            i18n::set_locale("ar");
            win.snapshot(shots.join("notice-live-rtl.png"))
                .map_err(|e| e.to_string())?;
            let rtl = notice.rect().unwrap();
            let rtlclose = notice.close_rect().unwrap();
            check(
                rtl.0 == theme::with(|t| t.gaps_out.max(8))
                    && rtlclose.0 < rtl.0 + rtl.2 / 2
                    && ltrclose.0 > old.0 + old.2 / 2,
                "stack and close hit mirror live",
            )?;
            let p = centre(notice.action_rect("open").unwrap());
            focus::click_at(&*win, p.0, p.1);
            check(
                events.borrow().as_slice() == ["open"],
                "RTL click uses the same exposed action rect",
            )?;
            let origin = (win.x(), win.y());
            win.resize(origin.0, origin.1, 280, 420);
            App::pump();
            win.snapshot(shots.join("notice-narrow-rtl.png"))
                .map_err(|e| e.to_string())?;
            let r = notice.rect().unwrap();
            check(
                r.0 >= 0 && r.0 + r.2 <= win.w(),
                "notice width tracks containing window resize",
            )?;
            let a = notice.action_rect("open").unwrap();
            check(
                a.0 >= r.0 && a.0 + a.2 <= r.0 + r.2,
                "long action clipped to available width",
            )?;
            let old_height = r.3;
            let mut scaled = theme::current();
            scaled.font.base += 3;
            scaled.font.body += 3;
            scaled.font.title += 3;
            scaled.spacing.control_height += 4;
            theme::set(scaled);
            App::pump();
            win.snapshot(shots.join("notice-scaled-rtl.png"))
                .map_err(|e| e.to_string())?;
            check(
                notice.rect().unwrap().3 > old_height
                    && notice.action_rect("open").unwrap().3 > a.3,
                "font/theme generation invalidates cached metrics and control geometry",
            )?;
            fltk::window::DoubleWindow::delete((*win).clone());
            Ok(())
        },
    );
    run(
        "notices: persistence, callback reentry and captured resources end with the native widget",
        &|shots| {
            let mut win = AppWindow::new("Lifetime", 500, 350);
            win.end();
            let token = Rc::new(());
            let weak = Rc::downgrade(&token);
            let fired = Rc::new(Cell::new(0));
            let f = fired.clone();
            let closed = Rc::new(Cell::new(0));
            let c = closed.clone();
            struct RestoreClock;
            impl Drop for RestoreClock {
                fn drop(&mut self) {
                    fltk_omarchy::anim::clock::resume();
                }
            }
            let _clock = RestoreClock;
            let notice = Notice::new("Persistent", "")
                .urgency(Urgency::Low)
                .action(NoticeAction::new("again", "Again"))
                .on_action(move |n, id| {
                    let _ = &token;
                    f.set(f.get() + 1);
                    assert!(
                        !n.activate(id),
                        "a running callback is not entered recursively"
                    );
                })
                .on_dismiss(move |_| c.set(c.get() + 1))
                .show(&*win)
                .ok_or("notice")?;
            fltk_omarchy::anim::clock::freeze();
            fltk_omarchy::anim::clock::advance(Duration::from_secs(60));
            App::pump();
            check(
                notice.is_open(),
                "advancing the animation clock never expires a persistent notice",
            )?;
            check(
                notice.activate("again") && fired.get() == 1,
                "reentrant callback activation never borrows or recurses",
            )?;
            win.snapshot(shots.join("notice-persistent.png"))
                .map_err(|e| e.to_string())?;
            fltk_omarchy::anim::clock::resume();
            notice.close();
            check(
                weak.upgrade().is_none() && closed.get() == 1,
                "explicit close immediately releases captured action resources",
            )?;
            let token = Rc::new(());
            let weak = Rc::downgrade(&token);
            let f = fired.clone();
            let c = closed.clone();
            let survivor = Notice::new("Window owned", "")
                .action(NoticeAction::new("do", "Do"))
                .on_action(move |_, _| {
                    let _ = &token;
                    f.set(f.get() + 1);
                })
                .on_dismiss(move |_| c.set(c.get() + 1))
                .show(&*win)
                .ok_or("survivor")?;
            fltk::window::DoubleWindow::delete((*win).clone());
            App::pump();
            check(
                weak.upgrade().is_none() && !survivor.is_open() && !survivor.activate("do"),
                "deleted window frees callback captures even while handle survives",
            )?;
            check(
                closed.get() == 1 && fired.get() == 1,
                "window deletion never runs app callbacks",
            )?;
            Ok(())
        },
    );
    #[cfg(feature = "a11y")]
    run(
        "notices: accessible action IDs, focus versus activation and close name",
        &|shots| {
            use accesskit::{Action, Role};
            use fltk_omarchy::a11y::testing;
            let mut win = AppWindow::new("Accessible notices", 500, 320);
            win.end();
            let label = State::new("Retry".to_owned());
            let fired = Rc::new(Cell::new(0));
            let out = fired.clone();
            let notice = Notice::new("Permission needed", "Choose a destination")
                .action(NoticeAction::new("retry", "").bind_text(&label))
                .on_action(move |_, id| {
                    assert_eq!(id, "retry");
                    out.set(out.get() + 1);
                })
                .show(&*win)
                .ok_or("notice")?;
            win.snapshot(shots.join("notice-accessible.png"))
                .map_err(|e| e.to_string())?;
            let rec = testing::record(&*win);
            rec.sync();
            let button = rec
                .find(Role::Button, "Retry")
                .ok_or("retry accessible node")?;
            rec.act(button, Action::Focus, None);
            check(fired.get() == 0, "AT Focus never activates")?;
            label.set("Try again".into());
            rec.sync();
            check(
                rec.find(Role::Button, "Try again") == Some(button),
                "live label preserves accessible identity",
            )?;
            rec.act(button, Action::Click, None);
            check(
                fired.get() == 1 && notice.is_open(),
                "AT Click dispatches stable action without dismissal",
            )?;
            let alert = rec.find(Role::Alert, "Permission needed").ok_or("alert")?;
            let node = rec.node(alert).ok_or("alert node")?;
            let close = node
                .children()
                .iter()
                .copied()
                .find(|id| {
                    rec.node(*id)
                        .is_some_and(|n| n.role() == Role::Button && n.label() == Some("Close"))
                })
                .ok_or("named close action")?;
            rec.act(close, Action::Click, None);
            check(!notice.is_open(), "AT named dismiss action closes")?;
            fltk::window::DoubleWindow::delete((*win).clone());
            Ok(())
        },
    );
}
