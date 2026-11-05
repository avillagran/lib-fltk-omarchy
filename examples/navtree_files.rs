//! A file manager's places sidebar on `NavTree`: favorites, bookmarks
//! (reorder them by dragging, drop files on them) and devices with free
//! space, a usage bar and an eject action. The free space changes live;
//! ejecting shows a spinner, then the device leaves.
//!   cargo run --example navtree_files
//!   cargo run --example navtree_files -- --snapshot build/navtree-files.png
use fltk::prelude::*;
use fltk_omarchy::layout::{self, FlexExt};
use fltk_omarchy::model::ListModel;
use fltk_omarchy::navtree::*;
use fltk_omarchy::popups::PopupItem;
use fltk_omarchy::widgets::{glyph, Label, TextStyle, Tone};
use fltk_omarchy::{tr, App, AppWindow, State};

fn places() -> Vec<NavItem> {
    let eject = || RowAction::new("eject", glyph::EJECT, &tr!("Eject"));
    vec![
        NavItem::section("fav", &tr!("Favorites")),
        NavItem::row("~", glyph::HOME, &tr!("Home")).parent("fav"),
        NavItem::row("~/Downloads", glyph::DOWNLOAD, &tr!("Downloads"))
            .parent("fav")
            .badge(Badge::text("3", ColorRole::Dim)),
        NavItem::row("~/Documents", glyph::FOLDER, &tr!("Documents")).parent("fav"),
        NavItem::section("marks", &tr!("Bookmarks"))
            .collapsible()
            .count("3"),
        NavItem::row("~/Work", glyph::BOOKMARK, "Work").parent("marks"),
        NavItem::row("~/Work/omarchy", glyph::BOOKMARK, "omarchy").parent("marks"),
        NavItem::row("/srv/media", glyph::BOOKMARK, "media")
            .parent("marks")
            .dimmed()
            .detail(&tr!("Not mounted")),
        NavItem::section("dev", &tr!("Devices")).collapsible(),
        NavItem::row("dev:nvme", glyph::HARDDISK, &tr!("System"))
            .parent("dev")
            .detail(&tr!(
                "{free} free of {size}",
                free = "212 GB",
                size = "512 GB"
            ))
            .usage(0.59),
        NavItem::row("dev:usb", glyph::USB, "KINGSTON")
            .parent("dev")
            .detail(&tr!(
                "{free} free of {size}",
                free = "1.2 GB",
                size = "16 GB"
            ))
            .usage(0.93)
            .action(eject()),
        NavItem::row("dev:sd", glyph::HARDDISK, "SD card")
            .parent("dev")
            .action(eject()),
        NavItem::row("trash", glyph::TRASH, &tr!("Trash")).footer(),
    ]
}

fn main() {
    let snapshot = std::env::args().skip_while(|a| a != "--snapshot").nth(1);
    let app = App::new("dev.omarchy.fltk.navtree-files");
    if snapshot.is_some() {
        fltk_omarchy::anim::set_enabled(false);
    }
    let mut win = AppWindow::new(&tr!("Files"), 760, 480);
    let mut body = win.body();
    body.set_margin(0);
    let model = ListModel::new(|t: &NavItem| t.key.clone());
    model.extend(places());
    let place = State::new("~/Downloads".to_string());
    let title = State::new(tr!("Downloads"));
    let mut row = layout::row();
    let (m1, m2, m3, t1) = (model.clone(), model.clone(), model.clone(), title.clone());
    let side = NavTree::new(&model, &place)
        .widths(48, 260)
        .resizable(200, 380)
        .reorder(Reorder::Siblings)
        // Bookmarks only: favorites and devices keep their order.
        .accept_move(|key, parent, _| parent == "marks" && !key.starts_with("dev:"))
        .on_move(move |key, parent, index| move_item(&m1, key, parent, index))
        .on_activate(move |key| {
            let name = m2
                .with_key(&key.to_string(), |t| t.title.clone())
                .unwrap_or_default();
            t1.set(name);
        })
        .on_action(move |key, action| {
            if action == "eject" {
                let key = key.to_string();
                m3.patch(&key, |t| t.busy = true);
                let m = m3.clone();
                fltk::app::add_timeout3(1.2, move |_| {
                    m.remove_key(&key);
                });
            }
        })
        .accept_drop(|key| !key.starts_with("dev:") && key != "/srv/media")
        .on_drop_files(|key, files| println!("copy {files:?} to {key}"));
    let menu_side = side.clone();
    let side = side.on_context(move |key, x, y| {
        let items = vec![
            PopupItem::new(glyph::FOLDER, &tr!("Open")),
            PopupItem::new(glyph::PLUS, &tr!("Open in new tab")).divider(),
            PopupItem::submenu(
                "",
                &tr!("Sort by"),
                vec![
                    PopupItem::new("", &tr!("Name")),
                    PopupItem::new("", &tr!("Date")),
                ],
            ),
            PopupItem::new(glyph::TRASH, &tr!("Remove bookmark")),
        ];
        let key = key.to_string();
        menu_side.open_menu(x, y, items, move |i| println!("{key}: entry {i}"));
    });
    row.fit(&*side);
    let mut content = layout::column();
    content.set_margin(16);
    content.fit(&*Label::styled("", TextStyle::Title).bind(&title));
    content.fit(
        &*Label::new(&tr!(
            "Drag bookmarks to reorder them; drop files on a folder."
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
    // Free space changes while the app runs: a patch repaints that row only.
    let m = model.clone();
    let mut used = 0.59f64;
    fltk::app::add_timeout3(1.0, move |h| {
        used = (used + 0.01).min(0.97);
        let free = format!("{:.0} GB", 512.0 * (1.0 - used));
        m.patch(&"dev:nvme".to_string(), |t| {
            t.usage = Some(used);
            t.detail = tr!(
                "{free} free of {size}",
                free = free.as_str(),
                size = "512 GB"
            );
        });
        fltk::app::repeat_timeout3(1.0, h);
    });
    win.show();
    app.run();
}
