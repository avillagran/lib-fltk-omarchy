//! UI tests on the main thread (fltk is single-threaded), offscreen: no
//! window is shown. Each check renders with `snapshot` and reads pixels.
//! Uses a fake Omarchy state dir so results do not depend on the desktop.
use fltk::prelude::*;
use fltk_omarchy::layout::FlexExt;
use fltk_omarchy::list::{List, ListRow};
use fltk_omarchy::overlay::{Menu, MenuItem, Notification, Urgency};
use fltk_omarchy::popups::{OperationToast, PopupItem, PopupMenu, SearchableDropdown};
use fltk_omarchy::theme::Corners;
use fltk_omarchy::Mode;
use fltk_omarchy::task::{Outcome, Task};
use fltk_omarchy::widgets::*;
use fltk_omarchy::{theme, App, AppWindow, State};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

const COLORS_A: &str = "accent = \"#e68e0d\"\nbackground = \"#121212\"\nforeground = \"#bebebe\"\nred = \"#d35f5f\"\n";
const COLORS_B: &str = "accent = \"#33ccff\"\nbackground = \"#1a1b26\"\nforeground = \"#c0caf5\"\nred = \"#f7768e\"\n";

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("build/test-ui");
    let _ = std::fs::remove_dir_all(&root);
    let state = root.join("state");
    std::fs::create_dir_all(state.join("theme")).unwrap();
    std::fs::write(state.join("theme/colors.toml"), COLORS_A).unwrap();
    std::fs::write(state.join("theme.name"), "A").unwrap();
    std::env::set_var("OMARCHY_FLTK_STATE_DIR", &state);
    std::env::set_var("OMARCHY_FLTK_USER_SHELL", root.join("no-user-shell.toml"));
    // Hyprland values are read live; pin them by hiding the socket.
    std::env::remove_var("HYPRLAND_INSTANCE_SIGNATURE");

    let app = App::new("dev.omarchy.fltk.tests");
    let shots = root.join("shots");
    let mut failed = 0;
    let mut run = |name: &str, f: &dyn Fn(&Path) -> Result<(), String>| {
        let r = f(&shots);
        match &r {
            Ok(()) => println!("ok   {name}"),
            Err(e) => { failed += 1; println!("FAIL {name}: {e}") }
        }
    };
    run("theme loads omarchy colors and shell tokens", &|_| {
        let t = theme::current();
        check(t.palette.accent == theme::Rgb(0xe68e0d), "accent")?;
        check(t.spacing.control_height == 28 && t.font.body == 12, "tokens")
    });
    run("primary button is painted with the accent", &|shots| {
        let mut win = AppWindow::new("t", 300, 140);
        let mut body = win.body();
        let b = Button::new("Go").primary();
        body.fit(&*b);
        win.end();
        let p = shots.join("button.png");
        win.snapshot(&p).map_err(|e| e.to_string())?;
        let (x, y) = (b.x() + 3, b.y() + 3);
        let px = pixel(&p, x, y)?;
        check(px == [0xe6, 0x8e, 0x0d], &format!("pixel {px:?} at {x},{y}"))
    });
    run("text field two-way binding and placeholder", &|shots| {
        let name = State::new(String::new());
        let mut win = AppWindow::new("t", 300, 140);
        let mut body = win.body();
        let mut f = TextField::new().placeholder("Name").bind(&name);
        body.fit(&*f);
        win.end();
        win.snapshot(shots.join("field-empty.png")).map_err(|e| e.to_string())?;
        f.type_text("Omarchy");
        check(name.get() == "Omarchy", "typed text reaches the State")?;
        name.set("FLTK".into());
        check(f.value() == "FLTK", "State reaches the field")
    });
    run("toggle, switch, slider, button group follow their State", &|_| {
        let on = State::new(false);
        let t = Toggle::new("Night light", "", &on);
        t.toggle();
        check(on.get() && t.is_on(), "toggle")?;
        let v = State::new(0.2);
        let s = Slider::new(&v, 0.0, 1.0, 0.1);
        v.set(0.7);
        check((s.value() - 0.7).abs() < 1e-9, "slider")?;
        let sel = State::new(0usize);
        let g = ButtonGroup::new(&["A", "B", "C"], &sel);
        g.button(2).unwrap().click();
        check(sel.get() == 2 && g.selected() == 2, "group")
    });
    run("button click, bound text and enabled state", &|_| {
        let clicks = Rc::new(RefCell::new(0));
        let c = clicks.clone();
        let label = State::new("Save".to_string());
        let enabled = State::new(true);
        let b = Button::new("x").bind_text(&label).bind_enabled(&enabled).on_click(move || *c.borrow_mut() += 1);
        b.click();
        label.set("Saved".into());
        enabled.set(false);
        check(*clicks.borrow() == 1 && b.text() == "Saved" && !b.active(), "button state")
    });
    run("virtual list builds only visible rows of 1 000 000", &|shots| {
        let count = State::new(1_000_000usize);
        let mut win = AppWindow::new("t", 400, 400);
        let mut list = List::new(&count, |i| ListRow::new(&format!("Row {i}")));
        win.end();
        win.snapshot(shots.join("list.png")).map_err(|e| e.to_string())?;
        let first = list.rows_built();
        list.select(500_000);
        win.snapshot(shots.join("list-mid.png")).map_err(|e| e.to_string())?;
        check(first <= 16 && list.rows_built() - first <= 16, &format!("built {first} then {}", list.rows_built() - first))?;
        check(list.selected() == Some(500_000), "selection")
    });
    run("menu overlay opens on top and filter ranks", &|shots| {
        let mut win = AppWindow::new("t", 400, 400);
        win.end();
        let before = win.children();
        Menu::open(&*win, "Go", vec![MenuItem::new("", "Apps"), MenuItem::new("", "System")], |_| {});
        win.snapshot(shots.join("menu.png")).map_err(|e| e.to_string())?;
        check(win.children() == before + 1, "layer added")
    });
    run("notification toast is stacked and counted", &|shots| {
        let mut win = AppWindow::new("t", 420, 300);
        win.end();
        Notification::show(&*win, "", "Saved", "body", Urgency::Critical);
        Notification::show(&*win, "", "Second", "", Urgency::Critical);
        win.snapshot(shots.join("toasts.png")).map_err(|e| e.to_string())?;
        check(Notification::count() == 2, "two toasts")
    });
    run("task progress/done run on the UI thread; cancel stops work", &|_| {
        let main = std::thread::current().id();
        let seen = Rc::new(RefCell::new(Vec::new()));
        let (s1, s2) = (seen.clone(), seen.clone());
        let h = Task::new(|ctx| { for i in 0..5 { ctx.progress(i as f64); std::thread::sleep(Duration::from_millis(5)); } Ok(42) })
            .on_progress(move |p| s1.borrow_mut().push((std::thread::current().id() == main, p)))
            .on_done(move |o| s2.borrow_mut().push((std::thread::current().id() == main, if o == Outcome::Done(42) { 42.0 } else { -1.0 })))
            .start();
        pump_until(|| h.is_finished() && seen.borrow().last().is_some_and(|x| x.1 == 42.0));
        check(seen.borrow().iter().all(|x| x.0), "callbacks on main thread")?;
        let done = Rc::new(RefCell::new(None));
        let d2 = done.clone();
        let h = Task::new(|ctx| { loop { ctx.sleep(Duration::from_millis(10))?; } })
            .on_done(move |o: Outcome<()>| *d2.borrow_mut() = Some(o)).start();
        std::thread::sleep(Duration::from_millis(30));
        let t0 = Instant::now();
        h.cancel();
        pump_until(|| done.borrow().is_some());
        check(*done.borrow() == Some(Outcome::Cancelled) && t0.elapsed() < Duration::from_millis(500), "cancelled quickly")
    });
    run("corner preference: square vs rounded button corner pixel", &|shots| {
        let mut win = AppWindow::new("t", 300, 140);
        let mut body = win.body();
        let b = Button::new("Go").primary();
        body.fit(&*b);
        win.end();
        theme::set_corners(Corners::Square);
        let p = shots.join("corner-square.png");
        win.snapshot(&p).map_err(|e| e.to_string())?;
        let square = pixel(&p, b.x(), b.y())?;
        theme::set_corners(Corners::Rounded);
        let p = shots.join("corner-rounded.png");
        win.snapshot(&p).map_err(|e| e.to_string())?;
        let rounded = pixel(&p, b.x(), b.y())?;
        theme::set_corners(Corners::Omarchy);
        let bg = theme::current().palette.background;
        check(square != [bg.r(), bg.g(), bg.b()], &format!("square corner painted {square:?}"))?;
        check(rounded == [bg.r(), bg.g(), bg.b()], &format!("rounded corner shows background {rounded:?}"))?;
        check(theme::current().badge_radius() == 8, "badges keep a fixed radius")
    });
    run("popup menu, tab bar, searchable filter, window mode parse", &|shots| {
        let mut win = AppWindow::new("t", 400, 300);
        let mut body = win.body();
        let tab = State::new(0usize);
        let closed = Rc::new(RefCell::new(None));
        let c2 = closed.clone();
        let tabs = TabBar::new(&["A", "B"], &tab).on_close(move |i| *c2.borrow_mut() = Some(i));
        body.fit(&*tabs);
        let anchor = Button::new("Menu");
        body.fit(&*anchor);
        win.end();
        let before = win.children();
        PopupMenu::open(&*anchor, vec![PopupItem::new("", "One").shortcut("Ctrl+1"), PopupItem::new("", "Two").disabled()], |_| {});
        win.snapshot(shots.join("popup.png")).map_err(|e| e.to_string())?;
        check(win.children() == before + 1, "popup layer added")?;
        tabs.request_close(1);
        check(*closed.borrow() == Some(1), "tab close callback")?;
        let opts = vec![("Home".to_string(), "~/".to_string()), ("Work".to_string(), "~/Work".to_string())];
        check(SearchableDropdown::filter(&opts, "work") == vec![1], "filter by label")?;
        check(SearchableDropdown::filter(&opts, "~/") == vec![0, 1], "filter by description")?;
        check(Mode::parse("fullscreen") == Some(Mode::Fullscreen) && Mode::Normal.as_str() == "normal", "mode parse")
    });
    run("operation toast pause/cancel callbacks", &|_| {
        let mut win = AppWindow::new("t", 420, 300);
        win.end();
        let paused = Rc::new(RefCell::new(Vec::new()));
        let cancelled = Rc::new(RefCell::new(false));
        let (p2, c2) = (paused.clone(), cancelled.clone());
        let mut t = OperationToast::show(&*win, "Copying").ok_or("toast")?
            .on_pause(move |p| p2.borrow_mut().push(p)).on_cancel(move || *c2.borrow_mut() = true);
        t.set_progress(0.5);
        t.toggle_pause();
        t.toggle_pause();
        t.cancel();
        t.close();
        check(*paused.borrow() == vec![true, false] && *cancelled.borrow(), "callbacks")
    });
    run("live theme switch: omarchy-theme-set style rename is picked up", &|shots| {
        let state = PathBuf::from(std::env::var("OMARCHY_FLTK_STATE_DIR").unwrap());
        std::fs::create_dir_all(state.join("next-theme")).unwrap();
        std::fs::write(state.join("next-theme/colors.toml"), COLORS_B).unwrap();
        std::fs::remove_dir_all(state.join("theme")).unwrap();
        std::fs::rename(state.join("next-theme"), state.join("theme")).unwrap();
        pump_until(|| theme::current().palette.accent == theme::Rgb(0x33ccff));
        check(theme::current().palette.accent == theme::Rgb(0x33ccff), "accent after swap")?;
        let mut win = AppWindow::new("t", 300, 140);
        let mut body = win.body();
        let b = Button::new("Go").primary();
        body.fit(&*b);
        win.end();
        let p = shots.join("button-theme-b.png");
        win.snapshot(&p).map_err(|e| e.to_string())?;
        let px = pixel(&p, b.x() + 3, b.y() + 3)?;
        check(px == [0x33, 0xcc, 0xff], &format!("pixel {px:?}"))?;
        // Edit in place after the swap: the watcher must follow the new dir.
        std::fs::write(state.join("theme/colors.toml"), COLORS_A).unwrap();
        pump_until(|| theme::current().palette.accent == theme::Rgb(0xe68e0d));
        check(theme::current().palette.accent == theme::Rgb(0xe68e0d), "accent after in-place edit")
    });
    drop(app);
    println!("{}", if failed == 0 { "all ui checks passed" } else { "ui checks FAILED" });
    std::process::exit(if failed == 0 { 0 } else { 1 });
}

