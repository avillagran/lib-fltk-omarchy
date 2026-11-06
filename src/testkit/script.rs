//! Scripted input: JSON steps through the same event path as real input.

use super::tree::{pick, tree, Element};
use crate::anim::clock;
use crate::focus;
use fltk::prelude::*;
use serde_json::{json, Value};
use std::path::Path;
use std::time::{Duration, Instant};

/// One frame of a `wait_ms` step on the frozen clock.
const FRAME_MS: u64 = 16;

/// The outcome of one step.
#[derive(Debug, Clone, PartialEq)]
pub struct StepResult {
    /// Position in the script (0-based).
    pub index: usize,
    /// The step's operation (`press`, `click`, `expect`…).
    pub op: String,
    pub ok: bool,
    /// Why it failed (empty when ok).
    pub message: String,
}

/// What [`run_script`] did: one result per step.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Report {
    pub steps: Vec<StepResult>,
}

impl Report {
    /// Every step succeeded.
    pub fn ok(&self) -> bool {
        self.steps.iter().all(|s| s.ok)
    }

    pub fn failures(&self) -> Vec<&StepResult> {
        self.steps.iter().filter(|s| !s.ok).collect()
    }

    /// `{"ok", "passed", "failed", "steps": [{"index", "op", "ok", "error"?}]}`.
    pub fn to_json(&self) -> String {
        let steps: Vec<Value> = self
            .steps
            .iter()
            .map(|s| {
                let mut v = json!({ "index": s.index, "op": s.op, "ok": s.ok });
                if !s.ok {
                    v["error"] = json!(s.message);
                }
                v
            })
            .collect();
        let failed = self.steps.iter().filter(|s| !s.ok).count();
        let v = json!({
            "ok": failed == 0,
            "passed": self.steps.len() - failed,
            "failed": failed,
            "steps": steps,
        });
        serde_json::to_string_pretty(&v).unwrap_or_default()
    }

    /// One line per failed step (or "ok"), for test messages.
    pub fn summary(&self) -> String {
        let f = self.failures();
        if f.is_empty() {
            return format!("ok ({} steps)", self.steps.len());
        }
        f.iter().map(|s| format!("step {} ({}): {}", s.index, s.op, s.message)).collect::<Vec<_>>().join("; ")
    }
}

/// Runs a JSON script against `window` and reports each step. The script
/// is an array of steps, or `{"steps": [..]}`. A step is one object:
///
/// | step | does |
/// |---|---|
/// | `{"press": "Mod+K"}` | a key press (`"repeat": n` presses it n times) |
/// | `{"type": "héllo"}` | committed text, as an input method sends it |
/// | `{"click": target}` | left click at the target's center (`"right": true`) |
/// | `{"hover": target}` | pointer moves there |
/// | `{"wheel": target, "dy": 3}` | wheel notches (positive: down; no target: window center) |
/// | `{"drag": target, "to": target}` | press, drag, release |
/// | `{"focus": target}` | keyboard focus to that widget |
/// | `{"wait_ms": 200}` | the frozen animation clock moves on, one 16 ms frame at a time |
/// | `{"sleep_ms": 50}` | real time passes, events and posted work run (background tasks) |
/// | `{"expect": target, "visible": true, "text": "..", "value": "..", "focused": true, "selected": true, "disabled": false, "count": 2}` | checks |
/// | `{"snapshot": "shot.png"}` | offscreen PNG of the window |
/// | `{"tree": "tree.json"}` | [`tree_json`](super::tree_json) to a file |
///
/// A target is a name (`"Save"`), or an object with any of `name`
/// (exact), `contains` (part of the name), `kind` (`"Button"`), `path`
/// (from [`tree`]), `index` (n-th match, 0-based) or `xy: [x, y]`
/// (window coordinates). Visible elements are preferred. `text` checks
/// that the name or value contains it; `value` compares exactly;
/// `visible: false` also passes when nothing matches.
///
/// The animation clock is frozen for the run (it stays frozen if it was).
/// Steps keep running after a failure; [`Report::ok`] tells the result.
pub fn run_script<W: WindowExt>(window: &W, script: &str) -> Report {
    match serde_json::from_str::<Value>(script) {
        Ok(v) => run_steps(window, &steps_of(&v)),
        Err(e) => Report {
            steps: vec![StepResult { index: 0, op: "parse".into(), ok: false, message: e.to_string() }],
        },
    }
}

