//! Shared parsing helpers (comments, front matter, statements, label
//! markup, a byte cursor), text measurement with wrapping, and geometry
//! primitives used by every diagram type.
//!
//! Label cleaning, statement splitting and the cursor follow Rune's
//! `rune-mermaid` (MIT, © 2026 Anomaly); see `super` for the notice.

use super::{Align, Error, Item, Limits, Measure, Role};

/// The source without comments, `%%{…}%%` directives and front matter:
/// its front-matter title (if any) and the remaining non-empty lines,
/// trimmed.
pub(crate) fn cleaned(src: &str) -> (Option<String>, Vec<&str>) {
    let mut lines = Vec::new();
    let mut in_directive = false;
    for raw in src.lines() {
        let l = raw.trim();
        if in_directive {
            if let Some(p) = l.find("}%%") {
                in_directive = false;
                let rest = l[p + 3..].trim();
                if !rest.is_empty() && !rest.starts_with("%%") {
                    lines.push(rest);
                }
            }
            continue;
        }
        if l.starts_with("%%{") {
            match l.find("}%%") {
                Some(p) => {
                    let rest = l[p + 3..].trim();
                    if !rest.is_empty() && !rest.starts_with("%%") {
                        lines.push(rest);
                    }
                }
                None => in_directive = true,
            }
            continue;
        }
        if !l.is_empty() && !l.starts_with("%%") {
            lines.push(l);
        }
    }
    let mut title = None;
    if lines.first() == Some(&"---") {
        let mut end = None;
        for (i, l) in lines.iter().enumerate().skip(1) {
            if *l == "---" {
                end = Some(i);
                break;
            }
            if let Some(rest) = l.strip_prefix("title:") {
                let t = clean_quoted(rest, usize::MAX);
                if !t.is_empty() {
                    title = Some(t);
                }
            }
        }
        // An unclosed front matter swallows everything (nothing to draw).
        let end = end.unwrap_or(lines.len().saturating_sub(1));
        lines.drain(..=end.min(lines.len().saturating_sub(1)));
    }
    (title, lines)
}

/// One line split at `;` outside quotes and brackets, trimmed, without
/// empty parts and comments.
pub(crate) fn statements(line: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut start, mut quoted, mut escaped, mut depth) = (0usize, false, false, 0usize);
    for (i, c) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if quoted {
            match c {
                '\\' => escaped = true,
                '"' => quoted = false,
                _ => {}
            }
            continue;
        }
        match c {
            '"' => quoted = true,
            '[' | '(' | '{' => depth += 1,
            ']' | ')' | '}' => depth = depth.saturating_sub(1),
            ';' if depth == 0 => {
                out.push(&line[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&line[start..]);
    out.into_iter()
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.starts_with("%%"))
        .collect()
}

/// [`clean`] after removing one pair of surrounding double quotes.
pub(crate) fn clean_quoted(s: &str, max: usize) -> String {
    let t = s.trim();
    if let Some(inner) = t.strip_prefix('"').and_then(|r| r.strip_suffix('"')) {
        return clean(&inner.replace("\\\"", "\""), max);
    }
    clean(t, max)
}

/// Mermaid label markup to plain text: `<br>` and `\n` breaks, entity
/// codes, Markdown-string backticks and `**`; at most `max` characters
/// (an ellipsis marks the cut).
pub(crate) fn clean(s: &str, max: usize) -> String {
    let s = s.trim();
    let s = s.strip_prefix('`').and_then(|s| s.strip_suffix('`')).unwrap_or(s);
    let mut out = String::with_capacity(s.len().min(max.saturating_mul(4)));
    let mut rest = s;
    while let Some(i) = rest.find(['<', '#', '&', '\\']) {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let head: String = rest.chars().take(6).collect::<String>().to_ascii_lowercase();
        let (rep, len) = if head.starts_with("<br>") {
            ("\n", 4)
        } else if head.starts_with("<br/>") {
            ("\n", 5)
        } else if head.starts_with("<br />") {
            ("\n", 6)
        } else if rest.starts_with("\\n") {
            ("\n", 2)
        } else if rest.starts_with("#quot;") || rest.starts_with("&quot;") {
            ("\"", 6)
        } else if rest.starts_with("#59;") {
            (";", 4)
        } else if rest.starts_with("#35;") {
            ("#", 4)
        } else if rest.starts_with("&amp;") {
            ("&", 5)
        } else if rest.starts_with("&lt;") {
            ("<", 4)
        } else if rest.starts_with("&gt;") {
            (">", 4)
        } else if rest.starts_with("&apos;") || rest.starts_with("#apos;") {
            ("'", 6)
        } else {
            // One whole character (`<`, `#`, `&` and `\` are ASCII).
            (&rest[..1], 1)
        };
        out.push_str(rep);
        rest = &rest[len..];
    }
    out.push_str(rest);
    let out = out.replace("**", "");
    let joined = out.lines().map(str::trim).collect::<Vec<_>>().join("\n");
    truncate(&joined, max)
}

/// At most `max` characters, with "…" in place of the rest.
pub(crate) fn truncate(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((i, _)) if max > 0 => {
            let mut t = s[..i].to_string();
            t.pop();
            t.push('…');
            t
        }
        Some(_) => String::new(),
        None => s.to_string(),
    }
}

