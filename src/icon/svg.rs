//! The SVG subset icon sets use (Lucide, Feather, Tabler, most UI icons):
//! `<svg viewBox>`, `<g>`, `<path d>` with every command (arcs included),
//! `<circle>`, `<ellipse>`, `<rect>` (with `rx`/`ry`), `<line>`,
//! `<polyline>`, `<polygon>`; the presentation attributes `fill`,
//! `stroke`, `stroke-width`, `stroke-linecap`, `stroke-linejoin`,
//! `stroke-miterlimit`, `fill-rule`, `opacity`, `fill-opacity`,
//! `stroke-opacity`, also inside `style="…"`; `transform` (matrix,
//! translate, scale, rotate, skewX, skewY). `currentColor` (and a fill
//! left unspecified) is the tint chosen when the icon is drawn.
//!
//! Not supported (ignored, never an error): gradients and patterns
//! (`url(#…)` paints draw in the tint), `<use>`, `<text>`, `<image>`,
//! clip paths, masks, filters, CSS `<style>` sheets, dashes. The contents
//! of `<defs>`, `<clipPath>`, `<mask>`, `<symbol>`, `<title>`, `<desc>`,
//! `<metadata>` and `<style>` are skipped.

use crate::gfx::{Cap, Color, Join, Path, Point, Stroke};
use std::fmt;

/// Largest SVG source accepted (bytes).
pub const MAX_SOURCE: usize = 1 << 20;
/// Most shapes in one icon.
pub const MAX_SHAPES: usize = 4096;
/// Most path segments in one icon (arcs count as their cubics).
pub const MAX_SEGMENTS: usize = 200_000;

/// Why an SVG could not be parsed. `offset` is the byte where it stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SvgError {
    pub message: String,
    pub offset: usize,
}

impl fmt::Display for SvgError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at byte {}", self.message, self.offset)
    }
}

impl std::error::Error for SvgError {}

fn err<T>(message: &str, offset: usize) -> Result<T, SvgError> {
    Err(SvgError { message: message.to_string(), offset })
}

/// A paint of an icon shape: the tint (`currentColor`) or a fixed color
/// written in the SVG (multicolor logos). Fixed colors keep their own
/// alpha (`fill-opacity`, `#rrggbbaa`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Ink {
    Current { alpha: f32 },
    Color(Color),
}

impl Ink {
    /// The color with `tint` for `currentColor`.
    pub fn resolve(self, tint: Color) -> Color {
        match self {
            Ink::Current { alpha } => tint.with_alpha(tint.a * alpha),
            Ink::Color(c) => c,
        }
    }

    fn with_alpha(self, a: f32) -> Ink {
        match self {
            Ink::Current { alpha } => Ink::Current { alpha: alpha * a },
            Ink::Color(c) => Ink::Color(c.with_alpha(c.a * a)),
        }
    }
}

/// One drawable element, in viewBox units (transforms already applied).
#[derive(Debug, Clone, PartialEq)]
pub struct Shape {
    pub path: Path,
    pub fill: Option<Ink>,
    /// `fill-rule="evenodd"`.
    pub even_odd: bool,
    pub stroke: Option<(Stroke, Ink)>,
}

/// Parsed icon geometry: `view_box` (x, y, w, h) and its shapes.
#[derive(Debug, Clone, PartialEq)]
pub struct Parsed {
    pub view_box: (f32, f32, f32, f32),
    pub shapes: Vec<Shape>,
}

// ------------------------------------------------------------ affine

/// `[a, b, c, d, e, f]`: x' = a x + c y + e, y' = b x + d y + f.
type Mat = [f32; 6];
const ID: Mat = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

fn mul(m: &Mat, n: &Mat) -> Mat {
    // m then n applied inside: m * n.
    [
        m[0] * n[0] + m[2] * n[1],
        m[1] * n[0] + m[3] * n[1],
        m[0] * n[2] + m[2] * n[3],
        m[1] * n[2] + m[3] * n[3],
        m[0] * n[4] + m[2] * n[5] + m[4],
        m[1] * n[4] + m[3] * n[5] + m[5],
    ]
}

fn apply(m: &Mat, (x, y): Point) -> Point {
    (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5])
}

fn scale_of(m: &Mat) -> f32 {
    (m[0] * m[3] - m[1] * m[2]).abs().sqrt()
}

// ------------------------------------------------------- path builder

/// Builds a [`Path`] in user units through a transform, counting segments.
struct Builder<'a> {
    m: Mat,
    path: Path,
    segments: &'a mut usize,
    open: bool,
}

