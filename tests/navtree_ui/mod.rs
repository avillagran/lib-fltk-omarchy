//! UI checks of `NavTree`: an o3-style tree (hosts → projects → threads →
//! sub-threads) with badges, actions, search, reordering, live model
//! updates and pause, 5 000 rows, reveal, the resizable edge, presence,
//! side, header widgets in push and overlay, custom rows and a11y.
use super::{check, png_rgb, pump_until};
use fltk::prelude::*;
use fltk_omarchy::anim::{self, clock};
use fltk_omarchy::focus::{self, press};
use fltk_omarchy::layout::{self, FlexExt};
use fltk_omarchy::model::ListModel;
use fltk_omarchy::navtree::*;
use fltk_omarchy::theme;
use fltk_omarchy::widgets::{glyph, Button};
use fltk_omarchy::{i18n, AppWindow, State};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

type Check<'a> = &'a dyn Fn(&Path) -> Result<(), String>;

fn shot(win: &AppWindow, dir: &Path, name: &str) -> Result<PathBuf, String> {
    let p = dir.join(name);
    win.clone().snapshot(&p).map_err(|e| e.to_string())?;
    Ok(p)
}

fn delete(win: AppWindow) {
    fltk::window::DoubleWindow::delete((*win).clone());
}

fn ms(n: u64) {
    clock::advance(Duration::from_millis(n));
}

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

fn rgb(c: theme::Rgb) -> [u8; 3] {
    [c.r(), c.g(), c.b()]
}

fn near(a: [u8; 3], b: [u8; 3]) -> bool {
    a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= 2)
}

fn count(p: &Path, (x, y, w, h): (i32, i32, i32, i32), color: [u8; 3]) -> Result<usize, String> {
    let img = png_rgb(p)?;
    let mut n = 0;
    for yy in y.max(0)..y + h {
        for xx in x.max(0)..x + w {
            if near(img.at(xx, yy), color) {
                n += 1;
            }
        }
    }
    Ok(n)
}

fn center(r: (i32, i32, i32, i32)) -> (i32, i32) {
    (r.0 + r.2 / 2, r.1 + r.3 / 2)
}

/// The center of a row's last trailing action, and the one before it.
fn last_action(r: (i32, i32, i32, i32)) -> (i32, i32) {
    (
        r.0 + r.2 - 18,
        r.1 + theme::with(|t| (t.spacing.control_height + t.spacing.md) / 2),
    )
}

fn prev_action(r: (i32, i32, i32, i32)) -> (i32, i32) {
    let (x, y) = last_action(r);
    (
        x - theme::with(|t| t.spacing.control_height - 2 * t.spacing.xs),
        y,
    )
}

/// Hosts → projects → threads → sub-threads, as o3 shows them.
pub fn o3_items() -> Vec<NavItem> {
    let archive = || RowAction::new("archive", glyph::CHECK, "Archive").on_hover();
    let pin = || RowAction::new("pin", glyph::BOOKMARK, "Pin").on_hover();
    vec![
        NavItem::section("h:local", "This computer")
            .collapsible()
            .count("6")
            .action(RowAction::new("new", glyph::PLUS, "New thread")),
        NavItem::row("p:omarchy", glyph::FOLDER, "omarchy").parent("h:local"),
        NavItem::row("t:bar", glyph::FILE, "Fix the bar")
            .parent("p:omarchy")
            .badge(Badge::pill("PR #42", ColorRole::Green))
            .badge(Badge::dot(ColorRole::Yellow))
            .action(archive())
            .action(pin()),
        NavItem::row("t:bar/icons", glyph::FILE, "Explore icons")
            .parent("t:bar")
            .busy(true),
        NavItem::row("t:bar/tests", glyph::FILE, "Write tests").parent("t:bar"),
        NavItem::row("t:theme", glyph::FILE, "Theme picker")
            .parent("p:omarchy")
            .attention(true)
            .detail("Waiting for your answer")
            .action(archive()),
        NavItem::row("p:t3", glyph::FOLDER, "t3-code")
            .parent("h:local")
            .collapsed()
            .summary("+2 threads"),
        NavItem::row("t:sync", glyph::FILE, "Sync engine").parent("p:t3"),
        NavItem::row("t:rank", glyph::FILE, "Search ranking").parent("p:t3"),
        NavItem::section("h:remote", "build-box")
            .collapsible()
            .count("1"),
        NavItem::row("p:fltk", glyph::FOLDER, "lib-fltk-omarchy").parent("h:remote"),
        NavItem::row("t:nav", glyph::FILE, "NavTree")
            .parent("p:fltk")
            .active(true)
            .shortcut("Ctrl+1")
            .badge(Badge::pill("failed", ColorRole::Red)),
        NavItem::row("settings", glyph::SETTINGS, "Settings").footer(),
    ]
}

fn o3_model() -> ListModel<String, NavItem> {
    let m = ListModel::new(|t: &NavItem| t.key.clone());
    m.extend(o3_items());
    m
}

struct Shell {
    win: AppWindow,
    tree: NavTree,
    model: ListModel<String, NavItem>,
    selected: State<String>,
    content: Button,
}

fn shell(model: ListModel<String, NavItem>, h: i32, f: impl FnOnce(NavTree) -> NavTree) -> Shell {
    let mut win = AppWindow::new("NavTree", 640, h);
    let mut body = win.body();
    body.set_margin(0);
    let selected = State::new(String::new());
    let mut row = layout::row();
    let tree = f(NavTree::new(&model, &selected).widths(44, 300));
    row.fit(&*tree);
    let mut col = layout::column();
    let content = Button::new("Content").bordered();
    col.fit(&*content);
    col.end();
    row.end();
    win.end();
    Shell {
        win,
        tree,
        model,
        selected,
        content,
    }
}

type Log = Rc<RefCell<Vec<String>>>;

