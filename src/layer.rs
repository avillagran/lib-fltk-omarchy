//! Layers: widgets drawn offscreen, then shown with real opacity or run
//! through an effect (o3-code's `T3Reveal` nebula/warp/fold masks and the
//! row snapshots `T3RowFlight` cross-fades).
//!
//! [`capture`] takes a picture of any widget as a [`gfx::Canvas`](crate::gfx::Canvas).
//! A [`Layer`] is a group whose children are drawn normally while it is
//! fully opaque and has no effect (no cost), and otherwise drawn
//! offscreen, passed to the effect, and blended over what is under it.
//!
//! ```no_run
//! use fltk::prelude::*;
//! use fltk_omarchy::{layer::Layer, gfx::Path, widgets::Button};
//! # let _app = fltk_omarchy::App::new("dev.example.layer");
//! let layer = Layer::new(0, 0, 200, 40);
//! let _b = Button::new("Hello");
//! layer.end();
//! layer.set_opacity(0.5);
//! // Reveal from the left: cover the part not revealed yet.
//! layer.effect(|c, u| {
//!     let w = c.width() as f32 / c.unit();
//!     c.fill(&Path::rect(w * u.params[0], 0.0, w, c.height() as f32), u.background);
//! });
//! ```
//!
//! The picture is taken at the widget's size in FLTK units (sharp at 1×,
//! softer on a 2× screen while the layer is translucent or has an effect).
//! Blending assumes the layer sits on its parent's color (true for toolkit
//! surfaces); over a picture or gradient the children are blended with the
//! parent's flat color.

use crate::gfx::{Canvas, Uniforms};
use crate::paint;
use crate::theme::{self, Rgb};
use fltk::{enums::*, group::Group, image::RgbImage, prelude::*};
use std::cell::RefCell;
use std::rc::Rc;

/// The color under `w`: the nearest parent with a box, else the theme
/// background.
fn under<W: WidgetExt>(w: &W) -> Rgb {
    let mut cur = w.parent();
    while let Some(g) = cur {
        if g.frame() != FrameType::NoBox || g.as_window().is_some() {
            let (r, gg, b) = g.color().to_rgb();
            return Rgb(((r as u32) << 16) | ((gg as u32) << 8) | b as u32);
        }
        cur = g.parent();
    }
    theme::with(|t| t.palette.background)
}

/// A picture of `w` as it is now, drawn offscreen over the color under it.
/// For a group, its children are drawn too. `None` for an empty widget.
pub fn capture<W: WidgetExt>(w: &W) -> Option<Canvas> {
    let (ww, hh) = (w.w(), w.h());
    if ww <= 0 || hh <= 0 {
        return None;
    }
    let bg = under(w);
    picture(ww, hh, bg, |surf| surf.draw(w, 0, 0))
}

fn picture(ww: i32, hh: i32, bg: Rgb, f: impl FnOnce(&fltk::surface::ImageSurface)) -> Option<Canvas> {
    use fltk::surface::ImageSurface;
    let surf = ImageSurface::new(ww, hh, false);
    ImageSurface::push_current(&surf);
    paint::fill(0, 0, ww, hh, 0, bg);
    f(&surf);
    let img = surf.image();
    ImageSurface::pop_current();
    let img = img?;
    let (dw, dh) = (img.data_w() as usize, img.data_h() as usize);
    let data = img.to_rgb_data();
    match img.depth() {
        ColorDepth::Rgb8 => Some(Canvas::from_rgb8(dw, dh, &data)),
        ColorDepth::Rgba8 => Some(Canvas::from_rgba8(dw, dh, &data)),
        _ => None,
    }
}

type Effect = Box<dyn FnMut(&mut Canvas, &Uniforms)>;

struct St {
    opacity: f32,
    effect: Option<Effect>,
    params: [f32; 4],
    upload: Vec<u8>,
}

/// A group drawn offscreen while translucent or with an effect. See the
/// [module docs](self).
#[derive(Clone)]
pub struct Layer {
    group: Group,
    st: Rc<RefCell<St>>,
}
fltk::widget_extends!(Layer, Group, group);

