//! `Sidebar`: a navigation rail that expands on hover.
//!
//! Collapsed it is a rail of icons; resting the pointer on it (hover intent
//! [`anim::ms::HOVER_OPEN`], 350 ms) expands it to show the labels, and it
//! collapses [`anim::ms::HOVER_CLOSE`] (140 ms) after the pointer leaves.
//! Keyboard focus inside expands it at once; Esc collapses it. Pinned, it
//! stays expanded. Selected = accent bar on the start edge + selected fill.
//!
//! - [`SidebarMode::Push`] (default): the rail's width changes in its row
//!   and the content beside it makes room. The width animates through the
//!   toolkit's automatic layout motion (190 ms OutCubic), like o3's
//!   sidebar reveal and Omarchy's tray drawer, which push too.
//! - [`SidebarMode::Overlay`]: the rail keeps its width; the expanded
//!   panel slides over the content (a non-modal layer).
//!
//! Right to left, the rail is on the right (its row mirrors), the accent
//! bar on its right edge and an overlay panel grows to the left.
//!
//! A file manager's places list fits in it: section headers
//! ([`SidebarItem::header`], optionally [`collapsible`](SidebarItem::collapsible)),
//! a dimmed second line with a usage bar ([`SidebarItem::detail`],
//! [`SidebarItem::usage`]), a trailing action such as eject
//! ([`SidebarItem::secondary`], [`Sidebar::on_secondary`]), a busy spinner,
//! dimmed and disabled rows, context menus ([`Sidebar::on_context`]), file
//! drops ([`Sidebar::on_drop_files`]) and stable keys
//! ([`SidebarItem::key`]) so [`Sidebar::set_items`] keeps the selection
//! and animates rows in and out. The rail scrolls when its rows do not fit.
//!
//! **Indexes** given to every callback and taken by every method are
//! indexes into the `Vec<SidebarItem>` the app passed (headers included).
//!
//! ```no_run
//! use fltk::prelude::*;
//! use fltk_omarchy::{layout::{self, FlexExt}, widgets::*, State};
//! let view = State::new(0usize);
//! let mut row = layout::row();
//! let side = Sidebar::new(vec![
//!     SidebarItem::new(glyph::FOLDER, "Files"),
//!     SidebarItem::new(glyph::SEARCH, "Search"),
//!     SidebarItem::new(glyph::SETTINGS, "Settings").footer(),
//! ], &view);
//! row.fit(&*side);
//! let content = fltk::frame::Frame::default(); // takes the rest
//! row.end();
//! ```

use crate::a11y::{self, Action, Role};
use crate::anim::{self, Anim, Ease};
use crate::layout::Mirror;
use crate::paint;
use crate::panel::ItemId;
use crate::rowmotion::RowMotion;
use crate::state::State;
use crate::theme::{self, Rgb, Theme};
use fltk::{app, enums::*, prelude::*, widget::Widget};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;

/// One entry of a [`Sidebar`]: a row, or a section header
/// ([`SidebarItem::header`]).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SidebarItem {
    pub icon: String,
    pub label: String,
    pub badge: String,
    pub footer: bool,
    /// Stable identity across [`Sidebar::set_items`] (empty: none).
    pub key: String,
    /// Dimmed second line shown when expanded.
    pub detail: String,
    /// Usage bar under the detail (0..1).
    pub usage: Option<f64>,
    /// Trailing action button: its glyph and tooltip (empty: none).
    pub secondary_icon: String,
    pub secondary_tooltip: String,
    /// A section header (not selectable).
    pub header: bool,
    /// A header that collapses its section when clicked.
    pub collapsible: bool,
    /// Drawn dimmed but still activatable.
    pub dimmed: bool,
    /// Dimmed, not activatable, skipped by the keyboard.
    pub disabled: bool,
    /// Shows a spinner.
    pub busy: bool,
}

impl SidebarItem {
    pub fn new(icon: &str, label: &str) -> SidebarItem {
        SidebarItem {
            icon: icon.into(),
            label: label.into(),
            ..Default::default()
        }
    }
    /// A section header: a dimmed caption over the rows that follow it (up
    /// to the next header), a hairline separator when the rail is
    /// collapsed. Not selectable; with [`collapsible`](Self::collapsible)
    /// a click, Enter or Space folds its rows.
    pub fn header(label: &str) -> SidebarItem {
        SidebarItem {
            label: label.into(),
            header: true,
            ..Default::default()
        }
    }
    /// A header that folds its section (click, Enter / Space, Left / Right).
    pub fn collapsible(mut self) -> Self {
        self.collapsible = true;
        self
    }
    /// A short count or mark at the end of the row (expanded).
    pub fn badge(mut self, b: &str) -> Self {
        self.badge = b.into();
        self
    }
    /// Kept at the bottom of the rail (settings, account).
    pub fn footer(mut self) -> Self {
        self.footer = true;
        self
    }
    /// A stable identity (a path, a device id): [`Sidebar::set_items`]
    /// keeps the selection, the cursor and folded sections on the item
    /// with the same key, and animates rows that arrive or leave.
    pub fn key(mut self, k: &str) -> Self {
        self.key = k.into();
        self
    }
    /// A dimmed second line ("12 GB free of 32 GB"); the row is two lines
    /// tall when expanded.
    pub fn detail(mut self, d: &str) -> Self {
        self.detail = d.into();
        self
    }
    /// A thin usage bar under the detail, `0..=1` (accent; the theme's
    /// urgent color above 0.9).
    pub fn usage(mut self, fraction: f64) -> Self {
        self.usage = fraction.is_finite().then(|| fraction.clamp(0.0, 1.0));
        self
    }
    /// A trailing icon button (eject, remove) shown when expanded, with its
    /// own hover and `tooltip` (also its accessible name). Clicked or
    /// Delete on the row: [`Sidebar::on_secondary`].
    pub fn secondary(mut self, icon: &str, tooltip: &str) -> Self {
        self.secondary_icon = icon.into();
        self.secondary_tooltip = tooltip.into();
        self
    }
    /// Drawn dimmed but still activatable (an unmounted device, a missing
    /// folder).
    pub fn dimmed(mut self) -> Self {
        self.dimmed = true;
        self
    }
    /// Dimmed, not activatable and skipped by the keyboard.
    pub fn disabled(mut self) -> Self {
        self.disabled = true;
        self
    }
    /// A spinner in place of the badge and the secondary button (in place
    /// of the icon while collapsed); it turns only while animations are on.
    pub fn busy(mut self, on: bool) -> Self {
        self.busy = on;
        self
    }

    fn has_secondary(&self) -> bool {
        !self.secondary_icon.is_empty() && !self.header
    }
}

/// How a [`Sidebar`] expands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SidebarMode {
    /// The rail grows in its row; the content beside it makes room.
    #[default]
    Push,
    /// The rail keeps its width; the expanded panel covers the content.
    Overlay,
}

/// The two places the rows are drawn: the rail widget, and the overlay
/// panel (overlay mode, expanded).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Surf {
    Rail,
    Layer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Part {
    Row,
    Secondary,
}

/// What a row that leaves is drawn with while it fades.
#[derive(Clone)]
struct GhostRow {
    item: SidebarItem,
    h: i32,
}

type Cb1 = Option<Box<dyn FnMut(usize)>>;

struct Sb {
    items: Vec<SidebarItem>,
    ids: Vec<ItemId>,
    collapsed: HashSet<ItemId>,
    hover: Option<usize>,
    hover_secondary: bool,
    cursor: usize,
    hovered: bool,
    layer_hovered: bool,
    open: bool,
    gen: u64,
    rail: i32,
    wide: i32,
    mode: SidebarMode,
    layer: Option<Widget>,
    layer_w: Anim<f64>,
    scroll: i32,
    wheel: crate::scroll::Scroll,
    drop_target: Option<usize>,
    drop_pending: Option<usize>,
    rm_rail: RowMotion<GhostRow>,
    rm_layer: RowMotion<GhostRow>,
    /// The tooltip of the rail / the layer is the hovered secondary's.
    tip: [bool; 2],
    on_activate: Cb1,
    on_secondary: Cb1,
    on_context: Option<Box<dyn FnMut(usize, i32, i32)>>,
    on_drop: Option<Box<dyn FnMut(usize, String)>>,
    on_drop_files: Option<Box<dyn FnMut(usize, Vec<PathBuf>)>>,
    accept_drop: Option<Rc<dyn Fn(usize) -> bool>>,
    on_section: Option<Box<dyn FnMut(usize, bool)>>,
}

