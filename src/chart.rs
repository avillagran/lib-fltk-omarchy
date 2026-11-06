//! Small charts for dashboards and status views: [`BarChart`],
//! [`LineChart`], [`Meter`] (a usage bar with an "expected" mark) and
//! [`Ring`] (a progress ring).
//!
//! Shapes are drawn anti-aliased with [`gfx`](crate::gfx) at the screen's
//! pixel density and kept until the data, the size, the theme or the
//! highlighted item change, so a redraw from a neighbour costs one image
//! blit. Text uses the theme fonts. Colors are theme roles
//! ([`ColorRole`]), never hex, so `omarchy-theme-set` restyles them live.
//!
//! [`BarChart`] and [`LineChart`] are keyboard stops: Left/Right (mirrored
//! in right-to-left layouts) move the highlighted item, Home/End jump to
//! the ends, and a readout at the top shows the item's text (also while
//! hovering). Screen readers get one list item per bar or sample.
//!
//! ```no_run
//! use fltk_omarchy::chart::{Bar, BarChart, Meter, MeterValue, Ring};
//! use fltk_omarchy::State;
//! # let _app = fltk_omarchy::App::new("dev.example.chart");
//! let days = State::new(vec![
//!     Bar::new("Mon", 3.2, "Mon · $3.20"),
//!     Bar::new("Tue", 5.0, "Tue · $5.00"),
//! ]);
//! let usage = BarChart::new(&days);
//! let quota = Meter::new(&State::new(MeterValue { used: 0.62, expected: Some(0.5) }));
//! let context = Ring::new(&State::new(0.4));
//! ```

use crate::a11y::{self, Role};
use crate::gfx::{Canvas, Cap, Color, Join, Path, Stroke};
use crate::layout::{self, Mirror};
use crate::paint;
use crate::state::State;
use crate::text::ColorRole;
use crate::theme::{self, Rgb, Theme};
use fltk::{app, enums::*, image::RgbImage, prelude::*, widget::Widget};
use std::cell::RefCell;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

fn col(c: Rgb) -> Color {
    c.into()
}

/// A rendered picture of the shapes, kept while its key is the same.
#[derive(Default)]
struct Picture {
    key: u64,
    rgba: Vec<u8>,
    size: (i32, i32),
}

impl Picture {
    /// Draws the cached shapes at (x, y), rendering them first when `key`
    /// changed. `f` paints in widget units on a canvas scaled to the
    /// screen's pixel density.
    fn draw(&mut self, w: &Widget, key: u64, f: impl FnOnce(&mut Canvas)) {
        let (ww, wh) = (w.w().max(1), w.h().max(1));
        let unit = crate::native::pixels_per_unit(w).clamp(1.0, 4.0);
        let (pw, ph) = ((ww as f32 * unit).round() as i32, (wh as f32 * unit).round() as i32);
        let key = {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            (key, pw, ph, theme::generation()).hash(&mut h);
            h.finish()
        };
        if key != self.key || self.rgba.is_empty() {
            let mut c = Canvas::new(pw as usize, ph as usize);
            c.set_unit(unit);
            f(&mut c);
            c.to_rgba8(&mut self.rgba);
            self.key = key;
            self.size = (pw, ph);
        }
        // SAFETY: the buffer outlives the image, which is drawn right away.
        if let Ok(mut img) = unsafe { RgbImage::from_data(&self.rgba, self.size.0, self.size.1, ColorDepth::Rgba8) } {
            if self.size != (ww, wh) {
                img.scale(ww, wh, false, true);
            }
            img.draw(w.x(), w.y(), ww, wh);
        }
    }
}

fn hash_f64s(h: &mut impl Hasher, v: impl IntoIterator<Item = f64>) {
    for x in v {
        x.to_bits().hash(h);
    }
}

// ================================================================ BarChart

/// One bar: a short axis label, its value and the readout text.
#[derive(Debug, Clone, PartialEq)]
pub struct Bar {
    pub label: String,
    pub value: f64,
    pub tip: String,
}