fn wire(tree: NavTree) -> (NavTree, Log) {
    let log: Log = Rc::default();
    let (a, b, c, d, e) = (
        log.clone(),
        log.clone(),
        log.clone(),
        log.clone(),
        log.clone(),
    );
    let t = tree
        .on_activate(move |k| a.borrow_mut().push(format!("activate {k}")))
        .on_action(move |k, id| b.borrow_mut().push(format!("action {k} {id}")))
        .on_context(move |k, _, _| c.borrow_mut().push(format!("context {k}")))
        .on_toggle(move |k, on| d.borrow_mut().push(format!("toggle {k} {on}")))
        .on_move(move |k, p, i| e.borrow_mut().push(format!("move {k} {p} {i}")));
    (t, log)
}

fn take(log: &Log) -> Vec<String> {
    std::mem::take(&mut *log.borrow_mut())
}

fn keys(t: &NavTree) -> Vec<String> {
    t.visible_keys()
}

pub fn checks(run: &mut dyn FnMut(&str, Check)) {
    run("navtree: o3 tree order, folded parent summary, tree keys (Right opens / first child, Left closes / parent), type-ahead, activate", &|shots| {
        anim::set_enabled(false);
        let mut log = Log::default();
        let s = shell(o3_model(), 460, |t| {
            let (t, l) = wire(t);
            log = l;
            t
        });
        s.tree.set_pinned(true);
        let p = shot(&s.win, shots, "navtree-o3-0.png")?;
        let want = ["h:local", "p:omarchy", "t:bar", "t:bar/icons", "t:bar/tests", "t:theme", "p:t3", "h:remote", "p:fltk", "t:nav", "settings"];
        check(keys(&s.tree) == want, &format!("order {:?}", keys(&s.tree)))?;
        check(s.tree.is_collapsed("p:t3"), "collapsed() folds on first sight")?;
        // Indentation: sub-threads right of threads.
        let (a, b) = (s.tree.item_rect("t:bar").ok_or("no t:bar")?, s.tree.item_rect("t:bar/icons").ok_or("no sub")?);
        check(a.0 == b.0 && a.2 == b.2, "rows span the width")?;
        let green = theme::with(|t| rgb(ColorRole::Green.rgb(t).over(t.palette.foreground.over(t.palette.background, 0.04), 0.18)));
        check(count(&p, a, green)? > 40, "PR pill on a soft fill of its role")?;
        let _ = s.tree.rows_widget().take_focus();
        s.tree.set_cursor("t:bar/icons");
        press("Left");
        check(s.tree.cursor().as_deref() == Some("t:bar"), &format!("Left: parent {:?}", s.tree.cursor()))?;
        press("Left");
        check(s.tree.is_collapsed("t:bar") && take(&log) == ["toggle t:bar true"], "Left closes")?;
        press("Right");
        check(!s.tree.is_collapsed("t:bar"), "Right opens")?;
        press("Right");
        check(s.tree.cursor().as_deref() == Some("t:bar/icons"), "Right again: first child")?;
        take(&log);
        press("Down");
        press("Down");
        press("Down");
        check(s.tree.cursor().as_deref() == Some("p:t3"), &format!("down to t3 {:?}", s.tree.cursor()))?;
        press("Right");
        check(keys(&s.tree).contains(&"t:rank".to_string()), "t3 opens")?;
        press("Home");
        check(s.tree.cursor().as_deref() == Some("h:local"), "Home: the section (foldable)")?;
        press("s");
        press("e");
        check(s.tree.cursor().as_deref() == Some("t:rank"), &format!("type-ahead 'se' → Search ranking: {:?}", s.tree.cursor()))?;
        take(&log);
        press("Enter");
        check(take(&log) == ["activate t:rank"] && s.selected.get() == "t:rank", "Enter activates")?;
        press("End");
        check(s.tree.cursor().as_deref() == Some("settings"), "End: footer")?;
        delete(s.win);
        Ok(())
    });
    run("navtree: hover-only actions, section '+' action, Delete = first action, Menu key, context and the menu helper", &|shots| {
        anim::set_enabled(false);
        let mut log = Log::default();
        let s = shell(o3_model(), 460, |t| {
            let (t, l) = wire(t);
            log = l;
            t
        });
        s.tree.set_pinned(true);
        shot(&s.win, shots, "navtree-actions-0.png")?;
        let bar = s.tree.item_rect("t:bar").ok_or("no t:bar")?;
        let (x, y) = last_action(bar);
        focus::hover_at(&*s.win, x, y);
        check(s.tree.rows_widget().tooltip().as_deref() == Some("Pin"), &format!("tooltip {:?}", s.tree.rows_widget().tooltip()))?;
        focus::click_at(&*s.win, x, y);
        let (px, py) = prev_action(bar);
        focus::hover_at(&*s.win, px, py);
        focus::click_at(&*s.win, px, py);
        check(take(&log) == ["action t:bar pin", "action t:bar archive"], "actions by id")?;
        let sec = s.tree.item_rect("h:local").ok_or("no section")?;
        let (x, _) = last_action(sec);
        focus::click_at(&*s.win, x, sec.1 + sec.3 / 2);
        check(take(&log) == ["action h:local new"], "section header action")?;
        let _ = s.tree.rows_widget().take_focus();
        s.tree.set_cursor("t:theme");
        press("Delete");
        press("Menu");
        check(take(&log) == ["action t:theme archive", "context t:theme"], "Delete + Menu")?;
        let picked = Rc::new(RefCell::new(None));
        let p2 = picked.clone();
        let r = s.tree.item_rect("t:theme").ok_or("no row")?;
        let h = s.tree.open_menu(r.0 + 10, r.1 + r.3, vec![fltk_omarchy::popups::PopupItem::new("", "Rename"), fltk_omarchy::popups::PopupItem::new("", "Archive")], move |i| *p2.borrow_mut() = Some(i));
        shot(&s.win, shots, "navtree-actions-1-menu.png")?;
        press("Down");
        press("Enter");
        check(*picked.borrow() == Some(1) && !h.is_open(), &format!("menu picked {:?}", picked.borrow()))?;
        delete(s.win);
        Ok(())
    });
    run("navtree: search filters and ranks, highlights matches, keeps ancestors, opens folded parents; Down from the field, Enter opens the first", &|shots| {
        anim::set_enabled(false);
        let mut log = Log::default();
        let s = shell(o3_model(), 460, |t| {
            let (t, l) = wire(t.search("Search threads"));
            log = l;
            t
        });
        s.tree.set_pinned(true);
        shot(&s.win, shots, "navtree-search-0.png")?;
        s.tree.set_query("sync");
        let p = shot(&s.win, shots, "navtree-search-1.png")?;
        check(keys(&s.tree) == ["h:local", "p:t3", "t:sync"], &format!("filtered {:?}", keys(&s.tree)))?;
        let r = s.tree.item_rect("t:sync").ok_or("no t:sync")?;
        let hl = theme::with(|t| {
            let under = t.palette.foreground.over(t.palette.background, 0.04);
            rgb(t.palette.accent.over(under, 0.3))
        });
        check(count(&p, r, hl)? > 20, "match highlighted")?;
        s.tree.set_query("t");
        let k = keys(&s.tree);
        // Word starts before inner matches: "Theme picker", "t3-code"... ordered by score.
        check(k.first().map(String::as_str) == Some("h:local") && k.contains(&"t:theme".to_string()), &format!("ranked {k:?}"))?;
        s.tree.set_query("");
        check(keys(&s.tree).len() == 11 && s.tree.is_collapsed("p:t3"), "cleared: folded state back")?;
        // The built-in field.
        s.tree.focus_search();
        focus::type_text("ranking");
        check(s.tree.query() == "ranking" && keys(&s.tree) == ["h:local", "p:t3", "t:rank"], &format!("typed: {:?} {:?}", s.tree.query(), keys(&s.tree)))?;
        press("Down");
        check(focus::is_focused(&s.tree.rows_widget()) && s.tree.cursor().as_deref() == Some("h:local"), "Down: to the results")?;
        s.tree.focus_search();
        take(&log);
        press("Enter");
        let got = take(&log);
        check(got == ["activate t:rank"], &format!("Enter in the field opens the best match: {got:?} (query {:?})", s.tree.query()))?;
        delete(s.win);
        Ok(())
    });
    run(
        "navtree: drag to reorder among siblings and into another parent, drop line, accept filter",
        &|shots| {
            anim::set_enabled(false);
            let mut log = Log::default();
            let s = shell(o3_model(), 460, |t| {
                let (t, l) = wire(t.reorder(Reorder::Anywhere).accept_move(|k, parent, _| {
                    !(k.starts_with("t:") && parent.starts_with("h:"))
                }));
                log = l;
                t
            });
            s.tree.set_pinned(true);
            shot(&s.win, shots, "navtree-drag-0.png")?;
            let theme_r = s.tree.item_rect("t:theme").ok_or("no t:theme")?;
            let bar = s.tree.item_rect("t:bar").ok_or("no t:bar")?;
            // t:theme above t:bar (same parent): upper part of t:bar.
            focus::mouse_drag(&*s.win, center(theme_r), (bar.0 + 60, bar.1 + 4), false);
            let p = shot(&s.win, shots, "navtree-drag-1-line.png")?;
            check(
                s.tree.drop_spot() == Some(("t:bar".into(), "before")),
                &format!("spot {:?}", s.tree.drop_spot()),
            )?;
            let accent = theme::with(|t| rgb(t.palette.accent));
            check(
                count(&p, (bar.0, bar.1 - 3, bar.2, 6), accent)? > 50,
                "drop line drawn",
            )?;
            focus::mouse_up(&*s.win, bar.0 + 60, bar.1 + 4);
            let got = take(&log);
            check(
                got.last().map(String::as_str) == Some("move t:theme p:omarchy 0"),
                &format!("moved {got:?}"),
            )?;
            // Into another project (middle of its row).
            let fltk_r = s.tree.item_rect("p:fltk").ok_or("no p:fltk")?;
            focus::mouse_drag(&*s.win, center(theme_r), center(fltk_r), false);
            check(
                s.tree.drop_spot() == Some(("p:fltk".into(), "into"))
                    && s.tree.drop_target().as_deref() == Some("p:fltk"),
                &format!("into {:?}", s.tree.drop_spot()),
            )?;
            focus::mouse_up(&*s.win, fltk_r.0 + 60, fltk_r.1 + fltk_r.3 / 2);
            check(
                take(&log).last().map(String::as_str) == Some("move t:theme p:fltk 1"),
                "into p:fltk, last",
            )?;
            // Refused: a thread straight under a host.
            let host = s.tree.item_rect("h:remote").ok_or("no host")?;
            focus::mouse_drag(&*s.win, center(theme_r), center(host), false);
            check(
                s.tree.drop_spot().is_none(),
                &format!("refused {:?}", s.tree.drop_spot()),
            )?;
            focus::mouse_up(&*s.win, host.0 + 60, host.1 + host.3 / 2);
            check(
                !take(&log).iter().any(|l| l.starts_with("move")),
                "nothing moved",
            )?;
            // The app applies a move through the model: the row follows.
            move_item(&s.model, "t:theme", "p:omarchy", 0);
            let k = keys(&s.tree);
            check(
                k.iter().position(|k| k == "t:theme") == Some(2) && k[3] == "t:bar",
                &format!("before t:bar {k:?}"),
            )?;
            move_item(&s.model, "t:theme", "p:fltk", 1);
            let k = keys(&s.tree);
            check(
                k.last().map(String::as_str) == Some("settings")
                    && k[k.len() - 2] == "t:theme"
                    && k[k.len() - 3] == "t:nav",
                &format!("last in p:fltk {k:?}"),
            )?;
            delete(s.win);
            Ok(())
        },
    );
    run("navtree: live updates repaint only rows in view, keep selection and the first row in view; pause holds them", &|shots| {
        anim::set_enabled(false);
        let model = ListModel::new(|t: &NavItem| t.key.clone());
        model.extend((0..200).map(|i| NavItem::row(&format!("r{i}"), glyph::FILE, &format!("Row {i}"))));
        let s = shell(model.clone(), 400, |t| t);
        s.tree.set_pinned(true);
        s.selected.set("r10".into());
        shot(&s.win, shots, "navtree-live-0.png")?;
        let mut canvas = s.tree.rows_widget();
        canvas.clear_damage();
        model.patch(&"r150".to_string(), |t| t.title = "Out of view".into());
        check(!canvas.damage(), "a change out of view: no repaint")?;
        model.patch(&"r2".to_string(), |t| t.title = "Row two".into());
        check(canvas.damage(), "a change in view repaints")?;
        // Scroll, then insert above: the first row in view stays.
        s.tree.scroll_to("r60");
        shot(&s.win, shots, "navtree-live-1.png")?;
        let before = s.tree.item_rect("r60").ok_or("no r60")?;
        model.insert(0, NavItem::row("new0", glyph::FILE, "New 0"));
        model.insert(0, NavItem::row("new1", glyph::FILE, "New 1"));
        shot(&s.win, shots, "navtree-live-2.png")?;
        let after = s.tree.item_rect("r60").ok_or("no r60 after")?;
        check(before.1 == after.1, &format!("anchored: {} → {}", before.1, after.1))?;
        check(s.selected.get() == "r10", "selection by key")?;
        // Paused: rows stay as they are, then everything at once.
        s.tree.pause_updates(true);
        model.remove_key(&"r61".to_string());
        model.insert(5, NavItem::row("held", glyph::FILE, "Held"));
        check(!keys(&s.tree).contains(&"held".to_string()) && keys(&s.tree).contains(&"r61".to_string()), "held while paused")?;
        s.tree.pause_updates(false);
        check(keys(&s.tree).contains(&"held".to_string()) && !keys(&s.tree).contains(&"r61".to_string()), "applied on resume")?;
        // sync_from (snapshot protocols) keeps folded state and selection.
        model.sync_from((0..200).rev().map(|i| NavItem::row(&format!("r{i}"), glyph::FILE, &format!("Row {i}"))));
        check(keys(&s.tree)[0] == "r199" && s.selected.get() == "r10", "sync_from")?;
        delete(s.win);
        Ok(())
    });
    run("navtree: 5 000 rows: rebuild, draw and live patches stay bounded, only rows in view are drawn", &|shots| {
        anim::set_enabled(false);
        let model = ListModel::new(|t: &NavItem| t.key.clone());
        let mut items = Vec::new();
        for p in 0..50 {
            items.push(NavItem::section(&format!("h{p}"), &format!("Host {p}")).collapsible().count("99"));
            for t in 0..99 {
                let mut it = NavItem::row(&format!("t{p}.{t}"), glyph::FILE, &format!("Thread {p}.{t}")).parent(&format!("h{p}"));
                if t % 7 == 0 {
                    it = it.detail("Working").badge(Badge::spinner());
                }
                items.push(it);
            }
        }
        model.extend(items);
        check(model.len() == 5000, "5000 items")?;
        let s = shell(model.clone(), 500, |t| t);
        s.tree.set_pinned(true);
        let t0 = Instant::now();
        let n = keys(&s.tree).len();
        let rebuild = t0.elapsed();
        let t1 = Instant::now();
        shot(&s.win, shots, "navtree-5000.png")?;
        let first = t1.elapsed();
        check(n == 5000 && s.tree.rows_drawn() <= 30, &format!("{n} rows, {} drawn", s.tree.rows_drawn()))?;
        // 60 live patches (a frame's worth each), half of them in view.
        let t2 = Instant::now();
        for i in 0..60 {
            let k = if i % 2 == 0 { format!("t0.{}", i % 5) } else { format!("t30.{}", i) };
            model.patch(&k, |t| t.detail = format!("step {i}"));
        }
        let patches = t2.elapsed();
        // 20 structural changes, each followed by a rebuild.
        let t3 = Instant::now();
        for i in 0..20 {
            model.insert(1, NavItem::row(&format!("n{i}"), glyph::FILE, "New").parent("h0"));
            let _ = s.tree.scroll_position();
        }
        let structural = t3.elapsed() / 20;
        println!("     5000 rows: rebuild {rebuild:?}, first draw (snapshot) {first:?}, 60 patches {patches:?}, structural change + rebuild {structural:?}");
        check(rebuild < Duration::from_millis(400), &format!("rebuild {rebuild:?}"))?;
        check(patches < Duration::from_millis(200), &format!("patches {patches:?}"))?;
        check(structural < Duration::from_millis(200), &format!("structural {structural:?}"))?;
        s.tree.reveal("t49.98");
        shot(&s.win, shots, "navtree-5000-end.png")?;
        check(s.tree.item_rect("t49.98").is_some() && s.tree.rows_drawn() <= 30, "reveal the last row; still only the view is drawn")?;
        delete(s.win);
        Ok(())
    });
    run(
        "navtree: reveal opens ancestors and scrolls (animated, 120 ms); scroll_to",
        &|shots| {
            anim::set_enabled(false);
            let model = ListModel::new(|t: &NavItem| t.key.clone());
            model.extend(
                (0..60).map(|i| NavItem::row(&format!("r{i}"), glyph::FOLDER, &format!("Row {i}"))),
            );
            model.extend([NavItem::row("deep", glyph::FILE, "Deep").parent("r55")]);
            model.patch(&"r55".to_string(), |t| t.collapsed = true);
            let s = shell(model, 360, |t| t);
            s.tree.set_pinned(true);
            shot(&s.win, shots, "navtree-reveal-0.png")?;
            check(
                s.tree.item_rect("deep").is_none(),
                "hidden in a folded branch",
            )?;
            let _m = Motion::on();
            s.tree.reveal("deep");
            shot(&s.win, shots, "navtree-reveal-1.png")?;
            let (mid, max) = s.tree.scroll_position();
            ms(60);
            shot(&s.win, shots, "navtree-reveal-2.png")?;
            let (mid2, _) = s.tree.scroll_position();
            ms(200);
            shot(&s.win, shots, "navtree-reveal-3.png")?;
            let (end, _) = s.tree.scroll_position();
            check(
                mid < mid2 && mid2 < end && end <= max,
                &format!("scroll glides {mid} → {mid2} → {end}"),
            )?;
            check(
                !s.tree.is_collapsed("r55")
                    && s.tree.item_rect("deep").is_some()
                    && s.tree.cursor().as_deref() == Some("deep"),
                "opened, in view, cursor",
            )?;
            s.tree.scroll_to("r0");
            ms(300);
            shot(&s.win, shots, "navtree-reveal-4.png")?;
            check(s.tree.scroll_position().0 == 0, "scroll_to the top")?;
            delete(s.win);
            Ok(())
        },
    );
    run("navtree: the edge drags the width (min/max, State<i32>), presence hidden/rail/expanded, side Right", &|shots| {
        anim::set_enabled(false);
        let width = State::new(260);
        let s = shell(o3_model(), 400, |t| t.resizable(200, 360).bind_width(&width));
        s.tree.set_pinned(true);
        shot(&s.win, shots, "navtree-resize-0.png")?;
        check(s.tree.w() == 260, &format!("bound width {}", s.tree.w()))?;
        let edge = s.tree.x() + s.tree.w() - 1;
        let y = s.tree.y() + 200;
        focus::mouse_drag(&*s.win, (edge, y), (edge + 60, y), false);
        shot(&s.win, shots, "navtree-resize-1.png")?;
        check(s.tree.w() == 320 && width.get() == 260, &format!("live {} (state {})", s.tree.w(), width.get()))?;
        focus::mouse_up(&*s.win, edge + 60, y);
        check(width.get() == 320, &format!("state on release {}", width.get()))?;
        let edge = s.tree.x() + s.tree.w() - 1;
        focus::mouse_drag(&*s.win, (edge, y), (edge + 200, y), true);
        shot(&s.win, shots, "navtree-resize-2.png")?;
        check(width.get() == 360 && s.tree.w() == 360, "clamped to max")?;
        width.set(220);
        shot(&s.win, shots, "navtree-resize-3.png")?;
        check(s.tree.w() == 220, "the app sets the width")?;
        s.tree.set_presence(Presence::Hidden);
        shot(&s.win, shots, "navtree-presence-hidden.png")?;
        check(!s.tree.visible() && s.content.x() < 10 && s.tree.presence() == Presence::Hidden, &format!("hidden: content at {}", s.content.x()))?;
        s.tree.set_presence(Presence::Rail);
        shot(&s.win, shots, "navtree-presence-rail.png")?;
        check(s.tree.visible() && s.tree.w() == 44 && !s.tree.is_expanded(), "rail")?;
        s.tree.set_presence(Presence::Expanded);
        shot(&s.win, shots, "navtree-presence-expanded.png")?;
        check(s.tree.w() == 220 && s.tree.is_pinned(), "expanded = pinned")?;
        delete(s.win);
        // Side Right: the accent bar on the right edge, in a left-to-right app.
        let mut win = AppWindow::new("Right", 500, 300);
        let mut body = win.body();
        body.set_margin(0);
        let model = o3_model();
        let sel = State::new(String::new());
        let mut row = layout::row();
        let mut col = layout::column();
        col.fit(&*Button::new("Content").bordered());
        col.end();
        model.patch(&"t:nav".to_string(), |t| t.active = false);
        model.patch(&"p:omarchy".to_string(), |t| t.active = true);
        let tree = NavTree::new(&model, &sel).side(Side::Right).widths(44, 260);
        row.fit(&*tree);
        row.end();
        win.end();
        tree.set_pinned(true);
        let p = shot(&win, shots, "navtree-side-right.png")?;
        let r = tree.item_rect("p:omarchy").ok_or("no p:omarchy")?;
        let accent = theme::with(|t| rgb(t.palette.accent));
        let right = count(&p, (r.0 + r.2 - 6, r.1, 6, r.3), accent)?;
        let left = count(&p, (r.0, r.1, 6, r.3), accent)?;
        check(right > 10 && left == 0, &format!("active bar on the right: {right} vs left {left}"))?;
        delete(win);
        Ok(())
    });
    run("navtree: header and footer widgets (search) show when expanded (push) and ride in the overlay panel", &|shots| {
        anim::set_enabled(false);
        let s = shell(o3_model(), 420, |t| t.search("Search").footer_widget(&*Button::new("New thread").bordered()));
        shot(&s.win, shots, "navtree-slots-0-rail.png")?;
        let root = s.tree.clone();
        check(root.w() == 44, "rail")?;
        let canvas = s.tree.rows_widget();
        check(canvas.y() == root.y() && canvas.h() == root.h(), "collapsed: only the rows")?;
        s.tree.expand();
        shot(&s.win, shots, "navtree-slots-1-push.png")?;
        check(canvas.y() > root.y() && canvas.y() + canvas.h() < root.y() + root.h() && root.w() == 300, &format!("expanded: field above, button below ({} {} / {} {})", canvas.y(), canvas.h(), root.y(), root.h()))?;
        delete(s.win);
        let mut win = AppWindow::new("Overlay", 640, 420);
        let mut body = win.body();
        body.set_margin(0);
        let model = o3_model();
        let sel = State::new(String::new());
        let mut row = layout::row();
        let tree = NavTree::new(&model, &sel).widths(44, 280).mode(SidebarMode::Overlay).search("Search");
        row.fit(&*tree);
        let mut col = layout::column();
        let content = Button::new("Content").bordered();
        col.fit(&*content);
        col.end();
        row.end();
        win.end();
        shot(&win, shots, "navtree-slots-2-overlay-rail.png")?;
        let cx = content.x();
        tree.expand();
        shot(&win, shots, "navtree-slots-3-overlay-open.png")?;
        let field_in_layer = win.children() > 1;
        let r = tree.item_rect("h:local").ok_or("no row")?;
        check(content.x() == cx && tree.overlay_width() == 280 && r.1 > tree.y() + 20 && field_in_layer, &format!("overlay: rows below the field {r:?}"))?;
        tree.focus_search();
        focus::type_text("sync");
        check(tree.query() == "sync", "the field works in the panel")?;
        let _ = tree.rows_widget().take_focus();
        tree.collapse();
        shot(&win, shots, "navtree-slots-4-overlay-closed.png")?;
        pump_until(|| tree.overlay_width() == 0);
        check(tree.overlay_width() == 0 && tree.rows_widget().y() == tree.y(), "closed: the field went back")?;
        delete(win);
        Ok(())
    });
    run("navtree: custom renderer (own height and paint), right to left mirrors and swaps Left/Right", &|shots| {
        anim::set_enabled(false);
        let model = o3_model();
        model.upsert(NavItem::row("card", glyph::BELL, "Card").renderer("card"));
        let painted = Rc::new(RefCell::new(0));
        let p2 = painted.clone();
        let s = shell(model, 460, move |t| {
            t.renderer("card", |_| 60, move |p| {
                *p2.borrow_mut() += 1;
                let (x, y, w, h) = p.rect;
                fltk_omarchy::paint::fill(x + 8, y + 8, w - 16, h - 16, 4, p.theme.palette.green.over(p.under, 0.5));
            })
        });
        s.tree.set_pinned(true);
        let p = shot(&s.win, shots, "navtree-custom.png")?;
        let r = s.tree.item_rect("card").ok_or("no card")?;
        let g = theme::with(|t| rgb(t.palette.green.over(t.palette.foreground.over(t.palette.background, 0.04), 0.5)));
        check(r.3 == 60 && *painted.borrow() > 0 && count(&p, r, g)? > 500, &format!("custom row {r:?}"))?;
        delete(s.win);
        i18n::set_locale("ar");
        let s = shell(o3_model(), 460, |t| t);
        s.tree.set_pinned(true);
        shot(&s.win, shots, "navtree-rtl.png")?;
        let ok = s.tree.x() > s.content.x();
        let _ = s.tree.rows_widget().take_focus();
        s.tree.set_cursor("t:bar");
        press("Right"); // in Arabic: toward the start = close
        let closed = s.tree.is_collapsed("t:bar");
        delete(s.win);
        i18n::set_locale("en");
        check(ok && closed, &format!("rtl: on the right {ok}, Right closes {closed}"))
    });
    run("navtree: o3 cards (82 px, three lines) and slim rows, status text replaced by hover/cursor actions, tree guides, Delete", &|shots| {
        anim::set_enabled(false);
        let mut log = Log::default();
        let s = shell(o3_cards(), 460, |t| {
            let (t, l) = wire(t.tree_guides(true));
            log = l;
            t
        });
        s.tree.set_pinned(true);
        let p = shot(&s.win, shots, "navtree-cards-0.png")?;
        let r1 = s.tree.item_rect("t1").ok_or("no t1")?;
        let r2 = s.tree.item_rect("t2").ok_or("no t2")?;
        let ra = s.tree.item_rect("t1/a").ok_or("no t1/a")?;
        let rh = theme::with(|t| t.spacing.control_height + t.spacing.md);
        check(r1.3 == 82 && r2.3 == 62 && ra.3 == rh, &format!("heights card {} two-line card {} slim {}", r1.3, r2.3, ra.3))?;
        let yellow = theme::with(|t| rgb(ColorRole::Yellow.rgb(t)));
        let line1 = (r1.0, r1.1 + 10, r1.2, 20);
        let status = count_close(&p, line1, yellow, 40)?;
        check(status > 10, &format!("status text in its role on the first line ({status} px)"))?;
        // Tree guide under the card's icon, through the sub-thread row.
        let surface = theme::with(|t| t.palette.foreground.over(t.palette.background, 0.04));
        let guide = theme::with(|t| rgb(t.palette.foreground.over(surface, 0.14)));
        let gx = ra.0 + 20;
        let on_guide = count(&p, (gx, ra.1, 1, ra.3), guide)?;
        check(on_guide >= (ra.3 - 4) as usize, &format!("guide column at {gx}: {on_guide} of {}", ra.3))?;
        // Hover: the actions take the status text's place, on the first line.
        focus::hover_at(&*s.win, r1.0 + r1.2 / 2, r1.1 + r1.3 - 12);
        let p = shot(&s.win, shots, "navtree-cards-1-hover.png")?;
        let status = count_close(&p, line1, yellow, 40)?;
        // A few antialiased pixels may land near the color (Windows GDI).
        check(status < 6, &format!("hovered: no status text ({status} px)"))?;
        let ay = r1.1 + 10 + 10;
        let ax = r1.0 + r1.2 - 12 - 11;
        focus::hover_at(&*s.win, ax, ay);
        check(s.tree.rows_widget().tooltip().as_deref() == Some("Snooze"), &format!("action tooltip {:?}", s.tree.rows_widget().tooltip()))?;
        focus::click_at(&*s.win, ax, ay);
        check(take(&log) == ["action t1 snooze"], "click on the first line's action")?;
        // Keyboard: the cursor row shows its actions too; Delete runs the first.
        focus::hover_at(&*s.win, s.content.x() + 4, s.content.y() + 4);
        let _ = s.tree.rows_widget().take_focus();
        s.tree.set_cursor("t1/a");
        press("Down");
        let p = shot(&s.win, shots, "navtree-cards-2-cursor.png")?;
        let r2l = (r2.0 + r2.2 - 60, r2.1 + 10, 60, 20);
        let ink_t2 = count_close(&p, r2l, theme::with(|t| rgb(t.palette.foreground)), 60)?;
        check(ink_t2 > 5, "cursor row: an action glyph where the time was")?;
        press("Delete");
        check(take(&log) == ["action t2 settle"], "Delete: first action")?;
        delete(s.win);
        Ok(())
    });
    run("navtree: action menu (right-click, Menu, Shift+F10) lists the row's actions without on_context", &|shots| {
        anim::set_enabled(false);
        let log: Log = Rc::default();
        let l2 = log.clone();
        let s = shell(o3_cards(), 460, move |t| {
            t.action_menu(true).on_action(move |k, a| l2.borrow_mut().push(format!("action {k} {a}")))
        });
        s.tree.set_pinned(true);
        let _ = s.tree.rows_widget().take_focus();
        s.tree.set_cursor("t1");
        press("Shift+F10");
        shot(&s.win, shots, "navtree-action-menu.png")?;
        press("Down");
        press("Enter");
        check(take(&log) == ["action t1 snooze"], &format!("picked the second action {:?}", log.borrow()))?;
        let r = s.tree.item_rect("t2").ok_or("no t2")?;
        focus::right_click_at(&*s.win, r.0 + 40, r.1 + r.3 / 2);
        press("Enter");
        check(take(&log) == ["action t2 settle"], "right-click menu")?;
        delete(s.win);
        Ok(())
    });
    run("navtree: hover preview card opens after 500 ms beside the row, stays while on the card, closes 140 ms after leaving, on scroll and on keys", &|shots| {
        anim::set_enabled(false);
        let built: Rc<RefCell<Vec<String>>> = Rc::default();
        let b2 = built.clone();
        let s = shell(o3_cards(), 460, move |t| {
            t.preview(240, move |key, col| {
                b2.borrow_mut().push(key.to_string());
                if key == "t3" {
                    return false;
                }
                col.fit(&*fltk_omarchy::widgets::Label::new(&format!("Preview of {key}")));
                col.fit(&*fltk_omarchy::widgets::Label::new("feature/branch"));
                true
            })
        });
        s.tree.set_pinned(true);
        shot(&s.win, shots, "navtree-preview-0.png")?;
        let r = s.tree.item_rect("t2").ok_or("no t2")?;
        // The pointer rests: hover again on every turn, since the X server
        // of a test run can send a stray Leave for windows of earlier checks.
        let rest = |x: i32, y: i32, until: &dyn Fn() -> bool, max: Duration| {
            let t = Instant::now();
            while !until() && t.elapsed() < max {
                focus::hover_at(&*s.win, x, y);
                fltk_omarchy::App::pump();
                std::thread::sleep(Duration::from_millis(10));
            }
        };
        let t0 = Instant::now();
        rest(r.0 + 60, r.1 + r.3 / 2, &|| s.tree.preview_key().is_some(), Duration::from_secs(3));
        let waited = t0.elapsed();
        check(s.tree.preview_key().as_deref() == Some("t2"), &format!("opened {:?}", s.tree.preview_key()))?;
        check(waited >= Duration::from_millis(450) && waited < Duration::from_millis(1500), &format!("after {waited:?}"))?;
        let pr = s.tree.preview_rect().ok_or("no rect")?;
        check(pr.0 >= s.tree.x() + s.tree.w() && pr.2 == 240 && pr.3 > 30, &format!("beside the sidebar {pr:?}"))?;
        let p = shot(&s.win, shots, "navtree-preview-1-open.png")?;
        let bg = theme::with(|t| rgb(t.palette.background));
        let painted = (pr.3 * pr.2) as usize - count(&p, pr, bg)?;
        check(painted > 200, &format!("the card is drawn ({painted} px)"))?;
        // On the card it stays.
        rest(pr.0 + pr.2 / 2, pr.1 + pr.3 / 2, &|| false, Duration::from_millis(400));
        check(s.tree.preview_key().is_some(), "stays while the pointer is on the card")?;
        // Away from both: closes after 140 ms.
        let t2 = Instant::now();
        rest(pr.0 + pr.2 / 2, pr.1 + pr.3 + 60, &|| s.tree.preview_key().is_none(), Duration::from_secs(3));
        check(s.tree.preview_key().is_none() && t2.elapsed() < Duration::from_millis(1000), &format!("closed after {:?}", t2.elapsed()))?;
        // Scroll and keys close it at once.
        s.tree.show_preview("t2");
        check(s.tree.preview_key().is_some(), "show_preview")?;
        focus::wheel_at(&*s.win, r.0 + 60, r.1 + r.3 / 2, 1);
        check(s.tree.preview_key().is_none(), "scroll closes it")?;
        s.tree.show_preview("t2");
        let _ = s.tree.rows_widget().take_focus();
        press("Escape");
        check(s.tree.preview_key().is_none(), "Esc closes it")?;
        // A row without a preview: nothing opens.
        s.tree.show_preview("t3");
        check(s.tree.preview_key().is_none() && built.borrow().contains(&"t3".to_string()), "build returned false")?;
        // The row going away closes it.
        s.tree.show_preview("t2");
        s.model.remove_key(&"t2".to_string());
        shot(&s.win, shots, "navtree-preview-2-removed.png")?;
        super::pump_until(|| s.tree.preview_key().is_none());
        check(s.tree.preview_key().is_none(), "removed row: closed")?;
        delete(s.win);
        Ok(())
    });
    run("navtree: shelf resizer sets the shelf height (keys ±20, Home resets, drag up grows, range), Slider role", &|shots| {
        anim::set_enabled(false);
        let mut win = AppWindow::new("Shelves", 360, 520);
        let mut body = win.body();
        body.set_margin(0);
        let mut col = layout::column();
        col.set_spacing(0);
        let selected = State::new(String::new());
        let top = NavTree::new(&o3_model(), &selected).widths(44, 360);
        col.add(&*top);
        let h = State::new(160);
        let grip = ShelfResizer::new("Settled threads height", &h, 160).range(60, 300);
        col.fit(&*grip);
        let shelf = NavTree::new(&o3_cards(), &selected).widths(44, 360);
        col.fit(&*shelf);
        let grip = grip.shelf(&*shelf);
        col.end();
        win.end();
        top.set_pinned(true);
        shelf.set_pinned(true);
        shot(&win, shots, "navtree-shelf-0.png")?;
        check(shelf.h() == 160, &format!("initial {}", shelf.h()))?;
        let _ = grip.clone().take_focus();
        press("Up");
        check(h.get() == 180 && shelf.h() == 180, &format!("Up +20: {} {}", h.get(), shelf.h()))?;
        press("Down");
        press("Down");
        check(h.get() == 140, "Down -20")?;
        let p = shot(&win, shots, "navtree-shelf-1-focus.png")?;
        let accent = theme::with(|t| rgb(t.palette.accent));
        let line = count_close(&p, (grip.x(), grip.y(), grip.w(), grip.h()), accent, 40)?;
        check(line > 100, &format!("focused grip shows the accent line ({line} px)"))?;
        press("Home");
        check(h.get() == 160, "Home: default")?;
        let (gx, gy) = (grip.x() + grip.w() / 2, grip.y() + grip.h() / 2);
        focus::mouse_drag(&*win, (gx, gy), (gx, gy - 30), true);
        check(h.get() == 190 && shelf.h() == 190, &format!("drag up 30: {}", h.get()))?;
        focus::mouse_drag(&*win, (gx, grip.y() + 2), (gx, grip.y() - 400), true);
        check(h.get() == 300, "clamped to the range")?;
        delete(win);
        Ok(())
    });
}

