//! `AppWindow`: an Omarchy application window = header (title + actions)
//! over a padded body column. Normal (tiled) by default; `Mode::Floating`
//! floats it on Hyprland at the requested size, the header drags it and
//! the bottom corners resize it: natively through the compositor on
//! Wayland, through Hyprland IPC on this window only under XWayland, and
//! by moving/resizing the FLTK window itself elsewhere (X11 window
//! managers, macOS, Windows), see docs/PLATFORMS.md.

use crate::paint;
use crate::theme;
use crate::widgets::{measure_offscreen, ActionButton};
use crate::{hyprland, png};
use fltk::{app, enums::*, group::Flex, prelude::*, widget::Widget, window::DoubleWindow};
use std::cell::{Cell, RefCell};
use std::path::Path;
use std::rc::Rc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// The compositor decides (tiled on Omarchy).
    Normal,
    /// Floating at the requested size; header drag + corner resize.
    Floating,
    /// Native fullscreen (FLTK); leave it through the app's own controls.
    Fullscreen,
}

impl Mode {
    /// Parses the config value used by Files' `window_mode` and similar.
    pub fn parse(s: &str) -> Option<Mode> {
        match s.trim().to_ascii_lowercase().as_str() {
            "normal" => Some(Mode::Normal),
            "floating" | "float" => Some(Mode::Floating),
            "fullscreen" => Some(Mode::Fullscreen),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Normal => "normal",
            Mode::Floating => "floating",
            Mode::Fullscreen => "fullscreen",
        }
    }
}

const GRIP: i32 = 14;

#[derive(Clone)]
pub struct AppWindow {
    win: DoubleWindow,
    header: Header,
    body: Flex,
    mode: Rc<Cell<Mode>>,
    size: (i32, i32),
    min: Rc<Cell<(i32, i32)>>,
    shortcuts: Rc<RefCell<Vec<(crate::keys::Keys, Box<dyn FnMut()>)>>>,
    table: crate::shortcuts::Shortcuts,
    conventions: Rc<RefCell<Conventions>>,
}

impl std::ops::Deref for AppWindow {
    type Target = DoubleWindow;
    fn deref(&self) -> &DoubleWindow {
        &self.win
    }
}
impl std::ops::DerefMut for AppWindow {
    fn deref_mut(&mut self) -> &mut DoubleWindow {
        &mut self.win
    }
}

/// Who moves the window during a header/corner drag that the compositor
/// does not run natively.
enum DragTarget {
    /// Hyprland IPC on this window (XWayland on Hyprland).
    Hyprland(String),
    /// FLTK moves/resizes the window itself (X11 window managers, macOS,
    /// Windows); `min` is the window's minimum size.
    Fltk(Box<dyn WindowExt>, (i32, i32)),
}

struct Drag {
    target: DragTarget,
    origin: (i32, i32),
    size: (i32, i32),
    cursor: (i32, i32),
    corner: u8, // 0 move, 1 bottom-left, 2 bottom-right
    last: std::time::Instant,
}

thread_local! {
    static DRAG: RefCell<Option<Drag>> = const { RefCell::new(None) };
}

/// Starts a fallback drag of `w`'s window (after the native gesture failed).
fn drag_begin<W: WidgetExt>(w: &W, title: &str, corner: u8, min: (i32, i32)) {
    let last = std::time::Instant::now();
    if let Some(c) = hyprland::own_clients(Some(title)).into_iter().next() {
        let Some(cursor) = hyprland::cursor() else { return };
        DRAG.with(|d| *d.borrow_mut() = Some(Drag { target: DragTarget::Hyprland(c.address), origin: c.at, size: c.size, cursor, corner, last }));
        return;
    }
    // Wayland clients cannot place themselves: nothing more to try there.
    if crate::platform::wayland() {
        return;
    }
    let Some(win) = w.top_window() else { return };
    let cursor = (app::event_x_root(), app::event_y_root());
    let (origin, size) = ((win.x(), win.y()), (win.w(), win.h()));
    DRAG.with(|d| *d.borrow_mut() = Some(Drag { target: DragTarget::Fltk(win, min), origin, size, cursor, corner, last }));
}

