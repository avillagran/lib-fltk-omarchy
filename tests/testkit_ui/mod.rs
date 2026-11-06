//! UI checks of `testkit`: the element tree, scripted input (in process
//! and through the env runner of an example app), the motion gate on a
//! deliberately jumping widget, layout motion, RichText prepend /
//! streaming / follow and NavTree row arrival, the frame log and the
//! blocked-UI warning.
use super::{check, pump_until};
use fltk::prelude::*;
use fltk_omarchy::anim::{self, clock};
use fltk_omarchy::layout::{self, FlexExt};
use fltk_omarchy::list::{List, ListRow};
use fltk_omarchy::model::ListModel;
use fltk_omarchy::navtree::{NavItem, NavTree};
use fltk_omarchy::testkit::{self, FindingKind, Gate, Rules};
use fltk_omarchy::text::{Message, RichText};
use fltk_omarchy::widgets::*;
use fltk_omarchy::{AppWindow, State};
use std::path::Path;
use std::time::Duration;

type Check<'a> = &'a dyn Fn(&Path) -> Result<(), String>;

fn delete(win: AppWindow) {
    fltk::window::DoubleWindow::delete((*win).clone());
}

/// Animations on, frozen clock; back to the test defaults afterwards.
struct Motion;
impl Motion {
    fn on() -> Motion {
        anim::set_enabled(true);
        clock::freeze();
        Motion
    }
}
impl Drop for Motion {
    fn drop(&mut self) {
        clock::resume();
        anim::set_enabled(false);
    }
}

/// A small app: a field, a counter button, a label, a list and a
/// Mod+K shortcut that changes a label.
struct Demo {
    win: AppWindow,
    name: State<String>,
    count: State<usize>,
    list: List,
}

fn demo() -> Demo {
    testkit::arm();
    let mut win = AppWindow::new("Demo", 420, 420);
    win.header().action(glyph::SETTINGS, "Preferences", || {});
    let mut body = win.body();
    let name = State::new(String::new());
    body.fit(&*TextField::new().placeholder("Your name").bind(&name));
    let count = State::new(0usize);
    let c2 = count.clone();
    body.fit(&*Button::new("Add").bordered().on_click(move || c2.update(|n| *n += 1)));
    body.fit(&*Label::new("").bind(&count.map(|n| format!("Count {n}"))));
    let palette = State::new("Palette closed".to_string());
    body.fit(&*Label::new("").bind(&palette));
    let rows = State::new(1000usize);
    let list = List::new(&rows, |i| ListRow::new(&format!("Row {i}")));
    body.fit(&*list);
    win.end();
    let p2 = palette.clone();
    win.shortcut("Mod+K", move || p2.set("Palette open".into()));
    Demo { win, name, count, list }
}

