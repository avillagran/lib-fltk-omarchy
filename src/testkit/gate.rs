//! The motion gate: follow rectangles frame by frame and find what a
//! person would see go wrong.

use crate::anim::clock;
use fltk::prelude::*;
use std::collections::HashMap;
use std::fmt;
use std::time::Duration;

type Rect = (i32, i32, i32, i32);

/// Where a tracked thing is in one frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sample {
    /// Window rectangle `(x, y, w, h)`.
    pub rect: Rect,
    /// 0 (invisible) to 1 (opaque); 1 when the probe does not know.
    pub opacity: f32,
}

impl Sample {
    pub fn at(rect: Rect) -> Sample {
        Sample { rect, opacity: 1.0 }
    }
}

/// Thresholds of [`Gate::check`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rules {
    /// A one-frame move longer than this (px, either axis) between frames
    /// that hold still is a jump. Default 8.
    pub max_jump: i32,
    /// Going past where a move ends (or back behind where it started) by
    /// more than this (px) is overshoot. Default 2.
    pub overshoot: i32,
    /// Shown → gone → shown (or gone → shown → gone) within this many
    /// frames is a pop. Default 4.
    pub pop_frames: usize,
    /// An opacity drop at least this big that comes back within
    /// `pop_frames` is a pop. Default 0.5.
    pub opacity_drop: f32,
    /// Frames marked by [`Gate::act`] may jump (a deliberate change). Off
    /// by default: an input must not make content that stays jump.
    pub inputs_excuse: bool,
}

impl Default for Rules {
    fn default() -> Self {
        Rules { max_jump: 8, overshoot: 2, pop_frames: 4, opacity_drop: 0.5, inputs_excuse: false }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindingKind {
    /// Moved far in one frame without a transition around it.
    Jump,
    /// Disappeared and came back (or blinked in) within a few frames, or
    /// its opacity dropped and came back.
    Pop,
    /// Went past where its move ended.
    Overshoot,
}

/// One thing [`Gate::check`] found.
#[derive(Debug, Clone, PartialEq)]
pub struct Finding {
    pub kind: FindingKind,
    pub track: String,
    /// The frame it shows on (0-based).
    pub frame: usize,
    pub detail: String,
}

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?} of {} at frame {}: {}", self.kind, self.track, self.frame, self.detail)
    }
}

type Probe = Box<dyn Fn() -> Vec<(String, Option<Sample>)>>;

/// Records tracked rectangles on every frame of a scenario on the frozen
/// animation clock, then [`check`](Gate::check)s them:
///
/// ```no_run
/// # use fltk_omarchy::{testkit::{Gate, Rules}, text::RichText, AppWindow};
/// # fn demo(win: &AppWindow, view: &RichText) {
/// let mut gate = Gate::new(&**win);
/// gate.track_messages(view);
/// gate.frame(); // the settled start
/// gate.act("prepend", || view.prepend_messages(Vec::new()));
/// gate.play(400);
/// gate.verify(&Rules::default()).unwrap();
/// # }
/// ```
///
/// Each frame advances the frozen clock ([`anim::clock`](crate::anim::clock)),
/// runs pending events and posted work, renders the window offscreen
/// (paint-driven motion moves on as on screen) and samples every probe.
/// Animations must be on (`anim::set_enabled(true)`) to see transitions.
pub struct Gate {
    win: fltk::window::DoubleWindow,
    step: Duration,
    probes: Vec<Probe>,
    names: Vec<String>,
    index: HashMap<String, usize>,
    series: Vec<Vec<Option<Sample>>>,
    frames: usize,
    inputs: Vec<(usize, String)>,
}

impl Gate {
    /// A gate on `window`, 16 ms frames.
    pub fn new<W: WindowExt>(window: &W) -> Gate {
        Gate {
            // SAFETY: a live window.
            win: unsafe { fltk::window::DoubleWindow::from_widget_ptr(window.as_widget_ptr()) },
            step: Duration::from_millis(16),
            probes: Vec::new(),
            names: Vec::new(),
            index: HashMap::new(),
            series: Vec::new(),
            frames: 0,
            inputs: Vec::new(),
        }
    }

    /// Frame length (default 16 ms).
    pub fn frame_ms(mut self, ms: u64) -> Gate {
        self.step = Duration::from_millis(ms.max(1));
        self
    }

    /// Follows one thing; `None` when it is not shown in that frame.
    pub fn track(&mut self, name: &str, f: impl Fn() -> Option<Rect> + 'static) -> &mut Gate {
        let name = name.to_string();
        self.probes.push(Box::new(move || vec![(name.clone(), f().map(Sample::at))]));
        self
    }

