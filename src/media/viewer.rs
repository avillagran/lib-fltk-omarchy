//! `ImageViewer`: a modal overlay (like `ConfirmDialog`) that shows one
//! image large, with zoom, pan, copy and keyboard control.

use super::image::crop;
use super::{cache, Bitmap, Decoded, Error, Fit, Limits, Source, Target};
use crate::a11y::{self, Action, Role};
use crate::icon::{self, lucide, Svg};
use crate::overlay;
use crate::paint;
use crate::theme;
use fltk::{app, draw, enums::*, image::RgbImage, prelude::*, widget::Widget};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

/// Zoom limits, relative to the fitted size.
const MIN_ZOOM: f32 = 0.1;
const MAX_ZOOM: f32 = 32.0;
const STEP: f32 = 1.25;
/// Largest side decoded for the viewer, in device pixels.
const MAX_SIDE: u32 = 4096;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tool {
    ZoomOut,
    ZoomIn,
    Fit,
    Copy,
    Close,
}

impl Tool {
    fn svg(self) -> &'static str {
        match self {
            Tool::ZoomOut => lucide::ZOOM_OUT,
            Tool::ZoomIn => lucide::ZOOM_IN,
            Tool::Fit => lucide::SCAN,
            Tool::Copy => lucide::COPY,
            Tool::Close => lucide::X,
        }
    }

    fn label(self) -> String {
        match self {
            Tool::ZoomOut => crate::tr!("Zoom out"),
            Tool::ZoomIn => crate::tr!("Zoom in"),
            Tool::Fit => crate::tr!("Fit to window"),
            Tool::Copy => crate::tr!("Copy image"),
            Tool::Close => crate::tr!("Close"),
        }
    }

    fn keys(self) -> &'static str {
        match self {
            Tool::ZoomOut => "-",
            Tool::ZoomIn => "+",
            Tool::Fit => "0",
            Tool::Copy => "Mod+C",
            Tool::Close => "Escape",
        }
    }
}

type CopyHook = Rc<dyn Fn(&Bitmap)>;

/// Builder of the image viewer popup:
///
/// ```no_run
/// # use fltk_omarchy::media::{ImageViewer, Source};
/// # fn f(anchor: &fltk::widget::Widget) {
/// ImageViewer::new(&Source::file("/tmp/a.png"))
///     .name("a.png")
///     .on_copy(|bitmap| { let _png = bitmap.to_png(); /* to the clipboard */ })
///     .open(anchor);
/// # }
/// ```
///
/// Keys: Esc closes (focus goes back), `+` / `-` zoom, `0` fits again,
/// arrows pan, `Mod+C` copies, Tab / Shift+Tab walk the toolbar and Enter
/// or Space press the focused button. The mouse wheel zooms at the
/// pointer, dragging pans, a click outside the image closes.
pub struct ImageViewer {
    source: Source,
    name: String,
    preview: Option<Arc<Decoded>>,
    copy: Option<CopyHook>,
    limits: Limits,
}

struct VState {
    name: String,
    decoded: Option<Arc<Decoded>>,
    loaded: bool,
    error: Option<Error>,
    zoom: f32,
    pan: (f32, f32),
    tools: Vec<Tool>,
    focus: Option<usize>,
    hover: Option<usize>,
    drag: Option<((i32, i32), (f32, f32))>,
    copy: Option<CopyHook>,
    start: Option<Instant>,
    timer: bool,
    frame: usize,
    shown: Option<((usize, usize, (u32, u32, u32, u32)), RgbImage)>,
    rtl: bool,
    ppu: f32,
}

/// An open viewer (for tests and app control). Closing twice is a no-op.
#[derive(Clone)]
pub struct ViewerHandle {
    layer: Widget,
    st: Rc<RefCell<VState>>,
}

fn sc(px: f64) -> i32 {
    theme::with(|t| (px * t.font.base as f64 / 12.0).round() as i32)
}

impl ImageViewer {
    pub fn new(source: &Source) -> ImageViewer {
        ImageViewer { source: source.clone(), name: String::new(), preview: None, copy: None, limits: Limits::default() }
    }

    /// The title and accessible name (a file name, alt text).
    pub fn name(mut self, name: &str) -> Self {
        self.name = name.to_string();
        self
    }

    /// A decode to show at once (the thumbnail) while the larger one loads.
    pub fn preview(mut self, d: Arc<Decoded>) -> Self {
        self.preview = Some(d);
        self
    }

