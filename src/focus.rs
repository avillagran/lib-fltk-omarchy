//! Keyboard-first focus model, shared by every component and every app.
//!
//! - **Tab / Shift+Tab** walk every focusable control of the window in
//!   layout order (depth-first over the widget tree) and wrap around.
//! - **Arrow keys** go only to the focused control and then to the arrow
//!   [`scope`]s around it (a `ButtonGroup`, a panel). fltk's own per-group
//!   arrow navigation is off, so arrows never jump between unrelated
//!   controls; unused arrows are offered to window shortcuts.
//! - **Menus and dialogs** take focus when they open and give it back to the
//!   control that had it when they close. While open they keep the keyboard
//!   (Tab does not leave a modal layer).
//! - Every component that reacts to the mouse can be focused and used with
//!   the keyboard; the focused control is drawn with the theme's
//!   `[controls] focus` state.
//!
//! [`App::new`](crate::App::new) installs this; nothing to call in apps.
//! [`press`] sends a key through the same path as a real key press (tests,
//! scripted tours).

use fltk::{app, enums::*, prelude::*, widget::Widget};
use std::cell::{Cell, RefCell};
use std::ffi::c_char;
use crate::panel::KeyboardPanel;

type ScopeFn = Box<dyn FnMut(Key) -> bool>;

thread_local! {
    static INSTALLED: Cell<bool> = const { Cell::new(false) };
    static SCOPES: RefCell<Vec<(Widget, Option<ScopeFn>)>> = RefCell::new(Vec::new());
    static RETURNS: RefCell<Vec<(usize, Widget)>> = const { RefCell::new(Vec::new()) };
    static KEYBOARD: Cell<bool> = const { Cell::new(false) };
    static HOSTS: RefCell<Vec<Host>> = const { RefCell::new(Vec::new()) };
}

/// A widget whose focusable regions are keyboard panels.
struct Host {
    widget: Widget,
    panels: Vec<KeyboardPanel>,
    active: usize,
}

/// Installs the keyboard router (idempotent; `App::new` calls it).
pub fn install() {
    if INSTALLED.with(|i| i.replace(true)) {
        return;
    }
    // SAFETY: `dispatch` forwards every event it does not consume to fltk's
    // default handler with the same window pointer.
    unsafe { app::event_dispatch(dispatch) };
}

fn dispatch(ev: Event, win: app::WindowPtr) -> bool {
    match ev {
        Event::KeyDown => set_keyboard_mode(true),
        Event::Push => set_keyboard_mode(false),
        _ => {}
    }
    if ev == Event::KeyDown {
        if let Some(used) = route_key(win) {
            return used;
        }
    }
    // SAFETY: `win` comes from fltk for this very event.
    unsafe { app::handle_raw(ev, win) }
}

fn ptr<W: WidgetExt>(w: &W) -> usize {
    w.as_widget_ptr() as usize
}

/// The widget that has keyboard focus (a control, not a window or group).
pub fn current() -> Option<Widget> {
    let f = app::focus()?;
    // SAFETY: fltk returned a live widget pointer.
    let w = unsafe { Widget::from_widget_ptr(f.as_widget_ptr()) };
    (w.as_window().is_none()).then_some(w)
}

/// True when `w` has keyboard focus.
pub fn is_focused<W: WidgetExt>(w: &W) -> bool {
    app::focus().is_some_and(|f| f.as_widget_ptr() == w.as_widget_ptr())
}

/// True while the user drives the app with the keyboard (last input was a
/// key, not a click). The focus ring is only drawn then, like
/// `:focus-visible` on the web: a mouse click does not leave a ring.
pub fn keyboard_mode() -> bool {
    KEYBOARD.with(|k| k.get())
}

fn set_keyboard_mode(on: bool) {
    if KEYBOARD.with(|k| k.replace(on)) != on {
        if let Some(mut f) = current() {
            f.redraw();
            if let Some(mut p) = f.parent() { p.redraw(); }
        }
    }
}

