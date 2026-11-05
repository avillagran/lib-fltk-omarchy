//! An o3-style thread sidebar on `NavTree`: hosts → projects → threads →
//! sub-threads, status and PR badges, a spinner while a thread works,
//! unread threads, a ranked search (`is:working`, words in any order),
//! per-thread actions on hover, drag to move threads between projects,
//! and live updates from a simulated server (snapshots through
//! `sync_from`, held while a context menu is open).
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

#[derive(Clone)]
struct Thread {
    key: String,
    host: String,
    project: String,
    parent: Option<String>,
    title: String,
    working: bool,
    unread: bool,
    pr: Option<u32>,
    failed: bool,
}

fn thread(key: &str, host: &str, project: &str, title: &str) -> Thread {
    Thread {
        key: key.into(),
        host: host.into(),
        project: project.into(),
        parent: None,
        title: title.into(),
        working: false,
        unread: false,
        pr: None,
        failed: false,
    }
}

/// The server's snapshot as sidebar items (the app's own model → rows).
fn items(threads: &[Thread]) -> Vec<NavItem> {
    let mut out = Vec::new();
    let mut hosts: Vec<&str> = threads.iter().map(|t| t.host.as_str()).collect();
    hosts.dedup();
    for host in hosts {
        let n = threads
            .iter()
            .filter(|t| t.host == host && t.parent.is_none())
            .count();
        out.push(
            NavItem::section(&format!("h:{host}"), host)
                .collapsible()
                .count(&n.to_string())
                .action(RowAction::new("new", glyph::PLUS, &tr!("New thread"))),
        );
        let mut projects: Vec<&str> = threads
            .iter()
            .filter(|t| t.host == host)
            .map(|t| t.project.as_str())
            .collect();
        projects.dedup();
        for p in projects {
            let pk = format!("p:{host}/{p}");
            out.push(
                NavItem::row(&pk, glyph::FOLDER, p)
                    .parent(&format!("h:{host}"))
                    .icon_role(ColorRole::Accent),
            );
            for t in threads.iter().filter(|t| t.host == host && t.project == p) {
                let kids = threads
                    .iter()
                    .filter(|c| c.parent.as_deref() == Some(t.key.as_str()))
                    .count();
                let mut it = NavItem::row(
                    &t.key,
                    if t.parent.is_some() {
                        glyph::LOADING
                    } else {
                        glyph::FILE
                    },
                    &t.title,
                )
                .parent(t.parent.as_deref().unwrap_or(&pk))
                .attention(t.unread)
                .action(RowAction::new("archive", glyph::CHECK, &tr!("Archive")).on_hover())
                .action(RowAction::new("pin", glyph::BOOKMARK, &tr!("Pin")).on_hover());
                if kids > 0 {
                    it = it.summary(&trn!("+{count} sub-threads", kids));
                }
                if t.working {
                    it = it.badge(Badge::spinner()).detail(&tr!("Working"));
                }
                if let Some(n) = t.pr {
                    it = it.badge(Badge::pill(&format!("PR #{n}"), ColorRole::Green));
                }
                if t.failed {
                    it = it.badge(Badge::pill(&tr!("failed"), ColorRole::Red));
                }
                out.push(it);
            }
        }
    }
    out.push(
        NavItem::row("settings", glyph::SETTINGS, &tr!("Settings"))
            .footer()
            .shortcut("Ctrl+,"),
    );
    out
}

fn server() -> Vec<Thread> {
    let mut v = vec![
        thread("t1", "this computer", "omarchy", "Fix the bar"),
        thread("t1a", "this computer", "omarchy", "Explore icons"),
        thread("t1b", "this computer", "omarchy", "Write tests"),
        thread("t2", "this computer", "omarchy", "Theme picker"),
        thread("t3", "this computer", "t3-code", "Sync engine"),
        thread("t4", "this computer", "t3-code", "Search ranking"),
        thread("t5", "build-box", "lib-fltk-omarchy", "NavTree"),
        thread("t6", "build-box", "lib-fltk-omarchy", "PopupMenu submenus"),
    ];
    v[0].pr = Some(42);
    v[1].parent = Some("t1".into());
    v[1].working = true;
    v[2].parent = Some("t1".into());
    v[3].unread = true;
    v[6].working = true;
    v[7].failed = true;
    v
}

