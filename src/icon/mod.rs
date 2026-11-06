//! Vector icons from SVG sources (Lucide and similar sets), tinted with a
//! theme color, crisp at any size and device pixel ratio. No extra
//! dependencies and no crate feature: the SVG subset is parsed into
//! [`gfx::Path`](crate::gfx::Path)s and rasterized by the toolkit's own
//! anti-aliased renderer. Nerd Font glyphs (`widgets::glyph`) keep working
//! next to them.
//!
//! ```no_run
//! use fltk_omarchy::icon::{self, Icon, Svg};
//! use fltk_omarchy::text::ColorRole;
//! let rocket = Svg::parse(r#"<svg viewBox="0 0 24 24" fill="none" stroke="currentColor"
//!     stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
//!     <path d="M4.5 16.5c-1.5 1.26-2 5-2 5s3.74-.5 5-2c.71-.84.7-2.13-.09-2.91a2.18 2.18 0 0 0-2.91-.09z"/></svg>"#)
//!     .expect("valid icon");
//! // Decorative (no accessible name), dim like secondary text:
//! let _i = Icon::new(&rocket, 16).tint(ColorRole::Dim);
//! // A button: Tab stop, Enter/Space/click, named by its tooltip.
//! let _b = Icon::new(&rocket, 16).tooltip("Launch").on_click(|| println!("go"));
//! // Inside any draw callback (`w` is the widget being drawn):
//! // icon::draw(&w, &rocket, x, y, 16, 16, theme::with(|t| t.palette.accent));
//! ```
//!
//! Rasters are cached per (icon, device size, color) in a small LRU
//! ([`set_cache_budget`], 4 MiB by default), so a list of rows with icons
//! rasterizes each icon once. Colors are resolved when drawn: a [`Tint`]
//! by role follows `omarchy-theme-set` live. Supported SVG: see
//! [`svg`](self::svg) (paths with every command, circles, ellipses, rounded
//! rects, lines, polylines, polygons, strokes with caps and joins,
//! `fill-rule`, transforms, `currentColor`). Guide: docs/MEDIA.md.

pub mod svg;

use crate::a11y::{self, Action, Role};
use crate::gfx::{BlendMode, Canvas, Color};
use crate::paint;
use crate::text::ColorRole;
use crate::theme::{self, Rgb, Theme};
use fltk::{enums::*, prelude::*, widget::Widget};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

pub use svg::{Ink, Shape, SvgError};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// A parsed icon. Cheap to clone (shared), `Send + Sync`: parse a large
/// set on a worker and hand the icons to the UI thread.
#[derive(Debug, Clone)]
pub struct Svg {
    id: u64,
    inner: Arc<svg::Parsed>,
}

impl PartialEq for Svg {
    fn eq(&self, other: &Svg) -> bool {
        self.id == other.id
    }
}

impl Svg {
    /// Parses an SVG document (the subset in [`svg`](self::svg)). Errors
    /// are values; nothing panics on bad input.
    pub fn parse(src: &str) -> Result<Svg, SvgError> {
        let inner = svg::parse(src)?;
        Ok(Svg { id: NEXT_ID.fetch_add(1, Ordering::Relaxed), inner: Arc::new(inner) })
    }

    /// Parses a `'static` source once per thread and returns the same icon
    /// afterwards (built-in icons, `include_str!` assets).
    pub fn cached(src: &'static str) -> Result<Svg, SvgError> {
        thread_local! {
            static PARSED: RefCell<HashMap<usize, Result<Svg, SvgError>>> = RefCell::new(HashMap::new());
        }
        let key = src.as_ptr() as usize ^ (src.len() << 48);
        PARSED.with(|p| p.borrow_mut().entry(key).or_insert_with(|| Svg::parse(src)).clone())
    }

    /// `(x, y, w, h)` of the viewBox (24 × 24 for Lucide).
    pub fn view_box(&self) -> (f32, f32, f32, f32) {
        self.inner.view_box
    }

    pub fn shapes(&self) -> &[Shape] {
        &self.inner.shapes
    }

