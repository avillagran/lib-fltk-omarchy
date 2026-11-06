//! UI checks of `diff::DiffView` and `diff::DiffFiles`: tints and line
//! numbers in pixels, keyboard navigation (hunks, files, folding,
//! horizontal scroll), split and wrap, selection and copy, find, the
//! expand-context hook, the file tree, right to left and a 12 000-line
//! patch.
use super::{check, pixel, pump_until};
use fltk::prelude::*;
use fltk_omarchy::diff::{DiffFiles, DiffView, ExpandRequest};
use fltk_omarchy::focus::{self, press};
use fltk_omarchy::layout;
use fltk_omarchy::theme::{self, Rgb};
use fltk_omarchy::{i18n, AppWindow};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Instant;

type Check<'a> = &'a dyn Fn(&Path) -> Result<(), String>;

/// Two files: a modified one with two hunks (function context, a word
/// change, a pure insertion) and a rename with a binary sibling.
pub const PATCH: &str = "diff --git a/src/player.rs b/src/player.rs
index 4f2a1c3..9b7e0d2 100644
--- a/src/player.rs
+++ b/src/player.rs
@@ -12,6 +12,7 @@ use crate::media::Clock;
 pub struct Player {
     clock: Clock,
-    frames: Vec<Frame>,
+    frames: VecDeque<Frame>,
     position: Duration,
+    ahead: usize,
 }

@@ -48,4 +49,4 @@ impl Player {
     pub fn tick(&mut self) {
-        self.frames.remove(0);
+        self.frames.pop_front();
     }
 }
diff --git a/docs/OLD.md b/docs/NEW.md
similarity index 90%
rename from docs/OLD.md
rename to docs/NEW.md
--- a/docs/OLD.md
+++ b/docs/NEW.md
@@ -1,2 +1,2 @@
-# Old
+# New
 text
diff --git a/logo.png b/logo.png
Binary files a/logo.png and b/logo.png differ
";

struct Fixture {
    win: AppWindow,
    view: DiffView,
    files: Option<DiffFiles>,
}

fn fixture(w: i32, h: i32, with_files: bool, patch: &str) -> Fixture {
    let mut win = AppWindow::new("Diff", w, h);
    let mut body = win.body();
    body.set_margin(0);
    let mut row = fltk::group::Flex::default().row();
    body.add(&row);
    row.begin();
    row.set_margin(0);
    row.set_pad(0);
    let view = DiffView::new();
    let files = with_files.then(|| {
        let f = DiffFiles::new(&view);
        row.add(&*f);
        row.fixed(&*f, 200);
        f
    });
    row.add(&*view);
    row.end();
    body.layout();
    win.end();
    view.set_text(patch).expect("patch parses");
    win.show();
    let v = view.clone();
    pump_until(|| v.w() > 100 && v.visible_r());
    Fixture { win, view, files }
}

fn close(f: Fixture) {
    fltk::window::DoubleWindow::delete((*f.win).clone());
}

fn shot(f: &Fixture, dir: &Path, name: &str) -> Result<PathBuf, String> {
    let p = dir.join(name);
    f.win.clone().snapshot(&p).map_err(|e| e.to_string())?;
    Ok(p)
}

fn rgb(c: Rgb) -> [u8; 3] {
    [c.r(), c.g(), c.b()]
}

/// Largest channel difference.
fn dist(a: [u8; 3], b: [u8; 3]) -> i32 {
    a.iter().zip(b.iter()).map(|(x, y)| (*x as i32 - *y as i32).abs()).max().unwrap_or(0)
}

fn near(a: [u8; 3], b: [u8; 3], tol: i32) -> bool {
    a.iter().zip(b.iter()).all(|(x, y)| (*x as i32 - *y as i32).abs() <= tol)
}

/// Pixel near the right end of a row (past any text).
fn row_px(p: &Path, f: &Fixture, row: usize, x_from_right: i32) -> Result<[u8; 3], String> {
    let (x, y, w, h) = f.view.row_rect(row).ok_or("row has no rect")?;
    pixel(p, x + w - x_from_right, y + h / 2)
}