/// Runs a callback stored in `Sb` with the state released (it may call
/// back into the sidebar), then puts it back unless it was replaced.
macro_rules! fire {
    ($st:expr, $field:ident, $($arg:expr),*) => {{
        let f = $st.borrow_mut().$field.take();
        if let Some(mut f) = f {
            f($($arg),*);
            let mut s = $st.borrow_mut();
            if s.$field.is_none() {
                s.$field = Some(f);
            }
        }
    }};
}

/// Navigation rail that expands on hover. See the [module docs](self).
#[derive(Clone)]
pub struct Sidebar {
    inner: Widget,
    st: Rc<RefCell<Sb>>,
    selected: State<usize>,
    pinned: State<bool>,
    expanded: State<bool>,
}
fltk::widget_extends!(Sidebar, Widget, inner);

// ================================================================ geometry

struct Metrics {
    /// A one-line row.
    rh: i32,
    /// A header.
    hh: i32,
    /// The label line and the detail line.
    lh: i32,
    dh: i32,
    /// Usage bar and the gap above it.
    bar: i32,
    gap: i32,
    inset: i32,
    pad: i32,
    /// The secondary button (square).
    bs: i32,
}

fn metrics(t: &Theme) -> Metrics {
    Metrics {
        rh: t.spacing.control_height + t.spacing.md,
        hh: t.font.caption + 2 * t.spacing.md + 2,
        lh: t.font.body + 6,
        dh: t.font.caption + 4,
        bar: t.spacing.xs.max(2),
        gap: t.spacing.xs,
        inset: t.spacing.sm,
        pad: t.spacing.md,
        bs: t.spacing.control_height - 2 * t.spacing.xs,
    }
}

/// Extra height of an expanded two-line row.
fn extra(it: &SidebarItem, m: &Metrics) -> i32 {
    let d = if it.detail.is_empty() { 0 } else { m.dh };
    let u = if it.usage.is_some() { m.gap + m.bar } else { 0 };
    d + u
}

fn item_h(it: &SidebarItem, m: &Metrics, reveal: f64) -> i32 {
    if it.header {
        m.hh
    } else {
        m.rh + (extra(it, m) as f64 * reveal).round() as i32
    }
}

/// How far the labels show (0 at rail width, 1 expanded).
fn reveal_of(rail: i32, wide: i32, w: i32) -> f64 {
    if wide > rail {
        ((w - rail) as f64 / (wide - rail) as f64).clamp(0.0, 1.0)
    } else {
        1.0
    }
}

/// Rows folded away by a collapsed header.
fn hidden_rows(items: &[SidebarItem], ids: &[ItemId], collapsed: &HashSet<ItemId>) -> Vec<bool> {
    let mut out = vec![false; items.len()];
    for footer in [false, true] {
        let mut hide = false;
        for (i, it) in items.iter().enumerate().filter(|(_, it)| it.footer == footer) {
            if it.header {
                hide = it.collapsible && collapsed.contains(&ids[i]);
            } else {
                out[i] = hide;
            }
        }
    }
    out
}

/// Identity of each item for motion and folded sections: its key, else
/// its icon and label (the n-th of equal ones).
fn identities(items: &[SidebarItem]) -> Vec<ItemId> {
    use std::hash::{Hash, Hasher};
    let mut seen: HashMap<(bool, &str, &str), u32> = HashMap::new();
    items
        .iter()
        .map(|it| {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            if it.key.is_empty() {
                let n = seen
                    .entry((it.header, it.icon.as_str(), it.label.as_str()))
                    .or_default();
                (1u8, it.header, &it.icon, &it.label, *n).hash(&mut h);
                *n += 1;
            } else {
                (0u8, &it.key).hash(&mut h);
            }
            h.finish()
        })
        .collect()
}

#[derive(Clone, Copy)]
struct Slot {
    /// Content y for rows that scroll; widget-relative y for footers.
    y: i32,
    h: i32,
    scrolls: bool,
}

struct Geo {
    rows: Vec<Option<Slot>>,
    /// The scrolling region, widget-relative `[top, bottom)`.
    view: (i32, i32),
    content: i32,
}

impl Geo {
    fn max_scroll(&self) -> i32 {
        (self.content - (self.view.1 - self.view.0)).max(0)
    }
    /// Widget-relative `(y, h)` of a shown row.
    fn at(&self, i: usize, scroll: i32) -> Option<(i32, i32)> {
        self.rows.get(i).copied().flatten().map(|s| {
            let y = if s.scrolls {
                self.view.0 + s.y - scroll
            } else {
                s.y
            };
            (y, s.h)
        })
    }
}

/// Top rows scroll between the top padding and the footer rows, which
/// stay at the bottom.
fn geometry(s: &Sb, h: i32, reveal: f64, t: &Theme) -> Geo {
    let m = metrics(t);
    let hidden = hidden_rows(&s.items, &s.ids, &s.collapsed);
    let heights: Vec<i32> = s.items.iter().map(|it| item_h(it, &m, reveal)).collect();
    let foot: i32 = (0..s.items.len())
        .filter(|&i| s.items[i].footer && !hidden[i])
        .map(|i| heights[i])
        .sum();
    let top = m.pad;
    let bottom = (h - m.pad - foot).max(top);
    let (mut cy, mut fy) = (0, h - m.pad - foot);
    let rows = (0..s.items.len())
        .map(|i| {
            if hidden[i] {
                return None;
            }
            let hh = heights[i];
            Some(if s.items[i].footer {
                let y = fy;
                fy += hh;
                Slot {
                    y,
                    h: hh,
                    scrolls: false,
                }
            } else {
                let y = cy;
                cy += hh;
                Slot {
                    y,
                    h: hh,
                    scrolls: true,
                }
            })
        })
        .collect();
    Geo {
        rows,
        view: (top, bottom),
        content: cy,
    }
}

/// The secondary button of a row, left to right: `(x, y, size)`.
fn secondary_box(m: &Metrics, t: &Theme, x: i32, w: i32, ry: i32, rh: i32) -> (i32, i32, i32) {
    let right = x + w - m.inset - t.spacing.xs;
    (right - m.bs, ry + (rh - m.bs) / 2, m.bs)
}

/// The row (and part) at widget point `(ex, ey)` of a surface drawn in
/// `(x, y, w, h)`.
fn hit(s: &Sb, area: (i32, i32, i32, i32), rtl: bool, ex: i32, ey: i32) -> Option<(usize, Part)> {
    let (x, y, w, h) = area;
    if ex < x || ex >= x + w || ey < y || ey >= y + h {
        return None;
    }
    theme::with(|t| {
        let m = metrics(t);
        let reveal = reveal_of(s.rail, s.wide, w);
        let geo = geometry(s, h, reveal, t);
        let scroll = s.scroll.clamp(0, geo.max_scroll());
        let ly = ey - y;
        let i = (0..s.items.len()).find(|&i| {
            geo.at(i, scroll).is_some_and(|(ry, rh)| {
                let in_view = !geo.rows[i].is_some_and(|r| r.scrolls)
                    || (ly >= geo.view.0 && ly < geo.view.1);
                in_view && ly >= ry && ly < ry + rh
            })
        })?;
        let it = &s.items[i];
        let (ry, rh) = geo.at(i, scroll)?;
        if it.has_secondary() && !it.busy && !it.disabled && reveal >= 0.5 {
            let px = Mirror::new(rtl, x, w).point(ex);
            let (bx, by, bs) = secondary_box(&m, t, x, w, y + ry, rh);
            if px >= bx && px < bx + bs && ey >= by && ey < by + bs {
                return Some((i, Part::Secondary));
            }
        }
        Some((i, Part::Row))
    })
}

// ================================================================ painting

fn surface(t: &Theme) -> Rgb {
    t.palette.foreground.over(t.palette.background, 0.04)
}

#[derive(Default, Clone, Copy)]
struct Look {
    selected: bool,
    hover: bool,
    hover_secondary: bool,
    ring: bool,
    drop: bool,
}

thread_local! {
    static EPOCH: std::time::Instant = std::time::Instant::now();
}

/// Spinner phase 0..1 (one turn per second); 0 with animations off.
fn spin_phase() -> f64 {
    if !anim::enabled() {
        return 0.0;
    }
    let e = EPOCH.with(|e| anim::now().saturating_duration_since(*e));
    e.as_secs_f64().fract()
}