    /// Adds the Copy button and `Mod+C`: `f` receives the frame on screen
    /// as straight RGBA (at the viewer's decode size, up to 4096 px a
    /// side); the app puts it on the clipboard (`Bitmap::to_png`).
    pub fn on_copy(mut self, f: impl Fn(&Bitmap) + 'static) -> Self {
        self.copy = Some(Rc::new(f));
        self
    }

    pub fn limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }

    /// Opens over `anchor`'s window; None if it has no window.
    pub fn open<A: WidgetExt>(self, anchor: &A) -> Option<ViewerHandle> {
        let win = anchor.top_window()?;
        let ppu = crate::native::pixels_per_unit(anchor);
        let mut layer = overlay::open_layer(anchor, None, true)?;
        let mut tools = vec![Tool::ZoomOut, Tool::ZoomIn, Tool::Fit];
        if self.copy.is_some() {
            tools.push(Tool::Copy);
        }
        tools.push(Tool::Close);
        let preview = self.preview.clone().or_else(|| cache::largest(&self.source));
        let st = Rc::new(RefCell::new(VState {
            name: if self.name.is_empty() { crate::tr!("Image") } else { self.name.clone() },
            decoded: preview,
            loaded: false,
            error: None,
            zoom: 1.0,
            pan: (0.0, 0.0),
            tools,
            focus: None,
            hover: None,
            drag: None,
            copy: self.copy.clone(),
            start: None,
            timer: false,
            frame: 0,
            shown: None,
            rtl: crate::layout::is_rtl(anchor),
            ppu,
        }));
        let h = ViewerHandle { layer: layer.clone(), st: st.clone() };
        // Larger than the window so zooming in stays sharp for a while.
        let side = |v: i32| ((v as f32 * ppu * 2.0) as u32).clamp(256, MAX_SIDE);
        let target = Target::new(side(win.w()), side(win.h()), Fit::Contain);
        let (s2, l2) = (st.clone(), layer.clone());
        cache::load_with(&self.source, target, self.limits, move |r| {
            if l2.was_deleted() {
                return;
            }
            {
                let mut s = s2.borrow_mut();
                s.loaded = true;
                match r {
                    Ok(d) => {
                        s.decoded = Some(d);
                        s.error = None;
                        s.shown = None;
                    }
                    Err(e) if s.decoded.is_none() => s.error = Some(e),
                    Err(_) => {}
                }
            }
            l2.clone().redraw();
            a11y::changed(&l2);
        });
        let (s3, h3) = (st.clone(), h.clone());
        overlay::draw_layer(&mut layer, move |l| draw_viewer(l, &s3, &h3));
        let h4 = h.clone();
        layer.handle(move |l, ev| handle(l, ev, &h4));
        let (s5, l5) = (st.clone(), layer.clone());
        a11y::describe(&layer, move |n| {
            let s = s5.borrow();
            n.role(Role::Dialog).name(&s.name).modal(true).bounds(l5.x(), l5.y(), l5.w(), l5.h());
            let (ix, iy, iw, ih) = image_rect(&l5, &s).unwrap_or((0, 0, 0, 0));
            n.child(0, |c| {
                c.role(Role::Image).name(&s.name).bounds(ix, iy, iw, ih);
                if s.error.is_some() {
                    c.description(&crate::tr!("Image could not be loaded"));
                }
            });
            for (i, t) in s.tools.iter().enumerate() {
                let (bx, by, bw, bh) = tool_rect(&l5, &s, i);
                n.child(i as u64 + 1, |b| {
                    b.role(Role::Button).name(&t.label()).bounds(bx, by, bw, bh).action(Action::Click).focusable().shortcut(t.keys());
                    if s.focus == Some(i) {
                        b.focused();
                    }
                });
            }
        });
        let h6 = h.clone();
        a11y::on_action(&layer, move |r| {
            let (Some(k), Action::Click) = (r.key(), r.action) else { return false };
            let tool = h6.st.borrow().tools.get((k as usize).wrapping_sub(1)).copied();
            match tool {
                Some(t) => {
                    run(&h6, t);
                    true
                }
                None => false,
            }
        });
        Some(h)
    }
}

impl ViewerHandle {
    pub fn is_open(&self) -> bool {
        !self.layer.was_deleted() && self.layer.parent().is_some() && self.layer.active()
    }

    pub fn close(&self) {
        if !self.layer.was_deleted() {
            overlay::close_layer(&self.layer);
        }
    }

