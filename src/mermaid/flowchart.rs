//! Flowcharts (`graph` / `flowchart`): the parser, building a
//! [`graph::Model`](super::graph::Model).
//!
//! Supported: directions `TD`/`TB`/`BT`/`LR`/`RL`; node shapes `[rect]`,
//! `(round)`, `([stadium])`, `[[subroutine]]`, `[(cylinder)]`,
//! `((circle))`, `(((double circle)))`, `>asymmetric]`, `{diamond}`,
//! `{{hexagon}}`, `[/lean/]`, `[\lean\]`, `[/trapezoid\]`,
//! `[\inverted/]`; links `-->`, `---`, `-.->`, `-.-`, `==>`, `===`,
//! `~~~`, `<-->`, `--o`, `--x`, longer forms for longer links; labels as
//! `-- text -->`, `-. text .->`, `== text ==>` or `-->|text|`; chains
//! (`A --> B --> C`), `&` groups, `:::class` (read and ignored), quoted
//! labels, `<br>` breaks, nested `subgraph … end` (edges to a subgraph id
//! go to its first or last member). `style`, `classDef`, `class`,
//! `linkStyle` and `click` are read and ignored (colors come from the
//! theme).
//!
//! The link and shape grammar follows Rune's `flowchart.rs` (MIT, © 2026
//! Anomaly).

use std::collections::HashMap;

use super::common::{clean, clean_quoted, ident, statements, Cur};
use super::graph::{Cluster, Dir, Edge, Head, Line, Model, Node};
use super::layered::MAX_MINLEN;
use super::shapes::NodeShape;
use super::{Error, Limits};

pub(crate) fn parse(header: &str, body: &[&str], limits: &Limits) -> Result<Model, Error> {
    let dir_word = header.split(|c: char| c.is_whitespace() || c == ';').find(|s| !s.is_empty()).unwrap_or("");
    let rest = header.trim_start().strip_prefix(dir_word).unwrap_or("");
    let mut p = Parser {
        m: Model { dir: Dir::parse(dir_word).unwrap_or(Dir::Down), nodes: Vec::new(), edges: Vec::new(), clusters: Vec::new() },
        ids: HashMap::new(),
        cluster_ids: HashMap::new(),
        stack: Vec::new(),
        limits,
    };
    for line in std::iter::once(rest).chain(body.iter().copied()) {
        for stmt in statements(line) {
            // A statement that does not read is skipped; one past the bounds fails.
            p.statement(stmt)?;
        }
    }
    p.resolve_cluster_ends();
    if p.m.nodes.is_empty() {
        return Err(Error::Invalid("no nodes".into()));
    }
    Ok(p.m)
}

struct Parser<'a> {
    m: Model,
    ids: HashMap<String, usize>,
    cluster_ids: HashMap<String, usize>,
    stack: Vec<usize>,
    limits: &'a Limits,
}

struct Link {
    line: Line,
    start: Head,
    end: Head,
    minlen: usize,
    label: Option<String>,
}

struct NodeRef {
    id: String,
    shape: Option<(String, NodeShape)>,
}