    /// A stable id for caches (equal for clones of the same parse).
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Draws the icon into a [`Canvas`] (a `gfx::View` paint callback),
    /// fitted into `(x, y, w, h)` and centered (SVG `xMidYMid meet`), with
    /// `tint` for `currentColor`. A translucent tint applies to the icon as
    /// a whole: overlapping strokes do not add up.
    pub fn paint(&self, c: &mut Canvas, x: f32, y: f32, w: f32, h: f32, tint: Color) {
        if tint.a < 1.0 {
            let opaque = tint.with_alpha(1.0);
            c.layer(tint.a.clamp(0.0, 1.0), BlendMode::Normal, |c| self.paint_opaque(c, x, y, w, h, opaque));
        } else {
            self.paint_opaque(c, x, y, w, h, tint);
        }
    }

    fn paint_opaque(&self, c: &mut Canvas, x: f32, y: f32, w: f32, h: f32, tint: Color) {
        let (vx, vy, vw, vh) = self.inner.view_box;
        let s = (w / vw).min(h / vh);
        if !(s.is_finite() && s > 0.0) {
            return;
        }
        let ox = x + (w - vw * s) / 2.0 - vx * s;
        let oy = y + (h - vh * s) / 2.0 - vy * s;
        for sh in &self.inner.shapes {
            let p = sh.path.scale(s, s).translate(ox, oy);
            if let Some(ink) = sh.fill {
                let col = ink.resolve(tint);
                if sh.even_odd { c.fill_even_odd(&p, col) } else { c.fill(&p, col) }
            }
            if let Some((st, ink)) = sh.stroke {
                let st = crate::gfx::Stroke { width: st.width * s, ..st };
                c.stroke(&p, &st, ink.resolve(tint));
            }
        }
    }

    /// Straight RGBA8 pixels (`w × h × 4`) of the icon fitted into a
    /// `w × h` box with `tint` for `currentColor`.
    pub fn rasterize(&self, w: usize, h: usize, tint: Color) -> Vec<u8> {
        let mut c = Canvas::new(w, h);
        self.paint(&mut c, 0.0, 0.0, w as f32, h as f32, tint);
        let mut out = Vec::new();
        c.to_rgba8(&mut out);
        out
    }
}

// ================================================================ colors

/// The color of an icon: a theme role (restyles with `omarchy-theme-set`)
/// or a fixed color from app data (a project's color, already taken from
/// the theme palette).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tint {
    Role(ColorRole),
    Fixed(Rgb),
}

impl From<ColorRole> for Tint {
    fn from(r: ColorRole) -> Tint {
        Tint::Role(r)
    }
}

impl From<Rgb> for Tint {
    fn from(c: Rgb) -> Tint {
        Tint::Fixed(c)
    }
}

impl Tint {
    pub fn rgb(self, t: &Theme) -> Rgb {
        match self {
            Tint::Role(r) => r.rgb(t),
            Tint::Fixed(c) => c,
        }
    }
}

// ================================================================= cache

type Key = (u64, u32, u32, [u8; 4]);

struct Raster {
    img: fltk::image::RgbImage,
    bytes: usize,
    used: u64,
}

#[derive(Default)]
struct Cache {
    map: HashMap<Key, Raster>,
    bytes: usize,
    tick: u64,
    budget: Option<usize>,
    hits: u64,
    misses: u64,
}

thread_local! {
    static CACHE: RefCell<Cache> = RefCell::new(Cache::default());
}

const DEFAULT_BUDGET: usize = 4 << 20;
/// Largest icon raster side, in device pixels.
const MAX_SIDE: u32 = 1024;

/// Bytes the icon raster cache may hold (default 4 MiB, ~1 000 icons of
/// 32 × 32 device pixels). 0 disables caching.
pub fn set_cache_budget(bytes: usize) {
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        c.budget = Some(bytes);
        evict(&mut c, 0);
    });
}

/// Icon raster cache use: `(entries, bytes, hits, misses)`.
pub fn cache_stats() -> (usize, usize, u64, u64) {
    CACHE.with(|c| {
        let c = c.borrow();
        (c.map.len(), c.bytes, c.hits, c.misses)
    })
}

