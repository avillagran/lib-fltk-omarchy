//! Renders the pictures of docs/COMPONENTS.md offscreen (no window is
//! shown) with the current Omarchy theme. The code of each block is the
//! snippet shown next to its picture.
//!   cargo run --example docs_shots -- docs/components
use fltk::{group::Flex, prelude::*, window::DoubleWindow};
use fltk_omarchy::focus::press;
use fltk_omarchy::layout::{self, FlexExt};
use fltk_omarchy::list::{List, ListRow};
use fltk_omarchy::overlay::{ConfirmDialog, Dropdown, Menu, MenuItem, Notification, Urgency};
use fltk_omarchy::popups::{OperationToast, PopupItem, PopupMenu, SearchableDropdown, ShortcutHelp};
use fltk_omarchy::widgets::{glyph, *};
use fltk_omarchy::{theme, App, AppWindow, State};
use std::path::{Path, PathBuf};

/// A bare window with a padded column (the picture frame).
fn frame(w: i32, h: i32) -> (DoubleWindow, Flex) {
    let win = DoubleWindow::new(0, 0, w, h, None);
    let pad = theme::with(|t| t.spacing.panel_padding);
    let mut col = layout::column();
    col.resize(pad, pad, w - 2 * pad, h - 2 * pad);
    (win, col)
}

fn done(win: DoubleWindow, col: Flex) -> DoubleWindow {
    layout::spacer();
    col.end();
    win.end();
    // Lay out now: a Flex places its children when first drawn, and the
    // popups opened below are placed from their anchor's real geometry.
    col.layout();
    for i in 0..col.children() {
        if let Some(mut g) = col.child(i).and_then(|c| c.as_group()) {
            if let Some(f) = Flex::from_dyn_widget(&g) { f.layout(); }
            g.redraw();
        }
    }
    win
}

fn save(win: &mut DoubleWindow, dir: &Path, name: &str) {
    let p = dir.join(format!("{name}.png"));
    fltk_omarchy::window::snapshot(win, &p).expect("snapshot");
    println!("{}", p.display());
}