/// o3's thread cards: a working card with meta and a sub-thread, a
/// two-line card, a slim settled row.
pub fn o3_cards() -> ListModel<String, NavItem> {
    let settle = || RowAction::new("settle", glyph::CHECK, "Settle").on_hover();
    let snooze = || RowAction::new("snooze", glyph::BELL, "Snooze").on_hover();
    let m = ListModel::new(|t: &NavItem| t.key.clone());
    m.extend([
        NavItem::section("s:active", "Active").count("3"),
        NavItem::row("t1", glyph::FOLDER, "Fix the bar")
            .parent("s:active")
            .card()
            .overline("omarchy")
            .status(glyph::LOADING, "Working 2m", ColorRole::Yellow)
            .badge(Badge::icon(glyph::BOOKMARK, "Pinned", ColorRole::Dim))
            .meta(Meta::new(glyph::FOLDER, "feature/a-very-long-branch-name-for-the-bar"))
            .meta(Meta::new(glyph::MONITOR, ""))
            .meta(Meta::new(glyph::SETTINGS, ""))
            .action(settle())
            .action(snooze()),
        NavItem::row("t1/a", glyph::LOADING, "Explore icons")
            .parent("t1")
            .trailing("3m")
            .action(settle()),
        NavItem::row("t2", glyph::FOLDER, "Theme picker")
            .parent("s:active")
            .card()
            .overline("omarchy")
            .trailing("1h")
            .note("5m")
            .action(settle()),
        NavItem::row("t3", glyph::FILE, "Old thread")
            .parent("s:active")
            .trailing("2d")
            .dimmed(),
    ]);
    m
}

