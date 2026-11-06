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
    static NATIVE_DISPATCH: Cell<bool> = const { Cell::new(false) };
    static CURRENT_DISPATCH: Cell<u64> = const { Cell::new(0) };
    static NEXT_DISPATCH: Cell<u64> = const { Cell::new(1) };
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
    // Catch-all for focus changes made outside any event (app code calling
    // `take_focus` from a timer): reported before the loop waits again.
    app::add_check(|_| {
        sync_panels(PanelFocusCause::Other);
    });
}

// One native event may visit a scene group and its GL child. Keep its
// identity across that routing (and restore it after nested dispatch).
pub(crate) fn dispatch_serial() -> u64 { CURRENT_DISPATCH.with(Cell::get) }
// Only native backends which borrow input serials need this distinction.
#[allow(dead_code)]
pub(crate) fn in_native_dispatch() -> bool { NATIVE_DISPATCH.with(Cell::get) }
struct DispatchScope(u64, bool);
impl Drop for DispatchScope {
    fn drop(&mut self) {CURRENT_DISPATCH.with(|s|s.set(self.0)); NATIVE_DISPATCH.with(|s|s.set(self.1));}
}

fn dispatch(ev: Event, win: app::WindowPtr) -> bool {
    dispatch_origin(ev, win, true)
}
fn synthetic_dispatch(ev: Event, win: app::WindowPtr) -> bool {
    dispatch_origin(ev, win, false)
}
fn dispatch_origin(ev: Event, win: app::WindowPtr, native: bool) -> bool {
    let serial=NEXT_DISPATCH.with(|s|{let n=s.get();s.set(n.wrapping_add(1).max(1));n});
    let _scope=DispatchScope(CURRENT_DISPATCH.with(|s|s.replace(serial)), NATIVE_DISPATCH.with(|s|s.replace(native)));
    #[cfg(feature="gpu")]
    if let Some((top,dx,dy))=crate::overlay::native_route(ev,win){
        return crate::platform::fl_abi::with_event_offset(dx,dy,||dispatch_one(ev,top));
    }
    dispatch_one(ev,win)
}

fn dispatch_one(ev: Event, win: app::WindowPtr) -> bool {
    match ev {
        Event::KeyDown => set_keyboard_mode(true),
        Event::Push => set_keyboard_mode(false),
        _ => {}
    }
    // FLTK turns an unused key into FL_SHORTCUT too (Fl::handle_).
    if matches!(ev, Event::Shortcut | Event::KeyDown | Event::KeyUp) {
        forget_stale_belowmouse();
    }
    if matches!(ev, Event::KeyDown | Event::KeyUp) {
        crate::keys::note(ev, app::event_key());
    }
    // A key-recording field (keys::record) takes the next key before
    // controls and app shortcuts.
    if ev == Event::KeyDown && crate::keys::record_key(app::event_key(), app::event_state()) {
        return true;
    }
    let used = if ev == Event::KeyDown { route_key(win) } else { None };
    // SAFETY: `win` comes from fltk for this very event.
    let used = used.unwrap_or_else(|| unsafe { app::handle_raw(ev, win) });
    // Pointer moves, clicks and keys show and hide tooltips.
    crate::tooltip::track(ev, win);
    // A click, an overlay opened by a key, app code in a callback: the panel
    // focus is reported before the next event is dispatched.
    sync_panels(PanelFocusCause::Other);
    crate::overlay::prepare_redraw();
    used
}

fn ptr<W: WidgetExt>(w: &W) -> usize {
    w.as_widget_ptr() as usize
}

/// Whether the keyboard router (and its event dispatcher) is installed.
pub(crate) fn installed() -> bool {
    INSTALLED.with(|i| i.get())
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
    if win.is_null() {
        return Some(false);
    }
    forget_stale_belowmouse();
    // SAFETY: `win` comes from fltk for this event.
    Some(unsafe { app::handle_raw(Event::Shortcut, win) })
}