pub(crate) fn steps_of(v: &Value) -> Vec<Value> {
    match v {
        Value::Array(a) => a.clone(),
        Value::Object(o) => o.get("steps").and_then(|s| s.as_array()).cloned().unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// [`run_script`] with parsed steps.
pub fn run_steps<W: WindowExt>(window: &W, steps: &[Value]) -> Report {
    let was_frozen = clock::frozen();
    clock::freeze();
    // Lay the window out (a headless window was never drawn).
    super::render_frame(window);
    let mut report = Report::default();
    for (index, step) in steps.iter().enumerate() {
        let op = op_of(step);
        let r = run_step(window, &op, step);
        pump();
        report.steps.push(StepResult { index, op, ok: r.is_ok(), message: r.err().unwrap_or_default() });
    }
    if !was_frozen {
        clock::resume();
    }
    report
}

const OPS: &[&str] = &[
    "press", "type", "click", "hover", "wheel", "drag", "focus", "wait_ms", "sleep_ms", "expect", "snapshot", "tree",
];

fn op_of(step: &Value) -> String {
    let Some(o) = step.as_object() else { return "?".into() };
    OPS.iter().find(|k| o.contains_key(**k)).map_or_else(|| o.keys().next().cloned().unwrap_or_else(|| "?".into()), |k| k.to_string())
}

pub(crate) fn pump() {
    crate::App::pump();
}

/// Keys go to the focused widget; give the window the focus when it has
/// none inside (a fresh headless window), so its shortcuts see the key.
fn focus_window<W: WindowExt>(window: &W) {
    let inside = fltk::app::focus().is_some_and(|f| {
        f.top_window().is_some_and(|t| t.as_widget_ptr() == window.as_widget_ptr())
            || f.as_widget_ptr() == window.as_widget_ptr()
    });
    if !inside {
        fltk::app::set_focus(window);
    }
}

fn run_step<W: WindowExt>(window: &W, op: &str, s: &Value) -> Result<(), String> {
    match op {
        "press" => {
            let spec = s["press"].as_str().ok_or("press needs a key string")?;
            crate::keys::parse(spec)?;
            let n = s.get("repeat").and_then(Value::as_u64).unwrap_or(1).max(1);
            for _ in 0..n {
                focus_window(window);
                focus::press(spec);
                pump();
            }
            Ok(())
        }
        "type" => {
            let text = s["type"].as_str().ok_or("type needs a string")?;
            focus_window(window);
            focus::type_text(text);
            Ok(())
        }
        "click" => {
            let (x, y) = point(window, &s["click"])?;
            if s.get("right").and_then(Value::as_bool).unwrap_or(false) {
                focus::right_click_at(window, x, y);
            } else {
                focus::click_at(window, x, y);
            }
            Ok(())
        }
        "hover" => {
            let (x, y) = point(window, &s["hover"])?;
            focus::hover_at(window, x, y);
            Ok(())
        }
        "wheel" => {
            let t = &s["wheel"];
            let (x, y) = if t.is_null() || t.as_i64().is_some() || t.as_object().is_some_and(|o| o.len() == 1 && o.contains_key("dy")) {
                (window.w() / 2, window.h() / 2)
            } else {
                point(window, t)?
            };
            let dy = s
                .get("dy")
                .or_else(|| t.get("dy"))
                .and_then(Value::as_i64)
                .or_else(|| t.as_i64())
                .unwrap_or(1) as i32;
            focus::wheel_at(window, x, y, dy);
            Ok(())
        }
        "drag" => {
            let from = point(window, &s["drag"])?;
            let to = point(window, s.get("to").ok_or("drag needs \"to\"")?)?;
            focus::mouse_drag(window, from, to, true);
            Ok(())
        }
        "focus" => {
            let e = target(window, &s["focus"])?;
            let mut w = widget_at_path(window, &e.path).ok_or_else(|| format!("no widget at {}", e.path))?;
            if e.key.is_some() {
                // A virtual row: focus its widget, then pick the row by a click.
                let _ = w.take_focus();
                let (x, y) = e.center();
                focus::click_at(window, x, y);
            } else if w.take_focus().is_err() {
                fltk::app::set_focus(&w);
            }
            Ok(())
        }
        "wait_ms" => {
            let ms = s["wait_ms"].as_u64().ok_or("wait_ms needs a number")?;
            wait_frames(window, ms);
            Ok(())
        }
        "sleep_ms" => {
            let ms = s["sleep_ms"].as_u64().ok_or("sleep_ms needs a number")?;
            let end = Instant::now() + Duration::from_millis(ms);
            while Instant::now() < end {
                let left = end.saturating_duration_since(Instant::now());
                let _ = fltk::app::wait_for(left.as_secs_f64().min(0.01));
                pump();
            }
            Ok(())
        }
        "expect" => expect(window, s),
        "snapshot" => {
            let p = s["snapshot"].as_str().ok_or("snapshot needs a path")?;
            let mut w = dw(window);
            crate::window::snapshot(&mut w, Path::new(p)).map_err(|e| format!("{p}: {e}"))
        }
        "tree" => {
            let p = s["tree"].as_str().ok_or("tree needs a path")?;
            std::fs::write(p, super::tree_json(window)).map_err(|e| format!("{p}: {e}"))
        }
        other => Err(format!("unknown step {other:?} (use one of {})", OPS.join(", "))),
    }
}

/// Moves the frozen clock `ms` on, rendering each frame offscreen so
/// paint-driven motion advances as on screen.
pub(crate) fn wait_frames<W: WindowExt>(window: &W, ms: u64) {
    let mut left = ms;
    while left > 0 {
        let d = left.min(FRAME_MS);
        clock::advance(Duration::from_millis(d));
        pump();
        super::render_frame(window);
        left -= d;
    }
}

fn dw<W: WindowExt>(window: &W) -> fltk::window::DoubleWindow {
    // SAFETY: a live window.
    unsafe { fltk::window::DoubleWindow::from_widget_ptr(window.as_widget_ptr()) }
}

/// The widget at a [`tree`] path (virtual row keys dropped).
fn widget_at_path<W: WindowExt>(window: &W, path: &str) -> Option<fltk::widget::Widget> {
    let path = path.split('#').next().unwrap_or("");
    // SAFETY: a live window.
    let mut w = unsafe { fltk::widget::Widget::from_widget_ptr(window.as_widget_ptr()) };
    for part in path.split('/').filter(|p| !p.is_empty()) {
        let i: i32 = part.parse().ok()?;
        w = w.as_group()?.child(i)?;
    }
    Some(w)
}

type Matcher = Box<dyn Fn(&Element) -> bool>;

fn matcher(t: &Value) -> Result<Matcher, String> {
    if let Some(name) = t.as_str() {
        let name = name.to_string();
        return Ok(Box::new(move |e: &Element| e.name == name));
    }
    let o = t.as_object().ok_or("a target is a name or an object")?;
    let name = o.get("name").and_then(Value::as_str).map(str::to_string);
    let contains = o.get("contains").and_then(Value::as_str).map(str::to_string);
    let kind = o.get("kind").and_then(Value::as_str).map(str::to_string);
    let path = o.get("path").and_then(Value::as_str).map(str::to_string);
    if name.is_none() && contains.is_none() && kind.is_none() && path.is_none() {
        return Err("target needs name, contains, kind, path or xy".into());
    }
    Ok(Box::new(move |e: &Element| {
        name.as_ref().map_or(true, |n| &e.name == n)
            && contains.as_ref().map_or(true, |c| e.name.contains(c.as_str()))
            && kind.as_ref().map_or(true, |k| &e.kind == k)
            && path.as_ref().map_or(true, |p| &e.path == p)
    }))
}

fn describe_target(t: &Value) -> String {
    t.as_str().map_or_else(|| t.to_string(), |s| format!("{s:?}"))
}

fn matches<W: WindowExt>(window: &W, t: &Value) -> Result<Vec<Element>, String> {
    let m = matcher(t)?;
    let all = tree(window);
    let mut hits: Vec<Element> = all.iter().filter(|e| e.visible && m(e)).cloned().collect();
    hits.extend(all.iter().filter(|e| !e.visible && m(e)).cloned());
    Ok(hits)
}

fn target<W: WindowExt>(window: &W, t: &Value) -> Result<Element, String> {
    let index = t.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
    let hits = matches(window, t)?;
    if index == 0 {
        let m = matcher(t)?;
        return pick(&hits, m).ok_or_else(|| format!("no element {}", describe_target(t)));
    }
    hits.into_iter().nth(index).ok_or_else(|| format!("no element {} at index {index}", describe_target(t)))
}

/// Window point of a target: `xy`, or the center of the element's part
/// inside the window.
fn point<W: WindowExt>(window: &W, t: &Value) -> Result<(i32, i32), String> {
    if let Some(xy) = t.get("xy").and_then(Value::as_array) {
        let x = xy.first().and_then(Value::as_i64).ok_or("xy needs [x, y]")?;
        let y = xy.get(1).and_then(Value::as_i64).ok_or("xy needs [x, y]")?;
        return Ok((x as i32, y as i32));
    }
    let e = target(window, t)?;
    if !e.visible {
        return Err(format!("{} is not visible", describe_target(t)));
    }
    let (x, y, w, h) = e.rect;
    let x0 = x.max(0);
    let y0 = y.max(0);
    let x1 = (x + w).min(window.w());
    let y1 = (y + h).min(window.h());
    Ok(((x0 + x1) / 2, (y0 + y1) / 2))
}

fn expect<W: WindowExt>(window: &W, s: &Value) -> Result<(), String> {
    let t = &s["expect"];
    let want_visible = s.get("visible").or_else(|| t.get("visible")).and_then(Value::as_bool);
    let field = |k: &str| s.get(k).or_else(|| t.get(k));
    let hits = matches(window, t)?;
    if let Some(n) = field("count").and_then(Value::as_u64) {
        let seen = hits.iter().filter(|e| e.visible).count();
        if seen as u64 != n {
            return Err(format!("{} visible matches of {}, expected {n}", seen, describe_target(t)));
        }
    }
    let e = match target(window, t) {
        Ok(e) => e,
        Err(_) if want_visible == Some(false) => return Ok(()),
        Err(e) => return Err(e),
    };
    let mut errs = Vec::new();
    if let Some(v) = want_visible {
        if e.visible != v {
            errs.push(format!("visible is {}", e.visible));
        }
    }
    if let Some(text) = field("text").and_then(Value::as_str) {
        let has = e.name.contains(text) || e.value.as_deref().is_some_and(|v| v.contains(text));
        if !has {
            errs.push(format!("text {text:?} not in name {:?} / value {:?}", e.name, e.value));
        }
    }
    if let Some(value) = field("value").and_then(Value::as_str) {
        if e.value.as_deref() != Some(value) {
            errs.push(format!("value is {:?}, expected {value:?}", e.value));
        }
    }
    for (k, got) in [("focused", e.focused), ("selected", e.selected), ("disabled", e.disabled)] {
        if let Some(want) = field(k).and_then(Value::as_bool) {
            if got != want {
                errs.push(format!("{k} is {got}"));
            }
        }
    }
    if errs.is_empty() {
        Ok(())
    } else {
        Err(format!("{} ({}): {}", describe_target(t), e.path, errs.join(", ")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ops_and_steps() {
        assert_eq!(op_of(&json!({"press": "Tab", "repeat": 2})), "press");
        assert_eq!(op_of(&json!({"expect": "Save", "visible": true})), "expect");
        assert_eq!(op_of(&json!({"wheel": {"name": "List"}, "dy": 2})), "wheel");
        assert_eq!(steps_of(&json!({"steps": [{"press": "a"}]})).len(), 1);
        assert_eq!(steps_of(&json!([{"press": "a"}, {"type": "b"}])).len(), 2);
    }

    #[test]
    fn targets_match_by_fields() {
        let e = Element { name: "Save file".into(), kind: "Button".into(), path: "0/1".into(), ..Element::default() };
        assert!(matcher(&json!("Save file")).unwrap()(&e));
        assert!(matcher(&json!({"contains": "Save", "kind": "Button"})).unwrap()(&e));
        assert!(!matcher(&json!({"name": "Save", "kind": "Button"})).unwrap()(&e));
        assert!(matcher(&json!({"path": "0/1"})).unwrap()(&e));
        assert!(matcher(&json!({"index": 1})).is_err());
    }

    #[test]
    fn report_json() {
        let r = Report {
            steps: vec![
                StepResult { index: 0, op: "press".into(), ok: true, message: String::new() },
                StepResult { index: 1, op: "expect".into(), ok: false, message: "no element".into() },
            ],
        };
        assert!(!r.ok());
        let v: Value = serde_json::from_str(&r.to_json()).unwrap();
        assert_eq!(v["failed"], 1);
        assert_eq!(v["steps"][1]["error"], "no element");
        assert!(r.summary().contains("step 1 (expect)"));
    }
}