impl Parser<'_> {
    fn budget(&self) -> Result<(), Error> {
        if self.m.nodes.len() + self.m.edges.len() + self.m.clusters.len() >= self.limits.max_items {
            return Err(Error::TooLarge("items"));
        }
        Ok(())
    }

    fn statement(&mut self, s: &str) -> Result<(), Error> {
        let word = s.split(|c: char| c.is_whitespace() || c == ':').next().unwrap_or("");
        match word {
            "subgraph" => {
                let rest = s["subgraph".len()..].trim();
                let (id, title) = match rest.find('[') {
                    Some(i) if rest.ends_with(']') => {
                        let raw = rest[..i].trim();
                        ((!raw.is_empty()).then(|| raw.to_string()), clean_quoted(&rest[i + 1..rest.len() - 1], self.limits.max_label))
                    }
                    _ => {
                        let id = (!rest.starts_with('"') && !rest.contains(char::is_whitespace) && !rest.is_empty()).then(|| rest.to_string());
                        (id, clean_quoted(rest, self.limits.max_label))
                    }
                };
                self.budget()?;
                if self.stack.len() >= self.limits.max_depth {
                    return Err(Error::TooLarge("nesting"));
                }
                self.m.clusters.push(Cluster { title, parent: self.stack.last().copied() });
                let c = self.m.clusters.len() - 1;
                if let Some(id) = id {
                    self.cluster_ids.insert(id, c);
                }
                self.stack.push(c);
            }
            "end" if s == "end" => {
                self.stack.pop();
            }
            "direction" => {
                if self.stack.is_empty() && self.m.edges.is_empty() {
                    if let Some(d) = Dir::parse(&s["direction".len()..]) {
                        self.m.dir = d;
                    }
                }
            }
            "classDef" | "class" | "style" | "linkStyle" | "click" | "accTitle" | "accDescr" | "title" => {}
            _ => self.chain(s)?,
        }
        Ok(())
    }

    /// `A --> B & C -.-> D`, read whole before anything is added.
    fn chain(&mut self, s: &str) -> Result<(), Error> {
        let mut c = Cur::new(s);
        let Some(first) = group(&mut c, self.limits.max_label) else { return Ok(()) };
        let mut groups = vec![first];
        let mut links = Vec::new();
        loop {
            c.skip_ws();
            if c.done() {
                break;
            }
            let Some(l) = link(&mut c, self.limits.max_label) else { return Ok(()) };
            links.push(l);
            c.skip_ws();
            let Some(g) = group(&mut c, self.limits.max_label) else { return Ok(()) };
            groups.push(g);
        }
        let mut idx: Vec<Vec<usize>> = Vec::with_capacity(groups.len());
        for g in groups {
            let mut v = Vec::with_capacity(g.len());
            for n in g {
                v.push(self.node(n)?);
            }
            idx.push(v);
        }
        for (l, pair) in links.iter().zip(idx.windows(2)) {
            for &from in &pair[0] {
                for &to in &pair[1] {
                    self.budget()?;
                    self.m.edges.push(Edge {
                        from,
                        to,
                        label: l.label.clone(),
                        line: l.line,
                        start: l.start,
                        end: l.end,
                        minlen: l.minlen,
                    });
                }
            }
        }
        Ok(())
    }

    fn node(&mut self, r: NodeRef) -> Result<usize, Error> {
        let cluster = self.stack.last().copied();
        let i = match self.ids.get(&r.id) {
            Some(&i) => i,
            None => {
                self.budget()?;
                self.m.nodes.push(Node { id: r.id.clone(), label: r.id.clone(), shape: NodeShape::Rect, cluster, desc: Vec::new() });
                self.ids.insert(r.id, self.m.nodes.len() - 1);
                self.m.nodes.len() - 1
            }
        };
        let n = &mut self.m.nodes[i];
        if n.cluster.is_none() {
            n.cluster = cluster;
        }
        if let Some((label, shape)) = r.shape {
            n.label = label;
            n.shape = shape;
        }
        Ok(i)
    }

    /// Edges to a `subgraph id` go to its first (target) or last (source)
    /// member; the placeholder node is dropped.
    fn resolve_cluster_ends(&mut self) {
        let mut redirect: HashMap<usize, (usize, usize)> = HashMap::new();
        for (id, &c) in &self.cluster_ids {
            let Some(&v) = self.ids.get(id) else { continue };
            if self.m.nodes[v].shape != NodeShape::Rect || self.m.nodes[v].label != *id {
                continue;
            }
            let members: Vec<usize> = (0..self.m.nodes.len()).filter(|&i| i != v && self.in_cluster(i, c)).collect();
            if let (Some(&first), Some(&last)) = (members.first(), members.last()) {
                redirect.insert(v, (first, last));
            }
        }
        if redirect.is_empty() {
            return;
        }
        for e in &mut self.m.edges {
            if let Some(&(_, last)) = redirect.get(&e.from) {
                e.from = last;
            }
            if let Some(&(first, _)) = redirect.get(&e.to) {
                e.to = first;
            }
        }
        let mut remap = vec![usize::MAX; self.m.nodes.len()];
        let mut kept = Vec::with_capacity(self.m.nodes.len());
        for (i, n) in std::mem::take(&mut self.m.nodes).into_iter().enumerate() {
            if !redirect.contains_key(&i) {
                remap[i] = kept.len();
                kept.push(n);
            }
        }
        self.m.nodes = kept;
        for e in &mut self.m.edges {
            e.from = remap[e.from];
            e.to = remap[e.to];
        }
        self.m.edges.retain(|e| e.from != usize::MAX && e.to != usize::MAX);
    }

    fn in_cluster(&self, v: usize, target: usize) -> bool {
        let mut cur = self.m.nodes[v].cluster;
        let mut steps = 0;
        while let Some(c) = cur {
            if c == target {
                return true;
            }
            steps += 1;
            if steps > self.m.clusters.len() {
                break;
            }
            cur = self.m.clusters.get(c).and_then(|k| k.parent);
        }
        false
    }
}