    /// Zoom relative to the fitted size (1 = fit).
    pub fn zoom(&self) -> f32 {
        self.st.borrow().zoom
    }

    pub fn set_zoom(&self, z: f32) {
        zoom_at(self, z, None);
    }

    /// Offset of the image center from the view center, in FLTK units.
    pub fn pan(&self) -> (f32, f32) {
        self.st.borrow().pan
    }

    /// True once the viewer's own decode finished (or failed).
    pub fn is_loaded(&self) -> bool {
        self.st.borrow().loaded
    }

    pub fn error(&self) -> Option<Error> {
        self.st.borrow().error.clone()
    }

    /// Where the image is drawn, in window coordinates.
    pub fn image_rect(&self) -> Option<(i32, i32, i32, i32)> {
        if self.layer.was_deleted() {
            return None;
        }
        image_rect(&self.layer, &self.st.borrow())
    }

    /// Runs the Copy hook with the frame on screen.
    pub fn copy(&self) {
        run(self, Tool::Copy);
    }
}

// ---------------------------------------------------------------- geometry

fn margin() -> i32 {
    sc(12.0)
}

fn tool_size() -> i32 {
    sc(30.0)
}

/// Toolbar button `i`, top right (top left in a right-to-left language).
fn tool_rect(l: &Widget, s: &VState, i: usize) -> (i32, i32, i32, i32) {
    let (m, b, gap) = (margin(), tool_size(), sc(4.0));
    let n = s.tools.len() as i32;
    let x0 = l.x() + l.w() - m - n * b - (n - 1) * gap;
    let r = (x0 + i as i32 * (b + gap), l.y() + m, b, b);
    crate::layout::Mirror::new(s.rtl, l.x(), l.w()).rect(r)
}

/// The area images use (below the toolbar).
fn view_rect(l: &Widget) -> (i32, i32, i32, i32) {
    let m = margin();
    let top = m + tool_size() + m;
    (l.x() + m, l.y() + top, (l.w() - 2 * m).max(1), (l.h() - top - m).max(1))
}

/// Units per decoded pixel at zoom 1: fitted, never above the file's
/// natural size on this screen.
fn base_scale(l: &Widget, s: &VState, d: &Decoded) -> f32 {
    let (_, _, vw, vh) = view_rect(l);
    let (iw, ih) = d.size();
    let fit = (vw as f32 / iw as f32).min(vh as f32 / ih as f32);
    let natural = (d.source_size.0 as f32 / iw as f32) / s.ppu.max(0.25);
    fit.min(natural)
}

fn image_rect_f(l: &Widget, s: &VState) -> Option<(f32, f32, f32, f32)> {
    let d = s.decoded.as_ref()?;
    let k = base_scale(l, s, d) * s.zoom;
    let (iw, ih) = d.size();
    let (w, h) = (iw as f32 * k, ih as f32 * k);
    let (vx, vy, vw, vh) = view_rect(l);
    let (cx, cy) = (vx as f32 + vw as f32 / 2.0 + s.pan.0, vy as f32 + vh as f32 / 2.0 + s.pan.1);
    Some((cx - w / 2.0, cy - h / 2.0, w, h))
}

fn image_rect(l: &Widget, s: &VState) -> Option<(i32, i32, i32, i32)> {
    image_rect_f(l, s).map(|(x, y, w, h)| (x.round() as i32, y.round() as i32, w.round() as i32, h.round() as i32))
}

/// Keeps the image on screen: centered on an axis where it fits,
/// edges no further in than the view's where it is larger.
fn clamp_pan(l: &Widget, s: &mut VState) {
    let Some(d) = s.decoded.clone() else { return };
    let k = base_scale(l, s, &d) * s.zoom;
    let (_, _, vw, vh) = view_rect(l);
    let (w, h) = (d.size().0 as f32 * k, d.size().1 as f32 * k);
    let lim = |img: f32, view: f32| ((img - view) / 2.0).max(0.0);
    let (lx, ly) = (lim(w, vw as f32), lim(h, vh as f32));
    s.pan = (s.pan.0.clamp(-lx, lx), s.pan.1.clamp(-ly, ly));
}