/// Eight dots around `(cx, cy)`, the brightest at `phase`.
fn spinner(cx: i32, cy: i32, r: i32, color: Rgb, under: Rgb, phase: f64) {
    let d = ((r as f64) / 2.4).round().max(2.0) as i32;
    let head = phase * 8.0;
    for k in 0..8 {
        let a = k as f64 * std::f64::consts::FRAC_PI_4 - std::f64::consts::FRAC_PI_2;
        let rr = (r - d / 2) as f64;
        let px = (cx as f64 + rr * a.cos()).round() as i32 - d / 2;
        let py = (cy as f64 + rr * a.sin()).round() as i32 - d / 2;
        let lag = (head - k as f64).rem_euclid(8.0) / 8.0;
        paint::fill(px, py, d, d, d / 2, color.over(under, 1.0 - 0.8 * lag));
    }
}

/// Paints one row at `(x, ry, w, rh)`; true when it drew a spinner.
#[allow(clippy::too_many_arguments)]
fn paint_row(
    t: &Theme,
    m: &Metrics,
    rail: i32,
    it: &SidebarItem,
    (x, ry, w, rh): (i32, i32, i32, i32),
    reveal: f64,
    look: Look,
    mir: &Mirror,
    bg: Rgb,
    phase: f64,
) -> bool {
    let fg = t.palette.foreground;
    let (bx, bw) = (x + m.inset, (w - 2 * m.inset).max(0));
    let ring = |under: Rgb| {
        paint::stroke(
            mir.x(bx, bw),
            ry + 2,
            bw,
            rh - 4,
            t.radius,
            t.controls.focus.border_width.max(1),
            t.border(&t.controls.focus, under),
        );
    };
    if it.header {
        let hk = 1.0 - reveal;
        if hk > 0.0 {
            let (hx, hw) = (bx + t.spacing.sm, (bw - 2 * t.spacing.sm).max(0));
            paint::fill(mir.x(hx, hw), ry + rh / 2, hw, 1, 0, fg.over(bg, 0.12 * hk));
        }
        if reveal > 0.0 {
            let mut under = bg;
            if look.hover && it.collapsible {
                under = t.fill(&t.controls.hover, bg);
                paint::fill(mir.x(bx, bw), ry + 1, bw, rh - 2, t.radius, under);
            }
            if look.ring {
                ring(under);
            }
            let ink = fg.darker(1.4).over(under, reveal);
            let tx = bx + t.spacing.md;
            let mut right = x + w - m.inset - t.spacing.md;
            if it.collapsible {
                let cs = t.font.body_small + 4;
                let open = !look.selected; // `selected` carries "folded" for headers
                let g = if open {
                    crate::widgets::glyph::CHEVRON_DOWN
                } else {
                    mir.glyph(
                        crate::widgets::glyph::CHEVRON_RIGHT,
                        crate::widgets::glyph::CHEVRON_LEFT,
                    )
                };
                paint::glyph(g, mir.x(right - cs, cs), ry, cs, rh, t.font.body_small, ink);
                right -= cs + t.spacing.sm;
            }
            let lw = (right - tx).max(0);
            paint::text_elided(
                &it.label,
                mir.x(tx, lw),
                ry,
                lw,
                rh,
                t.font.caption,
                true,
                ink,
                mir.align(Align::Left),
            );
        }
        return false;
    }
    let mut under = bg;
    if look.selected {
        under = t.fill(&t.controls.selected, bg);
        paint::fill(mir.x(bx, bw), ry + 2, bw, rh - 4, t.radius, under);
        paint::fill(mir.x(x + 2, 3), ry + 6, 3, rh - 12, 1, t.palette.accent);
    } else if (look.hover && !it.disabled) || look.drop {
        under = t.fill(&t.controls.hover, bg);
        paint::fill(mir.x(bx, bw), ry + 2, bw, rh - 4, t.radius, under);
    }
    if look.drop {
        paint::stroke(mir.x(bx, bw), ry + 2, bw, rh - 4, t.radius, 2, t.palette.accent);
    }
    if look.ring {
        ring(under);
    }
    let base = if it.disabled {
        fg.over(under, 0.38)
    } else if it.dimmed {
        fg.over(under, 0.55)
    } else {
        fg
    };
    let ink = if look.selected && !it.disabled {
        t.palette.accent
    } else {
        base
    };
    let mut spun = false;
    if it.busy && reveal < 0.5 {
        spinner(x + rail / 2, ry + rh / 2, t.font.icon / 2, ink, under, phase);
        spun = true;
    } else {
        paint::glyph(&it.icon, mir.x(x, rail), ry, rail, rh, t.font.icon, ink);
    }
    if reveal <= 0.0 {
        return spun;
    }
    let k = reveal;
    let tx = x + rail;
    let ly = ry + (m.rh - m.lh) / 2;
    let mut right = x + w - m.inset - t.spacing.md;
    if it.busy {
        let (sx, sy, ss) = secondary_box(m, t, x, w, ry, rh);
        spinner(
            mir.x(sx, ss) + ss / 2,
            sy + ss / 2,
            ss / 3,
            base,
            under,
            phase,
        );
        spun = true;
        right = sx - t.spacing.sm;
    } else if it.has_secondary() {
        let (sx, sy, ss) = secondary_box(m, t, x, w, ry, rh);
        if look.hover_secondary && !it.disabled {
            let hot = t.fill(&t.controls.hover, under);
            paint::fill(mir.x(sx, ss), sy, ss, ss, t.radius, hot.over(under, k));
        }
        paint::glyph(
            &it.secondary_icon,
            mir.x(sx, ss),
            sy,
            ss,
            ss,
            t.font.body + 2,
            base.over(under, k),
        );
        right = sx - t.spacing.sm;
    }
    let text_right = right;
    if !it.badge.is_empty() && !it.busy {
        let bw2 = paint::measure(&it.badge, t.font.caption, false);
        paint::text(
            &it.badge,
            mir.x(right - bw2, bw2),
            ly,
            bw2,
            m.lh,
            t.font.caption,
            false,
            base.darker(1.3).over(under, k),
            mir.align(Align::Right),
        );
        right -= bw2 + t.spacing.md;
    }
    let lw = (right - tx).max(0);
    let text = if it.disabled || it.dimmed { base } else { fg };
    paint::text_elided(
        &it.label,
        mir.x(tx, lw),
        ly,
        lw,
        m.lh,
        t.font.body,
        look.selected,
        text.over(under, k),
        mir.align(Align::Left),
    );
    let mut y2 = ly + m.lh;
    let tw = (text_right - tx).max(0);
    if !it.detail.is_empty() {
        paint::text_elided(
            &it.detail,
            mir.x(tx, tw),
            y2,
            tw,
            m.dh,
            t.font.caption,
            false,
            base.darker(1.4).over(under, k),
            mir.align(Align::Left),
        );
        y2 += m.dh;
    }
    if let Some(u) = it.usage {
        let by = y2 + m.gap;
        paint::fill(mir.x(tx, tw), by, tw, m.bar, m.bar / 2, fg.over(under, 0.15 * k));
        let fw = (tw as f64 * u).round() as i32;
        let c = if u > 0.9 {
            t.palette.urgent
        } else {
            t.palette.accent
        };
        let c = if it.disabled || it.dimmed {
            c.over(under, 0.5)
        } else {
            c
        };
        paint::fill(mir.x(tx, fw), by, fw, m.bar, m.bar / 2, c.over(under, k));
    }
    spun
}