impl Bar {
    pub fn new(label: &str, value: f64, tip: &str) -> Bar {
        Bar { label: label.into(), value, tip: tip.into() }
    }
}

struct Pick {
    /// Highlighted item (keyboard cursor or hover).
    cursor: Option<usize>,
    hover: Option<usize>,
    pic: Picture,
}

impl Pick {
    fn current(&self) -> Option<usize> {
        self.hover.or(self.cursor)
    }
}

/// Handles hover and keys for a chart of `count()` items laid out over the
/// plot width; `index_at(x)` maps a pointer x to an item.
fn interact(
    w: &mut Widget,
    st: Rc<RefCell<Pick>>,
    count: Rc<dyn Fn() -> usize>,
    index_at: Rc<dyn Fn(&Widget, i32) -> Option<usize>>,
    on_select: Rc<RefCell<Option<Box<dyn FnMut(usize)>>>>,
) {
    w.set_visible_focus();
    w.handle(move |w, ev| match ev {
        Event::Enter | Event::Move => {
            let i = index_at(w, app::event_x());
            if st.borrow().hover != i {
                st.borrow_mut().hover = i;
                w.redraw();
            }
            true
        }
        Event::Leave => {
            if st.borrow_mut().hover.take().is_some() {
                w.redraw();
            }
            true
        }
        Event::Push => {
            let _ = w.take_focus();
            if let Some(i) = index_at(w, app::event_x()) {
                st.borrow_mut().cursor = Some(i);
                w.redraw();
                if let Some(f) = on_select.borrow_mut().as_mut() {
                    f(i);
                }
            }
            true
        }
        Event::KeyDown => {
            let n = count();
            if n == 0 {
                return false;
            }
            let cur = st.borrow().cursor;
            let key = layout::logical_key(w, app::event_key());
            let next = match key {
                Key::Right => cur.map_or(0, |c| (c + 1).min(n - 1)),
                Key::Left => cur.map_or(n - 1, |c| c.saturating_sub(1)),
                Key::Home => 0,
                Key::End => n - 1,
                k if k == Key::Enter || k == Key::KPEnter || k == Key::from_char(' ') => {
                    let Some(c) = cur else { return false };
                    if let Some(f) = on_select.borrow_mut().as_mut() {
                        f(c);
                    }
                    return true;
                }
                _ => return false,
            };
            st.borrow_mut().cursor = Some(next);
            w.redraw();
            true
        }
        Event::Focus => {
            if st.borrow().cursor.is_none() && count() > 0 {
                st.borrow_mut().cursor = Some(count() - 1);
            }
            w.redraw();
            true
        }
        Event::Unfocus => {
            w.redraw();
            true
        }
        _ => false,
    });
}

/// Height of the readout line above the plot.
fn readout_h(t: &Theme) -> i32 {
    t.font.caption + 6
}

/// Height of the axis labels below the plot.
fn axis_h(t: &Theme) -> i32 {
    t.font.caption + 6
}

/// Vertical bars over time or categories (o3-code's usage per day). The
/// highlighted bar takes the full color, the others a lighter one.
#[derive(Clone)]
pub struct BarChart {
    inner: Widget,
    st: Rc<RefCell<Pick>>,
    data: State<Vec<Bar>>,
    on_select: Rc<RefCell<Option<Box<dyn FnMut(usize)>>>>,
    color: Rc<RefCell<ColorRole>>,
}
fltk::widget_extends!(BarChart, Widget, inner);

