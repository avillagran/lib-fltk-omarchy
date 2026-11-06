//! Painting a [`Diagram`]: shapes rasterized with [`gfx`](crate::gfx) at
//! the screen's pixel density into a cached picture, text drawn on top
//! with FLTK and the theme font, colors from the theme ([`color`]).

use super::{Align, Diagram, Item, Role};
use crate::gfx::{Canvas, Cap, Color, Join, Path, Stroke};
use crate::theme::{self, Rgb, Theme};
use fltk::{enums::ColorDepth, image::RgbImage, prelude::*};
use std::hash::{Hash, Hasher};

/// The color of `role` in theme `t` over `under` (the color drawn under
/// the diagram).
pub fn color(role: Role, t: &Theme, under: Rgb) -> Rgb {
    let p = &t.palette;
    match role {
        Role::Background => under,
        Role::NodeFill => p.accent.over(under, 0.10),
        Role::NodeBorder => p.accent.over(under, 0.85),
        Role::ClusterFill => p.foreground.over(under, 0.045),
        Role::ClusterBorder => p.foreground.over(under, 0.22),
        Role::Line => p.foreground.over(under, 0.62),
        Role::Text => p.foreground,
        Role::Muted => p.foreground.darker(1.4),
        Role::Accent => p.accent,
        Role::NoteFill => p.yellow.over(under, 0.14),
        Role::NoteBorder => p.yellow.over(under, 0.55),
        Role::Marker => p.foreground.over(under, 0.8),
        Role::ActivationFill => p.accent.over(under, 0.22),
    }
}

/// Most canvas pixels one picture holds (32 MB of RGBA).
const MAX_PIXELS: f32 = 8_000_000.0;

/// A rendered picture of a diagram's shapes, kept while the diagram, the
/// pixel density, the theme and the color under it are the same. Text is
/// drawn on top at every paint (FLTK text is crisp at any density).
#[derive(Default)]
pub struct Picture {
    key: u64,
    rgba: Vec<u8>,
    size: (i32, i32),
    renders: u64,
}

impl Picture {
    pub fn new() -> Picture {
        Picture::default()
    }

    /// Draws `d` with its top left at (x, y), `unit` canvas pixels per
    /// widget pixel (`native::pixels_per_unit`), over `under`.
    pub fn draw(&mut self, d: &Diagram, x: i32, y: i32, unit: f32, under: Rgb) {
        let (w, h) = (d.width.ceil().max(1.0) as i32, d.height.ceil().max(1.0) as i32);
        // At most MAX_PIXELS per picture: huge diagrams render coarser.
        let fit = (MAX_PIXELS / (w as f32 * h as f32)).sqrt();
        let unit = if unit.is_finite() { unit.clamp(1.0, 4.0).min(fit).max(0.25) } else { 1.0 };
        let (pw, ph) = (((w as f32) * unit).round() as i32, ((h as f32) * unit).round() as i32);
        let key = {
            let mut hs = std::collections::hash_map::DefaultHasher::new();
            (d.key(), pw, ph, theme::generation(), under.0).hash(&mut hs);
            hs.finish()
        };
        if key != self.key || self.rgba.is_empty() {
            let c = theme::with(|t| rasterize(d, pw as usize, ph as usize, unit, t, under));
            c.to_rgba8(&mut self.rgba);
            self.key = key;
            self.size = (pw, ph);
            self.renders += 1;
        }
        // SAFETY: the buffer outlives the image, which is drawn right away.
        if let Ok(mut img) = unsafe { RgbImage::from_data(&self.rgba, self.size.0, self.size.1, ColorDepth::Rgba8) } {
            if self.size != (w, h) {
                img.scale(w, h, false, true);
            }
            img.draw(x, y, w, h);
        }
        theme::with(|t| draw_text(d, x, y, t, under));
    }

    /// How many times the shapes were rasterized (tests, benchmarks).
    pub fn renders(&self) -> u64 {
        self.renders
    }

    /// Bytes held by the cached picture.
    pub fn memory(&self) -> usize {
        self.rgba.capacity()
    }
}

/// The shapes of `d` on a `pw × ph` canvas (`unit` pixels per diagram pixel).
pub fn rasterize(d: &Diagram, pw: usize, ph: usize, unit: f32, t: &Theme, under: Rgb) -> Canvas {
    let mut c = Canvas::new(pw.max(1), ph.max(1));
    c.set_unit(unit);
    let col = |r: Role| -> Color { color(r, t, under).into() };
    for it in &d.scene.items {
        match it {
            Item::Fill { points, role } if points.len() >= 3 => {
                c.fill(&polygon(points, true), col(*role));
            }
            Item::Stroke { points, closed, width, dash, role } if points.len() >= 2 => {
                let st = Stroke::new(*width).join(Join::Round).cap(if dash.is_some() { Cap::Butt } else { Cap::Round });
                match dash {
                    Some((on, off)) if *on > 0.1 && *off > 0.1 => {
                        let mut pts = points.clone();
                        if *closed {
                            pts.push(points[0]);
                        }
                        for seg in dashes(&pts, *on, *off) {
                            c.stroke(&polygon(&seg, false), &st, col(*role));
                        }
                    }
                    _ if !*closed => c.stroke(&polygon(&simplify(points, 0.3 / unit), false), &st, col(*role)),
                    _ => c.stroke(&polygon(points, true), &st, col(*role)),
                }
            }
            _ => {}
        }
    }
    c
}

