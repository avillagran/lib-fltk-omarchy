//! `View`: a widget that shows a [`Canvas`] painted by your closure and/or
//! a CPU [`Shader`], optionally animated, with shape hit-testing.

use super::canvas::{Canvas, Color};
use super::path::Path;
use super::raster::Stroke;
use super::shader::{Shader, Uniforms};
use crate::theme;
use fltk::{app, enums::*, image::RgbImage, prelude::*, widget::Widget};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

type PaintFn = Box<dyn FnMut(&mut Canvas, &Uniforms)>;

/// The shape that receives mouse events (outside it, events go to what
/// is below, as if the view were not there).
#[derive(Debug, Clone, PartialEq)]
pub enum Hit {
    /// Inside the filled path.
    Fill(Path),
    /// Within `width / 2` of the outline.
    Outline(Path, f32),
}

impl Hit {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        match self {
            Hit::Fill(p) => p.contains(x, y),
            Hit::Outline(p, w) => p.near_outline(x, y, *w),
        }
    }
}

/// Render counters, for budgets and measurements.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ViewStats {
    /// Completed frames.
    pub frames: u64,
    /// Draws that ran out of budget and continued on the next draw.
    pub partial_draws: u64,
    /// CPU time of the last completed frame (all its draws), in ms.
    pub last_frame_ms: f32,
    /// Resolution actually rendered (≤ widget size, see `max_area`).
    pub render_size: (usize, usize),
    /// Animation ticks skipped because the view was hidden.
    pub paused_ticks: u64,
    /// Bytes held by the view's buffers.
    pub memory: usize,
    /// Last draw: converting the frame for fltk and drawing it, in ms.
    pub last_blit_ms: f32,
}

const TILE: usize = 32;

struct State {
    present: Canvas,
    work: Canvas,
    upload: Vec<u8>,
    paint: Option<PaintFn>,
    post: Option<PaintFn>,
    shader: Option<Box<dyn Shader>>,
    fps: u32,
    params: [f32; 4],
    started: Instant,
    frozen: Option<f32>,
    frame: u64,
    pointer: Option<(f32, f32)>,
    max_area: usize,
    budget: Duration,
    next_tile: usize,
    in_progress: Option<(Uniforms, Duration)>,
    dirty: bool,
    hit: Option<Hit>,
    pressed: bool,
    click: Option<Box<dyn FnMut()>>,
    stats: ViewStats,
    timer: bool,
}

/// A software-rendered drawing surface:
///
/// ```no_run
/// use fltk_omarchy::gfx::{self, Path, Paint, Stroke, View};
/// let card = View::new(240, 120).paint(|c, u| {
///     let shape = Path::rounded_rect(8.0, 8.0, 224.0, 104.0, [16.0, 16.0, 4.0, 4.0]);
///     c.shadow(&shape, 0.0, 4.0, 8, u.foreground.with_alpha(0.35));
///     c.fill(&shape, Paint::linear((0.0, 0.0), (240.0, 0.0), &[(0.0, u.accent), (1.0, u.background)]));
///     c.stroke(&shape, &Stroke::new(1.5), u.foreground.with_alpha(0.4));
/// });
/// let glow = View::new(240, 120).shader(gfx::shaders::accent_glow()).animate(30);
/// ```
///
/// - `paint(f)` draws vector content (redrawn on theme change or `invalidate()`).
/// - `shader(s)` fills every pixel with a CPU shader; `post(f)` runs on
///   the shader output (blur a region, draw on top) before it is shown.
/// - `animate(fps)` redraws on a timer; ticks are skipped while hidden.
/// - Limits: `max_area(px)` (default 160 000 px; larger views render at a
///   lower resolution and are scaled up) and `budget_ms(ms)` per draw
///   (default 6 ms; a shader frame that needs more continues on the next
///   draw, keeping the last complete frame on screen).
#[derive(Clone)]
pub struct View {
    widget: Widget,
    st: Rc<RefCell<State>>,
}
fltk::widget_extends!(View, Widget, widget);