/// Paints a surface in `(x, y, w, h)` (`w` between the rail and the
/// expanded width), mirrored when `rtl`. `active`: the surface on screen
/// (it runs the wheel motion). Schedules the next frame while rows move,
/// the wheel glides or a spinner turns.
#[allow(clippy::too_many_arguments)]
fn paint_surface(
    s: &mut Sb,
    surf: Surf,
    sel: usize,
    (x, y, w, h): (i32, i32, i32, i32),
    rtl: bool,
    ring: bool,
    active: bool,
    widget: &Widget,
) {
    let mut again = false;
    let mut rm = std::mem::take(match surf {
        Surf::Rail => &mut s.rm_rail,
        Surf::Layer => &mut s.rm_layer,
    });
    theme::with(|t| {
        let m = metrics(t);
        let mir = Mirror::new(rtl, x, w);
        let bg = surface(t);
        paint::fill(x, y, w, h, 0, bg);
        let fg = t.palette.foreground;
        // Hairline on the edge facing the content.
        paint::fill(mir.x(x + w - 1, 1), y, 1, h, 0, fg.over(bg, 0.12));
        let reveal = reveal_of(s.rail, s.wide, w);
        let geo = geometry(s, h, reveal, t);
        let max = geo.max_scroll();
        if active {
            s.scroll = s.wheel.position(s.scroll as i64, max as i64) as i32;
            again |= s.wheel.running();
        }
        let scroll = s.scroll.clamp(0, max);
        let n = s.items.len();
        let alive: HashSet<ItemId> = (0..n)
            .filter(|&i| geo.rows[i].is_some())
            .map(|i| s.ids[i])
            .collect();
        let inview: Vec<(ItemId, i32)> = (0..n)
            .filter_map(|i| {
                let (ry, rh) = geo.at(i, scroll)?;
                (ry + rh > 0 && ry < h).then_some((s.ids[i], ry))
            })
            .collect();
        rm.begin(&inview, |id| alive.contains(&id));
        let drift = (m.rh / 4) as f64;
        let phase = spin_phase();
        let mut spun = false;
        fltk::draw::push_clip(x, y, w, h);
        // Rows leaving fade out under the others.
        for g in &rm.ghosts {
            let (dy, o) = g.sample(drift);
            let ry = y + g.y + dy;
            fltk::draw::push_clip(x, ry, w, g.row.h);
            paint_row(
                t,
                &m,
                s.rail,
                &g.row.item,
                (x, ry, w, g.row.h),
                reveal,
                Look::default(),
                &mir,
                bg,
                0.0,
            );
            paint::alpha_fill(x + 1, ry, w - 2, g.row.h, bg, 1.0 - o);
            fltk::draw::pop_clip();
        }
        let mut drawn = Vec::new();
        for i in 0..n {
            let Some((ry0, rh)) = geo.at(i, scroll) else {
                continue;
            };
            if ry0 + rh <= 0 || ry0 >= h {
                continue;
            }
            let it = &s.items[i];
            let (dy, op, _) = rm.at(s.ids[i], drift);
            let ry = y + ry0 + dy;
            let scrolls = geo.rows[i].is_some_and(|r| r.scrolls);
            if scrolls {
                fltk::draw::push_clip(x, y + geo.view.0, w, geo.view.1 - geo.view.0);
            }
            fltk::draw::push_clip(x, ry, w, rh);
            let folded = it.header && s.collapsed.contains(&s.ids[i]);
            let look = Look {
                selected: if it.header { folded } else { i == sel },
                hover: s.hover == Some(i) && !s.hover_secondary,
                hover_secondary: s.hover == Some(i) && s.hover_secondary,
                ring: ring && i == s.cursor,
                drop: s.drop_target == Some(i),
            };
            spun |= paint_row(
                t,
                &m,
                s.rail,
                it,
                (x, ry, w, rh),
                reveal,
                look,
                &mir,
                bg,
                phase,
            );
            if op < 1.0 {
                paint::alpha_fill(x + 1, ry, w - 2, rh, bg, 1.0 - op);
            }
            fltk::draw::pop_clip();
            if scrolls {
                fltk::draw::pop_clip();
            }
            if anim::enabled() {
                drawn.push((
                    s.ids[i],
                    ry - y,
                    GhostRow {
                        item: it.clone(),
                        h: rh,
                    },
                ));
            }
        }
        if max > 0 {
            let track = geo.view.1 - geo.view.0;
            let thumb = ((track as f64 * track as f64 / geo.content.max(1) as f64) as i32)
                .clamp(16.min(track), track);
            let ty = y
                + geo.view.0
                + ((scroll as f64 / max as f64) * (track - thumb) as f64).round() as i32;
            paint::fill(mir.x(x + w - 5, 3), ty, 3, thumb, 1, fg.over(bg, 0.25));
        }
        fltk::draw::pop_clip();
        rm.end(drawn);
        again |= rm.running() || (spun && anim::enabled());
    });
    match surf {
        Surf::Rail => s.rm_rail = rm,
        Surf::Layer => s.rm_layer = rm,
    }
    if again {
        anim::request_redraw(widget);
    }
}

/// Offset of `w`'s window in its top-level window (subwindows).
fn window_origin(w: &Widget) -> (i32, i32) {
    let (mut ox, mut oy) = (0, 0);
    let mut win = w.window();
    while let Some(x) = win {
        if x.parent().is_none() {
            break;
        }
        ox += x.x();
        oy += x.y();
        win = x.window();
    }
    (ox, oy)
}

// ================================================================== widget

impl Sidebar {
    /// A rail with `items`; `selected` is the selected item's index.
    pub fn new(items: Vec<SidebarItem>, selected: &State<usize>) -> Sidebar {
        let (rail, wide) = theme::with(|t| {
            (
                t.spacing.control_height + 2 * t.spacing.lg,
                240.max(t.font.body * 20),
            )
        });
        let mut inner = Widget::default().with_size(rail, 240);
        inner.set_frame(FrameType::NoBox);
        inner.set_visible_focus();
        let ids = identities(&items);
        let st = Rc::new(RefCell::new(Sb {
            items,
            ids,
            collapsed: HashSet::new(),
            hover: None,
            hover_secondary: false,
            cursor: selected.get(),
            hovered: false,
            layer_hovered: false,
            open: false,
            gen: 0,
            rail,
            wide,
            mode: SidebarMode::Push,
            layer: None,
            layer_w: Anim::new(),
            scroll: 0,
            wheel: crate::scroll::Scroll::new(),
            drop_target: None,
            drop_pending: None,
            rm_rail: RowMotion::default(),
            rm_layer: RowMotion::default(),
            tip: [false; 2],
            on_activate: None,
            on_secondary: None,
            on_context: None,
            on_drop: None,
            on_drop_files: None,
            accept_drop: None,
            on_section: None,
        }));
        let sb = Sidebar {
            inner,
            st,
            selected: selected.clone(),
            pinned: State::new(false),
            expanded: State::new(false),
        };
        sb.install();
        sb
    }

    /// Width collapsed (the rail) and expanded.
    pub fn widths(self, rail: i32, expanded: i32) -> Self {
        {
            let mut s = self.st.borrow_mut();
            s.rail = rail.max(1);
            s.wide = expanded.max(s.rail);
        }
        let mut w = self.inner.clone();
        let h = w.h();
        w.set_size(if self.expanded.get() { expanded } else { rail }, h);
        self
    }

    /// Push (default) or overlay, see [`SidebarMode`].
    pub fn mode(self, m: SidebarMode) -> Self {
        self.st.borrow_mut().mode = m;
        self
    }

    /// Keeps it expanded while `pinned` is true (two-way: [`Sidebar::set_pinned`]).
    pub fn bind_pinned(mut self, pinned: &State<bool>) -> Self {
        self.pinned = pinned.clone();
        let me = self.clone();
        crate::state::bind_widget(&self.inner, pinned, true, move |_| me.apply());
        self
    }

    /// Runs when an item is picked (click, Enter, Space), after `selected` changed.
    pub fn on_activate(self, f: impl FnMut(usize) + 'static) -> Self {
        self.st.borrow_mut().on_activate = Some(Box::new(f));
        self
    }

    /// Runs when an item's secondary button ([`SidebarItem::secondary`]) is
    /// clicked, or Delete is pressed on it. Not while it is busy or
    /// disabled. The row is not activated.
    pub fn on_secondary(self, f: impl FnMut(usize) + 'static) -> Self {
        self.st.borrow_mut().on_secondary = Some(Box::new(f));
        self
    }

    /// Context menu for an item: right click (at the pointer), or the Menu
    /// key / Shift+F10 on the cursor row (at the row's bottom start
    /// corner). `x, y` are coordinates in the top-level window, ready for
    /// `PopupMenu::open_at(&*sidebar, x, y, ..)`. Headers get it too; disabled
    /// rows do not.
    pub fn on_context(self, f: impl FnMut(usize, i32, i32) + 'static) -> Self {
        self.st.borrow_mut().on_context = Some(Box::new(f));
        self
    }

    /// Something dropped on an item: the text as FLTK delivers it (one
    /// `file://` URI per line from file managers on Linux, paths on macOS
    /// and Windows). Turns on the drop target: the row under the pointer is
    /// highlighted during the drag. See [`Sidebar::accept_drop`].
    pub fn on_drop(self, f: impl FnMut(usize, String) + 'static) -> Self {
        self.st.borrow_mut().on_drop = Some(Box::new(f));
        self
    }

