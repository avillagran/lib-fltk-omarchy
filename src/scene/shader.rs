//! Portable shader commands for scene composition.
use super::{record, Command, Image, Rect, MAX_IMAGE_BYTES};
use crate::gfx::{sl, Color, Uniforms};

/// Draw a portable shader in local coordinates (0..rect.w, 0..rect.h).
/// Recorded scenes retain the program and uniforms; no pixels are
/// evaluated until CPU replay. GPU scene renderers execute GLSL directly.
/// Returns false if the CPU fallback exceeds the image resource budget.
pub fn shader(rect: Rect, program: &sl::Shader, uniforms: &Uniforms) -> bool {
    if rect.w <= 0 || rect.h <= 0 { return true; }
    if record(|| Command::Shader { rect, program: program.clone(), uniforms: *uniforms }) { return true; }
    let Some(size) = (rect.w as usize).checked_mul(rect.h as usize).and_then(|n| n.checked_mul(4)).filter(|n| *n <= MAX_IMAGE_BYTES) else { return false };
    let mut u = *uniforms;
    u.size = (rect.w as f32, rect.h as f32);
    let mut exec = sl::Exec::default();
    let mut row = vec![Color::TRANSPARENT; rect.w as usize];
    let mut pixels = Vec::with_capacity(size);
    for y in 0..rect.h {
        exec.row(program, 0.0, y as f32, row.len(), (1.0, 1.0), &u, &mut row);
        for color in &row { pixels.extend_from_slice(&color.to_u8()); }
    }
    match Image::rgba(rect.w as u32, rect.h as u32, pixels) {
        Ok(image) => { image.draw(rect); true }
        Err(_) => false,
    }
}