fn zoom_at(h: &ViewerHandle, z: f32, at: Option<(i32, i32)>) {
    if h.layer.was_deleted() {
        return;
    }
    {
        let mut s = h.st.borrow_mut();
        let z = z.clamp(MIN_ZOOM, MAX_ZOOM);
        let (vx, vy, vw, vh) = view_rect(&h.layer);
        let c = (vx as f32 + vw as f32 / 2.0, vy as f32 + vh as f32 / 2.0);
        // The point under the pointer stays put.
        let p = at.map_or(c, |(x, y)| (x as f32, y as f32));
        let r = z / s.zoom;
        s.pan = (p.0 - c.0 - (p.0 - c.0 - s.pan.0) * r, p.1 - c.1 - (p.1 - c.1 - s.pan.1) * r);
        s.zoom = z;
        clamp_pan(&h.layer, &mut s);
    }
    h.layer.clone().redraw();
}

fn run(h: &ViewerHandle, t: Tool) {
    match t {
        Tool::ZoomIn => {
            let z = h.zoom() * STEP;
            zoom_at(h, z, None)
        }
        Tool::ZoomOut => {
            let z = h.zoom() / STEP;
            zoom_at(h, z, None)
        }
        Tool::Fit => {
            h.st.borrow_mut().pan = (0.0, 0.0);
            zoom_at(h, 1.0, None)
        }
        Tool::Copy => {
            let (hook, bm) = {
                let s = h.st.borrow();
                (s.copy.clone(), s.decoded.as_ref().map(|d| d.frames[s.frame.min(d.frames.len() - 1)].bitmap.clone()))
            };
            if let (Some(f), Some(b)) = (hook, bm) {
                f(&b);
            }
        }
        Tool::Close => h.close(),
    }
}

fn pan_by(h: &ViewerHandle, dx: f32, dy: f32) {
    {
        let mut s = h.st.borrow_mut();
        s.pan = (s.pan.0 + dx, s.pan.1 + dy);
        clamp_pan(&h.layer, &mut s);
    }
    h.layer.clone().redraw();
}

// ----------------------------------------------------------------- events

fn tool_at(l: &Widget, s: &VState, x: i32, y: i32) -> Option<usize> {
    (0..s.tools.len()).find(|&i| {
        let (bx, by, bw, bh) = tool_rect(l, s, i);
        x >= bx && x < bx + bw && y >= by && y < by + bh
    })
}

fn handle(l: &mut Widget, ev: Event, h: &ViewerHandle) -> bool {
    let (x, y) = app::event_coords();
    match ev {
        Event::KeyDown => {
            let key = app::event_key();
            let text = app::event_text();
            let (vw, vh) = {
                let (_, _, w, h) = view_rect(l);
                (w as f32, h as f32)
            };
            let step = if app::event_state().contains(Shortcut::Shift) { 0.5 } else { 0.1 };
            if key == Key::Escape {
                h.close();
            } else if crate::keys::primary_held() && key == Key::from_char('c') {
                run(h, Tool::Copy);
            } else if key == Key::Tab {
                let n = h.st.borrow().tools.len();
                let back = app::event_state().contains(Shortcut::Shift);
                let mut s = h.st.borrow_mut();
                s.focus = Some(match (s.focus, back) {
                    (None, false) => 0,
                    (None, true) => n - 1,
                    (Some(i), false) => (i + 1) % n,
                    (Some(i), true) => (i + n - 1) % n,
                });
                drop(s);
                l.redraw();
            } else if matches!(key, Key::Enter | Key::KPEnter) || key == Key::from_char(' ') {
                let t = { let s = h.st.borrow(); s.focus.and_then(|i| s.tools.get(i).copied()) };
                if let Some(t) = t {
                    run(h, t);
                }
            } else if text == "+" || text == "=" || key == Key::from_i32(0xffab) {
                run(h, Tool::ZoomIn);
            } else if text == "-" || key == Key::from_i32(0xffad) {
                run(h, Tool::ZoomOut);
            } else if text == "0" || key == Key::from_i32(0xffb0) {
                run(h, Tool::Fit);
            } else if key == Key::Left {
                pan_by(h, vw * step, 0.0);
            } else if key == Key::Right {
                pan_by(h, -vw * step, 0.0);
            } else if key == Key::Up {
                pan_by(h, 0.0, vh * step);
            } else if key == Key::Down {
                pan_by(h, 0.0, -vh * step);
            }
            true
        }
        Event::Move => {
            let hov = tool_at(l, &h.st.borrow(), x, y);
            if hov != h.st.borrow().hover {
                h.st.borrow_mut().hover = hov;
                l.redraw();
            }
            true
        }
        Event::Push => {
            let (tool, on_image) = {
                let s = h.st.borrow();
                let t = tool_at(l, &s, x, y).map(|i| s.tools[i]);
                let img = image_rect(l, &s).is_some_and(|(ix, iy, iw, ih)| x >= ix && x < ix + iw && y >= iy && y < iy + ih);
                (t, img)
            };
            if let Some(t) = tool {
                run(h, t);
            } else if on_image || h.st.borrow().decoded.is_none() {
                let mut s = h.st.borrow_mut();
                s.drag = Some(((x, y), s.pan));
            } else {
                h.close();
            }
            true
        }
        Event::Drag => {
            let start = h.st.borrow().drag;
            if let Some(((sx, sy), p0)) = start {
                {
                    let mut s = h.st.borrow_mut();
                    s.pan = (p0.0 + (x - sx) as f32, p0.1 + (y - sy) as f32);
                    clamp_pan(l, &mut s);
                }
                l.redraw();
            }
            true
        }
        Event::Released => {
            h.st.borrow_mut().drag = None;
            true
        }
        Event::MouseWheel => {
            let dy = app::event_dy();
            let z = h.zoom();
            let nz = match dy {
                app::MouseWheel::Up => z * STEP,
                app::MouseWheel::Down => z / STEP,
                _ => z,
            };
            zoom_at(h, nz, Some((x, y)));
            true
        }
        Event::Leave => {
            if h.st.borrow_mut().hover.take().is_some() {
                l.redraw();
            }
            true
        }
        Event::Focus | Event::Unfocus | Event::Enter | Event::Shortcut => true,
        _ => false,
    }
}

