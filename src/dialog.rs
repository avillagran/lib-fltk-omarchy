//! Open and save dialogs, the Omarchy way: through xdg-desktop-portal
//! (`org.freedesktop.portal.FileChooser` over D-Bus) on Linux, so the
//! desktop's own chooser opens and sandboxed apps get their files, then
//! FLTK's native chooser when there is no portal; macOS and Windows use
//! FLTK's native chooser (`NSOpenPanel`, `IFileDialog`).
//!
//! ```no_run
//! use fltk_omarchy::dialog::{self, Filter};
//! let md = [Filter::new("Markdown").glob("*.md").mime("text/markdown")];
//! // Blocking, but the UI keeps drawing and running timers meanwhile:
//! if let Some(paths) = dialog::open_files("Open notes", &md, true, false) {
//!     println!("{paths:?}");
//! }
//! // Or with a callback on the UI thread (no nested event loop):
//! dialog::save_file_async("Export", "notes.md", &md, |path| {
//!     if let Some(p) = path { println!("save to {}", p.display()); }
//! });
//! // Everything, with the reason when it fails ("say what's wrong"):
//! match dialog::Request::open("Pick a folder").directory(true).run() {
//!     Ok(paths) => println!("{paths:?}"),
//!     Err(dialog::Error::Cancelled) => {}
//!     Err(e) => eprintln!("{e}"), // show it in the window
//! }
//! ```
//!
//! Order: a picker set with [`set_picker`] (tests, remote pickers), else
//! the portal, else the native chooser. `OMARCHY_FLTK_FILE_DIALOG=portal`
//! (portal only), `native` (skip the portal) or `none` (no dialogs: CI,
//! kiosks) changes it. Scripted test runs (`OMARCHY_FLTK_SCRIPT`,
//! docs/TESTKIT.md) never open dialogs.
//!
//! Every function runs on the UI thread. One dialog is open at a time; a
//! second request meanwhile gets [`Error::Busy`].

use crate::tr;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};

/// One file type: a name shown in the chooser and its patterns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Filter {
    name: String,
    patterns: Vec<Pattern>,
}

/// A pattern of a [`Filter`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pattern {
    /// A shell glob (`*.png`); case-sensitive in most choosers.
    Glob(String),
    /// A MIME type (`image/png`), used by the portal. FLTK's native
    /// chooser only understands globs and skips these.
    Mime(String),
}

impl Filter {
    pub fn new(name: impl Into<String>) -> Self {
        Filter { name: name.into(), patterns: Vec::new() }
    }

    /// Adds a glob (`*.png`).
    pub fn glob(mut self, pattern: impl Into<String>) -> Self {
        self.patterns.push(Pattern::Glob(pattern.into()));
        self
    }

    /// Adds a MIME type (`image/png`; portal only).
    pub fn mime(mut self, mime: impl Into<String>) -> Self {
        self.patterns.push(Pattern::Mime(mime.into()));
        self
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn patterns(&self) -> &[Pattern] {
        &self.patterns
    }
}

/// Why no paths came back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The user closed the dialog without picking anything.
    Cancelled,
    /// Another dialog of this app is still open.
    Busy,
    /// No chooser could be shown; the reason says which ones were tried.
    Unavailable(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Cancelled => f.write_str(&tr!("Cancelled")),
            Error::Busy => f.write_str(&tr!("A file dialog is already open")),
            Error::Unavailable(why) => {
                f.write_str(&tr!("No file dialog is available: {reason}", reason = why))
            }
        }
    }
}

impl std::error::Error for Error {}

/// An open or save dialog. Build with [`Request::open`] or
/// [`Request::save`], then [`Request::run`] or [`Request::run_async`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Request {
    /// A save dialog (else open).
    pub save: bool,
    /// Window title.
    pub title: String,
    /// Open: several entries may be picked.
    pub multiple: bool,
    /// Open: folders instead of files.
    pub directory: bool,
    /// Save: the suggested file name.
    pub name: Option<String>,
    /// The folder the dialog starts in.
    pub folder: Option<PathBuf>,
    /// The text of the accept button (the chooser's own by default).
    pub accept_label: Option<String>,
    /// File types; the first one is selected.
    pub filters: Vec<Filter>,
}