/// o3-like search: `is:working`, `is:unread`, else every word in the title.
fn matcher(it: &NavItem, q: &str) -> Option<Match> {
    let mut words = Vec::new();
    for w in q.split_whitespace() {
        match w {
            "is:working" if it.badges.contains(&Badge::Spinner) => {}
            "is:unread" if it.attention => {}
            w if w.starts_with("is:") => return None,
            w => words.push(w),
        }
    }
    if words.is_empty() {
        return (it.kind == NavKind::Row && !it.key.starts_with("p:") && it.key != "settings")
            .then(Match::default);
    }
    default_match(it, &words.join(" "))
}

fn main() {
    let snapshot = std::env::args().skip_while(|a| a != "--snapshot").nth(1);
    let app = App::new("dev.omarchy.fltk.navtree-o3");
    if snapshot.is_some() {
        fltk_omarchy::anim::set_enabled(false);
    }
    let mut win = AppWindow::new("O3", 820, 520);
    let mut body = win.body();
    body.set_margin(0);
    let data = Rc::new(RefCell::new(server()));
    let model = ListModel::new(|t: &NavItem| t.key.clone());
    model.sync_from(items(&data.borrow()));
    let selected = State::new("t1".to_string());
    let shown = State::new(tr!("Fix the bar"));
    let mut row = layout::row();
    let (d1, m1) = (data.clone(), model.clone());
    let (d2, m2) = (data.clone(), model.clone());
    let (m3, s3) = (model.clone(), shown.clone());
    let side = NavTree::new(&model, &selected)
        .widths(48, 300)
        .resizable(220, 420)
        .search(&tr!("Search threads"))
        .matcher(matcher)
        .reorder(Reorder::Anywhere)
        // Threads go into projects (or under a thread, as sub-threads).
        .accept_move(|key, parent, _| {
            key.starts_with('t') && (parent.starts_with("p:") || parent.starts_with('t'))
        })
        .on_move(move |key, parent, _| {
            let mut d = d1.borrow_mut();
            if let Some(t) = d.iter_mut().find(|t| t.key == key) {
                if let Some(p) = parent.strip_prefix("p:") {
                    let (host, project) = p.split_once('/').unwrap_or(("", p));
                    t.host = host.into();
                    t.project = project.into();
                    t.parent = None;
                }
            }
            m1.sync_from(items(&d));
        })
        .on_action(move |key, action| {
            let mut d = d2.borrow_mut();
            match action {
                "archive" => d.retain(|t| t.key != key && t.parent.as_deref() != Some(key)),
                "pin" => {
                    if let Some(i) = d.iter().position(|t| t.key == key) {
                        let t = d.remove(i);
                        d.insert(0, t);
                    }
                }
                _ => println!("{action} on {key}"),
            }
            m2.sync_from(items(&d));
        })
        .on_activate(move |key| {
            m3.patch(&key.to_string(), |t| t.attention = false);
            s3.set(
                m3.with_key(&key.to_string(), |t| t.title.clone())
                    .unwrap_or_default(),
            );
        });
    // A context menu holds live updates until it closes.
    let menu_side = side.clone();
    let side = side.on_context(move |key, x, y| {
        menu_side.pause_updates(true);
        let items = vec![
            PopupItem::new(glyph::CHECK, &tr!("Archive")),
            PopupItem::new(glyph::BOOKMARK, &tr!("Pin")),
            PopupItem::submenu(
                "",
                &tr!("Move to"),
                vec![PopupItem::new("", "omarchy"), PopupItem::new("", "t3-code")],
            ),
        ];
        let key = key.to_string();
        let s = menu_side.clone();
        menu_side
            .open_menu(x, y, items, move |i| println!("{key}: entry {i}"))
            .on_dismiss(move |_| s.pause_updates(false));
    });
    row.fit(&*side);
    let mut content = layout::column();
    content.set_margin(16);
    content.fit(&*Label::styled("", TextStyle::Title).bind(&shown));
    content.fit(
        &*Label::new(&tr!(
            "Type in the search field: words, is:working or is:unread."
        ))
        .tone(Tone::Dim),
    );
    content.end();
    row.end();
    win.end();
    side.set_pinned(true);
    if let Some(path) = snapshot {
        win.snapshot(&path).expect("snapshot");
        return;
    }
    // The "server" sends snapshots: threads start and stop working.
    let (d, m) = (data.clone(), model.clone());
    let mut tick = 0usize;
    fltk::app::add_timeout3(1.5, move |h| {
        tick += 1;
        {
            let mut d = d.borrow_mut();
            let n = d.len();
            if n > 0 {
                let i = tick % n;
                d[i].working = !d[i].working;
                if !d[i].working {
                    d[i].unread = true;
                }
            }
            m.sync_from(items(&d));
        }
        fltk::app::repeat_timeout3(1.5, h);
    });
    win.show();
    app.run();
}
