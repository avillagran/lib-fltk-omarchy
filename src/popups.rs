//! Anchored popups in the omarchy-shell popup style (`[popups]` surface,
//! 28 px rows, hover fill): `PopupMenu` (header Preferences/Actions menus
//! and context menus), `SearchableDropdown`, `ShortcutHelp` (F1 overlay)
//! and `OperationToast` (progress with pause / cancel).

use crate::overlay::{bg, card, close_layer, open_layer};
use crate::paint;
use crate::state::State;
use crate::theme;
use crate::widgets::glyph;
use fltk::{app, enums::*, prelude::*, widget::Widget};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

fn s(t: &theme::Theme, px: f64) -> i32 {
    (px * t.font.base as f64 / 12.0).round() as i32
}

// =============================================================== PopupMenu

/// One entry of a [`PopupMenu`].
#[derive(Debug, Clone, PartialEq)]
pub struct PopupItem {
    pub icon: String,
    pub label: String,
    /// Shortcut hint shown dimmed on the right ("Ctrl+C").
    pub shortcut: String,
    pub enabled: bool,
    /// Shows "✓" on the right (radio/check entries).
    pub checked: bool,
    /// Draws a separator after this entry.
    pub divider: bool,
}

impl PopupItem {
    pub fn new(icon: &str, label: &str) -> PopupItem {
        PopupItem { icon: icon.into(), label: label.into(), shortcut: String::new(), enabled: true, checked: false, divider: false }
    }
    pub fn shortcut(mut self, s: &str) -> Self { self.shortcut = s.into(); self }
    pub fn checked(mut self, on: bool) -> Self { self.checked = on; self }
    pub fn disabled(mut self) -> Self { self.enabled = false; self }
    pub fn divider(mut self) -> Self { self.divider = true; self }
}

/// Menu card anchored to a widget (below it; right-aligned when the anchor
/// is in the right half, like omarchy-shell's PopupCard) or opened at a
/// point (context menu). Keys: Up/Down (j/k), Enter, Esc. Click outside closes.
pub struct PopupMenu;

impl PopupMenu {
    /// Opens under `anchor`. `on_pick(index)` receives the index into `items`.
    pub fn open<A: WidgetExt>(anchor: &A, items: Vec<PopupItem>, on_pick: impl FnOnce(usize) + 'static) {
        let Some(win) = anchor.top_window() else { return };
        let (w, h) = Self::size(&items);
        let right_half = anchor.x() + anchor.w() / 2 > win.w() / 2;
        let x = if right_half { anchor.x() + anchor.w() - w } else { anchor.x() };
        let y = anchor.y() + anchor.h() + 2;
        Self::open_rect(anchor, items, (x, y, w, h), on_pick);
    }

    /// Opens at window coordinates (e.g. the mouse position for a context menu).
    pub fn open_at<A: WidgetExt>(anchor: &A, x: i32, y: i32, items: Vec<PopupItem>, on_pick: impl FnOnce(usize) + 'static) {
        let (w, h) = Self::size(&items);
        Self::open_rect(anchor, items, (x, y, w, h), on_pick);
    }

    fn size(items: &[PopupItem]) -> (i32, i32) {
        theme::with(|t| {
            let row = t.spacing.popup_row_height;
            let pad = t.spacing.control_padding_x;
            let mut w = s(t, 200.0);
            for it in items {
                let lw = crate::widgets::measure_offscreen(&it.label, t.font.body);
                let sw = if it.shortcut.is_empty() { 0 } else { crate::widgets::measure_offscreen(&it.shortcut, t.font.caption) + t.spacing.xl };
                w = w.max(2 * pad + s(t, 22.0) + t.spacing.lg + lw + sw + s(t, 18.0) + 4);
            }
            let dividers = items.iter().filter(|i| i.divider).count() as i32;
            let h = items.len() as i32 * row + dividers * (t.spacing.md + 1) + 2 * (2 + t.spacing.xxs);
            (w, h)
        })
    }