// ------------------------------------------------------------------- draw

fn draw_viewer(l: &mut Widget, st: &Rc<RefCell<VState>>, h: &ViewerHandle) {
    if let Some(win) = l.top_window() {
        if (l.w(), l.h()) != (win.w(), win.h()) {
            l.resize(0, 0, win.w(), win.h());
        }
    }
    theme::with(|t| {
        let bg = t.palette.background;
        let fg = t.palette.foreground;
        overlay::draw_scrim(l, bg, 0.92);
        let (vx, vy, vw, vh) = view_rect(l);
        let mut s = st.borrow_mut();
        clamp_pan(l, &mut s);
        match s.decoded.clone() {
            Some(d) => {
                let idx = frame_index(&mut s, &d, l, st);
                s.frame = idx;
                draw_image(l, &mut s, &d, idx, (vx, vy, vw, vh));
            }
            None => {
                let (src, color, text) = if s.error.is_some() {
                    (lucide::CIRCLE_ALERT, t.palette.urgent, crate::tr!("Image could not be loaded"))
                } else {
                    (lucide::IMAGE, fg.darker(1.5), crate::tr!("Loading image…"))
                };
                let side = sc(32.0);
                let (cx, cy) = (vx + vw / 2, vy + vh / 2);
                if let Ok(svg) = Svg::cached(src) {
                    icon::draw(l, &svg, cx - side / 2, cy - side, side, side, color);
                }
                paint::text(&text, vx, cy + sc(8.0), vw, t.font.body + 8, t.font.body, false, fg, Align::Center);
            }
        }
        // Title and zoom on the toolbar row (start side).
        let m = margin();
        let first = tool_rect(l, &s, 0);
        let last = tool_rect(l, &s, s.tools.len() - 1);
        let tb = (first.0.min(last.0), first.1, (first.0.max(last.0) + first.2) - first.0.min(last.0), first.3);
        let title_w = (l.w() - 3 * m - tb.2).max(0);
        let title = format!("{}  {}%", s.name, (s.zoom * 100.0).round() as i32);
        let tx = crate::layout::Mirror::new(s.rtl, l.x(), l.w()).x(l.x() + m, title_w);
        let align = if s.rtl { Align::Right } else { Align::Left };
        paint::text_elided(&title, tx, tb.1, title_w, tb.3, t.font.body, false, fg, align);
        // Toolbar.
        let pad = sc(3.0);
        overlay::card(t, tb.0 - pad, tb.1 - pad, tb.2 + 2 * pad, tb.3 + 2 * pad, &t.popups, 1);
        let keyboard = crate::focus::keyboard_mode();
        for (i, tool) in s.tools.clone().iter().enumerate() {
            let (bx, by, bw, bh) = tool_rect(l, &s, i);
            let focused = s.focus == Some(i) && keyboard;
            let hot = s.hover == Some(i) || focused;
            let under = t.popups.fill(bg);
            if hot {
                paint::fill(bx, by, bw, bh, t.radius, fg.over(under, t.controls.hover.fill_alpha));
            }
            if focused {
                paint::stroke(bx, by, bw, bh, t.radius, t.controls.focus.border_width.max(1), t.border(&t.controls.focus, under));
            }
            if let Ok(svg) = Svg::cached(tool.svg()) {
                let side = sc(16.0);
                icon::draw(l, &svg, bx + (bw - side) / 2, by + (bh - side) / 2, side, side, fg);
            }
            if hot {
                // Its name under it: the tooltip of an overlay button.
                let label = format!("{}  {}", tool.label(), crate::keys::label(tool.keys()));
                let tw = paint::measure(&label, t.font.caption, false) + sc(16.0);
                let th = t.font.caption + sc(10.0);
                let tx = (bx + bw / 2 - tw / 2).clamp(l.x() + m / 2, l.x() + l.w() - tw - m / 2);
                overlay::card(t, tx, by + bh + sc(8.0), tw, th, &t.tooltip, 1);
                paint::text(&label, tx, by + bh + sc(8.0), tw, th, t.font.caption, false, t.tooltip.text, Align::Center);
            }
        }
        let _ = h;
    });
}

