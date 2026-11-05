//! Row geometry and painting of a `NavTree`. One layout function serves
//! painting and hit-testing, in left-to-right terms; `Mirror` flips it.

use super::flat::FRow;
use super::item::{Badge, Meta, NavItem, NavKind, RowPaint, RowState, RowStyle, Show};
use crate::anim;
use crate::layout::Mirror;
use crate::paint;
use crate::text::ColorRole;
use crate::theme::{Rgb, Theme};
use crate::widgets::glyph;
use fltk::enums::Align;
use std::ops::Range;
use std::rc::Rc;

pub(crate) struct Metrics {
    /// A one-line row, a section header, a separator.
    pub rh: i32,
    pub hh: i32,
    pub sep: i32,
    /// Title line and detail line.
    pub lh: i32,
    pub dh: i32,
    /// Usage bar and the gap above it.
    pub bar: i32,
    pub gap: i32,
    pub inset: i32,
    pub pad: i32,
    /// Action button (square) and the chevron cell.
    pub bs: i32,
    pub cs: i32,
    /// One indentation step.
    pub ind: i32,
    /// Card: vertical padding, gap between lines, the three line heights
    /// and the icon cell.
    pub cpad: i32,
    pub cgap: i32,
    pub l1: i32,
    pub l2: i32,
    pub l3: i32,
    pub ci: i32,
}

pub(crate) fn metrics(t: &Theme) -> Metrics {
    Metrics {
        rh: t.spacing.control_height + t.spacing.md,
        hh: t.font.caption + 2 * t.spacing.md + 2,
        sep: 2 * t.spacing.sm + 1,
        lh: t.font.body + 6,
        dh: t.font.caption + 4,
        bar: t.spacing.xs.max(2),
        gap: t.spacing.xs,
        inset: t.spacing.sm,
        pad: t.spacing.md,
        bs: t.spacing.control_height - 2 * t.spacing.xs,
        cs: t.font.body_small + 4,
        ind: (16.0 * t.font.base as f64 / 12.0).round() as i32,
        // 82 px with the default tokens, like o3's thread card.
        cpad: t.spacing.xxs + t.spacing.lg,
        cgap: t.spacing.xxs,
        l1: t.font.body + 8,
        l2: t.font.body + 8,
        l3: t.font.caption + 8,
        ci: t.font.body + 4,
    }
}

/// A card's height: two lines, three with meta or a detail.
pub(crate) fn card_h(it: &NavItem, m: &Metrics) -> i32 {
    let third = !it.meta.is_empty() || !it.detail.is_empty();
    2 * m.cpad + m.l1 + m.cgap + m.l2 + if third { m.cgap + m.l3 } else { 0 }
}

/// A custom row renderer: expanded height, and the painter.
pub(crate) struct Renderer {
    pub height: Box<dyn Fn(&NavItem) -> i32>,
    pub paint: Box<dyn Fn(&RowPaint)>,
}

/// (rail, expanded) height of an item.
pub(crate) fn heights(it: &NavItem, m: &Metrics, custom: Option<&Rc<Renderer>>) -> (i32, i32) {
    match it.kind {
        NavKind::Section => (m.hh, m.hh),
        NavKind::Separator => (m.sep, m.sep),
        NavKind::Row => {
            if let Some(r) = custom {
                return (m.rh, (r.height)(it).max(1));
            }
            if it.style == RowStyle::Card {
                return (m.rh, card_h(it, m));
            }
            let d = if it.detail.is_empty() { 0 } else { m.dh };
            let u = if it.usage.is_some() { m.gap + m.bar } else { 0 };
            (m.rh, m.rh + d + u)
        }
    }
}

/// What part of a row the pointer is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Part {
    Row,
    Action(usize),
    Chevron,
}

