//! UI checks of command shortcut tables (`AppWindow::shortcuts`), key
//! repeat, `keys::Suspend` and `keys::record`, through the same key path
//! as real keys (`focus::press`).
use super::{check, pump_until};
use fltk::prelude::*;
use fltk_omarchy::focus::{press, press_repeat};
use fltk_omarchy::layout::FlexExt;
use fltk_omarchy::widgets::{Button, TextField};
use fltk_omarchy::{keys, AppWindow, State};
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

type Check<'a> = &'a dyn Fn(&Path) -> Result<(), String>;

struct Fixture {
    win: AppWindow,
    ran: Rc<RefCell<Vec<String>>>,
    field: TextField,
    button: Button,
}

fn fixture() -> Fixture {
    let mut win = AppWindow::new("Keys", 420, 240);
    let mut body = win.body();
    let field = TextField::new().placeholder("Name");
    body.fit(&*field);
    let button = Button::new("Run").bordered();
    body.fit(&*button);
    win.end();
    win.show();
    let b = body.clone();
    pump_until(|| b.w() > 0);
    let ran = Rc::new(RefCell::new(Vec::new()));
    let r = ran.clone();
    win.shortcuts().on_command(move |c| r.borrow_mut().push(c.to_string()));
    let mut bt = (*button).clone();
    let _ = bt.take_focus();
    Fixture { win, ran, field, button }
}

fn close(f: Fixture) {
    fltk::window::DoubleWindow::delete((*f.win).clone());
}

fn take(f: &Fixture) -> Vec<String> {
    std::mem::take(&mut *f.ran.borrow_mut())
}

pub fn checks(run: &mut dyn FnMut(&str, Check)) {
    run("shortcuts: table runs commands, conditions, rebind, repeat", &|_| {
        let f = fixture();
        let t = f.win.shortcuts();
        let gate = State::new(false);
        let g = gate.clone();
        t.bind("gated", "Mod+k").unwrap().when(move || g.get());
        t.bind("fallback", "Mod+k").unwrap();
        t.bind("next", "Alt+Down").unwrap().repeat(true);
        t.bind("once", "Alt+Up").unwrap();
        press("Mod+k");
        check(take(&f) == ["fallback"], "a false condition passes to the next binding")?;
        gate.set(true);
        press("Mod+k");
        check(take(&f) == ["gated"], "a true condition wins")?;
        press("Alt+Down");
        press_repeat("Alt+Down");
        check(take(&f) == ["next", "next"], "repeat(true) fires on repeats")?;
        press("Alt+Up");
        press_repeat("Alt+Up");
        check(take(&f) == ["once"], "a repeat of a non-repeating binding is ignored")?;
        press("Alt+Up");
        check(take(&f) == ["once"], "a new press fires again")?;
        t.rebind("next", &["Mod+j"]).unwrap();
        press("Alt+Down");
        press("Mod+j");
        check(take(&f) == ["next"], "rebind moves the command to the new keys")?;
        check(t.conflicts("Mod+k", "gated") == ["fallback"], "conflicts")?;
        close(f);
        Ok(())
    });

    run("shortcuts: the focused control keeps its keys; Suspend; reentrant callbacks", &|_| {
        let mut f = fixture();
        let t = f.win.shortcuts();
        t.bind("enter", "Enter").unwrap();
        let submitted = Rc::new(RefCell::new(0));
        let s2 = submitted.clone();
        f.field = f.field.clone().on_submit(move |_| *s2.borrow_mut() += 1);
        let mut fl = (*f.field).clone();
        let _ = fl.take_focus();
        press("Enter");
        check(*submitted.borrow() == 1 && take(&f).is_empty(), "Enter in a text field submits it, not the shortcut")?;
        let _ = (*f.button).clone().take_focus();
        t.bind("cmd", "Mod+e").unwrap();
        let legacy = Rc::new(RefCell::new(0));
        let l2 = legacy.clone();
        let mut win = f.win.clone();
        let win_inner = f.win.clone();
        win.shortcut("Mod+l", move || {
            *l2.borrow_mut() += 1;
            // Registering a shortcut from inside one must not panic.
            let mut w = win_inner.clone();
            w.shortcut("Mod+m", || {});
        });
        {
            let _quiet = keys::Suspend::new();
            press("Mod+e");
            press("Mod+l");
            check(take(&f).is_empty() && *legacy.borrow() == 0, "nothing fires while suspended")?;
        }
        press("Mod+e");
        press("Mod+l");
        press("Mod+l");
        check(take(&f) == ["cmd"] && *legacy.borrow() == 2, &format!("back after the guard; legacy ran {}", legacy.borrow()))?;
        close(f);
        Ok(())
    });

    run("shortcuts: record the next key as a spec; Escape cancels", &|_| {
        let f = fixture();
        let t = f.win.shortcuts();
        t.bind("taken", "Mod+Shift+2").unwrap();
        let got: Rc<RefCell<Vec<Option<String>>>> = Rc::default();
        let g = got.clone();
        keys::record(move |s| g.borrow_mut().push(s));
        check(keys::recording() && keys::suspended(), "recording suspends shortcuts")?;
        press("Mod+Shift+2");
        check(*got.borrow() == [Some("Mod+Shift+2".to_string())], &format!("recorded {:?}", got.borrow()))?;
        check(take(&f).is_empty() && !keys::recording(), "the recorded key ran nothing; recording ended")?;
        let g = got.clone();
        keys::record(move |s| g.borrow_mut().push(s));
        press("Escape");
        check(got.borrow().last() == Some(&None), "Escape cancels")?;
        let g = got.clone();
        keys::record(move |s| g.borrow_mut().push(s));
        press("Alt+F4");
        check(got.borrow().last() == Some(&Some("Alt+F4".to_string())), "Alt+F4")?;
        press("Mod+Shift+2");
        check(take(&f) == ["taken"], "shortcuts are back after recording")?;
        close(f);
        Ok(())
    });
}
