//! Sequence diagrams (`sequenceDiagram`): parser and drawing.
//!
//! Supported: `participant` and `actor` with `as` aliases (participants
//! also appear on first use); messages `->`, `-->`, `->>`, `-->>`, `-x`,
//! `--x`, `-)`, `--)`, `<<->>`, `<<-->>` with `+`/`-` activation;
//! `activate` / `deactivate`; `Note left of|right of|over A[,B]`;
//! `autonumber [start [step]]` / `autonumber off`; `loop`, `alt`/`else`,
//! `opt`, `par`/`and`, `critical`/`option`, `break` frames and `rect`
//! bands, nested; `box … end` groups are read (drawn without a box);
//! `title`. Heads are drawn at the top and the bottom of each lifeline.
//!
//! The message grammar follows Rune's `sequence.rs` (MIT, © 2026 Anomaly).

use std::collections::HashMap;

use super::common::{arrowhead, clean, ellipse, measure, rect, rounded_rect, Label, Pt};
use super::{Align, Error, Fonts, Item, Limits, Measure, Role};

#[derive(Debug, Clone)]
pub(crate) struct Participant {
    #[allow(dead_code)]
    pub id: String,
    pub label: String,
    pub actor: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tip {
    None,
    Filled,
    Open,
    Cross,
}

#[derive(Debug, Clone)]
pub(crate) struct Message {
    pub from: usize,
    pub to: usize,
    pub text: String,
    pub dotted: bool,
    pub tip: Tip,
    pub both: bool,
    pub activate: bool,
    pub deactivate: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NotePos {
    Left(usize),
    Right(usize),
    Over(usize, usize),
}

#[derive(Debug, Clone)]
pub(crate) enum Event {
    Message(Message),
    Note(NotePos, String),
    Activate(usize),
    Deactivate(usize),
    Number(Option<(u32, u32)>),
    /// A frame (`kind` like "loop") or a `rect` band (`kind` empty).
    Open { kind: String, label: String },
    Else(String),
    End,
}

#[derive(Debug, Clone)]
pub(crate) struct Sequence {
    pub participants: Vec<Participant>,
    pub events: Vec<Event>,
}

const ARROWS: &[(&str, bool, Tip, bool)] = &[
    ("<<-->>", true, Tip::Filled, true),
    ("<<->>", false, Tip::Filled, true),
    ("-->>", true, Tip::Filled, false),
    ("->>", false, Tip::Filled, false),
    ("--x", true, Tip::Cross, false),
    ("-x", false, Tip::Cross, false),
    ("--)", true, Tip::Open, false),
    ("-)", false, Tip::Open, false),
    ("-->", true, Tip::None, false),
    ("->", false, Tip::None, false),
];

/// Most participants a diagram may have.
const MAX_PARTICIPANTS: usize = 40;

pub(crate) fn parse(body: &[&str], limits: &Limits) -> Result<Sequence, Error> {
    let mut p = Parser { s: Sequence { participants: Vec::new(), events: Vec::new() }, ids: HashMap::new(), frames: Vec::new(), limits };
    for line in body {
        p.line(line)?;
        if p.s.events.len() + p.s.participants.len() > limits.max_items {
            return Err(Error::TooLarge("items"));
        }
    }
    // Frames still open at the end close there.
    for f in std::mem::take(&mut p.frames) {
        if f {
            p.s.events.push(Event::End);
        }
    }
    if p.s.participants.is_empty() {
        return Err(Error::Invalid("no participants".into()));
    }
    Ok(p.s)
}

struct Parser<'a> {
    s: Sequence,
    ids: HashMap<String, usize>,
    /// Open blocks: `true` for a drawn frame, `false` for a `box`.
    frames: Vec<bool>,
    limits: &'a Limits,
}

impl Parser<'_> {
    fn participant(&mut self, id: &str, label: Option<String>, actor: bool) -> Result<usize, Error> {
        let id = id.trim();
        if let Some(&i) = self.ids.get(id) {
            if let Some(l) = label {
                self.s.participants[i].label = l;
            }
            if actor {
                self.s.participants[i].actor = true;
            }
            return Ok(i);
        }
        if self.s.participants.len() >= MAX_PARTICIPANTS {
            return Err(Error::TooLarge("participants"));
        }
        let label = label.unwrap_or_else(|| clean(id, self.limits.max_label));
        self.s.participants.push(Participant { id: id.to_string(), label, actor });
        self.ids.insert(id.to_string(), self.s.participants.len() - 1);
        Ok(self.s.participants.len() - 1)
    }