fn drag_update() {
    DRAG.with(|d| {
        let mut d = d.borrow_mut();
        let Some(st) = d.as_mut() else { return };
        match &mut st.target {
            DragTarget::Hyprland(address) => {
                if st.last.elapsed().as_millis() < 16 {
                    return;
                }
                st.last = std::time::Instant::now();
                let Some((cx, cy)) = hyprland::cursor() else { return };
                let (dx, dy) = (cx - st.cursor.0, cy - st.cursor.1);
                match st.corner {
                    0 => { hyprland::move_to(address, st.origin.0 + dx, st.origin.1 + dy); }
                    1 => {
                        hyprland::move_to(address, st.origin.0 + dx, st.origin.1);
                        hyprland::resize_to(address, st.size.0 - dx, st.size.1 + dy);
                    }
                    _ => { hyprland::resize_to(address, st.size.0 + dx, st.size.1 + dy); }
                }
            }
            DragTarget::Fltk(win, min) => {
                let (dx, dy) = (app::event_x_root() - st.cursor.0, app::event_y_root() - st.cursor.1);
                let (ox, oy, w, h) = (st.origin.0, st.origin.1, st.size.0, st.size.1);
                match st.corner {
                    0 => win.set_pos(ox + dx, oy + dy),
                    1 => {
                        let nw = (w - dx).max(min.0);
                        win.resize(ox + w - nw, oy, nw, (h + dy).max(min.1));
                    }
                    _ => win.resize(ox, oy, (w + dx).max(min.0), (h + dy).max(min.1)),
                }
            }
        }
    });
}

fn drag_end() {
    DRAG.with(|d| *d.borrow_mut() = None);
}