fn group(c: &mut Cur, max: usize) -> Option<Vec<NodeRef>> {
    let mut v = vec![node_ref(c, max)?];
    loop {
        c.skip_ws();
        if !c.eat("&") {
            return Some(v);
        }
        c.skip_ws();
        v.push(node_ref(c, max)?);
    }
}

fn node_ref(c: &mut Cur, max: usize) -> Option<NodeRef> {
    let id = ident(c)?;
    let shape = shape(c, max);
    if c.eat(":::") {
        ident(c)?;
    }
    Some(NodeRef { id, shape })
}

const SHAPES: &[(&str, &str, NodeShape)] = &[
    ("(((", ")))", NodeShape::DoubleCircle),
    ("([", "])", NodeShape::Stadium),
    ("[[", "]]", NodeShape::Subroutine),
    ("[(", ")]", NodeShape::Cylinder),
    ("((", "))", NodeShape::Circle),
    ("{{", "}}", NodeShape::Hexagon),
    (">", "]", NodeShape::Asymmetric),
    ("[", "]", NodeShape::Rect),
    ("(", ")", NodeShape::Round),
    ("{", "}", NodeShape::Diamond),
];

fn find_close(s: &str, open: &str, close: &str) -> Option<usize> {
    let nested = open.len() == 1 && close.len() == 1 && open != ">";
    let (mut depth, mut quoted, mut escaped) = (1usize, false, false);
    for (i, ch) in s.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if quoted {
            match ch {
                '\\' => escaped = true,
                '"' => quoted = false,
                _ => {}
            }
            continue;
        }
        if ch == '"' {
            quoted = true;
            continue;
        }
        let rest = &s[i..];
        if rest.starts_with(close) {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
            continue;
        }
        if nested && rest.starts_with(open) {
            depth += 1;
        }
    }
    None
}

fn find_unquoted(s: &str, needle: &str) -> Option<usize> {
    let (mut quoted, mut escaped) = (false, false);
    for (i, ch) in s.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if quoted {
            match ch {
                '\\' => escaped = true,
                '"' => quoted = false,
                _ => {}
            }
            continue;
        }
        if ch == '"' {
            quoted = true;
            continue;
        }
        if s[i..].starts_with(needle) {
            return Some(i);
        }
    }
    None
}

/// A node's bracketed label and shape, `None` when it has none. An opener
/// without its closer reads as a rectangle holding the rest as written.
fn shape(c: &mut Cur, max: usize) -> Option<(String, NodeShape)> {
    let start = c.i;
    if !matches!(c.peek()?, '[' | '(' | '{' | '>') {
        return None;
    }
    exact_shape(c, max).or_else(|| {
        c.i = start;
        let rest = c.rest();
        let (open, close) = match rest.chars().next() {
            Some('(') => ("(", ")"),
            Some('{') => ("{", "}"),
            Some('>') => (">", "]"),
            _ => ("[", "]"),
        };
        let body = &rest[1..];
        let (text, used) = find_close(body, open, close).map_or((body, rest.len()), |e| (&body[..e], 1 + e + 1));
        c.i += used.min(rest.len());
        Some((clean_quoted(text, max), NodeShape::Rect))
    })
}