    fn line(&mut self, l: &str) -> Result<(), Error> {
        let max = self.limits.max_label;
        let l = l.trim().trim_end_matches(';').trim_end();
        let (word, rest) = l.split_once(char::is_whitespace).map_or((l, ""), |(w, r)| (w, r.trim()));
        match word {
            "participant" | "actor" => {
                let (id, alias) = match rest.split_once(" as ") {
                    Some((id, a)) => (id.trim(), Some(clean(a, max))),
                    None => (rest, None),
                };
                // v11 metadata (`A@{ "type": "database" }`) is read and left out.
                let id = id.split("@{").next().unwrap_or(id).trim();
                if !id.is_empty() {
                    self.participant(id, alias, word == "actor")?;
                }
            }
            "create" => {
                let r = rest.trim_start_matches("participant").trim_start_matches("actor").trim();
                return self.line(&format!("participant {r}"));
            }
            "destroy" | "title" | "accTitle" | "accDescr" | "links" | "link" | "properties" | "details" => {}
            "autonumber" => {
                let mut nums = rest.split_whitespace().map(|n| n.parse::<u32>().ok());
                let ev = if rest == "off" {
                    Event::Number(None)
                } else {
                    let start = nums.next().flatten().unwrap_or(1);
                    let step = nums.next().flatten().unwrap_or(1).max(1);
                    Event::Number(Some((start, step)))
                };
                self.s.events.push(ev);
            }
            "activate" | "deactivate" => {
                if let Some(&i) = self.ids.get(rest) {
                    self.s.events.push(if word == "activate" { Event::Activate(i) } else { Event::Deactivate(i) });
                }
            }
            "loop" | "alt" | "opt" | "par" | "critical" | "break" | "rect" => {
                if self.frames.len() >= self.limits.max_depth {
                    return Err(Error::TooLarge("nesting"));
                }
                let label = if word == "rect" { String::new() } else { clean(rest, max) };
                let kind = if word == "rect" { String::new() } else { word.to_string() };
                self.frames.push(true);
                self.s.events.push(Event::Open { kind, label });
            }
            "box" => {
                if self.frames.len() >= self.limits.max_depth {
                    return Err(Error::TooLarge("nesting"));
                }
                self.frames.push(false);
            }
            "else" | "and" | "option" => {
                if self.frames.last() == Some(&true) {
                    self.s.events.push(Event::Else(clean(rest, max)));
                }
            }
            "end" if rest.is_empty() => {
                if self.frames.pop() == Some(true) {
                    self.s.events.push(Event::End);
                }
            }
            _ if word.eq_ignore_ascii_case("note") => self.note(rest)?,
            _ => self.message(l)?,
        }
        Ok(())
    }

    fn note(&mut self, rest: &str) -> Result<(), Error> {
        let Some((place, text)) = rest.split_once(':') else { return Ok(()) };
        let text = clean(text, self.limits.max_label);
        let place = place.trim();
        let lower = place.to_ascii_lowercase();
        let pos = if let Some(who) = lower.strip_prefix("left of").map(|_| place["left of".len()..].trim()) {
            NotePos::Left(self.participant(who, None, false)?)
        } else if let Some(who) = lower.strip_prefix("right of").map(|_| place["right of".len()..].trim()) {
            NotePos::Right(self.participant(who, None, false)?)
        } else if let Some(who) = lower.strip_prefix("over").map(|_| place["over".len()..].trim()) {
            match who.split_once(',') {
                Some((a, b)) => {
                    let (a, b) = (self.participant(a, None, false)?, self.participant(b, None, false)?);
                    NotePos::Over(a.min(b), a.max(b))
                }
                None => {
                    let a = self.participant(who, None, false)?;
                    NotePos::Over(a, a)
                }
            }
        } else {
            return Ok(());
        };
        self.s.events.push(Event::Note(pos, text));
        Ok(())
    }