    /// Like [`track`](Self::track) with opacity.
    pub fn track_sample(&mut self, name: &str, f: impl Fn() -> Option<Sample> + 'static) -> &mut Gate {
        let name = name.to_string();
        self.probes.push(Box::new(move || vec![(name.clone(), f())]));
        self
    }

    /// Follows a changing set: `f` lists what is shown now, by key (tracks
    /// are named `prefix` + key); missing keys are not shown.
    pub fn track_set(&mut self, prefix: &str, f: impl Fn() -> Vec<(String, Rect)> + 'static) -> &mut Gate {
        let prefix = prefix.to_string();
        self.probes.push(Box::new(move || {
            f().into_iter().map(|(k, r)| (format!("{prefix}{k}"), Some(Sample::at(r)))).collect()
        }));
        self
    }

    /// Follows a widget while it is shown in its window.
    pub fn track_widget<W: WidgetExt + Clone + 'static>(&mut self, name: &str, w: &W) -> &mut Gate {
        let w = w.clone();
        self.track(name, move || {
            (!w.was_deleted() && crate::focus::visible_in_window(&w)).then(|| (w.x(), w.y(), w.w(), w.h()))
        })
    }

    /// Follows every message of a transcript by key (`msg:<key>`), while
    /// it is in view ([`RichText::message_rect`](crate::text::RichText::message_rect)).
    pub fn track_messages(&mut self, view: &crate::text::RichText) -> &mut Gate {
        let v = view.clone();
        self.track_set("msg:", move || {
            v.message_keys().into_iter().filter_map(|k| v.message_rect(&k).map(|r| (k, r))).collect()
        })
    }

    /// Follows NavTree rows by key (`item:<key>`), row motion included
    /// ([`NavTree::item_rect`](crate::navtree::NavTree::item_rect)).
    pub fn track_items(&mut self, nav: &crate::navtree::NavTree, keys: &[&str]) -> &mut Gate {
        let nav = nav.clone();
        let keys: Vec<String> = keys.iter().map(|k| k.to_string()).collect();
        self.track_set("item:", move || {
            keys.iter().filter_map(|k| nav.item_rect(k).map(|r| (k.clone(), r))).collect()
        })
    }

    /// Follows the visible elements of [`tree`](super::tree) named `name`
    /// (`el:<name>`): list rows and other virtual rows by their name.
    pub fn track_element(&mut self, name: &str) -> &mut Gate {
        let (win, n) = (self.win.clone(), name.to_string());
        self.track(&format!("el:{name}"), move || {
            super::tree(&win).into_iter().find(|e| e.visible && e.name == n).map(|e| e.rect)
        })
    }

    /// Samples the current state as a frame (the clock does not move;
    /// the window is rendered first).
    pub fn frame(&mut self) {
        super::script::pump();
        super::render_frame(&self.win);
        self.sample();
    }

    /// Records `ms` of frames: each advances the clock one frame.
    pub fn play(&mut self, ms: u64) {
        let n = (ms as f64 / self.step.as_millis().max(1) as f64).ceil() as usize;
        for _ in 0..n {
            clock::advance(self.step);
            self.frame();
        }
    }

    /// An input: `f` runs, then the frame it lands on is recorded (marked
    /// `what`). See [`Rules::inputs_excuse`].
    pub fn act(&mut self, what: &str, f: impl FnOnce()) {
        f();
        self.inputs.push((self.frames, what.to_string()));
        clock::advance(self.step);
        self.frame();
    }

    /// Frames recorded so far.
    pub fn frames(&self) -> usize {
        self.frames
    }

    /// Track names, in the order they first appeared.
    pub fn tracks(&self) -> &[String] {
        &self.names
    }

    /// The samples of track `name`, one per frame.
    pub fn series(&self, name: &str) -> Option<&[Option<Sample>]> {
        self.index.get(name).map(|&i| self.series[i].as_slice())
    }

    fn sample(&mut self) {
        let mut seen: Vec<(String, Option<Sample>)> = Vec::new();
        for p in &self.probes {
            seen.extend(p());
        }
        for (name, s) in seen {
            let i = match self.index.get(&name) {
                Some(&i) => i,
                None => {
                    self.names.push(name.clone());
                    self.series.push(vec![None; self.frames]);
                    self.index.insert(name, self.series.len() - 1);
                    self.series.len() - 1
                }
            };
            let v = &mut self.series[i];
            if v.len() == self.frames {
                v.push(s);
            } else if let Some(last) = v.last_mut() {
                // Two probes reporting the same name: the shown one wins.
                if last.is_none() {
                    *last = s;
                }
            }
        }
        self.frames += 1;
        for v in &mut self.series {
            v.resize(self.frames, None);
        }
    }

    /// Everything found in the recording.
    pub fn check(&self, rules: &Rules) -> Vec<Finding> {
        let mut out = Vec::new();
        // Frames where some tracked thing moves: things appearing or leaving
        // then are scrolled in or out, not popping.
        let moving: Vec<bool> = (0..self.frames)
            .map(|k| self.series.iter().any(|v| step_of(v, k).is_some_and(|d| d > 0)))
            .collect();
        for (name, v) in self.names.iter().zip(&self.series) {
            let excused = |k: usize| rules.inputs_excuse && self.inputs.iter().any(|(f, _)| *f == k);
            check_track(name, v, rules, &excused, &moving, &mut out);
        }
        out.sort_by_key(|f| f.frame);
        out
    }

    /// `Ok` when [`check`](Self::check) finds nothing, else the findings,
    /// one per line, with the inputs.
    pub fn verify(&self, rules: &Rules) -> Result<(), String> {
        let f = self.check(rules);
        if f.is_empty() {
            return Ok(());
        }
        let mut s: Vec<String> = f.iter().map(|x| x.to_string()).collect();
        if !self.inputs.is_empty() {
            let i: Vec<String> = self.inputs.iter().map(|(k, w)| format!("{w}@{k}")).collect();
            s.push(format!("inputs: {}", i.join(", ")));
        }
        Err(s.join("\n"))
    }

    /// Track `name` frame by frame (`k: x,y wxh` or `k: -`), for messages.
    pub fn dump(&self, name: &str) -> String {
        let Some(v) = self.series(name) else { return format!("no track {name}") };
        v.iter()
            .enumerate()
            .map(|(k, s)| match s {
                Some(s) => format!("{k}: {},{} {}x{} a{:.2}", s.rect.0, s.rect.1, s.rect.2, s.rect.3, s.opacity),
                None => format!("{k}: -"),
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn step_of(v: &[Option<Sample>], k: usize) -> Option<i32> {
    if k == 0 || k >= v.len() {
        return None;
    }
    let (a, b) = (v[k - 1]?, v[k]?);
    Some((b.rect.0 - a.rect.0).abs().max((b.rect.1 - a.rect.1).abs()))
}

fn check_track(
    name: &str,
    v: &[Option<Sample>],
    r: &Rules,
    excused: &dyn Fn(usize) -> bool,
    moving: &[bool],
    out: &mut Vec<Finding>,
) {
    let n = v.len();
    // Jumps: a big step between frames that hold still.
    for k in 1..n {
        let Some(step) = step_of(v, k) else { continue };
        if step <= r.max_jump || excused(k) {
            continue;
        }
        // A neighbor frame where it is not shown (moved out of view) says
        // nothing: no verdict. The recording's edges count as still.
        let around = |j: usize| if j == 0 || j >= n { Some(0) } else { step_of(v, j) };
        let (Some(prev), Some(next)) = (around(k - 1), around(k + 1)) else { continue };
        if prev * 4 < step && next * 4 < step {
            let (a, b) = (v[k - 1].unwrap().rect, v[k].unwrap().rect);
            out.push(Finding {
                kind: FindingKind::Jump,
                track: name.to_string(),
                frame: k,
                detail: format!("moved from {},{} to {},{} in one frame", a.0, a.1, b.0, b.1),
            });
        }
    }
    // Pops: shown/gone flicker, opacity dips that come back.
    let h = r.pop_frames.max(1);
    for k in 1..n {
        let (was, is) = (v[k - 1].is_some(), v[k].is_some());
        if was == is {
            continue;
        }
        let back = (k + 1..n.min(k + h + 1))
            .find(|&j| v[j].is_some() == was)
            .filter(|&j| !moving.get(k).copied().unwrap_or(false) && !moving.get(j).copied().unwrap_or(false));
        if let Some(j) = back {
            out.push(Finding {
                kind: FindingKind::Pop,
                track: name.to_string(),
                frame: k,
                detail: if was {
                    format!("gone for {} frame(s), then back", j - k)
                } else {
                    format!("shown for {} frame(s), then gone", j - k)
                },
            });
        }
    }
    for k in 1..n {
        let (Some(a), Some(b)) = (v[k - 1], v[k]) else { continue };
        let drop = a.opacity - b.opacity;
        if drop < r.opacity_drop {
            continue;
        }
        let back = (k + 1..n.min(k + h + 1)).find(|&j| v[j].is_some_and(|s| s.opacity >= a.opacity - drop / 2.0));
        if let Some(j) = back {
            out.push(Finding {
                kind: FindingKind::Pop,
                track: name.to_string(),
                frame: k,
                detail: format!("opacity {:.2} → {:.2}, back by frame {j}", a.opacity, b.opacity),
            });
        }
    }
    // Overshoot, per axis: within a run of moving frames, going past the
    // run's end (or behind its start).
    for axis in 0..2 {
        let pos = |k: usize| v[k].map(|s| if axis == 0 { s.rect.0 } else { s.rect.1 });
        let mut k = 1;
        while k < n {
            let moving = |j: usize| matches!((pos(j - 1), pos(j)), (Some(a), Some(b)) if a != b);
            if !moving(k) {
                k += 1;
                continue;
            }
            let a = k;
            while k < n && moving(k) {
                k += 1;
            }
            let (Some(start), Some(end)) = (pos(a - 1), pos(k - 1)) else { continue };
            let dir = (end - start).signum();
            if dir == 0 {
                continue;
            }
            for j in a..k {
                let Some(p) = pos(j) else { continue };
                let past = (p - end) * dir;
                let behind = (start - p) * dir;
                if past > r.overshoot || behind > r.overshoot {
                    out.push(Finding {
                        kind: FindingKind::Overshoot,
                        track: name.to_string(),
                        frame: j,
                        detail: format!(
                            "{} at {p} on a move {start} → {end}",
                            if axis == 0 { "x" } else { "y" }
                        ),
                    });
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ys(v: &[Option<i32>]) -> Vec<Option<Sample>> {
        v.iter().map(|y| y.map(|y| Sample::at((0, y, 10, 10)))).collect()
    }

    fn kinds(v: &[Option<i32>]) -> Vec<FindingKind> {
        let mut out = Vec::new();
        check_track("t", &ys(v), &Rules::default(), &|_| false, &[], &mut out);
        out.into_iter().map(|f| f.kind).collect()
    }

    #[test]
    fn still_and_eased_moves_pass() {
        assert!(kinds(&[Some(0); 6]).is_empty());
        let eased: Vec<Option<i32>> = [0, 0, 6, 20, 40, 60, 74, 80, 80].iter().map(|y| Some(*y)).collect();
        assert!(kinds(&eased).is_empty(), "{:?}", kinds(&eased));
    }

    #[test]
    fn a_teleport_jumps() {
        assert_eq!(kinds(&[Some(0), Some(0), Some(100), Some(100)]), [FindingKind::Jump]);
        // Small steps (a line of streamed text under the limit) do not.
        assert!(kinds(&[Some(0), Some(0), Some(6), Some(6)]).is_empty());
    }

    #[test]
    fn excused_inputs_may_jump() {
        let mut out = Vec::new();
        let r = Rules { inputs_excuse: true, ..Rules::default() };
        check_track("t", &ys(&[Some(0), Some(100), Some(100)]), &r, &|k| k == 1, &[], &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn blinks_pop() {
        assert_eq!(kinds(&[Some(0), None, Some(0), Some(0)]), [FindingKind::Pop]);
        assert_eq!(kinds(&[None, Some(0), None, None]), [FindingKind::Pop]);
        // Leaving for good (scrolled out) is no pop.
        assert!(kinds(&[Some(0), Some(0), None, None, None, None, None]).is_empty());
    }

    #[test]
    fn passing_through_a_scroll_is_no_pop_or_jump() {
        // Shown one frame while everything moves (a fast glide), then out.
        let mut out = Vec::new();
        check_track("t", &ys(&[None, Some(200), Some(80), None]), &Rules::default(), &|_| false, &[false, true, true, true], &mut out);
        assert!(out.is_empty(), "{out:?}");
    }

    #[test]
    fn opacity_dips_pop() {
        let mut v = ys(&[Some(0); 5]);
        v[2] = Some(Sample { rect: (0, 0, 10, 10), opacity: 0.2 });
        let mut out = Vec::new();
        check_track("t", &v, &Rules::default(), &|_| false, &[], &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].kind, FindingKind::Pop);
    }

    #[test]
    fn springs_overshoot() {
        let v: Vec<Option<i32>> = [0, 0, 30, 70, 95, 104, 101, 100, 100].iter().map(|y| Some(*y)).collect();
        assert_eq!(kinds(&v), [FindingKind::Overshoot]);
    }
}