impl View {
    pub fn new(w: i32, h: i32) -> View {
        let mut widget = Widget::default().with_size(w, h);
        widget.set_frame(FrameType::NoBox);
        widget.clear_visible_focus();
        let st = Rc::new(RefCell::new(State {
            present: Canvas::new(0, 0),
            work: Canvas::new(0, 0),
            upload: Vec::new(),
            paint: None,
            post: None,
            shader: None,
            fps: 0,
            params: [0.0; 4],
            started: Instant::now(),
            frozen: None,
            frame: 0,
            pointer: None,
            max_area: 160_000,
            budget: Duration::from_millis(6),
            next_tile: 0,
            in_progress: None,
            dirty: true,
            hit: None,
            pressed: false,
            click: None,
            stats: ViewStats::default(),
            timer: false,
        }));
        let s2 = st.clone();
        widget.draw(move |w| draw(w, &s2));
        let s3 = st.clone();
        widget.handle(move |w, ev| handle(w, ev, &s3));
        let s4 = st.clone();
        let mut w2 = widget.clone();
        theme::on_change_while(&widget, move |_| {
            s4.borrow_mut().dirty = true;
            w2.redraw();
        });
        View { widget, st }
    }

    /// Vector content: `f(canvas, uniforms)`, in widget pixels.
    pub fn paint(self, f: impl FnMut(&mut Canvas, &Uniforms) + 'static) -> Self {
        self.st.borrow_mut().paint = Some(Box::new(f));
        self.invalidate();
        self
    }

    /// Fills every pixel with `shader` (the view becomes opaque over the
    /// theme background); `paint` draws on top of it.
    pub fn shader(self, shader: impl Shader + 'static) -> Self {
        self.st.borrow_mut().shader = Some(Box::new(shader));
        self.invalidate();
        self
    }

    /// Runs on the shader output before it is shown (e.g. `c.blur(..)`).
    pub fn post(self, f: impl FnMut(&mut Canvas, &Uniforms) + 'static) -> Self {
        self.st.borrow_mut().post = Some(Box::new(f));
        self.invalidate();
        self
    }

    /// Redraws `fps` times per second (0 stops). Hidden views skip ticks.
    pub fn animate(self, fps: u32) -> Self {
        self.st.borrow_mut().fps = fps.min(120);
        self.start_timer();
        self
    }

    /// Largest number of pixels rendered per frame (minimum 1024).
    pub fn max_area(self, px: usize) -> Self {
        self.st.borrow_mut().max_area = px.max(1024);
        self.invalidate();
        self
    }

    /// CPU time per draw before a shader frame continues on the next draw.
    pub fn budget_ms(self, ms: f32) -> Self {
        self.st.borrow_mut().budget = Duration::from_secs_f32(ms.max(0.5) / 1000.0);
        self
    }

    /// Free shader parameters (`Uniforms::params`).
    pub fn set_params(&self, p: [f32; 4]) {
        self.st.borrow_mut().params = p;
        self.invalidate();
    }

    /// Freezes `Uniforms::time` (screenshots, tests); None follows the clock.
    pub fn set_time(&self, t: Option<f32>) {
        self.st.borrow_mut().frozen = t;
        self.invalidate();
    }

    /// Only this shape gets mouse events; with `on_click` it is also a
    /// keyboard control (Tab, Enter/Space) with a focus ring along the shape.
    pub fn hit_shape(self, hit: Hit) -> Self {
        self.st.borrow_mut().hit = Some(hit);
        self
    }

    pub fn on_click(mut self, f: impl FnMut() + 'static) -> Self {
        self.st.borrow_mut().click = Some(Box::new(f));
        self.widget.set_visible_focus();
        self
    }

    /// Asks for a new frame.
    pub fn invalidate(&self) {
        self.st.borrow_mut().dirty = true;
        self.widget.clone().redraw();
    }

    /// Finishes the pending frame now, ignoring the budget (tests, snapshots).
    pub fn render_now(&self) {
        let (w, h) = (self.widget.w(), self.widget.h());
        render(&self.st, w, h, Duration::MAX);
    }

    pub fn stats(&self) -> ViewStats {
        self.st.borrow().stats
    }

    /// The last complete frame (render resolution).
    pub fn with_canvas<R>(&self, f: impl FnOnce(&Canvas) -> R) -> R {
        f(&self.st.borrow().present)
    }