fn check(ok: bool, what: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(what.to_string()) }
}

fn pump_until(mut f: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(3);
    while !f() && Instant::now() < end {
        App::pump();
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Reads one RGB pixel from a PNG written by our encoder (stored deflate).
fn pixel(path: &Path, x: i32, y: i32) -> Result<[u8; 3], String> {
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    let w = u32::from_be_bytes(data[16..20].try_into().unwrap()) as usize;
    let depth = if data[25] == 6 { 4 } else { 3 };
    let (mut pos, mut z) = (8, Vec::new());
    while pos + 8 <= data.len() {
        let len = u32::from_be_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
        if &data[pos + 4..pos + 8] == b"IDAT" { z.extend_from_slice(&data[pos + 8..pos + 8 + len]); }
        pos += 12 + len;
    }
    let (mut raw, mut i) = (Vec::new(), 2);
    loop {
        let last = z[i] & 1;
        let len = u16::from_le_bytes([z[i + 1], z[i + 2]]) as usize;
        raw.extend_from_slice(&z[i + 5..i + 5 + len]);
        i += 5 + len;
        if last == 1 { break; }
    }
    let stride = w * depth + 1;
    let off = y as usize * stride + 1 + x as usize * depth;
    Ok([raw[off], raw[off + 1], raw[off + 2]])
}