impl BarChart {
    pub fn new(data: &State<Vec<Bar>>) -> BarChart {
        let mut inner = Widget::default();
        inner.set_frame(FrameType::NoBox);
        inner.set_size(320, 140);
        let st = Rc::new(RefCell::new(Pick { cursor: None, hover: None, pic: Picture::default() }));
        let on_select: Rc<RefCell<Option<Box<dyn FnMut(usize)>>>> = Rc::new(RefCell::new(None));
        let color = Rc::new(RefCell::new(ColorRole::Accent));
        let d = data.clone();
        let count: Rc<dyn Fn() -> usize> = Rc::new(move || d.with(|v| v.len()));
        let d = data.clone();
        let index_at: Rc<dyn Fn(&Widget, i32) -> Option<usize>> = Rc::new(move |w, x| {
            let n = d.with(|v| v.len());
            if n == 0 {
                return None;
            }
            let x = Mirror::of(w).point(x) - w.x();
            let i = (x as f64 / (w.w().max(1) as f64 / n as f64)).floor();
            (0.0..n as f64).contains(&i).then_some(i as usize)
        });
        interact(&mut inner, st.clone(), count, index_at, on_select.clone());
        let (d, s, c) = (data.clone(), st.clone(), color.clone());
        inner.draw(move |w| {
            let bars = d.get();
            let mut s = s.borrow_mut();
            let cur = s.current().filter(|&i| i < bars.len());
            let ring = crate::focus::show_ring(w);
            theme::with(|t| {
                let (rh, ah) = (readout_h(t), axis_h(t));
                let plot_h = (w.h() - rh - ah).max(1);
                let max = bars.iter().map(|b| b.value).fold(0.0, f64::max).max(f64::MIN_POSITIVE);
                let n = bars.len().max(1);
                let rtl = layout::is_rtl(w);
                let role = *c.borrow();
                let mut h = std::collections::hash_map::DefaultHasher::new();
                hash_f64s(&mut h, bars.iter().map(|b| b.value));
                (cur, ring, rtl, role as u8, rh, ah).hash(&mut h);
                let full = role.rgb(t);
                let under = t.palette.background;
                let soft = full.over(under, 0.45);
                let base = t.palette.foreground.over(under, 0.15);
                let (ww, wh) = (w.w() as f32, w.h() as f32);
                s.pic.draw(w, h.finish(), |cv| {
                    let slot = ww / n as f32;
                    let gap = (slot * 0.25).clamp(1.0, 8.0);
                    let bw = (slot - gap).max(1.0);
                    for (i, b) in bars.iter().enumerate() {
                        let vis = if rtl { n - 1 - i } else { i };
                        let bh = (b.value.max(0.0) / max) as f32 * plot_h as f32;
                        if bh <= 0.0 {
                            continue;
                        }
                        let x = vis as f32 * slot + gap / 2.0;
                        let y = rh as f32 + plot_h as f32 - bh;
                        let r = (bw / 2.0).min(4.0).min(bh);
                        let p = Path::rounded_rect(x, y, bw, bh, [r, r, 0.0, 0.0]);
                        cv.fill(&p, col(if Some(i) == cur { full } else { soft }));
                    }
                    cv.fill(&Path::rect(0.0, (rh + plot_h) as f32 - 1.0, ww, 1.0), col(base));
                    if ring {
                        let p = Path::rounded_rect(0.5, 0.5, ww - 1.0, wh - 1.0, [t.radius as f32; 4]);
                        cv.stroke(&p, &Stroke::new(1.0), col(t.border(&t.controls.focus, under)));
                    }
                });
                let dim = t.palette.foreground.darker(1.4);
                // Axis: first, middle and last labels.
                let slot = w.w() as f64 / n as f64;
                let m = Mirror::of(w);
                let mut shown = vec![0, bars.len() / 2, bars.len().saturating_sub(1)];
                shown.dedup();
                for i in shown.into_iter().filter(|&i| i < bars.len()) {
                    let cx = w.x() + (slot * (i as f64 + 0.5)) as i32;
                    let lw = slot.max(60.0) as i32;
                    let x = m.x(cx - lw / 2, lw);
                    paint::text(&bars[i].label, x, w.y() + w.h() - ah, lw, ah, t.font.caption, false, dim, Align::Center);
                }
                if let Some(i) = cur {
                    let al = m.align(Align::Left);
                    paint::text_elided(&bars[i].tip, w.x(), w.y(), w.w(), rh, t.font.caption, true, t.palette.foreground, al);
                }
            });
        });
        let mut w2 = inner.clone();
        crate::state::bind_widget(&inner, data, false, move |_| w2.redraw());
        let (d, s) = (data.clone(), st.clone());
        a11y::describe(&inner, move |n| {
            let cur = s.borrow().cursor;
            n.role(Role::List).focusable();
            let bars = d.get();
            let total = bars.len();
            for (i, b) in bars.iter().enumerate() {
                n.child(i as u64, |c| {
                    let name = if b.tip.is_empty() { b.label.clone() } else { b.tip.clone() };
                    c.role(Role::ListItem).name(&name).selected(cur == Some(i)).position(i + 1, total);
                });
            }
        });
        BarChart { inner, st, data: data.clone(), on_select, color }
    }