fn polygon(points: &[[f32; 2]], closed: bool) -> Path {
    let mut p = Path::new().move_to(points[0][0], points[0][1]);
    for q in &points[1..] {
        p = p.line_to(q[0], q[1]);
    }
    if closed {
        p = p.close();
    }
    p
}

/// `pts` with the points that bend it less than `eps` dropped
/// (Ramer–Douglas–Peucker): flattened curves stroke as a few long
/// segments, so the joins of many short ones do not speckle thin lines.
fn simplify(pts: &[[f32; 2]], eps: f32) -> Vec<[f32; 2]> {
    if pts.len() < 3 {
        return pts.to_vec();
    }
    let mut keep = vec![false; pts.len()];
    keep[0] = true;
    keep[pts.len() - 1] = true;
    let mut stack = vec![(0usize, pts.len() - 1)];
    while let Some((a, b)) = stack.pop() {
        if b <= a + 1 {
            continue;
        }
        let (mut far, mut at) = (0.0f32, a);
        for (i, p) in pts.iter().enumerate().take(b).skip(a + 1) {
            let d = super::common::seg_dist(*p, pts[a], pts[b]);
            if d > far {
                far = d;
                at = i;
            }
        }
        if far > eps {
            keep[at] = true;
            stack.push((a, at));
            stack.push((at, b));
        }
    }
    pts.iter().zip(keep).filter(|(_, k)| *k).map(|(p, _)| *p).collect()
}

/// The "on" pieces of a dashed polyline.
fn dashes(pts: &[[f32; 2]], on: f32, off: f32) -> Vec<Vec<[f32; 2]>> {
    let mut out = Vec::new();
    let mut cur: Vec<[f32; 2]> = vec![pts[0]];
    let (mut drawing, mut left) = (true, on);
    for w in pts.windows(2) {
        let (mut a, b) = (w[0], w[1]);
        let mut len = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
        while len > 1e-4 {
            let step = left.min(len);
            let t = step / len;
            let p = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
            if drawing {
                cur.push(p);
            }
            len -= step;
            left -= step;
            a = p;
            if left <= 1e-4 {
                if drawing {
                    if cur.len() >= 2 {
                        out.push(std::mem::take(&mut cur));
                    }
                    cur.clear();
                } else {
                    cur = vec![p];
                }
                drawing = !drawing;
                left = if drawing { on } else { off };
            }
        }
        if out.len() > 4096 {
            break;
        }
    }
    if drawing && cur.len() >= 2 {
        out.push(cur);
    }
    out
}

/// Draws the text items of `d` with its top left at (x, y).
pub fn draw_text(d: &Diagram, x: i32, y: i32, t: &Theme, under: Rgb) {
    for it in &d.scene.items {
        let Item::Text { lines, x: tx, y: ty, width, size, line_height, bold, role, align } = it else { continue };
        let size_px = size.round().max(6.0) as i32;
        let n = lines.len() as f32;
        let c = color(*role, t, under);
        for (i, line) in lines.iter().enumerate() {
            let cy = ty + (i as f32 - (n - 1.0) / 2.0) * line_height;
            let (bx, bw, al) = match align {
                Align::Left => (tx - 1.0, width + 8.0, fltk::enums::Align::Left),
                Align::Center => (tx - width / 2.0 - 6.0, width + 12.0, fltk::enums::Align::Center),
            };
            let by = cy - line_height / 2.0;
            crate::paint::text(
                line,
                x + bx.round() as i32,
                y + by.round() as i32,
                bw.ceil() as i32 + 2,
                line_height.ceil() as i32 + 1,
                size_px,
                *bold,
                c,
                al,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simplify_keeps_corners_and_drops_flat_points() {
        let line: Vec<[f32; 2]> = (0..=10).map(|i| [i as f32, 0.0]).collect();
        assert_eq!(simplify(&line, 0.3), vec![[0.0, 0.0], [10.0, 0.0]]);
        let corner = [[0.0, 0.0], [5.0, 0.0], [5.0, 5.0]];
        assert_eq!(simplify(&corner, 0.3).len(), 3);
    }

    #[test]
    fn dashes_alternate() {
        let d = dashes(&[[0.0, 0.0], [20.0, 0.0]], 4.0, 4.0);
        assert_eq!(d.len(), 3);
        assert!((d[0][1][0] - 4.0).abs() < 1e-3 && (d[1][0][0] - 8.0).abs() < 1e-3);
    }
}
