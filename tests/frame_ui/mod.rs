//! UI checks of `frame::FrameView`: frames from an in-process `Producer`
//! (same shared memory and wake-ups as another process) reach the screen
//! 1:1 and letterboxed, RGBA and BGRA; keys, clicks, the wheel, hover
//! throttling, resize and copy reach the producer; Tab and Esc stay with
//! the app; PNG/BMP frame files; producer messages and exit.
use super::{check, pixel, pump_until};
use fltk::enums::Key;
use fltk::prelude::*;
use fltk_omarchy::focus::{self, press};
use fltk_omarchy::frame::{Button, Format, FrameView, Input, Message, Mods, Producer};
use fltk_omarchy::theme;
use fltk_omarchy::widgets::Button as Btn;
use std::cell::{Cell, RefCell};
use std::path::Path;
use std::rc::Rc;
use std::time::{Duration, Instant};

type Check<'a> = &'a dyn Fn(&Path) -> Result<(), String>;

fn plain(w: i32, h: i32) -> fltk::window::DoubleWindow {
    fltk::window::DoubleWindow::new(0, 0, w, h, None)
}

fn shot(win: &mut fltk::window::DoubleWindow, p: &Path) -> Result<(), String> {
    fltk_omarchy::window::snapshot(win, p).map_err(|e| e.to_string())
}

fn close(a: [u8; 3], b: [u8; 3], tol: i32) -> bool {
    a.iter().zip(b).all(|(x, y)| (*x as i32 - y as i32).abs() <= tol)
}

/// Fills the next frame with `rgb`, a `band`-pixel stripe of `left` at
/// its left edge, in the producer's format.
fn paint(p: &mut Producer, format: Format, rgb: [u8; 3], left: [u8; 3], band: u32) {
    p.set_format(format, true);
    let (w, h) = p.size();
    let stride = p.stride();
    let px = p.frame_mut();
    for y in 0..h as usize {
        for x in 0..w as usize {
            let c = if (x as u32) < band { left } else { rgb };
            let v = match format {
                Format::Rgba => [c[0], c[1], c[2], 255],
                Format::Bgra => [c[2], c[1], c[0], 255],
            };
            px[y * stride + x * 4..y * stride + x * 4 + 4].copy_from_slice(&v);
        }
    }
    p.present();
}

/// Pumps the loop until `f` holds or `ms` pass.
fn pump_for(ms: u64, mut f: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_millis(ms);
    while !f() && Instant::now() < end {
        fltk_omarchy::App::pump();
        std::thread::sleep(Duration::from_millis(5));
    }
}

