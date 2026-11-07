//! `Image`: a widget that shows a [`Source`] decoded at its device size.

use super::{cache, Decoded, Error, Fit, Limits, Source, Target};
use crate::a11y::{self, Action, Role};
use crate::icon::{self, lucide, Svg};
use crate::paint::{self, Interaction};
use crate::theme;
use fltk::{enums::*, image::RgbImage, prelude::*, widget::Widget};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

/// Composed frames for one geometry (crop + rounded corners), ready to blit.
struct Display {
    key: (usize, (i32, i32, i32, i32), (u32, u32, u32, u32), i32, Fit),
    frames: Vec<Option<RgbImage>>,
}

struct St {
    source: Option<Source>,
    fit: Fit,
    rounded: bool,
    framed: bool,
    name: String,
    limits: Limits,
    decoded: Option<Arc<Decoded>>,
    /// The target last asked for (loaded, failed or still loading).
    asked: Option<Target>,
    loading: bool,
    error: Option<Error>,
    /// Source generation: late results of a replaced source are dropped.
    gen: u64,
    display: Option<Display>,
    start: Option<Instant>,
    timer: bool,
    frame: usize,
    activate: Option<Rc<dyn Fn()>>,
}

/// Shows an image: decoded on a worker at the widget's size in device
/// pixels (re-decoded when it grows), cached ([`super::load`]), drawn with
/// [`Fit`] and the theme's badge radius. A placeholder (the Lucide image
/// icon, or an alert icon in the urgent color on error) shows while it
/// loads. Animated GIF and WebP play on the animation clock while the
/// widget is visible; with animations off (system setting) the first
/// frame shows. Decorative for screen readers unless it has a
/// [`label`](Image::label); with [`on_activate`](Image::on_activate) it
/// is a button (Tab stop, Enter, Space, click).
#[derive(Clone)]
pub struct Image {
    inner: Widget,
    st: Rc<RefCell<St>>,
    ia: Rc<RefCell<Interaction>>,
}
fltk::widget_extends!(Image, Widget, inner);

fn scaled(px: f64) -> i32 {
    theme::with(|t| (px * t.font.base as f64 / 12.0).round() as i32)
}

/// Device size, rounded up to 64 px steps so small resizes reuse a decode.
fn bucket(v: f32) -> u32 {
    (((v.max(1.0) / 64.0).ceil() as u32) * 64).max(64)
}

impl Image {
    /// An empty image box of `w × h` FLTK units.
    pub fn new(w: i32, h: i32) -> Image {
        let mut inner = Widget::default();
        inner.set_frame(FrameType::NoBox);
        inner.clear_visible_focus();
        inner.set_size(w.max(1), h.max(1));
        let st = Rc::new(RefCell::new(St {
            source: None,
            fit: Fit::Contain,
            rounded: true,
            framed: false,
            name: String::new(),
            limits: Limits::default(),
            decoded: None,
            asked: None,
            loading: false,
            error: None,
            gen: 0,
            display: None,
            start: None,
            timer: false,
            frame: 0,
            activate: None,
        }));
        let ia = Rc::new(RefCell::new(Interaction::default()));
        let s2 = st.clone();
        inner.draw(move |w| draw(w, &s2));
        let s3 = st.clone();
        a11y::describe(&inner, move |n| {
            let s = s3.borrow();
            if s.activate.is_some() {
                let name = if s.name.is_empty() { crate::tr!("Image") } else { s.name.clone() };
                n.role(Role::Button).name(&name).focusable().action(Action::Click);
            } else if s.name.is_empty() {
                n.role(Role::Unknown).leaf(); // decorative: not in the tree
            } else {
                n.role(Role::Image).name(&s.name);
            }
            if s.error.is_some() {
                n.description(&crate::tr!("Image could not be loaded"));
            }
        });
        Image { inner, st, ia }
    }

    /// Shows `source` (replacing the previous one).
    pub fn source(mut self, source: &Source) -> Self {
        self.set_source(Some(source));
        self
    }

    pub fn set_source(&mut self, source: Option<&Source>) {
        {
            let mut s = self.st.borrow_mut();
            if s.source.as_ref() == source {
                return;
            }
            s.source = source.cloned();
            s.gen += 1;
            s.decoded = None;
            s.asked = None;
            s.loading = false;
            s.error = None;
            s.display = None;
            s.start = None;
            s.frame = 0;
        }
        self.inner.redraw();
    }

    pub fn fit(self, fit: Fit) -> Self {
        {
            let mut s = self.st.borrow_mut();
            s.fit = fit;
            s.asked = None;
            s.display = None;
        }
        self.inner.clone().redraw();
        self
    }