    fn open_rect<A: WidgetExt>(anchor: &A, items: Vec<PopupItem>, rect: (i32, i32, i32, i32), on_pick: impl FnOnce(usize) + 'static) {
        let Some(win) = anchor.top_window() else { return };
        let (mut x, mut y, w, h) = rect;
        x = x.clamp(2, (win.w() - w - 2).max(2));
        if y + h > win.h() - 2 {
            y = (y - h - anchor.h() - 4).max(2);
        }
        let rect = (x, y, w, h);
        let Some(mut layer) = open_layer(anchor, None, true) else { return };
        let items = Rc::new(items);
        let current = Rc::new(Cell::new(items.iter().position(|i| i.enabled).map(|i| i as i32).unwrap_or(-1)));
        let rows = {
            let items = items.clone();
            move || -> Vec<(i32, i32)> {
                theme::with(|t| {
                    let row = t.spacing.popup_row_height;
                    let mut yy = rect.1 + 2 + t.spacing.xxs;
                    items.iter().map(|it| {
                        let r = (yy, row);
                        yy += row + if it.divider { t.spacing.md + 1 } else { 0 };
                        r
                    }).collect()
                })
            }
        };
        let rows = Rc::new(rows);
        let (it, cur, rw) = (items.clone(), current.clone(), rows.clone());
        layer.draw(move |l| {
            if let Some(win) = l.top_window() {
                if (l.w(), l.h()) != (win.w(), win.h()) { l.resize(0, 0, win.w(), win.h()); }
            }
            theme::with(|t| {
                let (x, y, w, h) = rect;
                card(t, x, y, w, h, &t.popups, t.controls.normal.border_width.max(1));
                let fill = t.popups.fill(bg(t));
                let fg = t.popups.text;
                let inset = 2;
                let pad = t.spacing.control_padding_x;
                for (i, (item, (ry, rh))) in it.iter().zip(rw().into_iter()).enumerate() {
                    if i as i32 == cur.get() && item.enabled {
                        paint::fill(x + inset, ry, w - 2 * inset, rh, t.radius, t.fill(&t.controls.hover, fill));
                    }
                    let color = if item.enabled { fg } else { fg.darker(2.0) };
                    let cell = s(t, 22.0);
                    if !item.icon.is_empty() {
                        paint::glyph(&item.icon, x + pad, ry, cell, rh, t.font.icon, color);
                    }
                    let tx = x + pad + cell + t.spacing.lg;
                    let mut right = x + w - pad;
                    if item.checked {
                        paint::glyph(glyph::CHECKMARK, right - s(t, 14.0), ry, s(t, 14.0), rh, t.font.body, t.palette.accent);
                        right -= s(t, 18.0);
                    }
                    if !item.shortcut.is_empty() {
                        let sw = paint::measure(&item.shortcut, t.font.caption, false);
                        paint::text(&item.shortcut, right - sw, ry, sw, rh, t.font.caption, false, fg.darker(1.4), Align::Right);
                        right -= sw + t.spacing.xl;
                    }
                    paint::text_elided(&item.label, tx, ry, right - tx, rh, t.font.body, false, color, Align::Left);
                    if item.divider {
                        paint::hairline(x + inset + pad, ry + rh + t.spacing.md / 2, w - 2 * (inset + pad), fg, fill, 0.12);
                    }
                }
            });
        });
        let on_pick = Rc::new(RefCell::new(Some(Box::new(on_pick) as Box<dyn FnOnce(usize)>)));
        let (it, cur, rw) = (items.clone(), current.clone(), rows.clone());
        layer.handle(move |l, ev| {
            let (x, y, w, h) = rect;
            let at = |ey: i32| -> Option<usize> {
                let (ex, _) = app::event_coords();
                if ex < x || ex >= x + w { return None; }
                rw().iter().position(|(ry, rh)| ey >= *ry && ey < ry + rh)
            };
            let pick = |l: &Widget, i: usize| {
                if !it[i].enabled { return; }
                close_layer(l);
                if let Some(f) = on_pick.borrow_mut().take() { f(i); }
            };
            let step = |d: i32| {
                let n = it.len() as i32;
                let mut c = cur.get();
                for _ in 0..n {
                    c = (c + d).rem_euclid(n.max(1));
                    if it[c as usize].enabled { break; }
                }
                cur.set(c);
            };
            match ev {
                Event::Move => {
                    if let Some(i) = at(app::event_y()) { if cur.get() != i as i32 { cur.set(i as i32); l.redraw(); } }
                    true
                }
                Event::Push => {
                    let (ex, ey) = app::event_coords();
                    if ex >= x && ex < x + w && ey >= y && ey < y + h {
                        if let Some(i) = at(ey) { pick(l, i); }
                    } else {
                        close_layer(l);
                    }
                    true
                }
                Event::KeyDown => {
                    let k = app::event_key();
                    if k == Key::Escape { close_layer(l); }
                    else if k == Key::Down || k == Key::from_char('j') { step(1); l.redraw(); }
                    else if k == Key::Up || k == Key::from_char('k') { step(-1); l.redraw(); }
                    else if matches!(k, Key::Enter | Key::KPEnter) && cur.get() >= 0 { pick(l, cur.get() as usize); }
                    true
                }
                Event::Focus | Event::Enter | Event::Shortcut => true,
                _ => false,
            }
        });
    }
}

