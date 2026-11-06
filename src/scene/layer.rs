//! Isolated transparent scene layers: composite the finished group once,
//! rather than applying opacity to each overlapping primitive.
use super::{record, Command, Image, ImageError, Rect, Scene, NodeId, MAX_IMAGE_BYTES};
use crate::theme::Rgb;
use fltk::{enums::ColorDepth, image::RgbImage, prelude::*};
use std::{cell::RefCell, rc::Rc};

#[derive(Debug)]
struct Inner {
    bounds: Rect,
    scene: Scene,
    image: RefCell<Option<Image>>,
    upload: RefCell<Vec<u8>>,
}

/// An immutable child scene rendered into transparent RGBA once, then
/// composited over the actual destination. Unlike `layer::Layer`, no flat
/// parent color is baked into the picture. Main UI thread only.
///
/// CPU fallback uses black/white passes to recover alpha from FLTK's
/// opaque native image surfaces. Native text subpixel coverage is reduced
/// to a single alpha channel. The image is at logical 1× resolution.
#[derive(Debug, Clone)]
pub struct Layer(Rc<Inner>);

impl Layer {
    /// Retains `scene` clipped to `bounds`. Children use those original
    /// coordinates; drawing at (x,y) places bounds' top-left there.
    pub fn new(bounds: Rect, scene: Scene) -> Result<Self, ImageError> {
        if bounds.w <= 0 || bounds.h <= 0 || (bounds.w as usize).checked_mul(bounds.h as usize).and_then(|n| n.checked_mul(4)).map_or(true, |n| n > MAX_IMAGE_BYTES) {
            return Err(ImageError);
        }
        Ok(Self(Rc::new(Inner { bounds, scene, image: RefCell::new(None), upload: RefCell::new(Vec::new()) })))
    }

    pub fn bounds(&self) -> Rect { self.0.bounds }
    pub fn scene(&self) -> &Scene { &self.0.scene }

    /// Releases the CPU RGBA and upload buffers (surface scale/font
    /// changes). For changed colors, language or layout, create a new layer.
    pub fn clear_raster(&self) {
        self.0.scene.clear_raster();
        *self.0.image.borrow_mut() = None;
        *self.0.upload.borrow_mut() = Vec::new();
    }

    /// Renders the isolated layer at (x,y) with group opacity. An outer
    /// `scene::clip` clips the composited result, not its child raster.
    /// Returns false on native surface failure or unrepresentable local
    /// coordinates. Recording defers native rendering until replay.
    pub fn draw_at(&self, x: i32, y: i32, opacity: f64) -> bool {
        let opacity = if opacity.is_finite() { (opacity.clamp(0.0, 1.0) * 255.0).round() as u8 } else { 0 };
        self.draw_quantized(Rect::new(x, y, self.0.bounds.w, self.0.bounds.h), opacity)
    }

    /// Records a keyed isolated layer. Use the same application ID after
    /// reordering or clipping changes; duplicate IDs make keyed updates
    /// and GL preparation fail. IDs are scoped to this recorded scene,
    /// not to the contents of nested isolated layers.
    /// Zero opacity is retained so the node can be faded in later.
    pub fn draw_keyed(&self, id: NodeId, x: i32, y: i32, opacity: f32) -> Result<bool, super::NodeError> {
        let opacity = super::node::opacity(opacity)?;
        let rect = Rect::new(x, y, self.0.bounds.w, self.0.bounds.h);
        if record(|| Command::Layer { rect, layer: self.clone(), opacity, id: Some(id) }) {
            return Ok(true);
        }
        Ok(self.draw_quantized(rect, opacity))
    }

    pub(super) fn draw_quantized(&self, rect: Rect, opacity: u8) -> bool {
        if opacity == 0 { return true; }
        if record(|| Command::Layer { rect, layer: self.clone(), opacity, id: None }) { return true; }
        if self.0.image.borrow().is_none() {
            let Some(image) = self.rasterize() else { return false };
            *self.0.image.borrow_mut() = Some(image);
        }
        let image = self.0.image.borrow();
        let image = image.as_ref().unwrap();
        if opacity == 255 {
            image.draw(rect);
        } else {
            let mut upload = self.0.upload.borrow_mut();
            upload.clear();
            upload.extend_from_slice(image.pixels());
            for p in upload.chunks_exact_mut(4) {
                p[3] = ((p[3] as u32 * opacity as u32 + 127) / 255) as u8;
            }
            // SAFETY: upload retains the bytes until after this borrowed
            // native image has drawn and been dropped.
            if let Ok(mut img) = unsafe { RgbImage::from_data(&upload, rect.w, rect.h, ColorDepth::Rgba8) } {
                img.draw(rect.x, rect.y, rect.w, rect.h);
            } else { return false; }
        }
        true
    }

    #[cfg(feature = "gpu")]
    pub(super) fn image(&self) -> Option<Image> {
        if self.0.image.borrow().is_none() {
            *self.0.image.borrow_mut() = Some(self.rasterize()?);
        }
        self.0.image.borrow().clone()
    }

    #[cfg(feature = "gpu")]
    pub(super) fn premultiplied_pixels(&self) -> Option<Vec<u8>> {
        self.rasterize_pixels(true)
    }

    fn rasterize(&self) -> Option<Image> {
        Image::rgba(self.0.bounds.w as u32, self.0.bounds.h as u32, self.rasterize_pixels(false)?).ok()
    }

    fn rasterize_pixels(&self, premultiplied: bool) -> Option<Vec<u8>> {
        let black = self.0.scene.render(self.0.bounds, Some(Rgb(0)))?;
        let white = self.0.scene.render(self.0.bounds, Some(Rgb(0xffffff)))?;
        if black.depth() != ColorDepth::Rgb8 || white.depth() != ColorDepth::Rgb8 { return None; }
        let (black, white) = (black.to_rgb_data(), white.to_rgb_data());
        let mut rgba = Vec::with_capacity(self.0.bounds.w as usize * self.0.bounds.h as usize * 4);
        for (b, w) in black.chunks_exact(3).zip(white.chunks_exact(3)) {
            // Pick the strongest coverage when native subpixel text has
            // different channel alphas. This keeps premultiplied channels
            // within alpha and avoids colored halos on other backdrops.
            let a = 255 - (0..3).map(|i| w[i].saturating_sub(b[i])).min().unwrap();
            if premultiplied {
                // Preserve the native black pass exactly for a GPU-only
                // fallback texture. Unpremultiplying and premultiplying
                // again loses low-alpha color precision at each overlap.
                rgba.extend_from_slice(b);
            } else {
                for value in b {
                    rgba.push(if a == 0 { 0 } else { ((*value as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8 });
                }
            }
            rgba.push(a);
        }
        Some(rgba)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_empty_and_overflowing_bounds_without_native_drawing() {
        assert!(Layer::new(Rect::new(0, 0, 0, 10), Scene::default()).is_err());
        assert!(Layer::new(Rect::new(0, 0, -1, 10), Scene::default()).is_err());
        assert!(Layer::new(Rect::new(0, 0, i32::MAX, i32::MAX), Scene::default()).is_err());
    }
}