    /// Rounded corners with the theme's `badge_radius()` (on by default).
    pub fn rounded(self, on: bool) -> Self {
        self.st.borrow_mut().rounded = on;
        self.st.borrow_mut().display = None;
        self
    }

    /// o3's thumbnail card: a faint fill and border around a 4-unit inset.
    pub fn framed(self, on: bool) -> Self {
        self.st.borrow_mut().framed = on;
        self.st.borrow_mut().display = None;
        self
    }

    /// The name screen readers read (a file name, alt text); an unnamed,
    /// inactive image is decorative.
    pub fn label(self, name: &str) -> Self {
        self.st.borrow_mut().name = name.to_string();
        self
    }

    /// Decode limits for this image's loads.
    pub fn limits(self, limits: Limits) -> Self {
        self.st.borrow_mut().limits = limits;
        self
    }

    /// Makes the image a button: `f` runs on click, Enter or Space (open
    /// an [`ImageViewer`](super::ImageViewer), select an attachment).
    pub fn on_activate(mut self, f: impl Fn() + 'static) -> Self {
        let f: Rc<dyn Fn()> = Rc::new(f);
        let first = self.st.borrow_mut().activate.replace(f.clone()).is_none();
        if first {
            let st = self.st.clone();
            let fire: Rc<dyn Fn()> = Rc::new(move || {
                let f = st.borrow().activate.clone();
                if let Some(f) = f {
                    f();
                }
            });
            crate::widgets::track_widget(&mut self.inner, self.ia.clone(), true, fire);
        }
        self
    }

    /// Runs the activation (as Enter would).
    pub fn activate(&self) {
        let f = self.st.borrow().activate.clone();
        if let Some(f) = f {
            f();
        }
    }

    /// The decode on screen, once loaded.
    pub fn decoded(&self) -> Option<Arc<Decoded>> {
        self.st.borrow().decoded.clone()
    }

    pub fn error(&self) -> Option<Error> {
        self.st.borrow().error.clone()
    }

    pub fn is_loading(&self) -> bool {
        self.st.borrow().loading
    }

    /// Index of the frame drawn last (animations).
    pub fn frame_index(&self) -> usize {
        self.st.borrow().frame
    }

    /// A viewer for this image's source, named like it, showing the
    /// current decode until the larger one is ready.
    pub fn viewer(&self) -> Option<super::ImageViewer> {
        let s = self.st.borrow();
        let src = s.source.as_ref()?;
        let mut v = super::ImageViewer::new(src).name(&s.name);
        if let Some(d) = &s.decoded {
            v = v.preview(d.clone());
        }
        Some(v)
    }

    /// Starts loading for the current size now (normally the first draw
    /// does it, so images off screen are not decoded).
    pub fn preload(&self) {
        let ppu = crate::native::pixels_per_unit(&self.inner);
        let (x, y, w, h) = content_box(&self.inner, &self.st.borrow());
        let _ = (x, y);
        ensure(&self.inner, &self.st, w, h, ppu);
    }
}

fn content_box(w: &Widget, s: &St) -> (i32, i32, i32, i32) {
    let inset = if s.framed { scaled(4.0) } else { 0 };
    (w.x() + inset, w.y() + inset, (w.w() - 2 * inset).max(1), (w.h() - 2 * inset).max(1))
}

/// Asks for a decode when nothing suitable is loaded or loading.
fn ensure(w: &Widget, st: &Rc<RefCell<St>>, bw: i32, bh: i32, ppu: f32) {
    let (source, need, limits, gen) = {
        let s = st.borrow();
        let Some(src) = s.source.clone() else { return };
        let need = Target::new(bucket(bw as f32 * ppu), bucket(bh as f32 * ppu), s.fit).normalized();
        if s.asked == Some(need) {
            return;
        }
        // What is loaded is enough: as large as needed (not more than 2×),
        // or already the file's full size.
        if let Some(asked) = s.asked.filter(|a| a.fit == need.fit) {
            let big_enough = asked.width >= need.width && asked.height >= need.height;
            let not_wasteful = asked.width <= need.width * 2 && asked.height <= need.height * 2;
            if big_enough && (not_wasteful || s.loading) {
                return;
            }
            // The file is smaller than both sizes: any decode is the same.
            if let Some(d) = &s.decoded {
                let (w, h) = d.size();
                if d.size() == d.source_size && w <= need.width && h <= need.height && w <= asked.width && h <= asked.height {
                    return;
                }
            }
        }
        (src, need, s.limits, s.gen)
    };
    {
        let mut s = st.borrow_mut();
        s.asked = Some(need);
        s.loading = true;
    }
    // Synchronous when cached: no placeholder flash.
    if let Some(d) = cache::peek(&source, need) {
        apply(st, Ok(d));
        return;
    }
    let (st2, alive) = (st.clone(), w.clone());
    cache::load_with(&source, need, limits, move |r| {
        if alive.was_deleted() || st2.borrow().gen != gen {
            return;
        }
        apply(&st2, r);
        let mut w = alive.clone();
        w.redraw();
        a11y::changed(&w);
    });
}