/// A byte cursor over one statement.
pub(crate) struct Cur<'a> {
    pub s: &'a str,
    pub i: usize,
}

impl<'a> Cur<'a> {
    pub fn new(s: &'a str) -> Self {
        Cur { s, i: 0 }
    }

    pub fn rest(&self) -> &'a str {
        &self.s[self.i..]
    }

    pub fn peek(&self) -> Option<char> {
        self.rest().chars().next()
    }

    pub fn peek2(&self) -> Option<char> {
        let mut it = self.rest().chars();
        it.next();
        it.next()
    }

    pub fn bump(&mut self) {
        if let Some(c) = self.peek() {
            self.i += c.len_utf8();
        }
    }

    pub fn eat(&mut self, t: &str) -> bool {
        if self.rest().starts_with(t) {
            self.i += t.len();
            true
        } else {
            false
        }
    }

    pub fn skip_ws(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.bump();
        }
    }

    pub fn done(&self) -> bool {
        self.i >= self.s.len()
    }
}

/// A word id: letters, digits, `_`, and `-` or `.` between word characters.
pub(crate) fn ident(c: &mut Cur) -> Option<String> {
    let start = c.i;
    let word = |ch: char| ch.is_alphanumeric() || ch == '_';
    while let Some(ch) = c.peek() {
        if word(ch) || ((ch == '-' || ch == '.') && c.i > start && c.peek2().is_some_and(word)) {
            c.bump();
        } else {
            break;
        }
    }
    (c.i > start).then(|| c.s[start..c.i].to_string())
}

/// Checks the source against the limits before any parsing.
pub(crate) fn check_source(src: &str, limits: &Limits) -> Result<(), Error> {
    if src.len() > limits.max_source {
        return Err(Error::TooLarge("source"));
    }
    if src.lines().count() > limits.max_items * 4 + 64 {
        return Err(Error::TooLarge("lines"));
    }
    Ok(())
}

// ------------------------------------------------------------------ text

/// Text measured for a diagram: its lines (wrapped), size and font.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Label {
    pub lines: Vec<String>,
    pub w: f32,
    pub h: f32,
    pub size: f32,
    pub bold: bool,
    pub lh: f32,
}

impl Label {
    pub fn is_empty(&self) -> bool {
        self.lines.iter().all(|l| l.is_empty())
    }

    pub fn size2(&self) -> [f32; 2] {
        [self.w, self.h]
    }

    /// The label centered at `c` (or starting at `c[0]` for `Align::Left`).
    pub fn item(&self, c: [f32; 2], role: Role, align: Align) -> Item {
        Item::Text {
            lines: self.lines.clone(),
            x: c[0],
            y: c[1],
            width: self.w,
            size: self.size,
            line_height: self.lh,
            bold: self.bold,
            role,
            align,
        }
    }
}

/// Most lines one label wraps to.
const MAX_LINES: usize = 12;

/// Measures `text` at `size`, wrapping lines wider than `wrap` at spaces.
pub(crate) fn measure(m: &mut dyn Measure, text: &str, size: f32, bold: bool, wrap: f32) -> Label {
    let lh = m.line_height(size).max(1.0);
    if text.is_empty() {
        return Label { lines: Vec::new(), w: 0.0, h: 0.0, size, bold, lh };
    }
    let mut lines: Vec<String> = Vec::new();
    for raw in text.split('\n') {
        if lines.len() >= MAX_LINES {
            break;
        }
        if m.width(raw, size, bold) <= wrap || !raw.contains(' ') {
            lines.push(raw.to_string());
            continue;
        }
        let mut cur = String::new();
        for word in raw.split(' ').filter(|w| !w.is_empty()) {
            let trial = if cur.is_empty() { word.to_string() } else { format!("{cur} {word}") };
            if !cur.is_empty() && m.width(&trial, size, bold) > wrap {
                lines.push(std::mem::take(&mut cur));
                cur = word.to_string();
                if lines.len() >= MAX_LINES {
                    break;
                }
            } else {
                cur = trial;
            }
        }
        if !cur.is_empty() && lines.len() < MAX_LINES {
            lines.push(cur);
        }
    }
    let w = lines.iter().map(|l| m.width(l, size, bold)).fold(0.0, f32::max).ceil();
    let h = (lines.len() as f32 * lh).ceil();
    Label { lines, w, h, size, bold, lh }
}

