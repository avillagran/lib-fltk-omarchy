//! Native diff review: a file tree, a toolbar and a `DiffView` (unified or
//! side by side, wrap, context, find, expand context from the "files").
//!
//!   cargo run --example diff                       # built-in sample patch
//!   git diff | cargo run --example diff -- -       # any patch on stdin
//!   cargo run --example diff -- --file x.patch --split --wrap
//!   cargo run --example diff -- --snapshot build/diff.png [--split]
//!   cargo run --release --example diff -- --bench  # 12 000 lines: open, scroll, memory
//!
//! Keys in the diff: arrows, PageUp/PageDown, Home/End, Shift for ranges,
//! n / p (Alt+Down / Alt+Up) next/previous hunk, Mod+Left/Right previous/next
//! file, Enter/Space or Left/Right fold a hunk or file, e / Shift+E show
//! more context, s split, w wrap, c context, Mod+C copy (Mod+Shift+C as a
//! patch), Mod+F find, F3 / Shift+F3 next/previous match, Esc clears it.
use fltk::prelude::*;
use fltk_omarchy::diff::{self, DiffFiles, DiffView, ExpandRequest, Limits};
use fltk_omarchy::layout::{self, FlexExt};
use fltk_omarchy::split::Split;
use fltk_omarchy::widgets::{glyph, Button, Label, TextField, Tone};
use fltk_omarchy::{i18n, tr, App, AppWindow, State};
use std::io::Read;
use std::time::Instant;