/// True when `w` has focus and the ring should be drawn (keyboard mode).
pub fn show_ring<W: WidgetExt>(w: &W) -> bool {
    keyboard_mode() && is_focused(w)
}

/// Tab / arrows handling. `None` = not a navigation key, let fltk route it.
fn route_key(win: app::WindowPtr) -> Option<bool> {
    let state = app::event_state() & (Shortcut::Ctrl | Shortcut::Shift | Shortcut::Alt | Shortcut::Meta);
    let k = app::event_key();
    // A focused keyboard panel sees every key first (any modifiers). What it
    // does not use goes on below exactly once (host handler, shortcuts).
    let panel = current().and_then(|f| active_panel(&f));
    if let Some(p) = &panel {
        if k != Key::Tab && p.handle_key(k, state) {
            return Some(true);
        }
    }
    if state.intersects(Shortcut::Ctrl | Shortcut::Alt | Shortcut::Meta) {
        return None;
    }
    let arrow = matches!(k, Key::Left | Key::Right | Key::Up | Key::Down);
    if k != Key::Tab && !arrow {
        return None;
    }
    if let Some(mut f) = current() {
        // A panel host does not get Tab: the router moves between its panels.
        if !(panel.is_some() && k == Key::Tab) && f.handle_event(Event::KeyDown) {
            return Some(true);
        }
        if arrow && run_scopes(&f, k) {
            return Some(true);
        }
    }
    if k == Key::Tab {
        // SAFETY: a Fl_Window is a Fl_Widget (single inheritance).
        let w = current().and_then(|f| f.top_window()).map(|w| unsafe { Widget::from_widget_ptr(w.as_widget_ptr()) })
            .or_else(|| (!win.is_null()).then(|| unsafe { Widget::from_widget_ptr(win as _) }));
        if let Some(w) = w {
            step(&w, !state.contains(Shortcut::Shift));
        }
        return Some(true);
    }
    // Unused arrow: offer it as a shortcut (window/AppWindow shortcuts).
    // SAFETY: `win` comes from fltk for this event.
    Some(!win.is_null() && unsafe { app::handle_raw(Event::Shortcut, win) })
}

/// Focusable candidates of `window` in traversal order.
pub fn chain<W: WidgetExt>(window: &W) -> Vec<Widget> {
    fn walk(g: &fltk::group::Group, out: &mut Vec<Widget>) {
        for i in 0..g.children() {
            let Some(c) = g.child(i) else { continue };
            if !c.visible() || !c.active() {
                continue;
            }
            if let Some(sub) = c.as_group() {
                walk(&sub, out);
            } else if c.has_visible_focus() {
                out.push(c);
            }
        }
    }
    let mut out = Vec::new();
    if let Some(g) = window.as_group() {
        walk(&g, &mut out);
    }
    out
}

/// Tab stops: each control, and each keyboard panel of a panel host.
fn stops<W: WidgetExt>(window: &W) -> Vec<(Widget, Option<usize>)> {
    let mut out = Vec::new();
    for w in chain(window) {
        match host_panels(&w) {
            Some((n, _)) if n > 0 => out.extend((0..n).map(|r| (w.clone(), Some(r)))),
            _ => out.push((w, None)),
        }
    }
    out
}

/// Focuses stop `(w, region)`; true on success.
fn enter(w: &Widget, region: Option<usize>) -> bool {
    let mut w = w.clone();
    if !is_focused(&w) && !(w.take_focus().is_ok() && is_focused(&w)) {
        return false;
    }
    if let Some(r) = region {
        set_active(&w, r);
    }
    true
}