impl Layer {
    /// A layer at (x, y) of size w × h; widgets created next go inside it
    /// until [`GroupExt::end`].
    pub fn new(x: i32, y: i32, w: i32, h: i32) -> Layer {
        let mut group = Group::new(x, y, w, h, None);
        group.set_frame(FrameType::NoBox);
        let st = Rc::new(RefCell::new(St { opacity: 1.0, effect: None, params: [0.0; 4], upload: Vec::new() }));
        let s = st.clone();
        group.draw(move |g| {
            let mut s = s.borrow_mut();
            if s.opacity >= 0.999 && s.effect.is_none() {
                drop(s);
                g.draw_children();
                return;
            }
            if s.opacity <= 0.001 {
                return;
            }
            let (gx, gy) = (g.x(), g.y());
            let bg = under(g);
            let pic = picture(g.w(), g.h(), bg, |surf| {
                for i in 0..g.children() {
                    if let Some(c) = g.child(i) {
                        if c.visible() {
                            surf.draw(&c, c.x() - gx, c.y() - gy);
                        }
                    }
                }
            });
            let Some(mut c) = pic else { return };
            let params = s.params;
            if let Some(f) = s.effect.as_mut() {
                let mut u = theme::with(|t| Uniforms::from_theme(t, (g.w() as f32, g.h() as f32)));
                u.params = params;
                f(&mut c, &u);
            }
            let a = s.opacity;
            let mut buf = std::mem::take(&mut s.upload);
            c.to_rgba8(&mut buf);
            if a < 0.999 {
                for px in buf.chunks_exact_mut(4) {
                    px[3] = (px[3] as f32 * a).round() as u8;
                }
            }
            let (cw, ch) = (c.width() as i32, c.height() as i32);
            // SAFETY: the buffer outlives the image, drawn right away.
            if let Ok(mut img) = unsafe { RgbImage::from_data(&buf, cw, ch, ColorDepth::Rgba8) } {
                if (cw, ch) != (g.w(), g.h()) {
                    img.scale(g.w(), g.h(), false, true);
                }
                img.draw(gx, gy, g.w(), g.h());
            }
            s.upload = buf;
        });
        // cfltk runs FLTK's own draw (the children, opaque) before the
        // closure unless told not to.
        group.super_draw(false);
        Layer { group, st }
    }

    /// Opacity of the whole layer, 0..1 (1: drawn normally).
    pub fn set_opacity(&self, a: f32) {
        self.st.borrow_mut().opacity = a.clamp(0.0, 1.0);
        self.group.clone().redraw();
    }

    pub fn opacity(&self) -> f32 {
        self.st.borrow().opacity
    }

    /// Runs `f` on the offscreen picture before it is shown (paint over it
    /// with the theme colors in `u`, mask parts with the background, read
    /// its pixels). `u.params` are [`Layer::set_params`].
    pub fn effect(&self, f: impl FnMut(&mut Canvas, &Uniforms) + 'static) {
        self.st.borrow_mut().effect = Some(Box::new(f));
        self.group.clone().redraw();
    }

    /// Back to drawing normally (when opaque).
    pub fn clear_effect(&self) {
        self.st.borrow_mut().effect = None;
        self.group.clone().redraw();
    }

    /// Four values for the effect (animation progress…), redraws.
    pub fn set_params(&self, p: [f32; 4]) {
        self.st.borrow_mut().params = p;
        self.group.clone().redraw();
    }
}

/// Mixes two pictures (`t` = 0: `a`, 1: `b`) over their common size: a
/// cross-fade between the old and the new state of a row or a view.
pub fn cross_fade(a: &Canvas, b: &Canvas, t: f32) -> Canvas {
    let (w, h) = (a.width().min(b.width()), a.height().min(b.height()));
    let (mut pa, mut pb) = (Vec::new(), Vec::new());
    a.to_rgba8(&mut pa);
    b.to_rgba8(&mut pb);
    let t = t.clamp(0.0, 1.0);
    let mut out = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let (ia, ib, io) = ((y * a.width() + x) * 4, (y * b.width() + x) * 4, (y * w + x) * 4);
            for k in 0..4 {
                out[io + k] = (pa[ia + k] as f32 + (pb[ib + k] as f32 - pa[ia + k] as f32) * t).round() as u8;
            }
        }
    }
    Canvas::from_rgba8(w, h, &out)
}