impl Builder<'_> {
    fn count(&mut self, n: usize) -> bool {
        *self.segments += n;
        *self.segments <= MAX_SEGMENTS
    }
    fn take(&mut self) -> Path {
        std::mem::take(&mut self.path)
    }
    fn move_to(&mut self, p: Point) {
        let (x, y) = apply(&self.m, p);
        self.path = self.take().move_to(x, y);
        self.open = true;
    }
    fn line_to(&mut self, p: Point) {
        let (x, y) = apply(&self.m, p);
        self.path = self.take().line_to(x, y);
    }
    fn quad_to(&mut self, c: Point, p: Point) {
        let (c, p) = (apply(&self.m, c), apply(&self.m, p));
        self.path = self.take().quad_to(c.0, c.1, p.0, p.1);
    }
    fn cubic_to(&mut self, a: Point, b: Point, p: Point) {
        let (a, b, p) = (apply(&self.m, a), apply(&self.m, b), apply(&self.m, p));
        self.path = self.take().cubic_to(a.0, a.1, b.0, b.1, p.0, p.1);
    }
    fn close(&mut self) {
        if self.open {
            self.path = self.take().close();
            self.open = false;
        }
    }

    /// SVG endpoint arc (`A rx ry phi large sweep x y`) from `p0`, as
    /// cubics (one per quarter turn at most). Returns false over budget.
    fn arc(&mut self, p0: Point, rx: f32, ry: f32, phi_deg: f32, large: bool, sweep: bool, p1: Point) -> bool {
        let (mut rx, mut ry) = (rx.abs(), ry.abs());
        if (p0.0 - p1.0).abs() < 1e-6 && (p0.1 - p1.1).abs() < 1e-6 {
            return true;
        }
        if rx < 1e-6 || ry < 1e-6 {
            self.line_to(p1);
            return self.count(1);
        }
        let phi = phi_deg.to_radians();
        let (cp, sp) = (phi.cos(), phi.sin());
        let (dx, dy) = ((p0.0 - p1.0) / 2.0, (p0.1 - p1.1) / 2.0);
        let x1 = cp * dx + sp * dy;
        let y1 = -sp * dx + cp * dy;
        // Radii too small for the chord grow until the arc fits (F.6.6).
        let lambda = (x1 * x1) / (rx * rx) + (y1 * y1) / (ry * ry);
        if lambda > 1.0 {
            let s = lambda.sqrt();
            rx *= s;
            ry *= s;
        }
        let num = rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1;
        let den = rx * rx * y1 * y1 + ry * ry * x1 * x1;
        let mut co = if den > 0.0 { (num / den).max(0.0).sqrt() } else { 0.0 };
        if large == sweep {
            co = -co;
        }
        let cx1 = co * rx * y1 / ry;
        let cy1 = -co * ry * x1 / rx;
        let cx = cp * cx1 - sp * cy1 + (p0.0 + p1.0) / 2.0;
        let cy = sp * cx1 + cp * cy1 + (p0.1 + p1.1) / 2.0;
        let angle = |ux: f32, uy: f32, vx: f32, vy: f32| {
            let dot = ux * vx + uy * vy;
            let len = ((ux * ux + uy * uy) * (vx * vx + vy * vy)).sqrt().max(1e-12);
            let a = (dot / len).clamp(-1.0, 1.0).acos();
            if ux * vy - uy * vx < 0.0 { -a } else { a }
        };
        let (ux, uy) = ((x1 - cx1) / rx, (y1 - cy1) / ry);
        let (vx, vy) = ((-x1 - cx1) / rx, (-y1 - cy1) / ry);
        let theta1 = angle(1.0, 0.0, ux, uy);
        let mut dtheta = angle(ux, uy, vx, vy);
        let tau = std::f32::consts::TAU;
        if !sweep && dtheta > 0.0 {
            dtheta -= tau;
        } else if sweep && dtheta < 0.0 {
            dtheta += tau;
        }
        let parts = (dtheta.abs() / (std::f32::consts::FRAC_PI_2) - 1e-3).ceil().max(1.0) as usize;
        if !self.count(parts) {
            return false;
        }
        let step = dtheta / parts as f32;
        let k = 4.0 / 3.0 * (step / 4.0).tan();
        let pt = |t: f32| {
            let (c, s) = (t.cos(), t.sin());
            (cx + rx * c * cp - ry * s * sp, cy + rx * c * sp + ry * s * cp)
        };
        let der = |t: f32| {
            let (c, s) = (t.cos(), t.sin());
            (-rx * s * cp - ry * c * sp, -rx * s * sp + ry * c * cp)
        };
        let mut t = theta1;
        for i in 0..parts {
            let t2 = t + step;
            let (a, da) = (pt(t), der(t));
            let (b, db) = (if i + 1 == parts { p1 } else { pt(t2) }, der(t2));
            self.cubic_to((a.0 + k * da.0, a.1 + k * da.1), (b.0 - k * db.0, b.1 - k * db.1), b);
            t = t2;
        }
        true
    }
}

// --------------------------------------------------------- numbers

struct Nums<'a> {
    s: &'a [u8],
    i: usize,
    base: usize,
}