/// Moves focus to the next (or previous) control, or keyboard panel, that
/// accepts it. Returns the newly focused widget.
pub fn step<W: WidgetExt>(window: &W, forward: bool) -> Option<Widget> {
    let stops = stops(window);
    let n = stops.len();
    if n == 0 {
        return None;
    }
    let cur = current().and_then(|f| {
        let region = host_panels(&f).map(|(_, a)| a);
        stops.iter().position(|(w, r)| ptr(w) == ptr(&f) && (r.is_none() || *r == region))
    });
    for k in 1..=n {
        let i = match cur {
            Some(c) if forward => (c + k) % n,
            Some(c) => (c + n - k % n) % n,
            None if forward => k - 1,
            None => n - k,
        };
        let (w, r) = &stops[i];
        if Some(i) == cur {
            return Some(w.clone());
        }
        if enter(w, *r) {
            return Some(w.clone());
        }
    }
    None
}

/// Focuses the first control (or keyboard panel) of `window` that accepts focus.
pub fn first<W: WidgetExt>(window: &W) -> Option<Widget> {
    stops(window).into_iter().find_map(|(w, r)| enter(&w, r).then_some(w))
}

// ------------------------------------------------------------ panel hosts

/// Makes `widget` (a canvas that draws several regions) host these keyboard
/// panels, in Tab order. Calling it again replaces the list (e.g. a dual
/// pane turned into a single list). The widget must accept `Event::Focus`.
pub fn host<W: WidgetExt>(widget: &W, panels: &[&KeyboardPanel]) {
    // SAFETY: `widget` is a live widget; the copy only tracks it.
    let w = unsafe { Widget::from_widget_ptr(widget.as_widget_ptr()) };
    let id = ptr(&w);
    let old = HOSTS.with(|h| {
        let mut h = h.borrow_mut();
        h.retain(|e| !e.widget.was_deleted());
        let i = h.iter().position(|e| ptr(&e.widget) == id);
        i.map(|i| h.remove(i))
    });
    let previous = old.as_ref().and_then(|o| o.panels.get(o.active).cloned());
    if let Some(o) = &old {
        for p in &o.panels {
            if !panels.iter().any(|n| *n == p) {
                p.set_host(None);
            }
        }
    }
    for p in panels {
        p.set_host(Some(w.clone()));
    }
    let active = previous.and_then(|p| panels.iter().position(|n| **n == p)).unwrap_or(0);
    let mut redraw = w.clone();
    HOSTS.with(|h| h.borrow_mut().push(Host { widget: w, panels: panels.iter().map(|p| (*p).clone()).collect(), active }));
    redraw.redraw();
}

/// (panel count, active index) when `w` hosts panels.
fn host_panels(w: &Widget) -> Option<(usize, usize)> {
    let id = ptr(w);
    HOSTS.with(|h| h.borrow().iter().find(|e| !e.widget.was_deleted() && ptr(&e.widget) == id).map(|e| (e.panels.len(), e.active)))
}

fn active_panel(w: &Widget) -> Option<KeyboardPanel> {
    let id = ptr(w);
    HOSTS.with(|h| h.borrow().iter().find(|e| !e.widget.was_deleted() && ptr(&e.widget) == id).and_then(|e| e.panels.get(e.active).cloned()))
}

fn set_active(w: &Widget, region: usize) {
    let id = ptr(w);
    let changed = HOSTS.with(|h| {
        let mut h = h.borrow_mut();
        let e = h.iter_mut().find(|e| ptr(&e.widget) == id)?;
        let r = region.min(e.panels.len().saturating_sub(1));
        let changed = e.active != r;
        e.active = r;
        Some(changed)
    });
    if changed == Some(true) {
        w.clone().redraw();
    }
}

/// The keyboard panel that has focus, if any (for drawing and status text).
pub fn focused_panel() -> Option<KeyboardPanel> {
    current().and_then(|f| active_panel(&f))
}

pub(crate) fn is_panel_focused(p: &KeyboardPanel) -> bool {
    focused_panel().is_some_and(|f| f == *p)
}