impl Request {
    /// An open dialog for one file.
    pub fn open(title: impl Into<String>) -> Self {
        Request { title: title.into(), ..Default::default() }
    }

    /// A save dialog.
    pub fn save(title: impl Into<String>) -> Self {
        Request { save: true, title: title.into(), ..Default::default() }
    }

    pub fn multiple(mut self, on: bool) -> Self {
        self.multiple = on;
        self
    }

    pub fn directory(mut self, on: bool) -> Self {
        self.directory = on;
        self
    }

    /// The suggested file name of a save dialog.
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn folder(mut self, dir: impl AsRef<Path>) -> Self {
        self.folder = Some(dir.as_ref().to_path_buf());
        self
    }

    pub fn accept_label(mut self, label: impl Into<String>) -> Self {
        self.accept_label = Some(label.into());
        self
    }

    pub fn filter(mut self, f: Filter) -> Self {
        self.filters.push(f);
        self
    }

    pub fn filters(mut self, f: impl IntoIterator<Item = Filter>) -> Self {
        self.filters.extend(f);
        self
    }

    /// Shows the dialog and waits for the answer. While the portal dialog
    /// is open the app's event loop keeps running (redraws, timers, posted
    /// work), like any modal dialog. Paths are absolute.
    pub fn run(&self) -> Result<Vec<PathBuf>, Error> {
        if let Some(r) = PICKER.with(|p| p.borrow_mut().as_mut().map(|f| f(self))) {
            return r;
        }
        let order = backends()?;
        self.run_in(&order)
    }

    fn run_in(&self, order: &[Backend]) -> Result<Vec<PathBuf>, Error> {
        let _busy = BusyGuard::enter()?;
        let mut why = Vec::new();
        for b in order {
            let r = match *b {
                Backend::Portal => portal_blocking(self),
                Backend::Native => native(self),
            };
            match r {
                Err(Error::Unavailable(w)) => why.push(w),
                r => return r,
            }
        }
        Err(Error::Unavailable(why.join("; ")))
    }

    /// Shows the dialog and returns at once; `f` gets the answer later on
    /// the UI thread (from [`crate::App::run`]'s loop). No nested event
    /// loop runs while the portal dialog is open. The native fallback is
    /// modal and runs inside the UI loop when its turn comes.
    pub fn run_async(self, f: impl FnOnce(Result<Vec<PathBuf>, Error>) + 'static) {
        if let Some(r) = PICKER.with(|p| p.borrow_mut().as_mut().map(|p| p(&self))) {
            return f(r);
        }
        if BUSY.with(Cell::get) {
            return f(Err(Error::Busy));
        }
        let order = match backends() {
            Ok(o) => o,
            Err(e) => return f(Err(e)),
        };
        BUSY.with(|b| b.set(true));
        let id = NEXT_ID.with(|n| {
            let id = n.get();
            n.set(id + 1);
            id
        });
        WAITING.with(|w| w.borrow_mut().insert(id, Box::new(f)));
        if order.first() == Some(&Backend::Portal) {
            let parent = crate::platform::portal_parent();
            let native_next = order.contains(&Backend::Native);
            std::thread::spawn(move || {
                let r = crate::platform::file_portal(&self, &parent);
                crate::task::post(move || {
                    let r = match r {
                        Err(Error::Unavailable(w)) if native_next => {
                            native(&self).map_err(|e| match e {
                                Error::Unavailable(n) => Error::Unavailable(format!("{w}; {n}")),
                                e => e,
                            })
                        }
                        r => r,
                    };
                    finish(id, r);
                });
            });
        } else {
            crate::task::post(move || finish(id, native(&self)));
        }
    }
}

/// Open dialog: `Some(paths)` (one unless `multiple`), `None` when
/// cancelled or when no chooser could be shown (said on stderr; use
/// [`Request::run`] to show the reason). An empty `title` gets "Open" or
/// "Open folder". The UI keeps running while it is open.
pub fn open_files(
    title: &str,
    filters: &[Filter],
    multiple: bool,
    directory: bool,
) -> Option<Vec<PathBuf>> {
    quiet(open_request(title, filters, multiple, directory).run())
}

