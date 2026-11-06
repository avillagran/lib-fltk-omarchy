//! Rendering and layout invariants with fixed-pitch measurement.

use super::common::{cleaned, seg_dist};
use super::graph::{draw_geo, Geo};
use super::shapes::outline;
use super::*;

pub(crate) const FLOW20: &str = "flowchart TD
    A[Request] --> B{Cached?}
    B -->|yes| C[Serve cache]
    B -->|no| D[Fetch origin]
    D --> E{Status}
    E -->|200| F[Store]
    E -->|404| G[Not found]
    E -->|5xx| H[Retry]
    H -.-> D
    F --> C
    C --> I((Done))
    G --> I
    subgraph auth [Auth]
        J[Token] --> K[Verify]
        K --> L{Valid?}
    end
    A --> J
    L -->|no| M[Reject]
    L -->|yes| B
    M --> I
    N[(Logs)] --- D
    O[[Metrics]] --- F
    P>Alert] -.-> H
    Q([Queue]) ==> D
    R{{Worker}} --> Q
    S[/Upload/] --> R
    T[\\Admin\\] --> S
";

pub(crate) const SEQ10: &str = "sequenceDiagram
    autonumber
    participant U as User
    actor A as Admin
    participant S as Server
    participant D as Database
    U->>+S: Login
    S->>D: Query user
    D-->>S: Row
    S-->>-U: Token
    loop Every minute
        U->>S: Heartbeat
    end
    alt ok
        S->>D: Save
    else failure
        S-xU: Error
    end
    Note over S,D: Replicated
    A-)S: Audit
    S->>S: Log
";

fn opts(w: f32) -> Options {
    Options { max_width: w, font_size: 13.0, small_size: 11.0, min_scale: 0.6, limits: Limits::default() }
}

fn geo(src: &str) -> Geo {
    let (_, lines) = cleaned(src);
    let header = lines[0];
    let (word, rest) = header.split_once(' ').unwrap_or((header, ""));
    let model = if word.starts_with("state") {
        super::state::parse(&lines[1..], &Limits::default()).unwrap()
    } else {
        super::flowchart::parse(rest, &lines[1..], &Limits::default()).unwrap()
    };
    let fonts = Fonts { body: 13.0, small: 11.0 };
    draw_geo(&model, &fonts, &mut ApproxMeasure, 2000.0).unwrap().1
}

fn no_overlaps(g: &Geo) {
    for i in 0..g.centers.len() {
        for j in i + 1..g.centers.len() {
            let (a, b) = (g.centers[i], g.centers[j]);
            let (sa, sb) = (g.sizes[i], g.sizes[j]);
            let apart = (a[0] - b[0]).abs() >= (sa[0] + sb[0]) / 2.0 - 0.5 || (a[1] - b[1]).abs() >= (sa[1] + sb[1]) / 2.0 - 0.5;
            assert!(apart, "nodes {i} and {j} overlap: {a:?} {sa:?} / {b:?} {sb:?}");
        }
    }
}

fn on_outline(g: &Geo, v: usize, p: [f32; 2]) -> f32 {
    let poly = outline(g.shapes[v], g.centers[v], g.sizes[v]);
    (0..poly.len()).map(|i| seg_dist(p, poly[i], poly[(i + 1) % poly.len()])).fold(f32::MAX, f32::min)
}

fn edges_end_at_borders(g: &Geo, ends: &[(usize, usize)]) {
    for (i, t) in g.tips.iter().enumerate() {
        let Some((s, e, curve)) = t else { continue };
        let (from, to) = ends[i];
        assert!(on_outline(g, from, *s) < 1.5, "edge {i} starts {} px off node {from}", on_outline(g, from, *s));
        assert!(on_outline(g, to, *e) < 1.5, "edge {i} ends {} px off node {to}", on_outline(g, to, *e));
        // Neither end of the drawn line is inside its node.
        for (p, v) in [(curve[0], from), (curve[curve.len() - 1], to)] {
            assert!(!super::common::inside(&outline(g.shapes[v], g.centers[v], g.sizes[v]), [p[0], p[1]]) || on_outline(g, v, p) < 1.5, "edge {i} line end inside node {v}");
        }
    }
}