pub(crate) fn focus_panel(p: &KeyboardPanel) {
    let Some(w) = p.host() else { return };
    let region = HOSTS.with(|h| h.borrow().iter().find(|e| ptr(&e.widget) == ptr(&w)).and_then(|e| e.panels.iter().position(|x| x == p)));
    if let Some(r) = region {
        enter(&w, Some(r));
    }
}

/// Arrow-key handler for a group of controls: while focus is anywhere
/// inside `group`, arrows the focused control does not use go to `f(key)`
/// (return true when used). `ButtonGroup` uses it; apps use it for panels
/// (e.g. Left/Right switch between a sidebar and the content).
pub fn scope<W: WidgetExt>(group: &W, f: impl FnMut(Key) -> bool + 'static) {
    // SAFETY: `group` is a live widget.
    let g = unsafe { Widget::from_widget_ptr(group.as_widget_ptr()) };
    SCOPES.with(|s| {
        let mut s = s.borrow_mut();
        s.retain(|(w, _)| !w.was_deleted());
        s.push((g, Some(Box::new(f))));
    });
}

fn run_scopes(focus: &Widget, k: Key) -> bool {
    let mut p = focus.parent();
    while let Some(g) = p {
        let id = ptr(&g);
        // Take the handler out while it runs so it may register scopes.
        let f = SCOPES.with(|s| s.borrow_mut().iter_mut().find(|(w, _)| !w.was_deleted() && ptr(w) == id).and_then(|(_, f)| f.take()));
        if let Some(mut f) = f {
            let used = f(k);
            SCOPES.with(|s| {
                if let Some(slot) = s.borrow_mut().iter_mut().find(|(w, f)| f.is_none() && ptr(w) == id) {
                    slot.1 = Some(f);
                }
            });
            if used {
                return true;
            }
        }
        p = g.parent();
    }
    false
}

/// Called when a layer takes focus (modal layers on open, toasts when
/// tabbed into): remembers who had focus, to give it back later.
pub(crate) fn remember(layer: &Widget) {
    if let Some(f) = current() {
        if ptr(&f) != ptr(layer) {
            RETURNS.with(|r| {
                let mut r = r.borrow_mut();
                r.retain(|(l, _)| *l != ptr(layer));
                r.push((ptr(layer), f));
            });
        }
    }
}

/// Called when a layer closes (or lets go of focus, e.g. Esc on a toast):
/// if focus is in it, gives focus back to the remembered control (or the
/// first control of `window`).
pub(crate) fn give_back(layer: &Widget, window: Option<Widget>) {
    let id = ptr(layer);
    let prev = RETURNS.with(|r| {
        let mut r = r.borrow_mut();
        let i = r.iter().position(|(l, _)| *l == id)?;
        Some(r.remove(i).1)
    });
    let lost = current().map_or(true, |f| ptr(&f) == id);
    if !lost {
        return;
    }
    if let Some(mut p) = prev {
        if !p.was_deleted() && visible_in_window(&p) && p.take_focus().is_ok() && is_focused(&p) {
            p.redraw();
            return;
        }
    }
    if let Some(w) = window {
        first(&w);
    }
}

// ------------------------------------------------------------ key injection

extern "C" {
    #[link_name = "_ZN2Fl8e_keysymE"]
    static mut FL_E_KEYSYM: i32;
    #[link_name = "_ZN2Fl17e_original_keysymE"]
    static mut FL_E_ORIGINAL_KEYSYM: i32;
    #[link_name = "_ZN2Fl7e_stateE"]
    static mut FL_E_STATE: i32;
    #[link_name = "_ZN2Fl6e_textE"]
    static mut FL_E_TEXT: *mut c_char;
    #[link_name = "_ZN2Fl8e_lengthE"]
    static mut FL_E_LENGTH: i32;
    #[link_name = "_ZN2Fl3e_xE"]
    static mut FL_E_X: i32;
    #[link_name = "_ZN2Fl3e_yE"]
    static mut FL_E_Y: i32;
    #[link_name = "_ZN2Fl8e_x_rootE"]
    static mut FL_E_X_ROOT: i32;
    #[link_name = "_ZN2Fl8e_y_rootE"]
    static mut FL_E_Y_ROOT: i32;
    #[link_name = "_ZN2Fl8e_clicksE"]
    static mut FL_E_CLICKS: i32;
}