    /// Color of the bars (default the accent).
    pub fn color(self, role: ColorRole) -> Self {
        *self.color.borrow_mut() = role;
        self.inner.clone().redraw();
        self
    }

    /// Called with the bar's index on click, Enter or Space.
    pub fn on_select(self, f: impl FnMut(usize) + 'static) -> Self {
        *self.on_select.borrow_mut() = Some(Box::new(f));
        self
    }

    /// The highlighted bar (keyboard cursor), if any.
    pub fn cursor(&self) -> Option<usize> {
        self.st.borrow().cursor
    }

    /// The bars shown.
    pub fn data(&self) -> State<Vec<Bar>> {
        self.data.clone()
    }
}

// =============================================================== LineChart

/// One series of a [`LineChart`].
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub name: String,
    pub values: Vec<f64>,
    pub color: ColorRole,
}

impl Line {
    pub fn new(name: &str, values: Vec<f64>, color: ColorRole) -> Line {
        Line { name: name.into(), values, color }
    }
}

type Readout = Rc<dyn Fn(usize, &[Line]) -> String>;

/// Lines over the same samples (o3-code's CPU and memory history), each
/// with a light area under it. The highlighted sample gets a guide and the
/// readout text (default: every series' name and value).
#[derive(Clone)]
pub struct LineChart {
    inner: Widget,
    st: Rc<RefCell<Pick>>,
    range: Rc<RefCell<Option<(f64, f64)>>>,
    readout: Rc<RefCell<Readout>>,
}
fltk::widget_extends!(LineChart, Widget, inner);

fn samples(lines: &[Line]) -> usize {
    lines.iter().map(|l| l.values.len()).max().unwrap_or(0)
}