    /// Files dropped on an item, as paths (the dropped text parsed by
    /// [`crate::edit::parse_files`]; not called when it is not a list of
    /// files). Turns on the drop target like [`Sidebar::on_drop`].
    pub fn on_drop_files(self, f: impl FnMut(usize, Vec<PathBuf>) + 'static) -> Self {
        self.st.borrow_mut().on_drop_files = Some(Box::new(f));
        self
    }

    /// Which items take drops (default: every row that is not a header or
    /// disabled). E.g. no drops on read-only devices.
    pub fn accept_drop(self, f: impl Fn(usize) -> bool + 'static) -> Self {
        self.st.borrow_mut().accept_drop = Some(Rc::new(f));
        self
    }

    /// Runs when the user folds (`true`) or unfolds a collapsible header's
    /// section (click, keys, screen reader), with the header's index.
    pub fn on_section_toggle(self, f: impl FnMut(usize, bool) + 'static) -> Self {
        self.st.borrow_mut().on_section = Some(Box::new(f));
        self
    }

    pub fn set_pinned(&self, on: bool) {
        self.pinned.set(on);
        self.apply();
    }

    pub fn is_pinned(&self) -> bool {
        self.pinned.get()
    }

    /// True while expanded (pinned, hovered or focused). Bind it to follow.
    pub fn expanded(&self) -> State<bool> {
        self.expanded.clone()
    }

    pub fn is_expanded(&self) -> bool {
        self.expanded.get()
    }

    /// Expands now (as hover would, without the delay).
    pub fn expand(&self) {
        self.st.borrow_mut().open = true;
        self.apply();
    }

    /// Collapses unless pinned.
    pub fn collapse(&self) {
        self.st.borrow_mut().open = false;
        self.apply();
    }

    pub fn items(&self) -> Vec<SidebarItem> {
        self.st.borrow().items.clone()
    }

    /// Replaces the items. Expanded and pinned state and the scroll
    /// position stay. With keys ([`SidebarItem::key`]) the selection, the
    /// cursor, the hovered row and folded sections follow their item to
    /// its new index (`selected` is set to it; `on_activate` does not run),
    /// rows that arrive fade in, rows that leave fade out and the others
    /// slide (the list row motion; at once with animations off). Without
    /// keys indexes stay, clamped to the new length.
    pub fn set_items(&mut self, items: Vec<SidebarItem>) {
        let moved_sel = {
            let mut s = self.st.borrow_mut();
            let ids = identities(&items);
            let len = items.len();
            let old = std::mem::take(&mut s.items);
            let follow = |i: usize| -> Option<usize> {
                let k = &old.get(i)?.key;
                if k.is_empty() {
                    return None;
                }
                items.iter().position(|it| &it.key == k)
            };
            let sel = self.selected.get();
            let moved_sel = follow(sel).filter(|&n| n != sel);
            s.cursor = follow(s.cursor).unwrap_or(s.cursor.min(len.saturating_sub(1)));
            s.hover = s
                .hover
                .and_then(|h| follow(h).or(Some(h)))
                .filter(|&h| h < len);
            s.drop_target = None;
            s.drop_pending = None;
            let before: HashSet<ItemId> = s.ids.iter().copied().collect();
            let inserted: Vec<ItemId> = ids.iter().copied().filter(|i| !before.contains(i)).collect();
            s.rm_rail.changed(&inserted, &[]);
            s.rm_layer.changed(&inserted, &[]);
            let now: HashSet<ItemId> = ids.iter().copied().collect();
            s.collapsed.retain(|id| now.contains(id));
            s.items = items;
            s.ids = ids;
            moved_sel
        };
        if let Some(n) = moved_sel {
            self.selected.set(n);
        }
        a11y::changed(&self.inner);
        self.redraw_all();
    }

    /// True when header `index` is collapsible and folded.
    pub fn section_collapsed(&self, index: usize) -> bool {
        let s = self.st.borrow();
        s.items.get(index).is_some_and(|it| it.header && it.collapsible)
            && s.collapsed.contains(&s.ids[index])
    }

    /// Folds (`true`) or unfolds the section of collapsible header `index`;
    /// the rows below slide (200 ms, as a tree branch). Remembered by the
    /// header's key (or label) across [`Sidebar::set_items`].
    pub fn set_section_collapsed(&self, index: usize, collapsed: bool) {
        self.fold(index, collapsed, false);
    }

    fn fold(&self, index: usize, collapsed: bool, user: bool) {
        let changed = {
            let mut s = self.st.borrow_mut();
            if !s
                .items
                .get(index)
                .is_some_and(|it| it.header && it.collapsible)
            {
                return;
            }
            let id = s.ids[index];
            let changed = if collapsed {
                s.collapsed.insert(id)
            } else {
                s.collapsed.remove(&id)
            };
            if changed {
                s.rm_rail.reflowed(anim::ms::REVEAL);
                s.rm_layer.reflowed(anim::ms::REVEAL);
                let hidden = hidden_rows(&s.items, &s.ids, &s.collapsed);
                if hidden.get(s.cursor).copied().unwrap_or(false) {
                    s.cursor = index;
                }
                if s.hover.is_some_and(|h| hidden.get(h).copied().unwrap_or(false)) {
                    s.hover = None;
                }
            }
            changed
        };
        if !changed {
            return;
        }
        if user {
            fire!(self.st, on_section, index, collapsed);
        }
        a11y::changed(&self.inner);
        self.redraw_all();
    }

    /// The current scroll offset of the rows (pixels) and its maximum.
    pub fn scroll_position(&self) -> (i32, i32) {
        let (_, area) = self.visible();
        let s = self.st.borrow();
        let max = theme::with(|t| {
            geometry(&s, area.3, reveal_of(s.rail, s.wide, area.2), t).max_scroll()
        });
        (s.scroll.clamp(0, max), max)
    }

    /// The row being dragged over (drop target), if any.
    pub fn drop_target(&self) -> Option<usize> {
        self.st.borrow().drop_target
    }

    /// Where item `index` is drawn now (row motion included), in the
    /// sidebar's window coordinates (like `x()`/`y()`); None when it is
    /// folded away, scrolled out of view or out of range.
    pub fn item_rect(&self, index: usize) -> Option<(i32, i32, i32, i32)> {
        let (surf, (x, y, w, h)) = self.visible();
        let s = self.st.borrow();
        theme::with(|t| {
            let m = metrics(t);
            let geo = geometry(&s, h, reveal_of(s.rail, s.wide, w), t);
            let scroll = s.scroll.clamp(0, geo.max_scroll());
            let (ry, rh) = geo.at(index, scroll)?;
            let rm = match surf {
                Surf::Rail => &s.rm_rail,
                Surf::Layer => &s.rm_layer,
            };
            let (dy, _, _) = rm.at(s.ids[index], (m.rh / 4) as f64);
            let ry = ry + dy;
            if geo.rows[index].is_some_and(|r| r.scrolls)
                && (ry + rh <= geo.view.0 || ry >= geo.view.1)
            {
                return None;
            }
            let (ox, oy) = match surf {
                Surf::Rail => (0, 0),
                // The layer is in the top-level window.
                Surf::Layer => {
                    let o = window_origin(&self.inner);
                    (-o.0, -o.1)
                }
            };
            Some((x + ox, y + ry + oy, w, rh))
        })
    }

    /// Simulates the pointer resting on (`true`) or leaving it, through the
    /// same path as real Enter/Leave events, hover intent included.
    pub fn hover(&self, on: bool) {
        if on {
            self.entered()
        } else {
            self.left()
        }
    }

    fn entered(&self) {
        let gen = {
            let mut s = self.st.borrow_mut();
            s.hovered = true;
            s.gen += 1;
            s.gen
        };
        let me = self.clone();
        app::add_timeout3(anim::ms::HOVER_OPEN as f64 / 1000.0, move |_| {
            let go = {
                let s = me.st.borrow();
                s.gen == gen && (s.hovered || s.layer_hovered)
            };
            if go && !me.inner.was_deleted() {
                me.expand();
            }
        });
    }