impl<'a> Nums<'a> {
    fn new(s: &'a str, base: usize) -> Self {
        Nums { s: s.as_bytes(), i: 0, base }
    }
    fn skip(&mut self) {
        while self.i < self.s.len() && matches!(self.s[self.i], b' ' | b'\t' | b'\n' | b'\r' | b',') {
            self.i += 1;
        }
    }
    fn done(&mut self) -> bool {
        self.skip();
        self.i >= self.s.len()
    }
    fn peek_number(&mut self) -> bool {
        self.skip();
        self.i < self.s.len() && matches!(self.s[self.i], b'0'..=b'9' | b'-' | b'+' | b'.')
    }
    fn number(&mut self) -> Result<f32, SvgError> {
        self.skip();
        let start = self.i;
        let s = self.s;
        let mut j = self.i;
        if j < s.len() && matches!(s[j], b'+' | b'-') {
            j += 1;
        }
        let mut digits = 0;
        while j < s.len() && s[j].is_ascii_digit() {
            j += 1;
            digits += 1;
        }
        if j < s.len() && s[j] == b'.' {
            j += 1;
            while j < s.len() && s[j].is_ascii_digit() {
                j += 1;
                digits += 1;
            }
        }
        if digits == 0 {
            return err("expected a number", self.base + start);
        }
        if j < s.len() && matches!(s[j], b'e' | b'E') {
            let mut k = j + 1;
            if k < s.len() && matches!(s[k], b'+' | b'-') {
                k += 1;
            }
            if k < s.len() && s[k].is_ascii_digit() {
                while k < s.len() && s[k].is_ascii_digit() {
                    k += 1;
                }
                j = k;
            }
        }
        self.i = j;
        let txt = std::str::from_utf8(&s[start..j]).unwrap_or("0");
        match txt.parse::<f32>() {
            Ok(v) if v.is_finite() => Ok(v),
            _ => err("number out of range", self.base + start),
        }
    }
    /// Arc flags may be written without separators (`a1 1 0 01.5.5`).
    fn flag(&mut self) -> Result<bool, SvgError> {
        self.skip();
        match self.s.get(self.i) {
            Some(b'0') => { self.i += 1; Ok(false) }
            Some(b'1') => { self.i += 1; Ok(true) }
            _ => err("expected an arc flag (0 or 1)", self.base + self.i),
        }
    }
    fn pair(&mut self) -> Result<Point, SvgError> {
        Ok((self.number()?, self.number()?))
    }
}

/// Parses a path `d` into `b`. Stops quietly at the first error (as
/// browsers do: what was parsed so far is drawn) but reports it.
fn path_data(d: &str, base: usize, b: &mut Builder) -> Result<(), SvgError> {
    let mut n = Nums::new(d, base);
    let mut cur: Point = (0.0, 0.0);
    let mut start: Point = (0.0, 0.0);
    // Last control point of a cubic (for S) or a quad (for T).
    let mut last_c: Option<Point> = None;
    let mut last_q: Option<Point> = None;
    let mut cmd = 0u8;
    let mut first = true;
    loop {
        if n.done() {
            return Ok(());
        }
        let c = n.s[n.i];
        if c.is_ascii_alphabetic() {
            cmd = c;
            n.i += 1;
        } else if cmd == 0 || matches!(cmd, b'z' | b'Z') {
            return err("path data must start with a command", base + n.i);
        }
        if first && !matches!(cmd, b'M' | b'm') {
            return err("path data must start with M", base + n.i);
        }
        first = false;
        let rel = cmd.is_ascii_lowercase();
        let off = |p: Point, cur: Point| if rel { (p.0 + cur.0, p.1 + cur.1) } else { p };
        if !b.count(1) {
            return err("too many path segments", base + n.i);
        }
        match cmd.to_ascii_uppercase() {
            b'M' => {
                let p = off(n.pair()?, cur);
                b.move_to(p);
                cur = p;
                start = p;
                last_c = None;
                last_q = None;
                // Further pairs are line-tos.
                cmd = if rel { b'l' } else { b'L' };
                continue;
            }
            b'L' => {
                let p = off(n.pair()?, cur);
                b.line_to(p);
                cur = p;
                last_c = None;
                last_q = None;
            }
            b'H' => {
                let x = n.number()?;
                let p = (if rel { cur.0 + x } else { x }, cur.1);
                b.line_to(p);
                cur = p;
                last_c = None;
                last_q = None;
            }
            b'V' => {
                let y = n.number()?;
                let p = (cur.0, if rel { cur.1 + y } else { y });
                b.line_to(p);
                cur = p;
                last_c = None;
                last_q = None;
            }
            b'C' => {
                let c1 = off(n.pair()?, cur);
                let c2 = off(n.pair()?, cur);
                let p = off(n.pair()?, cur);
                b.cubic_to(c1, c2, p);
                cur = p;
                last_c = Some(c2);
                last_q = None;
            }
            b'S' => {
                let c1 = last_c.map_or(cur, |c| (2.0 * cur.0 - c.0, 2.0 * cur.1 - c.1));
                let c2 = off(n.pair()?, cur);
                let p = off(n.pair()?, cur);
                b.cubic_to(c1, c2, p);
                cur = p;
                last_c = Some(c2);
                last_q = None;
            }
            b'Q' => {
                let c1 = off(n.pair()?, cur);
                let p = off(n.pair()?, cur);
                b.quad_to(c1, p);
                cur = p;
                last_q = Some(c1);
                last_c = None;
            }
            b'T' => {
                let c1 = last_q.map_or(cur, |c| (2.0 * cur.0 - c.0, 2.0 * cur.1 - c.1));
                let p = off(n.pair()?, cur);
                b.quad_to(c1, p);
                cur = p;
                last_q = Some(c1);
                last_c = None;
            }
            b'A' => {
                let rx = n.number()?;
                let ry = n.number()?;
                let phi = n.number()?;
                let large = n.flag()?;
                let sweep = n.flag()?;
                let p = off(n.pair()?, cur);
                if !b.arc(cur, rx, ry, phi, large, sweep, p) {
                    return err("too many path segments", base + n.i);
                }
                cur = p;
                last_c = None;
                last_q = None;
            }
            b'Z' => {
                b.close();
                cur = start;
                last_c = None;
                last_q = None;
                // A number after Z without a command is an error; a new
                // command may follow (often `m` for the next subpath).
                if n.peek_number() {
                    return err("number after Z", base + n.i);
                }
                // A drawing command after Z starts at the subpath start
                // (gfx::Path continues from there after a close).
                continue;
            }
            _ => return err("unknown path command", base + n.i - 1),
        }
    }
}

