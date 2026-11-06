//! UI checks of `RichText` as a transcript host (o3): keyed messages and
//! a model, embeds (painter, visibility, height changes, hosted widget,
//! activation), follow-the-end, bottom inset, prepend and scroll anchors,
//! code and table actions, custom link schemes and path links, images,
//! find and the selection API.
use super::{check, pixel, png_rgb};
use fltk::prelude::*;
use fltk_omarchy::focus::{self, press};
use fltk_omarchy::layout::FlexExt;
use fltk_omarchy::model::ListModel;
use fltk_omarchy::paint;
use fltk_omarchy::text::{BlockAction, BlockActionEvent, ImageData, MarkdownOptions, Message, RichText};
use fltk_omarchy::widgets::Button;
use fltk_omarchy::{theme, AppWindow};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

type Check<'a> = &'a dyn Fn(&Path) -> Result<(), String>;

fn shot(win: &AppWindow, dir: &Path, name: &str) -> Result<PathBuf, String> {
    let p = dir.join(name);
    win.clone().snapshot(&p).map_err(|e| e.to_string())?;
    Ok(p)
}

fn delete(win: AppWindow) {
    fltk::window::DoubleWindow::delete((*win).clone());
}

fn rgb(c: theme::Rgb) -> [u8; 3] {
    [c.r(), c.g(), c.b()]
}

/// A conversation of `n` messages `m0..` with unique words `w0..`.
fn conversation(rt: &RichText, n: usize) {
    for k in 0..n {
        rt.set_message(&format!("m{k}"), Message::markdown(&format!("**User** w{k}\n\nA reply with a few words to wrap, number {k}.")));
    }
}