/// Geometry of one row, left to right (x spans are `(x, w)`).
#[derive(Default)]
pub(crate) struct Lay {
    pub icon: (i32, i32),
    /// Icon box (y, h): the row, or a card's first line.
    pub icon_v: (i32, i32),
    /// Icon glyph size.
    pub icon_size: i32,
    pub tx: i32,
    /// Right end of the title (a card: of the overline), and of the
    /// detail / usage lines.
    pub title_r: i32,
    pub line2_r: i32,
    /// Title line box (y, h); a card's first line.
    pub line: (i32, i32),
    pub chevron: Option<(i32, i32, i32, i32)>,
    pub spin: Option<(i32, i32, i32, i32)>,
    pub actions: Vec<(usize, (i32, i32, i32, i32))>,
    pub shortcut: Option<(i32, i32)>,
    pub badges: Vec<(usize, i32, i32)>,
    pub summary: Option<(i32, i32)>,
    pub count: Option<(i32, i32)>,
    /// The trailing text (and glyph) on the first line, when shown.
    pub trailing: Option<(i32, i32)>,
    pub card: Option<CardLay>,
}

/// The lines under a card's first line.
#[derive(Default)]
pub(crate) struct CardLay {
    /// Start of the title and meta lines.
    pub x: i32,
    pub title: (i32, i32),
    pub title_r: i32,
    pub note: Option<(i32, i32)>,
    pub meta: Option<(i32, i32)>,
    pub meta_r: i32,
}

fn badge_w(t: &Theme, m: &Metrics, b: &Badge) -> i32 {
    match b {
        Badge::Text { text, pill, .. } => {
            paint::measure(text, t.font.caption, false) + if *pill { 4 * t.spacing.xs } else { 0 }
        }
        Badge::Dot(_) => 7,
        Badge::Spinner => m.dh,
        Badge::Icon { .. } => t.font.body + 2,
    }
}

/// Width of the trailing glyph + text.
fn trailing_w(t: &Theme, it: &NavItem) -> i32 {
    let tw = if it.trailing.is_empty() {
        0
    } else {
        paint::measure(&it.trailing, t.font.caption, false)
    };
    let iw = if it.trailing_icon.is_empty() {
        0
    } else {
        t.font.caption + 4 + if tw > 0 { t.spacing.xs } else { 0 }
    };
    tw + iw
}

fn lerp(a: i32, b: i32, k: f64) -> i32 {
    (a as f64 + (b - a) as f64 * k.clamp(0.0, 1.0)).round() as i32
}

/// x of a card's icon center from the start of its indentation.
pub(crate) fn card_icon_center(t: &Theme, m: &Metrics, rail: i32, reveal: f64) -> i32 {
    lerp(rail / 2, m.inset + t.spacing.lg + m.ci / 2, reveal)
}