/// Before any `FL_SHORTCUT` dispatch: FLTK 1.5.23 (Fl.cxx, FL_SHORTCUT)
/// sends the shortcut to `first_window()` without a null check when the
/// widget under the pointer is in another window: with no window shown
/// (offscreen snapshots, every window hidden) that is a null dereference.
/// Nothing is under a pointer then; forget the stale widget.
fn forget_stale_belowmouse() {
    if app::first_window().is_none() && app::grab().is_none() && app::belowmouse::<fltk::widget::Widget>().is_some() {
        // SAFETY: plain FLTK setter on the UI thread; null is valid.
        unsafe { crate::platform::fl_abi::Fl_set_belowmouse(std::ptr::null_mut()) };
    }
}

thread_local! {
    static CONTROL_GROUPS: RefCell<Vec<Widget>> = const { RefCell::new(Vec::new()) };
}

/// Marks a group that is one control (its children are an implementation
/// detail, e.g. `gfx::View` and its GL layer): Tab stops at the group.
pub fn control_group<W: WidgetExt>(g: &W) {
    // SAFETY: `g` is a live widget; the copy only tracks its deletion.
    let w = unsafe { Widget::from_widget_ptr(g.as_widget_ptr()) };
    CONTROL_GROUPS.with(|c| {
        let mut c = c.borrow_mut();
        c.retain(|x| !x.was_deleted());
        c.push(w);
    });
}

fn is_control_group(w: &Widget) -> bool {
    CONTROL_GROUPS.with(|c| c.borrow().iter().any(|x| !x.was_deleted() && x.as_widget_ptr() == w.as_widget_ptr()))
}

thread_local! {
    static MODALS: RefCell<Vec<Widget>> = const { RefCell::new(Vec::new()) };
}

/// While `group` is open, Tab and Shift+Tab stay inside it (popup cards,
/// dialogs made of widgets). Undo with [`pop_modal`].
pub fn push_modal<W: WidgetExt>(group: &W) {
    // SAFETY: `group` is a live widget; the copy only tracks its deletion.
    let w = unsafe { Widget::from_widget_ptr(group.as_widget_ptr()) };
    MODALS.with(|m| m.borrow_mut().push(w));
}

pub fn pop_modal<W: WidgetExt>(group: &W) {
    MODALS.with(|m| m.borrow_mut().retain(|x| !x.was_deleted() && x.as_widget_ptr() != group.as_widget_ptr()));
}

/// The innermost open modal group inside `window`, if any.
fn modal_in<W: WidgetExt>(window: &W) -> Option<Widget> {
    MODALS.with(|m| {
        let mut m = m.borrow_mut();
        m.retain(|x| !x.was_deleted());
        m.iter().rev().find(|x| x.top_window().is_some_and(|t| t.as_widget_ptr() == window.as_widget_ptr())).cloned()
    })
}

/// True when `w` is `ancestor` or inside it.
fn inside(w: &Widget, ancestor: usize) -> bool {
    if ptr(w) == ancestor {
        return true;
    }
    let mut p = w.parent();
    while let Some(g) = p {
        if ptr(&g) == ancestor {
            return true;
        }
        p = g.parent();
    }
    false
}

/// Focusable candidates of `window` in traversal order (of the open modal
/// group when there is one).
pub fn chain<W: WidgetExt>(window: &W) -> Vec<Widget> {
    if let Some(m) = modal_in(window) {
        return chain_of(&m);
    }
    chain_of(window)
}

fn chain_of<W: WidgetExt>(window: &W) -> Vec<Widget> {
    fn walk(g: &fltk::group::Group, out: &mut Vec<Widget>, followers: &[(Widget, Widget)]) {
        for i in 0..g.children() {
            let Some(c) = g.child(i) else { continue };
            if !c.visible() || !c.active() {
                continue;
            }
            // A follower group comes right after its owner, not here.
            if followers.iter().any(|(_, f)| ptr(f) == ptr(&c)) {
                continue;
            }
            if is_control_group(&c) {
                if c.has_visible_focus() {
                    out.push(c.clone());
                }
            } else if let Some(sub) = c.as_group() {
                walk(&sub, out, followers);
            } else if c.has_visible_focus() {
                out.push(c.clone());
            }
            for (_, f) in followers.iter().filter(|(o, _)| ptr(o) == ptr(&c)) {
                if let Some(fg) = f.as_group().filter(|g| g.visible()) {
                    walk(&fg, out, followers);
                }
            }
        }
    }
    let followers = FOLLOWERS.with(|f| {
        let mut f = f.borrow_mut();
        f.retain(|(o, g)| !o.was_deleted() && !g.was_deleted());
        f.clone()
    });
    let mut out = Vec::new();
    if let Some(g) = window.as_group() {
        walk(&g, &mut out, &followers);
    }
    out
}

