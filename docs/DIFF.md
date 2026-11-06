# Diff viewer

`diff` is a native viewer for unified patches: a parser, a virtualized
`DiffView` canvas and a `DiffFiles` tree. It replaces a web renderer (and
its WebEngine process) for the common case of reviewing a patch, the way
o3-code's `T3DiffReview` uses it.

```rust
use fltk_omarchy::diff::{DiffFiles, DiffView};
use fltk_omarchy::{split::Split, layout, State};

let split = State::new(false);
let view = DiffView::new()
    .bind_split(&split)                      // two-way: a toolbar toggle, `s` in the view
    .on_expand(move |req| {                  // "show more lines" (optional)
        let lines = read_new_file_lines(&req.path, req.first, req.last);
        VIEW.with(|v| v.provide_context(&req, lines));
    });
view.set_text(&patch_text)?;                 // or set_patch(diff::parse_with(..)?)
let mut row = layout::row();
let files = DiffFiles::new(&view);
row.add(&*files);
let _grip = Split::new(&mut row, &*files).range(160, 420).initial(240);
row.add(&*view);
row.end();
```

`cargo run --example diff` shows a review window with a toolbar (`git diff
| cargo run --example diff -- -` shows any patch; `-- --snapshot x.png
[--split] [--wrap] [--find text]` writes a PNG; `-- --bench` measures).

## Parser

`diff::parse(text)` reads `git diff`, `git show`, `git format-patch` and
plain `diff -u` output into a `Patch`:

- `Patch::files`: `FileDiff { old_path, new_path, status, binary,
  old_mode, new_mode, similarity, hunks, additions, deletions }`.
  `status` is Modified, Added, Deleted, Renamed or Copied; a deleted file
  has no `new_path`, an added one no `old_path`. `display_path()` writes
  renames as `old → new`.
- `Hunk { old_start, old_count, new_start, new_count, section, lines }`;
  `section` is the function context after the second `@@`.
- `Line { kind, old, new, no_newline }` with 1-based numbers (0 on the
  side a line does not exist); `patch.text(&line)` is its text. Removed and
  added lines that replace each other are paired (`line.pair()`): the i-th
  line of a removed block with the i-th line of the added block after it.

Handled: several files, renames and copies with or without changes, mode
changes, new and deleted (also empty) files, `Binary files … differ` and
`GIT binary patch` blocks (skipped), `\ No newline at end of file`,
C-quoted paths with octal escapes, unquoted paths with spaces, CRLF,
empty context lines whose space an editor stripped, hunk lines that look
like headers (`--- x` inside a hunk follows the counts), mail headers,
commit messages, diffstats and signatures around the diffs. Combined
diffs of merges (`diff --cc`) are an error.