    fn message(&mut self, l: &str) -> Result<(), Error> {
        let (head, text) = match l.split_once(':') {
            Some((h, t)) => (h, clean(t, self.limits.max_label)),
            None => (l, String::new()),
        };
        // The first arrow, longest form at its place.
        let mut found = None;
        for (i, _) in head.char_indices() {
            if let Some(a) = ARROWS.iter().find(|a| head[i..].starts_with(a.0)) {
                found = Some((i, *a));
                break;
            }
        }
        let Some((at, (arrow, dotted, tip, both))) = found else { return Ok(()) };
        let from = head[..at].trim();
        let mut to = head[at + arrow.len()..].trim();
        let (mut activate, mut deactivate) = (false, false);
        if let Some(t) = to.strip_prefix('+') {
            activate = true;
            to = t.trim();
        } else if let Some(t) = to.strip_prefix('-') {
            deactivate = true;
            to = t.trim();
        }
        if from.is_empty() || to.is_empty() {
            return Ok(());
        }
        let from = self.participant(from, None, false)?;
        let to = self.participant(to, None, false)?;
        self.s.events.push(Event::Message(Message { from, to, text, dotted, tip, both, activate, deactivate }));
        Ok(())
    }
}

// ------------------------------------------------------------------ draw

const BAR: f32 = 10.0;

struct Frame {
    kind: Label,
    label: Label,
    band: bool,
    top: f32,
    dividers: Vec<(f32, Label)>,
    x: [f32; 2],
}

fn head_size(actor: bool, t: [f32; 2]) -> [f32; 2] {
    if actor {
        [t[0].max(36.0) + 8.0, 40.0 + t[1]]
    } else {
        [(t[0] + 28.0).max(72.0), t[1] + 18.0]
    }
}

fn dashed(points: Vec<Pt>, role: Role, width: f32) -> Item {
    Item::Stroke { points, closed: false, width, dash: Some((4.0, 4.0)), role }
}

fn line(points: Vec<Pt>, role: Role, width: f32, dotted: bool) -> Item {
    Item::Stroke { points, closed: false, width, dash: dotted.then_some((4.0, 3.0)), role }
}

fn tip_len(t: Tip) -> f32 {
    match t {
        Tip::Filled => 9.0,
        Tip::Open => 2.0,
        Tip::Cross | Tip::None => 0.0,
    }
}

fn tip(items: &mut Vec<Item>, t: Tip, at: Pt, dir: Pt) {
    match t {
        Tip::None => {}
        Tip::Filled => items.push(Item::Fill { points: arrowhead(at, dir, 10.0, 8.0), role: Role::Line }),
        Tip::Open => {
            let a = arrowhead(at, dir, 9.0, 9.0);
            items.push(Item::Stroke { points: vec![a[1], a[0], a[2]], closed: false, width: 1.25, dash: None, role: Role::Line });
        }
        Tip::Cross => {
            let m = [at[0] - dir[0] * 5.0, at[1]];
            for s in [-1.0, 1.0] {
                items.push(Item::Stroke {
                    points: vec![[m[0] - 4.0, m[1] - 4.0 * s], [m[0] + 4.0, m[1] + 4.0 * s]],
                    closed: false,
                    width: 1.5,
                    dash: None,
                    role: Role::Line,
                });
            }
        }
    }
}

