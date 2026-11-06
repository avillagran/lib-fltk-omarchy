//! Immutable image resources shared by scene commands and future texture
//! caches. Pixels are straight RGBA8; decoding stays in `media`.
use super::{record, Command, Rect};
use fltk::{enums::ColorDepth, image::RgbImage, prelude::*};
use std::{
    fmt,
    sync::{Arc, Mutex},
};

/// Largest image resource accepted by the scene (64 MiB of RGBA pixels).
/// Decode or downsample oversized sources before constructing a resource.
pub const MAX_IMAGE_BYTES: usize = 64 * 1024 * 1024;
const MAX_SCALED_CACHE_BYTES: usize = 8 * 1024 * 1024;
#[derive(Debug)]
struct Scaled {
    source: usize,
    width: u32,
    height: u32,
    pixels: Arc<[u8]>,
}

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
    scaled: Arc<Mutex<Option<Scaled>>>,
}

impl Image {
    pub fn rgba(width: u32, height: u32, pixels: Vec<u8>) -> Result<Self, ImageError> {
        let size = (width as usize)
            .checked_mul(height as usize)
            .and_then(|n| n.checked_mul(4))
            .ok_or(ImageError)?;
        if width == 0
            || height == 0
            || width > i32::MAX as u32
            || height > i32::MAX as u32
            || size > MAX_IMAGE_BYTES
            || pixels.len() != size
        {
            return Err(ImageError);
        }
        Ok(Self {
            width: width as i32,
            height: height as i32,
            pixels: pixels.into(),
            scaled: Arc::new(Mutex::new(None)),
        })
    }