impl AppWindow {
    /// Creates the window and leaves its **body** open, so widgets created
    /// next are added to the body column. Call [`AppWindow::end`] when done.
    pub fn new(title: &str, width: i32, height: i32) -> AppWindow {
        let t = theme::current();
        let mut win = DoubleWindow::default().with_size(width, height).with_label(title);
        win.set_color(t.palette.background.fl());
        win.size_range((width / 2).max(200), (height / 2).max(150), 0, 0);
        let mut root = Flex::default_fill().column();
        root.set_spacing(0);
        let header = Header::new(title);
        root.fixed(&*header, header.h());
        let mut body = Flex::default().column();
        body.set_margin(t.spacing.panel_padding);
        body.set_spacing(t.spacing.panel_gap);
        root.end();
        win.resizable(&root);
        win.end();
        body.begin();

        let mode = Rc::new(Cell::new(Mode::Normal));
        let shortcuts: Rc<RefCell<Vec<(crate::keys::Keys, Box<dyn FnMut()>)>>> = Rc::default();
        let min = Rc::new(Cell::new(((width / 2).max(200), (height / 2).max(150))));
        let table = crate::shortcuts::Shortcuts::new();
        let conventions: Rc<RefCell<Conventions>> = Rc::default();
        let conv = conventions.clone();
        let (m2, sc, mn) = (mode.clone(), shortcuts.clone(), min.clone());
        let table2 = table.clone();
        win.handle(move |w, ev| {
            let table = &table2;
            let floating = m2.get() == Mode::Floating;
            match ev {
                Event::Push if floating && app::event_y() >= w.h() - GRIP => {
                    let corner = if app::event_x() < GRIP { 1 } else if app::event_x() >= w.w() - GRIP { 2 } else { 0 };
                    if corner > 0 {
                        // Native compositor resize first (any Wayland compositor), Hyprland IPC otherwise.
                        let edge = if corner == 1 { crate::native::Edge::BottomLeft } else { crate::native::Edge::BottomRight };
                        if !crate::native::begin_resize(w, edge) {
                            // Current title: it may have been translated since.
                            drag_begin(w, &w.label(), corner, mn.get());
                        }
                        return true;
                    }
                    false
                }
                Event::Drag if DRAG.with(|d| d.borrow().as_ref().is_some_and(|d| d.corner > 0)) => { drag_update(); true }
                Event::Released if DRAG.with(|d| d.borrow().is_some()) => { drag_end(); true }
                Event::Shortcut | Event::KeyDown => {
                    if crate::keys::suspended() {
                        return false;
                    }
                    // Take the callback out while it runs: it may add
                    // shortcuts or open a window that handles keys.
                    let hit = sc.borrow().iter().position(|(k, _)| k.matches_event());
                    if let Some(i) = hit {
                        let mut f = std::mem::replace(&mut sc.borrow_mut()[i].1, Box::new(|| {}));
                        f();
                        if let Some(slot) = sc.borrow_mut().get_mut(i) {
                            slot.1 = f;
                        }
                        return true;
                    }
                    if table.dispatch() {
                        return true;
                    }
                    omarchy_key(w, &conv)
                }
                _ => false,
            }
        });
        let m3 = mode.clone();
        win.draw(move |w| {
            let _frame = crate::testkit::frames::Frame::begin(w);
            // Only views that can move their own pixels scrolled: let FLTK
            // update just them, without repainting the background under them.
            if w.damage_type() == Damage::Child && scroll_only(w) {
                // SAFETY: the window being drawn, only read.
                let root = unsafe { fltk::group::Group::from_widget_ptr(w.as_widget_ptr()) };
                theme::with(|t| draw_planned(&root, t.palette.background));
                return;
            }
            // The window background is Omarchy's glass under Glassy windows.
            theme::with(|t| crate::glass::fill(0, 0, w.w(), w.h(), 0, t.palette.background));
            draw_stack(w);
            if m3.get() == Mode::Floating {
                theme::with(|t| {
                    let c = t.palette.foreground.darker(1.4).fl();
                    let (b, s) = (w.h() - 3, 9);
                    fltk::draw::set_draw_color(c);
                    fltk::draw::draw_polygon(w.w() - 3, b - s, w.w() - 3, b, w.w() - 3 - s, b);
                    fltk::draw::draw_polygon(3, b - s, 3, b, 3 + s, b);
                });
            }
        });
        // cfltk runs FLTK's own draw before this closure by default: it would
        // draw damaged children first and this background would then cover them.
        win.super_draw(false);
        // Escape does not close Omarchy app windows by default.
        win.set_callback(|w| {
            if app::event() == Event::Close {
                w.hide();
            }
        });
        crate::theme::on_change_while(&win, {
            let mut w = win.clone();
            move |t| { w.set_color(t.palette.background.fl()); w.redraw(); }
        });
        crate::testkit::note_window(&win);
        AppWindow { win, header, body, mode, size: (width, height), min, shortcuts, table, conventions }
    }

    /// Closes the body column (and the window group). Every `Flex` row
    /// inside then follows the writing direction
    /// ([`layout::mirror_rows`](crate::layout::mirror_rows)).
    pub fn end(&mut self) {
        self.body.end();
        self.win.end();
        crate::layout::mirror_rows(&self.win);
    }

    /// Sets the window title (taskbar, compositor) and the header title.
    pub fn set_title(&mut self, title: &str) {
        self.win.set_label(title);
        self.header.set_title(title);
    }

    /// Keeps both titles in sync with a State (`&i18n::text("Files")`).
    pub fn bind_title(&mut self, s: &crate::State<String>) {
        let (mut w, mut h) = (self.win.clone(), self.header.clone());
        crate::state::bind_widget(&self.win, s, true, move |v| {
            w.set_label(v);
            h.set_title(v);
        });
    }

    pub fn header(&self) -> Header {
        self.header.clone()
    }

    /// The padded column under the header (panel padding 18, gap 14).
    pub fn body(&self) -> Flex {
        self.body.clone()
    }

    pub fn set_min_size(&mut self, w: i32, h: i32) {
        self.min.set((w, h));
        self.win.size_range(w, h, 0, 0);
    }

    pub fn mode(&self) -> Mode {
        self.mode.get()
    }