// ------------------------------------------------------------- XML

struct Tag<'a> {
    name: &'a str,
    attrs: Vec<(&'a str, String, usize)>,
    self_closing: bool,
    closing: bool,
}

fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(end) = rest.find(';') else { break };
        let ent = &rest[1..end];
        let ch = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ if ent.starts_with("#x") => u32::from_str_radix(&ent[2..], 16).ok().and_then(char::from_u32),
            _ if ent.starts_with('#') => ent[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Next tag at or after `*pos`, skipping text, comments, processing
/// instructions, doctype and CDATA.
fn next_tag<'a>(src: &'a str, pos: &mut usize) -> Result<Option<Tag<'a>>, SvgError> {
    let b = src.as_bytes();
    loop {
        let Some(rel) = src[*pos..].find('<') else { return Ok(None) };
        let at = *pos + rel;
        let rest = &src[at..];
        let skip_to = |pat: &str, from: usize| -> Result<usize, SvgError> {
            match src[from..].find(pat) {
                Some(i) => Ok(from + i + pat.len()),
                None => err("unterminated markup", from),
            }
        };
        if rest.starts_with("<!--") {
            *pos = skip_to("-->", at + 4)?;
            continue;
        }
        if rest.starts_with("<![CDATA[") {
            *pos = skip_to("]]>", at + 9)?;
            continue;
        }
        if rest.starts_with("<?") {
            *pos = skip_to("?>", at + 2)?;
            continue;
        }
        if rest.starts_with("<!") {
            *pos = skip_to(">", at + 2)?;
            continue;
        }
        let mut i = at + 1;
        let closing = b.get(i) == Some(&b'/');
        if closing {
            i += 1;
        }
        let name_start = i;
        while i < b.len() && !matches!(b[i], b' ' | b'\t' | b'\n' | b'\r' | b'/' | b'>') {
            i += 1;
        }
        if i == name_start {
            return err("expected a tag name", at);
        }
        let name = &src[name_start..i];
        let mut attrs = Vec::new();
        let mut self_closing = false;
        loop {
            while i < b.len() && matches!(b[i], b' ' | b'\t' | b'\n' | b'\r') {
                i += 1;
            }
            match b.get(i) {
                None => return err("unterminated tag", at),
                Some(b'>') => {
                    i += 1;
                    break;
                }
                Some(b'/') if b.get(i + 1) == Some(&b'>') => {
                    self_closing = true;
                    i += 2;
                    break;
                }
                _ => {}
            }
            let an = i;
            while i < b.len() && !matches!(b[i], b'=' | b' ' | b'\t' | b'\n' | b'\r' | b'>' | b'/') {
                i += 1;
            }
            let aname = &src[an..i];
            if aname.is_empty() {
                return err("bad attribute", i);
            }
            while i < b.len() && matches!(b[i], b' ' | b'\t' | b'\n' | b'\r') {
                i += 1;
            }
            if b.get(i) != Some(&b'=') {
                // Attribute without a value (not valid XML): ignore it.
                continue;
            }
            i += 1;
            while i < b.len() && matches!(b[i], b' ' | b'\t' | b'\n' | b'\r') {
                i += 1;
            }
            let q = match b.get(i) {
                Some(&q @ (b'"' | b'\'')) => q,
                _ => return err("attribute value must be quoted", i),
            };
            let vs = i + 1;
            let Some(ve) = src[vs..].find(q as char).map(|e| vs + e) else { return err("unterminated attribute value", vs) };
            attrs.push((aname, decode_entities(&src[vs..ve]), vs));
            i = ve + 1;
        }
        *pos = i;
        return Ok(Some(Tag { name, attrs, self_closing, closing }));
    }
}

// --------------------------------------------------------- styling

#[derive(Clone)]
struct Ctx {
    fill: Option<Ink>,
    stroke: Option<Ink>,
    width: f32,
    cap: Cap,
    join: Join,
    miter: f32,
    even_odd: bool,
    fill_opacity: f32,
    stroke_opacity: f32,
    opacity: f32,
    m: Mat,
    skip: bool,
}

impl Default for Ctx {
    fn default() -> Ctx {
        Ctx {
            // SVG's initial fill is black; a monochrome icon set means "the
            // icon color" by it, so it is the tint here.
            fill: Some(Ink::Current { alpha: 1.0 }),
            stroke: None,
            width: 1.0,
            cap: Cap::Butt,
            join: Join::Miter(4.0),
            miter: 4.0,
            even_odd: false,
            fill_opacity: 1.0,
            stroke_opacity: 1.0,
            opacity: 1.0,
            m: ID,
            skip: false,
        }
    }
}