fn ends_of(src: &str) -> Vec<(usize, usize)> {
    let (_, lines) = cleaned(src);
    let (word, rest) = lines[0].split_once(' ').unwrap_or((lines[0], ""));
    let m = if word.starts_with("state") {
        super::state::parse(&lines[1..], &Limits::default()).unwrap()
    } else {
        super::flowchart::parse(rest, &lines[1..], &Limits::default()).unwrap()
    };
    m.edges.iter().map(|e| (e.from, e.to)).collect()
}

#[test]
fn flowchart_layout_invariants_in_every_direction() {
    for dir in ["TD", "LR", "BT", "RL"] {
        let src = FLOW20.replacen("flowchart TD", &format!("flowchart {dir}"), 1);
        let g = geo(&src);
        assert_eq!(g.centers.len(), 20, "{dir}");
        no_overlaps(&g);
        edges_end_at_borders(&g, &ends_of(&src));
    }
}

#[test]
fn state_layout_invariants() {
    let src = "stateDiagram-v2\n[*] --> Idle\nIdle --> Running : start\nRunning --> Idle : stop\nRunning --> Failed : error\nFailed --> [*]\nstate Running {\n[*] --> Warmup\nWarmup --> Steady\n}\nstate pick <<choice>>\nIdle --> pick\npick --> Failed : bad\npick --> Running : good";
    let g = geo(src);
    no_overlaps(&g);
    edges_end_at_borders(&g, &ends_of(src));
}

#[test]
fn ranks_follow_the_direction() {
    let g = geo("graph TD\nA --> B --> C");
    assert!(g.centers[0][1] < g.centers[1][1] && g.centers[1][1] < g.centers[2][1]);
    let g = geo("graph LR\nA --> B --> C");
    assert!(g.centers[0][0] < g.centers[1][0] && g.centers[1][0] < g.centers[2][0]);
    let g = geo("graph BT\nA --> B");
    assert!(g.centers[0][1] > g.centers[1][1]);
    let g = geo("graph RL\nA --> B");
    assert!(g.centers[0][0] > g.centers[1][0]);
}

#[test]
fn render_fits_and_reports() {
    let d = render_with(FLOW20, &opts(640.0), &mut ApproxMeasure).unwrap();
    assert_eq!(d.kind, Kind::Flowchart);
    assert!(d.width <= 640.0 + 1.0 || d.scale <= 0.6 + 1e-3, "{} at {}", d.width, d.scale);
    assert!(d.summary.starts_with("Flowchart: Request → Cached?"), "{}", d.summary);
    // Every point is inside the reported size.
    let b = bbox(&d.scene.items).unwrap();
    assert!(b[0] >= -0.5 && b[1] >= -0.5 && b[2] <= d.width + 0.5 && b[3] <= d.height + 0.5, "{b:?} vs {}x{}", d.width, d.height);
    let s = render_with(SEQ10, &opts(640.0), &mut ApproxMeasure).unwrap();
    assert_eq!(s.kind, Kind::Sequence);
    assert!(s.summary.contains("User → Server: Login"), "{}", s.summary);
    let texts: Vec<&str> = s.scene.items.iter().filter_map(|i| if let Item::Text { lines, .. } = i { lines.first().map(|l| l.as_str()) } else { None }).collect();
    assert!(texts.contains(&"Login") && texts.contains(&"Replicated") && texts.contains(&"loop"), "{texts:?}");
    // Two heads per participant.
    assert_eq!(texts.iter().filter(|t| **t == "Server").count(), 2);
}