pub fn checks(run: &mut dyn FnMut(&str, Check)) {
    run("frame: shared-memory frames reach the screen 1:1, letterboxed, RGBA zero-copy and BGRA", &|shots| {
        let mut win = plain(240, 160);
        let mut view = FrameView::new(200, 100);
        view.set_pos(20, 30);
        win.end();
        win.show();
        let (h, mut p) = Producer::create(200, 100).map_err(|e| e.to_string())?;
        view.open(&h).map_err(|e| e.to_string())?;
        let (green, red, blue) = ([0x20, 0xa0, 0x60], [0xd0, 0x30, 0x30], [0x30, 0x40, 0xe0]);
        paint(&mut p, Format::Rgba, green, red, 10);
        pump_until(|| view.stats().received >= 1);
        let path = shots.join("frame-1to1.png");
        shot(&mut win, &path)?;
        check(close(pixel(&path, 120, 80)?, green, 2), &format!("frame body {:?}", pixel(&path, 120, 80)?))?;
        check(close(pixel(&path, 24, 80)?, red, 2), &format!("left stripe {:?}", pixel(&path, 24, 80)?))?;
        check(close(pixel(&path, 32, 80)?, green, 2), "stripe ends at 10 px (1:1)")?;
        let st = view.stats();
        check(st.zero_copy && st.drawn >= 1 && st.frame_size == (200, 100), &format!("stats {st:?}"))?;
        check(st.last_latency.is_some(), "latency measured")?;
        // A square frame in a 2:1 view: centred with bars.
        p.set_size(100, 100);
        paint(&mut p, Format::Bgra, blue, blue, 0);
        pump_until(|| view.stats().received >= 2);
        let path = shots.join("frame-letterbox.png");
        shot(&mut win, &path)?;
        let bg = theme::with(|t| t.palette.background);
        let bg = [bg.r(), bg.g(), bg.b()];
        check(close(pixel(&path, 40, 80)?, bg, 2), &format!("bar {:?}", pixel(&path, 40, 80)?))?;
        check(close(pixel(&path, 120, 80)?, blue, 2), &format!("BGRA swizzled {:?}", pixel(&path, 120, 80)?))?;
        check(!view.stats().zero_copy, "BGRA is converted")?;
        check(view.frame_size() == Some((100, 100)), "frame size")?;
        win.hide();
        Ok(())
    });

    run("frame: keys go to the producer except Tab and Esc; clicks, wheel and hover map to frame pixels; resize and copy", &|_| {
        let mut win = plain(300, 200);
        let mut before = Btn::new("Before");
        before.set_pos(10, 5);
        let mut view = FrameView::new(200, 100)
            .reserve(|k, _| k == Key::F1)
            .resize_debounce(Duration::from_millis(30));
        view.set_pos(20, 50);
        let mut after = Btn::new("After");
        after.set_pos(10, 160);
        win.end();
        // Esc stays with the app: here the window would close on it.
        let esc = Rc::new(Cell::new(0));
        let e2 = esc.clone();
        win.set_callback(move |_| e2.set(e2.get() + 1));
        win.show();
        let (h, mut p) = Producer::create(200, 100).map_err(|e| e.to_string())?;
        view.open(&h).map_err(|e| e.to_string())?;
        paint(&mut p, Format::Rgba, [9, 9, 9], [9, 9, 9], 0);
        pump_until(|| view.stats().received >= 1);
        let got: RefCell<Vec<Input>> = RefCell::new(Vec::new());
        let drain = |p: &mut Producer| {
            fltk_omarchy::App::pump();
            got.borrow_mut().extend(p.events());
        };
        drain(&mut p);
        let first = got.borrow().clone();
        check(matches!(first.first(), Some(Input::Resize { width: 200, height: 100, .. })), &format!("size on attach: {first:?}"))?;
        got.borrow_mut().clear();
        // Tab from the button before lands on the view.
        let _ = (*before).clone().take_focus();
        press("Tab");
        check(focus::is_focused(&*view), "view is a Tab stop")?;
        press("x");
        press("Shift+x");
        press("Escape");
        press("F1");
        drain(&mut p);
        let keys: Vec<Input> = got.borrow().clone();
        let downs: Vec<(String, String, Mods)> = keys
            .iter()
            .filter_map(|e| match e {
                Input::KeyDown { key, text, mods, .. } => Some((key.clone(), text.clone(), *mods)),
                _ => None,
            })
            .collect();
        check(keys.first() == Some(&Input::FocusIn), &format!("focus in first: {keys:?}"))?;
        check(
            downs == vec![("x".into(), "x".into(), Mods::NONE), ("x".into(), "X".into(), Mods::SHIFT)],
            &format!("x and X only (Esc and the reserved F1 stay with the app): {downs:?}"),
        )?;
        got.borrow_mut().clear();
        check(esc.get() == 1, &format!("Esc reached the window ({})", esc.get()))?;
        check(focus::is_focused(&*view), "still focused after Esc and F1")?;
        press("Tab");
        let cur = focus::current().map(|w| w.label());
        check(focus::is_focused(&*after), &format!("Tab leaves the view (focus on {cur:?})"))?;
        drain(&mut p);
        check(got.borrow().as_slice() == [Input::FocusOut], &format!("only focus out: {:?}", got.borrow()))?;
        got.borrow_mut().clear();
        // Click at the frame's centre: frame pixels (1:1 here).
        focus::click_at(&win, 20 + 100, 50 + 40);
        check(focus::is_focused(&*view), "a click focuses the view")?;
        focus::wheel_at(&win, 20 + 50, 50 + 10, 3);
        drain(&mut p);
        let ev = got.borrow().clone();
        let press_at = ev.iter().find_map(|e| match e {
            Input::Press { x, y, button: Button::Left, buttons: 1, .. } => Some((*x, *y)),
            _ => None,
        });
        check(press_at == Some((100.0, 40.0)), &format!("press in frame pixels: {ev:?}"))?;
        check(ev.iter().any(|e| matches!(e, Input::Release { button: Button::Left, buttons: 0, .. })), "release")?;
        check(ev.iter().any(|e| matches!(e, Input::Wheel { x, y, dy, .. } if (*x, *y, *dy) == (50.0, 10.0, 3.0))), &format!("wheel: {ev:?}"))?;
        got.borrow_mut().clear();
        // Hover: three moves at once give one now and the newest after 40 ms.
        for (i, x) in [30, 40, 50].iter().enumerate() {
            let _ = i;
            focus::hover_at(&win, 20 + x, 50 + 20);
        }
        drain(&mut p);
        let now: Vec<Input> = got.borrow().iter().filter(|e| matches!(e, Input::Move { .. })).cloned().collect();
        check(now.len() <= 1, &format!("throttled: {now:?}"))?;
        pump_for(300, || {
            drain(&mut p);
            got.borrow().iter().any(|e| matches!(e, Input::Move { x, .. } if *x == 50.0))
        });
        let moves: Vec<Input> = got.borrow().iter().filter(|e| matches!(e, Input::Move { .. })).cloned().collect();
        check(moves.len() == 2 && matches!(moves[1], Input::Move { x, y, .. } if (x, y) == (50.0, 20.0)), &format!("newest move after the interval: {moves:?}"))?;
        got.borrow_mut().clear();
        // Copy: the key goes on and a copy request follows.
        let _ = (*view).clone().take_focus();
        press("Ctrl+c");
        drain(&mut p);
        check(got.borrow().iter().any(|e| e == &Input::Copy), &format!("copy request: {:?}", got.borrow()))?;
        got.borrow_mut().clear();
        // Resize: reported after the debounce, in logical pixels + scale.
        view.resize(20, 50, 160, 80);
        win.redraw();
        pump_for(1000, || {
            drain(&mut p);
            got.borrow().iter().any(|e| matches!(e, Input::Resize { width: 160, height: 80, .. }))
        });
        check(got.borrow().iter().any(|e| matches!(e, Input::Resize { width: 160, height: 80, .. })), &format!("resize: {:?}", got.borrow()))?;
        check(p.requested_size().map(|(w, h, _)| (w, h)) == Some((160, 80)), "requested size in the header")?;
        // Zoom: clamped, sent once per change.
        view.set_zoom(5.0);
        view.zoom_out();
        drain(&mut p);
        check(view.zoom() == 1.75 && (p.zoom() - 1.75).abs() < 1e-6, &format!("zoom {}", view.zoom()))?;
        win.hide();
        Ok(())
    });

    run("frame: content size and clipboard messages, producer exit, PNG/BMP frame files", &|shots| {
        let mut win = plain(220, 140);
        let content = Rc::new(Cell::new((0u32, 0u32)));
        let closed = Rc::new(Cell::new(false));
        let others = Rc::new(Cell::new(0));
        let (c2, cl2, o2) = (content.clone(), closed.clone(), others.clone());
        let mut view = FrameView::new(200, 100)
            .on_content_size(move |w, h| c2.set((w, h)))
            .on_closed(move || cl2.set(true))
            .on_message(move |m| {
                if matches!(m, Message::Other(_)) {
                    o2.set(o2.get() + 1)
                }
            });
        view.set_pos(10, 20);
        win.end();
        win.show();
        let (h, mut p) = Producer::create(200, 100).map_err(|e| e.to_string())?;
        view.open(&h).map_err(|e| e.to_string())?;
        p.send(&Message::ContentSize { width: 200, height: 1234 });
        p.send(&Message::Other(serde_json::json!({"appRequest": {"id": 1}})));
        pump_until(|| content.get().1 == 1234 && others.get() == 1);
        check(content.get() == (200, 1234), &format!("content size {:?}", content.get()))?;
        check(others.get() == 1, "app message")?;
        drop(p);
        pump_until(|| closed.get());
        check(closed.get(), "producer exit seen")?;
        // Frame files: a BMP written the way o3's helpers do (rename).
        let dir = shots.join("frame-files");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let file = dir.join("frame-0.bmp");
        std::fs::write(&file, bmp24(200, 100, [0xe0, 0x90, 0x10])).map_err(|e| e.to_string())?;
        view.watch_file(&file, Duration::from_millis(10));
        pump_until(|| view.stats().received >= 1);
        let path = shots.join("frame-file.png");
        shot(&mut win, &path)?;
        check(close(pixel(&path, 110, 70)?, [0xe0, 0x90, 0x10], 2), &format!("file frame {:?}", pixel(&path, 110, 70)?))?;
        let tmp = dir.join("frame-0.bmp.next");
        std::fs::write(&tmp, bmp24(200, 100, [0x10, 0x90, 0xe0])).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &file).map_err(|e| e.to_string())?;
        pump_until(|| view.stats().received >= 2);
        shot(&mut win, &path)?;
        check(close(pixel(&path, 110, 70)?, [0x10, 0x90, 0xe0], 2), "replaced file shows")?;
        view.detach();
        win.hide();
        Ok(())
    });
}