// ====================================================== SearchableDropdown

/// Omarchy `SearchableDropdown`: Dropdown trigger (260 px) whose popup has
/// a search line on top (type to filter, label or description), "No
/// matches" when empty, rows with an optional description line.
#[derive(Clone)]
pub struct SearchableDropdown {
    inner: Widget,
    options: Rc<Vec<(String, String)>>,
    value: State<Option<usize>>,
}
fltk::widget_extends!(SearchableDropdown, Widget, inner);

impl SearchableDropdown {
    /// `options`: (label, description). `value`: selected index or None.
    pub fn new(placeholder: &str, options: Vec<(String, String)>, value: &State<Option<usize>>) -> SearchableDropdown {
        let mut inner = Widget::default();
        inner.set_frame(FrameType::NoBox);
        inner.set_visible_focus();
        let options = Rc::new(options);
        let hover = Rc::new(Cell::new(false));
        let (opts, v, hv, ph) = (options.clone(), value.clone(), hover.clone(), placeholder.to_string());
        inner.draw(move |w| {
            theme::with(|t| {
                let focused = crate::focus::show_ring(w);
                let st = if focused { &t.controls.focus } else if hv.get() { &t.controls.hover } else { &t.controls.normal };
                let under = bg(t);
                paint::surface(w.x(), w.y(), w.w(), w.h(), t.radius, Some(t.fill(st, under)), Some((t.border(st, under), st.border_width)));
                let fg = t.palette.foreground;
                let pad = t.spacing.control_padding_x + 1;
                let chev = paint::measure(glyph::CHEVRON_DOWN, t.font.body, false);
                let (text, color) = match v.get().and_then(|i| opts.get(i)) {
                    Some((l, _)) => (l.clone(), fg),
                    None => (ph.clone(), fg.darker(1.5)),
                };
                paint::text_elided(&text, w.x() + pad, w.y(), w.w() - 2 * pad - chev - t.spacing.md, w.h(), t.font.body, false, color, Align::Left);
                paint::glyph(glyph::CHEVRON_DOWN, w.x() + w.w() - 9 - chev, w.y(), chev, w.h(), t.font.body, fg.darker(1.2));
            });
        });
        let (opts, v, hv) = (options.clone(), value.clone(), hover.clone());
        inner.handle(move |w, ev| match ev {
            Event::Enter => { hv.set(true); w.redraw(); true }
            Event::Leave => { hv.set(false); w.redraw(); true }
            Event::Focus | Event::Unfocus => { w.redraw(); true }
            Event::Push => { let _ = w.take_focus(); open_search(w, &opts, &v); true }
            Event::KeyDown if matches!(app::event_key(), Key::Enter | Key::KPEnter | Key::Down) => { open_search(w, &opts, &v); true }
            _ => false,
        });
        let (w, h) = theme::with(|t| (t.spacing.searchable_dropdown_width, t.spacing.control_height));
        inner.set_size(w, h);
        let mut w2 = inner.clone();
        crate::state::bind_widget(&inner, value, false, move |_| w2.redraw());
        SearchableDropdown { inner, options, value: value.clone() }
    }