fn exact_shape(c: &mut Cur, max: usize) -> Option<(String, NodeShape)> {
    for open in ["[/", "[\\"] {
        if c.rest().starts_with(open) {
            let start = c.i;
            c.eat(open);
            let end = find_close(c.rest(), "[", "]");
            let inner = end.map(|e| &c.rest()[..e]);
            let close = inner.and_then(|t| t.chars().last()).filter(|&ch| ch == '/' || ch == '\\');
            match (end, inner, close) {
                (Some(end), Some(inner), Some(close)) => {
                    let label = clean_quoted(&inner[..inner.len() - 1], max);
                    c.i += end + 1;
                    let kind = match (open, close) {
                        ("[/", '/') => NodeShape::LeanRight,
                        ("[/", _) => NodeShape::TrapBottom,
                        (_, '/') => NodeShape::TrapTop,
                        _ => NodeShape::LeanLeft,
                    };
                    return Some((label, kind));
                }
                _ => {
                    // `[/route]`: a plain rectangle (handled below).
                    c.i = start;
                    break;
                }
            }
        }
    }
    let &(open, close, kind) = SHAPES.iter().find(|(open, ..)| c.rest().starts_with(open))?;
    c.eat(open);
    let end = find_close(c.rest(), open, close)?;
    let label = clean_quoted(&c.rest()[..end], max);
    c.i += end + close.len();
    let label = if label.is_empty() { String::new() } else { label };
    Some((label, kind))
}

fn run(c: &mut Cur) -> String {
    let start = c.i;
    while c.peek().is_some_and(|ch| matches!(ch, '-' | '=' | '.' | '~')) {
        c.bump();
    }
    c.s[start..c.i].to_string()
}

fn end_head(c: &mut Cur) -> Head {
    match c.peek() {
        Some('>') => {
            c.bump();
            Head::Arrow
        }
        Some(ch @ ('o' | 'x')) if c.peek2().map_or(true, |n| n.is_whitespace() || n == '|') => {
            c.bump();
            if ch == 'o' {
                Head::Circle
            } else {
                Head::Cross
            }
        }
        _ => Head::None,
    }
}