    /// Switches mode; when shown on Hyprland, floats/tiles this window now.
    pub fn set_mode(&mut self, mode: Mode) {
        let previous = self.mode.replace(mode);
        self.header.set_draggable(mode == Mode::Floating);
        if self.win.shown() {
            if mode == Mode::Fullscreen {
                self.win.fullscreen(true);
                self.win.redraw();
                return;
            }
            if previous == Mode::Fullscreen || self.win.fullscreen_active() {
                self.win.fullscreen(false);
            }
        }
        if self.win.shown() && hyprland::available() {
            let title = self.win.label();
            let size = self.size;
            if let Some(c) = hyprland::own_clients(Some(&title)).into_iter().next() {
                hyprland::set_floating(&c.address, mode == Mode::Floating);
                if mode == Mode::Floating {
                    hyprland::resize_to(&c.address, size.0, size.1);
                    hyprland::center(&c.address);
                }
            }
        }
        self.win.redraw();
    }

    /// Keyboard shortcut for the whole window: `win.shortcut("Ctrl+Q", || app::quit())`.
    /// Omarchy's help key: `?` and F1 open the shortcut overlay titled
    /// `title` with the sections `sections()` returns (built on each open, so
    /// they follow the language). Keys a focused control uses are not taken.
    pub fn help_overlay(&self, title: &str, sections: impl Fn() -> Vec<(String, Vec<(String, String)>)> + 'static) {
        self.conventions.borrow_mut().help = Some((title.to_string(), Rc::new(sections)));
    }

    /// Omarchy's quit key: `Q` quits the app, asking first when
    /// `unsaved()` says there is work that would be lost.
    pub fn quit_key(&self, unsaved: impl Fn() -> bool + 'static) {
        self.conventions.borrow_mut().quit = Some(Rc::new(unsaved));
    }

    /// The window's command shortcut table: keys bound to command ids with
    /// conditions, rebound at run time ([`crate::shortcuts`]). Keys from
    /// [`AppWindow::shortcut`] are tried first.
    pub fn shortcuts(&self) -> crate::shortcuts::Shortcuts {
        self.table.clone()
    }

    #[track_caller]
    pub fn shortcut(&mut self, spec: &str, f: impl FnMut() + 'static) {
        let keys = crate::keys::parse(spec).unwrap_or_else(|e| panic!("{e}"));
        let (site, mut f) = (std::panic::Location::caller(), f);
        let f = move || crate::testkit::frames::watch(site, &mut f);
        self.shortcuts.borrow_mut().push((keys, Box::new(f)));
    }

    /// Shows the window. In floating mode on Hyprland the window is mapped
    /// with a fixed size so Hyprland floats it at the requested size
    /// (smaller first buffers), then the size range is relaxed.
    pub fn show(&mut self) {
        // Scripted runs stay offscreen (docs/TESTKIT.md).
        if crate::testkit::env::headless() {
            return;
        }
        if self.mode.get() == Mode::Floating && hyprland::available() && !self.win.shown() {
            // Map normally, then float it as soon as Hyprland lists the window.
            // (A fixed size range at map time makes Hyprland float it too, but
            // the window then stays fixed-size for good: it cannot be tiled or
            // resized afterwards, not even by IPC.)
            let size = (self.win.w(), self.win.h());
            self.win.show();
            let title = self.win.label();
            let tries = Cell::new(0u32);
            app::add_timeout3(0.02, move |h| {
                if let Some(c) = hyprland::own_clients(Some(&title)).into_iter().next() {
                    if !c.floating {
                        hyprland::set_floating(&c.address, true);
                    }
                    hyprland::resize_to(&c.address, size.0, size.1);
                    hyprland::center(&c.address);
                } else if tries.replace(tries.get() + 1) < 100 {
                    app::repeat_timeout3(0.02, h);
                }
            });
            self.header.set_draggable(true);
        } else {
            self.win.show();
            if self.mode.get() == Mode::Fullscreen {
                self.win.fullscreen(true);
            } else if self.mode.get() == Mode::Floating {
                let m = self.mode.get();
                self.set_mode(m);
            }
        }
    }