    fn left(&self) {
        let gen = {
            let mut s = self.st.borrow_mut();
            s.hovered = false;
            s.hover = None;
            s.gen += 1;
            s.gen
        };
        self.inner.clone().redraw();
        let me = self.clone();
        app::add_timeout3(anim::ms::HOVER_CLOSE as f64 / 1000.0, move |_| {
            if me.inner.was_deleted() {
                return;
            }
            let stay = {
                let s = me.st.borrow();
                s.gen != gen || s.hovered || s.layer_hovered
            };
            let held =
                crate::focus::is_focused(&me.inner) || crate::overlay::modal_open_over(&me.inner);
            if !stay && !held {
                me.collapse();
            }
        });
    }

    fn activate(&self, i: usize) {
        {
            let s = self.st.borrow();
            match s.items.get(i) {
                Some(it) if !it.header && !it.disabled => {}
                _ => return,
            }
        }
        self.st.borrow_mut().cursor = i;
        self.selected.set(i);
        fire!(self.st, on_activate, i);
        self.redraw_all();
    }

    fn secondary(&self, i: usize) {
        let ok = self
            .st
            .borrow()
            .items
            .get(i)
            .is_some_and(|it| it.has_secondary() && !it.busy && !it.disabled);
        if ok {
            self.st.borrow_mut().cursor = i;
            fire!(self.st, on_secondary, i);
            self.redraw_all();
        }
    }

    fn context(&self, i: usize, x: i32, y: i32) -> bool {
        let ok = {
            let s = self.st.borrow();
            s.on_context.is_some() && s.items.get(i).is_some_and(|it| !it.disabled)
        };
        if ok {
            fire!(self.st, on_context, i, x, y);
        }
        ok
    }

    fn redraw_all(&self) {
        self.inner.clone().redraw();
        let layer = self.st.borrow().layer.clone();
        if let Some(mut l) = layer.filter(|l| !l.was_deleted()) {
            l.redraw();
        }
    }

    /// The surface on screen and the rectangle its rows are laid out in
    /// (the overlay panel at its target width).
    fn visible(&self) -> (Surf, (i32, i32, i32, i32)) {
        Self::visible_of(&self.st.borrow(), &self.inner)
    }

    fn visible_of(s: &Sb, inner: &Widget) -> (Surf, (i32, i32, i32, i32)) {
        if s.mode == SidebarMode::Overlay {
            if let Some(l) = s.layer.as_ref().filter(|l| !l.was_deleted()) {
                let w = s.layer_w.target().unwrap_or(s.rail as f64).round() as i32;
                let x = if crate::layout::is_rtl(inner) {
                    l.x() + l.w() - w
                } else {
                    l.x()
                };
                return (Surf::Layer, (x, l.y(), w, l.h()));
            }
        }
        let w = inner;
        (Surf::Rail, (w.x(), w.y(), w.w(), w.h()))
    }

    fn area(&self, surf: Surf, w: &Widget) -> (i32, i32, i32, i32) {
        match self.visible() {
            (v, a) if v == surf => a,
            _ => (w.x(), w.y(), w.w(), w.h()),
        }
    }

    /// Pointer moved over a surface: hover and the secondary's tooltip.
    fn track(&self, surf: Surf, w: &mut Widget) {
        let area = self.area(surf, w);
        let rtl = crate::layout::is_rtl(&self.inner);
        let (ex, ey) = app::event_coords();
        let (hover, sec, tip) = {
            let s = self.st.borrow();
            match hit(&s, area, rtl, ex, ey) {
                Some((i, part)) => {
                    let it = &s.items[i];
                    if it.disabled || (it.header && !it.collapsible) {
                        (None, false, None)
                    } else {
                        let sec = part == Part::Secondary;
                        (Some(i), sec, sec.then(|| it.secondary_tooltip.clone()))
                    }
                }
                None => (None, false, None),
            }
        };
        let changed = {
            let mut s = self.st.borrow_mut();
            let c = s.hover != hover || s.hover_secondary != sec;
            s.hover = hover;
            s.hover_secondary = sec;
            c
        };
        self.set_tip(surf, w, tip);
        if changed {
            w.redraw();
        }
    }

    fn set_tip(&self, surf: Surf, w: &mut Widget, tip: Option<String>) {
        let k = (surf == Surf::Layer) as usize;
        match tip {
            Some(t) if !t.is_empty() => {
                if w.tooltip().as_deref() != Some(t.as_str()) {
                    w.set_tooltip(&t);
                }
                self.st.borrow_mut().tip[k] = true;
            }
            _ => {
                if std::mem::replace(&mut self.st.borrow_mut().tip[k], false) {
                    w.set_tooltip("");
                }
            }
        }
    }

    /// A click on a surface.
    fn click(&self, surf: Surf, w: &Widget) {
        let area = self.area(surf, w);
        let rtl = crate::layout::is_rtl(&self.inner);
        let (ex, ey) = app::event_coords();
        let Some((i, part)) = hit(&self.st.borrow(), area, rtl, ex, ey) else {
            return;
        };
        let it = self.st.borrow().items[i].clone();
        if app::event_mouse_button() == app::MouseButton::Right {
            if !it.disabled && (!it.header || it.collapsible) {
                self.st.borrow_mut().cursor = i;
            }
            let (ox, oy) = window_origin(w);
            if self.context(i, ex + ox, ey + oy) {
                self.redraw_all();
            }
            return;
        }
        if it.header {
            if it.collapsible {
                self.st.borrow_mut().cursor = i;
                let folded = self.section_collapsed(i);
                self.fold(i, !folded, true);
            }
        } else if part == Part::Secondary {
            self.secondary(i);
        } else {
            self.activate(i);
        }
    }

    fn wheel(&self, inside: bool) -> bool {
        let (_, area) = self.visible();
        let max = {
            let s = self.st.borrow();
            theme::with(|t| {
                geometry(&s, area.3, reveal_of(s.rail, s.wide, area.2), t).max_scroll()
            })
        };
        if max == 0 {
            return inside;
        }
        let rh = theme::with(|t| metrics(t).rh);
        {
            let mut s = self.st.borrow_mut();
            let cur = s.scroll.clamp(0, max) as i64;
            let dy = app::event_dy_value() as f64 * rh as f64;
            s.scroll = s.wheel.wheel(cur, dy, max as i64) as i32;
        }
        self.redraw_all();
        true
    }

    fn can_drop(&self, i: usize) -> bool {
        let filter = {
            let s = self.st.borrow();
            if !s.items.get(i).is_some_and(|it| !it.header && !it.disabled) {
                return false;
            }
            s.accept_drop.clone()
        };
        filter.map_or(true, |f| f(i))
    }

    /// Drag and drop over a surface (FLTK: accept DndEnter/DndDrag/
    /// DndRelease, then the text comes in Event::Paste).
    fn dnd(&self, surf: Surf, w: &Widget, ev: Event) -> bool {
        {
            let s = self.st.borrow();
            if s.on_drop.is_none() && s.on_drop_files.is_none() {
                return false;
            }
        }
        match ev {
            Event::DndEnter | Event::DndDrag => {
                if ev == Event::DndEnter {
                    if surf == Surf::Rail {
                        self.entered();
                    } else {
                        self.st.borrow_mut().layer_hovered = true;
                    }
                }
                let area = self.area(surf, w);
                let rtl = crate::layout::is_rtl(&self.inner);
                let (ex, ey) = app::event_coords();
                let row = hit(&self.st.borrow(), area, rtl, ex, ey).map(|h| h.0);
                let target = row.filter(|&i| self.can_drop(i));
                let changed =
                    std::mem::replace(&mut self.st.borrow_mut().drop_target, target) != target;
                if changed {
                    self.redraw_all();
                }
                target.is_some()
            }
            Event::DndLeave => {
                {
                    let mut s = self.st.borrow_mut();
                    s.drop_target = None;
                    if surf == Surf::Layer {
                        s.layer_hovered = false;
                    }
                }
                self.left();
                self.redraw_all();
                true
            }
            Event::DndRelease => {
                let t = {
                    let mut s = self.st.borrow_mut();
                    let t = s.drop_target.take();
                    s.drop_pending = t;
                    t
                };
                self.redraw_all();
                t.is_some()
            }
            _ => false,
        }
    }

    /// The dropped text (after an accepted DndRelease).
    fn paste(&self) -> bool {
        let Some(i) = self.st.borrow_mut().drop_pending.take() else {
            return false;
        };
        let text = app::event_text();
        fire!(self.st, on_drop, i, text.clone());
        if self.st.borrow().on_drop_files.is_some() {
            if let Some(paths) = crate::edit::parse_files(&text, true) {
                fire!(self.st, on_drop_files, i, paths);
            }
        }
        self.redraw_all();
        true
    }

