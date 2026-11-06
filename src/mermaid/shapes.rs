//! Node shapes of flowcharts and state diagrams: size around a label,
//! outline (what edges meet) and drawing into a scene.
//!
//! Sizes and outlines follow Rune's `rune-mermaid/src/shape.rs` (MIT,
//! © 2026 Anomaly) for the shapes this module supports.

use super::common::{arc, ellipse, rounded_rect, Label, Pt};
use super::{Align, Item, Role};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NodeShape {
    Rect,
    Round,
    Stadium,
    Subroutine,
    Cylinder,
    Circle,
    DoubleCircle,
    Asymmetric,
    Diamond,
    Hexagon,
    LeanRight,
    LeanLeft,
    TrapBottom,
    TrapTop,
    /// State diagram start: a filled dot.
    Start,
    /// State diagram end: a ring around a dot.
    End,
    /// `<<choice>>`: a small diamond.
    Choice,
    /// `<<fork>>` / `<<join>>`: a bar across the flow.
    Bar,
    /// A note with a folded corner.
    Note,
}

use NodeShape as S;

impl NodeShape {
    /// Shapes whose side facing the next rank is straight.
    pub(crate) fn flat(self) -> bool {
        !matches!(
            self,
            S::Stadium
                | S::Cylinder
                | S::Circle
                | S::DoubleCircle
                | S::Diamond
                | S::Hexagon
                | S::LeanRight
                | S::LeanLeft
                | S::TrapBottom
                | S::TrapTop
                | S::Start
                | S::End
                | S::Choice
        )
    }

    /// Markers that draw no label.
    pub(crate) fn unlabelled(self) -> bool {
        matches!(self, S::Start | S::End | S::Choice | S::Bar)
    }
}

const PAD: [f32; 2] = [28.0, 18.0];

/// The node size around a label `t` (`[w, h]`); `across` swaps a bar.
pub(crate) fn node_size(shape: NodeShape, t: [f32; 2], across: bool) -> [f32; 2] {
    let [w, h] = [t[0] + PAD[0], (t[1] + PAD[1]).max(36.0)];
    let rectw = w.max(56.0);
    match shape {
        S::Rect | S::Round | S::Note => [rectw, h],
        S::Stadium | S::Hexagon => [w + h / 2.0, h],
        S::Subroutine => [w + 16.0, h],
        S::Asymmetric => [w + h / 3.0, h],
        S::Cylinder => [rectw, h + 3.0 * rim(rectw)],
        S::LeanRight | S::LeanLeft | S::TrapBottom | S::TrapTop => [w + h * 0.6, h],
        S::Circle => circle(t, 18.0),
        S::DoubleCircle => circle(t, 28.0),
        S::Diamond => {
            let a = t[0] / 2.0 + t[1] + 12.0;
            let b = t[1] / 2.0 * a / (t[1] + 12.0) + 10.0;
            [2.0 * a, (2.0 * b).max(40.0)]
        }
        S::Start => [16.0, 16.0],
        S::End => [20.0, 20.0],
        S::Choice => [26.0, 26.0],
        S::Bar => {
            if across {
                [8.0, 64.0]
            } else {
                [64.0, 8.0]
            }
        }
    }
}

/// How deep a cylinder `w` wide draws its rims.
pub(crate) fn rim(w: f32) -> f32 {
    w / 2.0 / (2.5 + w / 50.0)
}

fn circle(t: [f32; 2], pad: f32) -> [f32; 2] {
    let d = ((t[0] * t[0] + t[1] * t[1]).sqrt() + pad).max(44.0);
    [d, d]
}

fn bounds(c: Pt, s: [f32; 2]) -> (f32, f32, f32, f32) {
    (c[0] - s[0] / 2.0, c[1] - s[1] / 2.0, c[0] + s[0] / 2.0, c[1] + s[1] / 2.0)
}

/// The outline polygon of a node centred at `c`.
pub(crate) fn outline(shape: NodeShape, c: Pt, s: [f32; 2]) -> Vec<Pt> {
    let (x0, y0, x1, y1) = bounds(c, s);
    let (h, cy) = (s[1], c[1]);
    match shape {
        S::Rect | S::Subroutine => rounded_rect(x0, y0, x1, y1, 3.0),
        S::Round => rounded_rect(x0, y0, x1, y1, 10.0),
        S::Stadium => rounded_rect(x0, y0, x1, y1, h / 2.0),
        S::Circle | S::DoubleCircle | S::Start | S::End => ellipse(c, s[0] / 2.0, s[1] / 2.0),
        S::Diamond | S::Choice => vec![[c[0], y0], [x1, cy], [c[0], y1], [x0, cy]],
        S::Hexagon => {
            let i = h / 4.0;
            vec![[x0 + i, y0], [x1 - i, y0], [x1, cy], [x1 - i, y1], [x0 + i, y1], [x0, cy]]
        }
        S::Asymmetric => vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1], [x0 + h / 3.0, cy]],
        S::LeanRight => vec![[x0 + h * 0.3, y0], [x1, y0], [x1 - h * 0.3, y1], [x0, y1]],
        S::LeanLeft => vec![[x0, y0], [x1 - h * 0.3, y0], [x1, y1], [x0 + h * 0.3, y1]],
        S::TrapBottom => vec![[x0 + h * 0.3, y0], [x1 - h * 0.3, y0], [x1, y1], [x0, y1]],
        S::TrapTop => vec![[x0, y0], [x1, y0], [x1 - h * 0.3, y1], [x0 + h * 0.3, y1]],
        S::Cylinder => {
            let (rx, ry) = (s[0] / 2.0, rim(s[0]));
            let mut pts = arc([c[0], y0 + ry], rx, ry, 180.0, 360.0, 20);
            pts.extend(arc([c[0], y1 - ry], rx, ry, 0.0, 180.0, 20));
            pts
        }
        S::Bar => rounded_rect(x0, y0, x1, y1, 2.0),
        S::Note => vec![[x0, y0], [x1 - 10.0, y0], [x1, y0 + 10.0], [x1, y1], [x0, y1]],
    }
}