/// A patch with `files` files of `per_file` lines (hunks of 60).
pub fn synthetic(files: usize, per_file: usize) -> String {
    let mut s = String::new();
    for f in 0..files {
        s.push_str(&format!("diff --git a/src/f{f}.rs b/src/f{f}.rs\n--- a/src/f{f}.rs\n+++ b/src/f{f}.rs\n"));
        for h in 0..per_file / 60 {
            let start = 1 + h * 100;
            s.push_str(&format!("@@ -{start},50 +{start},50 @@ fn f{h}()\n"));
            for i in 0..20 {
                s.push_str(&format!("     let c{i} = compute({i}); // context\n"));
            }
            for i in 0..10 {
                s.push_str(&format!("-    let x{i} = old({i});\n"));
            }
            for i in 0..10 {
                s.push_str(&format!("+    let x{i} = new({i});\n"));
            }
            for i in 20..40 {
                s.push_str(&format!("     let c{i} = compute({i}); // context\n"));
            }
        }
    }
    s
}

pub fn checks(run: &mut dyn FnMut(&str, Check)) {
    run("diff: unified rows, add/delete tints from the theme, word highlight, virtualized", &|shots| {
        let f = fixture(760, 420, false, PATCH);
        let p = shot(&f, shots, "diff-unified.png")?;
        let (bg, add, del, word) = theme::with(|t| {
            let bg = t.palette.background;
            (rgb(bg), rgb(t.palette.green.over(bg, 0.13)), rgb(t.palette.red.over(bg, 0.13)), rgb(t.palette.green.over(bg, 0.36)))
        });
        // file header, hunk header, then lines: 0 ctx, 1 removed, 2 added.
        let removed = f.view.row_of_line(0, 0, 2).ok_or("no removed row")?;
        let added = f.view.row_of_line(0, 0, 3).ok_or("no added row")?;
        let ctx = f.view.row_of_line(0, 0, 0).ok_or("no context row")?;
        check(near(row_px(&p, &f, removed, 20)?, del, 3), &format!("removed tint {:?} vs {del:?}", row_px(&p, &f, removed, 20)?))?;
        check(near(row_px(&p, &f, added, 20)?, add, 3), &format!("added tint {:?} vs {add:?}", row_px(&p, &f, added, 20)?))?;
        check(near(row_px(&p, &f, ctx, 20)?, bg, 3), "context row has no tint")?;
        // "VecDeque" (columns 12..20) is highlighted on the added line.
        let (code_x, _, _) = f.view.code_columns();
        let (_, y, _, h) = f.view.row_rect(added).unwrap();
        let cw = (f.view.code_columns().2 as f64 / 80.0).max(1.0);
        let _ = cw;
        // Scan the row's height: the highlight's vertical extent follows
        // the font's metrics (Consolas on Windows, Menlo on macOS).
        let mut found = false;
        // Windows GDI blends translucent tints its own way: accept a pixel
        // clearly nearer the word highlight than the line's tint.
        'scan: for yy in (y + 1..y + h - 1).step_by(2) {
            for x in code_x..code_x + 400 {
                let px = pixel(&p, x, yy)?;
                if near(px, word, 6) || (dist(px, word) < 48 && dist(px, word) + 12 < dist(px, add)) {
                    found = true;
                    break 'scan;
                }
            }
        }
        if !found {
            // Report what the row holds (diagnosis on platforms we cannot see).
            let mut seen: Vec<([u8; 3], usize)> = Vec::new();
            for x in code_x..code_x + 400 {
                let px = pixel(&p, x, y + h / 2)?;
                match seen.iter_mut().find(|(c, _)| dist(*c, px) <= 2) {
                    Some(e) => e.1 += 1,
                    None => seen.push((px, 1)),
                }
            }
            seen.sort_by(|a, b| b.1.cmp(&a.1));
            seen.truncate(6);
            return Err(format!("word highlight on the changed word: want {word:?} (tint {add:?}), row y {y} h {h}, code x {code_x}; colors {seen:?}"));
        }
        // Line numbers are drawn in the gutter (dim ink on the tint).
        let (rx, ry, _, rh) = f.view.row_rect(removed).unwrap();
        let mut ink = false;
        for x in rx..code_x - 4 {
            for y in ry + 2..ry + rh - 2 {
                if pixel(&p, x, y)?.iter().map(|c| *c as i32).sum::<i32>() > del.iter().map(|c| *c as i32).sum::<i32>() + 90 {
                    ink = true;
                }
            }
        }
        check(ink, "old line number in the gutter")?;
        close(f);
        // Virtualization: 12 000 lines, only a screenful drawn.
        let f = fixture(760, 420, false, &synthetic(200, 60));
        let t = Instant::now();
        f.win.clone().snapshot(shots.join("diff-big.png")).map_err(|e| e.to_string())?;
        let first = t.elapsed();
        check(f.view.row_count() > 12_000 && f.view.rows_drawn() < 40, &format!("rows {} drawn {}", f.view.row_count(), f.view.rows_drawn()))?;
        f.view.set_scroll_offset(f.view.content_height() / 2);
        f.win.clone().snapshot(shots.join("diff-big-mid.png")).map_err(|e| e.to_string())?;
        check(f.view.rows_drawn() < 40 && first.as_millis() < 3000, &format!("mid drawn {} first {:?}", f.view.rows_drawn(), first))?;
        close(f);
        Ok(())
    });

    run("diff: keys: n/p and Alt+Down hunks, Mod+Right files, Enter and Left/Right fold, Space collapses a file", &|_| {
        let f = fixture(760, 420, false, PATCH);
        let v = &f.view;
        v.panel().focus();
        check(press("n"), "n is used")?;
        check(v.panel().cursor_index() == v.row_of_hunk(0, 0), &format!("n: first hunk, cursor {:?}", v.panel().cursor_index()))?;
        press("Alt+Down");
        check(v.panel().cursor_index() == v.row_of_hunk(0, 1), "Alt+Down: second hunk")?;
        press("p");
        check(v.panel().cursor_index() == v.row_of_hunk(0, 0), "p: back to the first hunk")?;
        let rows = v.row_count();
        press("Enter");
        check(v.is_folded(0, 0) && v.row_count() == rows - 8, &format!("Enter folds: {} → {}", rows, v.row_count()))?;
        press("Right");
        check(!v.is_folded(0, 0) && v.row_count() == rows, "Right unfolds")?;
        press("Left");
        check(v.is_folded(0, 0), "Left folds")?;
        press("Right");
        press("Mod+Right");
        check(v.current_file() == Some(1), &format!("Mod+Right: next file, {:?}", v.current_file()))?;
        press("Space");
        check(v.is_collapsed(1), "Space collapses the file")?;
        press("Space");
        press("Mod+Left");
        check(v.current_file() == Some(0), "Mod+Left: previous file")?;
        // Down walks the rows; PageDown jumps a screen; End reaches the last row.
        press("Down");
        press("End");
        check(v.panel().cursor_index() == Some(v.row_count() - 1), "End")?;
        press("Home");
        check(v.panel().cursor_index() == Some(0), "Home")?;
        close(f);
        Ok(())
    });

    run("diff: split pairs removed/added lines side by side; wrap grows rows; shared horizontal offset", &|shots| {
        let long = format!("diff --git a/a b/a\n--- a/a\n+++ b/a\n@@ -1,2 +1,2 @@\n-short\n+{}\n same\n", "x".repeat(400));
        let f = fixture(760, 300, false, PATCH);
        let v = &f.view;
        let unified = v.row_count();
        v.set_split(true);
        // Each removed line shares a row with the added line after it (3 pairs).
        check(v.is_split() && v.row_count() == unified - 3, &format!("split rows {} vs {}", v.row_count(), unified))?;
        let p = shot(&f, shots, "diff-split.png")?;
        let pair = v.row_of_line(0, 0, 2).ok_or("pair row")?;
        check(v.row_of_line(0, 0, 3) == Some(pair), "removed and added line share a row")?;
        let (lx, rx, cw) = v.code_columns();
        let (_, y, _, h) = v.row_rect(pair).unwrap();
        let (add, del) = theme::with(|t| (rgb(t.palette.green.over(t.palette.background, 0.13)), rgb(t.palette.red.over(t.palette.background, 0.13))));
        check(near(pixel(&p, lx + cw - 10, y + h / 2)?, del, 3) && near(pixel(&p, rx + cw - 10, y + h / 2)?, add, 3), "left red, right green")?;
        close(f);
        let f = fixture(760, 300, false, &long);
        let v = &f.view;
        let h0 = v.content_height();
        v.panel().focus();
        let row = v.row_of_line(0, 0, 1).unwrap();
        v.panel().select_only(v.panel().id_at(row).unwrap());
        press("Right");
        check(v.h_offset() > 0, &format!("Right scrolls code horizontally: {}", v.h_offset()))?;
        press("w");
        check(v.is_wrap() && v.h_offset() == 0 && v.content_height() > h0, &format!("w wraps: {} → {}", h0, v.content_height()))?;
        press("s");
        check(v.is_split(), "s splits")?;
        press("c");
        check(!v.shows_context(), "c hides context")?;
        check(v.row_of_line(0, 0, 2) == v.row_of_hunk(0, 0), "a hidden context line resolves to its hunk")?;
        close(f);
        Ok(())
    });

    run("diff: Shift+Down selects lines, Mod+C copies text or a patch; mouse drag selects", &|_| {
        let f = fixture(760, 420, false, PATCH);
        let v = &f.view;
        v.panel().focus();
        let first = v.row_of_line(0, 0, 1).unwrap();
        v.panel().select_only(v.panel().id_at(first).unwrap());
        press("Shift+Down");
        press("Shift+Down");
        let text = v.selected_text(false);
        check(text == "    clock: Clock,\n    frames: Vec<Frame>,\n    frames: VecDeque<Frame>,\n", &format!("text {text:?}"))?;
        let patch = v.selected_text(true);
        check(patch == "     clock: Clock,\n-    frames: Vec<Frame>,\n+    frames: VecDeque<Frame>,\n", &format!("patch {patch:?}"))?;
        check(press("Mod+c"), "Mod+C is used")?;
        // Drag from the first context line to the added line.
        let (x, y0, _, h) = v.row_rect(v.row_of_line(0, 0, 0).unwrap()).unwrap();
        let (_, y1, _, _) = v.row_rect(v.row_of_line(0, 0, 3).unwrap()).unwrap();
        focus::mouse_drag(&*f.win, (x + 300, y0 + h / 2), (x + 300, y1 + h / 2), true);
        check(v.panel().selection_len() == 4, &format!("drag selects 4 rows: {}", v.panel().selection_len()))?;
        // A folded hunk header copies all of its lines as a patch.
        v.set_folded(0, 1, true);
        v.panel().select_only(v.panel().id_at(v.row_of_hunk(0, 1).unwrap()).unwrap());
        let folded = v.selected_text(true);
        check(folded.starts_with("@@ -48,4 +49,4 @@ impl Player {\n") && folded.contains("+        self.frames.pop_front();\n"), &format!("folded {folded:?}"))?;
        close(f);
        Ok(())
    });

    run("diff: find highlights every match, F3 walks them and unfolds; Esc clears", &|shots| {
        let f = fixture(760, 420, false, PATCH);
        let v = &f.view;
        check(v.find("frames") == 4, &format!("4 matches, got {}", v.find_count()))?;
        check(v.find("Frames") == 0, "smart case: capitals match exactly")?;
        v.find("frames");
        v.set_folded(0, 1, true);
        v.panel().focus();
        let first = v.find_next(true);
        check(first == Some((1, 4)), &format!("first {first:?}"))?;
        press("F3");
        press("F3");
        check(v.panel().cursor_index() == v.row_of_line(0, 1, 1) && !v.is_folded(0, 1), "F3 reaches the folded hunk and opens it")?;
        let p = shot(&f, shots, "diff-find.png")?;
        let current = theme::with(|t| rgb(t.palette.yellow.over(t.palette.background, 0.7)));
        let bgc = theme::with(|t| rgb(t.palette.background));
        let (cx, _, _) = v.code_columns();
        let (_, y, _, rh) = v.row_rect(v.row_of_line(0, 1, 1).unwrap()).unwrap();
        let mut seen = false;
        for yy in (y + 1..y + rh - 1).step_by(2) {
            for x in cx..cx + 300 {
                let px = pixel(&p, x, yy)?;
                seen |= near(px, current, 6) || (dist(px, current) < 48 && dist(px, current) + 12 < dist(px, bgc));
            }
        }
        if !seen {
            let mut cs: Vec<([u8; 3], usize)> = Vec::new();
            for x in cx..cx + 300 {
                let px = pixel(&p, x, y + rh / 2)?;
                match cs.iter_mut().find(|(c, _)| dist(*c, px) <= 2) {
                    Some(e) => e.1 += 1,
                    None => cs.push((px, 1)),
                }
            }
            cs.sort_by(|a, b| b.1.cmp(&a.1));
            cs.truncate(6);
            return Err(format!("current match is highlighted: want {current:?}, row y {y} h {rh}, code x {cx}; colors {cs:?}"));
        }
        press("Shift+F3");
        check(v.panel().cursor_index() == v.row_of_line(0, 0, 3), "Shift+F3 goes back")?;
        press("Escape");
        check(v.find_count() == 0, "Esc clears the find")?;
        close(f);
        Ok(())
    });

    run("diff: expand context: hunk header button and e ask the app; lines arrive with numbers; the end of a file", &|_| {
        let asked: Rc<RefCell<Vec<ExpandRequest>>> = Rc::default();
        let a2 = asked.clone();
        let f = fixture(760, 420, false, PATCH);
        let _ = f.view.clone().on_expand(move |r| a2.borrow_mut().push(r));
        let v = &f.view;
        // Click the expand button in the gutter of hunk 0 (lines 1-11 hidden).
        let (x, y, _, h) = v.row_rect(v.row_of_hunk(0, 0).unwrap()).unwrap();
        focus::click_at(&*f.win, x + 30, y + h / 2);
        let req = asked.borrow().last().cloned().ok_or("no request")?;
        check(req.file == 0 && req.hunk == 0 && !req.below && (req.first, req.last) == (1, 11) && req.path == "src/player.rs", &format!("{req:?}"))?;
        let lines: Vec<String> = (req.first..=req.last).map(|n| format!("// line {n}")).collect();
        let rows = v.row_count();
        check(v.provide_context(&req, lines), "context accepted")?;
        check(v.row_count() == rows + 11, "11 more rows")?;
        let first = v.with_patch(|p| { let l = &p.files[0].hunks[0].lines[0]; (l.old, l.new, p.text(l).to_string()) });
        check(first == (1, 1, "// line 1".into()), &format!("{first:?}"))?;
        check(v.with_patch(|p| p.gap_above(0, 0)).is_none(), "nothing hidden above")?;
        // "e" on the second hunk asks for lines 19..48 (capped at 20 lines: 29..48).
        v.panel().focus();
        v.panel().select_only(v.panel().id_at(v.row_of_hunk(0, 1).unwrap()).unwrap());
        press("e");
        let req = asked.borrow().last().cloned().unwrap();
        check((req.first, req.last, req.hunk) == (29, 48, 1), &format!("{req:?}"))?;
        // Shift+E: below the last hunk; a short answer marks the end of the file.
        press("Shift+e");
        let req = asked.borrow().last().cloned().unwrap();
        check(req.below && req.first == 53, &format!("{req:?}"))?;
        v.provide_context(&req, vec!["// end".into()]);
        let more_rows = v.row_count();
        press("Shift+e");
        check(asked.borrow().len() == 4 && v.row_count() == more_rows, "Shift+E asks again but the More row is gone")?;
        close(f);
        Ok(())
    });

    run("diff: mouse: hunk header click folds, file header click collapses, wheel scrolls", &|_| {
        let f = fixture(760, 200, false, &synthetic(3, 120));
        let v = &f.view;
        let (x, y, _, h) = v.row_rect(v.row_of_hunk(0, 0).unwrap()).unwrap();
        focus::click_at(&*f.win, x + 400, y + h / 2);
        check(v.is_folded(0, 0), "click on a hunk header folds it")?;
        focus::click_at(&*f.win, x + 400, y + h / 2);
        check(!v.is_folded(0, 0), "again unfolds")?;
        let (x, y, _, h) = v.row_rect(0).unwrap();
        focus::click_at(&*f.win, x + 400, y + h / 2);
        check(v.is_collapsed(0), "file header collapses")?;
        focus::click_at(&*f.win, x + 400, y + h / 2);
        let top = v.scroll_offset();
        check(focus::wheel_at(&*f.win, x + 300, y + 100, 3), "wheel used")?;
        for _ in 0..20 {
            fltk::app::wait_for(0.01).ok();
        }
        check(v.scroll_offset() > top, &format!("wheel scrolls: {top} → {}", v.scroll_offset()))?;
        close(f);
        Ok(())
    });

    run("diff: file tree shows folders and counts, drives the view and follows Mod+Right", &|shots| {
        let f = fixture(900, 420, true, PATCH);
        let files = f.files.as_ref().unwrap();
        let v = &f.view;
        let p = shot(&f, shots, "diff-files.png")?;
        let _ = p;
        let tree = files.tree();
        let labels: Vec<String> = (0..tree.visible_len()).filter_map(|i| tree.node(i)).map(|n| format!("{}|{}", n.label, n.detail)).collect();
        check(labels == ["src|", "player.rs|M  +3 −2", "docs|", "NEW.md ← OLD.md|R  +1 −1", "logo.png|M  ·"], &format!("{labels:?}"))?;
        files.panel().focus();
        press("Down");
        press("Down");
        check(files.panel().cursor() == Some(0) && v.current_file() == Some(0), &format!("tree cursor {:?}", files.panel().cursor()))?;
        press("Down");
        press("Down");
        check(v.current_file() == Some(1) && v.row_rect(v.row_of_hunk(1, 0).unwrap() - 1).unwrap().1 == v.y(), "Down in the tree shows file 1 at the top")?;
        press("Enter");
        check(v.panel().has_focus(), "Enter moves into the diff")?;
        press("Mod+Right");
        check(files.panel().cursor() == Some(2), &format!("tree follows: {:?}", files.panel().cursor()))?;
        close(f);
        Ok(())
    });

    run("diff: code stays left to right in a right-to-left language", &|shots| {
        i18n::set_locale("ar");
        let f = fixture(760, 300, false, PATCH);
        let rtl = layout::is_rtl(&*f.view);
        let p = shot(&f, shots, "diff-rtl.png")?;
        // The old line number stays on the left: ink in the left gutter.
        let row = f.view.row_of_line(0, 0, 0).unwrap();
        let (x, y, _, h) = f.view.row_rect(row).unwrap();
        let bg = theme::with(|t| rgb(t.palette.background));
        let mut ink = false;
        for xx in x..x + 60 {
            for yy in y..y + h {
                ink |= !near(pixel(&p, xx, yy)?, bg, 30);
            }
        }
        close(f);
        i18n::set_locale("en");
        check(!rtl && ink, &format!("rtl {rtl}, ink in the left gutter {ink}"))
    });
}