thread_local! {
    static FOLLOWERS: RefCell<Vec<(Widget, Widget)>> = const { RefCell::new(Vec::new()) };
}

/// Tab order: the controls of `group` (an overlay elsewhere in the window,
/// e.g. widgets hosted over a canvas) come right after `owner`, and
/// Shift+Tab from them goes back to it.
pub(crate) fn follow<W: WidgetExt, G: WidgetExt>(owner: &W, group: &G) {
    // SAFETY: both are live widgets; the copies only track them.
    let (o, g) = unsafe { (Widget::from_widget_ptr(owner.as_widget_ptr()), Widget::from_widget_ptr(group.as_widget_ptr())) };
    FOLLOWERS.with(|f| {
        let mut f = f.borrow_mut();
        f.retain(|(a, b)| !a.was_deleted() && !b.was_deleted() && ptr(b) != ptr(&g));
        f.push((o, g));
    });
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
    let r = step_inner(window, forward);
    sync_panels(if forward { PanelFocusCause::Tab } else { PanelFocusCause::ShiftTab });
    r
}

fn step_inner<W: WidgetExt>(window: &W, forward: bool) -> Option<Widget> {
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
    let r = stops(window).into_iter().find_map(|(w, r)| enter(&w, r).then_some(w));
    sync_panels(PanelFocusCause::Focus);
    r
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
    sync_panels(PanelFocusCause::HostChanged);
}

/// The active panel of a panel host, focused or not (the pane that stays
/// "current" while focus is in a path field or a menu).
pub fn active_panel_of<W: WidgetExt>(widget: &W) -> Option<KeyboardPanel> {
    // SAFETY: `widget` is a live widget; the copy is only compared.
    active_panel(&unsafe { Widget::from_widget_ptr(widget.as_widget_ptr()) })
}

/// The panels of a panel host and the active index (accessibility tree).
#[cfg(feature = "a11y")]
pub(crate) fn hosted_panels(w: &Widget) -> Option<(Vec<KeyboardPanel>, usize)> {
    let id = ptr(w);
    HOSTS.with(|h| h.borrow().iter().find(|e| !e.widget.was_deleted() && ptr(&e.widget) == id).map(|e| (e.panels.clone(), e.active)))
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
    sync_panels(PanelFocusCause::Focus);
}

// ------------------------------------------------------ panel focus events

/// Why the focused keyboard panel changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PanelFocusCause {
    /// Tab, or [`step`] forward.
    Tab,
    /// Shift+Tab, or [`step`] backward.
    ShiftTab,
    /// [`KeyboardPanel::focus`] or [`first`].
    Focus,
    /// [`host`] replaced the panel list (the focused panel went away or moved).
    HostChanged,
    /// A menu, dialog or popup opened and took the keyboard.
    Overlay,
    /// A menu, dialog or toast closed and gave focus back.
    Restored,
    /// Anything else: a click, `take_focus` in app code, a toast reached
    /// with Tab, the host widget hidden or deleted.
    #[default]
    Other,
}

/// One change of the focused keyboard panel (see [`on_panel_focus`]).
#[derive(Debug, Clone)]
pub struct PanelFocus {
    /// The panel that had focus (`None`: an ordinary control or nothing).
    pub previous: Option<KeyboardPanel>,
    /// The panel that has focus now.
    pub current: Option<KeyboardPanel>,
    pub cause: PanelFocusCause,
}

type PanelFocusFn = Box<dyn FnMut(&PanelFocus)>;

thread_local! {
    static PANEL_SEEN: RefCell<Option<KeyboardPanel>> = const { RefCell::new(None) };
    static PANEL_LISTENERS: RefCell<Vec<(u64, Option<PanelFocusFn>)>> = const { RefCell::new(Vec::new()) };
    static PANEL_NEXT_ID: Cell<u64> = const { Cell::new(1) };
    static SYNCING: Cell<bool> = const { Cell::new(false) };
    static PENDING_CAUSE: Cell<Option<PanelFocusCause>> = const { Cell::new(None) };
}

/// Handle returned by [`on_panel_focus`]; pass it to [`remove_panel_focus`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PanelFocusListener(u64);