/// Save dialog: `Some(path)` or `None` (as [`open_files`]). The chooser
/// asks before replacing an existing file. An empty `title` gets "Save".
pub fn save_file(title: &str, suggested_name: &str, filters: &[Filter]) -> Option<PathBuf> {
    quiet(save_request(title, suggested_name, filters).run()).and_then(|p| p.into_iter().next())
}

/// [`open_files`] with the answer given to `f` on the UI thread later.
pub fn open_files_async(
    title: &str,
    filters: &[Filter],
    multiple: bool,
    directory: bool,
    f: impl FnOnce(Option<Vec<PathBuf>>) + 'static,
) {
    open_request(title, filters, multiple, directory).run_async(move |r| f(quiet(r)));
}

/// [`save_file`] with the answer given to `f` on the UI thread later.
pub fn save_file_async(
    title: &str,
    suggested_name: &str,
    filters: &[Filter],
    f: impl FnOnce(Option<PathBuf>) + 'static,
) {
    save_request(title, suggested_name, filters)
        .run_async(move |r| f(quiet(r).and_then(|p| p.into_iter().next())));
}

/// A replacement chooser for [`set_picker`].
pub type Picker = Box<dyn FnMut(&Request) -> Result<Vec<PathBuf>, Error>>;

/// Replaces every dialog of this thread with `f` (tests, a remote or
/// in-app picker); `None` restores the system choosers. `f` answers each
/// [`Request`] synchronously.
pub fn set_picker(f: Option<Picker>) {
    PICKER.with(|p| *p.borrow_mut() = f);
}

/// True while a dialog of this app is open.
pub fn is_open() -> bool {
    BUSY.with(Cell::get)
}

// ---------------------------------------------------------------- internals

type Waiting = Box<dyn FnOnce(Result<Vec<PathBuf>, Error>)>;