**Limits** (o3-code's, `Limits::default()`): 2 MiB of patch, 256 files,
12 000 hunk lines, 16 KiB per line. Going over is a `DiffError`, never a
silently cut patch; `parse_with(text, Limits::none())` lifts them.
`DiffError` displays a translated message.

Line text lives in one buffer owned by the patch (≈ 24 bytes per line on
top of the text), so the raw patch can be dropped after parsing.

## DiffView

One widget; each frame draws only the rows in view (a 12 000-line patch
draws ≈ 40 rows per frame). Rows: a file header (status badge, path,
`+a −d`), a notice for binary files and mode changes, hunk headers, and
lines (unified) or line pairs (side by side).

| | |
|---|---|
| Layout | `set_split(bool)` / `bind_split(&State<bool>)`: unified (old and new number columns) or side by side (removed lines left, added right, paired rows; an empty side is shaded) |
| Long lines | `set_wrap(bool)` / `bind_wrap`: wrap at the code column, or scroll horizontally with one offset shared by every row and both sides (`h_offset`) |
| Context | `set_show_context(bool)` / `bind_show_context`: hide unchanged lines |
| Tints | added and removed rows are `green` / `red` of the theme over the background (stronger in the gutter); `omarchy-theme-set` restyles them live |
| Word highlights | the changed middle of each paired removed/added line (common prefix and suffix, widened to whole words), computed only for rows in view; `word_diff(false)` turns it off |
| Folding | `set_folded(file, hunk, bool)`, `set_collapsed(file, bool)`, `fold_all`; click a hunk or file header, or Enter / Space / Left / Right on it; a folded hunk says how many lines it hides |
| More context | `on_expand(|req| ..)`: an expand button in the gutter of hunk headers with hidden lines above them, a "Show more lines" row after a file's last hunk, `e` / Shift+E. The app answers with `provide_context(&req, lines)` (new-file text of lines `req.first..=req.last`, at most 20 per request); hunks that touch merge. Fewer lines than asked below the last hunk marks the end of the file |
| Navigation | `next_hunk(forward)`, `next_file(forward)`, `show_file(i)`, `current_file()`, `on_file_change(|i| ..)` |
| Selection | the rows' `KeyboardPanel` (`panel()`): click, Shift+click, drag, Shift+arrows; `selected_text(as_patch)`, `copy(as_patch)` |
| Find | `find(query) -> count` (smart case: lower-case queries ignore case), every match highlighted, `find_next(forward) -> (n, total)` opens folded hunks and scrolls to the match, `clear_find` |
| Font | the theme family in the monospace slot at `font.body_small`; code is always drawn left to right, also in right-to-left languages |

Keys while the diff has focus (rule 10; list them in your F1 help):

| Key | Effect |
|---|---|
| Up / Down, PageUp / PageDown, Home / End | move the cursor row (Shift extends the selection) |
| n / p, Alt+Down / Alt+Up | next / previous hunk |
| Mod+Right / Mod+Left | next / previous file |
| Enter, Space | fold or unfold the hunk, collapse or open the file, show more lines |
| Left / Right | on a header: fold / unfold; on a line: scroll the code horizontally (no-wrap) |
| e / Shift+E | more context above the hunk / below the file's last hunk |
| s, w, c | split ↔ unified, wrap, context |
| Mod+C / Mod+Shift+C | copy the selected lines / as a patch (markers, hunk headers; a folded hunk brings its lines) |
| F3 / Shift+F3, Mod+G / Mod+Shift+G | next / previous find match |
| Escape | clears the find (then goes on to the app: o3 closes the review) |

Mouse: click selects a row (Mod toggles, Shift extends), drag selects a
range (scrolling past the edges), wheel scrolls, Shift+wheel or a
horizontal wheel scrolls the code. Scrolling is eased by the toolkit's
shared animation clock (`scroll::Scroll`) and follows the system's
animation setting; nothing else animates.

### Screen readers

The view is a List named "Diff" (`a11y` feature). The rows near the
viewport (plus the cursor row) are virtual ListItems with their position:
"Added line 14: text", "Removed line 14: text", "Line 12: text", "Hunk at
line 12: section" (expanded or collapsed), "src/player.rs: 3 added, 2
removed". Click, Focus, ScrollIntoView, Expand and Collapse run the same
code as the keys.

## DiffFiles

`DiffFiles::new(&view)` is a toolkit `Tree` of the changed files: folders
first (single-folder chains compressed as `src/diff`), then files with
their status letter and `+a −d`; renames read `new ← old`. Moving its
cursor shows that file at the top of the view; Enter moves the focus into
the diff; the tree's cursor follows the view's current file. It refreshes
on `set_patch` (more context changes no counts). Named "Changed files" for
screen readers.

## Measurements

`cargo run --release --example diff -- --bench` (200 files × 60 lines =
12 000 lines, 804 KiB; Xvfb 1280×800, window 1100×700, release build,
2026-11-06):

| | |
|---|---|
| Parse | 0.95 ms (model heap 1.4 MiB) |
| Open (rows) | 0.54 ms; first frame (row heights + draw) 9.3 ms |
| Scroll, unified no-wrap | avg 1.8 ms per frame, p95 2.4 ms (1 622 frames, 40 rows drawn) |
| Scroll, split no-wrap | avg 3.4 ms, p95 5.5 ms |
| Scroll, unified wrap | avg 1.6 ms, p95 1.8 ms |
| Scroll, split wrap | avg 2.9 ms, p95 3.4 ms |
| Switch layout / wrap (all row heights again) | 1.7–3.2 ms |
| Find (400 matches in 12 000 lines) | 1.0 ms; next match + frame 3.2 ms |
| Memory | +1.2 MiB PSS for the loaded 12 000-line diff (window alone 10.1 MiB) |

## Known gaps

- No syntax highlighting (tints and word highlights only).
- Wrapping counts columns (tabs to 4, East Asian wide characters as 2);
  it expects a monospace theme family (Omarchy's always is).
- No intra-line diff beyond a common prefix and suffix (no LCS over words).
- The scrollbars are indicators (no thumb dragging); scroll with the
  wheel, keys or `set_scroll_offset`.
- Folding is immediate (no row motion); the toolkit's row motion is not
  wired to the diff rows.
- Combined (merge) diffs are rejected; binary contents are not shown.