    pub fn selected(&self) -> Option<usize> {
        self.value.get()
    }

    pub fn open(&mut self) {
        let (o, v) = (self.options.clone(), self.value.clone());
        open_search(&mut self.inner, &o, &v);
    }

    /// Filter used by the popup (case-insensitive substring on label or description).
    pub fn filter(options: &[(String, String)], q: &str) -> Vec<usize> {
        let q = q.trim().to_lowercase();
        options.iter().enumerate()
            .filter(|(_, (l, d))| q.is_empty() || l.to_lowercase().contains(&q) || d.to_lowercase().contains(&q))
            .map(|(i, _)| i).collect()
    }
}

fn open_search(trigger: &mut Widget, options: &Rc<Vec<(String, String)>>, value: &State<Option<usize>>) {
    let Some(mut layer) = open_layer(trigger, None, true) else { return };
    let query = Rc::new(RefCell::new(String::new()));
    let visible = Rc::new(RefCell::new(SearchableDropdown::filter(options, "")));
    let cur = Rc::new(Cell::new(0usize));
    let top = Rc::new(Cell::new(0usize));
    let (tx, ty, tw) = (trigger.x(), trigger.y() + trigger.h() + 2, trigger.w());
    let geo = move || theme::with(|t| {
        let row = t.spacing.popup_row_height.max(t.font.body + t.font.caption + 12);
        let header = t.spacing.popup_row_height + t.spacing.xl;
        let rows = 6usize;
        let h = (header + 1 + rows as i32 * (row + t.spacing.sm) + 4).max(220);
        (row, header, rows, h)
    });
    let (opts, q, vis, cu, tp) = (options.clone(), query.clone(), visible.clone(), cur.clone(), top.clone());
    layer.draw(move |l| {
        if let Some(win) = l.top_window() { if (l.w(), l.h()) != (win.w(), win.h()) { l.resize(0, 0, win.w(), win.h()); } }
        let (row, header, rows, h) = geo();
        theme::with(|t| {
            card(t, tx, ty, tw, h, &t.popups, t.controls.normal.border_width.max(1));
            let fill = t.popups.fill(bg(t));
            let fg = t.popups.text;
            let m = t.spacing.md;
            let fh = header - 2 * m;
            let st = &t.controls.focus;
            paint::surface(tx + m, ty + m, tw - 2 * m, fh, t.radius, Some(t.fill(st, fill)), Some((t.border(st, fill), st.border_width)));
            let qs = q.borrow();
            let (text, color) = if qs.is_empty() { ("Search...".to_string(), fg.darker(1.6)) } else { (format!("{qs}▏"), fg) };
            paint::text_elided(&text, tx + m + t.spacing.control_padding_x, ty + m, tw - 2 * m - 2 * t.spacing.control_padding_x, fh, t.font.body, false, color, Align::Left);
            paint::hairline(tx + 2, ty + header, tw - 4, fg, fill, 0.10);
            let vis = vis.borrow();
            let ly = ty + header + 1 + 2;
            if vis.is_empty() {
                paint::text("No matches", tx, ly, tw, h - header - 4, t.font.body, false, fg.darker(1.6), Align::Center);
                return;
            }
            for slot in 0..rows {
                let vi = tp.get() + slot;
                let Some(&idx) = vis.get(vi) else { break };
                let (label, desc) = &opts[idx];
                let ry = ly + slot as i32 * (row + t.spacing.sm);
                if vi == cu.get() {
                    paint::fill(tx + 2, ry, tw - 4, row, t.radius, t.fill(&t.controls.hover, fill));
                }
                let pad = t.spacing.control_padding_x;
                if desc.is_empty() {
                    paint::text_elided(label, tx + pad, ry, tw - 2 * pad, row, t.font.body, false, fg, Align::Left);
                } else {
                    let lh = t.font.body + 4;
                    let top_y = ry + (row - lh - t.font.caption - 4 - t.spacing.xxs) / 2;
                    paint::text_elided(label, tx + pad, top_y, tw - 2 * pad, lh, t.font.body, false, fg, Align::Left);
                    paint::text_elided(desc, tx + pad, top_y + lh + t.spacing.xxs, tw - 2 * pad, t.font.caption + 4, t.font.caption, false, fg.darker(1.5), Align::Left);
                }
            }
        });
    });
    let (opts, q, vis, cu, tp, v) = (options.clone(), query.clone(), visible.clone(), cur.clone(), top.clone(), value.clone());
    let mut trig = trigger.clone();
    layer.handle(move |l, ev| {
        let (row, header, rows, h) = geo();
        let refilter = |l: &mut Widget| { *vis.borrow_mut() = SearchableDropdown::filter(&opts, &q.borrow()); cu.set(0); tp.set(0); l.redraw(); };
        let mut finish = |l: &Widget, pick: Option<usize>| {
            if let Some(i) = pick { v.set(Some(i)); }
            close_layer(l);
            let _ = trig.take_focus();
            trig.redraw();
        };
        let ensure = |c: usize| { if c < tp.get() { tp.set(c) } else if c >= tp.get() + rows { tp.set(c + 1 - rows) } };
        match ev {
            Event::KeyDown => {
                let k = app::event_key();
                let n = vis.borrow().len();
                match k {
                    Key::Escape => finish(l, None),
                    Key::Down => { if n > 0 { cu.set((cu.get() + 1).min(n - 1)); ensure(cu.get()); l.redraw(); } }
                    Key::Up => { cu.set(cu.get().saturating_sub(1)); ensure(cu.get()); l.redraw(); }
                    Key::Enter | Key::KPEnter => { let p = vis.borrow().get(cu.get()).copied(); finish(l, p); }
                    Key::BackSpace => { q.borrow_mut().pop(); refilter(l); }
                    _ => {
                        let t = app::event_text();
                        if !app::is_event_ctrl() && !t.is_empty() && t.chars().all(|c| !c.is_control()) { q.borrow_mut().push_str(&t); refilter(l); }
                    }
                }
                true
            }
            Event::Move => {
                let ly = ty + header + 1 + 2;
                let (ex, ey) = app::event_coords();
                if ex >= tx && ex < tx + tw && ey >= ly {
                    let vi = tp.get() + ((ey - ly) / (row + 4)) as usize;
                    if vi < vis.borrow().len() && vi != cu.get() { cu.set(vi); l.redraw(); }
                }
                true
            }
            Event::Push => {
                let (ex, ey) = app::event_coords();
                if ex < tx || ex >= tx + tw || ey < ty || ey >= ty + h { finish(l, None); }
                else if ey > ty + header { let p = vis.borrow().get(cu.get()).copied(); finish(l, p); }
                true
            }
            Event::MouseWheel => {
                let d = match app::event_dy() { app::MouseWheel::Down => 1i32, app::MouseWheel::Up => -1, _ => 0 };
                let max_top = vis.borrow().len().saturating_sub(rows);
                tp.set((tp.get() as i32 + d).clamp(0, max_top as i32) as usize);
                l.redraw();
                true
            }
            Event::Focus | Event::Enter | Event::Shortcut => true,
            _ => false,
        }
    });
}