impl LineChart {
    pub fn new(data: &State<Vec<Line>>) -> LineChart {
        let mut inner = Widget::default();
        inner.set_frame(FrameType::NoBox);
        inner.set_size(320, 140);
        let st = Rc::new(RefCell::new(Pick { cursor: None, hover: None, pic: Picture::default() }));
        let range: Rc<RefCell<Option<(f64, f64)>>> = Rc::new(RefCell::new(None));
        let readout: Rc<RefCell<Readout>> = Rc::new(RefCell::new(Rc::new(|i: usize, lines: &[Line]| {
            lines
                .iter()
                .filter_map(|l| l.values.get(i).map(|v| format!("{} {}", l.name, trim(*v))))
                .collect::<Vec<_>>()
                .join(" · ")
        })));
        let d = data.clone();
        let count: Rc<dyn Fn() -> usize> = Rc::new(move || d.with(|v| samples(v)));
        let d = data.clone();
        let index_at: Rc<dyn Fn(&Widget, i32) -> Option<usize>> = Rc::new(move |w, x| {
            let n = d.with(|v| samples(v));
            if n == 0 {
                return None;
            }
            let x = (Mirror::of(w).point(x) - w.x()) as f64;
            let step = w.w().max(1) as f64 / (n.max(2) - 1) as f64;
            Some(((x / step).round().max(0.0) as usize).min(n - 1))
        });
        interact(&mut inner, st.clone(), count, index_at, Rc::new(RefCell::new(None)));
        let (d, s, r, ro) = (data.clone(), st.clone(), range.clone(), readout.clone());
        inner.draw(move |w| {
            let lines = d.get();
            let n = samples(&lines);
            let mut s = s.borrow_mut();
            let cur = s.current().filter(|&i| i < n);
            let ring = crate::focus::show_ring(w);
            theme::with(|t| {
                let rh = readout_h(t);
                let plot_h = (w.h() - rh - 2).max(1) as f32;
                let (lo, hi) = r.borrow().unwrap_or_else(|| {
                    let it = lines.iter().flat_map(|l| l.values.iter().copied());
                    let hi = it.clone().fold(f64::MIN, f64::max);
                    let lo = it.fold(f64::MAX, f64::min).min(0.0);
                    (lo, if hi > lo { hi } else { lo + 1.0 })
                });
                let span = (hi - lo).max(f64::MIN_POSITIVE);
                let rtl = layout::is_rtl(w);
                let mut h = std::collections::hash_map::DefaultHasher::new();
                for l in &lines {
                    hash_f64s(&mut h, l.values.iter().copied());
                    (l.color as u8).hash(&mut h);
                }
                (cur, ring, rtl, lo.to_bits(), hi.to_bits()).hash(&mut h);
                let under = t.palette.background;
                let (ww, wh) = (w.w() as f32, w.h() as f32);
                let step = ww / (n.max(2) - 1) as f32;
                let px = |i: usize| {
                    let x = i as f32 * step;
                    if rtl { ww - x } else { x }
                };
                let py = |v: f64| rh as f32 + 1.0 + plot_h - ((v - lo) / span) as f32 * plot_h;
                s.pic.draw(w, h.finish(), |cv| {
                    cv.fill(&Path::rect(0.0, rh as f32 + plot_h + 1.0, ww, 1.0), col(t.palette.foreground.over(under, 0.15)));
                    for l in &lines {
                        if l.values.is_empty() {
                            continue;
                        }
                        let c = l.color.rgb(t);
                        let mut p = Path::new().move_to(px(0), py(l.values[0]));
                        for (i, v) in l.values.iter().enumerate().skip(1) {
                            p = p.line_to(px(i), py(*v));
                        }
                        let last = l.values.len() - 1;
                        let area = p.clone().line_to(px(last), py(lo)).line_to(px(0), py(lo)).close();
                        cv.fill(&area, col(c).with_alpha(0.12));
                        cv.stroke(&p, &Stroke::new(1.5).join(Join::Round).cap(Cap::Round), col(c));
                    }
                    if let Some(i) = cur {
                        let x = px(i);
                        cv.fill(&Path::rect(x - 0.5, rh as f32, 1.0, plot_h + 1.0), col(t.palette.foreground.over(under, 0.35)));
                        for l in &lines {
                            if let Some(v) = l.values.get(i) {
                                cv.fill(&Path::circle(x, py(*v), 3.0), col(l.color.rgb(t)));
                            }
                        }
                    }
                    if ring {
                        let p = Path::rounded_rect(0.5, 0.5, ww - 1.0, wh - 1.0, [t.radius as f32; 4]);
                        cv.stroke(&p, &Stroke::new(1.0), col(t.border(&t.controls.focus, under)));
                    }
                });
                if let Some(i) = cur {
                    let text = (ro.borrow())(i, &lines);
                    let al = Mirror::of(w).align(Align::Left);
                    paint::text_elided(&text, w.x(), w.y(), w.w(), rh, t.font.caption, true, t.palette.foreground, al);
                }
            });
        });
        let mut w2 = inner.clone();
        crate::state::bind_widget(&inner, data, false, move |_| w2.redraw());
        let (d, s, ro) = (data.clone(), st.clone(), readout.clone());
        a11y::describe(&inner, move |nd| {
            let lines = d.get();
            let cur = s.borrow().cursor;
            nd.role(Role::List).focusable();
            // Samples near the cursor only: a long history is not read out
            // as hundreds of nodes.
            let n = samples(&lines);
            let c = cur.unwrap_or(n.saturating_sub(1));
            for i in c.saturating_sub(8)..(c + 9).min(n) {
                nd.child(i as u64, |ch| {
                    ch.role(Role::ListItem).name(&(ro.borrow())(i, &lines)).selected(cur == Some(i)).position(i + 1, n);
                });
            }
        });
        LineChart { inner, st, range, readout }
    }