const NAMED: &[(&str, u32)] = &[
    ("black", 0x000000),
    ("white", 0xffffff),
    ("red", 0xff0000),
    ("green", 0x008000),
    ("blue", 0x0000ff),
    ("yellow", 0xffff00),
    ("gray", 0x808080),
    ("grey", 0x808080),
];

/// `none` → None, `currentColor` / `url(…)` / unknown → the tint.
fn paint_value(v: &str) -> Option<Ink> {
    let v = v.trim();
    if v == "none" || v == "transparent" {
        return None;
    }
    if v.starts_with('#') {
        if let Some(c) = Color::hex(v) {
            return Some(Ink::Color(c));
        }
        // #rgb
        let h = &v[1..];
        if h.len() == 3 {
            let d: Vec<u32> = h.chars().filter_map(|c| c.to_digit(16)).collect();
            if d.len() == 3 {
                let c = |x: u32| (x * 17) as f32 / 255.0;
                return Some(Ink::Color(Color::rgba(c(d[0]), c(d[1]), c(d[2]), 1.0)));
            }
        }
        return Some(Ink::Current { alpha: 1.0 });
    }
    if let Some(inner) = v.strip_prefix("rgb(").or_else(|| v.strip_prefix("rgba(")).and_then(|s| s.strip_suffix(')')) {
        let parts: Vec<f32> = inner
            .split(|c: char| c == ',' || c == ' ' || c == '/')
            .filter(|s| !s.is_empty())
            .filter_map(|s| s.trim_end_matches('%').parse().ok())
            .collect();
        if parts.len() >= 3 {
            let a = parts.get(3).copied().unwrap_or(1.0);
            return Some(Ink::Color(Color::rgba(parts[0] / 255.0, parts[1] / 255.0, parts[2] / 255.0, a)));
        }
    }
    if let Some((_, rgb)) = NAMED.iter().find(|(n, _)| n.eq_ignore_ascii_case(v)) {
        return Some(Ink::Color(crate::theme::Rgb(*rgb).into()));
    }
    Some(Ink::Current { alpha: 1.0 })
}

fn length(v: &str) -> Option<f32> {
    let t = v.trim().trim_end_matches("px");
    t.parse::<f32>().ok().filter(|x| x.is_finite())
}

fn transform(v: &str, base: usize) -> Result<Mat, SvgError> {
    let mut m = ID;
    let mut rest = v.trim();
    while !rest.is_empty() {
        let Some(open) = rest.find('(') else { return err("bad transform", base) };
        let Some(close) = rest.find(')') else { return err("bad transform", base) };
        let name = rest[..open].trim().trim_start_matches(',').trim();
        let mut n = Nums::new(&rest[open + 1..close], base);
        let mut a = Vec::new();
        while !n.done() {
            a.push(n.number()?);
            if a.len() > 6 {
                return err("bad transform", base);
            }
        }
        let t: Mat = match (name, a.len()) {
            ("matrix", 6) => [a[0], a[1], a[2], a[3], a[4], a[5]],
            ("translate", 1) => [1.0, 0.0, 0.0, 1.0, a[0], 0.0],
            ("translate", 2) => [1.0, 0.0, 0.0, 1.0, a[0], a[1]],
            ("scale", 1) => [a[0], 0.0, 0.0, a[0], 0.0, 0.0],
            ("scale", 2) => [a[0], 0.0, 0.0, a[1], 0.0, 0.0],
            ("rotate", 1) | ("rotate", 3) => {
                let (s, c) = a[0].to_radians().sin_cos();
                let r = [c, s, -s, c, 0.0, 0.0];
                if a.len() == 3 {
                    let (cx, cy) = (a[1], a[2]);
                    mul(&mul(&[1.0, 0.0, 0.0, 1.0, cx, cy], &r), &[1.0, 0.0, 0.0, 1.0, -cx, -cy])
                } else {
                    r
                }
            }
            ("skewX", 1) => [1.0, 0.0, a[0].to_radians().tan(), 1.0, 0.0, 0.0],
            ("skewY", 1) => [1.0, a[0].to_radians().tan(), 0.0, 1.0, 0.0, 0.0],
            _ => return err("unsupported transform", base),
        };
        m = mul(&m, &t);
        rest = rest[close + 1..].trim_start();
    }
    Ok(m)
}

fn apply_style(ctx: &mut Ctx, name: &str, v: &str, base: usize) -> Result<(), SvgError> {
    let num = |v: &str| length(v);
    match name {
        "fill" => ctx.fill = paint_value(v),
        "stroke" => ctx.stroke = paint_value(v),
        "stroke-width" => {
            if let Some(w) = num(v) {
                ctx.width = w.max(0.0);
            }
        }
        "stroke-linecap" => {
            ctx.cap = match v.trim() {
                "round" => Cap::Round,
                "square" => Cap::Square,
                _ => Cap::Butt,
            }
        }
        "stroke-linejoin" => {
            ctx.join = match v.trim() {
                "round" => Join::Round,
                "bevel" => Join::Bevel,
                _ => Join::Miter(ctx.miter),
            }
        }
        "stroke-miterlimit" => {
            if let Some(l) = num(v) {
                ctx.miter = l.max(1.0);
                if let Join::Miter(_) = ctx.join {
                    ctx.join = Join::Miter(ctx.miter);
                }
            }
        }
        "fill-rule" => ctx.even_odd = v.trim() == "evenodd",
        "fill-opacity" => ctx.fill_opacity = num(v).unwrap_or(1.0).clamp(0.0, 1.0),
        "stroke-opacity" => ctx.stroke_opacity = num(v).unwrap_or(1.0).clamp(0.0, 1.0),
        "opacity" => ctx.opacity *= num(v).unwrap_or(1.0).clamp(0.0, 1.0),
        "transform" => ctx.m = mul(&ctx.m, &transform(v, base)?),
        "display" if v.trim() == "none" => ctx.skip = true,
        "visibility" if matches!(v.trim(), "hidden" | "collapse") => ctx.skip = true,
        _ => {}
    }
    Ok(())
}