fn evict(c: &mut Cache, room: usize) {
    let budget = c.budget.unwrap_or(DEFAULT_BUDGET);
    while c.bytes + room > budget && !c.map.is_empty() {
        // Least recently used; O(n) over a few hundred entries at most.
        let Some((&k, _)) = c.map.iter().min_by_key(|(_, r)| r.used) else { break };
        if let Some(r) = c.map.remove(&k) {
            c.bytes -= r.bytes;
        }
    }
}

/// Draws `svg` fitted into `(x, y, w, h)` (FLTK units) in `color`, at the
/// device resolution of `widget`'s window. Call it inside a draw callback
/// (`widget` is the one being drawn). Use [`draw_with_alpha`] for a
/// translucent color.
pub fn draw<W: WidgetExt>(widget: &W, svg: &Svg, x: i32, y: i32, w: i32, h: i32, color: Rgb) {
    draw_with_alpha(widget, svg, x, y, w, h, color.into());
}

/// [`draw`] with a straight-alpha color.
pub fn draw_with_alpha<W: WidgetExt>(widget: &W, svg: &Svg, x: i32, y: i32, w: i32, h: i32, color: Color) {
    if w <= 0 || h <= 0 {
        return;
    }
    let ppu = crate::native::pixels_per_unit(widget).clamp(0.25, 8.0);
    draw_at_scale(svg, x, y, w, h, color, ppu);
}

/// [`draw`] at an explicit device scale (`ppu` device pixels per unit).
pub fn draw_at_scale(svg: &Svg, x: i32, y: i32, w: i32, h: i32, color: Color, ppu: f32) {
    if w <= 0 || h <= 0 {
        return;
    }
    let dw = ((w as f32 * ppu).round() as u32).clamp(1, MAX_SIDE);
    let dh = ((h as f32 * ppu).round() as u32).clamp(1, MAX_SIDE);
    let key: Key = (svg.id, dw, dh, color.to_u8());
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        c.tick += 1;
        let tick = c.tick;
        if let Some(r) = c.map.get_mut(&key) {
            r.used = tick;
            let img = &mut r.img;
            img.scale(w, h, false, true);
            img.draw(x, y, w, h);
            c.hits += 1;
            return;
        }
        c.misses += 1;
        let rgba = svg.rasterize(dw as usize, dh as usize, color);
        let Ok(mut img) = fltk::image::RgbImage::new(&rgba, dw as i32, dh as i32, ColorDepth::Rgba8) else { return };
        img.scale(w, h, false, true);
        img.draw(x, y, w, h);
        let bytes = rgba.len() + 64;
        let budget = c.budget.unwrap_or(DEFAULT_BUDGET);
        if bytes <= budget {
            evict(&mut c, bytes);
            c.bytes += bytes;
            c.map.insert(key, Raster { img, bytes, used: tick });
        }
    });
}

// ============================================================ Icon widget

struct IconData {
    svg: Option<Svg>,
    tint: Tint,
    size: i32,
    name: String,
    button: bool,
}

/// An icon as a widget, for rows, headers and toolbars. Decorative (hidden
/// from screen readers) unless it gets a [`label`](Icon::label) or a
/// [`tooltip`](Icon::tooltip); with [`on_click`](Icon::on_click) it is a
/// button like `ActionButton` (Tab stop, Enter/Space/click, hover and
/// focus look, accessible button named by its tooltip).
#[derive(Clone)]
pub struct Icon {
    inner: Widget,
    data: Rc<RefCell<IconData>>,
    click: Rc<RefCell<Option<Box<dyn FnMut()>>>>,
    tracked: Rc<Cell<bool>>,
    st: Rc<RefCell<paint::Interaction>>,
}
fltk::widget_extends!(Icon, Widget, inner);