    pub fn width(&self) -> u32 {
        self.width as u32
    }
    pub fn height(&self) -> u32 {
        self.height as u32
    }
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }
    #[cfg(feature = "gpu")]
    pub(super) fn shared_pixels(&self) -> Arc<[u8]> {
        self.pixels.clone()
    }

    /// Deterministic pixel-center bilinear sampling in premultiplied alpha.
    /// Independent of native destination coordinates and clipping; shared
    /// by CPU replay and GPU preparation. The returned resource remains
    /// straight RGBA. No FLTK or other native calls; usable from workers.
    /// One result up to 8 MiB is cached per shared resource, within the
    /// usual 64 MiB maximum output size. Larger results are not cached.
    pub fn resampled(&self, width: u32, height: u32) -> Result<Self, ImageError> {
        let size = (width as usize)
            .checked_mul(height as usize)
            .and_then(|n| n.checked_mul(4))
            .ok_or(ImageError)?;
        if width == 0
            || height == 0
            || width > i32::MAX as u32
            || height > i32::MAX as u32
            || size > MAX_IMAGE_BYTES
        {
            return Err(ImageError);
        }
        if width == self.width() && height == self.height() {
            return Ok(self.clone());
        }
        let source = self.pixels.as_ptr() as usize;
        let mut cache = self.scaled.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(c) = cache
            .as_ref()
            .filter(|c| c.source == source && c.width == width && c.height == height)
        {
            return Ok(Self {
                width: width as i32,
                height: height as i32,
                pixels: c.pixels.clone(),
                scaled: self.scaled.clone(),
            });
        }
        let mut pixels = Vec::with_capacity(size);
        let axis = |p: u32, src: u32, dst: u32| {
            let x = ((p as f64 + 0.5) * src as f64 / dst as f64 - 0.5).clamp(0.0, src as f64 - 1.0);
            let lo = x.floor() as u32;
            (lo, (lo + 1).min(src - 1), x - lo as f64)
        };
        for y in 0..height {
            let (y0, y1, fy) = axis(y, self.height(), height);
            for x in 0..width {
                let (x0, x1, fx) = axis(x, self.width(), width);
                let samples = [
                    (x0, y0, (1.0 - fx) * (1.0 - fy)),
                    (x1, y0, fx * (1.0 - fy)),
                    (x0, y1, (1.0 - fx) * fy),
                    (x1, y1, fx * fy),
                ];
                let mut rgb = [0.0; 3];
                let mut alpha = 0.0;
                for (sx, sy, weight) in samples {
                    let p = &self.pixels[((sy as usize * self.width as usize + sx as usize) * 4)..]
                        [..4];
                    let a = p[3] as f64 * weight;
                    alpha += a;
                    for c in 0..3 {
                        rgb[c] += p[c] as f64 * a;
                    }
                }
                for c in rgb {
                    pixels.push(if alpha == 0.0 {
                        0
                    } else {
                        (c / alpha).round().clamp(0.0, 255.0) as u8
                    });
                }
                pixels.push(alpha.round().clamp(0.0, 255.0) as u8);
            }
        }
        let pixels: Arc<[u8]> = pixels.into();
        if size <= MAX_SCALED_CACHE_BYTES {
            *cache = Some(Scaled {
                source,
                width,
                height,
                pixels: pixels.clone(),
            });
        }
        Ok(Self {
            width: width as i32,
            height: height as i32,
            pixels,
            scaled: self.scaled.clone(),
        })
    }
    /// Releases the shared optional resampling cache; retained returned
    /// image resources remain valid through their own immutable pixels.
    pub fn clear_cache(&self) {
        *self.scaled.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    /// Source-over onto the current surface, stretched to `rect`. Uses
    /// the shared resampler before native 1:1 drawing. Recorded scenes
    /// retain the source resource; repeated equal-size draws share pixels.
    /// Main UI thread, inside a draw callback or `Scene::record`.
    pub fn draw(&self, rect: Rect) {
        if rect.w <= 0 || rect.h <= 0 {
            return;
        }
        if record(|| Command::Image {
            rect,
            image: self.clone(),
        }) {
            return;
        }
        if !fltk::draw::not_clipped(rect.x, rect.y, rect.w, rect.h) {
            return;
        }
        let Ok(scaled) = self.resampled(rect.w as u32, rect.h as u32) else {
            return;
        };
        // SAFETY: scaled retains the immutable buffer throughout the image's
        // lifetime; the borrowed image is drawn and dropped in this scope.
        if let Ok(mut image) = unsafe {
            RgbImage::from_data(
                &scaled.pixels,
                scaled.width,
                scaled.height,
                ColorDepth::Rgba8,
            )
        } {
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
    #[test]
    fn resampling_uses_pixel_centers_and_does_not_bleed_transparent_colors() {
        let image = Image::rgba(2, 1, vec![255, 0, 0, 255, 0, 0, 255, 0]).unwrap();
        assert_eq!(
            image.resampled(3, 1).unwrap().pixels(),
            &[255, 0, 0, 255, 255, 0, 0, 128, 0, 0, 0, 0]
        );
        assert_eq!(image.resampled(1, 1).unwrap().pixels(), &[255, 0, 0, 128]);
        assert!(image.resampled(0, 1).is_err());
        assert!(image.resampled(8192, 8192).is_err());
    }
    #[test]
    fn scaled_resources_share_a_bounded_cache_across_clones_and_workers() {
        let image =
            Image::rgba(3, 1, vec![0, 0, 0, 255, 255, 255, 255, 255, 0, 0, 0, 255]).unwrap();
        let copy = image.clone();
        let first = std::thread::spawn(move || copy.resampled(2, 1).unwrap())
            .join()
            .unwrap();
        assert!(std::ptr::eq(
            first.pixels(),
            image.resampled(2, 1).unwrap().pixels()
        ));
        assert_eq!(first.pixels(), &[64, 64, 64, 255, 64, 64, 64, 255]);
        assert_eq!(
            image.resampled(1, 1).unwrap().pixels(),
            &[255, 255, 255, 255]
        );
        // A returned resource shares the cache but has different source
        // pixels; matching only destination dimensions would return white.
        assert_eq!(first.resampled(1, 1).unwrap().pixels(), &[64, 64, 64, 255]);
        let retained = image.resampled(2, 1).unwrap();
        image.clear_cache();
        assert!(!std::ptr::eq(
            retained.pixels(),
            image.resampled(2, 1).unwrap().pixels()
        ));
        assert_eq!(retained.pixels(), first.pixels());
    }
}