    /// Fixed value range (default: from the data, starting at 0 or below).
    pub fn range(self, min: f64, max: f64) -> Self {
        *self.range.borrow_mut() = Some((min, max.max(min + f64::MIN_POSITIVE)));
        self.inner.clone().redraw();
        self
    }

    /// Text of the readout and of the screen-reader item for sample `i`.
    pub fn readout(self, f: impl Fn(usize, &[Line]) -> String + 'static) -> Self {
        *self.readout.borrow_mut() = Rc::new(f);
        self
    }

    /// The highlighted sample (keyboard cursor), if any.
    pub fn cursor(&self) -> Option<usize> {
        self.st.borrow().cursor
    }
}

fn trim(v: f64) -> String {
    let s = format!("{v:.1}");
    s.strip_suffix(".0").map(str::to_string).unwrap_or(s)
}

// =================================================================== Meter

/// A [`Meter`]'s value: the used fraction and, optionally, where it is
/// expected to be by now (o3-code's quota windows: the elapsed part of the
/// period).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct MeterValue {
    /// 0..1 (values above 1 show a full bar).
    pub used: f64,
    pub expected: Option<f64>,
}

/// A horizontal usage bar, optionally in segments, with a mark at the
/// expected level. Ahead of the expectation it takes the warning color
/// (yellow), at 100 % the urgent one.
#[derive(Clone)]
pub struct Meter {
    inner: Widget,
    segments: Rc<RefCell<usize>>,
}
fltk::widget_extends!(Meter, Widget, inner);

impl Meter {
    pub fn new(value: &State<MeterValue>) -> Meter {
        let mut inner = Widget::default();
        inner.set_frame(FrameType::NoBox);
        inner.set_size(200, 8);
        let segments = Rc::new(RefCell::new(0usize));
        let pic = Rc::new(RefCell::new(Picture::default()));
        let (v, sg) = (value.clone(), segments.clone());
        inner.draw(move |w| {
            let MeterValue { used, expected } = v.get();
            let segs = *sg.borrow();
            theme::with(|t| {
                let under = t.palette.background;
                let tone = if used >= 1.0 {
                    t.palette.urgent
                } else if expected.is_some_and(|e| used > e) {
                    t.palette.yellow
                } else {
                    t.palette.accent
                };
                let track = t.palette.foreground.over(under, 0.12);
                let mark = t.palette.foreground.over(under, 0.7);
                let rtl = layout::is_rtl(w);
                let mut h = std::collections::hash_map::DefaultHasher::new();
                hash_f64s(&mut h, [used, expected.unwrap_or(-1.0)]);
                (segs, rtl).hash(&mut h);
                let (ww, wh) = (w.w() as f32, w.h() as f32);
                let mirror = |x: f32, width: f32| if rtl { ww - x - width } else { x };
                let used = used.clamp(0.0, 1.0) as f32;
                pic.borrow_mut().draw(w, h.finish(), |cv| {
                    let r = wh / 2.0;
                    if segs >= 2 {
                        let gap = 2.0;
                        let sw = (ww - gap * (segs - 1) as f32) / segs as f32;
                        for i in 0..segs {
                            let x = i as f32 * (sw + gap);
                            let fill = ((used * segs as f32) - i as f32).clamp(0.0, 1.0);
                            let rr = r.min(sw / 2.0);
                            cv.fill(&Path::rounded_rect(mirror(x, sw), 0.0, sw, wh, [rr; 4]), col(track));
                            if fill > 0.0 {
                                let fw = sw * fill;
                                cv.fill(&Path::rounded_rect(mirror(x, fw), 0.0, fw, wh, [rr.min(fw / 2.0); 4]), col(tone));
                            }
                        }
                    } else {
                        cv.fill(&Path::rounded_rect(0.0, 0.0, ww, wh, [r; 4]), col(track));
                        let fw = ww * used;
                        if fw > 0.0 {
                            cv.fill(&Path::rounded_rect(mirror(0.0, fw), 0.0, fw, wh, [r.min(fw / 2.0); 4]), col(tone));
                        }
                    }
                    if let Some(e) = expected {
                        let x = mirror(ww * e.clamp(0.0, 1.0) as f32 - 1.0, 2.0).clamp(0.0, ww - 2.0);
                        cv.fill(&Path::rect(x, -1.0, 2.0, wh + 2.0), col(mark));
                    }
                });
            });
        });
        let mut w2 = inner.clone();
        crate::state::bind_widget(&inner, value, false, move |_| w2.redraw());
        let v = value.clone();
        a11y::describe(&inner, move |n| {
            let x = v.get();
            n.role(Role::ProgressIndicator).numeric(x.used.clamp(0.0, 1.0), 0.0, 1.0, 0.0);
        });
        Meter { inner, segments }
    }