    /// Shows the window (when hidden) and brings it to the front with the
    /// keyboard focus, as far as the OS lets an app do it: FLTK restores
    /// and raises it (X11: `_NET_ACTIVE_WINDOW`), macOS activates the app,
    /// Windows foregrounds it, Hyprland focuses it (`focuswindow`, on this
    /// window only). Other Wayland compositors decide themselves (FLTK has
    /// no xdg-activation): a newly mapped window usually gets the focus.
    /// For single-instance apps, see [`crate::instance`].
    pub fn raise(&mut self) {
        if self.win.shown() {
            self.win.show(); // restore + raise
        } else {
            self.show();
        }
        let has_focus = app::focus().is_some_and(|f| f.inside(&self.win));
        if !has_focus {
            // Fl::focus() on a window the OS has not focused asks the OS
            // for it (X11 EWMH activation); the window restores its own
            // last focused widget.
            let _ = self.win.take_focus();
        }
        crate::platform::instance::raise_native(&self.win);
        if hyprland::available() {
            // A window being mapped is listed a moment later.
            let title = self.win.label();
            let tries = Cell::new(0u32);
            app::add_timeout3(0.0, move |h| {
                if let Some(c) = hyprland::own_clients(Some(&title)).into_iter().next() {
                    hyprland::focus(&c.address);
                } else if tries.replace(tries.get() + 1) < 50 {
                    app::repeat_timeout3(0.02, h);
                }
            });
        }
    }

    /// True when this window is shown and has the keyboard focus. On
    /// Hyprland the compositor's active window decides; elsewhere FLTK's
    /// focus (cleared when the OS takes the focus away).
    pub fn is_active(&self) -> bool {
        if !self.win.shown() {
            return false;
        }
        if hyprland::available() {
            if let Some((pid, _, title)) = hyprland::active_window() {
                return pid == i64::from(std::process::id()) && title == self.win.label();
            }
        }
        app::focus().is_some_and(|f| f.inside(&self.win))
    }

    /// Toggles like a launcher button: hides the window when it is shown
    /// and active, raises it otherwise. Returns true when it is shown after.
    pub fn toggle(&mut self) -> bool {
        if self.win.shown() && self.is_active() {
            self.win.hide();
            false
        } else {
            self.raise();
            true
        }
    }

    /// Renders the window contents offscreen into a PNG (works before
    /// `show()`), so tests and agents can look at the UI.
    pub fn snapshot(&mut self, path: impl AsRef<Path>) -> std::io::Result<()> {
        snapshot(&mut self.win, path.as_ref())
    }
}

extern "C" {
    // cfltk wrapper of fl_read_image (linked through fltk-sys).
    fn Fl_read_image(p: *mut u8, x: i32, y: i32, w: i32, h: i32, alpha: i32) -> *mut u8;
}

/// The RGB pixels the window shows now (`w × h × 3`), read back from its
/// buffer without raising it — unlike [`snapshot`], which renders the
/// widgets offscreen, this sees what the real draw path produced (partial
/// redraws, scroll blits). For tests; `None` if the window is not shown.
/// Each call leaves FLTK's small read buffer allocated (test use).
pub fn read_pixels(win: &mut DoubleWindow) -> Option<Vec<u8>> {
    if !win.shown() {
        return None;
    }
    let (w, h) = (win.w(), win.h());
    win.make_current();
    // SAFETY: a shown window made current; FLTK returns a w*h*3 buffer.
    let p = unsafe { Fl_read_image(std::ptr::null_mut(), 0, 0, w, h, 0) };
    if p.is_null() {
        return None;
    }
    // SAFETY: as above, the buffer holds w*h*3 bytes.
    Some(unsafe { std::slice::from_raw_parts(p, (w * h * 3) as usize) }.to_vec())
}

/// Offscreen render of any window into a PNG.
pub fn snapshot(win: &mut DoubleWindow, path: &Path) -> std::io::Result<()> {
    let img = render(win)?;
    let data = img.to_rgb_data();
    png::write(path, img.data_w() as u32, img.data_h() as u32, img.depth() as u8, &data)
}