// ============================================================ ShortcutHelp

/// F1-style help card: sections of (keys, description) rows over a scrim.
/// Keys are drawn as chips. Esc, F1 or a click closes; the wheel scrolls.
pub struct ShortcutHelp;

impl ShortcutHelp {
    /// `sections`: (title, [(keys, description)]).
    pub fn open<A: WidgetExt>(anchor: &A, title: &str, sections: Vec<(String, Vec<(String, String)>)>) {
        let Some(mut layer) = open_layer(anchor, None, true) else { return };
        let scroll = Rc::new(Cell::new(0i32));
        let title = title.to_string();
        let sc = scroll.clone();
        layer.draw(move |l| {
            if let Some(win) = l.top_window() { if (l.w(), l.h()) != (win.w(), win.h()) { l.resize(0, 0, win.w(), win.h()); } }
            theme::with(|t| {
                crate::overlay::draw_scrim(l, t.menu.background, 0.5);
                let cw = s(t, 520.0).min(l.w() - 2 * t.gaps_out.max(8));
                let ch = (l.h() as f64 * 0.8) as i32;
                let (cx, cy) = ((l.w() - cw) / 2, (l.h() - ch) / 2);
                let fill = t.menu.fill(bg(t));
                paint::surface(cx, cy, cw, ch, t.radius, Some(fill), Some((t.menu.border.over(fill, t.menu.border_alpha), 2)));
                let pad = t.spacing.panel_padding;
                let fg = t.menu.text;
                paint::text_elided(&title, cx + pad, cy + pad, cw - 2 * pad, t.font.heading + 8, t.font.heading, true, fg, Align::Left);
                let top = cy + pad + t.font.heading + 8 + t.spacing.lg;
                let bottom = cy + ch - pad;
                fltk::draw::push_clip(cx + pad, top, cw - 2 * pad, bottom - top);
                let mut y = top - sc.get();
                let row = t.font.body + t.spacing.xl;
                let key_col = s(t, 170.0);
                for (sec, rows) in &sections {
                    paint::text(&sec.to_uppercase(), cx + pad, y, cw - 2 * pad, t.font.caption + 6, t.font.caption, true, fg.darker(1.4), Align::Left);
                    y += t.font.caption + 6 + t.spacing.sm;
                    for (keys, desc) in rows {
                        let mut kx = cx + pad;
                        for (i, part) in keys.split('+').enumerate() {
                            if i > 0 {
                                paint::text("+", kx, y, s(t, 10.0), row, t.font.caption, false, fg.darker(1.4), Align::Center);
                                kx += s(t, 10.0);
                            }
                            let kw = paint::measure(part, t.font.caption, true) + t.spacing.lg;
                            let kh = t.font.caption + t.spacing.md + 2;
                            paint::surface(kx, y + (row - kh) / 2, kw, kh, t.radius.min(4), Some(t.fill(&t.controls.normal, fill)),
                                           Some((t.border(&t.controls.normal, fill), 1)));
                            paint::text(part, kx, y + (row - kh) / 2, kw, kh, t.font.caption, true, fg, Align::Center);
                            kx += kw;
                        }
                        paint::text_elided(desc, cx + pad + key_col, y, cw - 2 * pad - key_col, row, t.font.body, false, fg, Align::Left);
                        y += row;
                    }
                    y += t.spacing.lg;
                }
                fltk::draw::pop_clip();
            });
        });
        layer.handle(move |l, ev| match ev {
            Event::KeyDown => {
                if matches!(app::event_key(), Key::Escape | Key::F1) { close_layer(l); }
                else if app::event_key() == Key::Down || app::event_key() == Key::PageDown { scroll.set(scroll.get() + 60); l.redraw(); }
                else if app::event_key() == Key::Up || app::event_key() == Key::PageUp { scroll.set((scroll.get() - 60).max(0)); l.redraw(); }
                true
            }
            Event::MouseWheel => {
                let d = match app::event_dy() { app::MouseWheel::Down => 40, app::MouseWheel::Up => -40, _ => 0 };
                scroll.set((scroll.get() + d).max(0));
                l.redraw();
                true
            }
            Event::Push => { close_layer(l); true }
            Event::Focus | Event::Enter | Event::Shortcut => true,
            _ => false,
        });
    }
}