impl Icon {
    /// `size`: the icon's side in FLTK units (the widget is that square).
    pub fn new(svg: &Svg, size: i32) -> Icon {
        let mut inner = Widget::default();
        inner.set_frame(FrameType::NoBox);
        inner.clear_visible_focus();
        let size = size.max(1);
        inner.set_size(size, size);
        let data = Rc::new(RefCell::new(IconData { svg: Some(svg.clone()), tint: Tint::Role(ColorRole::Foreground), size, name: String::new(), button: false }));
        let st = Rc::new(RefCell::new(paint::Interaction::default()));
        let (d, s2) = (data.clone(), st.clone());
        let mut fade = crate::anim::Anim::new();
        inner.draw(move |w| {
            let d = d.borrow();
            theme::with(|t| {
                let mut color = d.tint.rgb(t);
                if d.button {
                    let s = crate::widgets::interaction_of(w, &s2);
                    let under = t.palette.background;
                    let fg = t.palette.foreground;
                    let hot = (s.hover || s.pressed) && !s.disabled;
                    let fill = if s.focus {
                        Some(fg.over(under, t.controls.focus.fill_alpha))
                    } else if hot {
                        Some(fg.over(under, if s.pressed { t.controls.pressed_fill_alpha } else { t.controls.hover.fill_alpha }))
                    } else {
                        None
                    };
                    let border = s.focus.then(|| (t.border(&t.controls.focus, under), t.controls.focus.border_width));
                    let f = fill.unwrap_or(under);
                    let target = match border {
                        Some((c, bw)) => paint::Look { fill: f, border: c, border_width: bw },
                        None => paint::Look { fill: f, border: f, border_width: 0 },
                    };
                    let look = fade.get(target, 60, crate::anim::Ease::Linear, w);
                    paint::draw_look(w.x(), w.y(), w.w(), w.h(), t.radius, look, under);
                    if s.disabled {
                        color = color.darker(2.0);
                    } else if hot && matches!(d.tint, Tint::Role(ColorRole::Dim | ColorRole::Faint)) {
                        color = fg;
                    }
                }
                if let Some(svg) = &d.svg {
                    let sz = d.size.min(w.w()).min(w.h());
                    draw(w, svg, w.x() + (w.w() - sz) / 2, w.y() + (w.h() - sz) / 2, sz, sz, color);
                }
            });
        });
        let d = data.clone();
        a11y::describe(&inner, move |n| {
            let d = d.borrow();
            if d.button {
                n.role(Role::Button).focusable().action(Action::Click);
                if !d.name.is_empty() {
                    n.name(&d.name);
                }
            } else if d.name.is_empty() {
                n.role(Role::Unknown).leaf(); // decorative: not in the tree
            } else {
                n.role(Role::Image).name(&d.name);
            }
        });
        Icon { inner, data, click: Rc::new(RefCell::new(None)), tracked: Rc::new(Cell::new(false)), st }
    }

    /// The icon's color (a [`ColorRole`] or [`Tint::Fixed`]).
    pub fn tint(self, tint: impl Into<Tint>) -> Self {
        self.data.borrow_mut().tint = tint.into();
        self.inner.clone().redraw();
        self
    }

    pub fn set_tint(&mut self, tint: impl Into<Tint>) {
        self.data.borrow_mut().tint = tint.into();
        self.inner.redraw();
    }

    /// Replaces the icon (None draws nothing).
    pub fn set_svg(&mut self, svg: Option<&Svg>) {
        self.data.borrow_mut().svg = svg.cloned();
        self.inner.redraw();
    }

    /// The name screen readers read; a named icon is not decorative.
    pub fn label(self, name: &str) -> Self {
        self.data.borrow_mut().name = name.to_string();
        self
    }

    /// Tooltip (Omarchy look) that also names the icon.
    pub fn tooltip(mut self, text: &str) -> Self {
        self.inner.set_tooltip(text);
        self.data.borrow_mut().name = text.to_string();
        self
    }

    /// Makes the icon a button: a Tab stop that runs `f` on click, Enter
    /// or Space, with the ActionButton look (at least 22 units square).
    pub fn on_click(mut self, f: impl FnMut() + 'static) -> Self {
        *self.click.borrow_mut() = Some(Box::new(f));
        self.data.borrow_mut().button = true;
        if !self.tracked.replace(true) {
            let c = self.click.clone();
            let fire: Rc<dyn Fn()> = Rc::new(move || {
                let f = c.borrow_mut().take();
                if let Some(mut f) = f {
                    f();
                    let mut slot = c.borrow_mut();
                    if slot.is_none() {
                        *slot = Some(f);
                    }
                }
            });
            crate::widgets::track_widget(&mut self.inner, self.st.clone(), true, fire);
            let min = theme::with(|t| (22.0 * t.font.base as f64 / 12.0).round() as i32).max(22);
            let side = (self.data.borrow().size + 6).max(min);
            self.inner.set_size(side.max(self.inner.w()), side.max(self.inner.h()));
        }
        self
    }