/// Offscreen render of any window (what [`snapshot`] writes).
pub(crate) fn render(win: &mut DoubleWindow) -> std::io::Result<fltk::image::RgbImage> {
    use fltk::surface::ImageSurface;
    let (w, h) = (win.w(), win.h());
    // Lay out flex children for the current size even if never shown.
    let (x, y) = (win.x(), win.y());
    win.resize(x, y, w, h);
    let surf = ImageSurface::new(w, h, false);
    ImageSurface::push_current(&surf);
    theme::with(|t| paint::fill(0, 0, w, h, 0, t.palette.background));
    crate::scene::surface::cpu_scope(|| {
        for i in 0..win.children() {
            if let Some(c) = win.child(i) {
                if c.visible() { crate::scene::surface::draw_cpu_widget(&surf, &c, c.x(), c.y()); }
            }
        }
    });
    let img = surf.image();
    ImageSurface::pop_current();
    img.ok_or_else(|| std::io::Error::other("offscreen render failed"))
}

thread_local! {
    /// Views that paint a scroll by moving their pixels (`RichText`).
    static BLITTERS: RefCell<Vec<(usize, fltk::widget::Widget)>> = const { RefCell::new(Vec::new()) };
}

/// Registers a view whose `Damage::Scroll` frames move pixels and paint
/// only what they expose, so its window may skip repainting under it.
pub(crate) fn register_blitter<W: WidgetExt>(w: &W) {
    // SAFETY: a live widget; the copy only tracks its deletion.
    let wid = unsafe { fltk::widget::Widget::from_widget_ptr(w.as_widget_ptr()) };
    BLITTERS.with(|b| {
        let mut b = b.borrow_mut();
        b.retain(|(_, x)| !x.was_deleted());
        b.push((w.as_widget_ptr() as usize, wid));
    });
}

fn is_blitter(id: usize) -> bool {
    BLITTERS.with(|b| b.borrow().iter().any(|(i, x)| *i == id && !x.was_deleted()))
}

type Rect4 = (i32, i32, i32, i32);

fn overlaps(a: Rect4, b: Rect4) -> bool {
    a.0 < b.0 + b.2 && b.0 < a.0 + a.2 && a.1 < b.1 + b.3 && b.1 < a.1 + a.3
}

/// Plan for a frame where registered views only scrolled: the damaged
/// widgets under `g` are such views (`update`), groups leading to them
/// (`descend`), or other widgets that do not overlap any of them
/// (`repaint`: background under them + full draw). None when something
/// damaged overlaps a scrolled view (the frame is painted in full).
#[derive(Default)]
struct ScrollPlan {
    scrolled: Vec<Rect4>,
    others: Vec<Rect4>,
}

fn plan(g: &fltk::group::Group, p: &mut ScrollPlan) -> bool {
    for i in 0..g.children() {
        let Some(c) = g.child(i) else { continue };
        if !c.visible() || !c.damage() {
            continue;
        }
        let d = c.damage_type();
        let r = (c.x(), c.y(), c.w(), c.h());
        if d == Damage::Scroll && is_blitter(c.as_widget_ptr() as usize) {
            p.scrolled.push(r);
        } else if d == Damage::Child && c.as_group().is_some() {
            if !plan(&c.as_group().unwrap(), p) {
                return false;
            }
        } else {
            p.others.push(r);
        }
    }
    true
}

/// Draws a frame planned by [`plan`]: scrolled views update themselves,
/// other damaged widgets get their background and a full draw.
fn draw_planned(g: &fltk::group::Group, bg: crate::theme::Rgb) {
    for i in 0..g.children() {
        let Some(mut c) = g.child(i) else { continue };
        if !c.visible() || !c.damage() {
            continue;
        }
        let d = c.damage_type();
        if d == Damage::Scroll && is_blitter(c.as_widget_ptr() as usize) {
            g.update_child(&mut c);
        } else if d == Damage::Child && c.as_group().is_some() {
            draw_planned(&c.as_group().unwrap(), bg);
            c.clear_damage();
        } else {
            fltk::draw::push_clip(c.x(), c.y(), c.w(), c.h());
            crate::glass::fill(c.x(), c.y(), c.w(), c.h(), 0, bg);
            g.draw_child(&mut c);
            fltk::draw::pop_clip();
        }
    }
}