// ========================================================== OperationToast

/// Long operation card at the window's bottom-right (copy/move/delete):
/// title, detail line, accent progress bar and Pause/Resume + Cancel
/// buttons. Not modal; update it from the UI thread (e.g. task progress).
/// Keyboard: Tab reaches it (or call [`OperationToast::focus`] from a
/// shortcut); Left/Right pick Pause or Cancel, Enter/Space press, Esc gives
/// focus back without cancelling.
#[derive(Clone)]
pub struct OperationToast {
    layer: Widget,
    data: Rc<RefCell<ToastData>>,
}

struct ToastData {
    title: String,
    detail: String,
    progress: f64,
    paused: bool,
    hover: i32,
    /// Keyboard cursor: 0 = Pause/Resume, 1 = Cancel.
    kb: i32,
    on_pause: Option<Box<dyn FnMut(bool)>>,
    on_cancel: Option<Box<dyn FnMut()>>,
}

impl OperationToast {
    pub fn show<A: WidgetExt>(anchor: &A, title: &str) -> Option<OperationToast> {
        let win = anchor.top_window()?;
        let (w, h, m) = theme::with(|t| {
            let w = s(t, 380.0).min(win.w() - 2 * t.gaps_out.max(8));
            (w, t.spacing.popup_padding * 2 + t.font.title + 6 + t.font.body_small + 6 + t.spacing.lg + 4 + t.spacing.lg + t.spacing.control_height, t.gaps_out.max(8))
        });
        let rect = (win.w() - w - m, win.h() - h - m, w, h);
        let mut layer = open_layer(anchor, Some(rect), false)?;
        let data = Rc::new(RefCell::new(ToastData { title: title.into(), detail: String::new(), progress: 0.0, paused: false, hover: -1, kb: 0, on_pause: None, on_cancel: None }));
        let buttons = move |l: &Widget| -> [(i32, i32, i32, i32); 2] {
            theme::with(|t| {
                let bh = t.spacing.control_height;
                let bw = s(t, 96.0);
                let y = l.y() + l.h() - t.spacing.popup_padding - bh;
                let x1 = l.x() + l.w() - t.spacing.popup_padding - bw;
                [(x1 - t.spacing.md - bw, y, bw, bh), (x1, y, bw, bh)]
            })
        };
        let d = data.clone();
        layer.draw(move |l| {
            let d = d.borrow();
            theme::with(|t| {
                card(t, l.x(), l.y(), l.w(), l.h(), &t.notifications, 2);
                let fill = t.notifications.fill(bg(t));
                let fg = t.notifications.text;
                let p = t.spacing.popup_padding;
                let (x, mut y, w) = (l.x() + p, l.y() + p, l.w() - 2 * p);
                paint::text_elided(&d.title, x, y, w, t.font.title + 6, t.font.title, true, fg, Align::Left);
                y += t.font.title + 6;
                let pct = format!("{}%", (d.progress.clamp(0.0, 1.0) * 100.0).round());
                let detail = if d.paused { format!("Paused · {}", d.detail) } else { d.detail.clone() };
                paint::text_elided(&detail, x, y, w - s(t, 48.0), t.font.body_small + 6, t.font.body_small, false, fg.darker(1.4), Align::Left);
                paint::text(&pct, x, y, w, t.font.body_small + 6, t.font.body_small, true, fg, Align::Right);
                y += t.font.body_small + 6 + t.spacing.lg;
                paint::fill(x, y, w, 4, 2, t.fill(&t.controls.selected, fill));
                let bar = if d.paused { fg.darker(1.6) } else { t.notifications.border };
                paint::fill(x, y, (w as f64 * d.progress.clamp(0.0, 1.0)).round() as i32, 4, 2, bar);
                let focused = crate::focus::show_ring(l);
                for (i, (bx, by, bw, bh)) in buttons(l).iter().copied().enumerate() {
                    let hot = d.hover == i as i32 || (focused && d.kb == i as i32);
                    let (label, color) = if i == 0 {
                        (if d.paused { "Resume" } else { "Pause" }, fg)
                    } else {
                        ("Cancel", t.palette.urgent)
                    };
                    let st = if focused && d.kb == i as i32 { &t.controls.focus } else if hot { &t.controls.hover } else { &t.controls.normal };
                    paint::surface(bx, by, bw, bh, t.radius, Some(t.fill(st, fill)), Some((t.border(st, fill), st.border_width.max(1))));
                    paint::text(label, bx, by, bw, bh, t.font.body, false, color, Align::Center);
                }
            });
        });
        let d = data.clone();
        layer.handle(move |l, ev| {
            let hit = |b: (i32, i32, i32, i32)| { let (x, y) = app::event_coords(); x >= b.0 && x < b.0 + b.2 && y >= b.1 && y < b.1 + b.3 };
            let bs = buttons(l);
            match ev {
                Event::Enter => true,
                Event::Move => {
                    let h = bs.iter().position(|b| hit(*b)).map(|i| i as i32).unwrap_or(-1);
                    if d.borrow().hover != h { d.borrow_mut().hover = h; l.redraw(); }
                    true
                }
                Event::Leave => { d.borrow_mut().hover = -1; l.redraw(); true }
                Event::Push => {
                    if hit(bs[0]) { press_toast(&d, l, 0) } else if hit(bs[1]) { press_toast(&d, l, 1) }
                    true
                }
                Event::Focus => { crate::focus::remember(l); l.redraw(); true }
                Event::Unfocus => { l.redraw(); true }
                Event::KeyDown => {
                    let k = app::event_key();
                    match k {
                        Key::Left | Key::Right => { let kb = d.borrow().kb; d.borrow_mut().kb = 1 - kb; l.redraw(); true }
                        Key::Enter | Key::KPEnter => { let kb = d.borrow().kb; press_toast(&d, l, kb); true }
                        Key::Escape => {
                            // SAFETY: a Fl_Window is a Fl_Widget.
                            let win = l.top_window().map(|w| unsafe { Widget::from_widget_ptr(w.as_widget_ptr()) });
                            crate::focus::give_back(l, win);
                            true
                        }
                        _ if k == Key::from_char(' ') => { let kb = d.borrow().kb; press_toast(&d, l, kb); true }
                        _ => false,
                    }
                }
                _ => false,
            }
        });
        layer.set_visible_focus();
        Some(OperationToast { layer, data })
    }

