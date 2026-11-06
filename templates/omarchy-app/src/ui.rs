//! The window: layout and keys, nothing else. State lives in `State`s
//! bound to the widgets; the work is in `backend`.

use crate::backend::{self, Tally};
use crate::{DISPLAY_NAME, NAME};
use fltk_omarchy::dialog::{self, Filter};
use fltk_omarchy::layout::{self, FlexExt};
use fltk_omarchy::popups::ShortcutHelp;
use fltk_omarchy::widgets::*;
use fltk_omarchy::{combine, i18n, tr, trn, App, AppWindow, State};
use std::path::PathBuf;

/// Nerd Font keyboard glyph for the shortcuts action.
const KEYBOARD: &str = "\u{f030c}";

/// The window and the state it shows.
#[derive(Clone)]
pub struct Ui {
    pub win: AppWindow,
    pub tally: State<Tally>,
    /// A one-line message under the count (errors, "Exported to …");
    /// empty shows whether the tally is saved.
    pub notice: State<String>,
}

/// The shortcut overlay's sections, built on each open so they follow the
/// language. Keys are written as specs (`Mod` is Ctrl, or Cmd on macOS)
/// and shown the platform's way.
pub fn shortcuts() -> Vec<(String, Vec<(String, String)>)> {
    let row = |k: &str, d: String| (k.to_string(), d);
    vec![
        (
            tr!("Counting"),
            vec![
                row("Space", tr!("Count one more")),
                row("Mod+Z", tr!("Undo")),
                row("Mod+Shift+Z", tr!("Redo")),
            ],
        ),
        (tr!("File"), vec![row("Mod+S", tr!("Save")), row("Mod+E", tr!("Export…"))]),
        (tr!("App"), vec![row("?", tr!("Keyboard shortcuts")), row("Q", tr!("Quit"))]),
    ]
}

/// Builds the window around `tally` without showing it (the GUI, `render`
/// and the tests share it). `file` is where Mod+S saves; `None` turns
/// saving off (the file could not be read, so it is never overwritten).
pub fn build(tally: Tally, file: Option<PathBuf>) -> Ui {
    let tally = State::new(tally);
    let notice = State::new(String::new());
    let mut win = AppWindow::new(DISPLAY_NAME, 520, 380);
    let mut body = win.body();

    layout::spacer();
    let count = i18n::map(&tally, |t| trn!("{count} taps", t.count()));
    // The theme accent, followed live like every other theme color.
    body.fit(&*Label::styled("", TextStyle::DisplayLarge).tone(Tone::Accent).center().bind(&count));
    let hint = i18n::text("Press Space to count");
    body.fit(&*Label::new("").tone(Tone::Dim).center().bind(&hint));
    layout::spacer();
    let status = combine(&tally, &notice, |t, n| (t.unsaved(), n.clone()));
    let status = i18n::map(&status, |(unsaved, notice)| {
        if !notice.is_empty() {
            notice.clone()
        } else if *unsaved {
            tr!("Unsaved changes")
        } else {
            tr!("Saved")
        }
    });
    body.fit(&*Label::new("").tone(Tone::Dim).center().bind(&status));
    win.end();

    let ui = Ui { win: win.clone(), tally, notice };

    // Header actions: icon-only, so their tooltips are their names.
    let u = ui.clone();
    win.header()
        .action(glyph::DOWNLOAD, "", move || u.export())
        .bind_tooltip(&i18n::text("Export…"));
    let w = win.clone();
    win.header()
        .action(KEYBOARD, "", move || {
            ShortcutHelp::open(&*w, &tr!("Keyboard shortcuts"), shortcuts());
        })
        .bind_tooltip(&i18n::text("Keyboard shortcuts"));

    // Omarchy's keys: ? (and F1) shows them all, Q quits and asks first
    // when there is unsaved work, Space is the main action.
    win.help_overlay(&tr!("Keyboard shortcuts"), shortcuts);
    let t = ui.tally.clone();
    win.quit_key(move || t.with(Tally::unsaved));
    let u = ui.clone();
    win.shortcut("Space", move || u.act(Tally::add));
    let u = ui.clone();
    win.shortcut("Mod+Z", move || u.act(|t| {
        t.undo();
    }));
    let u = ui.clone();
    win.shortcut("Mod+Shift+Z", move || u.act(|t| {
        t.redo();
    }));
    let u = ui.clone();
    win.shortcut("Mod+S", move || u.save(file.as_deref()));
    let u = ui.clone();
    win.shortcut("Mod+E", move || u.export());
    ui
}

impl Ui {
    fn act(&self, f: impl FnOnce(&mut Tally)) {
        self.notice.set(String::new());
        self.tally.update(f);
    }

    fn save(&self, file: Option<&std::path::Path>) {
        let Some(file) = file else {
            self.notice.set(tr!("Saving is off: the saved tally could not be read"));
            return;
        };
        match backend::save(file, self.tally.with(Tally::count)) {
            Ok(()) => {
                self.notice.set(String::new());
                self.tally.update(Tally::mark_saved);
            }
            Err(e) => self.notice.set(tr!("Could not save: {error}", error = e)),
        }
    }

    /// Exports through the desktop's file chooser (xdg-desktop-portal);
    /// the answer comes back on the UI thread.
    fn export(&self) {
        let count = self.tally.with(Tally::count);
        let notice = self.notice.clone();
        dialog::Request::save(tr!("Export…"))
            .name(format!("{NAME}.txt"))
            .filter(Filter::new(tr!("Text")).glob("*.txt").mime("text/plain"))
            .run_async(move |r| match r {
                Ok(paths) => {
                    let Some(path) = paths.into_iter().next() else { return };
                    match backend::write_atomic(&path, backend::export_text(count).as_bytes()) {
                        Ok(()) => notice.set(tr!("Exported to {path}", path = path.display())),
                        Err(e) => notice.set(tr!("Could not export: {error}", error = e)),
                    }
                }
                Err(dialog::Error::Cancelled) => {}
                // Say what's wrong instead of quietly doing nothing.
                Err(e) => notice.set(e.to_string()),
            });
    }
}

/// The GUI: `{{name}}` or `{{name}} open`.
pub fn open() -> i32 {
    // The app id is the short name: Wayland app id = {{name}}.desktop.
    let app = App::new(NAME);
    if let Err(e) = crate::load_catalogs() {
        eprintln!("{NAME}: translations: {}", e.0);
    }
    let file = backend::data_file();
    let (tally, file, problem) = match backend::load(&file) {
        Ok(n) => (Tally::new(n), Some(file), None),
        Err(e) => (Tally::new(0), None, Some(e)),
    };
    let mut ui = build(tally, file);
    if let Some(e) = problem {
        ui.notice.set(tr!("Could not read the saved tally: {error}", error = e));
    }
    ui.win.show();
    app.run();
    0
}