/// Runs `f` every time the focused keyboard panel changes, synchronously,
/// inside the event or call that changed it. Order and guarantees are in
/// docs/KEYBOARD.md ("Panel focus events"). Per-panel:
/// [`KeyboardPanel::on_focus`].
pub fn on_panel_focus(f: impl FnMut(&PanelFocus) + 'static) -> PanelFocusListener {
    let id = PANEL_NEXT_ID.with(|n| n.replace(n.get() + 1));
    PANEL_LISTENERS.with(|l| l.borrow_mut().push((id, Some(Box::new(f)))));
    PanelFocusListener(id)
}

pub fn remove_panel_focus(listener: PanelFocusListener) {
    PANEL_LISTENERS.with(|l| l.borrow_mut().retain(|(id, _)| *id != listener.0));
}

/// Compares the focused panel with the last one reported and notifies the
/// change. Re-entrant calls (a listener that moves focus) are folded into
/// the running loop, so changes are reported one at a time, in order.
pub(crate) fn sync_panels(cause: PanelFocusCause) {
    if SYNCING.with(|s| s.replace(true)) {
        PENDING_CAUSE.with(|p| p.set(Some(cause)));
        return;
    }
    let mut cause = cause;
    // Bounded: listeners that keep moving focus cannot loop forever.
    for _ in 0..16 {
        PENDING_CAUSE.with(|p| p.set(None));
        let now = focused_panel();
        let prev = PANEL_SEEN.with(|s| s.borrow().clone());
        if now == prev {
            match PENDING_CAUSE.with(|p| p.take()) {
                Some(c) => { cause = c; continue; }
                None => break,
            }
        }
        PANEL_SEEN.with(|s| *s.borrow_mut() = now.clone());
        let ev = PanelFocus { previous: prev, current: now, cause };
        // Both hosts repaint (focus ring, active pane); the paint comes after
        // the listeners, at the next flush.
        for p in ev.previous.iter().chain(ev.current.iter()) {
            if let Some(mut h) = p.host() {
                h.redraw();
            }
        }
        if let Some(p) = &ev.previous {
            p.notify_focus(false);
        }
        if let Some(p) = &ev.current {
            p.notify_focus(true);
        }
        let ids: Vec<u64> = PANEL_LISTENERS.with(|l| l.borrow().iter().map(|(id, _)| *id).collect());
        for id in ids {
            // Taken out while it runs: it may add or remove listeners.
            let f = PANEL_LISTENERS.with(|l| l.borrow_mut().iter_mut().find(|(i, _)| *i == id).and_then(|(_, f)| f.take()));
            if let Some(mut f) = f {
                f(&ev);
                PANEL_LISTENERS.with(|l| {
                    if let Some(slot) = l.borrow_mut().iter_mut().find(|(i, f)| *i == id && f.is_none()) {
                        slot.1 = Some(f);
                    }
                });
            }
        }
        // A listener that moved focus is reported next, with its own cause.
        cause = PENDING_CAUSE.with(|p| p.take()).unwrap_or(PanelFocusCause::Other);
    }
    SYNCING.with(|s| s.set(false));
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

/// Remembers `prev` as the control to give focus back to when `layer` closes.
pub(crate) fn remember_for(layer: &Widget, prev: Widget) {
    RETURNS.with(|r| {
        let mut r = r.borrow_mut();
        r.retain(|(l, _)| *l != ptr(layer));
        r.push((ptr(layer), prev));
    });
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
    let lost = current().map_or(true, |f| inside(&f, id));
    if !lost {
        return;
    }
    if let Some(mut p) = prev {
        if !p.was_deleted() && visible_in_window(&p) && p.take_focus().is_ok() && is_focused(&p) {
            p.redraw();
            sync_panels(PanelFocusCause::Restored);
            return;
        }
    }
    if let Some(w) = window {
        let _ = stops(&w).into_iter().find_map(|(w, r)| enter(&w, r).then_some(w));
    }
    sync_panels(PanelFocusCause::Restored);
}

// ------------------------------------------------------------ key injection

use crate::platform::fl_abi::{
    FL_E_CLICKS, FL_E_DY, FL_E_KEYSYM, FL_E_LENGTH, FL_E_ORIGINAL_KEYSYM, FL_E_STATE, FL_E_TEXT, FL_E_X, FL_E_X_ROOT, FL_E_Y, FL_E_Y_ROOT,
};

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
        } else if let Some(c) = match keys.key {
            // The control characters real keys carry (xkb / XLookupString):
            // Wayland's Fl::compose tells text from editing keys by them.
            Key::Enter | Key::KPEnter => Some(b'\r'),
            Key::BackSpace => Some(0x08),
            Key::Tab => Some(b'\t'),
            Key::Escape => Some(0x1b),
            Key::Delete => Some(0x7f),
            _ => None,
        } {
            t[0] = c;
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
    let used = synthetic_dispatch(Event::KeyDown, window as app::WindowPtr);
    // SAFETY: as above.
    unsafe { FL_E_STATE = 0 };
    // No key release follows: the next press of this key is a new press.
    crate::keys::release_all();
    used
}

