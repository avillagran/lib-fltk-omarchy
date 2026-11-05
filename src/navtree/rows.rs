//! Row geometry and painting of a `NavTree`. One layout function serves
//! painting and hit-testing, in left-to-right terms; `Mirror` flips it.

use super::flat::FRow;
use super::item::{Badge, NavItem, NavKind, RowPaint, RowState, Show};
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
    }
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
    pub tx: i32,
    /// Right end of the title, and of the detail / usage lines.
    pub title_r: i32,
    pub line2_r: i32,
    /// Title line box (y, h).
    pub line: (i32, i32),
    pub chevron: Option<(i32, i32, i32, i32)>,
    pub spin: Option<(i32, i32, i32, i32)>,
    pub actions: Vec<(usize, (i32, i32, i32, i32))>,
    pub shortcut: Option<(i32, i32)>,
    pub badges: Vec<(usize, i32, i32)>,
    pub summary: Option<(i32, i32)>,
    pub count: Option<(i32, i32)>,
}

fn badge_w(t: &Theme, m: &Metrics, b: &Badge) -> i32 {
    match b {
        Badge::Text { text, pill, .. } => {
            paint::measure(text, t.font.caption, false) + if *pill { 4 * t.spacing.xs } else { 0 }
        }
        Badge::Dot(_) => 7,
        Badge::Spinner => m.dh,
    }
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
    l.icon = (x + ind, rail);
    let section = it.kind == NavKind::Section;
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
    let mid = ry + rh / 2;
    if it.busy && !section {
        l.spin = Some((right - m.bs, mid - m.bs / 2, m.bs, m.bs));
        right -= m.bs + t.spacing.sm;
    } else {
        let shown: Vec<usize> = (0..it.actions.len())
            .filter(|&i| it.actions[i].show == Show::Always || hot)
            .collect();
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
        l.chevron = Some((cx, ry, m.cs, if section { rh } else { m.rh }));
        right = cx - t.spacing.sm;
    }
    if it.actions.is_empty() && !row.folds && !(it.busy && !section) {
        // Nothing trailing: text ends where the rail's badge always did.
        right = x + w - m.inset - t.spacing.md;
    }
    l.line2_r = right;
    if section && !it.count.is_empty() {
        let cw = paint::measure(&it.count, t.font.caption, false);
        l.count = Some((right - cw, cw));
        right -= cw + t.spacing.md;
    }
    if !section {
        if !it.shortcut.is_empty() {
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
    l
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
    let l = layout(t, m, rail, it, row, rect, reveal, look.hot);
    let mut spun = false;
    let (ix, iw) = l.icon;
    if it.busy && reveal < 0.5 {
        spinner(
            mir.x(ix, iw) + iw / 2,
            ry + rh / 2,
            t.font.icon / 2,
            ink,
            under,
            phase,
        );
        spun = true;
    } else {
        paint::glyph(&it.icon, mir.x(ix, iw), ry, iw, rh, t.font.icon, ink);
    }
    if it.attention {
        let d = 7;
        let cx = ix + iw / 2 + t.font.icon / 2 - 1;
        paint::fill(
            mir.x(cx - d / 2, d),
            ry + rh / 2 - t.font.icon / 2 - 1,
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
    if let Some((sx, sw)) = l.shortcut {
        paint::text(
            &it.shortcut,
            mir.x(sx, sw),
            ly,
            sw,
            lh,
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
    let tw = (l.title_r - l.tx).max(0);
    let bold = look.selected || it.attention;
    let text = if it.disabled || it.dimmed { base } else { fg };
    if let Some(hits) = hits.filter(|_| tw > 0) {
        fltk::draw::push_clip(mir.x(l.tx, tw), ly, tw, lh);
        for r in hits {
            if r.end > it.title.len()
                || !it.title.is_char_boundary(r.start)
                || !it.title.is_char_boundary(r.end)
            {
                continue;
            }
            let hx = l.tx + paint::measure(&it.title[..r.start], t.font.body, bold);
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
        mir.x(l.tx, tw),
        ly,
        tw,
        lh,
        t.font.body,
        bold,
        text.over(under, k),
        mir.align(Align::Left),
    );
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