#[test]
fn wide_left_to_right_turns_top_down() {
    let src = "graph LR\nA[A long first step] --> B[A long second step] --> C[A long third step] --> D[A long fourth step] --> E[A long fifth step]";
    let d = render_with(src, &opts(400.0), &mut ApproxMeasure).unwrap();
    assert!(d.height > d.width, "turned: {}x{}", d.width, d.height);
    assert!((d.scale - 1.0).abs() < 1e-6);
}

#[test]
fn errors_are_values() {
    let o = opts(600.0);
    assert_eq!(render_with("", &o, &mut ApproxMeasure), Err(Error::Empty));
    assert_eq!(render_with("%% only a comment\n", &o, &mut ApproxMeasure), Err(Error::Empty));
    assert_eq!(render_with("pie\n\"a\": 1", &o, &mut ApproxMeasure), Err(Error::Unsupported("pie".into())));
    assert!(matches!(render_with(&format!("graph TD\nA[{}]", "x".repeat(40_000)), &o, &mut ApproxMeasure), Err(Error::TooLarge("source"))));
    assert!(matches!(render_with("graph TD\n", &o, &mut ApproxMeasure), Err(Error::Invalid(_))));
    assert!(matches!(render_with("sequenceDiagram\n", &o, &mut ApproxMeasure), Err(Error::Invalid(_))));
    assert_eq!(detect("graph TD\nA"), Some(Kind::Flowchart));
    assert_eq!(detect("sequenceDiagram"), Some(Kind::Sequence));
    assert_eq!(detect("stateDiagram-v2"), Some(Kind::State));
    assert_eq!(detect("gantt"), None);
}

#[test]
fn labels_are_bounded() {
    let d = render_with(&format!("graph TD\nA[\"{}\"] --> B", "word ".repeat(200)), &opts(600.0), &mut ApproxMeasure).unwrap();
    let longest = d
        .scene
        .items
        .iter()
        .filter_map(|i| if let Item::Text { lines, .. } = i { Some(lines.len()) } else { None })
        .max()
        .unwrap();
    assert!(longest <= 12, "{longest} lines");
}

/// Truncations and character swaps of the samples never panic.
#[test]
fn mangled_sources_never_panic() {
    let samples = [FLOW20, SEQ10, "stateDiagram-v2\n[*] --> A\nstate A {\n[*] --> B\n}\nA --> [*] : done\nnote right of A\nx\nend note"];
    let mut seed = 0x2545_f491_u32;
    let mut rnd = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed
    };
    let junk = ['[', ']', '(', ')', '{', '}', '|', '-', '>', '<', '"', ':', ';', '\n', 'x', '&', '%', '.', '=', 'é'];
    for s in samples {
        let chars: Vec<char> = s.chars().collect();
        for cut in (0..chars.len()).step_by(7) {
            let t: String = chars[..cut].iter().collect();
            let _ = render_with(&t, &opts(500.0), &mut ApproxMeasure);
        }
        for _ in 0..300 {
            let mut c = chars.clone();
            for _ in 0..4 {
                let i = rnd() as usize % c.len();
                c[i] = junk[rnd() as usize % junk.len()];
            }
            let t: String = c.into_iter().collect();
            let _ = render_with(&t, &opts(500.0), &mut ApproxMeasure);
        }
    }
}

#[test]
fn dense_graphs_stay_bounded() {
    // Many long edges: refused or drawn, quickly either way.
    let mut src = String::from("graph TD\n");
    for i in 0..60 {
        src.push_str(&format!("N{i} --> N{}\n", (i * 7 + 3) % 60));
        src.push_str(&format!("N{i} ---------> N{}\n", (i * 11 + 5) % 60));
    }
    let t = std::time::Instant::now();
    let _ = render_with(&src, &opts(600.0), &mut ApproxMeasure);
    assert!(t.elapsed().as_secs_f32() < 20.0, "took {:?}", t.elapsed());
}