// -------------------------------------------------------------- geometry

pub(crate) type Pt = [f32; 2];

pub(crate) fn dist(a: Pt, b: Pt) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

pub(crate) fn lerp(a: Pt, b: Pt, t: f32) -> Pt {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}

pub(crate) fn unit(a: Pt, b: Pt) -> Pt {
    let d = dist(a, b);
    if d < 1e-6 {
        [0.0, 1.0]
    } else {
        [(b[0] - a[0]) / d, (b[1] - a[1]) / d]
    }
}

pub(crate) fn rect(x0: f32, y0: f32, x1: f32, y1: f32) -> Vec<Pt> {
    vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]]
}

/// A rounded rectangle as a polygon (each corner a quarter circle).
pub(crate) fn rounded_rect(x0: f32, y0: f32, x1: f32, y1: f32, r: f32) -> Vec<Pt> {
    let r = r.min((x1 - x0) / 2.0).min((y1 - y0) / 2.0).max(0.0);
    if r < 0.5 {
        return rect(x0, y0, x1, y1);
    }
    let mut p = Vec::with_capacity(36);
    p.extend(arc([x1 - r, y0 + r], r, r, 270.0, 360.0, 8));
    p.extend(arc([x1 - r, y1 - r], r, r, 0.0, 90.0, 8));
    p.extend(arc([x0 + r, y1 - r], r, r, 90.0, 180.0, 8));
    p.extend(arc([x0 + r, y0 + r], r, r, 180.0, 270.0, 8));
    p
}

/// Points on an elliptic arc from `a0` to `a1` degrees (y down, clockwise).
pub(crate) fn arc(c: Pt, rx: f32, ry: f32, a0: f32, a1: f32, steps: usize) -> Vec<Pt> {
    let steps = steps.max(1);
    (0..=steps)
        .map(|i| {
            let a = (a0 + (a1 - a0) * i as f32 / steps as f32).to_radians();
            [c[0] + rx * a.cos(), c[1] + ry * a.sin()]
        })
        .collect()
}

pub(crate) fn ellipse(c: Pt, rx: f32, ry: f32) -> Vec<Pt> {
    let steps = ((rx.max(ry) * 0.9) as usize).clamp(16, 64);
    let mut p = arc(c, rx, ry, 0.0, 360.0, steps);
    p.pop();
    p
}

/// A filled arrowhead with its tip at `tip`, pointing along `dir`.
pub(crate) fn arrowhead(tip: Pt, dir: Pt, len: f32, width: f32) -> Vec<Pt> {
    let back = [tip[0] - dir[0] * len, tip[1] - dir[1] * len];
    let n = [-dir[1] * width / 2.0, dir[0] * width / 2.0];
    vec![tip, [back[0] + n[0], back[1] + n[1]], [back[0] - n[0], back[1] - n[1]]]
}

/// Whether `p` lies inside the polygon (even-odd).
pub(crate) fn inside(poly: &[Pt], p: Pt) -> bool {
    let mut odd = false;
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        if (a[1] > p[1]) != (b[1] > p[1]) && p[0] < a[0] + (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) {
            odd = !odd;
        }
    }
    odd
}

pub(crate) fn seg_dist(p: Pt, a: Pt, b: Pt) -> f32 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let l = dx * dx + dy * dy;
    let t = if l < 1e-6 { 0.0 } else { (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / l).clamp(0.0, 1.0) };
    dist(p, [a[0] + dx * t, a[1] + dy * t])
}

/// The runs of `curve` outside `r` (`[x0, y0, x1, y1]`), cut at its edge.
pub(crate) fn outside(curve: &[Pt], r: [f32; 4]) -> Vec<Vec<Pt>> {
    let mut runs: Vec<Vec<Pt>> = vec![Vec::new()];
    for w in curve.windows(2) {
        let (a, b) = (w[0], w[1]);
        if let Some(run) = runs.last_mut() {
            if run.is_empty() {
                run.push(a);
            }
        }
        match inside_span(a, b, r) {
            None => {
                if let Some(run) = runs.last_mut() {
                    run.push(b);
                }
            }
            Some((t0, t1)) => {
                if t0 > 0.0 {
                    if let Some(run) = runs.last_mut() {
                        run.push(lerp(a, b, t0));
                    }
                }
                let mut next = Vec::new();
                if t1 < 1.0 {
                    next.extend([lerp(a, b, t1), b]);
                }
                runs.push(next);
            }
        }
    }
    runs.retain(|r| r.len() >= 2);
    runs
}