fn apply(st: &Rc<RefCell<St>>, r: Result<Arc<Decoded>, Error>) {
    let mut s = st.borrow_mut();
    s.loading = false;
    match r {
        Ok(d) => {
            s.decoded = Some(d);
            s.error = None;
            s.display = None;
        }
        Err(Error::Cancelled) => s.asked = None,
        Err(e) => {
            if s.decoded.is_none() {
                s.error = Some(e);
            }
        }
    }
}

/// Multiplies alpha outside a rounded rectangle (`rect` in buffer pixels,
/// radius `r`) to 0, anti-aliased; only the four corner squares change.
pub(crate) fn round_corners(buf: &mut [u8], w: usize, h: usize, rect: (f32, f32, f32, f32), r: f32) {
    if r <= 0.0 {
        return;
    }
    let (rx, ry, rw, rh) = rect;
    let r = r.min(rw / 2.0).min(rh / 2.0);
    let corners = [(rx + r, ry + r, -1.0, -1.0), (rx + rw - r, ry + r, 1.0, -1.0), (rx + r, ry + rh - r, -1.0, 1.0), (rx + rw - r, ry + rh - r, 1.0, 1.0)];
    for (cx, cy, sx, sy) in corners {
        let (x0, x1) = if sx < 0.0 { (cx - r, cx) } else { (cx, cx + r) };
        let (y0, y1) = if sy < 0.0 { (cy - r, cy) } else { (cy, cy + r) };
        let xs = (x0.floor().max(0.0) as usize)..(x1.ceil().max(0.0) as usize).min(w);
        let ys = (y0.floor().max(0.0) as usize)..(y1.ceil().max(0.0) as usize).min(h);
        for y in ys {
            for x in xs.clone() {
                let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                if (px - cx) * sx < 0.0 || (py - cy) * sy < 0.0 {
                    continue;
                }
                let d = ((px - cx).powi(2) + (py - cy).powi(2)).sqrt();
                let cov = (r - d + 0.5).clamp(0.0, 1.0);
                if cov < 1.0 {
                    let a = &mut buf[(y * w + x) * 4 + 3];
                    *a = (*a as f32 * cov + 0.5) as u8;
                }
            }
        }
    }
}

/// Copies `crop` (x, y, w, h in pixels) out of a straight RGBA bitmap.
pub(crate) fn crop(src: &[u8], sw: usize, crop: (u32, u32, u32, u32)) -> Vec<u8> {
    let (x, y, w, h) = (crop.0 as usize, crop.1 as usize, crop.2 as usize, crop.3 as usize);
    if x == 0 && w == sw && y == 0 && src.len() == w * h * 4 {
        return src.to_vec();
    }
    let mut out = Vec::with_capacity(w * h * 4);
    for row in y..y + h {
        out.extend_from_slice(&src[(row * sw + x) * 4..(row * sw + x + w) * 4]);
    }
    out
}

fn draw(w: &mut Widget, st: &Rc<RefCell<St>>) {
    let ppu = crate::native::pixels_per_unit(w);
    let (bx, by, bw, bh) = content_box(w, &st.borrow());
    ensure(w, st, bw, bh, ppu);
    let weak = Rc::downgrade(st);
    let mut s = st.borrow_mut();
    let s = &mut *s;
    theme::with(|t| {
        let r = if s.rounded { t.badge_radius() } else { 0 };
        let (bg, fg) = (t.palette.background, t.palette.foreground);
        if s.framed {
            paint::surface(w.x(), w.y(), w.w(), w.h(), r, Some(fg.over(bg, 0.04)), Some((fg.over(bg, 0.12), 1)));
        }
        let inner_r = if s.framed { (r - scaled(2.0)).max(0) } else { r };
        match s.decoded.clone() {
            Some(d) => draw_frame(w, s, weak.clone(), &d, (bx, by, bw, bh), inner_r, t.palette.darker_background),
            None => {
                if !s.framed {
                    paint::fill(bx, by, bw, bh, inner_r, fg.over(bg, 0.04));
                }
                let (src, color) = if s.error.is_some() { (lucide::CIRCLE_ALERT, t.palette.urgent) } else { (lucide::IMAGE, fg.darker(1.5)) };
                if let Ok(svg) = Svg::cached(src) {
                    let side = scaled(20.0).min(bw.min(bh) / 2).max(1);
                    icon::draw(w, &svg, bx + (bw - side) / 2, by + (bh - side) / 2, side, side, color);
                }
            }
        }
        if s.activate.is_some() && crate::focus::show_ring(w) {
            paint::stroke(w.x(), w.y(), w.w(), w.h(), r, 2, t.border(&t.controls.focus, bg));
        }
    });
}