    /// Rows the keyboard lands on, in screen order (top rows, then footers).
    fn order(s: &Sb) -> Vec<usize> {
        let hidden = hidden_rows(&s.items, &s.ids, &s.collapsed);
        let ok = |i: usize| {
            let it = &s.items[i];
            !hidden[i] && !it.disabled && (!it.header || it.collapsible)
        };
        let n = s.items.len();
        (0..n)
            .filter(|&i| !s.items[i].footer && ok(i))
            .chain((0..n).filter(|&i| s.items[i].footer && ok(i)))
            .collect()
    }

    /// Scrolls so the cursor row is in view.
    fn reveal_cursor(&self) {
        let (_, (_, _, _, h)) = self.visible();
        let reveal = if self.is_expanded() { 1.0 } else { 0.0 };
        let mut s = self.st.borrow_mut();
        let cur = s.cursor;
        let target = theme::with(|t| {
            let geo = geometry(&s, h, reveal, t);
            let slot = geo.rows.get(cur).copied().flatten().filter(|r| r.scrolls)?;
            let view = geo.view.1 - geo.view.0;
            let sc = s.scroll.clamp(0, geo.max_scroll());
            Some(if slot.y < sc {
                slot.y
            } else if slot.y + slot.h > sc + view {
                (slot.y + slot.h - view).min(geo.max_scroll())
            } else {
                sc
            })
        });
        if let Some(v) = target {
            s.scroll = v;
            s.wheel.jump(v as i64);
        }
    }

    fn key(&self, w: &mut Widget) -> bool {
        let k = crate::layout::logical_key(&self.inner, app::event_key());
        let mods = app::event_state();
        let cmd = mods.intersects(Shortcut::Ctrl | Shortcut::Alt | Shortcut::Meta);
        let shift = mods.contains(Shortcut::Shift);
        let plain = !cmd && !shift;
        let (order, cur, it) = {
            let s = self.st.borrow();
            let order = Self::order(&s);
            let cur = s.cursor;
            (order, cur, s.items.get(cur).cloned())
        };
        if k == Key::Escape {
            if self.is_expanded() && !self.is_pinned() {
                self.collapse();
                return true;
            }
            return false;
        }
        if order.is_empty() || cmd {
            return false;
        }
        let pos = order.iter().position(|&i| i == cur);
        // Not on a row the keyboard can use (initial index on a header):
        // the next usable one.
        let near = || order.iter().position(|&i| i >= cur).unwrap_or(0);
        let page = {
            let (_, (_, _, _, h)) = self.visible();
            theme::with(|t| ((h - 2 * metrics(t).pad) / metrics(t).rh).max(1)) as usize
        };
        let next = match k {
            Key::Up if plain => Some(pos.map_or(near(), |p| p.saturating_sub(1))),
            Key::Down if plain => Some(pos.map_or(near(), |p| (p + 1).min(order.len() - 1))),
            Key::PageUp if plain => Some(pos.map_or(near(), |p| p.saturating_sub(page))),
            Key::PageDown if plain => {
                Some(pos.map_or(near(), |p| (p + page).min(order.len() - 1)))
            }
            Key::Home if plain => Some(0),
            Key::End if plain => Some(order.len() - 1),
            _ => None,
        };
        if let Some(p) = next {
            self.st.borrow_mut().cursor = order[p];
            if !self.is_expanded() {
                self.expand();
            }
            self.reveal_cursor();
            w.redraw();
            self.redraw_all();
            return true;
        }
        let Some(it) = it.filter(|_| pos.is_some()) else {
            return false;
        };
        let space = k == Key::from_char(' ');
        match k {
            Key::Enter | Key::KPEnter if plain => {
                if it.header {
                    let f = self.section_collapsed(cur);
                    self.fold(cur, !f, true);
                } else {
                    self.activate(cur);
                }
                true
            }
            _ if space && plain => {
                if it.header {
                    let f = self.section_collapsed(cur);
                    self.fold(cur, !f, true);
                } else {
                    self.activate(cur);
                }
                true
            }
            Key::Left | Key::Right if plain && it.header => {
                self.fold(cur, k == Key::Left, true);
                true
            }
            Key::Delete if plain => {
                if it.has_secondary() && !it.busy {
                    self.secondary(cur);
                    true
                } else {
                    false
                }
            }
            _ if (k == Key::Menu && plain) || (k == Key::F10 && shift && !cmd) => {
                let Some((x, y, rw, rh)) = self.item_rect(cur) else {
                    return false;
                };
                let rtl = crate::layout::is_rtl(&self.inner);
                let inset = theme::with(|t| metrics(t).inset);
                let px = if rtl { x + rw - inset } else { x + inset };
                // item_rect is in the sidebar's window; on_context wants the top window.
                let (ox, oy) = window_origin(&self.inner);
                self.context(cur, px + ox, y + rh + oy)
            }
            _ => false,
        }
    }

    /// Expanded = pinned or open; moves the width (push) or the layer (overlay).
    fn apply(&self) {
        let want = self.pinned.get() || self.st.borrow().open;
        if self.expanded.get() != want {
            self.expanded.set(want);
        }
        let (mode, rail, wide) = {
            let s = self.st.borrow();
            (s.mode, s.rail, s.wide)
        };
        match mode {
            SidebarMode::Push => {
                let target = if want { wide } else { rail };
                if self.inner.w() != target
                    && !crate::layout::refit(&self.inner, (target, self.inner.h()))
                {
                    {
                        let mut w = self.inner.clone();
                        let h = w.h();
                        w.set_size(target, h);
                        if let Some(mut p) = w.parent() {
                            p.redraw();
                        }
                    }
                }
            }
            SidebarMode::Overlay => {
                if want {
                    self.open_layer();
                }
                if let Some(mut l) = self.st.borrow().layer.clone() {
                    l.redraw();
                }
            }
        }
        a11y::changed(&self.inner);
        self.inner.clone().redraw();
    }

    /// Where the overlay panel goes: over the rail, in top-level window
    /// coordinates (the rail may be in a subwindow).
    fn layer_rect(&self) -> (i32, i32, i32, i32) {
        let wide = self.st.borrow().wide;
        let (ox, oy) = window_origin(&self.inner);
        let rtl = crate::layout::is_rtl(&self.inner);
        let x = if rtl {
            self.inner.x() + self.inner.w() - wide
        } else {
            self.inner.x()
        };
        (x + ox, self.inner.y() + oy, wide, self.inner.h())
    }

    fn open_layer(&self) {
        if self
            .st
            .borrow()
            .layer
            .as_ref()
            .is_some_and(|l| !l.was_deleted())
        {
            return;
        }
        let rail = self.st.borrow().rail;
        let Some(mut layer) =
            crate::overlay::open_layer(&self.inner, Some(self.layer_rect()), false)
        else {
            return;
        };
        {
            let mut s = self.st.borrow_mut();
            s.layer_w = Anim::new();
            s.layer_w.jump(rail as f64);
            s.layer = Some(layer.clone());
            s.rm_layer = RowMotion::default();
            s.tip[1] = false;
        }
        let me = self.clone();
        layer.draw(move |l| {
            // Follows the rail (window resized, layout changed).
            let r = me.layer_rect();
            if (l.x(), l.y(), l.w(), l.h()) != r {
                l.resize(r.0, r.1, r.2, r.3);
            }
            let mut s = me.st.borrow_mut();
            let want = me.expanded.get();
            let (rail, wide) = (s.rail as f64, s.wide as f64);
            let w = s
                .layer_w
                .get(
                    if want { wide } else { rail },
                    anim::ms::LAYOUT,
                    Ease::OutCubic,
                    l,
                )
                .round() as i32;
            let closing = !want && !s.layer_w.running();
            let rtl = crate::layout::is_rtl(&me.inner);
            let x = if rtl { l.x() + l.w() - w } else { l.x() };
            let ring = crate::focus::show_ring(&me.inner);
            let sel = me.selected.get();
            paint_surface(
                &mut s,
                Surf::Layer,
                sel,
                (x, l.y(), w, l.h()),
                rtl,
                ring,
                true,
                l,
            );
            if closing {
                s.layer = None;
                drop(s);
                let l2 = l.clone();
                app::add_timeout3(0.0, move |_| crate::overlay::close_layer(&l2));
            }
        });
        let me = self.clone();
        layer.handle(move |l, ev| {
            let width = me.st.borrow_mut().layer_w.target().unwrap_or(0.0) as i32;
            let rtl = crate::layout::is_rtl(&me.inner);
            let inside_panel = {
                let ex = app::event_x();
                if rtl {
                    ex >= l.x() + l.w() - width
                } else {
                    ex < l.x() + width
                }
            };
            match ev {
                Event::Enter | Event::Move => {
                    let was = me.st.borrow().layer_hovered;
                    me.st.borrow_mut().layer_hovered = inside_panel;
                    if inside_panel {
                        me.track(Surf::Layer, l);
                    } else {
                        me.st.borrow_mut().hover = None;
                        me.set_tip(Surf::Layer, l, None);
                    }
                    if was && !inside_panel {
                        me.left();
                    }
                    l.redraw();
                    inside_panel
                }
                Event::Leave => {
                    me.st.borrow_mut().layer_hovered = false;
                    me.set_tip(Surf::Layer, l, None);
                    me.left();
                    true
                }
                Event::Push if inside_panel => {
                    me.click(Surf::Layer, l);
                    true
                }
                Event::MouseWheel if inside_panel => me.wheel(true),
                Event::DndEnter | Event::DndDrag if inside_panel => me.dnd(Surf::Layer, l, ev),
                Event::DndLeave | Event::DndRelease => me.dnd(Surf::Layer, l, ev),
                Event::Paste => me.paste(),
                _ => false,
            }
        });
    }

