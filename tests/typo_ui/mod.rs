//! UI checks of font weights and letter spacing (`paint::Weight`,
//! `paint::text_spaced`).
use super::{check, pixel, pump_until};
use fltk::prelude::*;
use fltk_omarchy::paint::{self, Weight};
use fltk_omarchy::{theme, AppWindow};
use std::cell::Cell;
use std::path::Path;
use std::rc::Rc;

type Check<'a> = &'a dyn Fn(&Path) -> Result<(), String>;

pub fn checks(run: &mut dyn FnMut(&str, Check)) {
    run("paint: font weights (semibold has at least regular's ink) and letter spacing", &|shots| {
        let mut win = AppWindow::new("Type", 420, 200);
        let mut body = win.body();
        let mut canvas = fltk::widget::Widget::default();
        body.add(&canvas);
        let widths = Rc::new(Cell::new((0, 0)));
        let wd = widths.clone();
        canvas.draw(move |w| {
            theme::with(|t| {
                let fg = t.palette.foreground;
                paint::text_weight("Working on it", w.x() + 10, w.y() + 10, 300, 30, 20, Weight::Regular, fg, fltk::enums::Align::Left);
                paint::text_weight("Working on it", w.x() + 10, w.y() + 50, 300, 30, 20, Weight::SemiBold, fg, fltk::enums::Align::Left);
                let a = paint::text_spaced("SECTION", w.x() + 10, w.y() + 90, 380, 24, 12, Weight::Medium, 0.0, fg, fltk::enums::Align::Left);
                let b = paint::text_spaced("SECTION", w.x() + 10, w.y() + 120, 380, 24, 12, Weight::Medium, 2.0, fg, fltk::enums::Align::Left);
                wd.set((a, b));
            });
        });
        win.end();
        win.show();
        let c2 = canvas.clone();
        pump_until(|| c2.w() > 0);
        let p = shots.join("typo-weights.png");
        win.clone().snapshot(&p).map_err(|e| e.to_string())?;
        let bg = theme::with(|t| t.palette.background);
        let ink = |y0: i32| -> Result<u32, String> {
            let mut n = 0;
            for y in y0..y0 + 30 {
                for x in canvas.x() + 10..canvas.x() + 310 {
                    let [r, g, b] = pixel(&p, x, canvas.y() + y)?;
                    n += (r.abs_diff(bg.r()) as u32 + g.abs_diff(bg.g()) as u32 + b.abs_diff(bg.b()) as u32) / 3;
                }
            }
            Ok(n)
        };
        let (regular, semibold) = (ink(10)?, ink(50)?);
        check(regular > 0 && semibold >= regular, &format!("ink regular {regular} semibold {semibold}"))?;
        let (a, b) = widths.get();
        check(b == a + 2 * 6 || b == a + 2 * 6 + 1, &format!("letter spacing adds 2 px between 7 letters: {a} → {b}"))?;
        fltk::window::DoubleWindow::delete((*win).clone());
        Ok(())
    });

    #[cfg(feature = "qr")]
    run("qr: dark modules on a light card with the quiet zone", &|shots| {
        use fltk_omarchy::layout::FlexExt;
        use fltk_omarchy::qr::{modules, Qr, QUIET};
        use fltk_omarchy::State;
        let text = "https://example.com/pair?code=123456";
        let link = State::new(text.to_string());
        let mut win = AppWindow::new("QR", 300, 300);
        let mut body = win.body();
        let mut code = Qr::new(&link).accessible_label("Pairing code");
        code.set_size(200, 200);
        body.fit(&*code);
        win.end();
        win.show();
        let c2 = code.clone();
        pump_until(|| c2.w() > 0);
        let p = shots.join("qr.png");
        win.clone().snapshot(&p).map_err(|e| e.to_string())?;
        let m = modules(text).ok_or("no modules")?;
        let n = (m.len() + 2 * QUIET) as i32;
        let unit = code.w().min(code.h()) / n;
        let side = unit * n;
        let (x0, y0) = (code.x() + (code.w() - side) / 2, code.y() + (code.h() - side) / 2);
        let (fg, bg) = theme::with(|t| (t.palette.foreground, t.palette.background));
        let at = |r: usize, c: usize| pixel(&p, x0 + (c + QUIET) as i32 * unit + unit / 2, y0 + (r + QUIET) as i32 * unit + unit / 2);
        let same = |a: [u8; 3], b: theme::Rgb| a[0].abs_diff(b.r()) <= 2 && a[1].abs_diff(b.g()) <= 2 && a[2].abs_diff(b.b()) <= 2;
        let mut wrong = 0;
        for (r, row) in m.iter().enumerate() {
            for (c, on) in row.iter().enumerate() {
                if !same(at(r, c)?, if *on { bg } else { fg }) {
                    wrong += 1;
                }
            }
        }
        check(wrong == 0, &format!("{wrong} modules differ from the code"))?;
        let quiet = pixel(&p, x0 + unit * 2, y0 + unit * 2)?;
        check(same(quiet, fg), "quiet zone is the light card")?;
        link.set("x".repeat(5000));
        fltk::window::DoubleWindow::delete((*win).clone());
        Ok(())
    });

    run("text field mask: typed literals, paste, dropped letters, bound state", &|_| {
        use fltk_omarchy::focus::{press, type_text};
        use fltk_omarchy::layout::FlexExt;
        use fltk_omarchy::widgets::TextField;
        use fltk_omarchy::State;
        let time = State::new(String::new());
        let mut win = AppWindow::new("Mask", 300, 160);
        let mut body = win.body();
        let field = TextField::new().placeholder("HH:MM").mask("99:99").bind(&time);
        body.fit(&*field);
        win.end();
        win.show();
        let f2 = field.clone();
        pump_until(|| f2.w() > 0);
        let mut input = field.input();
        let _ = input.take_focus();
        for k in ["0", "7", "3"] {
            press(k);
        }
        check(field.value() == "07:3" && time.get() == "07:3", &format!("typing 073 → {:?} / state {:?}", field.value(), time.get()))?;
        check(!field.is_complete(), "07:3 is not complete")?;
        press("x");
        press("0");
        press("9");
        check(field.value() == "07:30" && field.is_complete(), &format!("letter dropped, end reached: {:?}", field.value()))?;
        press("BackSpace");
        press("BackSpace");
        check(field.value() == "07:", &format!("backspace keeps a typed literal: {:?}", field.value()))?;
        input.set_value("");
        let _ = input.set_position(0);
        type_text("1945");
        check(field.value() == "19:45" && time.get() == "19:45", &format!("paste 1945 → {:?}", field.value()))?;
        time.set("0815".into());
        pump_until(|| time.get() == "08:15");
        check(field.value() == "08:15" && time.get() == "08:15", &format!("state set by the app is formatted: {:?} / {:?}", field.value(), time.get()))?;
        fltk::window::DoubleWindow::delete((*win).clone());
        Ok(())
    });

    run("rich text: a settled streaming tail hides open markers; finish gives the exact text", &|_| {
        use fltk_omarchy::text::{markdown::MarkdownOptions, RichText};
        let mut win = AppWindow::new("Settle", 420, 200);
        let rt = RichText::new().markdown_options(MarkdownOptions::default().with_settle_tail(true));
        win.end();
        win.show();
        let r2 = rt.clone();
        pump_until(|| r2.w() > 0);
        rt.push_markdown("Hello **wor");
        fltk::app::wait_for(0.01).ok();
        check(rt.plain_text().contains("Hello wor"), &format!("the open bold word is shown: {:?}", rt.plain_text()))?;
        check(rt.plain_text().find('*').is_none(), &format!("no raw markers while streaming: {:?}", rt.plain_text()))?;
        rt.push_markdown("ld** and `co");
        check(rt.plain_text().find('`').is_none(), "an open code span is shown closed")?;
        rt.push_markdown("de` done");
        rt.finish();
        check(rt.plain_text().trim_end() == "Hello world and code done", &format!("exact after finish: {:?}", rt.plain_text()))?;
        fltk::window::DoubleWindow::delete((*win).clone());
        Ok(())
    });

    run("rich text: on_far_from_bottom reports only beyond its threshold; follow_end returns", &|_| {
        use fltk_omarchy::text::RichText;
        use std::cell::RefCell;
        use std::rc::Rc;
        let log: Rc<RefCell<Vec<bool>>> = Rc::default();
        let l2 = log.clone();
        let mut win = AppWindow::new("Far", 420, 300);
        let rt = RichText::new().stick_to_bottom(true).on_far_from_bottom(400, move |far| l2.borrow_mut().push(far));
        win.end();
        win.show();
        for k in 0..80 {
            rt.append_markdown(&format!("Paragraph {k} with a few words to wrap once in this width maybe."));
        }
        let r2 = rt.clone();
        pump_until(|| r2.w() > 0 && r2.is_at_bottom());
        let settle = || {
            for _ in 0..4 {
                fltk::app::flush();
                let _ = fltk::app::wait_for(0.02);
            }
        };
        settle();
        check(log.borrow().is_empty(), &format!("at the end: nothing to report {:?}", log.borrow()))?;
        let end = rt.content_height() - rt.h() as i64;
        rt.scroll_to(end - 100);
        settle();
        check(log.borrow().is_empty(), "100 px above the end is within the threshold")?;
        rt.scroll_to(0);
        settle();
        check(*log.borrow() == [true], &format!("far from the end: {:?}", log.borrow()))?;
        rt.follow_end();
        settle();
        check(*log.borrow() == [true, false] && rt.is_following(), &format!("follow_end comes back: {:?}", log.borrow()))?;
        fltk::window::DoubleWindow::delete((*win).clone());
        Ok(())
    });

    run("rich text: word pacing holds a partial word from a sender for at most 80 ms", &|_| {
        use fltk_omarchy::text::RichText;
        use std::time::{Duration, Instant};
        let mut win = AppWindow::new("Pace", 420, 200);
        let rt = RichText::new().pace_words(true);
        win.end();
        win.show();
        let r2 = rt.clone();
        pump_until(|| r2.w() > 0);
        let tx = rt.sender();
        tx.push("Hello wor");
        let t0 = Instant::now();
        while Instant::now() < t0 + Duration::from_millis(30) {
            fltk_omarchy::App::pump();
            let _ = fltk::app::wait_for(0.005);
        }
        check(rt.plain_text().trim_end() == "Hello", &format!("the partial word waits: {:?}", rt.plain_text()))?;
        pump_until(|| rt.plain_text().contains("wor"));
        check(rt.plain_text().trim_end() == "Hello wor" && t0.elapsed() < Duration::from_millis(400), &format!("then it arrives: {:?} after {:?}", rt.plain_text(), t0.elapsed()))?;
        tx.push("ld and more ");
        pump_until(|| rt.plain_text().contains("more"));
        check(rt.plain_text().trim_end() == "Hello world and more", &format!("{:?}", rt.plain_text()))?;
        fltk::window::DoubleWindow::delete((*win).clone());
        Ok(())
    });

    run("composer: page keys scroll the transcript while the text fits", &|_| {
        use fltk_omarchy::edit::Composer;
        use fltk_omarchy::focus::press;
        use fltk_omarchy::layout::FlexExt;
        use fltk_omarchy::text::RichText;
        let mut win = AppWindow::new("Pages", 420, 360);
        let mut body = win.body();
        let rt = RichText::new().stick_to_bottom(true);
        body.add(&*rt);
        let composer = Composer::new().page_keys_to(&rt);
        body.fit(&*composer);
        win.end();
        win.show();
        for k in 0..80 {
            rt.append_markdown(&format!("Paragraph {k} with words enough to wrap once."));
        }
        let r2 = rt.clone();
        pump_until(|| r2.w() > 0 && r2.is_at_bottom());
        let _ = composer.clone().take_focus();
        fltk_omarchy::focus::type_text("hi");
        let end = rt.scroll_offset();
        press("PageUp");
        check(rt.scroll_offset() < end && !rt.is_following(), &format!("PageUp scrolls up: {end} -> {}", rt.scroll_offset()))?;
        press("Mod+Home");
        check(rt.scroll_offset() == 0, "Mod+Home goes to the start")?;
        press("Mod+End");
        pump_until(|| r2.is_at_bottom());
        check(rt.is_following() && rt.is_at_bottom(), "Mod+End follows the end again")?;
        check(composer.value().contains("hi"), "the typed text stays")?;
        fltk::window::DoubleWindow::delete((*win).clone());
        Ok(())
    });

    run("task::Latest: debounced, latest wins, slow older answers are dropped", &|_| {
        use fltk_omarchy::task::Latest;
        use std::cell::RefCell;
        use std::rc::Rc;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        use std::time::Duration;
        let got: Rc<RefCell<Vec<String>>> = Rc::default();
        let g = got.clone();
        let runs = Arc::new(AtomicUsize::new(0));
        let latest = Latest::new(Duration::from_millis(60), move |v: String| g.borrow_mut().push(v));
        for q in ["a", "ab", "abc"] {
            let (r, q) = (runs.clone(), q.to_string());
            latest.request(move |_| {
                r.fetch_add(1, Ordering::SeqCst);
                Ok(q)
            });
        }
        pump_until(|| !got.borrow().is_empty());
        check(*got.borrow() == ["abc"] && runs.load(Ordering::SeqCst) == 1, &format!("debounced: {:?}, {} runs", got.borrow(), runs.load(Ordering::SeqCst)))?;
        let fast = Latest::new(Duration::ZERO, {
            let g = got.clone();
            move |v: String| g.borrow_mut().push(v)
        });
        fast.request(|ctx| {
            ctx.sleep(Duration::from_millis(300))?;
            Ok("slow".to_string())
        });
        fast.request(|_| Ok("new".to_string()));
        pump_until(|| got.borrow().len() >= 2);
        let t = std::time::Instant::now();
        while t.elapsed() < Duration::from_millis(450) {
            fltk_omarchy::App::pump();
            std::thread::sleep(Duration::from_millis(10));
        }
        check(*got.borrow() == ["abc", "new"], &format!("the older slow answer is dropped: {:?}", got.borrow()))?;
        Ok(())
    });

    run("rich text: chip links draw as accent pills and still open", &|shots| {
        use fltk_omarchy::text::{markdown::MarkdownOptions, RichText};
        use std::cell::RefCell;
        use std::rc::Rc;
        let opened: Rc<RefCell<Vec<String>>> = Rc::default();
        let o2 = opened.clone();
        let mut win = AppWindow::new("Chips", 420, 160);
        let rt = RichText::new()
            .markdown_options(MarkdownOptions::default().with_chip_links(|u| u.starts_with("file:")))
            .on_link(move |u| o2.borrow_mut().push(u.to_string()));
        win.end();
        win.show();
        rt.append_markdown("Look at [@notes](file:notes.md) please");
        let r2 = rt.clone();
        pump_until(|| r2.w() > 0);
        let p = shots.join("chips.png");
        win.clone().snapshot(&p).map_err(|e| e.to_string())?;
        let r = rt.text_rect("@notes").ok_or("chip word")?;
        let (accent, bg) = theme::with(|t| (t.palette.accent, t.palette.background));
        let want = accent.over(bg, 0.16);
        let [cr, cg, cb] = pixel(&p, r.0 - 2, r.1 + r.3 / 2)?;
        let near = |a: u8, b: u8| a.abs_diff(b) <= 3;
        check(near(cr, want.r()) && near(cg, want.g()) && near(cb, want.b()), &format!("chip fill left of the word: {:?} vs {want:?}", (cr, cg, cb)))?;
        fltk_omarchy::focus::click_at(&*win, r.0 + r.2 / 2, r.1 + r.3 / 2);
        check(*opened.borrow() == ["file:notes.md"], &format!("a click opens it: {:?}", opened.borrow()))?;
        fltk::window::DoubleWindow::delete((*win).clone());
        Ok(())
    });

    run("i18n: follows Omarchy's Setup > Language file, unless the app picked a language", &|_| {
        use fltk_omarchy::i18n;
        use fltk_omarchy::platform::{Os, OS};
        if OS != Os::Linux {
            return Ok(()); // Omarchy's language file exists on Linux only
        }
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("build/test-ui/xdg-config");
        std::fs::create_dir_all(root.join("omarchy")).map_err(|e| e.to_string())?;
        std::fs::write(root.join("omarchy/language"), "da\n").map_err(|e| e.to_string())?;
        let old = std::env::var_os("XDG_CONFIG_HOME");
        std::env::set_var("XDG_CONFIG_HOME", &root);
        let result = (|| -> Result<(), String> {
            check(i18n::system_locales().first().map(String::as_str) == Some("da"), &format!("system languages start with Omarchy's: {:?}", i18n::system_locales()))?;
            i18n::follow_system();
            check(i18n::current().starts_with("da"), &format!("following: {}", i18n::current()))?;
            std::fs::write(root.join("omarchy/language"), "es-MX\n").map_err(|e| e.to_string())?;
            fltk_omarchy::i18n::load_omarchy_catalog("es-MX", r#"{"Hello": "Hola"}"#).map_err(|e| e.to_string())?;
            // The watcher calls this on the UI thread; call it directly here.
            i18n::refresh_system_language();
            check(i18n::current().starts_with("es-mx"), &format!("switched live: {}", i18n::current()))?;
            check(fltk_omarchy::tr!("Hello") == "Hola", "app catalog in Omarchy's format")?;
            i18n::set_locale("en");
            std::fs::write(root.join("omarchy/language"), "fr\n").map_err(|e| e.to_string())?;
            i18n::refresh_system_language();
            check(i18n::current() == "en", &format!("an app's own choice stays: {}", i18n::current()))?;
            Ok(())
        })();
        match old {
            Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
        let _ = std::fs::remove_file(root.join("omarchy/language"));
        fltk_omarchy::i18n::set_locale("en");
        result
    });

    run("Omarchy keys: ? opens the shortcut overlay, q in a field types, Q asks with unsaved work", &|_| {
        use fltk_omarchy::focus::{press, type_text};
        use fltk_omarchy::layout::FlexExt;
        use fltk_omarchy::widgets::TextField;
        let mut win = AppWindow::new("Keys", 420, 260);
        let mut body = win.body();
        let field = TextField::new();
        body.fit(&*field);
        let btn = fltk_omarchy::widgets::Button::new("Main");
        body.fit(&*btn);
        win.end();
        win.show();
        win.help_overlay("Shortcuts", || vec![("General".to_string(), vec![("?".to_string(), "Shortcuts".to_string()), ("Q".to_string(), "Quit".to_string())])]);
        win.quit_key(|| true);
        let f2 = field.clone();
        pump_until(|| f2.w() > 0);
        let layers = |w: &AppWindow| w.children();
        let base = layers(&win);
        let _ = btn.clone().take_focus();
        type_text("?");
        check(layers(&win) > base, "? opened the overlay")?;
        press("Escape");
        pump_until(|| layers(&win) == base);
        let mut input = field.input();
        let _ = input.take_focus();
        type_text("q");
        check(field.value() == "q" && layers(&win) == base, &format!("q typed into the field: {:?}", field.value()))?;
        let _ = btn.clone().take_focus();
        press("q");
        check(layers(&win) > base, "Q with unsaved work asks first")?;
        press("Escape");
        fltk::window::DoubleWindow::delete((*win).clone());
        Ok(())
    });
}
