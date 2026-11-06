//! Retained drawing commands for the toolkit's shared painter.
//!
//! This is the first step toward a window-wide CPU/GPU scene. Recording
//! intercepts `paint` helpers only, not arbitrary FLTK drawing or widgets.
//! Replay currently uses the existing CPU painter; it creates no timers
//! or GL contexts. Damage replay and isolated layers lazily retain bounded
//! pixel buffers. Record again after theme, font, language,
//! scale or layout changes. Coordinates are FLTK logical window units.

mod image;
mod layer;
mod shader;
mod node;
mod motion;
#[cfg(feature = "gpu")]
mod bridge;
pub use motion::NodeMotion;
pub use node::{NodeError, NodeId};
pub(crate) mod surface;
pub use surface::{Surface, SurfaceStats, WeakSurface};
pub use shader::shader;
#[cfg(feature = "gpu")]
pub mod gpu;
pub use layer::Layer;
pub use image::{Image, ImageError, MAX_IMAGE_BYTES};

use crate::{paint, theme::Rgb};
use fltk::{draw, enums::{Align, Font}};
use std::cell::RefCell;
use fltk::prelude::*;

/// A logical rectangle. Non-positive dimensions describe an empty region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self { Self { x, y, w, h } }

    /// Half-open bounds, computed without overflowing logical coordinates.
    pub fn contains(self, x: i32, y: i32) -> bool {
        self.w > 0 && self.h > 0 && x >= self.x && y >= self.y
            && (x as i64) < self.x as i64 + self.w as i64
            && (y as i64) < self.y as i64 + self.h as i64
    }

    pub fn intersects(self, other: Self) -> bool {
        self.w > 0 && self.h > 0 && other.w > 0 && other.h > 0
            && (self.x as i64) < other.x as i64 + other.w as i64
            && (other.x as i64) < self.x as i64 + self.w as i64
            && (self.y as i64) < other.y as i64 + other.h as i64
            && (other.y as i64) < self.y as i64 + self.h as i64
    }
}

/// Backend-neutral commands in paint order. Text remains logical text
/// with its font, rather than a raster image; glyph shaping/atlases are
/// future backend work. Clips are balanced by [`clip`].
#[derive(Debug, Clone)]
pub enum Command {
    Fill { rect: Rect, radius: i32, color: Rgb },
    Stroke { rect: Rect, radius: i32, width: i32, color: Rgb },
    Text { rect: Rect, text: String, font: Font, size: i32, color: Rgb, align: Align },
    SpacedText { rect: Rect, text: String, size: i32, weight: paint::Weight, spacing: f64, color: Rgb, align: Align },
    AlphaFill { rect: Rect, color: Rgb, alpha: f64 },
    Image { rect: Rect, image: Image },
    Layer { rect: Rect, layer: Layer, opacity: u8, id: Option<NodeId> },
    Shader { rect: Rect, program: crate::gfx::sl::Shader, uniforms: crate::gfx::Uniforms },
    PushClip(Rect),
    PopClip,
}

impl Command {
    fn bounds(&self) -> Option<Rect> {
        match self {
            Self::Fill { rect, .. } | Self::Stroke { rect, .. } | Self::Text { rect, .. }
                | Self::AlphaFill { rect, .. } | Self::SpacedText { rect, .. } | Self::Image { rect, .. } | Self::Layer { rect, .. } | Self::Shader { rect, .. } => Some(*rect),
            _ => None,
        }
    }
}

/// A retained display list. Ordinary paint calls without recording keep
/// their immediate CPU path and do not allocate a command list.
#[derive(Debug, Clone, Default)]
pub struct Scene {
    commands: Vec<Command>,
    raster: RefCell<Option<Raster>>,
    nodes: RefCell<Option<node::Index>>,
}

#[derive(Debug, Clone)]
struct Raster {
    bounds: Rect,
    image: fltk::image::RgbImage,
}

thread_local! {
    static RECORDING: RefCell<Option<Scene>> = const { RefCell::new(None) };
}

// Restores the outer recorder on normal return and on unwind. A nested
// recording is isolated; use Scene::append to include it explicitly.
struct Restore(Option<Scene>);
impl Drop for Restore {
    fn drop(&mut self) {
        RECORDING.with(|r| { *r.borrow_mut() = self.0.take(); });
    }
}

/// Called lazily: strings are copied only when a scene is being recorded.
pub(crate) fn record(command: impl FnOnce() -> Command) -> bool {
    RECORDING.with(|r| {
        let mut r = r.borrow_mut();
        if let Some(s) = r.as_mut() {
            s.commands.push(command());
            true
        } else { false }
    })
}

impl Scene {
    /// Records shared `paint` calls from `f`, without painting them. Text
    /// measurement still needs FLTK on the main thread. This does not
    /// capture raw `fltk::draw` calls: use [`clip`] for recorded clipping.
    pub fn record<T>(f: impl FnOnce() -> T) -> (Self, T) {
        let restore = Restore(RECORDING.with(|r| r.replace(Some(Scene::default()))));
        let value = f();
        let scene = RECORDING.with(|r| r.borrow_mut().take().unwrap());
        drop(restore);
        (scene, value)
    }