/// Lays out a row at `(x, ry, w, rh)`. `hot`: hover-only actions show.
#[allow(clippy::too_many_arguments)]
pub(crate) fn layout(
    t: &Theme,
    m: &Metrics,
    rail: i32,
    it: &NavItem,
    row: &FRow,
    (x, ry, w, rh): (i32, i32, i32, i32),
    reveal: f64,
    hot: bool,
) -> Lay {
    let mut l = Lay::default();
    let ind = (row.indent as f64 * m.ind as f64 * reveal).round() as i32;
    let section = it.kind == NavKind::Section;
    let card = it.style == RowStyle::Card && it.kind == NavKind::Row;
    l.icon = (x + ind, rail);
    l.icon_v = (ry, rh);
    l.icon_size = t.font.icon;
    l.tx = if section {
        x + m.inset + t.spacing.md + ind
    } else {
        x + ind + rail
    };
    l.line = if section {
        (ry, rh)
    } else {
        (ry + (m.rh - m.lh) / 2, m.lh)
    };
    let mut right = x + w - m.inset - t.spacing.xs;
    let mut mid = ry + rh / 2;
    let pad_x = m.inset + t.spacing.lg;
    if card {
        let x0 = x + ind + pad_x;
        let y1 = ry + m.cpad;
        l.line = (y1, m.l1);
        mid = y1 + m.l1 / 2;
        // From the rail cell (collapsed) to the card's own icon cell.
        let cx = x + ind + card_icon_center(t, m, rail, reveal);
        l.icon = (cx - rail / 2, rail);
        let cy = lerp(ry + rh / 2, mid, reveal);
        l.icon_v = (cy - m.l1 / 2, m.l1);
        if reveal >= 0.5 {
            l.icon_size = t.font.body + 2;
        }
        l.tx = x0 + m.ci + t.spacing.md;
        right = x + w - pad_x;
    }
    let mut hover_shown = false;
    if it.busy && !section {
        l.spin = Some((right - m.bs, mid - m.bs / 2, m.bs, m.bs));
        right -= m.bs + t.spacing.sm;
    } else {
        let shown: Vec<usize> = (0..it.actions.len())
            .filter(|&i| it.actions[i].show == Show::Always || hot)
            .collect();
        hover_shown = shown.iter().any(|&i| it.actions[i].show == Show::Hover);
        for &i in shown.iter().rev() {
            l.actions
                .push((i, (right - m.bs, mid - m.bs / 2, m.bs, m.bs)));
            right -= m.bs;
        }
        if !shown.is_empty() {
            right -= t.spacing.sm;
        }
    }
    if row.folds {
        let cx = if section && it.actions.is_empty() {
            x + w - m.inset - t.spacing.md - m.cs
        } else {
            right - m.cs
        };
        l.chevron = Some(if section {
            (cx, ry, m.cs, rh)
        } else if card {
            (cx, l.line.0, m.cs, l.line.1)
        } else {
            (cx, ry, m.cs, m.rh)
        });
        right = cx - t.spacing.sm;
    }
    if it.actions.is_empty() && !row.folds && !(it.busy && !section) && !card {
        // Nothing trailing: text ends where the rail's badge always did.
        right = x + w - m.inset - t.spacing.md;
    }
    let has_trailing = !it.trailing.is_empty() || !it.trailing_icon.is_empty();
    if has_trailing && !section && !hover_shown {
        let tw = trailing_w(t, it);
        l.trailing = Some((right - tw, tw));
        right -= tw + t.spacing.md;
    }
    l.line2_r = right;
    if section && !it.count.is_empty() {
        let cw = paint::measure(&it.count, t.font.caption, false);
        l.count = Some((right - cw, cw));
        right -= cw + t.spacing.md;
    }
    if !section {
        if !it.shortcut.is_empty() && !card {
            let sw = paint::measure(&it.shortcut, t.font.caption, false);
            l.shortcut = Some((right - sw, sw));
            right -= sw + t.spacing.md;
        }
        for (i, b) in it.badges.iter().enumerate().rev() {
            let bw = badge_w(t, m, b);
            l.badges.push((i, right - bw, bw));
            right -= bw + t.spacing.sm;
        }
        if !it.badges.is_empty() {
            right -= t.spacing.sm;
        }
        if row.folds && !row.open && !it.summary.is_empty() {
            let sw = paint::measure(&it.summary, t.font.caption, false);
            l.summary = Some((right - sw, sw));
            right -= sw + t.spacing.md;
        }
    }
    l.title_r = right;
    if card {
        let x0 = x + ind + pad_x;
        let end = x + w - pad_x;
        let mut c = CardLay {
            x: x0,
            title: (l.line.0 + m.l1 + m.cgap, m.l2),
            ..Default::default()
        };
        let mut r2 = end;
        if !it.note.is_empty() {
            let nw = paint::measure(&it.note, t.font.caption, false);
            c.note = Some((r2 - nw, nw));
            r2 -= nw + t.spacing.md;
        }
        if !it.shortcut.is_empty() {
            let sw = paint::measure(&it.shortcut, t.font.caption, false);
            l.shortcut = Some((r2 - sw, sw));
            r2 -= sw + t.spacing.md;
        }
        c.title_r = r2;
        if !it.meta.is_empty() || !it.detail.is_empty() {
            c.meta = Some((c.title.0 + m.l2 + m.cgap, m.l3));
        }
        c.meta_r = end;
        l.line2_r = end;
        l.card = Some(c);
    }
    l
}

