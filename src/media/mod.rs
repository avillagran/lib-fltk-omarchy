//! Images: decode PNG, JPEG, GIF (every frame) and WebP (still and
//! animated) on the task pool, at the size they are shown in device
//! pixels, into a byte-bounded LRU cache; show them with [`Image`]
//! (fit / cover / letterbox, rounded corners, placeholder, GIF playback on
//! the animation clock) and open them in [`ImageViewer`] (zoom, pan, copy).
//!
//! Behind the `images` crate feature (off by default). The decoders are
//! pure Rust crates (png, gif, zune-jpeg, image-webp); fltk keeps its
//! `no-images` build. The feature needs Rust 1.80.1 (image-webp 0.2.4).
//!
//! ```no_run
//! use fltk_omarchy::media::{Fit, Image, ImageViewer, Source};
//! use fltk_omarchy::a11y::AccessibleExt;
//! let src = Source::file("/tmp/screenshot.png");
//! let thumb = Image::new(220, 160).source(&src).fit(Fit::Cover).framed(true).label("screenshot.png");
//! let s2 = src.clone();
//! let t2 = thumb.clone();
//! let thumb = thumb.on_activate(move || {
//!     ImageViewer::new(&s2).name("screenshot.png").on_copy(|bitmap| {
//!         let png = bitmap.to_png(); // hand it to the clipboard as image/png
//!         let _ = png;
//!     }).open(&*t2);
//! });
//! ```
//!
//! Inputs are bounded ([`Limits`]: input bytes, pixels per frame, frames,
//! output bytes); bad, truncated or hostile files give an [`Error`] value.
//! Guide: docs/MEDIA.md.

mod cache;
mod decode;
mod image;
#[cfg(feature = "playback")]
pub mod playback;
mod viewer;

pub use cache::{cache_stats, forget, load, load_with, peek, set_cache_budget, CacheStats, Source};
pub use decode::{decode, fitted_size, sniff};
pub use image::Image;
pub use viewer::{ImageViewer, ViewerHandle};
#[cfg(feature = "playback")]
pub use playback::{muted, Audio, PlayState, Video};

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

/// Image file formats the decoders read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Format {
    Png,
    Jpeg,
    Gif,
    WebP,
}

/// How an image fills its box.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Fit {
    /// Whole image, centered; the box shows through around it.
    #[default]
    Contain,
    /// Fills the box, cropped to its aspect ratio (thumbnails).
    Cover,
    /// Whole image on a `darker_background` box (bars fill the rest).
    Letterbox,
}

/// The size to decode for, in device pixels (a 0 side is unbounded;
/// `Target::original()` keeps the full size). Images are never upscaled
/// by the decoder: the widget scales on the screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Target {
    pub width: u32,
    pub height: u32,
    pub fit: Fit,
}

impl Target {
    pub fn new(width: u32, height: u32, fit: Fit) -> Target {
        Target { width, height, fit }
    }

    pub fn original() -> Target {
        Target { width: 0, height: 0, fit: Fit::Contain }
    }

    /// Cache identity: Letterbox decodes like Contain.
    pub(crate) fn normalized(self) -> Target {
        Target { fit: if self.fit == Fit::Cover { Fit::Cover } else { Fit::Contain }, ..self }
    }
}

/// Bounds for one decode. Defaults: 32 MiB of input, 40 megapixels per
/// frame (o3's limit), 1 000 frames, 96 MiB of decoded output (frames
/// past it are dropped, [`Decoded::truncated`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub max_input_bytes: usize,
    pub max_pixels: u64,
    pub max_frames: usize,
    pub max_output_bytes: usize,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits { max_input_bytes: 32 << 20, max_pixels: 40_000_000, max_frames: 1000, max_output_bytes: 96 << 20 }
    }
}

impl Limits {
    /// Working memory one decoder may allocate (a full-size frame and its
    /// conversions).
    pub(crate) fn max_decode_bytes(&self) -> usize {
        (self.max_pixels.saturating_mul(8)).min(usize::MAX as u64 / 2) as usize + (1 << 20)
    }
}