fn close(a: [u8; 3], b: [u8; 3], tol: u8) -> bool {
    a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= tol)
}

fn count_close(p: &Path, (x, y, w, h): (i32, i32, i32, i32), color: [u8; 3], tol: u8) -> Result<usize, String> {
    let img = png_rgb(p)?;
    let mut n = 0;
    for yy in y.max(0)..y + h {
        for xx in x.max(0)..x + w {
            if close(img.at(xx, yy), color, tol) {
                n += 1;
            }
        }
    }
    Ok(n)
}

#[cfg(feature = "a11y")]
pub fn a11y_checks(run: &mut dyn FnMut(&str, Check)) {
    run("a11y: o3 cards describe overline, status, note, meta and icon badges; shelf resizer is a named Slider", &|shots| {
        anim::set_enabled(false);
        let mut win = AppWindow::new("Shelves", 360, 520);
        let mut body = win.body();
        body.set_margin(0);
        let mut col = layout::column();
        let selected = State::new(String::new());
        let tree = NavTree::new(&o3_cards(), &selected).widths(44, 360).action_menu(true);
        col.add(&*tree);
        let h = State::new(120);
        let grip = ShelfResizer::new("Settled threads height", &h, 120);
        col.fit(&*grip);
        let shelf = NavTree::new(&o3_model(), &selected).widths(44, 360);
        col.fit(&*shelf);
        let _grip = grip.shelf(&*shelf);
        col.end();
        win.end();
        tree.set_pinned(true);
        shelf.set_pinned(true);
        shot(&win, shots, "a11y-navtree-cards.png")?;
        let snap = fltk_omarchy::a11y::snapshot(&*win).ok_or("no tree")?;
        let has = |f: &dyn Fn(&str) -> bool| snap.lines().any(f);
        let ok = [
            ("card", has(&|l| l.contains("TreeItem \"Fix the bar\"") && l.contains("omarchy") && l.contains("Working 2m") && l.contains("feature/a-very-long-branch-name-for-the-bar") && l.contains("Pinned"))),
            ("note", has(&|l| l.contains("TreeItem \"Theme picker\"") && l.contains("1h") && l.contains("5m"))),
            ("slider", has(&|l| l.contains("Slider \"Settled threads height\"") && l.contains("120"))),
        ];
        delete(win);
        let bad: Vec<&str> = ok.iter().filter(|(_, v)| !v).map(|(n, _)| *n).collect();
        check(bad.is_empty(), &format!("missing {bad:?}:\n{snap}"))
    });
    run("a11y: navtree is a Tree of TreeItems with level, position, expanded, selected; actions are named buttons", &|shots| {
        anim::set_enabled(false);
        let s = shell(o3_model(), 460, |t| t);
        s.tree.set_pinned(true);
        s.selected.set("t:bar".into());
        shot(&s.win, shots, "a11y-navtree.png")?;
        let snap = fltk_omarchy::a11y::snapshot(&*s.win).ok_or("no tree")?;
        let has = |f: &dyn Fn(&str) -> bool| snap.lines().any(f);
        let ok = [
            ("Tree", has(&|l| l.contains("Tree \"Sidebar\"") && l.contains("expanded"))),
            ("host", has(&|l| l.contains("TreeItem \"This computer\"") && l.contains("expanded") && l.contains("level 1") && l.contains("desc \"6\""))),
            ("thread", has(&|l| l.contains("TreeItem \"Fix the bar\"") && l.contains("selected") && l.contains("level 3") && l.contains("1 of 2") && l.contains("PR #42"))),
            ("sub", has(&|l| l.contains("TreeItem \"Explore icons\"") && l.contains("level 4"))),
            ("folded", has(&|l| l.contains("TreeItem \"t3-code\"") && l.contains("collapsed") && l.contains("+2 threads"))),
            ("action", has(&|l| l.contains("Button \"Archive\""))),
            ("new", has(&|l| l.contains("Button \"New thread\""))),
        ];
        delete(s.win);
        let bad: Vec<&str> = ok.iter().filter(|(_, v)| !v).map(|(n, _)| *n).collect();
        check(bad.is_empty(), &format!("missing {bad:?}:\n{snap}"))
    });
}