pub fn checks(run: &mut dyn FnMut(&str, Check)) {
    run("transcript: keyed messages, embed painter, visibility callbacks, height change keeps the view", &|shots| {
        let mut win = AppWindow::new("t", 520, 320);
        let seen = Rc::new(RefCell::new(Vec::<(String, bool)>::new()));
        let s2 = seen.clone();
        let rt = RichText::new()
            .embed_painter(|p| paint::fill(p.x, p.y, p.w, p.h, 6, p.theme.palette.accent))
            .on_embed_visibility(move |id, on| s2.borrow_mut().push((id.to_string(), on)));
        win.end();
        rt.set_message("intro", Message::markdown("Hello **there**"));
        rt.set_message("card", Message::embed("card-1", 80, "Ran 3 commands"));
        conversation(&rt, 30);
        check(rt.message_count() == 32 && rt.message_index("card") == Some(1), "keys in order")?;
        let p = shot(&win, shots, "transcript-embed.png")?;
        let (x, y, w, h) = rt.embed_rect("card-1").ok_or_else(|| format!("embed rect: blocks {:?} of {} {:?} at {} h {}", rt.message_blocks("card"), rt.block_count(), rt.block(1), rt.scroll_offset(), rt.h()))?;
        check(h == 80 && w > 300, &format!("embed reserves its height ({w}x{h})"))?;
        let accent = rgb(theme::current().palette.accent);
        check(pixel(&p, x + w / 2, y + h / 2)? == accent, "the painter draws the embed")?;
        check(seen.borrow().as_slice() == [("card-1".to_string(), true)], &format!("entered the view: {:?}", seen.borrow()))?;
        // Scroll down: the card leaves the view.
        rt.scroll_to(rt.content_height() / 2);
        shot(&win, shots, "transcript-embed-gone.png")?;
        check(seen.borrow().last() == Some(&("card-1".to_string(), false)), &format!("left the view: {:?}", seen.borrow()))?;
        // Growing the card above the view does not move what is on screen.
        let word = (0..30).map(|k| format!("w{k}")).find(|w| rt.text_rect(w).is_some()).ok_or("a word in view")?;
        let before = rt.text_rect(&word).ok_or("word")?;
        check(rt.set_embed_height("card-1", 400), "height set")?;
        shot(&win, shots, "transcript-embed-grown.png")?;
        let after = rt.text_rect(&word).ok_or("word after")?;
        check(before.1 == after.1, &format!("view stays still ({before:?} → {after:?})"))?;
        // Replace and remove by key.
        rt.set_message("m3", Message::markdown("replaced w3 text"));
        check(rt.plain_text().contains("replaced w3 text"), "replace by key")?;
        check(rt.remove_message("m3") && !rt.plain_text().contains("replaced"), "remove by key")?;
        delete(win);
        Ok(())
    });
    run("transcript: follow the end stops on scroll up and resumes; bottom inset; prepend and anchors keep the view", &|shots| {
        let mut win = AppWindow::new("t", 520, 320);
        let follows = Rc::new(RefCell::new(Vec::<bool>::new()));
        let f2 = follows.clone();
        let rt = RichText::new().stick_to_bottom(true).bottom_inset(60).on_follow_change(move |on| f2.borrow_mut().push(on));
        let after = Button::new("after");
        win.end();
        conversation(&rt, 40);
        shot(&win, shots, "transcript-follow.png")?;
        check(rt.is_following() && rt.is_at_bottom(), "follows the end")?;
        let last = rt.text_rect("number 39").ok_or("last line")?;
        check(last.1 + last.3 <= rt.y() + rt.h() - 60, &format!("the inset keeps the end above the composer ({last:?})"))?;
        rt.push_message_markdown("m39", " streamed tail w999");
        shot(&win, shots, "transcript-follow-2.png")?;
        check(rt.text_rect("w999").is_some(), "still at the end while it grows")?;
        let _ = rt.clone().take_focus();
        press("Up");
        shot(&win, shots, "transcript-unfollow.png")?;
        check(!rt.is_following(), "scrolling up stops following")?;
        check(follows.borrow().last() == Some(&false), &format!("callback: {:?}", follows.borrow()))?;
        rt.push_message_markdown("m39", " more w1000");
        rt.follow_end();
        shot(&win, shots, "transcript-refollow.png")?;
        check(rt.is_following() && rt.is_at_bottom(), "follow_end resumes")?;
        // Prepend older history: the view stays still.
        rt.scroll_to(rt.content_height() / 2);
        shot(&win, shots, "transcript-mid.png")?;
        let word = (0..40).map(|k| format!("w{k}")).find(|w| rt.text_rect(w).is_some()).ok_or("a word in view")?;
        let before = rt.text_rect(&word).ok_or("word")?;
        rt.prepend_messages((0..10).map(|k| (format!("old{k}"), Message::markdown(&format!("Older message o{k}\n\nwith a second paragraph")))));
        shot(&win, shots, "transcript-prepended.png")?;
        let now = rt.text_rect(&word).ok_or("word after prepend")?;
        check(now.1 == before.1, &format!("prepend keeps the view ({before:?} → {now:?})"))?;
        check(rt.message_keys()[0] == "old0" && rt.message_count() == 50, "older messages first")?;
        // Save, move away, restore.
        let a = rt.scroll_anchor().ok_or("anchor")?;
        check(!a.at_end && a.key.starts_with('m'), &format!("anchor {a:?}"))?;
        rt.scroll_to(0);
        shot(&win, shots, "transcript-top.png")?;
        check(rt.restore_scroll(&a), "restore")?;
        shot(&win, shots, "transcript-restored.png")?;
        let back = rt.text_rect(&word).ok_or("word after restore")?;
        check(back.1 == before.1, &format!("restored ({before:?} → {back:?})"))?;
        let end = fltk_omarchy::text::ScrollAnchor { at_end: true, ..Default::default() };
        rt.restore_scroll(&end);
        check(rt.is_following(), "at_end restores following")?;
        let _ = after;
        delete(win);
        Ok(())
    });
    run("transcript: code header and table footer actions by Tab/Enter/Space and click, built-in copies", &|shots| {
        let mut win = AppWindow::new("t", 560, 420);
        let mut body = win.body();
        let before = Button::new("before");
        body.fit(&*before);
        let events = Rc::new(RefCell::new(Vec::<BlockActionEvent>::new()));
        let e2 = events.clone();
        let rt = RichText::new()
            .code_actions(|lang, _code| if lang == "sh" { vec![BlockAction::new("run", "Run in terminal")] } else { vec![] })
            .table_actions(true)
            .on_block_action(move |e| e2.borrow_mut().push(e.clone()));
        win.end();
        rt.set_message("a", Message::markdown("Run this:\n\n```sh\ncargo test\n```\n\n| a | b, c |\n|---|---|\n| 1 | 2 |\n"));
        let p = shot(&win, shots, "transcript-actions.png")?;
        let code = (0..rt.block_count()).find(|&k| rt.block(k).is_some_and(|b| b.is_code())).ok_or("code block")?;
        let (ax, ay, aw, ah) = rt.action_rect(code, "run").ok_or("run rect")?;
        let img = png_rgb(&p)?;
        let surface = img.at(ax + 1, ay + 1);
        let ink = (ax..ax + aw).flat_map(|x| (ay..ay + ah).map(move |y| (x, y))).filter(|&(x, y)| img.at(x, y) != surface).count();
        check(ink > 20, &format!("the action label is drawn ({ink} px)"))?;
        let _ = before.clone().take_focus();
        press("Tab");
        press("Tab");
        check(rt.focused_action().as_deref() == Some("run"), &format!("Tab: run action ({:?})", rt.focused_action()))?;
        press("Enter");
        {
            let e = events.borrow();
            let e = e.last().ok_or("event")?;
            check(e.action == "run" && e.lang.as_deref() == Some("sh") && e.text == "cargo test" && e.message == "a", &format!("run event {e:?}"))?;
        }
        press("Tab");
        check(rt.focused_action().as_deref() == Some(BlockAction::COPY), "Tab: copy")?;
        press("Enter");
        check(events.borrow().last().is_some_and(|e| e.action == BlockAction::COPY), "copy reported")?;
        press("Tab");
        check(rt.focused_action().as_deref() == Some(BlockAction::COPY_MARKDOWN), &format!("Tab: table copy as Markdown ({:?})", rt.focused_action()))?;
        press("Enter");
        check(events.borrow().last().is_some_and(|e| e.text == "| a | b, c |\n| --- | --- |\n| 1 | 2 |\n"), &format!("markdown {:?}", events.borrow().last()))?;
        press("Tab");
        check(rt.focused_action().as_deref() == Some(BlockAction::COPY_CSV), "Tab: copy as CSV")?;
        press("space");
        check(events.borrow().last().is_some_and(|e| e.text == "a,\"b, c\"\r\n1,2\r\n"), &format!("csv {:?}", events.borrow().last()))?;
        let p = shot(&win, shots, "transcript-actions-copied.png")?;
        let _ = p;
        // Click on the run action.
        let n = events.borrow().len();
        let (ax, ay, aw, ah) = rt.action_rect(code, "run").ok_or("run rect")?;
        focus::click_at(&*win, ax + aw / 2, ay + ah / 2);
        check(events.borrow().len() == n + 1 && events.borrow()[n].action == "run", "click runs the action")?;
        delete(win);
        Ok(())
    });
    run("transcript: custom link schemes, inline code paths as links, activatable embeds, hosted widget", &|shots| {
        let mut win = AppWindow::new("t", 520, 320);
        let mut body = win.body();
        let before = Button::new("before");
        body.fit(&*before);
        let opened = Rc::new(RefCell::new(Vec::<String>::new()));
        let activated = Rc::new(RefCell::new(Vec::<(String, Option<(i32, i32)>)>::new()));
        let (o, a2) = (opened.clone(), activated.clone());
        let rt = RichText::new()
            .markdown_options(MarkdownOptions::default().with_path_links())
            .on_link(move |u| o.borrow_mut().push(u.to_string()))
            .on_embed_activate(move |id, at| a2.borrow_mut().push((id.to_string(), at)));
        win.end();
        rt.set_message("a", Message::markdown("See [the thread](t3-thread://abc) and `src/lib.rs:12`."));
        rt.set_message("card", Message::embed("card", 60, "Approval needed"));
        rt.set_message("tail", Message::embed("hosted", 50, "Hosted card"));
        let btn = Button::new("Approve").bordered();
        rt.host_widget("hosted", &*btn);
        shot(&win, shots, "transcript-links.png")?;
        let _ = before.clone().take_focus();
        press("Tab");
        press("Tab");
        check(rt.focused_link().as_deref() == Some("t3-thread://abc"), "custom scheme link")?;
        press("Enter");
        press("Tab");
        check(rt.focused_link().as_deref() == Some("file://src/lib.rs:12"), &format!("path link ({:?})", rt.focused_link()))?;
        press("Enter");
        check(*opened.borrow() == ["t3-thread://abc", "file://src/lib.rs:12"], &format!("on_link gets them unchanged: {:?}", opened.borrow()))?;
        press("Tab");
        check(rt.focused_action().as_deref() == Some("embed:card"), &format!("Tab: the embed ({:?})", rt.focused_action()))?;
        press("Enter");
        let (x, y, w, h) = rt.embed_rect("card").ok_or("card rect")?;
        focus::click_at(&*win, x + 10, y + 20);
        check(*activated.borrow() == [("card".to_string(), None), ("card".to_string(), Some((10, 20)))], &format!("activated {:?}", activated.borrow()))?;
        let _ = (w, h);
        let (hx, hy, hw, hh) = rt.embed_rect("hosted").ok_or("hosted rect")?;
        check(btn.visible() && (btn.x(), btn.y(), btn.w(), btn.h()) == (hx, hy, hw, hh), &format!("hosted widget over its embed ({},{} {}x{} vs {hx},{hy} {hw}x{hh})", btn.x(), btn.y(), btn.w(), btn.h()))?;
        conversation(&rt, 30);
        rt.scroll_to(rt.content_height());
        shot(&win, shots, "transcript-hosted-gone.png")?;
        check(!btn.visible(), "hosted widget hidden out of view")?;
        delete(win);
        Ok(())
    });
    run("transcript: images are requested in view, sized placeholders, then pixels; alt text until then", &|shots| {
        let mut win = AppWindow::new("t", 520, 360);
        let asked = Rc::new(RefCell::new(Vec::<String>::new()));
        let a2 = asked.clone();
        let rt = RichText::new().on_image(move |src| a2.borrow_mut().push(src.to_string()));
        win.end();
        rt.set_message("a", Message::markdown("Here is the chart:\n\n![a chart](mem://chart.png)\n\nAfter the image."));
        shot(&win, shots, "transcript-image-alt.png")?;
        check(*asked.borrow() == ["mem://chart.png"], &format!("requested once in view: {:?}", asked.borrow()))?;
        check(rt.text_rect("a chart").is_some(), "alt text shown until resolved")?;
        rt.set_image("mem://chart.png", ImageData::Size { width: 200, height: 100 });
        shot(&win, shots, "transcript-image-size.png")?;
        let (_, _, w, h) = rt.embed_rect("mem://chart.png").ok_or("image rect")?;
        check(h == 100 && w > 200, &format!("placeholder of the image's size ({w}x{h})"))?;
        let mut px = Vec::new();
        for _ in 0..(80 * 40) {
            px.extend_from_slice(&[0x20, 0xc0, 0x40, 0xff]);
        }
        rt.set_image("mem://chart.png", ImageData::Rgba { width: 80, height: 40, pixels: px });
        let p = shot(&win, shots, "transcript-image.png")?;
        let (x, y, _, h) = rt.embed_rect("mem://chart.png").ok_or("image rect")?;
        check(h == 40, &format!("image height {h}"))?;
        let c = pixel(&p, x + 40, y + 20)?;
        // Within 2 levels: Windows GDI rounds blitted colors by one.
        let near = |a: u8, b: u8| a.abs_diff(b) <= 2;
        check(near(c[0], 0x20) && near(c[1], 0xc0) && near(c[2], 0x40), &format!("image pixels drawn: {c:?}"))?;
        check(asked.borrow().len() == 1, "not requested again")?;
        delete(win);
        Ok(())
    });
    run("transcript: find highlights every match, F3 / Shift+F3 move; selection rect and programmatic select", &|shots| {
        let mut win = AppWindow::new("t", 520, 320);
        let sel_changes = Rc::new(RefCell::new(0));
        let s2 = sel_changes.clone();
        let rt = RichText::new().on_selection_change(move || *s2.borrow_mut() += 1);
        win.end();
        rt.set_message("a", Message::markdown("needle one, plain words\n\nanother Needle here"));
        conversation(&rt, 20);
        rt.set_message("z", Message::markdown("last needle"));
        let n = rt.find("needle");
        check(n == 3 && rt.find_current() == Some(0), &format!("count {n}, current {:?}", rt.find_current()))?;
        let p = shot(&win, shots, "transcript-find.png")?;
        let bg = rgb(theme::current().palette.background);
        let (x, y, _, _) = rt.text_rect("Needle").ok_or("second match")?;
        check(pixel(&p, x + 1, y + 1)? != bg, "matches are highlighted")?;
        let (px, py, _, _) = rt.text_rect("plain").ok_or("plain")?;
        check(pixel(&p, px + 1, py + 1)? == bg, "other words are not")?;
        let _ = rt.clone().take_focus();
        press("F3");
        check(rt.find_current() == Some(1), "F3: next")?;
        press("F3");
        shot(&win, shots, "transcript-find-last.png")?;
        check(rt.find_current() == Some(2) && rt.text_rect("last needle").is_some(), "the current match is scrolled into view")?;
        press("Shift+F3");
        check(rt.find_current() == Some(1), "Shift+F3: previous")?;
        rt.push_message_markdown("z", " and one more needle");
        check(rt.find_count() == 4, "new text is searched")?;
        rt.clear_find();
        check(rt.find_count() == 0, "cleared")?;
        // Programmatic selection and its rectangle.
        rt.scroll_to(0);
        let text = rt.plain_text();
        let at = text.find("plain words").ok_or("offset")?;
        rt.select_text(at..at + "plain words".len());
        shot(&win, shots, "transcript-select.png")?;
        check(rt.selected_text() == "plain words", &format!("selected {:?}", rt.selected_text()))?;
        let (sx, sy, sw, sh) = rt.selection_rect().ok_or("selection rect")?;
        let (wx, wy, ww, _) = rt.text_rect("plain").ok_or("plain")?;
        check(sx <= wx && sy <= wy && sx + sw >= wx + ww && sh > 0, &format!("rect covers the selection ({sx},{sy} {sw}x{sh})"))?;
        check(*sel_changes.borrow() >= 1, "selection change reported")?;
        delete(win);
        Ok(())
    });
    run("transcript: bind_model follows a ListModel by key (updates stream, inserts, removals)", &|shots| {
        #[derive(Clone, PartialEq)]
        struct Row {
            id: String,
            md: String,
        }
        let mut win = AppWindow::new("t", 520, 320);
        let rt = RichText::new();
        win.end();
        let rows = ListModel::new(|r: &Row| r.id.clone());
        rows.push(Row { id: "r1".into(), md: "first".into() });
        rt.bind_model(&rows, |r| Message::markdown(&r.md));
        rows.push(Row { id: "r2".into(), md: "second".into() });
        rows.patch(&"r2".to_string(), |r| r.md.push_str(" grows"));
        rows.insert(0, Row { id: "r0".into(), md: "zeroth".into() });
        shot(&win, shots, "transcript-model.png")?;
        check(rt.message_keys() == ["r0", "r1", "r2"], &format!("keys {:?}", rt.message_keys()))?;
        check(rt.plain_text() == "zeroth\nfirst\nsecond grows", &format!("text {:?}", rt.plain_text()))?;
        rows.remove_key(&"r1".to_string());
        rows.move_key(&"r0".to_string(), 1);
        check(rt.message_keys() == ["r2", "r0"], &format!("after remove/move {:?}", rt.message_keys()))?;
        delete(win);
        Ok(())
    });
}