/// A bottom-up 24-bit BMP filled with `rgb`.
fn bmp24(w: u32, h: u32, rgb: [u8; 3]) -> Vec<u8> {
    let row = (w as usize * 3).div_ceil(4) * 4;
    let size = 54 + row * h as usize;
    let mut b = Vec::with_capacity(size);
    b.extend_from_slice(b"BM");
    b.extend_from_slice(&(size as u32).to_le_bytes());
    b.extend_from_slice(&[0; 4]);
    b.extend_from_slice(&54u32.to_le_bytes());
    b.extend_from_slice(&40u32.to_le_bytes());
    b.extend_from_slice(&(w as i32).to_le_bytes());
    b.extend_from_slice(&(h as i32).to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&24u16.to_le_bytes());
    b.extend_from_slice(&[0; 24]);
    for _ in 0..h {
        let start = b.len();
        for _ in 0..w {
            b.extend_from_slice(&[rgb[2], rgb[1], rgb[0]]);
        }
        b.resize(start + row, 0);
    }
    b
}

#[cfg(feature = "a11y")]
pub fn a11y_checks(run: &mut dyn FnMut(&str, Check)) {
    run("a11y: a FrameView is a named, focusable Document (or Image)", &|_| {
        use fltk_omarchy::a11y;
        use fltk_omarchy::frame::FrameRole;
        let win = plain(300, 200);
        let mut a = FrameView::new(120, 80).name("Browser preview");
        a.set_pos(10, 10);
        let mut b = FrameView::new(120, 80).name("Phone screen").role(FrameRole::Image);
        b.set_pos(150, 10);
        win.end();
        let snap = a11y::snapshot(&win).ok_or("no tree")?;
        check(snap.contains("Document \"Browser preview\""), &format!("document:\n{snap}"))?;
        check(snap.contains("Image \"Phone screen\""), &format!("image:\n{snap}"))
    });
}