thread_local! {
    static PICKER: RefCell<Option<Picker>> = const { RefCell::new(None) };
    static BUSY: Cell<bool> = const { Cell::new(false) };
    static NEXT_ID: Cell<u64> = const { Cell::new(1) };
    static WAITING: RefCell<HashMap<u64, Waiting>> = RefCell::new(HashMap::new());
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Backend {
    Portal,
    Native,
}

/// The choosers to try, in order, from `OMARCHY_FLTK_FILE_DIALOG`.
fn backends() -> Result<Vec<Backend>, Error> {
    if crate::testkit::env::active() {
        return Err(Error::Unavailable(
            "file dialogs are off during scripted runs (OMARCHY_FLTK_SCRIPT)".into(),
        ));
    }
    let v = std::env::var("OMARCHY_FLTK_FILE_DIALOG").unwrap_or_default();
    order_for(&v, crate::platform::has_file_portal())
}

fn order_for(setting: &str, has_portal: bool) -> Result<Vec<Backend>, Error> {
    match setting.trim() {
        "none" | "off" => Err(Error::Unavailable("OMARCHY_FLTK_FILE_DIALOG=none".into())),
        "native" => Ok(vec![Backend::Native]),
        "portal" => Ok(vec![Backend::Portal]),
        _ if has_portal => Ok(vec![Backend::Portal, Backend::Native]),
        _ => Ok(vec![Backend::Native]),
    }
}

struct BusyGuard;

impl BusyGuard {
    fn enter() -> Result<BusyGuard, Error> {
        if BUSY.with(|b| b.replace(true)) {
            return Err(Error::Busy);
        }
        Ok(BusyGuard)
    }
}

impl Drop for BusyGuard {
    fn drop(&mut self) {
        BUSY.with(|b| b.set(false));
    }
}

fn finish(id: u64, r: Result<Vec<PathBuf>, Error>) {
    BUSY.with(|b| b.set(false));
    if let Some(f) = WAITING.with(|w| w.borrow_mut().remove(&id)) {
        f(r);
    }
}

fn quiet<T>(r: Result<T, Error>) -> Option<T> {
    match r {
        Ok(v) => Some(v),
        Err(Error::Cancelled) => None,
        Err(e) => {
            eprintln!("fltk-omarchy: {e}");
            None
        }
    }
}

fn open_request(title: &str, filters: &[Filter], multiple: bool, directory: bool) -> Request {
    let title = if !title.is_empty() {
        title.to_string()
    } else if directory {
        tr!("Open folder")
    } else {
        tr!("Open")
    };
    Request::open(title)
        .multiple(multiple)
        .directory(directory)
        .filters(filters.iter().cloned())
}

fn save_request(title: &str, suggested_name: &str, filters: &[Filter]) -> Request {
    let title = if title.is_empty() { tr!("Save") } else { title.to_string() };
    let mut r = Request::save(title).filters(filters.iter().cloned());
    if !suggested_name.is_empty() {
        r = r.name(suggested_name);
    }
    r
}

/// The portal on a worker thread; this thread keeps the event loop going
/// while windows are shown (else it just waits).
fn portal_blocking(req: &Request) -> Result<Vec<PathBuf>, Error> {
    use std::sync::mpsc::{channel, TryRecvError};
    let parent = crate::platform::portal_parent();
    let (tx, rx) = channel();
    let r = req.clone();
    std::thread::spawn(move || {
        let _ = tx.send(crate::platform::file_portal(&r, &parent));
        fltk::app::awake();
    });
    let gone = || Error::Unavailable("the portal thread ended".into());
    let pump = fltk::app::windows().is_some_and(|w| !w.is_empty());
    if !pump {
        return rx.recv().unwrap_or_else(|_| Err(gone()));
    }
    crate::testkit::frames::nested_loop();
    loop {
        match rx.try_recv() {
            Ok(r) => return r,
            Err(TryRecvError::Disconnected) => return Err(gone()),
            Err(TryRecvError::Empty) => {
                let _ = fltk::app::wait_for(0.1);
                crate::task::drain();
            }
        }
    }
}

/// FLTK's filter string: `Name\t*.a` per line, several globs as `{*.a,*.b}`.
fn native_filter(filters: &[Filter]) -> String {
    filters
        .iter()
        .filter_map(|f| {
            let globs: Vec<&str> = f
                .patterns
                .iter()
                .filter_map(|p| match p {
                    Pattern::Glob(g) => Some(g.as_str()),
                    Pattern::Mime(_) => None,
                })
                .collect();
            match globs.len() {
                0 => None,
                1 => Some(format!("{}\t{}", f.name, globs[0])),
                _ => Some(format!("{}\t{{{}}}", f.name, globs.join(","))),
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// FLTK's native chooser (GTK, zenity or kdialog, or FLTK's own on
/// Linux; the system panels on macOS and Windows).
fn native(req: &Request) -> Result<Vec<PathBuf>, Error> {
    use fltk::dialog::{
        NativeFileChooser as Chooser, NativeFileChooserOptions as Opt,
        NativeFileChooserType as Kind,
    };
    let kind = match (req.save, req.directory, req.multiple) {
        (true, _, _) => Kind::BrowseSaveFile,
        (false, true, false) => Kind::BrowseDir,
        (false, true, true) => Kind::BrowseMultiDir,
        (false, false, true) => Kind::BrowseMultiFile,
        (false, false, false) => Kind::BrowseFile,
    };
    let mut fc = Chooser::new(kind);
    fc.set_title(&req.title);
    if req.save {
        fc.set_option(Opt::SaveAsConfirm | Opt::NewFolder);
        if let Some(n) = &req.name {
            fc.set_preset_file(n);
        }
    }
    if let Some(dir) = &req.folder {
        let _ = fc.set_directory(dir);
    }
    let filter = native_filter(&req.filters);
    if !filter.is_empty() {
        fc.set_filter(&filter);
    }
    crate::testkit::frames::nested_loop();
    fc.show();
    let paths: Vec<PathBuf> = fc
        .filenames()
        .into_iter()
        .filter(|p| !p.as_os_str().is_empty())
        .collect();
    if !paths.is_empty() {
        return Ok(paths);
    }
    match fc.error_message() {
        Some(e) if !e.is_empty() && e != "No error" => {
            Err(Error::Unavailable(format!("native file chooser: {e}")))
        }
        _ => Err(Error::Cancelled),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_order() {
        assert_eq!(order_for("", true), Ok(vec![Backend::Portal, Backend::Native]));
        assert_eq!(order_for("", false), Ok(vec![Backend::Native]));
        assert_eq!(order_for("native", true), Ok(vec![Backend::Native]));
        assert_eq!(order_for("portal", false), Ok(vec![Backend::Portal]));
        assert!(matches!(order_for("none", true), Err(Error::Unavailable(_))));
    }

    #[test]
    fn native_filters() {
        let f = [
            Filter::new("Images").glob("*.png").glob("*.jpg").mime("image/webp"),
            Filter::new("Only MIME").mime("text/plain"),
            Filter::new("Text").glob("*.txt"),
        ];
        assert_eq!(native_filter(&f), "Images\t{*.png,*.jpg}\nText\t*.txt");
        assert_eq!(native_filter(&[]), "");
    }

    #[test]
    fn simple_requests() {
        let r = open_request("", &[], true, true);
        assert!(!r.save && r.multiple && r.directory);
        assert!(!r.title.is_empty());
        let r = save_request("Export", "a.md", &[Filter::new("Md").glob("*.md")]);
        assert!(r.save);
        assert_eq!(r.name.as_deref(), Some("a.md"));
        assert_eq!(r.filters.len(), 1);
        assert_eq!(save_request("", "", &[]).name, None);
    }

    #[test]
    fn picker_replaces_dialogs() {
        set_picker(Some(Box::new(|r: &Request| {
            if r.save {
                Ok(vec![PathBuf::from("/out").join(r.name.clone().unwrap_or_default())])
            } else {
                Err(Error::Cancelled)
            }
        })));
        assert_eq!(save_file("", "x.txt", &[]), Some(PathBuf::from("/out/x.txt")));
        assert_eq!(open_files("", &[], false, false), None);
        let got = std::rc::Rc::new(Cell::new(false));
        let g = got.clone();
        save_file_async("", "y", &[], move |p| {
            assert_eq!(p, Some(PathBuf::from("/out/y")));
            g.set(true);
        });
        assert!(got.get());
        set_picker(None);
    }

    #[test]
    fn busy_while_open() {
        let g = BusyGuard::enter().unwrap();
        assert!(is_open());
        assert_eq!(BusyGuard::enter().err(), Some(Error::Busy));
        let r = std::rc::Rc::new(RefCell::new(None));
        let r2 = r.clone();
        Request::open("x").run_async(move |v| *r2.borrow_mut() = Some(v));
        assert_eq!(*r.borrow(), Some(Err(Error::Busy)));
        drop(g);
        assert!(!is_open());
    }

    /// The whole chain against a fake portal on a private bus: the
    /// blocking call (no window shown: it just waits) and the async one
    /// (answered through task::post).
    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn fake_portal_end_to_end() {
        use crate::platform::portal::{fake, set_test_bus};
        let f = fake::start(
            "dialog",
            fake::Answer::Respond { code: 0, uris: vec!["file:///home/me/pic%201.png".into()] },
        );
        set_test_bus(Some(f.address.clone()));
        // Portal only: a failure must not open a real native dialog here.
        let r = Request::open("Open").run_in(&[Backend::Portal]);
        assert_eq!(r, Ok(vec![PathBuf::from("/home/me/pic 1.png")]));
        assert!(!is_open());
        let call = f.calls.recv().unwrap();
        assert_eq!(call.body[1].as_str(), Some("Open"));

        let f = fake::start("dialog-async", fake::Answer::Respond { code: 1, uris: vec![] });
        set_test_bus(Some(f.address.clone()));
        let (tx, rx) = std::sync::mpsc::channel();
        let req = Request::save("Save").name("a.txt");
        std::thread::spawn(move || {
            let _ = tx.send(crate::platform::file_portal(&req, ""));
        });
        assert_eq!(rx.recv().unwrap(), Err(Error::Cancelled));
        set_test_bus(None);
    }
}