    pub fn commands(&self) -> &[Command] { &self.commands }

    /// Copies another list, preserving its order and balanced clips.
    pub fn append(&mut self, scene: &Scene) {
        self.commands.extend_from_slice(&scene.commands);
        *self.nodes.get_mut() = None;
        *self.raster.get_mut() = None;
    }

    /// Replays on the current FLTK drawing surface, respecting its clip.
    pub fn draw(&self) { self.replay(None); }

    /// Draws a local-coordinate scene translated into its parent's surface.
    /// Shader time can be overridden without rebuilding scene content.
    /// Returns false before painting if translated coordinates overflow.
    pub fn draw_at(&self, x: i32, y: i32, time: Option<f32>) -> bool {
        let Some(local) = self.translated(x, y, time) else { return false };
        local.draw();
        true
    }

    fn translated(&self, dx: i32, dy: i32, time: Option<f32>) -> Option<Scene> {
        let mut scene = Scene { commands: self.commands.clone(), ..Scene::default() };
        for command in &mut scene.commands {
            let rect = match command {
                Command::Fill { rect, .. } | Command::Stroke { rect, .. }
                | Command::Text { rect, .. } | Command::SpacedText { rect, .. }
                | Command::AlphaFill { rect, .. } | Command::Image { rect, .. }
                | Command::Layer { rect, .. } | Command::PushClip(rect) => Some(rect),
                Command::Shader { rect, uniforms, .. } => {
                    if let Some(time) = time { uniforms.time = time; }
                    Some(rect)
                }
                Command::PopClip => None,
            };
            if let Some(rect) = rect {
                rect.x = rect.x.checked_add(dx)?;
                rect.y = rect.y.checked_add(dy)?;
            }
        }
        Some(scene)
    }

    /// Replays only within `damage`, leaving all other pixels untouched.
    /// A lazily cached opaque raster makes clipping deterministic across
    /// native rasterizers. Returns false if its bounds exceed 64 MiB or
    /// the native surface cannot produce an image. No pixels then change.
    /// The caller must include the background and every intersecting
    /// layer in the list (including both old and new bounds after a move).
    pub fn draw_damage(&self, damage: Rect) -> bool {
        if damage.w <= 0 || damage.h <= 0 { return true; }
        // Native Cairo/CoreGraphics rasterizers can change antialiasing
        // and scaled-image sampling when the clip changes. Raster once
        // with the original scene clips, then copy exact pixels per damage
        // region. The immediate/full-replay path remains allocation-free.
        let mut raster = self.raster.borrow_mut();
        if raster.is_none() {
            *raster = self.rasterize();
        }
        let Some(raster) = raster.as_mut() else { return false };
        let r = raster.bounds;
        if !r.intersects(damage) { return true; }
        clip(damage, || raster.image.draw(r.x, r.y, r.w, r.h));
        true
    }

    /// Releases the optional CPU damage raster. Use after changing the
    /// surface scale/font rendering environment; theme/layout changes
    /// still require recording a new scene.
    pub fn clear_raster(&self) {
        *self.raster.borrow_mut() = None;
        for command in &self.commands {
            match command {
                Command::Layer { layer, .. } => layer.clear_raster(),
                Command::Image { image, .. } => image.clear_cache(),
                _ => {}
            }
        }
    }

    fn rasterize(&self) -> Option<Raster> {
        let mut bounds: Option<(i64, i64, i64, i64)> = None;
        for r in self.commands.iter().filter_map(Command::bounds).filter(|r| r.w > 0 && r.h > 0) {
            let b = (r.x as i64, r.y as i64, r.x as i64 + r.w as i64, r.y as i64 + r.h as i64);
            bounds = Some(match bounds { None => b, Some(a) => (a.0.min(b.0), a.1.min(b.1), a.2.max(b.2), a.3.max(b.3)) });
        }
        let (x, y, right, bottom) = bounds?;
        let (w, h) = (right - x, bottom - y);
        if w > i32::MAX as i64 || h > i32::MAX as i64 || w.checked_mul(h)?.checked_mul(4)? > MAX_IMAGE_BYTES as i64 { return None; }
        let r = Rect::new(x as i32, y as i32, w as i32, h as i32);
        Some(Raster { bounds: r, image: self.render(r, None)? })
    }