/// Where the ray from `c` toward `toward` leaves the node.
pub(crate) fn boundary(shape: NodeShape, c: Pt, size: [f32; 2], toward: Pt) -> Pt {
    let d = [toward[0] - c[0], toward[1] - c[1]];
    if d[0].abs() < 1e-3 && d[1].abs() < 1e-3 {
        return c;
    }
    let poly = outline(shape, c, size);
    let mut best: Option<f32> = None;
    for i in 0..poly.len() {
        let (p, q) = (poly[i], poly[(i + 1) % poly.len()]);
        let e = [q[0] - p[0], q[1] - p[1]];
        let den = d[0] * e[1] - d[1] * e[0];
        if den.abs() < 1e-9 {
            continue;
        }
        let w = [p[0] - c[0], p[1] - c[1]];
        let t = (w[0] * e[1] - w[1] * e[0]) / den;
        let u = (w[0] * d[1] - w[1] * d[0]) / den;
        if t > 0.0 && (0.0..=1.0).contains(&u) && best.map_or(true, |b| t > b) {
            best = Some(t);
        }
    }
    let t = best.unwrap_or(1.0).min(1.0);
    [c[0] + d[0] * t, c[1] + d[1] * t]
}

/// Fill and border roles of a node.
#[derive(Clone, Copy)]
pub(crate) struct Look {
    pub fill: Role,
    pub border: Role,
    pub text: Role,
}

impl Look {
    pub(crate) const NODE: Look = Look { fill: Role::NodeFill, border: Role::NodeBorder, text: Role::Text };
}

fn shape(items: &mut Vec<Item>, pts: Vec<Pt>, fill: Role, border: Role, width: f32) {
    items.push(Item::Fill { points: pts.clone(), role: fill });
    items.push(Item::Stroke { points: pts, closed: true, width, dash: None, role: border });
}

fn line(items: &mut Vec<Item>, pts: Vec<Pt>, role: Role) {
    items.push(Item::Stroke { points: pts, closed: false, width: 1.0, dash: None, role });
}

/// Draws a node of `shape` at `c` with its label.
pub(crate) fn draw(items: &mut Vec<Item>, shape_kind: NodeShape, label: &Label, c: Pt, size: [f32; 2], look: Look) {
    let (x0, y0, x1, y1) = bounds(c, size);
    let mut at = c;
    match shape_kind {
        S::Start => items.push(Item::Fill { points: ellipse(c, size[0] / 2.0, size[1] / 2.0), role: Role::Marker }),
        S::End => {
            shape(items, ellipse(c, size[0] / 2.0, size[1] / 2.0), Role::Background, Role::Marker, 1.5);
            items.push(Item::Fill { points: ellipse(c, size[0] / 2.0 - 4.5, size[1] / 2.0 - 4.5), role: Role::Marker });
        }
        S::Bar => items.push(Item::Fill { points: outline(S::Bar, c, size), role: Role::Marker }),
        S::Note => {
            shape(items, outline(S::Note, c, size), Role::NoteFill, Role::NoteBorder, 1.0);
            line(items, vec![[x1 - 10.0, y0], [x1 - 10.0, y0 + 10.0], [x1, y0 + 10.0]], Role::NoteBorder);
        }
        _ => {
            shape(items, outline(shape_kind, c, size), look.fill, look.border, 1.25);
            match shape_kind {
                S::Subroutine => {
                    line(items, vec![[x0 + 8.0, y0], [x0 + 8.0, y1]], look.border);
                    line(items, vec![[x1 - 8.0, y0], [x1 - 8.0, y1]], look.border);
                }
                S::Cylinder => {
                    let ry = rim(size[0]);
                    line(items, arc([c[0], y0 + ry], size[0] / 2.0, ry, 0.0, 180.0, 20), look.border);
                    at[1] += ry / 2.0;
                }
                S::DoubleCircle => items.push(Item::Stroke {
                    points: ellipse(c, size[0] / 2.0 - 5.0, size[1] / 2.0 - 5.0),
                    closed: true,
                    width: 1.0,
                    dash: None,
                    role: look.border,
                }),
                S::Asymmetric => at[0] += size[1] / 6.0,
                _ => {}
            }
        }
    }
    if !shape_kind.unlabelled() && !label.is_empty() {
        items.push(label.item(at, if shape_kind == S::Note { Role::Text } else { look.text }, Align::Center));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boundary_is_on_the_outline() {
        for s in [S::Rect, S::Round, S::Diamond, S::Circle, S::Hexagon, S::Stadium, S::Cylinder] {
            let size = node_size(s, [60.0, 16.0], false);
            let p = boundary(s, [100.0, 100.0], size, [100.0, 400.0]);
            assert!(p[1] > 100.0 && p[1] <= 100.0 + size[1] / 2.0 + 0.5, "{s:?} {p:?}");
            let poly = outline(s, [100.0, 100.0], size);
            let d = (0..poly.len()).map(|i| super::super::common::seg_dist(p, poly[i], poly[(i + 1) % poly.len()])).fold(f32::MAX, f32::min);
            assert!(d < 0.5, "{s:?} off the outline by {d}");
        }
    }
}
