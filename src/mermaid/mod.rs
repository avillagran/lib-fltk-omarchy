//! Native Mermaid diagrams (crate feature `mermaid`, off by default):
//! flowcharts, sequence diagrams and state diagrams parsed, laid out and
//! drawn in Rust with the CPU [`gfx`](crate::gfx) canvas and theme fonts,
//! so they follow `omarchy-theme-set` live and cost no web view (o3-code's
//! WebEngine helper took 150–240 MB).
//!
//! - [`render`] turns a source into a [`Diagram`]: its size and a
//!   backend-neutral [`Scene`] of polygons, polylines (width, dash) and
//!   text items, colored by [`Role`] (never hex).
//! - [`MermaidView`] shows a bound source, cached until the source, the
//!   width, the pixel density or the theme change.
//! - [`RichText::native_mermaid`](crate::text::RichText::native_mermaid)
//!   draws closed ```` ```mermaid ```` fences of a transcript as diagrams,
//!   with a "Show source" action in the code header; a fence still
//!   streaming shows its code until it closes.
//!
//! ```no_run
//! # #[cfg(feature = "mermaid")] {
//! use fltk_omarchy::mermaid;
//! # let _app = fltk_omarchy::App::new("dev.example.mermaid");
//! let d = mermaid::render("graph LR\n  A[Start] --> B{Ok?}\n  B -->|yes| C[Done]", 600.0).unwrap();
//! println!("{} x {} with {} items", d.width, d.height, d.scene.items.len());
//! # }
//! ```
//!
//! Untrusted input is bounded ([`Limits`]: source size, items, label
//! length, nesting); problems come back as [`Error`] values, never panics.
//! Supported syntax per type: see the `flowchart`, `sequence` and `state`
//! submodule docs and docs/MERMAID.md.
//!
//! Attribution: the layered layout, edge routing, shape geometry and the
//! flowchart/sequence grammars are adapted from Rune's `rune-mermaid`
//! crate (<https://github.com/anomalyco/rune>, MIT License, Copyright (c)
//! 2026 Anomaly). The MIT notice is reproduced in [`NOTICE`].

mod common;
mod flowchart;
mod graph;
mod layered;
pub(crate) mod paint;
pub(crate) mod rich;
mod route;
mod sequence;
mod shapes;
mod state;
mod view;

pub use paint::{color, draw_text, rasterize, Picture};
pub use rich::{is_mermaid, SHOW_DIAGRAM, SHOW_SOURCE};
pub use view::MermaidView;

use std::hash::{Hash, Hasher};

/// The MIT notice of the Rune code this module adapts.
pub const NOTICE: &str = "Portions adapted from rune-mermaid (https://github.com/anomalyco/rune)\n\
MIT License\n\nCopyright (c) 2026 Anomaly\n\n\
Permission is hereby granted, free of charge, to any person obtaining a copy of this software and \
associated documentation files (the \"Software\"), to deal in the Software without restriction, \
including without limitation the rights to use, copy, modify, merge, publish, distribute, \
sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is \
furnished to do so, subject to the following conditions:\n\n\
The above copyright notice and this permission notice shall be included in all copies or \
substantial portions of the Software.\n\n\
THE SOFTWARE IS PROVIDED \"AS IS\", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING \
BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND \
NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, \
DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, \
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.";

/// The diagram types this module draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Flowchart,
    Sequence,
    State,
}

/// What went wrong; a caller shows the source instead.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// Nothing but blank lines, comments or front matter.
    Empty,
    /// A diagram type this module does not draw (its header word).
    Unsupported(String),
    /// Past a [`Limits`] bound (which one).
    TooLarge(&'static str),
    /// Nothing drawable in it (no nodes, no participants…).
    Invalid(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Empty => f.write_str("empty diagram"),
            Error::Unsupported(k) => write!(f, "unsupported diagram type: {k}"),
            Error::TooLarge(what) => write!(f, "diagram too large ({what})"),
            Error::Invalid(why) => write!(f, "invalid diagram: {why}"),
        }
    }
}