    fn install(&self) {
        let (st, sel, me) = (self.st.clone(), self.selected.clone(), self.clone());
        let mut inner = self.inner.clone();
        inner.draw(move |w| {
            let mut s = st.borrow_mut();
            let overlay_open = s.mode == SidebarMode::Overlay
                && s.layer.as_ref().is_some_and(|l| !l.was_deleted());
            let ring = crate::focus::show_ring(w) && !overlay_open;
            paint_surface(
                &mut s,
                Surf::Rail,
                sel.get(),
                (w.x(), w.y(), w.w(), w.h()),
                crate::layout::is_rtl(w),
                ring,
                !overlay_open,
                w,
            );
            let _ = &me;
        });
        let me = self.clone();
        inner.handle(move |w, ev| match ev {
            Event::Enter => {
                me.entered();
                me.track(Surf::Rail, w);
                true
            }
            Event::Leave => {
                me.left();
                me.set_tip(Surf::Rail, w, None);
                true
            }
            Event::Move => {
                me.track(Surf::Rail, w);
                true
            }
            Event::Push => {
                if app::event_mouse_button() != app::MouseButton::Right {
                    let _ = w.take_focus();
                }
                me.click(Surf::Rail, w);
                true
            }
            Event::MouseWheel => me.wheel(false),
            Event::Focus => {
                me.st.borrow_mut().open = true;
                me.apply();
                true
            }
            Event::Unfocus => {
                let hovered = {
                    let s = me.st.borrow();
                    s.hovered || s.layer_hovered
                };
                if !hovered {
                    me.collapse();
                } else {
                    w.redraw();
                }
                true
            }
            Event::KeyDown => me.key(w),
            Event::DndEnter | Event::DndDrag | Event::DndLeave | Event::DndRelease => {
                me.dnd(Surf::Rail, w, ev)
            }
            Event::Paste => me.paste(),
            _ => false,
        });
        let mut w2 = self.inner.clone();
        crate::state::bind_widget(&self.inner, &self.selected, false, move |_| w2.redraw());
        let (sel, ex, me) = (self.selected.clone(), self.expanded.clone(), self.clone());
        a11y::describe(&self.inner, move |n| {
            n.role(Role::TabList)
                .name(&crate::i18n::tr("Sidebar"))
                .expanded(ex.get())
                .focusable();
            let Ok(s) = me.st.try_borrow() else { return };
            let (_, (x, y, w, h)) = Self::visible_of(&s, &me.inner);
            let rtl = crate::layout::is_rtl(&me.inner);
            theme::with(|t| {
                let m = metrics(t);
                let reveal = reveal_of(s.rail, s.wide, w);
                let geo = geometry(&s, h, reveal, t);
                let scroll = s.scroll.clamp(0, geo.max_scroll());
                let mir = Mirror::new(rtl, x, w);
                let tabs = s.items.iter().filter(|it| !it.header).count();
                let mut pos = 0;
                for (i, it) in s.items.iter().enumerate() {
                    if !it.header {
                        pos += 1;
                    }
                    let Some((ry, rh)) = geo.at(i, scroll) else {
                        continue;
                    };
                    let shown = !geo.rows[i].is_some_and(|r| r.scrolls)
                        || (ry + rh > geo.view.0 && ry < geo.view.1);
                    n.child(i as u64, |c| {
                        if shown {
                            c.bounds(x, y + ry, w, rh);
                        }
                        if it.header {
                            c.role(Role::Heading).name(&it.label).level(2);
                            if it.collapsible {
                                let folded = s.collapsed.contains(&s.ids[i]);
                                c.expanded(!folded)
                                    .action(Action::Click)
                                    .action(if folded {
                                        Action::Expand
                                    } else {
                                        Action::Collapse
                                    })
                                    .action(Action::Focus)
                                    .focusable();
                                if s.cursor == i {
                                    c.focused();
                                }
                            }
                            return;
                        }
                        c.role(Role::Tab)
                            .name(&it.label)
                            .selected(sel.get() == i)
                            .position(pos, tabs)
                            .disabled(it.disabled);
                        if !it.disabled {
                            c.action(Action::Click).action(Action::Focus).focusable();
                        }
                        if s.on_context.is_some() && !it.disabled {
                            c.action(Action::ShowContextMenu);
                        }
                        let desc: Vec<&str> = [it.detail.as_str(), it.badge.as_str()]
                            .into_iter()
                            .filter(|d| !d.is_empty())
                            .collect();
                        if !desc.is_empty() {
                            c.description(&desc.join(", "));
                        }
                        if s.cursor == i {
                            c.focused();
                        }
                        if it.has_secondary() {
                            c.child(0, |b| {
                                b.role(Role::Button)
                                    .name(&it.secondary_tooltip)
                                    .disabled(it.busy || it.disabled);
                                if !it.busy && !it.disabled {
                                    b.action(Action::Click);
                                }
                                if shown && reveal >= 0.5 {
                                    let (bx, by, bs) = secondary_box(&m, t, x, w, y + ry, rh);
                                    b.bounds(mir.x(bx, bs), by, bs, bs);
                                }
                            });
                        }
                        if let Some(u) = it.usage {
                            c.child(1, |p| {
                                p.role(Role::ProgressIndicator)
                                    .name(if it.detail.is_empty() {
                                        &it.label
                                    } else {
                                        &it.detail
                                    })
                                    .numeric(u, 0.0, 1.0, 0.01);
                            });
                        }
                    });
                }
            });
        });
        let me = self.clone();
        a11y::on_action(&self.inner, move |r| {
            let Some(i) = r
                .path
                .first()
                .map(|k| *k as usize)
                .filter(|i| *i < me.st.borrow().items.len())
            else {
                return false;
            };
            let it = me.st.borrow().items[i].clone();
            if r.path.len() == 2 {
                if r.path[1] == 0 && r.action == Action::Click && it.has_secondary() {
                    me.secondary(i);
                    return true;
                }
                return false;
            }
            match r.action {
                Action::Click if it.header => {
                    let f = me.section_collapsed(i);
                    me.fold(i, !f, true);
                }
                Action::Expand | Action::Collapse if it.header => {
                    me.fold(i, r.action == Action::Collapse, true);
                }
                Action::Click => me.activate(i),
                Action::Focus => {
                    me.st.borrow_mut().cursor = i;
                    let _ = me.inner.clone().take_focus();
                    me.reveal_cursor();
                }
                Action::ShowContextMenu => {
                    let Some((x, y, _, rh)) = me.item_rect(i) else {
                        return false;
                    };
                    let (ox, oy) = window_origin(&me.inner);
                    return me.context(i, x + ox, y + rh + oy);
                }
                _ => return false,
            }
            true
        });
    }

    /// The item under the keyboard cursor.
    pub fn cursor(&self) -> usize {
        self.st.borrow().cursor
    }

    /// The overlay panel's current width (overlay mode; 0 when closed).
    pub fn overlay_width(&self) -> i32 {
        let s = self.st.borrow();
        match &s.layer {
            Some(l) if !l.was_deleted() => s.layer_w.target().map_or(0, |v| v as i32),
            _ => 0,
        }
    }
}