thread_local! {
    static TEXT: RefCell<[u8; 8]> = const { RefCell::new([0; 8]) };
}

/// Sends a key press through the same path as a real one: `press("Tab")`,
/// `press("Shift+Tab")`, `press("Down")`, `press("Enter")`, `press("Ctrl+Tab")`,
/// `press("a")`. Goes to the focused control's window (or the first
/// window). Returns true when something used the key. For tests and
/// scripted tours; real input never needs it.
pub fn press(spec: &str) -> bool {
    let keys = crate::keys::parse(spec).unwrap_or_else(|e| panic!("{e}"));
    let window = current().and_then(|f| f.top_window()).map(|w| w.as_widget_ptr())
        .or_else(|| app::first_window().map(|w| w.as_widget_ptr()));
    let Some(window) = window else { return false };
    let printable = keys.key.bits() >= 0x20 && keys.key.bits() < 0x7f;
    TEXT.with(|t| {
        let mut t = t.borrow_mut();
        *t = [0; 8];
        let mut len = 0;
        if printable && !keys.mods.intersects(Shortcut::Ctrl | Shortcut::Alt | Shortcut::Meta) {
            let mut c = keys.key.bits() as u8;
            if keys.mods.contains(Shortcut::Shift) {
                c = c.to_ascii_uppercase();
            }
            t[0] = c;
            len = 1;
        } else if matches!(keys.key, Key::Enter | Key::KPEnter) {
            t[0] = b'\r';
            len = 1;
        }
        // SAFETY: fltk's event globals are plain data, used on the UI thread;
        // the text buffer lives in this thread's storage for the program's life.
        unsafe {
            FL_E_KEYSYM = keys.key.bits();
            FL_E_ORIGINAL_KEYSYM = keys.key.bits();
            FL_E_STATE = keys.mods.bits();
            FL_E_TEXT = t.as_mut_ptr() as *mut c_char;
            FL_E_LENGTH = len;
        }
    });
    let used = dispatch(Event::KeyDown, window as app::WindowPtr);
    // SAFETY: as above.
    unsafe { FL_E_STATE = 0 };
    used
}

/// True when `w` and its groups are visible inside its window (the window
/// itself may be unmapped, e.g. offscreen tests). Window-wide shortcuts of a
/// component use it so hidden pages do not react.
pub fn visible_in_window<W: WidgetExt>(w: &W) -> bool {
    if !w.visible() {
        return false;
    }
    let mut p = w.parent();
    while let Some(g) = p {
        if g.as_window().is_some() {
            return true;
        }
        if !g.visible() {
            return false;
        }
        p = g.parent();
    }
    true
}

/// Sends a left click (press + release) at window coordinates `(x, y)` of
/// `window` through fltk's normal event path (tests, scripted tours).
pub fn click_at<W: WindowExt>(window: &W, x: i32, y: i32) {
    let win = window.as_widget_ptr() as app::WindowPtr;
    // SAFETY: fltk's event globals are plain data, used on the UI thread.
    unsafe {
        FL_E_X = x;
        FL_E_Y = y;
        FL_E_X_ROOT = window.x() + x;
        FL_E_Y_ROOT = window.y() + y;
        FL_E_CLICKS = 0;
        FL_E_KEYSYM = Key::from_i32(0xfee8 + 1).bits(); // FL_Button + 1 (left)
        FL_E_STATE = 0x0100_0000; // FL_BUTTON1
    }
    dispatch(Event::Push, win);
    // SAFETY: as above.
    unsafe { FL_E_STATE = 0 };
    dispatch(Event::Released, win);
}