    /// Draws `n` separate segments (0 or 1: one continuous bar).
    pub fn segments(self, n: usize) -> Self {
        *self.segments.borrow_mut() = n;
        self.inner.clone().redraw();
        self
    }
}

// ==================================================================== Ring

/// A progress ring (o3-code's context window meter): the track in a faint
/// color and the value as an arc from the top, clockwise (counter-clockwise
/// in right-to-left layouts).
#[derive(Clone)]
pub struct Ring {
    inner: Widget,
    style: Rc<RefCell<(ColorRole, f32)>>,
}
fltk::widget_extends!(Ring, Widget, inner);

impl Ring {
    pub fn new(value: &State<f64>) -> Ring {
        let mut inner = Widget::default();
        inner.set_frame(FrameType::NoBox);
        inner.set_size(18, 18);
        let style = Rc::new(RefCell::new((ColorRole::Accent, 2.5f32)));
        let pic = Rc::new(RefCell::new(Picture::default()));
        let (v, st) = (value.clone(), style.clone());
        inner.draw(move |w| {
            let value = v.get().clamp(0.0, 1.0) as f32;
            let (role, width) = *st.borrow();
            theme::with(|t| {
                let rtl = layout::is_rtl(w);
                let mut h = std::collections::hash_map::DefaultHasher::new();
                (value.to_bits(), role as u8, width.to_bits(), rtl).hash(&mut h);
                let under = t.palette.background;
                let (track, c) = (t.palette.foreground.over(under, 0.15), role.rgb(t));
                let (ww, wh) = (w.w() as f32, w.h() as f32);
                pic.borrow_mut().draw(w, h.finish(), |cv| {
                    let (cx, cy) = (ww / 2.0, wh / 2.0);
                    let r = (ww.min(wh) - width) / 2.0;
                    let s = Stroke::new(width).cap(Cap::Round);
                    cv.stroke(&Path::circle(cx, cy, r), &Stroke::new(width), col(track));
                    if value > 0.0 {
                        let start = -std::f32::consts::FRAC_PI_2;
                        let sweep = value * std::f32::consts::TAU * if rtl { -1.0 } else { 1.0 };
                        let p = Path::new().arc(cx, cy, r, start, sweep);
                        cv.stroke(&p, &s, col(c));
                    }
                });
            });
        });
        let mut w2 = inner.clone();
        crate::state::bind_widget(&inner, value, false, move |_| w2.redraw());
        let v = value.clone();
        a11y::describe(&inner, move |n| {
            n.role(Role::ProgressIndicator).numeric(v.get().clamp(0.0, 1.0), 0.0, 1.0, 0.0);
        });
        Ring { inner, style }
    }

    /// Arc color (default the accent).
    pub fn color(self, role: ColorRole) -> Self {
        self.style.borrow_mut().0 = role;
        self.inner.clone().redraw();
        self
    }

    /// Line width in pixels (default 2.5).
    pub fn thickness(self, px: f32) -> Self {
        self.style.borrow_mut().1 = px.max(0.5);
        self.inner.clone().redraw();
        self
    }
}