    fn start_timer(&self) {
        if std::mem::replace(&mut self.st.borrow_mut().timer, true) {
            return;
        }
        let fps = self.st.borrow().fps;
        if fps == 0 {
            self.st.borrow_mut().timer = false;
            return;
        }
        let (st, w) = (Rc::downgrade(&self.st), self.widget.clone());
        app::add_timeout3(1.0 / fps as f64, move |h| {
            let Some(st) = st.upgrade() else { return };
            if w.was_deleted() {
                return;
            }
            let fps = st.borrow().fps;
            if fps == 0 {
                st.borrow_mut().timer = false;
                return;
            }
            let shown = w.visible_r() && w.window().is_some_and(|win| win.shown());
            if shown {
                st.borrow_mut().dirty = true;
                w.clone().redraw();
                app::repeat_timeout3(1.0 / fps as f64, h);
            } else {
                st.borrow_mut().stats.paused_ticks += 1;
                app::repeat_timeout3(0.5, h);
            }
        });
    }
}

fn render_size(w: i32, h: i32, max_area: usize) -> (usize, usize) {
    let (w, h) = (w.max(1) as usize, h.max(1) as usize);
    let area = w * h;
    if area <= max_area {
        return (w, h);
    }
    let s = (max_area as f64 / area as f64).sqrt();
    (((w as f64 * s) as usize).max(1), ((h as f64 * s) as usize).max(1))
}

/// Advances the frame; returns true when a frame is complete.
fn render(st: &Rc<RefCell<State>>, w: i32, h: i32, budget: Duration) -> bool {
    let mut s = st.borrow_mut();
    let (rw, rh) = render_size(w, h, s.max_area);
    if (s.present.width(), s.present.height()) != (rw, rh) {
        s.present.resize(rw, rh);
        s.work.resize(rw, rh);
        if s.upload.capacity() > 2 * rw * rh * 4 + 4096 {
            s.upload = Vec::new();
        }
        let unit = rw as f32 / w.max(1) as f32;
        s.present.set_unit(unit);
        s.work.set_unit(unit);
        s.in_progress = None;
        s.dirty = true;
    }
    if !s.dirty && s.in_progress.is_none() {
        return true;
    }
    let t0 = Instant::now();
    if s.in_progress.is_none() {
        let mut u = theme::with(|t| Uniforms::from_theme(t, (w as f32, h as f32)));
        u.time = s.frozen.unwrap_or_else(|| s.started.elapsed().as_secs_f32());
        u.frame = s.frame;
        u.pointer = s.pointer;
        u.params = s.params;
        s.frame += 1;
        s.dirty = false;
        s.next_tile = 0;
        s.work.clear(Color::TRANSPARENT);
        s.in_progress = Some((u, Duration::ZERO));
    }
    let (u, spent) = s.in_progress.expect("frame");
    let first_frame = s.stats.frames == 0;
    // Shader tiles, until done or out of budget (the first frame always completes).
    if let Some(shader) = s.shader.take() {
        let (cols, rows) = (rw.div_ceil(TILE), rh.div_ceil(TILE));
        while s.next_tile < cols * rows {
            let t = s.next_tile;
            let rect = ((t % cols) * TILE, (t / cols) * TILE, TILE, TILE);
            s.work.shade(rect, shader.as_ref(), &u);
            s.next_tile += 1;
            if !first_frame && t0.elapsed() >= budget {
                break;
            }
        }
        s.shader = Some(shader);
        if s.next_tile < cols * rows {
            s.in_progress = Some((u, spent + t0.elapsed()));
            s.stats.partial_draws += 1;
            return false;
        }
    }
    let State { work, post, paint, .. } = &mut *s;
    // Shader output is in widget pixels scaled to the render size: draw
    // vector content in widget pixels too.
    if let Some(f) = post.as_mut() {
        f(work, &u);
    }
    if let Some(f) = paint.as_mut() {
        f(work, &u);
    }
    let s = &mut *s;
    std::mem::swap(&mut s.present, &mut s.work);
    s.in_progress = None;
    s.stats.frames += 1;
    s.stats.last_frame_ms = (spent + t0.elapsed()).as_secs_f32() * 1000.0;
    s.stats.render_size = (rw, rh);
    s.stats.memory = s.present.memory() + s.work.memory() + s.upload.capacity();
    true
}