/// True when the frame can be drawn by [`draw_planned`]: at least one
/// registered view only scrolled and nothing else damaged overlaps it.
fn scroll_only(g: &impl GroupExt) -> bool {
    // SAFETY: a live group, only read.
    let root = unsafe { fltk::group::Group::from_widget_ptr(g.as_widget_ptr()) };
    let mut p = ScrollPlan::default();
    plan(&root, &mut p)
        && !p.scrolled.is_empty()
        && !p.others.iter().any(|o| p.scrolled.iter().any(|s| overlaps(*o, *s)))
}

/// Omarchy's app keys (see [`AppWindow::help_overlay`], [`AppWindow::quit_key`]).
#[derive(Default)]
struct Conventions {
    #[allow(clippy::type_complexity)]
    help: Option<(String, Rc<dyn Fn() -> Vec<(String, Vec<(String, String)>)>>)>,
    quit: Option<Rc<dyn Fn() -> bool>>,
}

/// `?` / F1 open the shortcut overlay, `Q` quits: keys the focused control
/// did not use (typing `q` in a field types it).
fn omarchy_key(w: &DoubleWindow, conv: &Rc<RefCell<Conventions>>) -> bool {
    let mods = app::event_state() & (Shortcut::Ctrl | Shortcut::Alt | Shortcut::Meta);
    if !mods.is_empty() || app::event() != Event::KeyDown && app::event() != Event::Shortcut {
        return false;
    }
    let text = app::event_text();
    let key = app::event_key();
    let help = conv.borrow().help.clone();
    if let Some((title, sections)) = help.filter(|_| text == "?" || key == Key::F1) {
        crate::popups::ShortcutHelp::open(w, &title, sections());
        return true;
    }
    let quit = conv.borrow().quit.clone();
    if let Some(unsaved) = quit.filter(|_| key == Key::from_char('q') && !app::event_state().contains(Shortcut::Shift)) {
        if unsaved() {
            crate::overlay::ConfirmDialog::open(w, &crate::tr!("Quit without saving?"), &crate::tr!("Quit"), true, |yes| {
                if yes {
                    crate::app::App::quit();
                }
            });
        } else {
            crate::app::App::quit();
        }
        return true;
    }
    false
}

/// Draws the window's children over a freshly painted background.
///
/// Partial redraws (only some children damaged) are clipped by FLTK to the
/// damaged area, which this window's background has just covered. Every
/// child that overlaps a damaged one is drawn again, in stacking order: the
/// content under an open menu, toast or dialog, and the layer above a
/// control that changed. Otherwise a hovered menu would hide the window
/// behind it, or a redrawn control would cover a notification.
fn draw_stack(w: &mut DoubleWindow) {
    if w.damage_type() != Damage::Child {
        w.draw_children(); // full redraw: everything is damaged
        return;
    }
    let kids: Vec<fltk::widget::Widget> = (0..w.children()).filter_map(|i| w.child(i)).filter(|c| c.visible()).collect();
    let rect = |c: &fltk::widget::Widget| (c.x(), c.y(), c.x() + c.w(), c.y() + c.h());
    let damaged: Vec<(i32, i32, i32, i32)> = kids.iter().filter(|c| c.damage()).map(rect).collect();
    let overlaps = |r: (i32, i32, i32, i32)| damaged.iter().any(|d| r.0 < d.2 && d.0 < r.2 && r.1 < d.3 && d.1 < r.3);
    for c in kids {
        let mut c = c;
        if overlaps(rect(&c)) {
            w.draw_child(&mut c);
        }
    }
}

// ================================================================== Header

/// Omarchy window header: bold title on the left, action buttons on the
/// right, hairline below (mirrored in a right-to-left language: title on
/// the right, actions on the left). Dragging it moves a floating window.
#[derive(Clone)]
pub struct Header {
    inner: Flex,
    title: Rc<RefCell<String>>,
    actions: Flex,
    draggable: Rc<Cell<bool>>,
}
fltk::widget_extends!(Header, Flex, inner);

