//! o3's thread sidebar on `NavTree`, at real size (231 threads): thread
//! cards (project, pin and status on top, title and age, branch / host /
//! provider icons), slim rows for sub-threads (with tree guides) and
//! settled threads, hover actions in place of the time, a hover preview
//! card (500 ms), a context menu (right-click, Menu, Shift+F10), a
//! ranked search (`is:working`, words), drag to move threads, the width
//! edge (260–600 px), and Working / Settled shelves with their own
//! drag-resizable heights. A simulated server sends snapshots
//! (`sync_from`), held while a menu is open.
//!   cargo run --example navtree_o3
//!   cargo run --example navtree_o3 -- --snapshot build/navtree-o3.png
use fltk::prelude::*;
use fltk_omarchy::layout::{self, FlexExt};
use fltk_omarchy::model::ListModel;
use fltk_omarchy::navtree::*;
use fltk_omarchy::popups::PopupItem;
use fltk_omarchy::widgets::{glyph, Label, TextStyle, Tone};
use fltk_omarchy::{tr, trn, App, AppWindow, State};
use std::cell::RefCell;
use std::rc::Rc;

// Nerd Font (Material Design) glyphs this sidebar uses beyond `glyph`.
const BRANCH: &str = "\u{f062c}";
const BOT: &str = "\u{f06a9}";
const SERVER: &str = "\u{f048b}";
const LAPTOP: &str = "\u{f0322}";
const PIN: &str = "\u{f0403}";
const DONE: &str = "\u{f05e1}";
const ALERT: &str = "\u{f0028}";
const QUESTION: &str = "\u{f02d7}";
const SNOOZE: &str = "\u{f0150}";

#[derive(Clone, Copy, PartialEq)]
enum St {
    Working,
    Input,
    Failed,
    Done,
    Settled,
}

#[derive(Clone)]
struct Thread {
    key: String,
    parent: Option<String>,
    project: String,
    title: String,
    branch: String,
    remote: bool,
    state: St,
    pinned: bool,
    unread: bool,
    minutes: u32,
}

fn age(m: u32) -> String {
    match m {
        0..=59 => format!("{m}m"),
        60..=1439 => format!("{}h", m / 60),
        _ => format!("{}d", m / 1440),
    }
}

/// 231 threads like a real o3 sidebar: six projects, sub-threads,
/// a few working or waiting, most settled.
fn server() -> Vec<Thread> {
    let projects = ["omarchy", "o3-code", "lib-fltk-omarchy", "files", "hyprland", "t3-server"];
    let topics = [
        "Fix the bar", "Theme picker", "Sync engine", "Search ranking", "NavTree rows",
        "Popup submenus", "Image decoding", "Diff viewer", "Single instance", "Shelf heights",
        "Dictation ring", "Context meter", "Branch picker", "Prompt history", "Hover card",
    ];
    let mut v = Vec::new();
    let mut n = 0u32;
    while v.len() < 231 {
        let i = n as usize;
        let state = match n % 23 {
            0 | 7 => St::Working,
            3 => St::Input,
            11 => St::Failed,
            1 | 5 | 9 | 13 | 17 => St::Done,
            _ => St::Settled,
        };
        let key = format!("t{n}");
        v.push(Thread {
            key: key.clone(),
            parent: None,
            project: projects[i % projects.len()].into(),
            title: format!("{} #{}", topics[i % topics.len()], n + 1),
            branch: if n % 3 == 0 { format!("feature/{}-{}", topics[i % topics.len()].to_lowercase().replace(' ', "-"), n) } else { String::new() },
            remote: n % 4 == 1,
            state,
            pinned: n == 2 || n == 9,
            unread: n % 10 == 3,
            minutes: (n * 37) % 4000,
        });
        // Working threads run subagents.
        if state == St::Working && v.len() < 229 {
            for k in 0..2 {
                v.push(Thread {
                    key: format!("{key}/s{k}"),
                    parent: Some(key.clone()),
                    project: projects[i % projects.len()].into(),
                    title: format!("{} {}", tr!("Subagent"), k + 1),
                    branch: String::new(),
                    remote: false,
                    state: if k == 0 { St::Working } else { St::Done },
                    pinned: false,
                    unread: false,
                    minutes: k * 3,
                });
            }
        }
        n += 1;
    }
    v
}

fn status(t: &Thread) -> (&'static str, String, ColorRole) {
    match t.state {
        St::Working => (glyph::LOADING, format!("{} {}", tr!("Working"), age(t.minutes)), ColorRole::Yellow),
        St::Input => (QUESTION, tr!("Needs input"), ColorRole::Accent),
        St::Failed => (ALERT, tr!("Failed"), ColorRole::Red),
        St::Done => (DONE, tr!("Done"), ColorRole::Green),
        St::Settled => ("", age(t.minutes), ColorRole::Dim),
    }
}