/// Spans of a card's meta entries in `[x, end)`: (index, icon x, text x,
/// text w). The first entry with text takes the free width; entries
/// after it go to the end.
pub(crate) fn meta_spans(t: &Theme, metas: &[Meta], x: i32, end: i32) -> Vec<(usize, i32, i32, i32)> {
    let iw = t.font.caption + 4;
    let gap = t.spacing.xs;
    let width = |mm: &Meta| {
        let i = if mm.icon.is_empty() { 0 } else { iw };
        let tw = if mm.text.is_empty() {
            0
        } else {
            paint::measure(&mm.text, t.font.caption, false)
        };
        (i, tw)
    };
    let fill = metas.iter().position(|mm| !mm.text.is_empty());
    let mut out = Vec::new();
    // After the filling entry: from the end.
    let mut r = end;
    let split = fill.map_or(metas.len(), |f| f + 1);
    for (i, mm) in metas.iter().enumerate().skip(split).rev() {
        let (i_w, tw) = width(mm);
        let g = if i_w > 0 && tw > 0 { gap } else { 0 };
        let sx = r - (i_w + g + tw);
        out.push((i, sx, sx + i_w + g, tw));
        r = sx - t.spacing.md;
    }
    let mut cx = x;
    for (i, mm) in metas.iter().enumerate().take(split) {
        let (i_w, tw) = width(mm);
        let tx = cx + i_w + if i_w > 0 && tw > 0 { gap } else { 0 };
        let tw = if Some(i) == fill { (r - tx).max(0) } else { tw };
        out.push((i, cx, tx, tw));
        cx = tx + tw + t.spacing.md;
    }
    out.sort_by_key(|e| e.0);
    out
}

/// Cuts the middle of `s` with "…" to fit `max_w` (branch names keep
/// both ends; uses the current font).
fn elide_middle(s: &str, max_w: i32) -> String {
    if (fltk::draw::width(s).ceil() as i32) <= max_w {
        return s.to_string();
    }
    let chars: Vec<char> = s.chars().collect();
    let mut keep = chars.len();
    while keep > 0 {
        keep -= 1;
        let head = keep.div_ceil(2);
        let tail = keep / 2;
        let c: String = chars[..head]
            .iter()
            .chain(std::iter::once(&'…'))
            .chain(chars[chars.len() - tail..].iter())
            .collect();
        if (fltk::draw::width(&c).ceil() as i32) <= max_w {
            return c;
        }
    }
    String::new()
}

/// How a row looks now.
#[derive(Default, Clone, Copy)]
pub(crate) struct Look {
    pub selected: bool,
    pub hover: Option<Part>,
    pub ring: bool,
    pub drop: bool,
    /// Hover-only actions show (hovered, selected or cursor row).
    pub hot: bool,
    /// Tree guides: x of the line from the start of each indentation
    /// step (the parent's icon center); None draws none.
    pub guide: Option<i32>,
}

thread_local! {
    static EPOCH: std::time::Instant = std::time::Instant::now();
}

/// Spinner phase 0..1 (a turn per second); 0 with animations off.
pub(crate) fn spin_phase() -> f64 {
    if !anim::enabled() {
        return 0.0;
    }
    let e = EPOCH.with(|e| anim::now().saturating_duration_since(*e));
    e.as_secs_f64().fract()
}