    /// Runs the click action (as a click, Enter or Space would).
    pub fn click(&self) {
        let f = self.click.borrow_mut().take();
        if let Some(mut f) = f {
            f();
            let mut slot = self.click.borrow_mut();
            if slot.is_none() {
                *slot = Some(f);
            }
        }
    }
}

// ======================================================== built-in icons

/// A few Lucide icons the toolkit itself uses (image placeholder, viewer
/// toolbar). lucide-static 0.564.0, ISC license (© Lucide Contributors,
/// portions © Cole Bemis as part of Feather, MIT). Parse with
/// [`Svg::cached`].
pub mod lucide {
    macro_rules! lucide {
        ($name:ident, $body:literal) => {
            pub const $name: &str = concat!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">"#,
                $body,
                "</svg>"
            );
        };
    }
    lucide!(IMAGE, r#"<rect width="18" height="18" x="3" y="3" rx="2" ry="2"/><circle cx="9" cy="9" r="2"/><path d="m21 15-3.086-3.086a2 2 0 0 0-2.828 0L6 21"/>"#);
    lucide!(CIRCLE_ALERT, r#"<circle cx="12" cy="12" r="10"/><line x1="12" x2="12" y1="8" y2="12"/><line x1="12" x2="12.01" y1="16" y2="16"/>"#);
    lucide!(ZOOM_IN, r#"<circle cx="11" cy="11" r="8"/><line x1="21" x2="16.65" y1="21" y2="16.65"/><line x1="11" x2="11" y1="8" y2="14"/><line x1="8" x2="14" y1="11" y2="11"/>"#);
    lucide!(ZOOM_OUT, r#"<circle cx="11" cy="11" r="8"/><line x1="21" x2="16.65" y1="21" y2="16.65"/><line x1="8" x2="14" y1="11" y2="11"/>"#);
    lucide!(SCAN, r#"<path d="M3 7V5a2 2 0 0 1 2-2h2"/><path d="M17 3h2a2 2 0 0 1 2 2v2"/><path d="M21 17v2a2 2 0 0 1-2 2h-2"/><path d="M7 21H5a2 2 0 0 1-2-2v-2"/>"#);
    lucide!(COPY, r#"<rect width="14" height="14" x="8" y="8" rx="2" ry="2"/><path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2"/>"#);
    lucide!(X, r#"<path d="M18 6 6 18"/><path d="m6 6 12 12"/>"#);
    lucide!(FOLDER, r#"<path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"/>"#);
    lucide!(SEARCH, r#"<path d="m21 21-4.34-4.34"/><circle cx="11" cy="11" r="8"/>"#);
    lucide!(SETTINGS, r#"<path d="M9.671 4.136a2.34 2.34 0 0 1 4.659 0 2.34 2.34 0 0 0 3.319 1.915 2.34 2.34 0 0 1 2.33 4.033 2.34 2.34 0 0 0 0 3.831 2.34 2.34 0 0 1-2.33 4.033 2.34 2.34 0 0 0-3.319 1.915 2.34 2.34 0 0 1-4.659 0 2.34 2.34 0 0 0-3.32-1.915 2.34 2.34 0 0 1-2.33-4.033 2.34 2.34 0 0 0 0-3.831A2.34 2.34 0 0 1 6.35 6.051a2.34 2.34 0 0 0 3.319-1.915"/><circle cx="12" cy="12" r="3"/>"#);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alpha_at(px: &[u8], w: usize, x: usize, y: usize) -> u8 {
        px[(y * w + x) * 4 + 3]
    }

    #[test]
    fn builtins_parse_and_rasterize_in_the_tint() {
        for src in [lucide::IMAGE, lucide::CIRCLE_ALERT, lucide::ZOOM_IN, lucide::ZOOM_OUT, lucide::SCAN, lucide::COPY, lucide::X, lucide::FOLDER, lucide::SEARCH, lucide::SETTINGS] {
            let s = Svg::cached(src).expect("built-in icon parses");
            assert!(!s.shapes().is_empty());
            assert_eq!(Svg::cached(src).unwrap().id(), s.id(), "parsed once");
        }
        let x = Svg::cached(lucide::X).unwrap();
        let red = Color::rgba(1.0, 0.0, 0.0, 1.0);
        let px = x.rasterize(24, 24, red);
        // The diagonals cross at the center; the corners stay empty.
        let i = (12 * 24 + 12) * 4;
        assert_eq!(&px[i..i + 4], &[255, 0, 0, 255]);
        assert_eq!(alpha_at(&px, 24, 1, 12), 0);
        assert_eq!(alpha_at(&px, 24, 12, 2), 0);
        // Same icon at 3x keeps proportions: center still covered.
        let big = x.rasterize(72, 72, red);
        assert_eq!(alpha_at(&big, 72, 36, 36), 255);
        assert_eq!(alpha_at(&big, 72, 36, 6), 0);
    }

    #[test]
    fn translucent_tint_does_not_double_where_strokes_cross() {
        let x = Svg::cached(lucide::X).unwrap();
        let px = x.rasterize(24, 24, Color::rgba(1.0, 1.0, 1.0, 0.5));
        let center = alpha_at(&px, 24, 12, 12);
        let arm = alpha_at(&px, 24, 8, 8);
        assert!((center as i32 - 128).abs() <= 2 && (arm as i32 - 128).abs() <= 2, "{center} {arm}");
    }

    #[test]
    fn even_odd_cuts_holes_and_non_square_view_boxes_center() {
        let ring = Svg::parse(r#"<svg viewBox="0 0 20 10"><path fill-rule="evenodd" d="M0 0H10V10H0Z M2 2H8V8H2Z"/></svg>"#).unwrap();
        // 20 × 10 viewBox in a 40 × 40 box: scale 2, centered vertically (y 10..30).
        let px = ring.rasterize(40, 40, Color::rgba(0.0, 0.0, 0.0, 1.0));
        assert_eq!(alpha_at(&px, 40, 2, 20), 255, "ring");
        assert_eq!(alpha_at(&px, 40, 10, 20), 0, "hole (same winding, even-odd)");
        assert_eq!(alpha_at(&px, 40, 10, 5), 0, "letterboxed above");
        assert_eq!(alpha_at(&px, 40, 30, 20), 0, "right half of the viewBox is empty");
        let nz = Svg::parse(r#"<svg viewBox="0 0 20 10"><path d="M0 0H10V10H0Z M2 2H8V8H2Z"/></svg>"#).unwrap();
        let px = nz.rasterize(40, 40, Color::rgba(0.0, 0.0, 0.0, 1.0));
        assert_eq!(alpha_at(&px, 40, 10, 20), 255, "non-zero fills the same-wound inner square");
    }

    #[test]
    fn fixed_colors_stay_and_current_color_follows_the_tint() {
        let s = Svg::parse(r##"<svg viewBox="0 0 10 10"><rect width="5" height="10" fill="#00ff00"/><rect x="5" width="5" height="10" fill="currentColor"/></svg>"##).unwrap();
        let px = s.rasterize(10, 10, Color::rgba(0.0, 0.0, 1.0, 1.0));
        assert_eq!(&px[(5 * 10 + 2) * 4..(5 * 10 + 2) * 4 + 4], &[0, 255, 0, 255]);
        assert_eq!(&px[(5 * 10 + 7) * 4..(5 * 10 + 7) * 4 + 4], &[0, 0, 255, 255]);
    }

    #[test]
    fn tint_roles_follow_the_theme() {
        let t = theme::current();
        assert_eq!(Tint::from(ColorRole::Accent).rgb(&t), t.palette.accent);
        assert_eq!(Tint::Fixed(Rgb(0x123456)).rgb(&t), Rgb(0x123456));
    }
}