/// Like [`press`], as the automatic repeat of a held key
/// ([`keys::is_repeat`](crate::keys::is_repeat) is true for it).
pub fn press_repeat(spec: &str) -> bool {
    let keys = crate::keys::parse(spec).unwrap_or_else(|e| panic!("{e}"));
    crate::keys::hold(keys.key);
    press(spec)
}

thread_local! {
    static TYPED: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

/// Sends `text` as one key event carrying that text (what FLTK delivers
/// for a committed input-method string or a compose result: "é", "日本",
/// "👍"), through the same path as [`press`]. The focused control gets it
/// via `Fl::compose`. Returns true when something used it.
pub fn type_text(text: &str) -> bool {
    let window = current().and_then(|f| f.top_window()).map(|w| w.as_widget_ptr())
        .or_else(|| app::first_window().map(|w| w.as_widget_ptr()));
    let Some(window) = window else { return false };
    TYPED.with(|t| {
        let mut t = t.borrow_mut();
        t.clear();
        t.extend_from_slice(text.as_bytes());
        t.push(0);
        // SAFETY: fltk's event globals are plain data, used on the UI thread;
        // the buffer lives in this thread's storage until the next call.
        unsafe {
            FL_E_KEYSYM = text.chars().next().map_or(0, |c| if (c as u32) < 0x7f { c as i32 } else { 0 });
            FL_E_ORIGINAL_KEYSYM = FL_E_KEYSYM;
            FL_E_STATE = 0;
            FL_E_TEXT = t.as_mut_ptr() as *mut c_char;
            FL_E_LENGTH = text.len() as i32;
        }
    });
    synthetic_dispatch(Event::KeyDown, window as app::WindowPtr)
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

// Native child windows use coordinates in their containing native window.
// Include every native ancestor in screen coordinates, but not groups:
// their widget positions already use the containing window's coordinates.
fn screen_coords<W: WindowExt>(window: &W, mut x: i32, mut y: i32) -> (i32, i32) {
    let mut current = Some(window.as_base_widget());
    while let Some(w) = current {
        if w.as_window().is_some() {
            x = x.saturating_add(w.x());
            y = y.saturating_add(w.y());
        }
        current = w.parent().map(|g| g.as_base_widget());
    }
    (x, y)
}

/// Moves the pointer to window coordinates `(x, y)` of `window` (no
/// button held) through fltk's normal event path: hover states and
/// tooltips follow it (tests, scripted tours).
pub fn hover_at<W: WindowExt>(window: &W, x: i32, y: i32) {
    let win = window.as_widget_ptr() as app::WindowPtr;
    // SAFETY: fltk's event globals are plain data, used on the UI thread.
    unsafe {
        FL_E_X = x;
        FL_E_Y = y;
        let (root_x, root_y) = screen_coords(window, x, y);
        FL_E_X_ROOT = root_x;
        FL_E_Y_ROOT = root_y;
        FL_E_STATE = 0;
    }
    synthetic_dispatch(Event::Move, win);
}

/// Sends a left click (press + release) at window coordinates `(x, y)` of
/// `window` through fltk's normal event path (tests, scripted tours).
pub fn click_at<W: WindowExt>(window: &W, x: i32, y: i32) {
    click_at_with(window, x, y, Shortcut::None);
}

/// [`click_at`] with modifier keys held (`Shortcut::Shift`, …).
pub fn click_at_with<W: WindowExt>(window: &W, x: i32, y: i32, mods: Shortcut) {
    let win = window.as_widget_ptr() as app::WindowPtr;
    // SAFETY: fltk's event globals are plain data, used on the UI thread.
    unsafe {
        FL_E_X = x;
        FL_E_Y = y;
        let (root_x, root_y) = screen_coords(window, x, y);
        FL_E_X_ROOT = root_x;
        FL_E_Y_ROOT = root_y;
        FL_E_CLICKS = 0;
        FL_E_KEYSYM = Key::from_i32(0xfee8 + 1).bits(); // FL_Button + 1 (left)
        FL_E_STATE = 0x0100_0000 | mods.bits(); // FL_BUTTON1
    }
    synthetic_dispatch(Event::Push, win);
    // SAFETY: as above.
    unsafe { FL_E_STATE = 0 };
    synthetic_dispatch(Event::Released, win);
}

/// Sends a right click (press + release) at window coordinates `(x, y)` of
/// `window` through fltk's normal event path (tests of context menus).
pub fn right_click_at<W: WindowExt>(window: &W, x: i32, y: i32) {
    click_button_at(window, x, y, 3);
}

/// Click a numbered mouse button through the normal event path: 1 left,
/// 2 middle, 3 right. Other positive button numbers test auxiliary inputs.
pub fn click_button_at<W: WindowExt>(window: &W, x: i32, y: i32, button: i32) {
    assert!((1..=32).contains(&button), "invalid mouse button");
    let win = window.as_widget_ptr() as app::WindowPtr;
    // SAFETY: fltk's event globals are plain data, used on the UI thread.
    unsafe {
        FL_E_X = x;
        FL_E_Y = y;
        let (root_x, root_y) = screen_coords(window, x, y);
        FL_E_X_ROOT = root_x;
        FL_E_Y_ROOT = root_y;
        FL_E_CLICKS = 0;
        FL_E_KEYSYM = Key::from_i32(0xfee8 + button).bits();
        FL_E_STATE = match button { 1 => 0x0100_0000, 2 => 0x0200_0000, 3 => 0x0400_0000, _ => 0 };
    }
    synthetic_dispatch(Event::Push, win);
    // SAFETY: as above.
    unsafe { FL_E_STATE = 0 };
    synthetic_dispatch(Event::Released, win);
}

/// Presses the left button at `from` (window coordinates of `window`),
/// drags to `to` in a few steps and, with `release`, lets go there; without
/// it the button stays down (finish with [`mouse_up`]). Through fltk's
/// normal event path (tests of drags: reordering, resizing).
pub fn mouse_drag<W: WindowExt>(window: &W, from: (i32, i32), to: (i32, i32), release: bool) {
    let win = window.as_widget_ptr() as app::WindowPtr;
    let at = |x: i32, y: i32, state: i32| {
        // SAFETY: fltk's event globals are plain data, used on the UI thread.
        unsafe {
            FL_E_X = x;
            FL_E_Y = y;
            let (root_x, root_y) = screen_coords(window, x, y);
            FL_E_X_ROOT = root_x;
            FL_E_Y_ROOT = root_y;
            FL_E_CLICKS = 0;
            FL_E_KEYSYM = Key::from_i32(0xfee8 + 1).bits(); // FL_Button + 1 (left)
            FL_E_STATE = state;
        }
    };
    at(from.0, from.1, 0x0100_0000);
    synthetic_dispatch(Event::Push, win);
    for k in 1..=4 {
        let x = from.0 + (to.0 - from.0) * k / 4;
        let y = from.1 + (to.1 - from.1) * k / 4;
        at(x, y, 0x0100_0000);
        synthetic_dispatch(Event::Drag, win);
    }
    if release {
        mouse_up(window, to.0, to.1);
    }
}

/// Releases the left button at `(x, y)` (after [`mouse_drag`] without release).
pub fn mouse_up<W: WindowExt>(window: &W, x: i32, y: i32) {
    let win = window.as_widget_ptr() as app::WindowPtr;
    // SAFETY: as above.
    unsafe {
        FL_E_X = x;
        FL_E_Y = y;
        let (root_x, root_y) = screen_coords(window, x, y);
        FL_E_X_ROOT = root_x;
        FL_E_Y_ROOT = root_y;
        FL_E_KEYSYM = Key::from_i32(0xfee8 + 1).bits();
        FL_E_STATE = 0;
    }
    synthetic_dispatch(Event::Released, win);
}

/// Turns the mouse wheel by `dy` notches (positive: down) at window
/// coordinates `(x, y)` of `window`, through fltk's normal event path.
/// Returns true when something used it.
pub fn wheel_at<W: WindowExt>(window: &W, x: i32, y: i32, dy: i32) -> bool {
    let win = window.as_widget_ptr() as app::WindowPtr;
    // SAFETY: fltk's event globals are plain data, used on the UI thread.
    unsafe {
        FL_E_X = x;
        FL_E_Y = y;
        let (root_x, root_y) = screen_coords(window, x, y);
        FL_E_X_ROOT = root_x;
        FL_E_Y_ROOT = root_y;
        FL_E_STATE = 0;
        FL_E_DY = dy;
    }
    let used = synthetic_dispatch(Event::MouseWheel, win);
    // SAFETY: as above.
    unsafe { FL_E_DY = 0 };
    used
}

thread_local! {
    static DROPPED: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

/// Drags `text` (e.g. `file:///home/me/a.txt`, one URI per line, as file
/// managers send files) to window coordinates `(x, y)` of `window`
/// through the events FLTK delivers on every platform: `DndEnter` (first
/// time), `DndDrag`, and with `release` `DndRelease` then, when the widget
/// under the pointer accepts it, `Paste` with the text. Without `release`
/// the drag stays there (call it again to move, [`drag_leave`] to leave).
/// Returns whether the position (`release: false`) or the drop was
/// accepted. For tests.
pub fn drop_at<W: WindowExt>(window: &W, x: i32, y: i32, text: &str, release: bool) -> bool {
    let win = window.as_widget_ptr() as app::WindowPtr;
    // SAFETY: fltk's event globals are plain data, used on the UI thread.
    unsafe {
        FL_E_X = x;
        FL_E_Y = y;
        let (root_x, root_y) = screen_coords(window, x, y);
        FL_E_X_ROOT = root_x;
        FL_E_Y_ROOT = root_y;
        FL_E_STATE = 0;
    }
    if DRAGGING.with(|d| !d.replace(true)) {
        synthetic_dispatch(Event::DndEnter, win);
    }
    let over = synthetic_dispatch(Event::DndDrag, win);
    if !release {
        return over;
    }
    DRAGGING.with(|d| d.set(false));
    if !synthetic_dispatch(Event::DndRelease, win) {
        return false;
    }
    let Some(mut target) = app::belowmouse::<Widget>() else {
        return false;
    };
    DROPPED.with(|t| {
        let mut t = t.borrow_mut();
        t.clear();
        t.extend_from_slice(text.as_bytes());
        t.push(0);
        // SAFETY: as above; the buffer lives in this thread's storage until
        // the next call.
        unsafe {
            FL_E_TEXT = t.as_mut_ptr() as *mut c_char;
            FL_E_LENGTH = text.len() as i32;
        }
    });
    target.handle_event(Event::Paste)
}

thread_local! {
    static DRAGGING: Cell<bool> = const { Cell::new(false) };
}

/// The drag of [`drop_at`] (`release: false`) leaves the window.
pub fn drag_leave<W: WindowExt>(window: &W) {
    DRAGGING.with(|d| d.set(false));
    synthetic_dispatch(Event::DndLeave, window.as_widget_ptr() as app::WindowPtr);
}


#[cfg(test)]
mod presentation_dispatch_tests {
    use super::*;
    #[test]
    fn nested_synthetic_dispatch_never_borrows_native_permission() {
        assert!(!in_native_dispatch());
        let native = DispatchScope(CURRENT_DISPATCH.with(|s| s.replace(7)), NATIVE_DISPATCH.with(|s| s.replace(true)));
        assert!(in_native_dispatch());
        {
            let _synthetic = DispatchScope(CURRENT_DISPATCH.with(|s| s.replace(8)), NATIVE_DISPATCH.with(|s| s.replace(false)));
            assert!(!in_native_dispatch());
            assert_eq!(dispatch_serial(), 8);
        }
        assert!(in_native_dispatch());
        assert_eq!(dispatch_serial(), 7);
        drop(native);
        assert!(!in_native_dispatch());
        assert_eq!(dispatch_serial(), 0);
    }
}