fn style_of(ctx: &mut Ctx, tag: &Tag) -> Result<(), SvgError> {
    for (k, v, at) in &tag.attrs {
        if *k != "style" {
            apply_style(ctx, k, v, *at)?;
        }
    }
    // Inline CSS wins over presentation attributes.
    if let Some((_, css, at)) = tag.attrs.iter().find(|(k, _, _)| *k == "style") {
        for decl in css.split(';') {
            if let Some((k, v)) = decl.split_once(':') {
                apply_style(ctx, k.trim(), v.trim(), *at)?;
            }
        }
    }
    Ok(())
}

fn attr<'t>(tag: &'t Tag, name: &str) -> Option<(&'t str, usize)> {
    tag.attrs.iter().find(|(k, _, _)| *k == name).map(|(_, v, at)| (v.as_str(), *at))
}

fn num_attr(tag: &Tag, name: &str) -> Option<f32> {
    attr(tag, name).and_then(|(v, _)| length(v))
}

fn points(v: &str, base: usize) -> Result<Vec<Point>, SvgError> {
    let mut n = Nums::new(v, base);
    let mut pts = Vec::new();
    while !n.done() {
        let x = n.number()?;
        // An odd count drops the last coordinate (SVG error handling).
        if n.done() {
            break;
        }
        pts.push((x, n.number()?));
    }
    Ok(pts)
}

/// K for a quarter ellipse with one cubic.
const K: f32 = 0.552_284_8;

fn ellipse(b: &mut Builder, cx: f32, cy: f32, rx: f32, ry: f32) {
    b.move_to((cx + rx, cy));
    b.cubic_to((cx + rx, cy + K * ry), (cx + K * rx, cy + ry), (cx, cy + ry));
    b.cubic_to((cx - K * rx, cy + ry), (cx - rx, cy + K * ry), (cx - rx, cy));
    b.cubic_to((cx - rx, cy - K * ry), (cx - K * rx, cy - ry), (cx, cy - ry));
    b.cubic_to((cx + K * rx, cy - ry), (cx + rx, cy - K * ry), (cx + rx, cy));
    b.close();
}

fn rect(b: &mut Builder, x: f32, y: f32, w: f32, h: f32, rx: f32, ry: f32) {
    let (rx, ry) = (rx.clamp(0.0, w / 2.0), ry.clamp(0.0, h / 2.0));
    if rx <= 0.0 || ry <= 0.0 {
        b.move_to((x, y));
        b.line_to((x + w, y));
        b.line_to((x + w, y + h));
        b.line_to((x, y + h));
        b.close();
        return;
    }
    let (kx, ky) = (K * rx, K * ry);
    b.move_to((x + rx, y));
    b.line_to((x + w - rx, y));
    b.cubic_to((x + w - rx + kx, y), (x + w, y + ry - ky), (x + w, y + ry));
    b.line_to((x + w, y + h - ry));
    b.cubic_to((x + w, y + h - ry + ky), (x + w - rx + kx, y + h), (x + w - rx, y + h));
    b.line_to((x + rx, y + h));
    b.cubic_to((x + rx - kx, y + h), (x, y + h - ry + ky), (x, y + h - ry));
    b.line_to((x, y + ry));
    b.cubic_to((x, y + ry - ky), (x + rx - kx, y), (x + rx, y));
    b.close();
}