/// The part of `a`→`b` inside `r`, as a range of `t` (Liang–Barsky).
fn inside_span(a: Pt, b: Pt, r: [f32; 4]) -> Option<(f32, f32)> {
    let d = [b[0] - a[0], b[1] - a[1]];
    let (mut t0, mut t1) = (0.0f32, 1.0f32);
    for (p, q) in [(-d[0], a[0] - r[0]), (d[0], r[2] - a[0]), (-d[1], a[1] - r[1]), (d[1], r[3] - a[1])] {
        if p.abs() < 1e-9 {
            if q < 0.0 {
                return None;
            }
        } else {
            let k = q / p;
            if p < 0.0 {
                t0 = t0.max(k)
            } else {
                t1 = t1.min(k)
            }
        }
    }
    (t0 < t1).then_some((t0, t1))
}

/// A cubic from `a` to `b` leaving and arriving along `y`, flattened.
pub(crate) fn cubic(a: Pt, b: Pt) -> Vec<Pt> {
    let dy = b[1] - a[1];
    if (b[0] - a[0]).abs() < 0.25 {
        return vec![a, b];
    }
    let (c1, c2) = ([a[0], a[1] + dy * 0.5], [b[0], b[1] - dy * 0.5]);
    let len = dist(a, c1) + dist(c1, c2) + dist(c2, b);
    let steps = (len / 3.0).ceil().clamp(8.0, 96.0) as usize;
    (0..=steps)
        .map(|s| {
            let t = s as f32 / steps as f32;
            let u = 1.0 - t;
            let (b0, b1, b2, b3) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            [
                b0 * a[0] + b1 * c1[0] + b2 * c2[0] + b3 * b[0],
                b0 * a[1] + b1 * c1[1] + b2 * c2[1] + b3 * b[1],
            ]
        })
        .collect()
}

/// The curve through `anchors`, one [`cubic`] per pair.
pub(crate) fn curve(anchors: &[Pt]) -> Vec<Pt> {
    let mut out = anchors.first().map(|a| vec![*a]).unwrap_or_default();
    for w in anchors.windows(2) {
        out.extend(cubic(w[0], w[1]).into_iter().skip(1));
    }
    out
}

/// A smooth polyline through `pts` (Catmull-Rom), for self-loops.
pub(crate) fn smooth(pts: &[Pt]) -> Vec<Pt> {
    if pts.len() < 3 {
        return pts.to_vec();
    }
    let mut out = vec![pts[0]];
    for i in 0..pts.len() - 1 {
        let p0 = pts[i.saturating_sub(1)];
        let (p1, p2) = (pts[i], pts[i + 1]);
        let p3 = pts[(i + 2).min(pts.len() - 1)];
        for s in 1..=8 {
            let t = s as f32 / 8.0;
            let (t2, t3) = (t * t, t * t * t);
            let f = |a: f32, b: f32, c: f32, d: f32| {
                0.5 * (2.0 * b + (-a + c) * t + (2.0 * a - 5.0 * b + 4.0 * c - d) * t2 + (-a + 3.0 * b - 3.0 * c + d) * t3)
            };
            out.push([f(p0[0], p1[0], p2[0], p3[0]), f(p0[1], p1[1], p2[1], p3[1])]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleaned_drops_comments_directives_and_front_matter() {
        let (title, lines) = cleaned("---\ntitle: \"Plan\"\n---\n%%{init: {}}%%\ngraph TD\n  %% c\n  A-->B\n");
        assert_eq!(title.as_deref(), Some("Plan"));
        assert_eq!(lines, vec!["graph TD", "A-->B"]);
    }

    #[test]
    fn statements_split_outside_quotes_and_brackets() {
        assert_eq!(statements("A[x;y] --> B; C --> D"), vec!["A[x;y] --> B", "C --> D"]);
        assert_eq!(statements("A[\"a;b\"]"), vec!["A[\"a;b\"]"]);
    }

    #[test]
    fn clean_handles_breaks_entities_and_limits() {
        assert_eq!(clean("a<br>b<br/>c", 99), "a\nb\nc");
        assert_eq!(clean("#quot;x#quot; &amp; y", 99), "\"x\" & y");
        assert_eq!(clean("`**bold**`", 99), "bold");
        assert_eq!(clean("abcdef", 4), "abc…");
        assert_eq!(clean("ü<", 9), "ü<");
    }
}