fn draw(w: &mut Widget, st: &Rc<RefCell<State>>) {
    let budget = st.borrow().budget;
    let done = render(st, w.w(), w.h(), budget);
    if !done {
        let mut w2 = w.clone();
        app::add_timeout3(0.0, move |_| if !w2.was_deleted() { w2.redraw() });
    }
    let t_blit = Instant::now();
    let mut s = st.borrow_mut();
    let s = &mut *s;
    let (rw, rh) = (s.present.width() as i32, s.present.height() as i32);
    if rw == 0 || rh == 0 {
        return;
    }
    let opaque = s.shader.is_some();
    // SAFETY: `img` borrows `s.upload` and is dropped at the end of this
    // function; the buffer is not touched while it lives.
    let img = unsafe { if opaque {
        let bg: Color = theme::with(|t| t.palette.background.into());
        s.present.to_rgb8_over(bg, &mut s.upload);
        RgbImage::from_data(&s.upload, rw, rh, ColorDepth::Rgb8)
    } else {
        s.present.to_rgba8(&mut s.upload);
        RgbImage::from_data(&s.upload, rw, rh, ColorDepth::Rgba8)
    } };
    if let Ok(mut img) = img {
        if (rw, rh) != (w.w(), w.h()) {
            img.scale(w.w(), w.h(), false, true);
        }
        img.draw(w.x(), w.y(), w.w(), w.h());
    }
    s.stats.last_blit_ms = t_blit.elapsed().as_secs_f32() * 1000.0;
    // Keyboard focus ring along the hit shape (or the bounds).
    if crate::focus::show_ring(w) {
        let (ring, halo) = theme::with(|t| (t.border(&t.controls.focus, t.palette.background), t.palette.background));
        let mut c = Canvas::new(w.w().max(1) as usize, w.h().max(1) as usize);
        let shape = match &s.hit {
            Some(Hit::Fill(p)) | Some(Hit::Outline(p, _)) => p.clone(),
            None => Path::rect(1.0, 1.0, w.w() as f32 - 2.0, w.h() as f32 - 2.0),
        };
        // A background halo under the ring keeps it visible over any fill.
        c.stroke(&shape, &Stroke::new(6.0), Color::from(halo));
        c.stroke(&shape, &Stroke::new(2.5), ring);
        let mut buf = Vec::new();
        c.to_rgba8(&mut buf);
        if let Ok(mut img) = RgbImage::new(&buf, w.w(), w.h(), ColorDepth::Rgba8) {
            img.draw(w.x(), w.y(), w.w(), w.h());
        }
    }
}

fn handle(w: &mut Widget, ev: Event, st: &Rc<RefCell<State>>) -> bool {
    let local = || ((app::event_x() - w.x()) as f32 + 0.5, (app::event_y() - w.y()) as f32 + 0.5);
    let inside = |st: &State| st.hit.as_ref().map_or(true, |h| h.contains(local().0, local().1));
    match ev {
        Event::Enter | Event::Move => {
            let mut s = st.borrow_mut();
            s.pointer = Some(local());
            if s.shader.is_some() && s.fps == 0 {
                s.dirty = true;
                drop(s);
                w.redraw();
            }
            true
        }
        Event::Leave => {
            st.borrow_mut().pointer = None;
            false
        }
        Event::Push => {
            let s = st.borrow();
            if s.hit.is_some() && !inside(&s) {
                return false; // let the widget below get it
            }
            if s.click.is_none() {
                return false;
            }
            drop(s);
            st.borrow_mut().pressed = true;
            let _ = w.take_focus();
            true
        }
        Event::Released => {
            let ok = {
                let mut s = st.borrow_mut();
                std::mem::replace(&mut s.pressed, false) && inside(&s)
            };
            if ok {
                fire(st);
            }
            ok
        }
        Event::Focus | Event::Unfocus => {
            let focusable = st.borrow().click.is_some();
            if focusable {
                w.redraw();
            }
            focusable
        }
        Event::KeyDown if crate::focus::is_focused(w) => {
            let k = app::event_key();
            if k == Key::Enter || k == Key::KPEnter || k == Key::from_char(' ') {
                fire(st);
                return true;
            }
            false
        }
        _ => false,
    }
}

fn fire(st: &Rc<RefCell<State>>) {
    let f = st.borrow_mut().click.take();
    if let Some(mut f) = f {
        f();
        let mut s = st.borrow_mut();
        if s.click.is_none() {
            s.click = Some(f);
        }
    }
}