/// Parses an SVG document into shapes in viewBox units.
pub fn parse(src: &str) -> Result<Parsed, SvgError> {
    if src.len() > MAX_SOURCE {
        return err("SVG source is larger than 1 MiB", 0);
    }
    let mut pos = 0;
    let mut stack: Vec<(String, Ctx)> = Vec::new();
    let mut ctx = Ctx::default();
    let mut view_box: Option<(f32, f32, f32, f32)> = None;
    let mut size: (Option<f32>, Option<f32>) = (None, None);
    let mut shapes = Vec::new();
    let mut segments = 0usize;
    let mut seen_svg = false;
    while let Some(tag) = next_tag(src, &mut pos)? {
        if tag.closing {
            // Pop to the matching element (tolerates sloppy nesting).
            if let Some(i) = stack.iter().rposition(|(n, _)| n == tag.name) {
                ctx = stack[i].1.clone();
                stack.truncate(i);
            }
            continue;
        }
        let mut el = ctx.clone();
        let local = tag.name.rsplit(':').next().unwrap_or(tag.name);
        if !seen_svg {
            if local != "svg" {
                return err("not an SVG document", pos);
            }
            seen_svg = true;
        }
        if matches!(local, "defs" | "clipPath" | "mask" | "symbol" | "title" | "desc" | "metadata" | "style" | "pattern" | "linearGradient" | "radialGradient" | "filter" | "marker") {
            el.skip = true;
        }
        if !el.skip {
            style_of(&mut el, &tag)?;
        }
        if local == "svg" && view_box.is_none() {
            if let Some((v, at)) = attr(&tag, "viewBox") {
                let mut n = Nums::new(v, at);
                let vb = (n.number()?, n.number()?, n.number()?, n.number()?);
                if vb.2 <= 0.0 || vb.3 <= 0.0 {
                    return err("viewBox width and height must be positive", at);
                }
                view_box = Some(vb);
            }
            size = (num_attr(&tag, "width"), num_attr(&tag, "height"));
        }
        if !el.skip {
            let mut b = Builder { m: el.m, path: Path::new(), segments: &mut segments, open: false };
            let drawn = match local {
                "path" => {
                    if let Some((d, at)) = attr(&tag, "d") {
                        path_data(d, at, &mut b)?;
                    }
                    true
                }
                "circle" => {
                    let r = num_attr(&tag, "r").unwrap_or(0.0);
                    if r > 0.0 {
                        ellipse(&mut b, num_attr(&tag, "cx").unwrap_or(0.0), num_attr(&tag, "cy").unwrap_or(0.0), r, r);
                    }
                    true
                }
                "ellipse" => {
                    let (rx, ry) = (num_attr(&tag, "rx").unwrap_or(0.0), num_attr(&tag, "ry").unwrap_or(0.0));
                    if rx > 0.0 && ry > 0.0 {
                        ellipse(&mut b, num_attr(&tag, "cx").unwrap_or(0.0), num_attr(&tag, "cy").unwrap_or(0.0), rx, ry);
                    }
                    true
                }
                "rect" => {
                    let (w, h) = (num_attr(&tag, "width").unwrap_or(0.0), num_attr(&tag, "height").unwrap_or(0.0));
                    if w > 0.0 && h > 0.0 {
                        let (rx, ry) = match (num_attr(&tag, "rx"), num_attr(&tag, "ry")) {
                            (Some(x), Some(y)) => (x, y),
                            (Some(x), None) => (x, x),
                            (None, Some(y)) => (y, y),
                            (None, None) => (0.0, 0.0),
                        };
                        rect(&mut b, num_attr(&tag, "x").unwrap_or(0.0), num_attr(&tag, "y").unwrap_or(0.0), w, h, rx, ry);
                    }
                    true
                }
                "line" => {
                    let p0 = (num_attr(&tag, "x1").unwrap_or(0.0), num_attr(&tag, "y1").unwrap_or(0.0));
                    let p1 = (num_attr(&tag, "x2").unwrap_or(0.0), num_attr(&tag, "y2").unwrap_or(0.0));
                    b.move_to(p0);
                    b.line_to(p1);
                    true
                }
                "polyline" | "polygon" => {
                    if let Some((v, at)) = attr(&tag, "points") {
                        let pts = points(v, at)?;
                        if !b.count(pts.len()) {
                            return err("too many path segments", at);
                        }
                        for (i, p) in pts.iter().enumerate() {
                            if i == 0 { b.move_to(*p) } else { b.line_to(*p) }
                        }
                        if local == "polygon" && pts.len() > 1 {
                            b.close();
                        }
                    }
                    true
                }
                _ => false,
            };
            if *b.segments > MAX_SEGMENTS {
                return err("too many path segments", pos);
            }
            let path = b.take();
            if drawn && !path.is_empty() {
                let o = el.opacity;
                let fill = el.fill.map(|i| i.with_alpha(el.fill_opacity * o));
                // `line`/`polyline` have no area: their fill draws nothing.
                let fill = if matches!(local, "line") { None } else { fill };
                let stroke = el.stroke.filter(|_| el.width > 0.0).map(|i| {
                    let w = el.width * scale_of(&el.m);
                    (Stroke::new(w).cap(el.cap).join(el.join), i.with_alpha(el.stroke_opacity * o))
                });
                if fill.is_some() || stroke.is_some() {
                    if shapes.len() >= MAX_SHAPES {
                        return err("too many shapes", pos);
                    }
                    shapes.push(Shape { path, fill, even_odd: el.even_odd, stroke });
                }
            }
        }
        if !tag.self_closing {
            stack.push((tag.name.to_string(), ctx.clone()));
            ctx = el;
        }
    }
    if !seen_svg {
        return err("not an SVG document", 0);
    }
    let view_box = view_box.unwrap_or_else(|| (0.0, 0.0, size.0.unwrap_or(24.0).max(1.0), size.1.unwrap_or(24.0).max(1.0)));
    Ok(Parsed { view_box, shapes })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_flags_and_compact_arcs() {
        let mut n = Nums::new("-.5.5 1e-1,2E+1 3", 0);
        assert_eq!(n.number().unwrap(), -0.5);
        assert_eq!(n.number().unwrap(), 0.5);
        assert!((n.number().unwrap() - 0.1).abs() < 1e-6);
        assert_eq!(n.number().unwrap(), 20.0);
        assert_eq!(n.number().unwrap(), 3.0);
        assert!(n.number().is_err());
        // Lucide writes arc flags without separators.
        let p = parse(r#"<svg viewBox="0 0 24 24" fill="none" stroke="currentColor"><path d="m2 16 4.039-9.69a.5.5 0 0 1 .923 0L11 16"/></svg>"#).unwrap();
        assert_eq!(p.shapes.len(), 1);
        let (x0, _y0, x1, _) = p.shapes[0].path.bounds();
        assert!(x0 >= 1.99 && x1 <= 11.01, "{:?}", p.shapes[0].path.bounds());
    }

    #[test]
    fn arcs_land_on_their_end_point_and_bulge_the_right_way() {
        // Half circle from (0,10) to (20,10), radius 10, sweep 1: through (10,0).
        let p = parse(r##"<svg viewBox="0 0 20 20"><path d="M0 10A10 10 0 0 1 20 10" fill="none" stroke="#000"/></svg>"##).unwrap();
        let pts = &p.shapes[0].path.flatten(0.05)[0].0;
        let last = pts[pts.len() - 1];
        assert!((last.0 - 20.0).abs() < 1e-3 && (last.1 - 10.0).abs() < 1e-3);
        let top = pts.iter().map(|p| p.1).fold(f32::MAX, f32::min);
        assert!((top - 0.0).abs() < 0.05, "sweep=1 goes up (y down): {top}");
        // Radii too small are scaled up: still ends at the end point.
        let p = parse(r##"<svg viewBox="0 0 20 20"><path d="M0 0a1 1 0 0 0 10 10" stroke="#000"/></svg>"##).unwrap();
        let pts = &p.shapes[0].path.flatten(0.05)[0].0;
        let last = pts[pts.len() - 1];
        assert!((last.0 - 10.0).abs() < 1e-3 && (last.1 - 10.0).abs() < 1e-3, "{last:?}");
    }

    #[test]
    fn inheritance_style_and_transforms() {
        let p = parse(
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <!-- comment --><title>x</title>
            <g transform="translate(10 0) scale(2)" style="stroke-width:1.5">
              <rect x="1" y="1" width="4" height="2" rx="1"/>
            </g>
            <circle cx="12" cy="12" r="3" fill="#ff0000" stroke="none"/>
            <defs><path d="M0 0L5 5"/></defs>
            <line x1="0" y1="0" x2="4" y2="0" stroke-linecap="round"/>
          </svg>"##,
        )
        .unwrap();
        assert_eq!(p.view_box, (0.0, 0.0, 24.0, 24.0));
        assert_eq!(p.shapes.len(), 3, "defs are skipped");
        let r = &p.shapes[0];
        assert_eq!(r.path.bounds(), (12.0, 2.0, 20.0, 6.0));
        let (s, ink) = r.stroke.unwrap();
        assert!((s.width - 3.0).abs() < 1e-5, "style 1.5 × scale 2: {}", s.width);
        assert_eq!(ink, Ink::Current { alpha: 1.0 });
        assert!(r.fill.is_none());
        let c = &p.shapes[1];
        assert!(c.stroke.is_none());
        assert_eq!(c.fill, Some(Ink::Color(Color::rgba(1.0, 0.0, 0.0, 1.0))));
        assert_eq!(p.shapes[2].stroke.unwrap().0.cap, Cap::Round);
    }

    #[test]
    fn errors_are_values() {
        for bad in [
            "",
            "<html/>",
            "<svg viewBox=\"0 0 0 24\"/>",
            "<svg><path d=\"L 1 2\"/></svg>",
            "<svg><path d=\"M 1\"/></svg>",
            "<svg><path d=\"M1 1 a1 1 0 2 1 3 3\"/></svg>",
            "<svg><rect x=1/></svg>",
            "<svg><!-- unterminated",
            "<svg><path d=\"M1 1Z 3\"/></svg>",
            "<svg><g transform=\"perspective(3)\"></g></svg>",
        ] {
            assert!(parse(bad).is_err(), "{bad:?} should fail");
        }
        let huge = format!("<svg>{}</svg>", " ".repeat(MAX_SOURCE));
        assert!(parse(&huge).is_err());
        let many = format!("<svg><path d=\"M0 0{}\"/></svg>", "l1 1".repeat(MAX_SEGMENTS + 1));
        assert!(parse(&many).is_err_and(|e| e.message.contains("too many")));
        // Odd numbers of polygon coordinates drop the last one.
        let p = parse("<svg><polygon points=\"0,0 10,0 10,10 5\" stroke=\"#000\"/></svg>").unwrap();
        assert_eq!(p.shapes[0].path.bounds(), (0.0, 0.0, 10.0, 10.0));
        // No viewBox: width/height, else 24.
        assert_eq!(parse("<svg width=\"16\" height=\"8\"/>").unwrap().view_box, (0.0, 0.0, 16.0, 8.0));
    }

    #[test]
    fn smooth_curves_reflect_their_control_points() {
        let p = parse(r##"<svg><path d="M0 0C0 10 10 10 10 0S20 -10 20 0Q25 5 30 0T40 0" stroke="#000"/></svg>"##).unwrap();
        let (_, y0, x1, y1) = p.shapes[0].path.bounds();
        // S reflects (10,10) around (10,0) to (10,-10); T reflects (25,5) to (35,-5).
        assert_eq!((x1, y0, y1), (40.0, -10.0, 10.0));
    }
}