fn head(items: &mut Vec<Item>, p: &Participant, label: &Label, cx: f32, top: f32, size: [f32; 2]) {
    if p.actor {
        // A stick figure over the name.
        let c = [cx, top + 7.0];
        let ring = ellipse(c, 6.0, 6.0);
        items.push(Item::Fill { points: ring.clone(), role: Role::NodeFill });
        items.push(Item::Stroke { points: ring, closed: true, width: 1.5, dash: None, role: Role::NodeBorder });
        let s = |pts: Vec<Pt>| Item::Stroke { points: pts, closed: false, width: 1.5, dash: None, role: Role::NodeBorder };
        items.push(s(vec![[cx, top + 13.0], [cx, top + 25.0]]));
        items.push(s(vec![[cx - 9.0, top + 17.0], [cx + 9.0, top + 17.0]]));
        items.push(s(vec![[cx - 8.0, top + 34.0], [cx, top + 25.0], [cx + 8.0, top + 34.0]]));
        items.push(label.item([cx, top + 38.0 + label.h / 2.0], Role::Text, Align::Center));
    } else {
        let r = rounded_rect(cx - size[0] / 2.0, top, cx + size[0] / 2.0, top + size[1], 4.0);
        items.push(Item::Fill { points: r.clone(), role: Role::NodeFill });
        items.push(Item::Stroke { points: r, closed: true, width: 1.25, dash: None, role: Role::NodeBorder });
        items.push(label.item([cx, top + size[1] / 2.0], Role::Text, Align::Center));
    }
}