/// Eight dots around `(cx, cy)`, the brightest at `phase`.
pub(crate) fn spinner(cx: i32, cy: i32, r: i32, color: Rgb, under: Rgb, phase: f64) {
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

/// The surface color of the sidebar.
pub(crate) fn surface(t: &Theme) -> Rgb {
    t.palette.foreground.over(t.palette.background, 0.04)
}

/// Paints one row; true when it drew a spinner (the caller schedules
/// frames while animations are on).
#[allow(clippy::too_many_arguments)]
pub(crate) fn paint_row(
    t: &Theme,
    m: &Metrics,
    rail: i32,
    it: &NavItem,
    row: &FRow,
    rect: (i32, i32, i32, i32),
    reveal: f64,
    look: Look,
    mir: &Mirror,
    bg: Rgb,
    phase: f64,
    hits: Option<&[Range<usize>]>,
    custom: Option<&Rc<Renderer>>,
) -> bool {
    let (x, ry, w, rh) = rect;
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
    let row_hover = look.hover.is_some();
    match it.kind {
        NavKind::Separator => {
            let (hx, hw) = (bx + t.spacing.sm, (bw - 2 * t.spacing.sm).max(0));
            paint::fill(mir.x(hx, hw), ry + rh / 2, hw, 1, 0, fg.over(bg, 0.12));
            return false;
        }
        NavKind::Section => {
            let hk = 1.0 - reveal;
            if hk > 0.0 {
                let (hx, hw) = (bx + t.spacing.sm, (bw - 2 * t.spacing.sm).max(0));
                paint::fill(mir.x(hx, hw), ry + rh / 2, hw, 1, 0, fg.over(bg, 0.12 * hk));
            }
            if reveal <= 0.0 {
                return false;
            }
            let mut under = bg;
            if row_hover && row.folds {
                under = t.fill(&t.controls.hover, bg);
                crate::glass::fill(mir.x(bx, bw), ry + 1, bw, rh - 2, t.radius, under);
            }
            if look.ring {
                ring(under);
            }
            let l = layout(t, m, rail, it, row, rect, reveal, look.hot);
            let ink = fg.darker(1.4).over(under, reveal);
            paint_trailing(t, m, it, row, &l, look, mir, under, reveal, phase);
            if let Some((cx, cw)) = l.count {
                paint::text(
                    &it.count,
                    mir.x(cx, cw),
                    ry,
                    cw,
                    rh,
                    t.font.caption,
                    false,
                    ink,
                    mir.align(Align::Right),
                );
            }
            let lw = (l.title_r - l.tx).max(0);
            paint::text_elided(
                &it.title,
                mir.x(l.tx, lw),
                ry,
                lw,
                rh,
                t.font.caption,
                true,
                ink,
                mir.align(Align::Left),
            );
            return false;
        }
        NavKind::Row => {}
    }
    let mut under = bg;
    if look.selected {
        under = t.fill(&t.controls.selected, bg);
        crate::glass::fill(mir.x(bx, bw), ry + 2, bw, rh - 4, t.radius, under);
    } else if (row_hover && !it.disabled) || look.drop {
        under = t.fill(&t.controls.hover, bg);
        crate::glass::fill(mir.x(bx, bw), ry + 2, bw, rh - 4, t.radius, under);
    }
    if it.active {
        paint::fill(
            mir.x(x + 2, 3),
            ry + 6,
            3,
            (rh - 12).max(4),
            1,
            t.palette.accent,
        );
    }
    if look.drop {
        paint::stroke(
            mir.x(bx, bw),
            ry + 2,
            bw,
            rh - 4,
            t.radius,
            2,
            t.palette.accent,
        );
    }
    if look.ring {
        ring(under);
    }
    if let Some(off) = look.guide.filter(|_| row.indent > 0 && reveal > 0.0) {
        let c = fg.over(under, 0.14 * reveal);
        for lvl in 0..row.indent as i32 {
            let gx = x + (lvl as f64 * m.ind as f64 * reveal).round() as i32 + off;
            paint::fill(mir.x(gx, 1), ry, 1, rh, 0, c);
        }
    }
    if let Some(r) = custom {
        let state = RowState {
            selected: look.selected,
            active: it.active,
            hover: row_hover,
            focused: look.ring,
            drop_target: look.drop,
            depth: row.depth as usize,
            expanded: row.folds.then_some(row.open),
            reveal,
            dimmed: it.dimmed,
            disabled: it.disabled,
        };
        (r.paint)(&RowPaint {
            item: it,
            rect,
            state,
            theme: t,
            under,
            rtl: mir.on,
        });
        return false;
    }
    let base = if it.disabled {
        fg.over(under, 0.38)
    } else if it.dimmed {
        fg.over(under, 0.55)
    } else {
        fg
    };
    let ink = if it.active && !it.disabled {
        t.palette.accent
    } else {
        it.icon_role.map_or(base, |r| r.rgb(t))
    };
    // Rows with a trailing text show their hover actions in its place
    // only while hovered or under the keyboard cursor (o3).
    let hot = if it.trailing.is_empty() && it.trailing_icon.is_empty() {
        look.hot
    } else {
        look.hover.is_some() || look.ring
    };
    let l = layout(t, m, rail, it, row, rect, reveal, hot);
    let mut spun = false;
    let (ix, iw) = l.icon;
    let (iy, ih) = l.icon_v;
    let isz = l.icon_size;
    if it.busy && reveal < 0.5 {
        spinner(
            mir.x(ix, iw) + iw / 2,
            iy + ih / 2,
            isz / 2,
            ink,
            under,
            phase,
        );
        spun = true;
    } else {
        paint::glyph(&it.icon, mir.x(ix, iw), iy, iw, ih, isz, ink);
    }
    if it.attention {
        let d = 7;
        let cx = ix + iw / 2 + isz / 2 - 1;
        paint::fill(
            mir.x(cx - d / 2, d),
            iy + ih / 2 - isz / 2 - 1,
            d,
            d,
            d / 2,
            t.palette.accent,
        );
    }
    if reveal <= 0.0 {
        return spun;
    }
    let k = reveal;
    spun |= paint_trailing(t, m, it, row, &l, look, mir, under, k, phase);
    let (ly, lh) = l.line;
    let dim = base.darker(1.4).over(under, k);
    if let Some((tx, tw)) = l.trailing {
        let c = it.trailing_role.map_or(dim, |r| r.rgb(t).over(under, k));
        let mut cx = tx;
        if !it.trailing_icon.is_empty() {
            let gw = t.font.caption + 4;
            paint::glyph(&it.trailing_icon, mir.x(cx, gw), ly, gw, lh, t.font.caption + 2, c);
            cx += gw + t.spacing.xs;
        }
        let rest = (tx + tw - cx).max(0);
        if !it.trailing.is_empty() && rest > 0 {
            paint::text(
                &it.trailing,
                mir.x(cx, rest),
                ly,
                rest,
                lh,
                t.font.caption,
                false,
                c,
                mir.align(Align::Right),
            );
        }
    }
    let (shy, shh) = l
        .card
        .as_ref()
        .map_or((ly, lh), |c| c.title);
    if let Some((sx, sw)) = l.shortcut {
        paint::text(
            &it.shortcut,
            mir.x(sx, sw),
            shy,
            sw,
            shh,
            t.font.caption,
            false,
            base.darker(1.5).over(under, k),
            mir.align(Align::Right),
        );
    }
    for &(i, bx2, bw2) in &l.badges {
        spun |= paint_badge(
            t,
            m,
            &it.badges[i],
            mir.x(bx2, bw2),
            ly,
            bw2,
            lh,
            under,
            k,
            phase,
        );
    }
    if let Some((sx, sw)) = l.summary {
        paint::text(
            &it.summary,
            mir.x(sx, sw),
            ly,
            sw,
            lh,
            t.font.caption,
            false,
            base.darker(1.4).over(under, k),
            mir.align(Align::Right),
        );
    }
    let bold = look.selected || it.attention;
    let text = if it.disabled || it.dimmed { base } else { fg };
    if let Some(c) = &l.card {
        let ow = (l.title_r - l.tx).max(0);
        paint::text_elided(
            &it.overline,
            mir.x(l.tx, ow),
            ly,
            ow,
            lh,
            t.font.caption,
            false,
            dim,
            mir.align(Align::Left),
        );
        let (ty, th) = c.title;
        paint_title(t, it, hits, (c.x, ty, (c.title_r - c.x).max(0), th), bold, text.over(under, k), under, k, mir);
        if let Some((nx, nw)) = c.note {
            paint::text(&it.note, mir.x(nx, nw), ty, nw, th, t.font.caption, false, dim, mir.align(Align::Right));
        }
        if let Some((my, mh)) = c.meta {
            let faint = base.over(under, 0.4 * k);
            if it.meta.is_empty() {
                let w3 = (c.meta_r - c.x).max(0);
                paint::text_elided(&it.detail, mir.x(c.x, w3), my, w3, mh, t.font.caption, false, dim, mir.align(Align::Left));
            }
            for (i, gx, tx, tw) in meta_spans(t, &it.meta, c.x, c.meta_r) {
                let mm = &it.meta[i];
                let col = mm.role.map_or(faint, |r| r.rgb(t).over(under, k));
                if !mm.icon.is_empty() {
                    let gw = t.font.caption + 4;
                    paint::glyph(&mm.icon, mir.x(gx, gw), my, gw, mh, t.font.caption + 1, col);
                }
                if tw > 0 && !mm.text.is_empty() {
                    fltk::draw::set_font(paint::font(false), t.font.caption);
                    let shown = elide_middle(&mm.text, tw);
                    paint::text(&shown, mir.x(tx, tw), my, tw, mh, t.font.caption, false, col, mir.align(Align::Left));
                }
            }
        }
        return spun;
    }
    let tw = (l.title_r - l.tx).max(0);
    paint_title(t, it, hits, (l.tx, ly, tw, lh), bold, text.over(under, k), under, k, mir);
    let mut y2 = ly + lh;
    let w2 = (l.line2_r - l.tx).max(0);
    if !it.detail.is_empty() {
        paint::text_elided(
            &it.detail,
            mir.x(l.tx, w2),
            y2,
            w2,
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
        paint::fill(
            mir.x(l.tx, w2),
            by,
            w2,
            m.bar,
            m.bar / 2,
            fg.over(under, 0.15 * k),
        );
        let fw = (w2 as f64 * u).round() as i32;
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
        paint::fill(mir.x(l.tx, fw), by, fw, m.bar, m.bar / 2, c.over(under, k));
    }
    spun
}

/// The title in `(x, y, w, h)` with the search hits highlighted.
#[allow(clippy::too_many_arguments)]
fn paint_title(
    t: &Theme,
    it: &NavItem,
    hits: Option<&[Range<usize>]>,
    (tx, ly, tw, lh): (i32, i32, i32, i32),
    bold: bool,
    color: Rgb,
    under: Rgb,
    k: f64,
    mir: &Mirror,
) {
    if let Some(hits) = hits.filter(|_| tw > 0) {
        fltk::draw::push_clip(mir.x(tx, tw), ly, tw, lh);
        for r in hits {
            if r.end > it.title.len()
                || !it.title.is_char_boundary(r.start)
                || !it.title.is_char_boundary(r.end)
            {
                continue;
            }
            let hx = tx + paint::measure(&it.title[..r.start], t.font.body, bold);
            let hw = paint::measure(&it.title[r.start..r.end], t.font.body, bold);
            paint::fill(
                mir.x(hx - 1, hw + 2),
                ly + 2,
                hw + 2,
                lh - 4,
                2,
                t.palette.accent.over(under, 0.3 * k),
            );
        }
        fltk::draw::pop_clip();
    }
    paint::text_elided(
        &it.title,
        mir.x(tx, tw),
        ly,
        tw,
        lh,
        t.font.body,
        bold,
        color,
        mir.align(Align::Left),
    );
}

/// Actions, chevron and the busy spinner at the end of a row.
#[allow(clippy::too_many_arguments)]
fn paint_trailing(
    t: &Theme,
    _m: &Metrics,
    it: &NavItem,
    row: &FRow,
    l: &Lay,
    look: Look,
    mir: &Mirror,
    under: Rgb,
    k: f64,
    phase: f64,
) -> bool {
    let fg = t.palette.foreground;
    let base = if it.disabled {
        fg.over(under, 0.38)
    } else if it.dimmed {
        fg.over(under, 0.55)
    } else {
        fg
    };
    let mut spun = false;
    if let Some((sx, sy, ss, _)) = l.spin {
        spinner(
            mir.x(sx, ss) + ss / 2,
            sy + ss / 2,
            ss / 3,
            base,
            under,
            phase,
        );
        spun = true;
    }
    for &(i, (ax, ay, aw, ah)) in &l.actions {
        let a = &it.actions[i];
        let on = a.enabled && !it.disabled;
        if on && look.hover == Some(Part::Action(i)) {
            let hot = t.fill(&t.controls.hover, under);
            crate::glass::fill(mir.x(ax, aw), ay, aw, ah, t.radius, hot.over(under, k));
        }
        let c = if on { base } else { fg.over(under, 0.3) };
        paint::glyph(
            &a.icon,
            mir.x(ax, aw),
            ay,
            aw,
            ah,
            t.font.body + 2,
            c.over(under, k),
        );
    }
    if let Some((cx, cy, cw, ch)) = l.chevron {
        let g = if row.open {
            glyph::CHEVRON_DOWN
        } else {
            mir.glyph(glyph::CHEVRON_RIGHT, glyph::CHEVRON_LEFT)
        };
        let ink = if look.hover == Some(Part::Chevron) {
            fg
        } else {
            fg.darker(1.4)
        };
        paint::glyph(
            g,
            mir.x(cx, cw),
            cy,
            cw,
            ch,
            t.font.body_small,
            ink.over(under, k),
        );
    }
    spun
}

#[allow(clippy::too_many_arguments)]
fn paint_badge(
    t: &Theme,
    m: &Metrics,
    b: &Badge,
    x: i32,
    ly: i32,
    w: i32,
    lh: i32,
    under: Rgb,
    k: f64,
    phase: f64,
) -> bool {
    match b {
        Badge::Text { text, role, pill } => {
            let c = role.rgb(t);
            if *pill {
                let ph = m.dh;
                paint::fill(
                    x,
                    ly + (lh - ph) / 2,
                    w,
                    ph,
                    ph / 2,
                    c.over(under, 0.18 * k),
                );
            }
            paint::text(
                text,
                x,
                ly,
                w,
                lh,
                t.font.caption,
                false,
                c.over(under, k),
                Align::Center,
            );
            false
        }
        Badge::Dot(role) => {
            let d = 7;
            paint::fill(
                x,
                ly + (lh - d) / 2,
                d,
                d,
                d / 2,
                role.rgb(t).over(under, k),
            );
            false
        }
        Badge::Icon { glyph, role, .. } => {
            paint::glyph(glyph, x, ly, w, lh, t.font.body, role.rgb(t).over(under, k));
            false
        }
        Badge::Spinner => {
            spinner(
                x + w / 2,
                ly + lh / 2,
                w / 2,
                ColorRole::Dim.rgb(t).over(under, k),
                under,
                phase,
            );
            true
        }
    }
}