fn draw_frame(w: &Widget, s: &mut St, weak: std::rc::Weak<RefCell<St>>, d: &Arc<Decoded>, b: (i32, i32, i32, i32), r: i32, bars: theme::Rgb) {
    let (bx, by, bw, bh) = b;
    let (iw, ih) = d.size();
    let (fw, fh) = (iw as f32, ih as f32);
    let (bwf, bhf) = (bw as f32, bh as f32);
    let (dest, crop_px) = match s.fit {
        Fit::Cover => {
            let sc = (bwf / fw).max(bhf / fh);
            let (cw, ch) = ((bwf / sc).round().clamp(1.0, fw), (bhf / sc).round().clamp(1.0, fh));
            ((bx, by, bw, bh), (((fw - cw) / 2.0) as u32, ((fh - ch) / 2.0) as u32, cw as u32, ch as u32))
        }
        _ => {
            let sc = (bwf / fw).min(bhf / fh);
            let (dw, dh) = ((fw * sc).round().max(1.0) as i32, (fh * sc).round().max(1.0) as i32);
            ((bx + (bw - dw) / 2, by + (bh - dh) / 2, dw, dh), (0, 0, iw, ih))
        }
    };
    if s.fit == Fit::Letterbox {
        paint::fill(bx, by, bw, bh, r, bars);
    }
    // Animation on the shared clock, only while it can be seen.
    let idx = if d.is_animated() && crate::anim::enabled() {
        let now = crate::anim::now();
        let start = *s.start.get_or_insert(now);
        let (i, left) = d.frame_at(now.saturating_duration_since(start));
        if !s.timer && left != std::time::Duration::MAX {
            s.timer = true;
            let alive = w.clone();
            // Redraws once the frame is due if the image is still shown;
            // that draw arms the next timer. Hidden: it stops here.
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
    } else {
        s.start = None;
        0
    };
    s.frame = idx;
    let k = crop_px.2 as f32 / dest.2.max(1) as f32;
    let mask = match s.fit {
        Fit::Letterbox => (((bx - dest.0) as f32) * k, ((by - dest.1) as f32) * k, bwf * k, bhf * k),
        _ => (0.0, 0.0, crop_px.2 as f32, crop_px.3 as f32),
    };
    let key = (Arc::as_ptr(d) as *const u8 as usize, dest, crop_px, (r as f32 * k * 16.0) as i32, s.fit);
    if s.display.as_ref().map_or(true, |x| x.key != key) {
        s.display = Some(Display { key, frames: (0..d.frames.len()).map(|_| None).collect() });
    }
    let disp = s.display.as_mut().expect("display");
    if disp.frames[idx].is_none() {
        let bm = &d.frames[idx].bitmap;
        let mut px = crop(bm.rgba(), bm.width as usize, crop_px);
        round_corners(&mut px, crop_px.2 as usize, crop_px.3 as usize, mask, r as f32 * k);
        disp.frames[idx] = RgbImage::new(&px, crop_px.2 as i32, crop_px.3 as i32, ColorDepth::Rgba8).ok();
    }
    if let Some(img) = disp.frames[idx].as_mut() {
        img.scale(dest.2, dest.3, false, true);
        img.draw(dest.0, dest.1, dest.2, dest.3);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounded_corners_clear_only_the_corners() {
        let (w, h) = (20usize, 10usize);
        let mut px = vec![255u8; w * h * 4];
        round_corners(&mut px, w, h, (0.0, 0.0, w as f32, h as f32), 4.0);
        let a = |x: usize, y: usize| px[(y * w + x) * 4 + 3];
        assert_eq!(a(0, 0), 0);
        assert_eq!(a(19, 9), 0);
        assert_eq!(a(10, 0), 255, "edges stay");
        assert_eq!(a(3, 3), 255, "inside the arc");
        assert!(a(1, 1) < 255);
        assert_eq!(px[0..3], [255, 255, 255], "color untouched (straight alpha)");
    }

    #[test]
    fn crop_copies_the_rectangle() {
        let src: Vec<u8> = (0..4 * 3 * 4).map(|i| i as u8).collect();
        let c = crop(&src, 4, (1, 1, 2, 2));
        assert_eq!(c.len(), 16);
        assert_eq!(&c[0..4], &src[(4 + 1) * 4..(4 + 1) * 4 + 4]);
        assert_eq!(crop(&src, 4, (0, 0, 4, 3)), src);
    }
}