pub(crate) fn draw(s: &Sequence, fonts: &Fonts, m: &mut dyn Measure) -> Vec<Item> {
    let wrap = 220.0 * fonts.body / 13.0;
    let n = s.participants.len();
    let heads: Vec<Label> = s.participants.iter().map(|p| measure(m, &p.label, fonts.body, false, wrap)).collect();
    let boxes: Vec<[f32; 2]> = s.participants.iter().zip(&heads).map(|(p, l)| head_size(p.actor, l.size2())).collect();
    let top_h = boxes.iter().map(|b| b[1]).fold(0.0, f32::max);
    let texts: Vec<Option<Label>> = s
        .events
        .iter()
        .map(|e| match e {
            Event::Message(msg) => Some(measure(m, &msg.text, fonts.small, false, wrap)),
            Event::Note(_, t) => Some(measure(m, t, fonts.small, false, wrap)),
            Event::Open { label, .. } | Event::Else(label) => {
                Some(measure(m, &if label.is_empty() { String::new() } else { format!("[{label}]") }, fonts.small, false, wrap))
            }
            _ => None,
        })
        .collect();
    let width_of = |i: usize| texts[i].as_ref().map_or(0.0, |l| l.w);
    let numbered = s.events.iter().any(|e| matches!(e, Event::Number(Some(_))));
    let number = if numbered { 20.0 } else { 0.0 };

    // Columns: heads side by side, then room for the text between them.
    let mut need: Vec<(usize, usize, f32)> = Vec::new();
    for (i, e) in s.events.iter().enumerate() {
        match e {
            Event::Message(msg) if msg.from != msg.to => {
                need.push((msg.from.min(msg.to), msg.from.max(msg.to), width_of(i) + 32.0 + number))
            }
            Event::Message(msg) if msg.from + 1 < n => need.push((msg.from, msg.from + 1, width_of(i) + 48.0)),
            Event::Note(NotePos::Right(a), _) if a + 1 < n => need.push((*a, a + 1, width_of(i) + 40.0)),
            Event::Note(NotePos::Left(a), _) if *a > 0 => need.push((a - 1, *a, width_of(i) + 40.0)),
            _ => {}
        }
    }
    let left_room = s
        .events
        .iter()
        .enumerate()
        .filter_map(|(i, e)| match e {
            Event::Note(NotePos::Left(0), _) => Some(width_of(i) + 32.0),
            _ => None,
        })
        .fold(0.0, f32::max);
    let mut cx = vec![(boxes[0][0] / 2.0).max(left_room); n];
    for j in 1..n {
        cx[j] = cx[j - 1] + (boxes[j - 1][0] + boxes[j][0]) / 2.0 + 28.0;
        for &(a, b, d) in &need {
            if b == j {
                cx[j] = cx[j].max(cx[a] + d);
            }
        }
    }

    let (mut back, mut lifelines, mut bars, mut front, mut tabs) = (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut active: Vec<Vec<f32>> = vec![Vec::new(); n];
    let mut frames: Vec<Frame> = Vec::new();
    let mut y = top_h + 20.0;
    let mut num: Option<(u32, u32)> = None;
    let edge_x = |active: &[Vec<f32>], i: usize, right: bool| {
        let d = active[i].len();
        if d == 0 {
            cx[i]
        } else {
            cx[i] + (d - 1) as f32 * 4.0 + if right { BAR / 2.0 } else { -BAR / 2.0 }
        }
    };
    let close_bar = |bars: &mut Vec<Item>, active: &mut [Vec<f32>], i: usize, y: f32| {
        let d = active[i].len();
        if let Some(top) = active[i].pop() {
            let x = cx[i] + (d - 1) as f32 * 4.0;
            let r = rect(x - BAR / 2.0, top, x + BAR / 2.0, y.max(top + 8.0));
            bars.push(Item::Fill { points: r.clone(), role: Role::ActivationFill });
            bars.push(Item::Stroke { points: r, closed: true, width: 1.0, dash: None, role: Role::NodeBorder });
        }
    };
    let touch = |frames: &mut [Frame], x0: f32, x1: f32| {
        for f in frames.iter_mut() {
            f.x = [f.x[0].min(x0), f.x[1].max(x1)];
        }
    };
    let small = fonts.small;

    for (i, e) in s.events.iter().enumerate() {
        match e {
            Event::Message(msg) => {
                let t = texts[i].clone().unwrap_or_default();
                if msg.activate {
                    active[msg.to].push(y + t.h + 6.0);
                }
                let line_y;
                if msg.from == msg.to {
                    let x = edge_x(&active, msg.from, true);
                    if !t.is_empty() {
                        front.push(t.item([x + 8.0, y + t.h / 2.0], Role::Muted, Align::Left));
                    }
                    line_y = y + t.h + 6.0;
                    let pts = vec![[x, line_y], [x + 30.0, line_y], [x + 30.0, line_y + 18.0], [x + 2.0 + tip_len(msg.tip), line_y + 18.0]];
                    front.push(line(pts, Role::Line, 1.25, msg.dotted));
                    tip(&mut front, msg.tip, [x, line_y + 18.0], [-1.0, 0.0]);
                    touch(&mut frames, cx[msg.from] - BAR, (x + 30.0).max(x + 8.0 + t.w));
                    y = line_y + 18.0 + 16.0;
                } else {
                    let right = msg.to > msg.from;
                    let sx = edge_x(&active, msg.from, right);
                    let ex = edge_x(&active, msg.to, !right);
                    let dir = if right { 1.0 } else { -1.0 };
                    let mid = (sx + ex) / 2.0;
                    if !t.is_empty() {
                        front.push(t.item([mid, y + t.h / 2.0], Role::Muted, Align::Center));
                    }
                    line_y = y + if t.is_empty() { 4.0 } else { t.h + 6.0 };
                    let a = if msg.both { sx + dir * tip_len(msg.tip) } else { sx };
                    front.push(line(vec![[a, line_y], [ex - dir * tip_len(msg.tip), line_y]], Role::Line, 1.25, msg.dotted));
                    tip(&mut front, msg.tip, [ex, line_y], [dir, 0.0]);
                    if msg.both {
                        tip(&mut front, msg.tip, [sx, line_y], [-dir, 0.0]);
                    }
                    if let Some((k, step)) = &mut num {
                        let label = measure(m, &k.to_string(), (small - 2.0).max(8.0), true, f32::MAX);
                        let r = (label.w / 2.0 + 4.0).max(8.0);
                        front.push(Item::Fill { points: rounded_rect(sx - r, line_y - 8.0, sx + r, line_y + 8.0, 8.0), role: Role::Accent });
                        front.push(label.item([sx, line_y], Role::Background, Align::Center));
                        *k = k.saturating_add(*step);
                    }
                    let (lo, hi) = (cx[msg.from].min(cx[msg.to]) - BAR, cx[msg.from].max(cx[msg.to]) + BAR);
                    touch(&mut frames, lo.min(mid - t.w / 2.0), hi.max(mid + t.w / 2.0));
                    y = line_y + 16.0;
                }
                if msg.deactivate {
                    close_bar(&mut bars, &mut active, msg.from, line_y);
                }
            }
            Event::Number(on) => num = *on,
            Event::Note(pos, _) => {
                let t = texts[i].clone().unwrap_or_default();
                let w = t.w + 20.0;
                let (x0, x1) = match *pos {
                    NotePos::Left(a) => (cx[a] - 12.0 - w, cx[a] - 12.0),
                    NotePos::Right(a) => (cx[a] + 12.0, cx[a] + 12.0 + w),
                    NotePos::Over(a, b) => {
                        let span = (cx[b] - cx[a] + 40.0).max(w);
                        let mid = (cx[a] + cx[b]) / 2.0;
                        (mid - span / 2.0, mid + span / 2.0)
                    }
                };
                let h = t.h + 14.0;
                y += 2.0;
                let r = rounded_rect(x0, y, x1, y + h, 3.0);
                front.push(Item::Fill { points: r.clone(), role: Role::NoteFill });
                front.push(Item::Stroke { points: r, closed: true, width: 1.0, dash: None, role: Role::NoteBorder });
                front.push(t.item([(x0 + x1) / 2.0, y + h / 2.0], Role::Text, Align::Center));
                touch(&mut frames, x0, x1);
                y += h + 14.0;
            }
            Event::Activate(a) => active[*a].push(y - 8.0),
            Event::Deactivate(a) => close_bar(&mut bars, &mut active, *a, y - 8.0),
            Event::Open { kind, .. } => {
                let label = texts[i].clone().unwrap_or_default();
                let band = kind.is_empty();
                let kind = measure(m, kind, small, true, f32::MAX);
                let h = if band { 8.0 } else { kind.h.max(label.h) + 14.0 };
                frames.push(Frame { kind, label, band, top: y, dividers: Vec::new(), x: [f32::MAX, f32::MIN] });
                y += h;
            }
            Event::Else(_) => {
                let label = texts[i].clone().unwrap_or_default();
                let h = label.h.max(small) + 14.0;
                if let Some(f) = frames.last_mut() {
                    f.dividers.push((y, label));
                }
                y += h;
            }
            Event::End => {
                let Some(mut f) = frames.pop() else { continue };
                if f.x[0] > f.x[1] {
                    f.x = [cx[0], cx[n - 1]];
                }
                let x0 = f.x[0] - 12.0;
                let tab_w = if f.band { 0.0 } else { f.kind.w + 18.0 };
                let x1 = (f.x[1] + 12.0).max(x0 + tab_w + f.label.w + 20.0);
                let bottom = y - 4.0;
                if f.band {
                    back.push(Item::Fill { points: rounded_rect(x0, f.top - 2.0, x1, bottom, 4.0), role: Role::ClusterFill });
                } else {
                    let th = f.kind.h.max(f.label.h) + 6.0;
                    back.push(Item::Fill { points: rect(x0, f.top, x1, bottom), role: Role::ClusterFill });
                    back.push(Item::Stroke { points: rect(x0, f.top, x1, bottom), closed: true, width: 1.0, dash: None, role: Role::ClusterBorder });
                    let tab = vec![[x0, f.top], [x0 + tab_w, f.top], [x0 + tab_w, f.top + th - 6.0], [x0 + tab_w - 6.0, f.top + th], [x0, f.top + th]];
                    tabs.push(Item::Fill { points: tab.clone(), role: Role::NodeFill });
                    tabs.push(Item::Stroke { points: tab, closed: true, width: 1.0, dash: None, role: Role::ClusterBorder });
                    tabs.push(f.kind.item([x0 + 9.0, f.top + th / 2.0], Role::Text, Align::Left));
                    if !f.label.is_empty() {
                        tabs.push(f.label.item([x0 + tab_w + 8.0, f.top + th / 2.0], Role::Muted, Align::Left));
                    }
                    for (dy, label) in &f.dividers {
                        back.push(dashed(vec![[x0, *dy], [x1, *dy]], Role::ClusterBorder, 1.0));
                        if !label.is_empty() {
                            tabs.push(label.item([x0 + 9.0, dy + 4.0 + label.h / 2.0], Role::Muted, Align::Left));
                        }
                    }
                }
                touch(&mut frames, x0, x1);
                y = bottom + 16.0;
            }
        }
    }
    let end = y + 4.0;
    for a in 0..n {
        while !active[a].is_empty() {
            close_bar(&mut bars, &mut active, a, end - 8.0);
        }
    }
    let mut heads_items = Vec::new();
    for (j, p) in s.participants.iter().enumerate() {
        let size = boxes[j];
        let top = top_h - size[1];
        head(&mut heads_items, p, &heads[j], cx[j], top, size);
        head(&mut heads_items, p, &heads[j], cx[j], end, size);
        lifelines.push(dashed(vec![[cx[j], top_h], [cx[j], end]], Role::ClusterBorder, 1.0));
    }
    let mut items = back;
    items.append(&mut lifelines);
    items.append(&mut bars);
    items.append(&mut front);
    items.append(&mut tabs);
    items.append(&mut heads_items);
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(src: &str) -> Sequence {
        let (_, lines) = super::super::common::cleaned(src);
        parse(&lines[1..], &Limits::default()).unwrap()
    }

    #[test]
    fn participants_messages_and_notes() {
        let s = p("sequenceDiagram\nparticipant A as Alice\nactor B as Bob\nA->>+B: Hello\nB-->>-A: Hi\nA-)B: async\nA-xB: lost\nA->>A: self\nNote over A,B: both\nNote left of A: left\nC->>A: new");
        assert_eq!(s.participants.len(), 3);
        assert_eq!(s.participants[0].label, "Alice");
        assert!(s.participants[1].actor);
        let msgs: Vec<&Message> = s.events.iter().filter_map(|e| if let Event::Message(m) = e { Some(m) } else { None }).collect();
        assert_eq!(msgs.len(), 6);
        assert!(msgs[0].activate && msgs[0].tip == Tip::Filled && !msgs[0].dotted);
        assert!(msgs[1].deactivate && msgs[1].dotted);
        assert_eq!(msgs[2].tip, Tip::Open);
        assert_eq!(msgs[3].tip, Tip::Cross);
        assert_eq!(msgs[4].from, msgs[4].to);
        assert!(s.events.iter().any(|e| matches!(e, Event::Note(NotePos::Over(0, 1), _))));
    }

    #[test]
    fn frames_nest_and_close() {
        let s = p("sequenceDiagram\nloop Every minute\nA->>B: ping\nalt ok\nB->>A: pong\nelse fail\nB-xA: error\nend\nbox Group\nend\nopt unclosed\nA->>B: x");
        let opens = s.events.iter().filter(|e| matches!(e, Event::Open { .. })).count();
        let ends = s.events.iter().filter(|e| matches!(e, Event::End)).count();
        assert_eq!((opens, ends), (3, 3));
        assert!(s.events.iter().any(|e| matches!(e, Event::Else(l) if l == "fail")));
    }

    #[test]
    fn too_many_participants_is_an_error() {
        let mut src = String::from("sequenceDiagram\n");
        for i in 0..50 {
            src.push_str(&format!("participant P{i}\n"));
        }
        let (_, lines) = super::super::common::cleaned(&src);
        assert!(matches!(parse(&lines[1..], &Limits::default()), Err(Error::TooLarge(_))));
    }
}
