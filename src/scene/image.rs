//! Immutable image resources shared by scene commands and future texture
//! caches. Pixels are straight RGBA8; decoding stays in `media`.
use super::{record, Command, Rect};
use fltk::{enums::ColorDepth, image::RgbImage, prelude::*};
use std::{fmt, sync::Arc};

/// Largest image resource accepted by the scene (64 MiB of RGBA pixels).
/// Decode or downsample oversized sources before constructing a resource.
pub const MAX_IMAGE_BYTES: usize = 64 * 1024 * 1024;

/// Invalid dimensions, byte count or resource size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageError;
impl fmt::Display for ImageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid scene image: expected nonempty RGBA8 within 64 MiB")
    }
}
impl std::error::Error for ImageError {}

/// Immutable, cheap-to-clone RGBA resource. Scenes retain the pixels, so
/// callers can drop their original handle after recording. This performs
/// no decoding, file I/O, widget creation or GPU allocation.
#[derive(Debug, Clone)]
pub struct Image {
    width: i32,
    height: i32,
    pixels: Arc<[u8]>,
}

impl Image {
    pub fn rgba(width: u32, height: u32, pixels: Vec<u8>) -> Result<Self, ImageError> {
        let size = (width as usize).checked_mul(height as usize).and_then(|n| n.checked_mul(4)).ok_or(ImageError)?;
        if width == 0 || height == 0 || width > i32::MAX as u32 || height > i32::MAX as u32
            || size > MAX_IMAGE_BYTES || pixels.len() != size {
            return Err(ImageError);
        }
        Ok(Self { width: width as i32, height: height as i32, pixels: pixels.into() })
    }

    pub fn width(&self) -> u32 { self.width as u32 }
    pub fn height(&self) -> u32 { self.height as u32 }
    pub fn pixels(&self) -> &[u8] { &self.pixels }
    #[cfg(feature = "gpu")]
    pub(super) fn shared_pixels(&self) -> Arc<[u8]> { self.pixels.clone() }

    /// Source-over onto the current surface, stretched to `rect`. Uses
    /// FLTK's image sampling, like existing image widgets. Recorded scenes
    /// retain the source resource rather than allocating a scaled bitmap.
    /// Main UI thread, inside a draw callback or `Scene::record`.
    pub fn draw(&self, rect: Rect) {
        if rect.w <= 0 || rect.h <= 0 { return; }
        if record(|| Command::Image { rect, image: self.clone() }) { return; }
        // SAFETY: self retains the immutable buffer throughout the image's
        // lifetime; the borrowed image is drawn and dropped in this scope.
        if let Ok(mut image) = unsafe { RgbImage::from_data(&self.pixels, self.width, self.height, ColorDepth::Rgba8) } {
            image.scale(rect.w, rect.h, false, true);
            image.draw(rect.x, rect.y, rect.w, rect.h);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_invalid_resources_and_retains_owned_pixels() {
        assert!(Image::rgba(0, 1, vec![]).is_err());
        assert!(Image::rgba(u32::MAX, u32::MAX, vec![]).is_err());
        assert!(Image::rgba(1, 1, vec![0; 3]).is_err());
        assert!(Image::rgba(8192, 8192, vec![]).is_err());
        let image = Image::rgba(1, 1, vec![7, 8, 9, 128]).unwrap();
        let clone = image.clone();
        drop(image);
        assert_eq!(clone.pixels(), &[7, 8, 9, 128]);
        assert_eq!((clone.width(), clone.height()), (1, 1));
    }
}