    // One private surface path for opaque damage caches and transparent
    // layer reconstruction. Localize integer coordinates explicitly:
    // ImageSurface::set_origin only offsets widget drawing, not all raw
    // drawing helpers on every platform.
    fn render(&self, r: Rect, background: Option<Rgb>) -> Option<fltk::image::RgbImage> {
        let mut local = Scene { commands: self.commands.clone(), ..Scene::default() };
        for command in &mut local.commands {
            let rect = match command {
                Command::Fill { rect, .. } | Command::Stroke { rect, .. }
                | Command::Text { rect, .. } | Command::SpacedText { rect, .. }
                | Command::AlphaFill { rect, .. } | Command::Image { rect, .. }
                | Command::Layer { rect, .. } | Command::Shader { rect, .. } | Command::PushClip(rect) => Some(rect),
                Command::PopClip => None,
            };
            if let Some(rect) = rect {
                rect.x = rect.x.checked_sub(r.x)?;
                rect.y = rect.y.checked_sub(r.y)?;
            }
        }
        let surface = fltk::surface::ImageSurface::new(r.w, r.h, false);
        fltk::surface::ImageSurface::push_current(&surface);
        draw::push_no_clip();
        if let Some(bg) = background {
            // Do not let an outer recording intercept the clear.
            draw::set_draw_color(bg.fl());
            draw::draw_rectf(0, 0, r.w, r.h);
        }
        local.replay(None);
        draw::pop_clip();
        let image = surface.image();
        fltk::surface::ImageSurface::pop_current();
        image
    }

    fn replay(&self, damage: Option<Rect>) {
        if damage.is_some_and(|r| r.w <= 0 || r.h <= 0) { return; }
        let _restore = Restore(RECORDING.with(|r| r.replace(None)));
        let _damage_clip = damage.map(ClipGuard::new);
        for command in &self.commands {
            if let (Some(bounds), Some(damage)) = (command.bounds(), damage) {
                if !bounds.intersects(damage) { continue; }
            }
            match command {
                Command::Fill { rect: r, radius, color } => paint::fill(r.x, r.y, r.w, r.h, *radius, *color),
                Command::Stroke { rect: r, radius, width, color } => paint::stroke(r.x, r.y, r.w, r.h, *radius, *width, *color),
                Command::Text { rect, text, font, size, color, align } => paint::text_font(text, *rect, *size, *font, *color, *align),
                Command::SpacedText { rect: r, text, size, weight, spacing, color, align } => {
                    paint::text_spaced(text, r.x, r.y, r.w, r.h, *size, *weight, *spacing, *color, *align);
                }
                Command::AlphaFill { rect: r, color, alpha } => paint::alpha_fill(r.x, r.y, r.w, r.h, *color, *alpha),
                Command::Image { rect, image } => image.draw(*rect),
                Command::Layer { rect, layer, opacity, .. } => { layer.draw_quantized(*rect, *opacity); }
                Command::Shader { rect, program, uniforms } => { shader(*rect, program, uniforms); }
                Command::PushClip(r) => draw::push_clip(r.x, r.y, r.w.max(0), r.h.max(0)),
                Command::PopClip => draw::pop_clip(),
            }
        }
    }
}

struct ClipGuard { recorded: bool }
impl ClipGuard {
    fn new(r: Rect) -> Self {
        let recorded = record(|| Command::PushClip(r));
        if !recorded { draw::push_clip(r.x, r.y, r.w.max(0), r.h.max(0)); }
        Self { recorded }
    }
}
impl Drop for ClipGuard {
    fn drop(&mut self) {
        if self.recorded { record(|| Command::PopClip); } else { draw::pop_clip(); }
    }
}

/// A balanced clip scope, for immediate painting and recorded scenes.
/// Restores the previous clip even when `f` unwinds.
pub fn clip<T>(rect: Rect, f: impl FnOnce() -> T) -> T {
    let _guard = ClipGuard::new(rect);
    f()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recording_is_nested_and_unwind_safe() {
        let (outer, ()) = Scene::record(|| {
            paint::fill(0, 0, 20, 20, 3, Rgb(0));
            let (inner, ()) = Scene::record(|| clip(Rect::new(1, 1, 2, 2), || {
                paint::fill(0, 0, 5, 5, 0, Rgb(1));
            }));
            assert_eq!(inner.commands().len(), 3);
            assert!(std::panic::catch_unwind(|| {
                Scene::record(|| panic!("restore recorder"));
            }).is_err());
            paint::fill(0, 0, 3, 3, 0, Rgb(2));
        });
        assert_eq!(outer.commands().len(), 2);
        assert!(!record(|| Command::PopClip));
    }

    #[test]
    fn damage_rejects_unbounded_backing_storage_before_drawing() {
        let (s, ()) = Scene::record(|| paint::fill(0, 0, i32::MAX, i32::MAX, 0, Rgb(0)));
        assert!(!s.draw_damage(Rect::new(0, 0, 10, 10)));
    }

    #[test]
    fn intersection_does_not_overflow_or_include_empty_rects() {
        let a = Rect::new(i32::MAX - 1, 0, 100, 10);
        assert!(a.intersects(Rect::new(i32::MAX, 1, 2, 2)));
        assert!(!a.intersects(Rect::new(i32::MIN, 0, 100, 10)));
        assert!(!a.intersects(Rect::new(i32::MAX, 0, 0, 10)));
        assert!(!Rect::new(0, 0, 2, 2).intersects(Rect::new(2, 0, 2, 2)));
    }
}
