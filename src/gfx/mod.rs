//! Shapes, paints and CPU shaders for custom visuals, rendered in
//! software (no GPU, no extra fltk features).
//!
//! - [`Path`]: lines, quadratic/cubic Béziers, arcs, polygons, rounded
//!   rectangles with a radius per corner, squircles; `contains` and
//!   `near_outline` for hit-testing.
//! - [`Canvas`]: anti-aliased `fill` and `stroke` ([`Stroke`] width, joins,
//!   caps), [`Paint`] (solid, linear, radial, conic gradients), `clip`,
//!   `layer` (group opacity), `shadow`, `blur`, [`BlendMode`].
//! - [`Shader`]: a per-pixel function with [`Uniforms`] (time, size,
//!   pointer, theme palette); examples in [`shaders`].
//! - [`View`]: the widget that shows it, with an area limit, a time budget
//!   per draw, buffer reuse, animation that pauses while hidden and shape
//!   hit-testing.
//!
//! Colors come from the theme through [`Uniforms`] (`u.accent`,
//! `u.background`…), so every visual follows `omarchy-theme-set`.
//! Guide with pictures: `docs/SHADERS.md`.

mod canvas;
mod path;
mod raster;
mod shader;
mod view;
#[cfg(feature = "gpu")]
pub mod gpu;

pub use canvas::{BlendMode, Canvas, Color, Paint, MAX_BLUR};
pub use path::{Path, Point};
pub use raster::{Cap, Join, Stroke};
pub use shader::{Shader, Uniforms};
pub use view::{Hit, View, ViewStats};

/// Example shaders and noise helpers.
pub mod shaders {
    pub use super::shader::{accent_glow, blobs, fbm, gradient, hash2, noise, value_noise};
}
