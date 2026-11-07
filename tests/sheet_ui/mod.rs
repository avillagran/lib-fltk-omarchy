//! Real widget ownership, lazy retention, modal input and motion reversal.
use super::*;
use fltk_omarchy::overlay::{Sheet, SheetDismiss};

type Check<'a> = &'a dyn Fn(&Path) -> Result<(), String>;
struct Clock;
impl Clock {
    fn frozen(motion: bool) -> Self {
        fltk_omarchy::anim::clock::freeze();
        fltk_omarchy::anim::set_enabled(motion);
        Self
    }
}
impl Drop for Clock {
    fn drop(&mut self) {
        fltk_omarchy::anim::clock::resume();
        fltk_omarchy::anim::set_enabled(false);
        i18n::set_locale("en");
    }
}
struct CountDrop(Rc<Cell<usize>>);
impl Drop for CountDrop {
    fn drop(&mut self) {
        self.0.set(self.0.get() + 1);
    }
}
fn advance(ms: u64) {
    fltk_omarchy::anim::clock::advance(Duration::from_millis(ms));
    App::pump();
    App::pump();
}
fn centre(r: (i32, i32, i32, i32)) -> (i32, i32) {
    (r.0 + r.2 / 2, r.1 + r.3 / 2)
}
fn ui() -> (AppWindow, Button) {
    let mut w = AppWindow::new("Sheet fixture", 620, 420);
    let mut body = w.body();
    let anchor = Button::new("Details");
    body.fit(&*anchor);
    w.end();
    (w, anchor)
}
fn picture(win: &mut AppWindow, shots: &Path, name: &str) -> Result<Rgb, String> {
    let path = shots.join(format!("{name}.png"));
    win.snapshot(&path).map_err(|e| e.to_string())?;
    png_rgb(&path)
}
pub fn checks(run: &mut dyn FnMut(&str, Check)) {
    run(
        "sheet: lazy build, warm reuse, exact 30-second release and app-owned draft reconstruction",
        &|shots| {
            let _clock = Clock::frozen(false);
            let (mut win, anchor) = ui();
            win.snapshot(shots.join("sheet-owner.png"))
                .map_err(|e| e.to_string())?;
            let base = win.children();
            let draft = State::new("Original draft".to_owned());
            let built = Rc::new(Cell::new(0));
            let dropped = Rc::new(Cell::new(0));
            let fields = Rc::new(RefCell::new(Vec::<TextField>::new()));
            let (b, d, f, s) = (
                built.clone(),
                dropped.clone(),
                fields.clone(),
                draft.clone(),
            );
            let sheet = Sheet::new(&*anchor, "Details", 440, 260, move |col, _sheet| {
                b.set(b.get() + 1);
                let field = TextField::new().bind(&s);
                col.fit(&*field);
                f.borrow_mut().push(field);
                let guard = CountDrop(d.clone());
                let mut frame = fltk::frame::Frame::default();
                frame.draw(move |_| {
                    let _ = &guard;
                });
            });
            check(
                !sheet.is_built() && built.get() == 0 && win.children() == base,
                "eager content allocation",
            )?;
            let _ = anchor.clone().take_focus();
            check(sheet.open(), "open")?;
            check(
                sheet.is_open() && sheet.is_built() && built.get() == 1,
                "first opening",
            )?;
            win.snapshot(shots.join("sheet-open.png"))
                .map_err(|e| e.to_string())?;
            draft.set("State survives widgets".into());
            check(
                sheet.close() && focus::is_focused(&*anchor),
                "close restores opener",
            )?;
            advance(29_999);
            check(
                sheet.is_built() && dropped.get() == 0,
                "released before 30 seconds",
            )?;
            check(
                sheet.open()
                    && built.get() == 1
                    && fields.borrow()[0].value() == "State survives widgets",
                "warm reopen lost widget state",
            )?;
            advance(90_000);
            check(
                sheet.is_open() && dropped.get() == 0,
                "open content expired",
            )?;
            sheet.close();
            advance(29_999);
            check(sheet.is_built(), "retention starts at latest close")?;
            advance(1);
            pump_until(|| fields.borrow()[0].was_deleted());
            check(
                !sheet.is_built() && dropped.get() == 1,
                "expired controls/resources not destroyed",
            )?;
            check(
                sheet.open()
                    && built.get() == 2
                    && fields.borrow()[1].value() == "State survives widgets",
                "cold rebuild did not use app State",
            )?;
            sheet.close();
            check(sheet.release(), "explicit hidden release")?;
            App::pump();
            App::pump();
            check(
                dropped.get() == 2 && win.children() == base,
                "content leaked after release",
            )?;
            fltk::window::DoubleWindow::delete((*win).clone());
            Ok(())
        },
    );
    run(
        "sheet: modal focus, stacked popup, primary release ownership and owner destruction",
        &|shots| {
            let _clock = Clock::frozen(false);
            let (mut win, anchor) = ui();
            win.snapshot(shots.join("sheet-input-owner.png"))
                .map_err(|e| e.to_string())?;
            let field = Rc::new(RefCell::new(None::<TextField>));
            let f = field.clone();
            let actions = Rc::new(Cell::new(0));
            let a = actions.clone();
            let dropped = Rc::new(Cell::new(0));
            let d = dropped.clone();
            let reasons = Rc::new(RefCell::new(Vec::new()));
            let r = reasons.clone();
            let sheet = Sheet::new(&*win, "Modal details", 400, 250, move |col, _sheet| {
                let edit = TextField::new();
                col.fit(&*edit);
                *f.borrow_mut() = Some(edit);
                let a = a.clone();
                col.fit(&*Button::new("Run").on_click(move || a.set(a.get() + 1)));
                let guard = CountDrop(d.clone());
                let mut frame = fltk::frame::Frame::default();
                frame.draw(move |_| {
                    let _ = &guard;
                });
            })
            .on_dismiss(move |why| r.borrow_mut().push(why));
            let _ = anchor.clone().take_focus();
            sheet.open();
            check(
                focus::current().is_some_and(|w| w.inside(&**field.borrow().as_ref().unwrap())),
                "first content focus",
            )?;
            for _ in 0..6 {
                press("Tab");
                check(!focus::is_focused(&*anchor), "Tab escaped modal")?;
            }
            let (x, y) = centre(sheet.close_rect().ok_or("close rect")?);
            for b in [2, 3] {
                focus::click_button_at(&*win, x, y, b);
                focus::click_button_at(&*win, 2, 2, b);
            }
            check(
                sheet.is_open() && actions.get() == 0 && reasons.borrow().is_empty(),
                "auxiliary button activated a primary action",
            )?;
            let popup = PopupMenu::open(
                &**field.borrow().as_ref().ok_or("field")?,
                vec![PopupItem::new("", "Nested")],
                |_| {},
            );
            press("Escape");
            check(
                !popup.is_open() && sheet.is_open(),
                "Escape skipped top popup",
            )?;
            focus::mouse_drag(&*win, (2, 2), (x, y), true);
            check(
                sheet.is_open(),
                "outside press then inside release dismissed",
            )?;
            focus::click_at(&*win, 2, 2);
            check(
                !sheet.is_open()
                    && *reasons.borrow() == [SheetDismiss::ClickOutside]
                    && focus::is_focused(&*anchor),
                "outside dismissal/focus",
            )?;
            sheet.open();
            press("Escape");
            check(
                !sheet.is_open() && reasons.borrow().last() == Some(&SheetDismiss::Escape),
                "Escape dismissal",
            )?;
            sheet.open();
            focus::click_at(&*win, x, y);
            check(
                !sheet.is_open() && reasons.borrow().last() == Some(&SheetDismiss::CloseButton),
                "close button",
            )?;
            sheet.open();
            fltk::window::DoubleWindow::delete((*win).clone());
            App::pump();
            check(
                !sheet.is_open() && !sheet.is_built() && dropped.get() == 1,
                "owner deletion did not free content with controller alive",
            )?;
            advance(40_000);
            check(!sheet.open(), "deleted owner reopened")
        },
    );
    run(
        "sheet: retained fade reversals, reduced motion, resize and live RTL title",
        &|shots| {
            let _clock = Clock::frozen(true);
            let (mut win, anchor) = ui();
            win.begin();
            let mut stripe = fltk::frame::Frame::new(0, 360, 620, 60, None);
            stripe.draw(|f| {
                theme::with(|t| {
                    fltk_omarchy::paint::fill(f.x(), f.y(), f.w(), f.h(), 0, t.palette.accent)
                })
            });
            win.end();
            win.snapshot(shots.join("sheet-motion-owner.png"))
                .map_err(|e| e.to_string())?;
            let title = State::new("Inspect item".to_owned());
            let sheet = Sheet::new(&*win, "", 420, 280, |col, _sheet| {
                col.fit(&*Button::new("Content"))
            })
            .bind_title(&title);
            let _ = anchor.clone().take_focus();
            sheet.open();
            advance(65);
            let before = picture(&mut win, shots, "sheet-before-close")?;
            sheet.close();
            let closing = picture(&mut win, shots, "sheet-close-phase")?;
            check(
                before.at(310, 400) == closing.at(310, 400),
                "closing transition jumped instead of sampling phase",
            )?;
            advance(15);
            let before = picture(&mut win, shots, "sheet-before-reopen")?;
            sheet.open();
            let reversed = picture(&mut win, shots, "sheet-reopen-phase")?;
            check(
                before.at(310, 400) == reversed.at(310, 400),
                "reversal jumped instead of sampling phase",
            )?;
            advance(300);
            check(
                sheet.is_open() && sheet.is_built(),
                "stale close hid reopened sheet",
            )?;
            title.set("Live title".into());
            i18n::set_locale("ar");
            win.resize(0, 0, 330, 210);
            win.snapshot(shots.join("sheet-small-rtl.png"))
                .map_err(|e| e.to_string())?;
            let (x, y, w, h) = sheet.rect().ok_or("rect")?;
            let close = sheet.close_rect().ok_or("close rect")?;
            check(
                x >= 0 && y >= 0 && x + w <= win.w() && y + h <= win.h() && close.0 < x + w / 2,
                "resize/RTL close placement",
            )?;
            sheet.close();
            fltk_omarchy::anim::set_enabled(false);
            App::pump();
            App::pump();
            check(
                !sheet.is_open() && sheet.release(),
                "reduced motion did not finish hiding",
            )?;
            fltk::window::DoubleWindow::delete((*win).clone());
            Ok(())
        },
    );
    run(
        "sheet: weak content actions, reentrant reopen, keep-content and final-owner drop",
        &|shots| {
            let _clock = Clock::frozen(false);
            let (mut win, anchor) = ui();
            win.snapshot(shots.join("sheet-weak-owner.png"))
                .map_err(|e| e.to_string())?;
            let action = Rc::new(RefCell::new(None::<Button>));
            let a = action.clone();
            let drops = Rc::new(Cell::new(0));
            let d = drops.clone();
            let sheet = Sheet::new(&*anchor, "Stateful form", 400, 260, move |col, owner| {
                let button = Button::new("Done").on_click(move || {
                    if let Some(sheet) = owner.upgrade() {
                        sheet.close();
                    }
                });
                col.fit(&*button);
                *a.borrow_mut() = Some(button);
                let guard = CountDrop(d.clone());
                let mut frame = fltk::frame::Frame::default();
                frame.draw(move |_| {
                    let _ = &guard;
                });
            })
            .keep_content();
            let weak = sheet.downgrade();
            let reopen = weak.clone();
            let sheet = sheet.on_dismiss(move |reason| {
                if reason == SheetDismiss::Escape {
                    reopen.upgrade().unwrap().open();
                }
            });
            sheet.open();
            press("Escape");
            advance(40_000);
            check(
                sheet.is_open() && drops.get() == 0,
                "dismiss callback reopen lost content",
            )?;
            action.borrow().as_ref().ok_or("action")?.click();
            advance(80_000);
            check(
                !sheet.is_open() && sheet.is_built() && drops.get() == 0,
                "keep-content expired or weak action failed",
            )?;
            sheet.open();
            let reasons = Rc::new(RefCell::new(Vec::new()));
            let r = reasons.clone();
            let sheet = sheet.on_dismiss(move |reason| r.borrow_mut().push(reason));
            win.show();
            win.hide();
            check(
                !sheet.is_open() && *reasons.borrow() == [SheetDismiss::OwnerHidden],
                "owner hiding left an interactive modal",
            )?;
            drop(sheet);
            App::pump();
            App::pump();
            check(
                weak.upgrade().is_none()
                    && drops.get() == 1
                    && action.borrow().as_ref().unwrap().was_deleted(),
                "weak content callback retained controller/resources",
            )?;
            fltk::window::DoubleWindow::delete((*win).clone());
            Ok(())
        },
    );
    run(
        "sheet: child hide callback may release retained controls without a borrow conflict",
        &|shots| {
            let _clock = Clock::frozen(false);
            let (mut win, anchor) = ui();
            win.snapshot(shots.join("sheet-hide-owner.png"))
                .map_err(|e| e.to_string())?;
            let released = Rc::new(Cell::new(false));
            let out = released.clone();
            let drops = Rc::new(Cell::new(0));
            let d = drops.clone();
            let sheet = Sheet::new(&*anchor, "Disposable", 380, 240, move |_, owner| {
                let out = out.clone();
                let mut frame = fltk::frame::Frame::default();
                frame.handle(move |_, event| {
                    if event == fltk::enums::Event::Hide {
                        if let Some(sheet) = owner.upgrade() {
                            out.set(sheet.release());
                        }
                    }
                    false
                });
                let guard = CountDrop(d.clone());
                frame.draw(move |_| {
                    let _ = &guard;
                });
            });
            sheet.open();
            win.begin();
            let hides = Rc::new(RefCell::new(Vec::new()));
            for id in [1, 2] {
                let mut sibling = fltk::frame::Frame::new(0, 390, 10, 10, None);
                let out = hides.clone();
                sibling.handle(move |_, event| {
                    if event == fltk::enums::Event::Hide {
                        out.borrow_mut().push(id);
                    }
                    false
                });
            }
            win.end();
            win.show();
            win.hide();
            App::pump();
            App::pump();
            check(
                *hides.borrow() == [1, 2],
                "release from Hide changed the owner's sibling notification walk",
            )?;
            check(
                released.get() && !sheet.is_built() && drops.get() == 1,
                "hide callback failed to release native resources",
            )?;
            fltk::window::DoubleWindow::delete((*win).clone());
            Ok(())
        },
    );
    #[cfg(feature = "a11y")]
    run(
        "sheet a11y: named live dialog, translated close and retained hiding",
        &|shots| {
            let _clock = Clock::frozen(false);
            let (mut win, anchor) = ui();
            win.snapshot(shots.join("sheet-a11y-owner.png"))
                .map_err(|e| e.to_string())?;
            let title = State::new("Properties".to_owned());
            let sheet = Sheet::new(&*anchor, "", 400, 240, |col, _sheet| {
                col.fit(&*TextField::new())
            })
            .bind_title(&title);
            sheet.open();
            let tree = fltk_omarchy::a11y::snapshot(&*win).ok_or("a11y tree")?;
            check(
                tree.contains("Dialog") && tree.contains("Properties") && tree.contains("Close"),
                &tree,
            )?;
            title.set("Updated properties".into());
            i18n::set_locale("es");
            let tree = fltk_omarchy::a11y::snapshot(&*win).ok_or("a11y tree")?;
            check(
                tree.contains("Updated properties") && tree.contains("Cerrar"),
                &tree,
            )?;
            sheet.close();
            let tree = fltk_omarchy::a11y::snapshot(&*win).ok_or("a11y tree")?;
            check(
                !tree.contains("Updated properties"),
                "cached dialog exposed to AT",
            )?;
            sheet.release();
            fltk::window::DoubleWindow::delete((*win).clone());
            Ok(())
        },
    );
}