const SAMPLE: &str = r#"diff --git a/src/player.rs b/src/player.rs
index 4f2a1c3..9b7e0d2 100644
--- a/src/player.rs
+++ b/src/player.rs
@@ -12,13 +12,15 @@ use crate::media::{Clock, Frame};
 /// Plays one stream on the toolkit's animation clock.
 pub struct Player {
     clock: Clock,
-    frames: Vec<Frame>,
+    frames: VecDeque<Frame>,
     position: Duration,
+    /// Frames decoded ahead of the clock.
+    ahead: usize,
 }

 impl Player {
-    pub fn new(clock: Clock) -> Player {
-        Player { clock, frames: Vec::new(), position: Duration::ZERO }
+    pub fn new(clock: Clock, ahead: usize) -> Player {
+        Player { clock, frames: VecDeque::with_capacity(ahead), position: Duration::ZERO, ahead }
     }

     pub fn position(&self) -> Duration {
@@ -48,9 +50,12 @@ impl Player {
     pub fn tick(&mut self, now: Instant) -> Option<&Frame> {
         let due = self.clock.at(now);
-        while self.frames.len() > 1 && self.frames[1].pts <= due {
-            self.frames.remove(0);
+        while self.frames.len() > 1 && self.frames[1].pts <= due {
+            self.frames.pop_front();
         }
-        self.frames.first()
+        if self.frames.len() < self.ahead {
+            self.request_more();
+        }
+        self.frames.front()
     }
 }
diff --git a/docs/OLD_NOTES.md b/docs/NOTES.md
similarity index 88%
rename from docs/OLD_NOTES.md
rename to docs/NOTES.md
index 1a2b3c4..5d6e7f8 100644
--- a/docs/OLD_NOTES.md
+++ b/docs/NOTES.md
@@ -1,4 +1,4 @@
-# Old notes
+# Notes

 The player keeps a few frames ahead of the clock so a slow decode
 never shows as a stutter.
diff --git a/src/queue.rs b/src/queue.rs
new file mode 100644
index 0000000..e3b0c44
--- /dev/null
+++ b/src/queue.rs
@@ -0,0 +1,6 @@
+//! A bounded frame queue.
+pub struct Queue<T> {
+    items: std::collections::VecDeque<T>,
+    cap: usize,
+}
+
diff --git a/assets/poster.png b/assets/poster.png
index 9f8e7d6..1c2b3a4 100644
Binary files a/assets/poster.png and b/assets/poster.png differ
diff --git a/script/bench b/script/bench
old mode 100644
new mode 100755
diff --git a/src/legacy.rs b/src/legacy.rs
deleted file mode 100644
index 7c6b5a4..0000000
--- a/src/legacy.rs
+++ /dev/null
@@ -1,3 +0,0 @@
-pub fn old_tick() {
-    // replaced by Player::tick
-}
diff --git a/Cargo.toml b/Cargo.toml
index 2222222..3333333 100644
--- a/Cargo.toml
+++ b/Cargo.toml
@@ -9,3 +9,3 @@ edition = "2021"
 [dependencies]
-fltk = "1.4"
+fltk = "1.5"
 serde_json = "1"
\ No newline at end of file
"#;

/// Lines the "files" of the sample would have: line N reads "line N".
fn fake_file_lines(req: &ExpandRequest) -> Vec<String> {
    // Pretend every file ends at line 80.
    (req.first..=req.last.min(80)).map(|n| format!("    // {} line {n}", req.path)).collect()
}

fn arg(name: &str) -> Option<String> {
    std::env::args().skip_while(|a| a != name).nth(1)
}

fn flag(name: &str) -> bool {
    std::env::args().any(|a| a == name)
}

fn main() {
    if flag("--bench") {
        bench();
        return;
    }
    let text = if flag("-") {
        let mut s = String::new();
        std::io::stdin().read_to_string(&mut s).expect("stdin");
        s
    } else if let Some(path) = arg("--file") {
        std::fs::read_to_string(path).expect("patch file")
    } else {
        SAMPLE.to_string()
    };
    let snapshot = arg("--snapshot");
    let app = App::new("dev.omarchy.fltk.diff");
    if snapshot.is_some() {
        fltk_omarchy::anim::set_enabled(false);
    }
    let mut win = AppWindow::new(&tr!("Changed files"), 1100, 640);
    let (view, files, find) = review(&mut win, &text);
    if flag("--split") {
        view.set_split(true);
    }
    if flag("--wrap") {
        view.set_wrap(true);
    }
    win.end();
    if let Some(q) = arg("--find") {
        find.set(q);
    }
    if let Some(path) = snapshot {
        view.panel().focus();
        view.next_hunk(true);
        win.snapshot(&path).expect("snapshot");
        let _ = files;
        return;
    }
    win.show();
    view.panel().focus();
    app.run();
}

/// The review layout: toolbar, then file tree | grip | diff.
fn review(win: &mut AppWindow, text: &str) -> (DiffView, DiffFiles, State<String>) {
    let mut body = win.body();
    body.set_margin(0);
    let view = DiffView::new().on_expand(|req| {
        // An app reads the file (or asks its backend) and answers later.
        let lines = fake_file_lines(&req);
        if let Some(v) = VIEW.with(|v| v.borrow().clone()) {
            v.provide_context(&req, lines);
        }
    });
    VIEW.with(|v| *v.borrow_mut() = Some(view.clone()));
    if let Err(e) = view.set_text(text) {
        eprintln!("{e}");
    }
    let (split, wrap, context) = (State::new(false), State::new(false), State::new(true));
    let find = State::new(String::new());
    let count = State::new(String::new());

    let mut bar = layout::row();
    bar.set_margin(8);
    let (v1, v2) = (view.clone(), view.clone());
    bar.fit(&*Button::icon(glyph::CHEVRON_LEFT, "").tooltip(&tr!("Previous file")).on_click(move || {
        v1.next_file(false);
    }));
    bar.fit(&*Button::icon(glyph::CHEVRON_RIGHT, "").tooltip(&tr!("Next file")).on_click(move || {
        v2.next_file(true);
    }));
    let (a, d) = view.with_patch(|p| p.totals());
    let n = view.with_patch(|p| p.files.len());
    bar.fit(&*Label::new(&format!("{} · +{a} −{d}", i18n::trn("{count} files", n as i64, &[]))).tone(Tone::Dim));
    bar.add(&layout::spacer());
    let s2 = split.clone();
    bar.fit(&*Button::new("").bind_text(&i18n::map(&split, |on| if *on { tr!("Unified") } else { tr!("Split") })).bordered().on_click(move || s2.update(|v| *v = !*v)));
    let w2 = wrap.clone();
    bar.fit(&*Button::new("").bind_text(&i18n::map(&wrap, |on| if *on { tr!("No wrap") } else { tr!("Wrap") })).bordered().on_click(move || w2.update(|v| *v = !*v)));
    let c2 = context.clone();
    bar.fit(&*Button::new("").bind_text(&i18n::map(&context, |on| if *on { tr!("Hide context") } else { tr!("Show context") })).bordered().on_click(move || c2.update(|v| *v = !*v)));
    let (vf, cnt) = (view.clone(), count.clone());
    find.subscribe(move |q| {
        let n = vf.find(q);
        cnt.set(if q.is_empty() { String::new() } else { n.to_string() });
    })
    .forever();
    let field = TextField::new().placeholder(&tr!("Find in diff")).bind(&find);
    let vn = view.clone();
    let field = field.on_submit(move |_| {
        vn.find_next(true);
    });
    let mut field_w = (*field).clone();
    field_w.set_size(220, field_w.h());
    bar.fit(&*field);
    bar.fit(&*Label::new("").bind(&count).tone(Tone::Dim));
    bar.close_fitted();

    let mut row = layout::row();
    row.set_margin(0);
    let files = DiffFiles::new(&view);
    row.add(&*files);
    let _grip = Split::new(&mut row, &*files).range(160, 420).initial(240);
    row.add(&*view);
    row.end();
    let view = view.bind_split(&split).bind_wrap(&wrap).bind_show_context(&context);
    // Mod+F focuses the find field; F3 works in the diff.
    let mut f2 = field_w.clone();
    win.shortcut("Mod+F", move || {
        let _ = f2.take_focus();
    });
    (view, files, find)
}

thread_local! {
    static VIEW: std::cell::RefCell<Option<DiffView>> = const { std::cell::RefCell::new(None) };
}

/// A patch of `files` files with `per_file` lines each, in hunks of 60.
fn synthetic(files: usize, per_file: usize) -> String {
    let mut s = String::new();
    for f in 0..files {
        s.push_str(&format!("diff --git a/src/module_{f}/file_{f}.rs b/src/module_{f}/file_{f}.rs\nindex 1111111..2222222 100644\n--- a/src/module_{f}/file_{f}.rs\n+++ b/src/module_{f}/file_{f}.rs\n"));
        let hunks = per_file / 60;
        for h in 0..hunks {
            let start = 1 + h * 100;
            // 60 lines: 40 context, 10 removed, 10 added.
            s.push_str(&format!("@@ -{start},50 +{start},50 @@ fn function_{h}(value: usize) -> usize {{\n"));
            for i in 0..20 {
                s.push_str(&format!("     let context_{i} = compute(value, {i}); // unchanged line in hunk {h}\n"));
            }
            for i in 0..10 {
                s.push_str(&format!("-    let changed_{i} = old_call(value, {i}, \"removed text\");\n"));
            }
            for i in 0..10 {
                s.push_str(&format!("+    let changed_{i} = new_call(value, {i}, \"added text\");\n"));
            }
            for i in 20..40 {
                s.push_str(&format!("     let context_{i} = compute(value, {i}); // unchanged line in hunk {h}\n"));
            }
        }
    }
    s
}

fn pss_kib() -> u64 {
    // Linux only (absent elsewhere: 0).
    std::fs::read_to_string("/proc/self/smaps_rollup").unwrap_or_default().lines()
        .find(|l| l.starts_with("Pss:"))
        .and_then(|l| l.split_whitespace().nth(1)?.parse().ok())
        .unwrap_or(0)
}

/// 12 000 hunk lines (200 files × 60): parse, open, first frame, scroll
/// frames and memory, unified and split, no-wrap and wrap.
fn bench() {
    let app = App::new("dev.omarchy.fltk.diff-bench");
    fltk_omarchy::anim::set_enabled(false);
    let text = synthetic(200, 60);
    let t = Instant::now();
    let patch = diff::parse_with(&text, Limits::default()).expect("parse");
    let parse_ms = t.elapsed().as_secs_f64() * 1e3;
    println!("patch: {} files, {} lines, {} KiB; parse {:.2} ms; model heap {} KiB", patch.files.len(), patch.line_count(), text.len() / 1024, parse_ms, patch.heap_bytes() / 1024);
    let pss0 = pss_kib();
    let mut win = AppWindow::new("bench", 1100, 700);
    let mut body = win.body();
    body.set_margin(0);
    let mut view = DiffView::new();
    body.add(&*view);
    win.end();
    win.show();
    for _ in 0..20 {
        fltk::app::wait_for(0.01).ok();
    }
    let pss_win = pss_kib();
    let t = Instant::now();
    view.set_patch(patch);
    let open_ms = t.elapsed().as_secs_f64() * 1e3;
    let t = Instant::now();
    view.redraw();
    fltk::app::flush();
    let first_ms = t.elapsed().as_secs_f64() * 1e3;
    println!("open (rows) {open_ms:.2} ms; first frame (heights + draw) {first_ms:.2} ms; rows {}", view.row_count());
    for (split, wrap) in [(false, false), (true, false), (false, true), (true, true)] {
        view.set_split(split);
        view.set_wrap(wrap);
        view.set_scroll_offset(0);
        view.redraw();
        let t = Instant::now();
        fltk::app::flush();
        let relayout = t.elapsed().as_secs_f64() * 1e3;
        let total = view.content_height();
        let step = 37i64;
        let mut frames = Vec::new();
        let mut y = 0;
        while y < total.min(60_000) {
            view.set_scroll_offset(y);
            let t = Instant::now();
            fltk::app::flush();
            frames.push(t.elapsed().as_secs_f64() * 1e3);
            y += step;
        }
        frames.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let avg = frames.iter().sum::<f64>() / frames.len() as f64;
        let p95 = frames[(frames.len() as f64 * 0.95) as usize];
        println!(
            "{:7} {:7}: relayout {relayout:.2} ms, {} scroll frames: avg {avg:.2} ms, p95 {p95:.2} ms, max {:.2} ms, rows drawn per frame {}",
            if split { "split" } else { "unified" },
            if wrap { "wrap" } else { "no-wrap" },
            frames.len(),
            frames.last().unwrap(),
            view.rows_drawn()
        );
    }
    let t = Instant::now();
    let n = view.find("changed_3");
    let find_ms = t.elapsed().as_secs_f64() * 1e3;
    let t = Instant::now();
    for _ in 0..50 {
        view.find_next(true);
        fltk::app::flush();
    }
    println!("find: {n} matches in {find_ms:.2} ms; find_next + frame avg {:.2} ms", t.elapsed().as_secs_f64() * 1e3 / 50.0);
    let pss1 = pss_kib();
    println!("PSS: window {} KiB, with the 12 000-line diff {} KiB (+{} KiB for the diff)", pss_win, pss1, pss1.saturating_sub(pss_win));
    let _ = pss0;
    drop(app);
}