/// Why an image could not be shown. Errors are values: nothing panics on
/// bad input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The file is larger than [`Limits::max_input_bytes`].
    TooLarge { bytes: usize, limit: usize },
    /// Width × height is over [`Limits::max_pixels`].
    TooManyPixels { width: u32, height: u32, limit: u64 },
    /// Not PNG, JPEG, GIF or WebP (HEIC and AVIF are not decoded).
    Unsupported,
    /// The decoder rejected the data.
    Corrupt(String),
    /// A buffer could not be allocated.
    OutOfMemory { bytes: usize },
    /// Reading the file failed.
    Io(String),
    Cancelled,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::TooLarge { bytes, limit } => write!(f, "image file too large ({bytes} bytes, limit {limit})"),
            Error::TooManyPixels { width, height, limit } => write!(f, "image too large ({width}×{height}, limit {limit} pixels)"),
            Error::Unsupported => write!(f, "unsupported image format"),
            Error::Corrupt(e) => write!(f, "invalid image: {e}"),
            Error::OutOfMemory { bytes } => write!(f, "not enough memory for the image ({bytes} bytes)"),
            Error::Io(e) => write!(f, "could not read the image: {e}"),
            Error::Cancelled => write!(f, "cancelled"),
        }
    }
}

impl std::error::Error for Error {}

/// Straight (not premultiplied) RGBA8 pixels, row-major, shared.
#[derive(Clone, PartialEq, Eq)]
pub struct Bitmap {
    pub width: u32,
    pub height: u32,
    data: Arc<[u8]>,
}

impl fmt::Debug for Bitmap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Bitmap({}×{})", self.width, self.height)
    }
}

impl Bitmap {
    /// `rgba` must hold `width × height × 4` bytes.
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> Result<Bitmap, Error> {
        if width == 0 || height == 0 || rgba.len() != width as usize * height as usize * 4 {
            return Err(Error::Corrupt("bitmap size does not match its data".into()));
        }
        Ok(Bitmap { width, height, data: rgba.into() })
    }

    pub fn rgba(&self) -> &[u8] {
        &self.data
    }

    pub fn bytes(&self) -> usize {
        self.data.len()
    }

    /// Straight RGBA of one pixel.
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = (y as usize * self.width as usize + x as usize) * 4;
        [self.data[i], self.data[i + 1], self.data[i + 2], self.data[i + 3]]
    }

    /// PNG bytes (the clipboard's `image/png`; stored deflate, fast).
    pub fn to_png(&self) -> Vec<u8> {
        crate::png::encode(self.width, self.height, 4, &self.data)
    }
}

/// One frame and how long it shows (zero for still images).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub bitmap: Bitmap,
    pub delay: Duration,
}

/// A decoded image: one frame, or every frame of an animation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decoded {
    pub format: Format,
    /// Full size of the file's image (after EXIF orientation).
    pub source_size: (u32, u32),
    pub frames: Vec<Frame>,
    /// Frames were dropped (limits) or the file ended early.
    pub truncated: bool,
}

impl Decoded {
    pub fn first(&self) -> &Bitmap {
        &self.frames[0].bitmap
    }

    /// Size of the decoded frames.
    pub fn size(&self) -> (u32, u32) {
        (self.first().width, self.first().height)
    }

    pub fn is_animated(&self) -> bool {
        self.frames.len() > 1
    }

    /// Bytes of pixel data held.
    pub fn bytes(&self) -> usize {
        self.frames.iter().map(|f| f.bitmap.bytes()).sum()
    }

    /// Length of one loop.
    pub fn duration(&self) -> Duration {
        self.frames.iter().map(|f| f.delay).sum()
    }

    /// The frame shown `elapsed` after the animation started (looping)
    /// and how long until the next one.
    pub fn frame_at(&self, elapsed: Duration) -> (usize, Duration) {
        let total = self.duration();
        if self.frames.len() < 2 || total.is_zero() {
            return (0, Duration::MAX);
        }
        let mut t = Duration::from_nanos((elapsed.as_nanos() % total.as_nanos()) as u64);
        for (i, f) in self.frames.iter().enumerate() {
            if t < f.delay {
                return (i, f.delay - t);
            }
            t -= f.delay;
        }
        (0, self.frames[0].delay)
    }
}

/// Reads and decodes a file now, on the calling thread (a worker of your
/// own, a CLI). The UI uses [`load`] instead.
pub fn decode_file(path: &std::path::Path, target: Target, limits: &Limits) -> Result<Decoded, Error> {
    let bytes = cache::read_bounded(path, limits.max_input_bytes)?;
    decode(&bytes, target, limits, &|| false)
}