fn link(c: &mut Cur, max: usize) -> Option<Link> {
    let start = if c.eat("<") {
        Head::Arrow
    } else if matches!(c.peek(), Some('o' | 'x')) && matches!(c.peek2(), Some('-' | '=')) {
        let ch = c.peek();
        c.bump();
        if ch == Some('o') {
            Head::Circle
        } else {
            Head::Cross
        }
    } else {
        Head::None
    };
    let first = run(c);
    if first.len() < 2 {
        return None;
    }
    let mut end = end_head(c);
    let mut body = first.clone();
    let mut label = None;
    if end == Head::None && matches!(first.as_str(), "--" | "==" | "-.") {
        // `-- text -->`: the text runs to the link's second half.
        let close = if first == "-." { ".-" } else { &first[..] };
        let at = find_unquoted(c.rest(), close)?;
        let text = clean_quoted(&c.rest()[..at], max);
        if text.is_empty() {
            return None;
        }
        label = Some(text);
        c.i += at;
        body = run(c);
        end = end_head(c);
    }
    let line = if body.contains('~') {
        Line::Invisible
    } else if body.contains('=') {
        Line::Thick
    } else if body.contains('.') {
        Line::Dotted
    } else {
        Line::Solid
    };
    let minlen = match line {
        Line::Dotted => body.matches('.').count(),
        _ => {
            let n = body.chars().filter(|&ch| ch == '-' || ch == '=' || ch == '~').count();
            if end == Head::None {
                n.saturating_sub(2)
            } else {
                n.saturating_sub(1)
            }
        }
    }
    .clamp(1, MAX_MINLEN);
    let save = c.i;
    c.skip_ws();
    if c.eat("|") {
        let at = find_unquoted(c.rest(), "|")?;
        let text = clean(c.rest()[..at].trim().trim_matches('"'), max);
        c.i += at + 1;
        if !text.is_empty() {
            label = Some(text);
        }
    } else {
        c.i = save;
    }
    Some(Link { line, start, end, minlen, label })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(src: &str) -> Model {
        let (_, lines) = super::super::common::cleaned(src);
        let header = lines[0].strip_prefix("graph").or_else(|| lines[0].strip_prefix("flowchart")).unwrap();
        parse(header, &lines[1..], &Limits::default()).unwrap()
    }

    #[test]
    fn shapes_and_labels() {
        let m = p("graph LR\nA[Rect] --> B(Round)\nB --> C{Choice?}\nC --> D((Circle))\nD --> E[[Sub]]\nE --> F>Flag]\nF --> G[(DB)]\nG --> H([Pill])\nH --> I{{Hex}}\nI --> J[/In/]");
        assert_eq!(m.dir, Dir::Right);
        let shapes: Vec<NodeShape> = m.nodes.iter().map(|n| n.shape).collect();
        use NodeShape::*;
        assert_eq!(shapes, vec![Rect, Round, Diamond, Circle, Subroutine, Asymmetric, Cylinder, Stadium, Hexagon, LeanRight]);
        assert_eq!(m.nodes[2].label, "Choice?");
        assert_eq!(m.edges.len(), 9);
    }

    #[test]
    fn links_and_labels() {
        let m = p("flowchart TD\nA -->|yes| B\nA -- no --> C\nB -.-> D\nC ==> D\nD --- E\nE -. maybe .-> F\nF --o G\nG --x H\nA <--> H\nA ----> Z");
        let e = &m.edges;
        assert_eq!(e[0].label.as_deref(), Some("yes"));
        assert_eq!(e[1].label.as_deref(), Some("no"));
        assert_eq!((e[2].line, e[2].end), (Line::Dotted, Head::Arrow));
        assert_eq!((e[3].line, e[3].end), (Line::Thick, Head::Arrow));
        assert_eq!((e[4].line, e[4].end), (Line::Solid, Head::None));
        assert_eq!(e[5].label.as_deref(), Some("maybe"));
        assert_eq!(e[6].end, Head::Circle);
        assert_eq!(e[7].end, Head::Cross);
        assert_eq!((e[8].start, e[8].end), (Head::Arrow, Head::Arrow));
        assert_eq!(e[9].minlen, 3);
    }

    #[test]
    fn chains_groups_and_subgraphs() {
        let m = p("graph TD\nsubgraph one [First group]\nA --> B\nend\nsubgraph two\nC\nend\nA & B --> C --> D\none --> D");
        assert_eq!(m.clusters.len(), 2);
        assert_eq!(m.clusters[0].title, "First group");
        assert_eq!(m.nodes.iter().find(|n| n.id == "A").unwrap().cluster, Some(0));
        assert_eq!(m.edges.len(), 5);
        // `one --> D` starts at the subgraph's last member.
        let last = m.edges.last().unwrap();
        assert_eq!(m.nodes[last.from].id, "B");
    }

    #[test]
    fn bounds_are_errors() {
        let mut src = String::from("graph TD\n");
        for i in 0..500 {
            src.push_str(&format!("N{i} --> N{}\n", i + 1));
        }
        let (_, lines) = super::super::common::cleaned(&src);
        assert!(matches!(parse("TD", &lines[1..], &Limits::default()), Err(Error::TooLarge(_))));
        let deep = "graph TD\n".to_string() + &"subgraph s\n".repeat(20) + "A\n";
        let (_, lines) = super::super::common::cleaned(&deep);
        assert!(matches!(parse("TD", &lines[1..], &Limits::default()), Err(Error::TooLarge(_))));
    }

    #[test]
    fn junk_does_not_panic() {
        for s in ["graph TD\nA[", "graph TD\nA-->", "graph TD\n-->B", "graph TD\nA[\"q]", "graph TD\nA-- x", "graph TD\n[/]", "graph TD\nA{{", "graph TD\nä --> ö[ü]"] {
            let (_, lines) = super::super::common::cleaned(s);
            let _ = parse("TD", &lines[1..], &Limits::default());
        }
    }
}