fn main() {
    let dir = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| "docs/components".into()));
    std::fs::create_dir_all(&dir).unwrap();
    let _app = App::without_theme_watch("dev.omarchy.fltk.docs");

    // ---- buttons
    let (win, col) = frame(600, 76);
    let mut r = layout::row();
    r.fit(&*Button::new("Default"));
    r.fit(&*Button::new("Bordered").bordered());
    r.fit(&*Button::icon(glyph::PLUS, "New").primary());
    r.fit(&*Button::icon(glyph::TRASH, "Delete").danger().bordered());
    r.fit(&*ActionButton::new(glyph::REFRESH, "Refresh"));
    r.fit(&*ActionButton::destructive(glyph::CLOSE, "Close"));
    r.close_fitted();
    save(&mut done(win, col), &dir, "buttons");

    // ---- button group
    let (win, col) = frame(600, 76);
    let period = State::new(1usize);
    let mut r = layout::row();
    r.fit(&*ButtonGroup::new(&["Day", "Week", "Month"], &period));
    r.close_fitted();
    save(&mut done(win, col), &dir, "button-group");

    // ---- text fields
    let (win, mut col) = frame(600, 176);
    let query = State::new(String::new());
    col.fit(&*TextField::new().placeholder("Search apps…").bind(&query));
    let name = State::new("Omarchy".to_string());
    col.fit(&*TextField::new().bind(&name));
    let mut r = layout::row();
    r.fit(&*TextField::password().placeholder("Password"));
    r.fit(&*NumberField::new(&State::new(42), 0, 100, 1));
    r.close_fitted();
    save(&mut done(win, col), &dir, "text-fields");

    // ---- toggles
    let (win, mut col) = frame(600, 214);
    let wifi = State::new(true);
    let mut r = layout::row();
    r.fit(&*Label::new("Wi-Fi"));
    layout::spacer();
    r.fit(&*ToggleSwitch::new(&wifi));
    r.close_fitted();
    col.fit(&*Toggle::new("Night light", "Warmer colors after sunset", &State::new(true)));
    col.fit(&*Toggle::new("Do not disturb", "Silence notifications", &State::new(false)));
    save(&mut done(win, col), &dir, "toggles");

    // ---- slider and progress
    let (win, mut col) = frame(600, 110);
    let volume = State::new(0.64);
    let mut r = layout::row();
    r.fit(&*Label::new("Volume"));
    let _slider = Slider::new(&volume, 0.0, 1.0, 0.05);
    r.fit(&*Label::new("").right().bind(&volume.map(|v| format!("{:.0}%", v * 100.0))));
    r.close_fitted();
    col.fit(&*Progress::new(&State::new(0.42)));
    save(&mut done(win, col), &dir, "slider-progress");

    // ---- text styles, hero, separator
    let (win, mut col) = frame(600, 250);
    col.fit(&*Hero::new(glyph::VOLUME_HIGH, "Audio", "Built-in speakers").detail("64%"));
    col.fit(&*Separator::new());
    col.fit(&*Label::section("Output"));
    col.fit(&*Label::styled("Title text", TextStyle::Title));
    col.fit(&*Label::new("Body text"));
    col.fit(&*Label::new("Dim caption").tone(Tone::Dim));
    save(&mut done(win, col), &dir, "labels");

    // ---- rows
    let (win, mut col) = frame(600, 160);
    col.fit(&*Row::new(glyph::MONITOR, "HDMI / DisplayPort").detail("Digital"));
    col.fit(&*Row::new(glyph::HEADPHONES, "Built-in speakers").detail("Analog").current(true));
    col.fit(&*Row::new(glyph::BLUETOOTH, "WH-1000XM4").detail("Bluetooth").on_click(|| println!("connect")));
    save(&mut done(win, col), &dir, "rows");

    // ---- tabs
    let (win, mut col) = frame(600, 76);
    let tab = State::new(1usize);
    col.fit(&*TabBar::new(&["Home", "Downloads", "Work"], &tab).on_close(|i| println!("close {i}")));
    save(&mut done(win, col), &dir, "tabs");

    // ---- dropdown (open)
    let (win, mut col) = frame(600, 210);
    let output = State::new(0usize);
    let mut dd = Dropdown::new("Output", &["Speakers", "Headphones", "HDMI"], &output);
    col.fit(&*dd);
    let mut win = done(win, col);
    dd.open();
    save(&mut win, &dir, "dropdown");

    // ---- searchable dropdown (open, filtered)
    let (win, mut col) = frame(600, 320);
    let place = State::new(None::<usize>);
    let places = vec![
        ("Home".to_string(), "~/".to_string()),
        ("Downloads".to_string(), "~/Downloads".to_string()),
        ("Work".to_string(), "~/Work".to_string()),
        ("Pictures".to_string(), "~/Pictures".to_string()),
    ];
    let mut sd = SearchableDropdown::new("Go to…", places, &place);
    col.fit(&*sd);
    let mut win = done(win, col);
    sd.open();
    press("o");
    save(&mut win, &dir, "searchable-dropdown");

    // ---- virtual list
    let (win, mut col) = frame(600, 300);
    let count = State::new(100_000usize);
    let selected = State::new(Some(2usize));
    let list = List::new(&count, |i| ListRow::new(&format!("Item {i}")).icon(glyph::FILE).detail(&format!("{} KB", i * 3)))
        .bind_selected(&selected);
    col.fit(&*list);
    col.fixed(&*list, 250);
    save(&mut done(win, col), &dir, "list");

    // ---- popup menu (header Actions menu / context menu)
    let (win, col) = frame(600, 260);
    let mut r = layout::row();
    let actions = Button::icon("\u{f0241}", "Actions");
    r.fit(&*actions);
    r.close_fitted();
    let mut win = done(win, col);
    PopupMenu::open(&*actions, vec![
        PopupItem::new(glyph::FOLDER, "Open").shortcut("Enter"),
        PopupItem::new(glyph::SEARCH, "Preview").shortcut("Space").divider(),
        PopupItem::new(glyph::FILE, "Copy").shortcut("Ctrl+C"),
        PopupItem::new(glyph::FILE, "Show hidden files").checked(true),
        PopupItem::new(glyph::TRASH, "Move to trash").shortcut("Delete").disabled(),
    ], |i| println!("picked {i}"));
    save(&mut win, &dir, "popup-menu");

    // ---- Omarchy menu (type to filter)
    let (win, col) = frame(600, 420);
    let mut win = done(win, col);
    Menu::open(&win, "Go", vec![
        MenuItem::new(glyph::SEARCH, "Apps").detail("launch an app"),
        MenuItem::new(glyph::MONITOR, "Screenshot").detail("capture the screen"),
        MenuItem::new(glyph::MONITOR, "Screen record").detail("record the screen"),
        MenuItem::new(glyph::SETTINGS, "Setup").submenu(),
    ], |i| println!("picked {i}"));
    press("s"); press("c");
    save(&mut win, &dir, "menu");

    // ---- confirm dialog
    let (win, col) = frame(600, 260);
    let mut win = done(win, col);
    ConfirmDialog::open(&win, "Move 3 files to the trash?", "Move", true, |ok| println!("confirmed: {ok}"));
    save(&mut win, &dir, "confirm-dialog");

    // ---- notifications
    let (win, col) = frame(600, 200);
    let mut win = done(win, col);
    Notification::show(&win, glyph::BELL, "Copied 3 files", "to ~/Work/reports", Urgency::Normal);
    Notification::show(&win, glyph::WIFI, "Connected", "", Urgency::Low);
    save(&mut win, &dir, "notification");

    // ---- operation toast
    let (win, col) = frame(600, 200);
    let mut win = done(win, col);
    let mut op = OperationToast::show(&win, "Copying 3 files").unwrap()
        .on_pause(|paused| println!("paused: {paused}"))
        .on_cancel(|| println!("cancel"));
    op.set_progress(0.42);
    op.set_detail("report.pdf · 12 of 30 MB");
    save(&mut win, &dir, "operation-toast");
    op.close();

    // ---- F1 shortcut help
    let (win, col) = frame(600, 420);
    let mut win = done(win, col);
    let row = |k: &str, d: &str| (k.to_string(), d.to_string());
    ShortcutHelp::open(&win, "Keyboard shortcuts", vec![
        ("Move".into(), vec![row("Tab", "Next control"), row("Shift+Tab", "Previous control"), row("Ctrl+Tab", "Next tab")]),
        ("Files".into(), vec![row("Enter", "Open"), row("F2", "Rename"), row("Delete", "Move to trash")]),
        ("Window".into(), vec![row("Ctrl+,", "Preferences"), row("F10", "Actions"), row("F1", "This help")]),
    ]);
    save(&mut win, &dir, "shortcut-help");

    // ---- app window: header actions + body
    let mut w = AppWindow::new("Notes", 600, 260);
    w.header().action(glyph::SETTINGS, "Preferences", || {});
    w.header().action("\u{f0241}", "Actions", || {});
    w.header().destructive_action(glyph::CLOSE, "Close", || {});
    let mut body = w.body();
    body.fit(&*TextField::new().placeholder("Title"));
    body.fit(&*Toggle::new("Pinned", "Keep at the top of the list", &State::new(true)));
    let mut r = layout::row();
    layout::spacer();
    r.fit(&*Button::new("Cancel").bordered());
    r.fit(&*Button::new("Save").primary());
    r.close_fitted();
    w.end();
    w.snapshot(dir.join("app-window.png")).unwrap();
    println!("{}", dir.join("app-window.png").display());

    // ---- keyboard: focus ring while driving the app with the keyboard
    let mut w = AppWindow::new("Keyboard", 600, 330);
    let mut body = w.body();
    body.fit(&*TextField::new().placeholder("Search…"));
    let view = State::new(0usize);
    let mut r = layout::row();
    r.fit(&*ButtonGroup::new(&["Day", "Week", "Month"], &view));
    r.close_fitted();
    let tab = State::new(0usize);
    body.fit(&*TabBar::new(&["Home", "Downloads", "Work"], &tab));
    let _list = List::new(&State::new(1000usize), |i| ListRow::new(&format!("Item {i}")).icon(glyph::FILE));
    w.end();
    fltk_omarchy::focus::first(&*w);
    press("Tab");
    press("Right");                           // button group: Week
    w.snapshot(dir.join("keyboard-1-group.png")).unwrap();
    press("Tab"); press("Ctrl+Tab");          // tab bar: Downloads
    w.snapshot(dir.join("keyboard-2-tabs.png")).unwrap();
    press("Tab"); press("Down"); press("Down"); // list cursor
    w.snapshot(dir.join("keyboard-3-list.png")).unwrap();
    println!("{}", dir.join("keyboard-*.png").display());
}