impl std::error::Error for Error {}

/// Bounds for untrusted sources: work stays small however hostile the
/// input is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Limits {
    /// Bytes of source.
    pub max_source: usize,
    /// Nodes + edges + subgraphs (or participants + events).
    pub max_items: usize,
    /// Characters of one label.
    pub max_label: usize,
    /// Nesting of subgraphs, composites and frames.
    pub max_depth: usize,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits { max_source: 32 * 1024, max_items: 400, max_label: 300, max_depth: 8 }
    }
}

/// What a scene item is colored with: resolved against the theme and the
/// color under the diagram at paint time ([`color`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Role {
    /// The color under the diagram (cut-outs, ring centres).
    Background,
    /// Node fill: a light accent tint over the background.
    NodeFill,
    /// Node border: the accent.
    NodeBorder,
    /// Subgraph, composite and frame fill.
    ClusterFill,
    /// Subgraph and frame border, lifelines.
    ClusterBorder,
    /// Edges, messages and arrowheads.
    Line,
    /// Labels.
    Text,
    /// Edge labels, frame labels, descriptions.
    Muted,
    Accent,
    NoteFill,
    NoteBorder,
    /// Start/end dots and fork bars.
    Marker,
    /// Sequence activation bars.
    ActivationFill,
}

/// How a text item is anchored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Align {
    /// `x` is the left edge.
    Left,
    /// `x` is the centre.
    Center,
}

/// One drawing primitive, in diagram pixels (origin top left).
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    /// A filled polygon.
    Fill { points: Vec<[f32; 2]>, role: Role },
    /// A polyline (closed: a polygon outline) with round joins; `dash`
    /// is (on, off) lengths.
    Stroke { points: Vec<[f32; 2]>, closed: bool, width: f32, dash: Option<(f32, f32)>, role: Role },
    /// Lines of text centred vertically at `y`; `x` per [`Align`];
    /// `width` is the measured width of the widest line.
    Text { lines: Vec<String>, x: f32, y: f32, width: f32, size: f32, line_height: f32, bold: bool, role: Role, align: Align },
}

/// Backend-neutral drawing of a diagram.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Scene {
    pub items: Vec<Item>,
}

/// A laid out diagram: size (already scaled to fit), scale and scene.
#[derive(Debug, Clone, PartialEq)]
pub struct Diagram {
    pub kind: Kind,
    pub width: f32,
    pub height: f32,
    /// How much the natural layout was shrunk to fit the width (≤ 1).
    pub scale: f32,
    pub scene: Scene,
    /// The diagram's text for screen readers and copy ("Flowchart: A → B…").
    pub summary: String,
    key: u64,
}

impl Diagram {
    /// A hash of the source and options it was rendered from (caches).
    pub fn key(&self) -> u64 {
        self.key
    }
}

/// Text measurement for layout. [`FltkMeasure`] uses the theme fonts
/// (needs an open display); [`ApproxMeasure`] is a fixed-pitch estimate
/// for tests and headless use.
pub trait Measure {
    fn width(&mut self, text: &str, size: f32, bold: bool) -> f32;
    fn line_height(&mut self, size: f32) -> f32 {
        (size * 1.3).ceil()
    }
}

/// Measures with FLTK and the toolkit's theme font ([`crate::paint::font`]).
#[derive(Debug, Default, Clone, Copy)]
pub struct FltkMeasure;

impl Measure for FltkMeasure {
    fn width(&mut self, text: &str, size: f32, bold: bool) -> f32 {
        if text.is_empty() {
            return 0.0;
        }
        fltk::draw::set_font(crate::paint::font(bold), size.round().max(1.0) as i32);
        fltk::draw::width(text) as f32
    }
}

/// Every character `0.6 × size` wide (bold a little wider).
#[derive(Debug, Default, Clone, Copy)]
pub struct ApproxMeasure;

impl Measure for ApproxMeasure {
    fn width(&mut self, text: &str, size: f32, bold: bool) -> f32 {
        text.chars().count() as f32 * size * if bold { 0.64 } else { 0.6 }
    }
}