pub fn checks(run: &mut dyn FnMut(&str, Check)) {
    run("testkit: tree lists widgets and list rows with names, rects, focus; JSON", &|shots| {
        let d = demo();
        d.win.clone().snapshot(shots.join("testkit-tree.png")).map_err(|e| e.to_string())?;
        let all = testkit::tree(&*d.win);
        let add = testkit::find(&*d.win, "Add").ok_or("no Add")?;
        check(add.kind == "Button" && add.visible && add.rect.2 > 20 && add.rect.1 > 30, &format!("Add button {add:?}"))?;
        let prefs = testkit::find(&*d.win, "Preferences").ok_or("no Preferences (tooltip name)")?;
        check(prefs.visible && prefs.rect.1 < 40, &format!("header action {prefs:?}"))?;
        check(testkit::find(&*d.win, "Count 0").is_some_and(|e| e.visible), "bound label text")?;
        let rows: Vec<_> = all.iter().filter(|e| e.name.starts_with("Row ") && e.key.is_some()).collect();
        check(!rows.is_empty() && rows.len() < 40, &format!("virtual rows near the viewport only ({})", rows.len()))?;
        let r0 = rows[0];
        check(r0.name == "Row 0" && r0.visible && r0.path.contains('#'), &format!("first row {r0:?}"))?;
        let list = all.iter().find(|e| e.path == r0.path.split('#').next().unwrap_or("")).ok_or("row host")?;
        check(r0.rect.1 >= list.rect.1 && r0.rect.1 < list.rect.1 + list.rect.3, "rows inside their list")?;
        let _ = d.list.clone().take_focus();
        let focused: Vec<_> = testkit::tree(&*d.win).into_iter().filter(|e| e.focused && e.key.is_none()).collect();
        check(focused.len() == 1 && focused[0].path == list.path, &format!("focus follows: {focused:?}"))?;
        let json: serde_json::Value = serde_json::from_str(&testkit::tree_json(&*d.win)).map_err(|e| e.to_string())?;
        check(json["window"] == "Demo" && json["elements"].as_array().is_some_and(|a| a.len() == all.len()), "JSON dump")?;
        delete(d.win);
        Ok(())
    });
    run("testkit: a transcript exposes rows near its viewport, inside the view", &|_| {
        testkit::arm();
        let mut win = AppWindow::new("t", 520, 320);
        let rt = RichText::new();
        win.end();
        for k in 0..60 {
            rt.set_message(&format!("m{k}"), Message::markdown(&format!("Message w{k}\n\nwith a second paragraph")));
        }
        rt.scroll_to(rt.content_height() / 2);
        testkit::run_steps(&*win, &[]);
        let all = testkit::tree(&*win);
        let host = all.iter().find(|e| e.kind == "Document").ok_or("no Document")?;
        let prefix = format!("{}#", host.path);
        let rows: Vec<_> = all.iter().filter(|e| e.key.is_some() && e.path.starts_with(&prefix)).collect();
        let shown: Vec<_> = rows.iter().filter(|e| e.visible).collect();
        check(!shown.is_empty() && rows.len() < 400, &format!("{} rows, {} shown", rows.len(), shown.len()))?;
        let (top, bottom) = (host.rect.1, host.rect.1 + host.rect.3);
        check(
            shown.iter().all(|e| e.rect.1 < bottom && e.rect.1 + e.rect.3 > top),
            &format!("shown rows meet the view {top}..{bottom}: {:?}", shown.iter().map(|e| e.rect).collect::<Vec<_>>()),
        )?;
        let key = (0..60).map(|k| format!("m{k}")).find(|k| rt.message_rect(k).is_some()).ok_or("a message in view")?;
        let r = rt.message_rect(&key).ok_or("rect")?;
        check(r.1 < bottom && r.1 + r.3 > top && r.2 == rt.w(), &format!("message_rect {key} {r:?}"))?;
        delete(win);
        Ok(())
    });
    run("testkit: a script types, clicks by name, presses Mod+K, wheels a list and checks; failures are reported", &|shots| {
        let d = demo();
        let shot = shots.join("testkit-script.png");
        let script = serde_json::json!([
            {"click": {"kind": "TextInput"}},
            {"type": "Ada"},
            {"expect": {"kind": "TextInput"}, "value": "Ada", "focused": true},
            {"click": "Add"},
            {"click": "Add"},
            {"expect": "Count 2", "visible": true},
            {"press": "Mod+K"},
            {"expect": {"contains": "Palette"}, "text": "open"},
            {"expect": "Row 0", "visible": true},
            {"wheel": {"kind": "ListBox"}, "dy": 10},
            {"wait_ms": 300},
            {"expect": "Row 0", "visible": false},
            {"expect": "Nothing like this", "visible": false},
            {"snapshot": shot.to_string_lossy()},
        ]);
        let r = testkit::run_script(&*d.win, &script.to_string());
        check(r.ok(), &r.summary())?;
        check(d.name.get() == "Ada" && d.count.get() == 2, &format!("state {:?} {}", d.name.get(), d.count.get()))?;
        check(shot.exists(), "snapshot step wrote the PNG")?;
        let bad = testkit::run_script(&*d.win, r#"[{"expect": "Count 9"}, {"click": "Nope"}, {"press": "Tab"}, {"bogus": 1}]"#);
        let f = bad.failures();
        check(f.len() == 3 && f[0].index == 0 && f[1].message.contains("Nope") && f[2].op == "bogus", &bad.summary())?;
        let v: serde_json::Value = serde_json::from_str(&bad.to_json()).map_err(|e| e.to_string())?;
        check(v["failed"] == 3 && v["passed"] == 1, "JSON report")?;
        delete(d.win);
        Ok(())
    });
    run("testkit: OMARCHY_FLTK_SCRIPT runs an example app headlessly and writes tree and report", &|shots| {
        // cargo test builds the examples next to the test binary's deps dir.
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let name = format!("testkit_demo{}", std::env::consts::EXE_SUFFIX);
        let Some(bin) = exe.parent().and_then(|d| d.parent()).map(|d| d.join("examples").join(&name)).filter(|p| p.exists()) else {
            println!("     (testkit_demo example not built: skipped)");
            return Ok(());
        };
        let dir = shots.join("testkit-env");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let (script, tree, report) = (dir.join("steps.json"), dir.join("tree.json"), dir.join("report.json"));
        std::fs::write(&script, r#"{"steps": [{"click": "Add"}, {"press": "Mod+K"}, {"expect": "Count 1"}, {"expect": "Palette open"}]}"#)
            .map_err(|e| e.to_string())?;
        let out = std::process::Command::new(&bin)
            .env("OMARCHY_FLTK_SCRIPT", &script)
            .env("OMARCHY_FLTK_TREE", &tree)
            .env("OMARCHY_FLTK_REPORT", &report)
            .output()
            .map_err(|e| e.to_string())?;
        let rep = std::fs::read_to_string(&report).unwrap_or_default();
        check(out.status.success(), &format!("exit {:?}: {} {rep}", out.status, String::from_utf8_lossy(&out.stderr)))?;
        let v: serde_json::Value = serde_json::from_str(&rep).map_err(|e| format!("report: {e}"))?;
        check(v["ok"] == true && v["passed"] == 4, &rep)?;
        let t: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&tree).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        let has = |n: &str| t["elements"].as_array().is_some_and(|a| a.iter().any(|e| e["name"] == n));
        check(has("Count 1") && has("Palette open"), "tree after the script")?;
        // A failing expectation exits with status 1.
        std::fs::write(&script, r#"[{"expect": "Count 5"}]"#).map_err(|e| e.to_string())?;
        let st = std::process::Command::new(&bin).env("OMARCHY_FLTK_SCRIPT", &script).env("OMARCHY_FLTK_REPORT", &report).status().map_err(|e| e.to_string())?;
        check(st.code() == Some(1), &format!("failing script exits 1: {st:?}"))
    });
    run("testkit gate: a deliberately jumping widget fails; toolkit layout motion passes", &|_| {
        let _m = Motion::on();
        let mut win = AppWindow::new("t", 360, 260);
        let mut body = win.body();
        let a = Button::new("First").bordered();
        body.fit(&*a);
        let b = Button::new("Second").bordered();
        body.fit(&*b);
        let c = Button::new("Third").bordered();
        body.fit(&*c);
        win.end();
        // Teleport: one frame, 100 px, no transition.
        let mut gate = Gate::new(&*win);
        let jumper = Button::new("Jumper");
        let mut j2 = (*jumper).clone();
        win.clone().add(&*jumper);
        j2.resize(10, 200, 80, 24);
        gate.track_widget("jumper", &*jumper);
        gate.frame();
        gate.frame();
        gate.act("teleport", move || j2.resize(10, 100, 80, 24));
        gate.play(64);
        let f = gate.check(&Rules::default());
        check(f.len() == 1 && f[0].kind == FindingKind::Jump && f[0].track == "jumper", &format!("{f:?}"))?;
        check(gate.verify(&Rules::default()).is_err(), "verify fails")?;
        check(gate.check(&Rules { inputs_excuse: true, ..Rules::default() }).is_empty(), "an excused input passes")?;
        // Blink: hidden for one frame.
        let mut gate = Gate::new(&*win);
        gate.track_widget("blink", &*jumper);
        let (mut h1, mut h2) = ((*jumper).clone(), (*jumper).clone());
        gate.frame();
        gate.act("hide", move || h1.hide());
        gate.act("show", move || h2.show());
        gate.play(32);
        let f = gate.check(&Rules::default());
        check(f.iter().any(|x| x.kind == FindingKind::Pop), &format!("blink pops: {f:?}"))?;
        // The toolkit's own layout motion: Third glides up when Second hides.
        let mut gate = Gate::new(&*win);
        gate.track_widget("third", &*c);
        gate.frame();
        let b2 = b.clone();
        gate.act("hide second", move || layout::set_visible(&*b2, false));
        gate.play(400);
        let s = gate.series("third").ok_or("no track")?;
        let (y0, y1) = (s[0].map(|x| x.rect.1), s.last().copied().flatten().map(|x| x.rect.1));
        check(y0.zip(y1).is_some_and(|(a, b)| b < a), &format!("third moved up: {y0:?} → {y1:?}"))?;
        gate.verify(&Rules::default()).map_err(|e| format!("{e}\n{}", gate.dump("third")))?;
        // The same change with animations off: one big step, caught.
        anim::set_enabled(false);
        let mut gate = Gate::new(&*win);
        gate.track_widget("third", &*c);
        gate.frame();
        gate.frame();
        let b3 = b.clone();
        gate.act("show second", move || layout::set_visible(&*b3, true));
        gate.play(64);
        check(gate.check(&Rules::default()).iter().any(|x| x.kind == FindingKind::Jump), &format!("no motion → jump\n{}", gate.dump("third")))?;
        delete(win);
        Ok(())
    });
    run("testkit gate: RichText streaming, follow_end and prepend keep messages from jumping", &|_| {
        let _m = Motion::on();
        let mut win = AppWindow::new("t", 520, 320);
        let rt = RichText::new().stick_to_bottom(true);
        win.end();
        for k in 0..40 {
            rt.set_message(&format!("m{k}"), Message::markdown(&format!("**User** w{k}\n\nA reply with a few words to wrap, number {k}.")));
        }
        let mut gate = Gate::new(&*win);
        gate.track_messages(&rt);
        gate.play(400); // settle at the end
        // Streaming into the last message while following the end.
        let r2 = rt.clone();
        for i in 0..6 {
            let r = r2.clone();
            gate.act("stream", move || r.push_message_markdown("m39", &format!(" more streamed words, part {i}, to grow the tail")));
            gate.play(48);
        }
        gate.play(300);
        // While following, a streamed line scrolls the view by one line at
        // once (no spring growth on the CPU backend, docs/O3-PORT.md §8):
        // allow a line, catch anything bigger (a re-layout bump).
        let line = rt.text_rect("w39").map_or(20, |r| r.3);
        gate.verify(&Rules { max_jump: line + 2, ..Rules::default() }).map_err(|e| format!("streaming: {e}"))?;
        let rules = Rules::default();
        check(rt.is_at_bottom(), "still at the end")?;
        // Scrolled to the middle, older history is prepended: what is in view stays.
        let mut gate = Gate::new(&*win);
        gate.track_messages(&rt);
        rt.scroll_to(rt.content_height() / 2);
        gate.play(300);
        let r = rt.clone();
        gate.act("prepend", move || {
            r.prepend_messages((0..10).map(|k| (format!("old{k}"), Message::markdown(&format!("Older message o{k}\n\nwith a second paragraph")))))
        });
        gate.play(300);
        gate.verify(&rules).map_err(|e| format!("prepend: {e}"))?;
        let moved: Vec<String> = gate
            .tracks()
            .iter()
            .filter(|t| t.starts_with("msg:m"))
            .filter(|t| {
                let s = gate.series(t).unwrap_or(&[]);
                let shown: Vec<i32> = s.iter().flatten().map(|x| x.rect.1).collect();
                shown.windows(2).any(|w| w[0] != w[1])
            })
            .cloned()
            .collect();
        check(moved.is_empty(), &format!("messages in view did not move at all: {moved:?}"))?;
        // Follow the end again from the middle: a cut, then a glide.
        let mut gate = Gate::new(&*win);
        gate.track_messages(&rt);
        gate.frame();
        let r = rt.clone();
        gate.act("follow_end", move || r.follow_end());
        gate.play(600);
        gate.verify(&rules).map_err(|e| format!("follow: {e}"))?;
        check(rt.is_at_bottom(), "follow_end reaches the end")?;
        delete(win);
        Ok(())
    });
    run("testkit gate: NavTree rows below an arriving row glide down without jumps or pops", &|_| {
        let _m = Motion::on();
        let model = ListModel::new(|t: &NavItem| t.key.clone());
        model.extend((0..8).map(|k| NavItem::row(&format!("r{k}"), glyph::FILE, &format!("Row {k}"))));
        let selected = State::new(String::new());
        let mut win = AppWindow::new("NavTree", 520, 420);
        let mut body = win.body();
        body.set_margin(0);
        let mut row = layout::row();
        let tree = NavTree::new(&model, &selected).widths(44, 300);
        row.fit(&*tree);
        row.end();
        win.end();
        tree.set_pinned(true);
        let mut gate = Gate::new(&*win);
        let keys = ["r0", "r1", "r2", "r3", "r4", "r5", "r6", "r7", "new"];
        gate.track_items(&tree, &keys);
        gate.play(400);
        let before = tree.item_rect("r5").ok_or("no r5")?;
        let m = model.clone();
        gate.act("insert", move || m.insert(3, NavItem::row("new", glyph::FILE, "Arrived")));
        gate.play(500);
        let after = tree.item_rect("r5").ok_or("no r5 after")?;
        check(after.1 > before.1, &format!("rows below moved down ({before:?} → {after:?})"))?;
        check(gate.series("item:new").is_some_and(|s| s.last().is_some_and(|x| x.is_some())), "the new row is tracked")?;
        gate.verify(&Rules::default()).map_err(|e| format!("{e}\n{}", gate.dump("item:r5")))?;
        delete(win);
        Ok(())
    });
    run("testkit: frame log writes one JSON line per painted frame; a sleeping click warns", &|shots| {
        let log = shots.join("testkit-frames.jsonl");
        testkit::frames::log_to(Some(&log)).map_err(|e| e.to_string())?;
        let mut win = AppWindow::new("Frames", 300, 200);
        let mut body = win.body();
        let b = Button::new("Slow").on_click(|| std::thread::sleep(Duration::from_millis(80)));
        body.fit(&*b);
        win.end();
        win.show();
        let w2 = win.clone();
        pump_until(|| w2.shown());
        for _ in 0..10 {
            win.redraw();
            let _ = fltk::app::wait_for(0.02);
            fltk_omarchy::App::pump();
        }
        testkit::frames::log_to(None).map_err(|e| e.to_string())?;
        let text = std::fs::read_to_string(&log).map_err(|e| e.to_string())?;
        let lines: Vec<serde_json::Value> = text.lines().filter_map(|l| serde_json::from_str(l).ok()).collect();
        check(!lines.is_empty(), "frames logged")?;
        let l = &lines[lines.len() - 1];
        check(
            l["window"] == "Frames" && l["paint_ms"].is_number() && l["damage"].is_number() && l["size"][0] == 300,
            &format!("fields: {l}"),
        )?;
        check(lines.len() < 2 || lines[1]["wall_ms"].is_number(), "frame interval after the first frame")?;
        testkit::frames::set_block_warnings(true);
        b.click();
        let blocked = testkit::frames::blocked();
        if fltk_omarchy::platform::thread_cpu_time().is_some() {
            check(blocked.iter().any(|x| x.site.contains("testkit_ui")), &format!("blocked click warned: {blocked:?}"))?;
        }
        win.hide();
        delete(win);
        Ok(())
    });
}