#[cfg(feature = "a11y")]
pub fn a11y_checks(run: &mut dyn FnMut(&str, Check)) {
    run("a11y: diff rows near the viewport read as added/removed lines; hunks and files expand", &|_| {
        let f = fixture(760, 420, false, PATCH);
        f.view.panel().focus();
        let hunk = f.view.row_of_hunk(0, 0).unwrap();
        f.view.panel().select_only(f.view.panel().id_at(hunk).unwrap());
        f.win.clone().snapshot(Path::new("build/test-ui/shots/a11y-diff.png")).ok();
        let snap = fltk_omarchy::a11y::snapshot(&*f.win).ok_or("no tree")?;
        let has = |s: &str| snap.lines().any(|l| l.contains(s));
        let ok = [
            ("list", has("List \"Diff\"")),
            ("removed", has("Removed line 14: ")),
            ("added", has("Added line 14: ")),
            ("context", has("Line 12: pub struct Player {")),
            ("hunk", has("Hunk at line 12: use crate::media::Clock;")),
            ("file", has("src/player.rs: 3 added, 2 removed")),
            ("expanded", snap.lines().any(|l| l.contains("Hunk at line 12") && l.contains("expanded"))),
        ];
        close(f);
        let bad: Vec<&str> = ok.iter().filter(|(_, v)| !v).map(|(n, _)| *n).collect();
        check(bad.is_empty(), &format!("missing {bad:?}:\n{snap}"))
    });
}