/// Fonts and fitting for [`render_with`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Options {
    /// The widest the drawing may be; wider layouts turn top-down
    /// (flowcharts drawn left to right) or shrink down to `min_scale`.
    pub max_width: f32,
    /// Node and participant labels.
    pub font_size: f32,
    /// Edge labels, messages, notes and titles.
    pub small_size: f32,
    /// The least a diagram shrinks to fit; past it the drawing is wider
    /// than `max_width` (a view clips it).
    pub min_scale: f32,
    pub limits: Limits,
}

impl Options {
    /// Theme font sizes (body and caption).
    pub fn from_theme(max_width: f32) -> Options {
        let (body, small) = crate::theme::with(|t| (t.font.body as f32, t.font.caption as f32));
        Options { max_width, font_size: body, small_size: small.min(body), min_scale: 0.6, limits: Limits::default() }
    }
}

pub(crate) struct Fonts {
    pub body: f32,
    pub small: f32,
}

/// Margin around the drawing.
const MARGIN: f32 = 6.0;

/// The diagram type named by the source's header, if this module draws it.
pub fn detect(src: &str) -> Option<Kind> {
    let (_, lines) = common::cleaned(src);
    let header = lines.first()?;
    match header.split(|c: char| c.is_whitespace() || c == ';').next()? {
        "graph" | "flowchart" | "flowchart-elk" => Some(Kind::Flowchart),
        "sequenceDiagram" => Some(Kind::Sequence),
        "stateDiagram" | "stateDiagram-v2" => Some(Kind::State),
        _ => None,
    }
}

/// [`render_with`] with theme fonts and FLTK measurement.
pub fn render(src: &str, max_width: f32) -> Result<Diagram, Error> {
    render_with(src, &Options::from_theme(max_width), &mut FltkMeasure)
}

/// Parses, lays out and draws `src` into a [`Diagram`].
pub fn render_with(src: &str, opts: &Options, m: &mut dyn Measure) -> Result<Diagram, Error> {
    let limits = &opts.limits;
    common::check_source(src, limits)?;
    let (_title, lines) = common::cleaned(src);
    let Some(header) = lines.first().copied() else { return Err(Error::Empty) };
    let (word, rest) = header.split_once(|c: char| c.is_whitespace() || c == ';').unwrap_or((header, ""));
    let fonts = Fonts { body: opts.font_size.clamp(6.0, 48.0), small: opts.small_size.clamp(6.0, 48.0) };
    let room = (opts.max_width - 2.0 * MARGIN).max(40.0);
    let body = &lines[1..];
    let (kind, items, summary) = match word {
        "graph" | "flowchart" | "flowchart-elk" => {
            let model = flowchart::parse(rest, body, limits)?;
            let items = graph::draw(&model, &fonts, m, room).ok_or(Error::TooLarge("layout"))?;
            (Kind::Flowchart, items, summary_graph("Flowchart", &model))
        }
        "stateDiagram" | "stateDiagram-v2" => {
            let model = state::parse(body, limits)?;
            let items = graph::draw(&model, &fonts, m, room).ok_or(Error::TooLarge("layout"))?;
            (Kind::State, items, summary_graph("State diagram", &model))
        }
        "sequenceDiagram" => {
            let seq = sequence::parse(body, limits)?;
            let items = sequence::draw(&seq, &fonts, m);
            (Kind::Sequence, items, summary_sequence(&seq))
        }
        other => return Err(Error::Unsupported(other.to_string())),
    };
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (src, opts.max_width.to_bits(), opts.font_size.to_bits(), opts.small_size.to_bits(), opts.min_scale.to_bits()).hash(&mut h);
    Ok(finish(kind, items, summary, opts, h.finish()))
}

