//! State diagrams (`stateDiagram` / `stateDiagram-v2`), parsed into a
//! [`graph::Model`](super::graph::Model) and drawn with the layered layout.
//!
//! Supported: `direction TB|BT|LR|RL`; `[*]` start and end pseudostates
//! (one pair per composite); `state "Label" as id`; `id : description`;
//! transitions `A --> B` and `A --> B : label`; `state id <<choice>>`,
//! `<<fork>>`, `<<join>>`; composite `state Name { … }` (nested, with
//! `--` region separators read as one region); notes `note left of A :
//! text` and multi-line `note right of A` … `end note`. `classDef`,
//! `class`, `style` and `:::class` are read and ignored.

use std::collections::HashMap;

use super::common::{clean, clean_quoted, statements};
use super::graph::{Cluster, Dir, Edge, Head, Line, Model, Node};
use super::shapes::NodeShape;
use super::{Error, Limits};

pub(crate) fn parse(body: &[&str], limits: &Limits) -> Result<Model, Error> {
    let mut p = Parser {
        m: Model { dir: Dir::Down, nodes: Vec::new(), edges: Vec::new(), clusters: Vec::new() },
        ids: HashMap::new(),
        composites: HashMap::new(),
        stack: Vec::new(),
        note: None,
        limits,
    };
    for line in body {
        if p.note.is_some() {
            p.note_line(line)?;
            continue;
        }
        for stmt in statements(line) {
            p.statement(stmt)?;
        }
    }
    if p.note.is_some() {
        p.note_line("end note")?;
    }
    p.resolve_composites();
    if p.m.nodes.is_empty() {
        return Err(Error::Invalid("no states".into()));
    }
    Ok(p.m)
}

struct Parser<'a> {
    m: Model,
    ids: HashMap<String, usize>,
    /// Composite id → (cluster, its start and end pseudostates).
    composites: HashMap<String, usize>,
    stack: Vec<usize>,
    /// A multi-line note being read: the state it belongs to and its lines.
    note: Option<(String, bool, Vec<String>)>,
    limits: &'a Limits,
}