    /// Moves keyboard focus to the toast (bind it to a shortcut so a long
    /// operation can be paused or cancelled without the mouse).
    pub fn focus(&mut self) {
        let _ = self.layer.take_focus();
    }

    pub fn has_focus(&self) -> bool {
        crate::focus::is_focused(&self.layer)
    }

    pub fn on_pause(self, f: impl FnMut(bool) + 'static) -> Self { self.data.borrow_mut().on_pause = Some(Box::new(f)); self }
    pub fn on_cancel(self, f: impl FnMut() + 'static) -> Self { self.data.borrow_mut().on_cancel = Some(Box::new(f)); self }

    pub fn set_progress(&mut self, p: f64) { self.data.borrow_mut().progress = p; self.layer.redraw(); }
    pub fn set_detail(&mut self, s: &str) { self.data.borrow_mut().detail = s.into(); self.layer.redraw(); }
    pub fn set_title(&mut self, s: &str) { self.data.borrow_mut().title = s.into(); self.layer.redraw(); }
    pub fn is_paused(&self) -> bool { self.data.borrow().paused }

    /// Same as pressing Pause/Resume (tests, keyboard shortcuts).
    pub fn toggle_pause(&mut self) {
        press_toast(&self.data, &self.layer, 0);
    }

    /// Same as pressing Cancel.
    pub fn cancel(&mut self) {
        press_toast(&self.data, &self.layer, 1);
    }

    pub fn close(&self) {
        close_layer(&self.layer);
    }
}

/// Runs the toast button `i` (0 = Pause/Resume, 1 = Cancel).
fn press_toast(d: &Rc<RefCell<ToastData>>, l: &Widget, i: i32) {
    if i == 0 {
        let paused = { let mut dd = d.borrow_mut(); dd.paused = !dd.paused; dd.paused };
        let cb = d.borrow_mut().on_pause.take();
        if let Some(mut f) = cb { f(paused); d.borrow_mut().on_pause = Some(f); }
    } else {
        let cb = d.borrow_mut().on_cancel.take();
        if let Some(mut f) = cb { f(); d.borrow_mut().on_cancel = Some(f); }
    }
    l.clone().redraw();
}