fn frame_index(s: &mut VState, d: &Decoded, l: &Widget, st: &Rc<RefCell<VState>>) -> usize {
    if !(d.is_animated() && crate::anim::enabled()) {
        s.start = None;
        return 0;
    }
    let now = crate::anim::now();
    let start = *s.start.get_or_insert(now);
    let (i, left) = d.frame_at(now.saturating_duration_since(start));
    if !s.timer && left != std::time::Duration::MAX {
        s.timer = true;
        let (alive, weak) = (l.clone(), Rc::downgrade(st));
        crate::timer::add(left.as_secs_f64().max(1.0 / 120.0), move |_| {
            if let Some(st) = weak.upgrade() {
                st.borrow_mut().timer = false;
            }
            if !alive.was_deleted() && alive.visible_r() {
                alive.clone().redraw();
            }
        });
    }
    i
}

fn draw_image(l: &Widget, s: &mut VState, d: &Arc<Decoded>, idx: usize, view: (i32, i32, i32, i32)) {
    let Some((ix, iy, iw, ih)) = image_rect_f(l, s) else { return };
    let (vx, vy, vw, vh) = (view.0 as f32, view.1 as f32, view.2 as f32, view.3 as f32);
    // Visible part of the image, in decoded pixels.
    let (x0, y0) = (ix.max(vx), iy.max(vy));
    let (x1, y1) = ((ix + iw).min(vx + vw), (iy + ih).min(vy + vh));
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let (bw, bh) = d.size();
    let k = bw as f32 / iw; // pixels per unit
    let cx0 = ((x0 - ix) * k).floor().clamp(0.0, bw as f32 - 1.0) as u32;
    let cy0 = ((y0 - iy) * k).floor().clamp(0.0, bh as f32 - 1.0) as u32;
    let cx1 = ((x1 - ix) * k).ceil().clamp(cx0 as f32 + 1.0, bw as f32) as u32;
    let cy1 = ((y1 - iy) * k).ceil().clamp(cy0 as f32 + 1.0, bh as f32) as u32;
    let crop_px = (cx0, cy0, cx1 - cx0, cy1 - cy0);
    // Destination of the whole-pixel crop (may overhang the view: clipped).
    let dx = (ix + cx0 as f32 / k).round() as i32;
    let dy = (iy + cy0 as f32 / k).round() as i32;
    let dw = ((crop_px.2 as f32) / k).round().max(1.0) as i32;
    let dh = ((crop_px.3 as f32) / k).round().max(1.0) as i32;
    let key = (Arc::as_ptr(d) as *const u8 as usize, idx, crop_px);
    if s.shown.as_ref().map_or(true, |(k2, _)| *k2 != key) {
        let bm = &d.frames[idx].bitmap;
        let px = crop(bm.rgba(), bm.width as usize, crop_px);
        s.shown = RgbImage::new(&px, crop_px.2 as i32, crop_px.3 as i32, ColorDepth::Rgba8).ok().map(|img| (key, img));
    }
    if let Some((_, img)) = s.shown.as_mut() {
        draw::push_clip(view.0, view.1, view.2, view.3);
        img.scale(dw, dh, false, true);
        img.draw(dx, dy, dw, dh);
        draw::pop_clip();
    }
}