/// The bounding box `[x0, y0, x1, y1]` of items (text included).
pub(crate) fn bbox(items: &[Item]) -> Option<[f32; 4]> {
    let mut b = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
    let mut add = |x: f32, y: f32| {
        if x.is_finite() && y.is_finite() {
            b = [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)];
        }
    };
    for it in items {
        match it {
            Item::Fill { points, .. } => points.iter().for_each(|p| add(p[0], p[1])),
            Item::Stroke { points, width, .. } => points.iter().for_each(|p| {
                add(p[0] - width / 2.0, p[1] - width / 2.0);
                add(p[0] + width / 2.0, p[1] + width / 2.0);
            }),
            Item::Text { lines, x, y, width, line_height, align, .. } => {
                let (w, h) = (*width, lines.len() as f32 * line_height);
                let x0 = match align {
                    Align::Left => *x,
                    Align::Center => x - w / 2.0,
                };
                add(x0, y - h / 2.0);
                add(x0 + w, y + h / 2.0);
            }
        }
    }
    (b[0] <= b[2]).then_some(b)
}

/// Moves the items to a margin from the origin and scales them to fit.
fn finish(kind: Kind, mut items: Vec<Item>, summary: String, opts: &Options, key: u64) -> Diagram {
    let b = bbox(&items).unwrap_or([0.0, 0.0, 1.0, 1.0]);
    let (w, h) = (b[2] - b[0] + 2.0 * MARGIN, b[3] - b[1] + 2.0 * MARGIN);
    let scale = if w > opts.max_width && opts.max_width > 0.0 { (opts.max_width / w).max(opts.min_scale.clamp(0.05, 1.0)) } else { 1.0 };
    let (dx, dy) = (MARGIN - b[0], MARGIN - b[1]);
    let map = |p: &mut [f32; 2]| {
        p[0] = (p[0] + dx) * scale;
        p[1] = (p[1] + dy) * scale;
    };
    for it in &mut items {
        match it {
            Item::Fill { points, .. } => points.iter_mut().for_each(map),
            Item::Stroke { points, width, dash, .. } => {
                points.iter_mut().for_each(map);
                *width = (*width * scale).max(0.75);
                if let Some(d) = dash {
                    *d = (d.0 * scale, d.1 * scale);
                }
            }
            Item::Text { x, y, width, size, line_height, .. } => {
                let mut p = [*x, *y];
                map(&mut p);
                *x = p[0];
                *y = p[1];
                *width *= scale;
                *size *= scale;
                *line_height *= scale;
            }
        }
    }
    Diagram { kind, width: (w * scale).ceil(), height: (h * scale).ceil(), scale, scene: Scene { items }, summary, key }
}

fn summary_graph(kind: &str, m: &graph::Model) -> String {
    let name = |i: usize| {
        let n = &m.nodes[i];
        match n.shape {
            shapes::NodeShape::Start => "start".to_string(),
            shapes::NodeShape::End => "end".to_string(),
            _ if n.label.is_empty() => n.id.clone(),
            _ => n.label.replace('\n', " "),
        }
    };
    let mut s = format!("{kind}: ");
    let parts: Vec<String> = m
        .edges
        .iter()
        .take(60)
        .map(|e| match &e.label {
            Some(l) => format!("{} → {} ({})", name(e.from), name(e.to), l.replace('\n', " ")),
            None => format!("{} → {}", name(e.from), name(e.to)),
        })
        .collect();
    if parts.is_empty() {
        s.push_str(&(0..m.nodes.len().min(60)).map(name).collect::<Vec<_>>().join(", "));
    } else {
        s.push_str(&parts.join("; "));
    }
    s
}

fn summary_sequence(q: &sequence::Sequence) -> String {
    let mut parts = Vec::new();
    for e in q.events.iter().take(60) {
        if let sequence::Event::Message(msg) = e {
            let (a, b) = (&q.participants[msg.from].label, &q.participants[msg.to].label);
            parts.push(if msg.text.is_empty() { format!("{a} → {b}") } else { format!("{a} → {b}: {}", msg.text) });
        }
    }
    let who: Vec<&str> = q.participants.iter().map(|p| p.label.as_str()).collect();
    format!("Sequence diagram ({}): {}", who.join(", "), parts.join("; "))
}

#[cfg(test)]
mod tests;