impl Header {
    fn new(title: &str) -> Header {
        let t = theme::current();
        let h = t.spacing.control_height + 2 * t.spacing.md + 1;
        let mut inner = Flex::default().row().with_size(400, h);
        inner.set_margins(t.spacing.panel_padding - t.spacing.md, t.spacing.md, t.spacing.md, t.spacing.md + 1);
        inner.set_spacing(t.spacing.sm);
        let title_rc = Rc::new(RefCell::new(title.to_string()));
        let draggable = Rc::new(Cell::new(false));
        let mut handle = Widget::default();
        handle.set_frame(FrameType::NoBox);
        let tt = title_rc.clone();
        handle.draw(move |w| {
            // Reading start: the right in a right-to-left language.
            let align = crate::layout::Mirror::of(w).align(Align::Left);
            theme::with(|t| paint::text_elided(&tt.borrow(), w.x(), w.y(), w.w(), w.h(), t.font.title, true, t.palette.foreground, align));
        });
        let (dr, tt) = (draggable.clone(), title_rc.clone());
        handle.handle(move |w, ev| match ev {
            Event::Push if dr.get() => {
                // Native compositor move first (any Wayland compositor), Hyprland IPC otherwise.
                if !crate::native::begin_move(w) {
                    let title = w.window().map(|win| win.label()).unwrap_or_else(|| tt.borrow().clone());
                    drag_begin(w, &title, 0, (0, 0));
                }
                true
            }
            Event::Drag if dr.get() => { drag_update(); true }
            Event::Released if dr.get() => { drag_end(); true }
            _ => false,
        });
        let mut actions = Flex::default().row();
        actions.set_spacing(t.spacing.xs);
        actions.end();
        // Right to left: title on the right, actions on the left.
        crate::layout::mirror(&actions);
        inner.fixed(&actions, 0);
        inner.end();
        crate::layout::register(&inner);
        inner.draw(move |f| {
            crate::layout::relayout_if_mirrored(f);
            theme::with(|t| {
                crate::glass::fill(f.x(), f.y(), f.w(), f.h(), 0, t.palette.background);
                let line = t.palette.foreground.over(t.palette.background, 0.12);
                crate::glass::fill(f.x(), f.y() + f.h() - 1, f.w(), 1, 0, line);
            });
            f.draw_children();
        });
        // cfltk runs FLTK's own draw before this closure by default: it would
        // draw damaged children first and this background would then cover them.
        inner.super_draw(false);
        let tt = title_rc.clone();
        crate::a11y::describe(&handle, move |n| {
            n.role(crate::a11y::Role::Heading).level(1).name(&tt.borrow());
        });
        let acts = actions.clone();
        crate::a11y::describe(&actions, move |n| {
            if acts.children() > 0 {
                n.role(crate::a11y::Role::Toolbar);
            }
        });
        Header { inner, title: title_rc, actions, draggable }
    }

    pub fn set_title(&mut self, title: &str) {
        *self.title.borrow_mut() = title.into();
        self.inner.redraw();
    }

    pub fn title(&self) -> String {
        self.title.borrow().clone()
    }

    /// Adds an action button (Nerd glyph + tooltip) on the right side (the
    /// left in a right-to-left language).
    pub fn action(&mut self, glyph: &str, tooltip: &str, f: impl FnMut() + 'static) -> ActionButton {
        self.add(ActionButton::new(glyph, tooltip).on_click(f))
    }

    /// Adds a destructive action (hover in the urgent color).
    pub fn destructive_action(&mut self, glyph: &str, tooltip: &str, f: impl FnMut() + 'static) -> ActionButton {
        self.add(ActionButton::destructive(glyph, tooltip).on_click(f))
    }

    fn add(&mut self, b: ActionButton) -> ActionButton {
        let prev = fltk::group::Group::try_current();
        let size = b.w(); // before add(): Flex relayouts its children on add
        self.actions.add(&*b);
        self.actions.fixed(&*b, size);
        let n = self.actions.children();
        let total = n * size + (n - 1) * self.actions.spacing();
        self.inner.fixed(&self.actions, total);
        self.inner.layout();
        self.actions.layout();
        if let Some(g) = prev {
            g.begin();
        }
        b
    }

    fn set_draggable(&self, on: bool) {
        self.draggable.set(on);
    }

    #[allow(dead_code)]
    fn title_width(&self) -> i32 {
        theme::with(|t| measure_offscreen(&self.title.borrow(), t.font.title))
    }
}
