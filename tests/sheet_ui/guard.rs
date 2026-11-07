//! Busy/ACK ownership is decided before any user dismissal side effect.
use super::*;
use fltk_omarchy::testkit;

pub fn checks(run: &mut dyn FnMut(&str, Check)) {
    run(
        "sheet guard: rebinding during State notification ignores an already queued old callback",
        &|shots| {
            let _clock = Clock::frozen(false);
            testkit::arm();
            let (mut win, _) = ui();
            win.snapshot(shots.join("sheet-guard-rebind-owner.png"))
                .map_err(|e| e.to_string())?;
            let first = State::new(true);
            let second = State::new(true);
            let slot = Rc::new(RefCell::new(None::<fltk_omarchy::overlay::WeakSheet>));
            let weak = slot.clone();
            let next = second.clone();
            let _application = first.subscribe(move |_| {
                let owner = weak.borrow().as_ref().unwrap().upgrade().unwrap();
                assert!(owner.rect().is_some());
                owner.bind_dismissible(&next);
            });
            let sheet = Sheet::new(&*win, "Reentrant permission", 400, 250, |col, _| {
                col.fit(&*TextField::new())
            })
            .bind_dismissible(&first);
            *slot.borrow_mut() = Some(sheet.downgrade());
            sheet.open();
            first.set(false);
            check(
                sheet.is_dismissible()
                    && testkit::find(&*win, "Close").is_some_and(|e| !e.disabled),
                "obsolete source callback overwrote a newer policy",
            )?;
            check(
                first.listener_count() == 1 && second.listener_count() == 1,
                "rebind left stale sheet subscription",
            )?;
            fltk::window::DoubleWindow::delete((*win).clone());
            Ok(())
        },
    );
    run("sheet guard: OwnerHidden cannot reopen from callbacks or trap input before the owner returns", &|shots| {
        let _clock=Clock::frozen(false);let(mut win,_)=ui();win.snapshot(shots.join("sheet-guard-owner-hidden.png")).map_err(|e|e.to_string())?;
        let allowed=State::new(false);let owner=Rc::new(RefCell::new(None::<fltk_omarchy::overlay::WeakSheet>));let weak=owner.clone();let attempt=Rc::new(Cell::new(None));let a=attempt.clone();
        let sheet=Sheet::new(&*win,"Owner teardown",400,250,|col,_|col.fit(&*TextField::new())).bind_dismissible(&allowed).on_dismiss(move|why|{if why==SheetDismiss::OwnerHidden {a.set(Some(weak.borrow().as_ref().unwrap().upgrade().unwrap().open()));}});
        *owner.borrow_mut()=Some(sheet.downgrade());win.show();sheet.open();win.hide();
        check(attempt.get()==Some(false) && !sheet.is_open() && !sheet.open(),&format!("hidden owner: attempted={:?}, open={}, shown={}, visible={}",attempt.get(),sheet.is_open(),win.shown(),win.visible()))?;
        win.show();check(sheet.open(),"owner return did not permit opening")?;
        fltk::window::DoubleWindow::delete((*win).clone());App::pump();allowed.set(true);
        check(!sheet.is_open() && !sheet.is_built() && !sheet.open(),"deleted owner kept guarded input/content")?;Ok(())
    });
    run("sheet guard: busy State rejects native-path user actions without focus/retention changes; owner close bypasses", &|shots| {
        let _clock = Clock::frozen(false); testkit::arm();
        let (mut win, anchor) = ui(); win.snapshot(shots.join("sheet-guard-owner.png")).map_err(|e|e.to_string())?;
        let busy = State::new(true);
        let field = Rc::new(RefCell::new(None::<TextField>)); let f = field.clone();
        let dropped = Rc::new(Cell::new(0)); let d = dropped.clone();
        let reasons = Rc::new(RefCell::new(Vec::new())); let r = reasons.clone();
        let sheet = Sheet::new(&*win, "Guarded form", 420, 260, move |col,_| {
            let edit = TextField::new(); col.fit(&*edit); *f.borrow_mut() = Some(edit);
            let guard = CountDrop(d.clone()); let mut frame = fltk::frame::Frame::default();
            frame.draw(move |_| { let _ = &guard; });
        }).bind_dismissible(&busy.map(|busy| !busy)).on_dismiss(move |why|r.borrow_mut().push(why));
        check(!sheet.is_built() && !sheet.is_dismissible(), "binding allocated eager widgets or lost initial permission")?;
        let _ = anchor.clone().take_focus(); sheet.open();
        let edit = field.borrow().clone().ok_or("field")?; let _ = edit.clone().take_focus();
        check(testkit::find(&*win,"Close").is_some_and(|e|e.disabled), "close action remains enabled while busy")?;
        let (x,y) = centre(sheet.close_rect().ok_or("close rect")?);
        // Holding the application model borrow is safe: denied requests must
        // never enter the post-dismissal callback, which owns that model.
        let model = reasons.borrow_mut();
        let focused = focus::current().map(|w|w.as_widget_ptr());
        press("Escape");
        check(focused == focus::current().map(|w|w.as_widget_ptr()), &format!("Escape moved focus: {:?}",focus::current().map(|w|(w.x(),w.y(),w.w(),w.h(),w.active_r()))))?;
        focus::click_at(&*win,2,2);
        check(focused == focus::current().map(|w|w.as_widget_ptr()), &format!("outside moved focus: {:?}",focus::current().map(|w|(w.x(),w.y(),w.w(),w.h(),w.active_r()))))?;
        focus::click_at(&*win,x,y);
        drop(model);
        check(sheet.is_open() && reasons.borrow().is_empty() && focused == focus::current().map(|w|w.as_widget_ptr()), &format!("rejected input: open={} callbacks={:?} focus={:?}",sheet.is_open(),reasons.borrow(),focus::current().map(|w|(w.x(),w.y(),w.w(),w.h(),w.active_r()))))?;
        advance(90_000); check(sheet.is_built() && dropped.get()==0, "denial started retention")?;
        busy.set(false);
        check(sheet.is_dismissible() && testkit::find(&*win,"Close").is_some_and(|e|!e.disabled), "temporary mapped State stopped updating")?;
        press("Escape"); check(!sheet.is_open() && *reasons.borrow()==[SheetDismiss::Escape] && focus::is_focused(&*anchor), "allowed Escape did not dismiss normally")?;
        advance(29_999); check(sheet.is_built(), "retention began during rejected request")?;
        advance(1); pump_until(||edit.was_deleted()); check(dropped.get()==1, "accepted dismissal did not expire")?;
        busy.set(true); sheet.open(); check(sheet.close(), "owner close was incorrectly vetoed")?;
        check(reasons.borrow().last()==Some(&SheetDismiss::Closed), "owner close reason")?;
        fltk::window::DoubleWindow::delete((*win).clone()); Ok(())
    });
    run(
        "sheet guard: state changes between press/release cannot commit a blocked outside gesture",
        &|shots| {
            let _clock = Clock::frozen(false);
            let (mut win, _) = ui();
            win.snapshot(shots.join("sheet-guard-gesture-owner.png"))
                .map_err(|e| e.to_string())?;
            let allowed = State::new(false);
            let sheet = Sheet::new(&*win, "Guarded gestures", 400, 250, |col, _| {
                col.fit(&*TextField::new())
            })
            .bind_dismissible(&allowed);
            sheet.open();
            focus::mouse_drag(&*win, (2, 2), (2, 2), false);
            allowed.set(true);
            focus::mouse_up(&*win, 2, 2);
            check(
                sheet.is_open(),
                "outside press while blocked acquired a release after ACK",
            )?;
            focus::mouse_drag(&*win, (2, 2), (2, 2), false);
            allowed.set(false);
            focus::mouse_up(&*win, 2, 2);
            check(
                sheet.is_open(),
                "outside release ignored the current busy permission",
            )?;
            allowed.set(true);
            focus::click_at(&*win, 2, 2);
            check(!sheet.is_open(), "allowed outside gesture was swallowed")?;
            sheet.open();
            let (x, y) = centre(sheet.close_rect().ok_or("close")?);
            focus::mouse_drag(&*win, (x, y), (x, y), false);
            allowed.set(false);
            focus::mouse_up(&*win, x, y);
            check(
                sheet.is_open(),
                "close release ignored a newly blocked permission",
            )?;
            fltk::window::DoubleWindow::delete((*win).clone());
            Ok(())
        },
    );
    run("sheet guard: rejected Escape preserves reversal phase and release starts only at eventual hidden state", &|shots| {
        let _clock = Clock::frozen(true); let (mut win,_) = ui();
        win.snapshot(shots.join("sheet-guard-motion-owner.png")).map_err(|e|e.to_string())?;
        let allowed=State::new(false);let reasons=Rc::new(RefCell::new(Vec::new()));let r=reasons.clone();
        let sheet=Sheet::new(&*win,"Guarded motion",420,260,|col,_|col.fit(&*TextField::new())).bind_dismissible(&allowed).release_after(Duration::ZERO).on_dismiss(move|why|r.borrow_mut().push(why));
        sheet.open();press("Tab");advance(65);sheet.close();advance(15);sheet.open();press("Tab");
        let before=picture(&mut win,shots,"sheet-guard-reversal-before")?;let focused=focus::current().map(|w|w.as_widget_ptr());
        press("Escape");let after=picture(&mut win,shots,"sheet-guard-reversal-rejected")?;
        check(before.raw==after.raw && focused==focus::current().map(|w|w.as_widget_ptr()) && sheet.is_open(),"rejection changed a frozen reversal frame/focus")?;
        check(*reasons.borrow()==[SheetDismiss::Closed],"denial notified dismissal")?;
        advance(90_000);check(sheet.is_open() && sheet.is_built(),"stale close/rejected request released the reopened content")?;
        allowed.set(true);press("Escape");check(!sheet.is_open() && sheet.is_built(),"accepted close skipped its fade")?;
        advance(500);pump_until(||!sheet.is_built());check(!sheet.is_built(),"zero-retention accepted close failed to release")?;
        fltk::window::DoubleWindow::delete((*win).clone());Ok(())
    });
    run(
        "sheet guard: rebinding, weak lifetime and owner hide/deletion bypass busy input policy",
        &|shots| {
            let _clock = Clock::frozen(false);
            let (mut win, anchor) = ui();
            win.snapshot(shots.join("sheet-guard-weak-owner.png"))
                .map_err(|e| e.to_string())?;
            let first = State::new(true);
            let second = State::new(false);
            let action = Rc::new(RefCell::new(None::<Button>));
            let a = action.clone();
            let reasons = Rc::new(RefCell::new(Vec::new()));
            let r = reasons.clone();
            let sheet = Sheet::new(&*win, "Guarded lifetime", 400, 250, move |col, weak| {
                let button = Button::new("Owner finish").on_click(move || {
                    if let Some(sheet) = weak.upgrade() {
                        sheet.close();
                    }
                });
                col.fit(&*button);
                *a.borrow_mut() = Some(button);
            })
            .bind_dismissible(&first)
            .on_dismiss(move |why| r.borrow_mut().push(why));
            let sheet = sheet.bind_dismissible(&second);
            check(
                first.listener_count() == 0 && second.listener_count() == 1,
                "rebind retained stale subscription",
            )?;
            first.set(false);
            first.set(true);
            check(
                !sheet.is_dismissible(),
                "old binding overrode new permission",
            )?;
            let _ = anchor.clone().take_focus();
            sheet.open();
            action.borrow().as_ref().ok_or("action")?.click();
            check(
                !sheet.is_open() && reasons.borrow().last() == Some(&SheetDismiss::Closed),
                "weak owner completion could not close busy form",
            )?;
            sheet.open();
            win.show();
            win.hide();
            check(
                !sheet.is_open() && reasons.borrow().last() == Some(&SheetDismiss::OwnerHidden),
                "owner hide was vetoed",
            )?;
            win.show();
            check(sheet.open(), "shown owner cannot reopen")?;
            fltk::window::DoubleWindow::delete((*win).clone());
            App::pump();
            check(
                !sheet.is_open() && !sheet.is_built(),
                "owner deletion left busy modal mounted",
            )?;
            let weak = sheet.downgrade();
            drop(sheet);
            check(
                weak.upgrade().is_none() && second.listener_count() == 0,
                "permission subscription retained controller/captures",
            )?;
            Ok(())
        },
    );
}