/// One thread as a row: a card while active, slim when settled or a sub-thread.
fn row(t: &Thread, parent: &str, threads: &[Thread]) -> NavItem {
    let settle = RowAction::new("settle", glyph::CHECK, &tr!("Mark settled")).on_hover();
    let snooze = RowAction::new("snooze", SNOOZE, &tr!("Snooze thread")).on_hover();
    let parent = t.parent.as_deref().unwrap_or(parent);
    if t.parent.is_some() {
        let (icon, text, role) = status(t);
        return NavItem::row(&t.key, if t.state == St::Working { glyph::LOADING } else { BOT }, &t.title)
            .parent(parent)
            .status(icon, &text, role);
    }
    let (icon, text, role) = status(t);
    let mut it = NavItem::row(&t.key, glyph::FOLDER, &t.title)
        .parent(parent)
        .icon_role(ColorRole::Accent)
        .attention(t.unread)
        .action(settle)
        .action(snooze);
    if t.state == St::Settled {
        return it.trailing(&text).dimmed();
    }
    it = it.card().overline(&t.project).status(icon, &text, role);
    if t.state == St::Done {
        it = it.note(&age(t.minutes));
    }
    if t.pinned {
        it = it.badge(Badge::icon(PIN, &tr!("Pinned"), ColorRole::Dim));
    }
    if !t.branch.is_empty() {
        it = it.meta(Meta::new(BRANCH, &t.branch));
    }
    if t.remote {
        it = it.meta(Meta::new(SERVER, ""));
    }
    let kids = threads.iter().filter(|c| c.parent.as_deref() == Some(t.key.as_str())).count();
    if kids > 0 {
        it = it.summary(&trn!("{count} subagents", kids));
    }
    it.meta(Meta::new(LAPTOP, ""))
}

/// The server's snapshot split into o3's lists: active (pinned first),
/// the Working shelf and the Settled shelf.
fn lists(threads: &[Thread]) -> [Vec<NavItem>; 3] {
    let mut active = vec![NavItem::section("pinned", &tr!("Pinned")), NavItem::section("active", &tr!("Active"))];
    let mut working = vec![NavItem::section("working", &tr!("Working"))];
    let mut settled = vec![NavItem::section("settled", &tr!("Settled"))];
    let top = |t: &&Thread| t.parent.is_none();
    for t in threads.iter().filter(top).filter(|t| t.pinned) {
        active.push(row(t, "pinned", threads));
    }
    for t in threads.iter().filter(top).filter(|t| !t.pinned) {
        let (list, sec) = match t.state {
            St::Working => (&mut working, "working"),
            St::Settled => (&mut settled, "settled"),
            _ => (&mut active, "active"),
        };
        list.push(row(t, sec, threads));
        for c in threads.iter().filter(|c| c.parent.as_deref() == Some(t.key.as_str())) {
            list.push(row(c, sec, threads));
        }
    }
    let count = |v: &mut Vec<NavItem>| {
        let sec = v[0].key.clone();
        let n = v.iter().filter(|i| i.parent == sec).count();
        v[0] = v[0].clone().count(&n.to_string());
    };
    count(&mut working);
    count(&mut settled);
    [active, working, settled]
}

/// o3-like search: `is:working`, `is:unread`, else every word in the title.
fn matcher(it: &NavItem, q: &str) -> Option<Match> {
    let mut words = Vec::new();
    for w in q.split_whitespace() {
        match w {
            "is:working" if it.trailing_role == Some(ColorRole::Yellow) => {}
            "is:unread" if it.attention => {}
            w if w.starts_with("is:") => return None,
            w => words.push(w),
        }
    }
    if words.is_empty() {
        return (it.kind == NavKind::Row).then(Match::default);
    }
    default_match(it, &words.join(" "))
}