#[cfg(feature = "a11y")]
pub fn a11y_checks(run: &mut dyn FnMut(&str, Check)) {
    use fltk_omarchy::a11y::{self, AccessibleExt};
    run("a11y: transcript embeds, images and block actions are nodes", &|shots| {
        let mut win = AppWindow::new("t", 560, 420);
        let rt = RichText::new()
            .accessible_name("Conversation")
            .code_actions(|_, _| vec![BlockAction::new("run", "Run in terminal")])
            .table_actions(true)
            .on_embed_activate(|_, _| {})
            .on_image(|_| {});
        win.end();
        rt.set_message("card", Message::embed("c", 60, "Ran 3 commands"));
        rt.set_message("a", Message::markdown("![a chart](mem://x.png)\n\n```sh\nls\n```\n\n| a |\n|---|\n| 1 |\n"));
        rt.set_image("mem://x.png", ImageData::Size { width: 100, height: 50 });
        shot(&win, shots, "a11y-transcript.png")?;
        let snap = a11y::snapshot(&*win).ok_or("snapshot")?;
        std::fs::write(shots.join("a11y-transcript.txt"), &snap).map_err(|e| e.to_string())?;
        for want in ["Group \"Ran 3 commands\"", "Image \"a chart\"", "Button \"Run in terminal\"", "Button \"Copy\"", "Button \"Copy as Markdown\"", "Button \"Copy as CSV\""] {
            check(snap.contains(want), &format!("tree has {want:?}:\n{snap}"))?;
        }
        delete(win);
        Ok(())
    });
}
