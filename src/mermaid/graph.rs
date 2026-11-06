//! Layered drawing shared by flowcharts and state diagrams: measures
//! labels, lays the graph out ([`super::layered`]), routes edges
//! ([`super::route`]) and emits scene items: subgraph frames, edges with
//! their labels in gaps of the line and their heads, then nodes.
//!
//! Follows Rune's `layered_draw.rs` and `connector.rs` (MIT, © 2026
//! Anomaly) for the parts this module supports.

use super::common::{arrowhead, ellipse, measure, outside, rounded_rect, smooth, unit, Label, Pt};
use super::layered::{layout, GCluster, GEdge, Graph, Spacing};
use super::route::Ctx;
use super::shapes::{self, node_size, Look, NodeShape};
use super::{Align, Fonts, Item, Measure, Role};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Dir {
    Down,
    Up,
    Right,
    Left,
}

impl Dir {
    pub(crate) fn parse(s: &str) -> Option<Dir> {
        match s.trim() {
            "" | "TD" | "TB" => Some(Dir::Down),
            "BT" => Some(Dir::Up),
            "LR" => Some(Dir::Right),
            "RL" => Some(Dir::Left),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Line {
    Solid,
    Dotted,
    Thick,
    Invisible,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Head {
    None,
    Arrow,
    Circle,
    Cross,
}

#[derive(Clone, Debug)]
pub(crate) struct Node {
    pub id: String,
    pub label: String,
    pub shape: NodeShape,
    pub cluster: Option<usize>,
    /// State descriptions, drawn under the name.
    pub desc: Vec<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct Edge {
    pub from: usize,
    pub to: usize,
    pub label: Option<String>,
    pub line: Line,
    pub start: Head,
    pub end: Head,
    pub minlen: usize,
}

#[derive(Clone, Debug)]
pub(crate) struct Cluster {
    pub title: String,
    pub parent: Option<usize>,
}

/// A flowchart or state diagram ready to draw.
#[derive(Clone, Debug)]
pub(crate) struct Model {
    pub dir: Dir,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    pub clusters: Vec<Cluster>,
}

const ARROW: f32 = 9.0;
/// How far a horizontal diagram may shrink to fit before it turns top-down.
const TURN_BELOW: f32 = 0.78;
/// Where a subgraph's title sits inside its top left corner.
const TITLE_INSET: [f32; 2] = [10.0, 6.0];

fn inset(h: Head) -> f32 {
    match h {
        Head::None => 0.0,
        Head::Arrow => ARROW * 0.7,
        Head::Circle => 8.0,
        Head::Cross => 4.0,
    }
}

fn label_box(m: &Label) -> [f32; 2] {
    [m.w + 12.0, m.h + 6.0]
}

struct Parts {
    labels: Vec<Label>,
    descs: Vec<Option<Label>>,
    sizes: Vec<[f32; 2]>,
    edge_labels: Vec<Option<Label>>,
    titles: Vec<Label>,
}

/// Node boxes and edge tips of a drawn layout (tests check invariants).
#[derive(Debug, Clone, Default)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct Geo {
    pub centers: Vec<Pt>,
    pub sizes: Vec<[f32; 2]>,
    pub shapes: Vec<NodeShape>,
    /// Per edge (None for self-loops): tip at the source, tip at the
    /// target, and the curve.
    pub tips: Vec<Option<(Pt, Pt, Vec<Pt>)>>,
}

/// The diagram drawn, as unpositioned items (call `super::finish`).
pub(crate) fn draw(model: &Model, fonts: &Fonts, m: &mut dyn Measure, room: f32) -> Option<Vec<Item>> {
    draw_geo(model, fonts, m, room).map(|(items, _)| items)
}

/// [`draw`], with the geometry of the layout it chose.
pub(crate) fn draw_geo(model: &Model, fonts: &Fonts, m: &mut dyn Measure, room: f32) -> Option<(Vec<Item>, Geo)> {
    let wrap = 200.0 * fonts.body / 13.0;
    let labels: Vec<Label> = model.nodes.iter().map(|n| measure(m, &n.label, fonts.body, false, wrap)).collect();
    let descs: Vec<Option<Label>> = model
        .nodes
        .iter()
        .map(|n| (!n.desc.is_empty()).then(|| measure(m, &n.desc.join("\n"), fonts.small, false, wrap)))
        .collect();
    let edge_labels: Vec<Option<Label>> = model
        .edges
        .iter()
        .map(|e| e.label.as_ref().filter(|l| !l.is_empty()).map(|l| measure(m, l, fonts.small, false, wrap * 0.8)))
        .collect();
    let titles: Vec<Label> = model.clusters.iter().map(|c| measure(m, &c.title, fonts.small, true, wrap * 1.5)).collect();
    let sizes_for = |across: bool| -> Vec<[f32; 2]> {
        model
            .nodes
            .iter()
            .enumerate()
            .map(|(i, n)| {
                let mut t = labels[i].size2();
                if let Some(d) = &descs[i] {
                    t = [t[0].max(d.w), t[1] + 10.0 + d.h];
                }
                node_size(n.shape, t, across)
            })
            .collect()
    };
    let across = matches!(model.dir, Dir::Right | Dir::Left);
    let (first, down_sizes) = (sizes_for(across), sizes_for(false));
    let mut parts = Parts { labels, descs, sizes: first, edge_labels, titles };
    let (items, w, geo) = draw_in(model, model.dir, &parts)?;
    if across && w > room / TURN_BELOW {
        parts.sizes = down_sizes;
        let down = if model.dir == Dir::Right { Dir::Down } else { Dir::Up };
        if let Some((d_items, dw, d_geo)) = draw_in(model, down, &parts) {
            if dw < w {
                return Some((d_items, d_geo));
            }
        }
    }
    Some((items, geo))
}

fn draw_in(model: &Model, dir: Dir, p: &Parts) -> Option<(Vec<Item>, f32, Geo)> {
    let across = matches!(dir, Dir::Right | Dir::Left);
    let ax = |s: [f32; 2]| if across { [s[1], s[0]] } else { s };
    let flip = matches!(dir, Dir::Up | Dir::Left);
    let spacing = if across {
        Spacing { node: 24.0, rank: 52.0, cluster: 14.0 }
    } else {
        Spacing { node: 32.0, rank: 44.0, cluster: 14.0 }
    };
    let graph = Graph {
        sizes: p.sizes.iter().map(|&s| ax(s)).collect(),
        edges: model
            .edges
            .iter()
            .zip(&p.edge_labels)
            .map(|(e, l)| {
                let (from, to) = if flip { (e.to, e.from) } else { (e.from, e.to) };
                GEdge { from, to, label: l.as_ref().map(|m| ax(label_box(m))), minlen: e.minlen }
            })
            .collect(),
        node_cluster: model.nodes.iter().map(|n| n.cluster).collect(),
        clusters: model
            .clusters
            .iter()
            .zip(&p.titles)
            .map(|(c, t)| {
                let lead = if t.is_empty() { 0.0 } else { t.h + TITLE_INSET[1] };
                let min_w = if t.is_empty() { 0.0 } else { t.w + 24.0 };
                GCluster {
                    parent: c.parent,
                    lead: if across { [lead, 0.0] } else { [0.0, lead] },
                    min_span: if across { [0.0, min_w] } else { [min_w, 0.0] },
                }
            })
            .collect(),
    };
    let placed = layout(&graph, &spacing)?;
    let pt = |q: Pt| if across { [q[1], q[0]] } else { q };
    let mut items = Vec::new();

    // Subgraph frames, outer first.
    let depth = |mut c: Option<usize>| {
        let mut d = 0;
        while let Some(i) = c {
            d += 1;
            if d > model.clusters.len() {
                break;
            }
            c = model.clusters.get(i).and_then(|k| k.parent);
        }
        d
    };
    let mut order: Vec<usize> = (0..model.clusters.len()).collect();
    order.sort_by_key(|&i| depth(Some(i)));
    let mut title_boxes = Vec::new();
    for i in order {
        let b = placed.clusters[i];
        if b == [0.0; 4] {
            continue;
        }
        let (p0, p1) = (pt([b[0], b[1]]), pt([b[2], b[3]]));
        let r = rounded_rect(p0[0].min(p1[0]), p0[1].min(p1[1]), p0[0].max(p1[0]), p0[1].max(p1[1]), 6.0);
        items.push(Item::Fill { points: r.clone(), role: Role::ClusterFill });
        items.push(Item::Stroke { points: r, closed: true, width: 1.0, dash: None, role: Role::ClusterBorder });
        let t = &p.titles[i];
        if !t.is_empty() {
            let at = [p0[0].min(p1[0]) + TITLE_INSET[0], p0[1].min(p1[1]) + TITLE_INSET[1] + t.h / 2.0];
            items.push(t.item(at, Role::Muted, Align::Left));
            title_boxes.push([at[0] - 4.0, at[1] - t.h / 2.0 - 4.0, at[0] + t.w + 4.0, at[1] + t.h / 2.0 + 4.0]);
        }
    }

    let shapes_v: Vec<NodeShape> = model.nodes.iter().map(|n| n.shape).collect();
    let ends = model.edges.iter().map(|e| (e.from, e.to)).collect();
    let boxes = p.edge_labels.iter().map(|l| l.as_ref().map(|m| ax(label_box(m)))).collect();
    // Titles are screen rects; `titles` maps them back to layout axes.
    let ctx = Ctx::new(&placed, &p.sizes, shapes_v, boxes, across, ends).titles(title_boxes.iter().copied());
    // Routes first: every edge's line breaks around every label.
    let routes: Vec<Option<(super::route::Route, Option<[f32; 4]>)>> = (0..model.edges.len())
        .map(|i| {
            let e = &model.edges[i];
            (placed.paths[i].len() > 1).then(|| {
                let r = ctx.route(i, flip, [inset(e.start), inset(e.end)]);
                let gap = p.edge_labels[i].as_ref().zip(r.label).map(|(m, at)| {
                    let s = label_box(m);
                    [at[0] - s[0] / 2.0, at[1] - s[1] / 2.0, at[0] + s[0] / 2.0, at[1] + s[1] / 2.0]
                });
                (r, gap)
            })
        })
        .collect();
    for (i, e) in model.edges.iter().enumerate() {
        match &routes[i] {
            None => {
                let c = pt(placed.centers[e.from]);
                let at = self_loop(&mut items, e, c, p.sizes[e.from]);
                if let Some(m) = &p.edge_labels[i] {
                    items.push(m.item([at[0] + 4.0, at[1]], Role::Muted, Align::Left));
                }
            }
            Some((r, gap)) => {
                let others = routes
                    .iter()
                    .enumerate()
                    .filter(|&(j, _)| j != i)
                    .filter_map(|(_, o)| o.as_ref().and_then(|o| o.1));
                let mut runs = vec![r.curve.clone()];
                for rect in gap.iter().copied().chain(title_boxes.iter().copied()).chain(others) {
                    runs = runs.iter().flat_map(|run| outside(run, rect)).collect();
                }
                connector(&mut items, &runs, [(r.tip_start, r.dir_start), (r.tip_end, r.dir_end)], e);
                if let (Some(m), Some(at)) = (&p.edge_labels[i], r.label) {
                    items.push(m.item(at, Role::Muted, Align::Center));
                }
            }
        }
    }

    for (i, n) in model.nodes.iter().enumerate() {
        let c = pt(placed.centers[i]);
        match &p.descs[i] {
            Some(d) => {
                let size = p.sizes[i];
                let (x0, y0, x1) = (c[0] - size[0] / 2.0, c[1] - size[1] / 2.0, c[0] + size[0] / 2.0);
                shapes::draw(&mut items, n.shape, &Label::default(), c, size, Look::NODE);
                let name = &p.labels[i];
                let top = y0 + 8.0;
                items.push(name.item([c[0], top + name.h / 2.0], Role::Text, Align::Center));
                let sep = top + name.h + 4.0;
                items.push(Item::Stroke { points: vec![[x0, sep], [x1, sep]], closed: false, width: 1.0, dash: None, role: Role::NodeBorder });
                items.push(d.item([c[0], sep + 6.0 + d.h / 2.0], Role::Muted, Align::Center));
            }
            None => shapes::draw(&mut items, n.shape, &p.labels[i], c, p.sizes[i], Look::NODE),
        }
    }
    let width = super::bbox(&items).map_or(0.0, |b| b[2] - b[0]);
    let geo = Geo {
        centers: (0..model.nodes.len()).map(|i| pt(placed.centers[i])).collect(),
        sizes: p.sizes.clone(),
        shapes: model.nodes.iter().map(|n| n.shape).collect(),
        tips: routes.into_iter().map(|r| r.map(|(r, _)| (r.tip_start, r.tip_end, r.curve))).collect(),
    };
    Some((items, width, geo))
}

/// Draws `e` along `runs` with its heads at `tips` (`[start, end]`, each a
/// tip and the unit direction it points).
fn connector(items: &mut Vec<Item>, runs: &[Vec<Pt>], tips: [(Pt, Pt); 2], e: &Edge) {
    if e.line == Line::Invisible {
        return;
    }
    let width = if e.line == Line::Thick { 2.5 } else { 1.25 };
    let dash = (e.line == Line::Dotted).then_some((3.0, 3.0));
    for run in runs {
        items.push(Item::Stroke { points: run.clone(), closed: false, width, dash, role: Role::Line });
    }
    head(items, e.end, tips[1].0, tips[1].1, width);
    head(items, e.start, tips[0].0, tips[0].1, width);
}

fn head(items: &mut Vec<Item>, h: Head, tip: Pt, d: Pt, width: f32) {
    let back = |by: f32| [tip[0] - d[0] * by, tip[1] - d[1] * by];
    match h {
        Head::None => {}
        Head::Arrow => items.push(Item::Fill { points: arrowhead(tip, d, ARROW + width, 6.5 + width * 1.6), role: Role::Line }),
        Head::Circle => {
            let c = ellipse(back(4.0), 4.0, 4.0);
            items.push(Item::Fill { points: c.clone(), role: Role::Background });
            items.push(Item::Stroke { points: c, closed: true, width, dash: None, role: Role::Line });
        }
        Head::Cross => {
            let m = back(4.0);
            let (a, b) = ([d[0] + d[1], d[1] - d[0]], [d[0] - d[1], d[1] + d[0]]);
            let k = 3.5 / std::f32::consts::SQRT_2;
            for v in [a, b] {
                items.push(Item::Stroke {
                    points: vec![[m[0] - v[0] * k, m[1] - v[1] * k], [m[0] + v[0] * k, m[1] + v[1] * k]],
                    closed: false,
                    width,
                    dash: None,
                    role: Role::Line,
                });
            }
        }
    }
}

/// A loop off the node's right side; returns the left middle of where its
/// label goes.
fn self_loop(items: &mut Vec<Item>, e: &Edge, c: Pt, size: [f32; 2]) -> Pt {
    let x = c[0] + size[0] / 2.0;
    let (a, b) = ([x, c[1] - size[1] / 4.0], [x, c[1] + size[1] / 4.0]);
    let mut path = [a, [x + 18.0, a[1] - 4.0], [x + 26.0, c[1]], [x + 18.0, b[1] + 4.0], b];
    let n = path.len();
    let (dir_end, dir_start) = (unit(path[n - 2], path[n - 1]), unit(path[1], path[0]));
    let (tip_end, tip_start) = (path[n - 1], path[0]);
    path[n - 1] = [tip_end[0] - dir_end[0] * inset(e.end), tip_end[1] - dir_end[1] * inset(e.end)];
    path[0] = [tip_start[0] - dir_start[0] * inset(e.start), tip_start[1] - dir_start[1] * inset(e.start)];
    connector(items, &[smooth(&path)], [(tip_start, dir_start), (tip_end, dir_end)], e);
    [x + 30.0, c[1]]
}