impl Parser<'_> {
    fn budget(&self) -> Result<(), Error> {
        if self.m.nodes.len() + self.m.edges.len() + self.m.clusters.len() >= self.limits.max_items {
            return Err(Error::TooLarge("items"));
        }
        Ok(())
    }

    fn scope(&self) -> String {
        self.stack.last().map_or(String::new(), |c| c.to_string())
    }

    /// The node for `id` in the current scope, created when new.
    fn state(&mut self, id: &str, start_side: bool) -> Result<usize, Error> {
        let (key, shape, label) = if id == "[*]" {
            let k = if start_side { format!("[*]start@{}", self.scope()) } else { format!("[*]end@{}", self.scope()) };
            (k, if start_side { NodeShape::Start } else { NodeShape::End }, String::new())
        } else {
            (id.to_string(), NodeShape::Round, id.to_string())
        };
        if let Some(&i) = self.ids.get(&key) {
            return Ok(i);
        }
        self.budget()?;
        self.m.nodes.push(Node { id: key.clone(), label, shape, cluster: self.stack.last().copied(), desc: Vec::new() });
        let i = self.m.nodes.len() - 1;
        self.ids.insert(key, i);
        Ok(i)
    }

    fn note_line(&mut self, line: &str) -> Result<(), Error> {
        let l = line.trim();
        if l.eq_ignore_ascii_case("end note") {
            if let Some((target, left, lines)) = self.note.take() {
                self.add_note(&target, left, &lines.join("\n"))?;
            }
        } else if let Some((_, _, lines)) = &mut self.note {
            if lines.len() < 32 {
                lines.push(clean(l, self.limits.max_label));
            }
        }
        Ok(())
    }

    fn add_note(&mut self, target: &str, left: bool, text: &str) -> Result<(), Error> {
        let t = self.state(target, false)?;
        self.budget()?;
        let id = format!("note{}", self.m.nodes.len());
        self.m.nodes.push(Node {
            id: id.clone(),
            label: clean(text, self.limits.max_label),
            shape: NodeShape::Note,
            cluster: self.m.nodes[t].cluster,
            desc: Vec::new(),
        });
        let n = self.m.nodes.len() - 1;
        self.ids.insert(id, n);
        let (from, to) = if left { (n, t) } else { (t, n) };
        self.m.edges.push(Edge { from, to, label: None, line: Line::Dotted, start: Head::None, end: Head::None, minlen: 1 });
        Ok(())
    }

    fn statement(&mut self, s: &str) -> Result<(), Error> {
        let max = self.limits.max_label;
        if s == "}" {
            self.stack.pop();
            return Ok(());
        }
        if s == "--" {
            return Ok(());
        }
        let word = s.split_whitespace().next().unwrap_or("");
        match word {
            "direction" => {
                if let Some(d) = Dir::parse(&s["direction".len()..]) {
                    if self.stack.is_empty() {
                        self.m.dir = d;
                    }
                }
                return Ok(());
            }
            "classDef" | "class" | "style" | "accTitle" | "accDescr" | "title" | "hide" | "scale" => return Ok(()),
            "note" => return self.note_stmt(s),
            "state" => return self.state_stmt(s["state".len()..].trim()),
            _ => {}
        }
        if let Some(at) = s.find("-->") {
            let from = s[..at].trim();
            let rest = &s[at + 3..];
            let (to, label) = match rest.find(':') {
                Some(c) => (rest[..c].trim(), Some(clean(&rest[c + 1..], max))),
                None => (rest.trim(), None),
            };
            let (from, to) = (strip_class(from), strip_class(to));
            if from.is_empty() || to.is_empty() {
                return Ok(());
            }
            let a = self.state(from, true)?;
            let b = self.state(to, false)?;
            self.budget()?;
            self.m.edges.push(Edge {
                from: a,
                to: b,
                label: label.filter(|l| !l.is_empty()),
                line: Line::Solid,
                start: Head::None,
                end: Head::Arrow,
                minlen: 1,
            });
            return Ok(());
        }
        if let Some(c) = s.find(':') {
            // `id : description`
            let id = strip_class(s[..c].trim());
            if !id.is_empty() && !id.contains(char::is_whitespace) {
                let i = self.state(id, false)?;
                let d = clean(&s[c + 1..], max);
                if !d.is_empty() && self.m.nodes[i].desc.len() < 8 {
                    self.m.nodes[i].desc.push(d);
                }
            }
            return Ok(());
        }
        let id = strip_class(s);
        if !id.is_empty() && !id.contains(char::is_whitespace) {
            self.state(id, false)?;
        }
        Ok(())
    }

    fn note_stmt(&mut self, s: &str) -> Result<(), Error> {
        let rest = s["note".len()..].trim();
        let (left, rest) = if let Some(r) = rest.strip_prefix("left of") {
            (true, r)
        } else if let Some(r) = rest.strip_prefix("right of") {
            (false, r)
        } else {
            return Ok(());
        };
        let rest = rest.trim();
        match rest.find(':') {
            Some(c) => {
                let target = rest[..c].trim().to_string();
                self.add_note(&target, left, &rest[c + 1..])
            }
            None => {
                self.note = Some((rest.to_string(), left, Vec::new()));
                Ok(())
            }
        }
    }

    fn state_stmt(&mut self, rest: &str) -> Result<(), Error> {
        let max = self.limits.max_label;
        // `state "Label" as id`
        if rest.starts_with('"') {
            if let Some(end) = rest[1..].find('"') {
                let label = clean_quoted(&rest[..end + 2], max);
                let after = rest[end + 2..].trim();
                let after = after.strip_prefix("as").map(str::trim).unwrap_or(after);
                let (id, open) = match after.strip_suffix('{') {
                    Some(id) => (id.trim(), true),
                    None => (after, false),
                };
                if id.is_empty() {
                    return Ok(());
                }
                if open {
                    return self.open_composite(id, &label);
                }
                let i = self.state(id, false)?;
                self.m.nodes[i].label = label;
                return Ok(());
            }
            return Ok(());
        }
        // `state id as "Label"` (accepted) / `state id <<choice>>` / `state id {`
        if let Some(id) = rest.strip_suffix('{') {
            let id = id.trim();
            let (id, label) = match id.split_once(" as ") {
                Some((a, b)) => (a.trim(), clean_quoted(b, max)),
                None => (id, id.to_string()),
            };
            return self.open_composite(id, &label);
        }
        let mut parts = rest.split_whitespace();
        let id = strip_class(parts.next().unwrap_or(""));
        if id.is_empty() {
            return Ok(());
        }
        let kind = parts.next().unwrap_or("");
        let i = self.state(id, false)?;
        match kind {
            "<<choice>>" => {
                self.m.nodes[i].shape = NodeShape::Choice;
                self.m.nodes[i].label.clear();
            }
            "<<fork>>" | "<<join>>" => {
                self.m.nodes[i].shape = NodeShape::Bar;
                self.m.nodes[i].label.clear();
            }
            _ => {}
        }
        Ok(())
    }

    fn open_composite(&mut self, id: &str, label: &str) -> Result<(), Error> {
        self.budget()?;
        if self.stack.len() >= self.limits.max_depth {
            return Err(Error::TooLarge("nesting"));
        }
        let c = match self.composites.get(id) {
            Some(&c) => c,
            None => {
                self.m.clusters.push(Cluster { title: label.to_string(), parent: self.stack.last().copied() });
                let c = self.m.clusters.len() - 1;
                self.composites.insert(id.to_string(), c);
                c
            }
        };
        self.stack.push(c);
        Ok(())
    }

    /// Edges to a composite go to its start state (or first member); edges
    /// from it leave its end state (or last member). Placeholder nodes
    /// named like a composite are dropped.
    fn resolve_composites(&mut self) {
        let mut redirect: HashMap<usize, (usize, usize)> = HashMap::new();
        for (id, &c) in &self.composites {
            let Some(&v) = self.ids.get(id) else { continue };
            let members: Vec<usize> = (0..self.m.nodes.len()).filter(|&i| i != v && self.in_cluster(i, c)).collect();
            let start = members.iter().copied().find(|&i| self.m.nodes[i].shape == NodeShape::Start && self.m.nodes[i].cluster == Some(c));
            let end = members.iter().copied().find(|&i| self.m.nodes[i].shape == NodeShape::End && self.m.nodes[i].cluster == Some(c));
            let first = start.or_else(|| members.first().copied());
            let last = end.or_else(|| members.last().copied());
            if let (Some(f), Some(l)) = (first, last) {
                redirect.insert(v, (f, l));
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
        self.m.edges.retain(|e| e.from != usize::MAX && e.to != usize::MAX && !(e.from == e.to && e.label.is_none()));
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

/// `A:::class` → `A`.
fn strip_class(s: &str) -> &str {
    s.split(":::").next().unwrap_or("").trim()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(src: &str) -> Model {
        let (_, lines) = super::super::common::cleaned(src);
        parse(&lines[1..], &Limits::default()).unwrap()
    }

    #[test]
    fn states_transitions_and_pseudostates() {
        let m = p("stateDiagram-v2\n[*] --> Still\nStill --> Moving : push\nMoving --> Still\nMoving --> Crash\nCrash --> [*]\nstate \"Long name\" as L\nL : a description");
        assert_eq!(m.nodes.iter().filter(|n| n.shape == NodeShape::Start).count(), 1);
        assert_eq!(m.nodes.iter().filter(|n| n.shape == NodeShape::End).count(), 1);
        assert_eq!(m.edges.len(), 5);
        assert_eq!(m.edges[1].label.as_deref(), Some("push"));
        let l = m.nodes.iter().find(|n| n.id == "L").unwrap();
        assert_eq!(l.label, "Long name");
        assert_eq!(l.desc, vec!["a description".to_string()]);
    }

    #[test]
    fn composites_choice_fork_and_notes() {
        let m = p("stateDiagram\ndirection LR\n[*] --> Active\nstate Active {\n[*] --> Idle\nIdle --> Busy\nBusy --> [*]\n}\nActive --> Done\nstate pick <<choice>>\nstate f <<fork>>\nDone --> pick\nnote right of Done : finished\nnote left of pick\nline one\nline two\nend note");
        assert_eq!(m.dir, Dir::Right);
        assert_eq!(m.clusters.len(), 1);
        assert_eq!(m.clusters[0].title, "Active");
        assert!(m.nodes.iter().all(|n| n.id != "Active"), "composite placeholder dropped");
        assert!(m.nodes.iter().any(|n| n.shape == NodeShape::Choice));
        assert!(m.nodes.iter().any(|n| n.shape == NodeShape::Bar));
        let notes: Vec<&Node> = m.nodes.iter().filter(|n| n.shape == NodeShape::Note).collect();
        assert_eq!(notes.len(), 2);
        assert_eq!(notes[1].label, "line one\nline two");
        // Two start states: the top-level one and the composite's.
        assert_eq!(m.nodes.iter().filter(|n| n.shape == NodeShape::Start).count(), 2);
    }
}