fn main() {
    let snapshot = std::env::args().skip_while(|a| a != "--snapshot").nth(1);
    let app = App::new("dev.omarchy.fltk.navtree-o3");
    if snapshot.is_some() {
        fltk_omarchy::anim::set_enabled(false);
    }
    let mut win = AppWindow::new("O3", 980, 760);
    let mut body = win.body();
    body.set_margin(0);
    let data = Rc::new(RefCell::new(server()));
    let models: [ListModel<String, NavItem>; 3] = std::array::from_fn(|_| ListModel::new(|t: &NavItem| t.key.clone()));
    let sync = {
        let models = models.clone();
        move |threads: &[Thread]| {
            for (m, items) in models.iter().zip(lists(threads)) {
                m.sync_from(items);
            }
        }
    };
    sync(&data.borrow());
    let selected = State::new("t3".to_string());
    let shown = State::new(String::new());
    let width = State::new(340);
    let mut row = layout::row();
    let mut side = layout::column();
    side.set_spacing(0);
    // The trees share the selection, actions, menu and preview.
    let make = |model: &ListModel<String, NavItem>| {
        let (d, m, s, sh) = (data.clone(), model.clone(), sync.clone(), shown.clone());
        let tree = NavTree::new(model, &selected)
            .widths(48, 340)
            .tree_guides(true)
            .on_activate(move |key| {
                m.patch(&key.to_string(), |t| t.attention = false);
                sh.set(m.with_key(&key.to_string(), |t| t.title.clone()).unwrap_or_default());
            })
            .on_action(move |key, action| {
                let mut d = d.borrow_mut();
                if let Some(t) = d.iter_mut().find(|t| t.key == key) {
                    match action {
                        "settle" => t.state = St::Settled,
                        "snooze" => t.minutes += 60,
                        _ => {}
                    }
                }
                s(&d);
            });
        let d = data.clone();
        tree.preview(300, move |key, col| {
            let d = d.borrow();
            let Some(t) = d.iter().find(|t| t.key == key) else { return false };
            col.fit(&*Label::styled(&t.title, TextStyle::Caption).bold());
            for (k, v) in [(tr!("Project"), t.project.clone()), (tr!("Branch"), t.branch.clone()), (tr!("Host"), if t.remote { "build-box".into() } else { tr!("This computer") })] {
                if !v.is_empty() {
                    col.fit(&*Label::styled(&format!("{k}: {v}"), TextStyle::Caption).tone(Tone::Dim));
                }
            }
            true
        })
    };
    let main_tree = make(&models[0])
        .search(&tr!("Search threads"))
        .matcher(matcher)
        .resizable(260, 600)
        .bind_width(&width)
        .reorder(Reorder::Siblings);
    let menu_tree = main_tree.clone();
    let main_tree = main_tree.on_context(move |key, x, y| {
        menu_tree.pause_updates(true);
        let items = vec![
            PopupItem::new(PIN, &tr!("Pin thread")),
            PopupItem::new(glyph::CHECK, &tr!("Mark settled")),
            PopupItem::new(SNOOZE, &tr!("Snooze thread")),
            PopupItem::submenu("", &tr!("Copy"), vec![PopupItem::new("", &tr!("Path")), PopupItem::new("", &tr!("Branch"))]),
        ];
        let key = key.to_string();
        let s = menu_tree.clone();
        menu_tree
            .open_menu(x, y, items, move |i| println!("{key}: entry {i}"))
            .on_dismiss(move |_| s.pause_updates(false));
    });
    side.add(&*main_tree);
    let working_h = State::new(220);
    let grip_w = ShelfResizer::new(&tr!("Working threads height"), &working_h, 220);
    side.fit(&*grip_w);
    let working = make(&models[1]).action_menu(true);
    side.fit(&*working);
    let grip_w = grip_w.shelf(&*working);
    let settled_h = State::new(160);
    let grip_s = ShelfResizer::new(&tr!("Settled threads height"), &settled_h, 160);
    side.fit(&*grip_s);
    let settled = make(&models[2]).action_menu(true);
    side.fit(&*settled);
    let grip_s = grip_s.shelf(&*settled);
    side.end();
    // The sidebar column follows the main tree's width edge.
    let side_w = side.clone();
    width
        .subscribe(move |w| {
            let _ = layout::refit(&side_w, (*w, side_w.h()));
        })
        .forever();
    row.fit(&side);
    row.fixed(&side, width.get());
    let mut content = layout::column();
    content.set_margin(16);
    content.fit(&*Label::styled("", TextStyle::Title).bind(&shown));
    content.fit(
        &*Label::new(&tr!("Hover a thread for its card; search with words, is:working or is:unread."))
            .tone(Tone::Dim),
    );
    content.end();
    row.end();
    win.end();
    for t in [&main_tree, &working, &settled] {
        t.set_pinned(true);
    }
    let _ = (grip_w, grip_s);
    shown.set(tr!("Theme picker #4"));
    if let Some(path) = snapshot {
        win.snapshot(&path).expect("snapshot");
        return;
    }
    // The "server" sends snapshots: threads start and stop working.
    let d = data.clone();
    let mut tick = 0usize;
    fltk::app::add_timeout3(1.5, move |h| {
        tick += 1;
        {
            let mut d = d.borrow_mut();
            let n = d.len();
            let i = (tick * 7) % n;
            if d[i].parent.is_none() {
                d[i].state = match d[i].state {
                    St::Working => St::Done,
                    St::Done => St::Settled,
                    St::Settled => St::Working,
                    s => s,
                };
                d[i].unread = d[i].state == St::Done;
            }
            sync(&d);
        }
        fltk::app::repeat_timeout3(1.5, h);
    });
    win.show();
    app.run();
}
