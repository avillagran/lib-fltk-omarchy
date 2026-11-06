# Porting o3-code to lib-fltk-omarchy: what the toolkit still needs

Review of o3-code (Omarchy T3 Code) v0.2.24, `07f0a01`, on 2026-11-05,
against lib-fltk-omarchy `cab5930`. o3-code is the next app after Files.
The direction for the toolkit: high level, GPU capable where it pays off
(GPUI-like scene rendering), and still the lightest option on idle and
streaming loads.

The 2026-11-06 continuation covers the complete plan. The renderer review
now includes [Rune, GPUI and Flutter](RENDERER-RESEARCH.md), with pinned
primary sources, concrete transfer ideas and verification gates.

## 1. Where o3-code is today

| | Today (QML + Rust) | Port target |
|---|---|---|
| UI | Quickshell instance, ~130 QML files, 18 083 lines, ~985 inline JS functions, 5 061 lines of `ui/js` | One Rust process on this toolkit |
| Backend | `omarchy-t3-client watch`, child process, JSON lines over stdin/stdout, ~134 ops | Same Rust code as a library, events over a channel |
| Live transcript | Full normalized transcript re-sent every 75 ms while streaming; QML `JSON.parse` + array rebuild + per-row diff by `rev` | Per-row diffs into `model::ListModel` (`feed()` from the worker) |
| Sidebar | Full snapshot of every thread (231 in real use) on each change, `JSON.stringify` per row to fingerprint | Keyed diffs into `NavTree` |
| Memory, measured 2026-11-05 on NODE (v0.2.24, 2 h uptime, agents streaming) | Quickshell UI 469.5 MB PSS, Rust client 125.7 MB PSS | ≤ 100 MB PSS for UI + backend (o3's own documented budget, `docs/UI_MEMORY.md`, never reached) |
| CPU | Lifetime average ≈ 10 % (UI) + 9 % (client) of a core; 10 s sample while streaming: 30 % + 51 % | Idle 0 %; one streaming chat ≤ 5 % of a core |
| Platforms | Linux/Omarchy only (Hyprland, Quickshell, layer-shell, wl-clipboard) | Linux Wayland/X11, macOS, Windows through `src/platform` |

`examples/o3_shell.rs --bench` builds an o3-shaped window on the toolkit
(sidebar of 231 card rows from a keyed model, 5 000-message transcript,
Composer; a reply streaming at 60 Hz while the sidebar gets a snapshot
every 1.5 s). Measured 2026-11-06 on NODE, Wayland, 1896×1030, release, two
runs: build 37–39 ms, PSS 26.6–26.9 MB, idle 0.0 % CPU, streaming 12.5–14.7 %
CPU, idle again 0.0 %. Streaming then dropped to 7.5 % by moving the
pixels when the growing message is the last one and the view follows the
end (repaint from the changed block down), and to **4.0 %** with
`RichText::stream_pace(Pace::Low)` (streamed frames at 30 fps; pushes in
between join one frame). Same PSS (27–28 MB). The UI process alone, without the backend.

The two biggest costs are not the renderer: they are (1) full snapshots
re-serialized and re-parsed many times per second, and (2) QtWebEngine
helper processes (150–240 MB each) for HTML, Mermaid and the diff
renderer. Most of the efficiency win comes from diffs and from native
replacements for the web-rendered surfaces, before any GPU work.

## 2. Already covered by the toolkit

- Transcript text: `text::RichText` (CommonMark + GFM, streaming O(tail),
  virtualized, selection, code copy button, tables, links, bidi, a11y).
- Chat input: `edit::Composer` (chips, `@`/`$`/`/` triggers, attachments,
  Enter/Shift+Enter, busy/stop, IME, file drops) and `edit::TextArea`.
- Data: `model::ListModel` with `sync_from`, `patch`, `transaction`, thread
  `feed()`; `State<T>` with `batch`.
- Sidebar engine: `navtree::NavTree` (sections, sub-trees, badges, row
  actions, search ranking, drag reorder, file drops, resize, custom
  renderers with variable height, row arrive/leave motion).
- Popups: `PopupMenu` with submenus and checkable items, `PopupCard` with
  arbitrary content, `SearchableDropdown`, `Dropdown`, `ShortcutHelp` (F1),
  `ConfirmDialog`, `Notification`, `OperationToast`, tooltips (Omarchy
  look, `tooltips.json`).
- Controls: Button, ActionButton, ButtonGroup, TextField (incl.
  `password()`), NumberField, Toggle, ToggleSwitch, Slider, Progress,
  TabBar, Label, virtual `List`/`Table`/`Tree`.
- Window: header actions, Normal/Floating/Fullscreen modes, glass, live
  Omarchy theme, native move/resize.
- Cross-cutting: keyboard-first router and `KeyboardPanel`, i18n (o3's
  catalog format, 20 languages, RTL), AccessKit a11y, `task::Task`,
  animations following the system setting, `gfx` vector canvas, portable
  shaders (CPU or GPU).

## 3. Gaps, by priority

P0 blocks the spike (thread list + transcript + composer on the real
protocol). P1 is needed for feature parity. P2 can wait or stay outside.

### P0: the spike

1. **Transcript as one heterogeneous virtual document.** o3 mixes Markdown
   with cards (activity, subagents, plan, changes, errors, secret
   requests, questions, approvals) and media. Extend `RichText` with an
   embedded block (`Block::Embed { key, height, paint | widget }`) instead
   of a second list type, so the existing virtualization (Fenwick heights),
   selection, find and a11y work across messages and cards. Required
   behaviours: rows keyed by message id fed from a `ListModel`; scroll
   restore by (row id, pixel offset, at-end); follow the end until the user
   scrolls away; prepend of older history without moving the view; bottom
   inset for the composer overlay; "block entered/left viewport" callbacks
   for lazy media; no reuse glitches between tall and short rows.
2. **Markdown action hooks.** Code-block header with language and app
   actions (Copy, Run in terminal, Show diagram); per-table actions (Copy
   as Markdown / CSV); custom link schemes routed to the app
   (`t3-thread://`, `file://`); inline code that looks like a path becomes a
   link; link glyph decorations; image references as sized placeholders
   that the app fills later. o3 then sends raw Markdown and drops its
   Rust Markdown → Qt-HTML renderer.
3. **RichText selection and find API.** Selection rectangle (floating
   Quote button), programmatic select of a range, find with highlight of
   every match and next/previous, plain text of the selection.
4. **Composer additions.** Up/Down prompt history, swap of Enter vs
   Mod+Enter in the expanded editor, a file picker hook, a slot for app
   widgets (dictation level ring, model/permission/branch pickers, context
   meter).
5. **Images.** `media::Image`: async decode on a worker (PNG, JPEG, GIF,
   WebP; HEIC/AVIF optional) at a target size, LRU cache in device pixels,
   thumbnail, viewer popup with zoom, copy, GIF animation (o3 shows only the
   first frame today). Decode with Rust crates into RGBA; keep fltk's
   `no-images` so Files' build is unchanged.
6. **Icons.** o3 uses 116 UI SVGs and 1 917 Lucide project icons tinted
   with `currentColor`. Lucide is stroke-only paths, circles, rects and
   lines: a small SVG-subset parser into `gfx::Path` with a per-(icon, size,
   color) raster cache covers it without resvg. Nerd Font glyphs keep
   working.
7. **Single-instance app IPC.** `platform::instance`: a local socket (Unix
   socket, named pipe on Windows) so a second launch or `ot3 call FN ARG`
   reaches the running app; start hidden; show, hide, toggle and raise
   (xdg-activation or Hyprland `focuswindow` on Linux, OS calls elsewhere);
   "is our window active". About 40 functions return strings or JSON.
8. **NavTree rows like o3's.** Variable heights (82 px card, 36 px slim),
   three-line rich rows, hover actions that replace the trailing time,
   tree guides for sub-threads, delayed hover preview card (500 ms open,
   140 ms close, closes on scroll), several sections with drag-resizable
   heights (o3's shelves) or a single list. Mostly renderer work on
   existing pieces; check it against `examples/navtree_o3.rs`.

### P1: parity

9. **Native diff viewer.** Unified and split rows, line numbers,
   add/delete tints, hunk folding, wrap or horizontal scroll, file tree,
   Esc / Ctrl+Left/Right / PageUp/Down; limits 2 MiB, 256 files, 12 000
   lines. Built on `Table`/`KeyboardPanel`. It replaces the web "Pierre"
   renderer in the common case, so no WebEngine process just to show a
   diff.
10. **External frame surface.** `FrameView`: shows frames produced by
    another process and forwards mouse, wheel, keys, resize, zoom and
    copy. Use shared memory (memfd/ftruncate with two buffers and a
    sequence number; file mapping on Windows) instead of PNG/BMP files.
    The input events are toolkit-neutral; o3 translates them to the Qt
    codes its helpers expect. Covers the browser, HTML attachments,
    Mermaid, MCP apps, QML previews and device screens. The Qt WebEngine
    and Qt Quick helpers stay as they are, started on demand.
11. **Video and audio.** Promote Files' software video stream (one system
    ffmpeg process, latest-frame buffer, audio track, paced) into a
    toolkit `media::Video` / `media::Audio` with play/pause, seek, position
    and duration, created only while a player is open.
12. **Splitter and modal sheet.** Resizable split (mouse, keyboard
    ±10 px, double-click reset, RTL aware, min/max). A modal sheet with
    arbitrary content, created lazily and released after it has been
    closed for a while (o3's `T3DeferredPopup`, 30 s).
13. **Reorder in plain lists.** Drag by a grip with a drop indicator and
    keyboard reorder (queue). `NavTree` has it; `List` does not.
14. **Scrolling.** Nested scroll areas that hand the wheel to the parent
    at their edges; wheel coast for angle-only wheels; touchpad deltas
    unchanged.
15. **Animation clock with paced subscribers.** Each animation asks for a
    rate (8, 30, 60 fps or the display rate, plus a saver mode) and the
    fastest request wins; today `anim` runs one fixed 60 fps timer. Small
    region effects on it: text shimmer, per-row aura border clipped to the
    row, spinners.
16. **Layer rendering.** Draw a widget subtree offscreen, then use it as an
    image or shader input (reveal masks, row flights that cross-fade
    snapshots) and real layer opacity instead of colour veils. Also
    snapshot of a sub-widget, not only the whole window.
17. **Keys.** Physical key codes in key events (shortcut recording maps
    XKB codes to physical punctuation and digits), a switch that suspends
    every app shortcut while recording, dynamic shortcut tables keyed by
    command and rebuilt when the context ("when" variables) changes,
    auto-repeat per shortcut, Menu key and Shift+F10 open context menus.
18. **Fonts and colours.** Medium and SemiBold weights and letter spacing
    through fontconfig names (FLTK only knows regular/bold/italic); named
    terminal colours (`color0`–`color15`, blue, green, yellow, magenta,
    orange) on `Palette` for state tones and per-project colours.
19. **Inputs and small views.** Masked input (`HH:MM`, ISO date-time),
    small charts on `gfx` (bars, lines, segmented quota bars, hover
    tooltips), QR codes drawn as rects from a Rust QR crate, persistent
    notice cards stacked top-right with actions.
20. **Math.** A native subset for inline and display LaTeX
    (fractions, scripts, roots, Greek, operators). Anything beyond the
    subset can fall back to code.

### P2: later or outside the toolkit

- **Panel mode under the bar.** It needs wlr-layer-shell, and FLTK 1.4
  gives every window an `xdg_toplevel` before its first commit, so a
  layer surface means patching FLTK. Emulate it on Hyprland first
  (floating, pinned, no border, placed under the bar button, dismiss on
  focus loss); revisit layer-shell only if that is not good enough.
- **Full-window animated background** (o3's starfield) behind normal
  widgets: on the CPU it forces every widget to repaint per frame. It
  becomes affordable only with the GPU scene below; until then use a
  static frame or animate only an area.
- **Frame-presented callback / FPS meter** for diagnostics.
- **Stays outside the app**: the bar widget, its indicator and badges
  (QML inside the Omarchy shell; keep the `status.json` schema and the
  `ot3` CLI contract), notifications and sounds (`notify-send`,
  `pw-play` from Rust), the terminal (external Omarchy terminal), QML
  previews and the browser (Qt helpers through `FrameView`).

## 4. GPU rendering, the efficient way

Decision already on record (2026-10-28): the interface is designed once;
the toolkit decides whether the CPU or the GPU paints it. GPU support now
covers portable `gfx::View` shaders and explicit composed `scene::Surface`
regions, including retained CPU-widget overlays. Full-window widget
rendering and automatic renderer policy remain below; CPU stays the
default for managed scenes. [Current measurements](SCENE.md) distinguish
warm rendering from cold shader compilation.

Proposal, in order:

1. **One painter for the toolkit's own components.** Every Omarchy
   component already draws itself (`paint.rs`, `gfx`). Route that drawing
   through a `Painter` that records a scene: rounded rects with borders
   and shadows, glyph runs, images, paths, clips, layers. The CPU backend
   replays it with FLTK/Cairo and damage regions (current behaviour and
   cost).
2. **GPU backend for the whole window** (feature `gpu`, as now): one GL
   surface per window, instanced quads with SDF corners and shadows, a
   glyph atlas, image textures, buffer-age partial redraw. Redraw only on
   change, and at the display rate only while something animates or
   scrolls. Plain fltk widgets inside a GPU window draw into an offscreen
   image that becomes a texture.
3. **Pick per window, by policy.** CPU by default (lowest memory, 0 %
   idle). GPU when the app asks for it or when the content needs it (large
   scrolling transcript at high refresh rate, background shader, layer
   effects). Same pixels either way, checked by read-back, as the shaders
   are today.

Measured on the CPU backend (2026-11-06, NODE, 1920×1080 @ 60 Hz, scale
1; `richtext_bench --messages 5000 --scroll 20 --hz 60`, release, real
Wayland window on a hidden Hyprland workspace; two runs each):

| Window | Paint per frame | CPU while scrolling at 60 fps | PSS |
|---|---|---|---|
| 1100×734 | 1.2 ms | 7.5 % of a core | 19–21 MB |
| 1896×1030 | 1.7 ms | 10.7 % of a core | 26 MB |

Paint grows about 0.41 ms per megapixel on a 0.9 ms base, so a 4K window
would need about 4.3 ms per frame: ≈ 26 % of a core at 60 Hz and ≈ 52 % at
120 Hz (extrapolated, not measured). At 1080p the CPU backend is cheaper
overall than a GL context (+35–40 MB PSS for the driver), which is why it
stays the default; the GPU scene pays off on 4K and high-refresh displays.

Before any GPU work, scrolling now moves pixels instead of repainting
(`fl_scroll` + the exposed strip, docs/PERFORMANCE.md): 2–3× less CPU at
1896×1030 (Wayland 12.3 % → 5.6–6.0 %, X11 9 % → 2.6–3.4 %), so the cost of
a scroll no longer grows with the window size. That moves the point where a
GPU scene pays off further out.

Gates before the GPU scene becomes a default for o3: on a 4K/120 Hz
display, scrolling a 5 000-message transcript must cost clearly less CPU
on the GPU backend than on the CPU backend; idle stays 0 %; total PSS
(driver included) stays inside the 100 MB budget.

## 5. Work that belongs to o3-code, not the toolkit

- Move `watch()` and its bin-only modules into the library; replace
  `emit()` (stdout) with a channel.
- Send per-row diffs keyed by id/`rev` from `live.rs` and per-thread
  changes from the shell reducer, instead of whole snapshots.
- Send raw Markdown plus action metadata; drop `rendered_markdown`
  (Qt HTML) and `MarkdownStyle.js`.
- Port the view-model logic now in QML/JS to Rust: `T3Backend.qml`
  (drafts with debounced saves, optimistic sends, approvals, questions,
  rewinds, history paging, plan workflow), `T3Panel.qml` (list sync,
  scroll restore, window modes), `SidebarModel.js`, `ProjectModel.js`,
  `PinOrder.js`, and the generated `Native*.js` files (keybindings with
  when-clauses, usage limits, browser import), with their `tools/test-*`
  checks as oracles.
- Map toolkit input events to the Qt codes the helper processes expect.

## 6. Order

1. Finish Files (current work).
2. P0 items 1–8 in the toolkit, each with UI tests and a bench on the
   hidden Hyprland workspace.
3. o3 spike on the real protocol: thread list, transcript, composer.
   Measure against the table in section 1.
4. P1 items, starting with the diff viewer, `FrameView` and media, since
   they remove the heaviest processes.
5. GPU scene (section 4) as a measured spike in parallel with P1, made
   the default only if the gates hold.

## 7. Status (branch `o3-port`)

| Item | State |
|---|---|
| P0 1–3 transcript in `RichText` (keyed messages, `bind_model`, embeds, images, scroll anchors, follow-end, bottom inset, code and table actions, selection and find API) | merged |
| P0 4 Composer: prompt history, expanded keys, attach button, bottom-row slots | merged |
| P0 5 `media::Image` / `ImageViewer` (feature `images`: PNG, JPEG, GIF, WebP) | merged; HEIC/AVIF not decoded |
| P0 6 SVG icons (`icon`, Lucide subset, theme tint, raster cache) | merged |
| P0 7 single instance and app IPC (`instance`) | merged |
| P0 8 NavTree o3 rows: cards, trailing actions, tree guides, hover preview, `ShelfResizer` | merged |
| P1 9 native diff viewer (`diff`: parser, `DiffView`, `DiffFiles`; docs/DIFF.md) | merged |
| P1 12 splitter (`split::Split`) | merged |
| P1 12 lazy modal sheet with delayed content release | pending; generic `PopupCard` does not provide this lifecycle |
| P1 13 reorder in plain lists (`List::reorderable`) | merged |
| P1 14 nested scroll hand-off (`Scroll::passes`) | merged for List, Table, Tree, RichText, NavTree and TextArea |
| P1 17 keys: command tables, record, repeat, suspend | merged |
| P1 18 font weights, letter spacing, named and terminal colors, project colors | merged |
| P1 19 charts (`chart`), QR codes (`qr`, feature), masked input (`TextField::mask`) | merged |
| P1 19 persistent notice cards with actions | pending; existing critical notifications persist but only dismiss |
| P1 15 paced animation clock (`anim::Pace`, `PaceMode`) | merged |
| P1 16 layer rendering (`layer::Layer`, `capture`, `cross_fade`) | merged |
| P1 10 `FrameView`: shared-memory frames (memfd/shm/file mapping, triple buffer, pipe/FIFO/event wake-ups), input events with Qt mapping, resize/zoom/copy, PNG/BMP file mode for gradual migration (`frame`, docs/FRAMES.md) | merged |
| P1 11 video and audio (`media::Video` / `media::Audio`, feature `playback`, system ffmpeg) | merged; Linux verified, macOS/Windows type-checked |
| P1 20 math (`text::math`, `MarkdownOptions::with_math`: Unicode text like o3, no math layout engine) | merged |
| Section 4 shared painter / GPU scene | CPU command list, immutable RGBA resources, bounded damage replay, managed `scene::Surface`, keyed motion and live lava example implemented ([SCENE.md](SCENE.md)); CPU widget overlays now compose over managed GL regions with native-child input routing. Full-window widget rendering, atlases, native isolated layers and policy remain pending |

Verified locally on the merged branch: unit, doc and UI tests on X11
(default, `a11y`, `images`, `qr`), CI clippy, platform rustfmt,
Windows/macOS type checks, MSRV builds (1.80 default, 1.85 `a11y`).

## 8. Lessons from Rune for chat views

Rune (github.com/anomalyco/rune, MIT, by the opencode team) is a GPU Rust UI
framework with its own text, layout and springs; its opencode client was the
reference for its chat pieces. Reviewed 2026-11-06 (read only) as a guide
for o3's chat views. What to adopt, in order:

| # | Idea | Rune source | Effort | Fits CPU/FLTK |
|---|---|---|---|---|
| 1 | **Done** (`MarkdownOptions::with_settle_tail`). Streaming tail settling: draw an open `**`, `` ` ``, `~~`, `[link](` already closed in a display copy of the tail; hold back a trailing run of markers and an unconfirmed `\|` line; keep table column widths while the last row is still arriving | `rune-kit/src/markdown/stream.rs` | S–M | yes |
| 2 | **Done** (`MarkdownOptions::with_model_lists`). A `4. ` line inside a paragraph continues the list (models write lists that way) | `markdown/interrupt.rs` | S | yes |
| 3 | **Done** (`RichText::on_far_from_bottom`, `follow_end` cuts then glides). `on_far_from_bottom(min_px)` for the jump-to-latest button, separate from following; a long jump to the end cuts to two viewports above it, then glides | `runtime/scroll/bottom.rs` | S | yes |
| 4 | **Done** (`set_bottom_inset` keeps the room while following). Bottom slack: when the composer shrinks after a send, keep the freed room under the last block until growth stops (~120 ms) | `bottom.rs` | S–M | yes |
| 5 | **Done, partly** (`RichText::pace_words`: a trailing partial word from a sender waits ≤ 80 ms; big backlogs are not split into extra frames). Stream pacing at word boundaries: backlogs under 512 chars at once, else ~24 ms chunks cut at words; hold back a trailing partial word | `research/18` | S | yes (fewer reflows) |
| 6 | **Done** (`mermaid`, feature `mermaid`: `render`, `MermaidView`, `RichText::native_mermaid`; docs/MERMAID.md). Native Mermaid (flowchart, sequence, state) as polygons/polylines + labels drawn with `gfx` under the code header, instead of a 150–240 MB helper | `crates/rune-mermaid` (layered layout, cubic routing) | L | yes |
| 7 | **Done** (`DiffView::on_comment`, `set_annotations`, `selected_range`; [DIFF.md](DIFF.md#review-line-ranges-comments-annotations)). Diff annotations under lines (painted or hosting a widget, variable heights, view anchored) and line comments (`on_comment(range)`), selection by (file, side, line) that survives fold/expand/split/wrap | `rune-kit/src/diff` | M | yes |
| 8 | **Done** (`Composer::page_keys_to`). A composer that fits passes PageUp/PageDown/Home/End to the transcript | rune README | S | yes |
| 9 | **Done** (`Style::chip`, `MarkdownOptions::with_chip_links`). Chips in read-only text, matching the composer's | `el/pills.rs` | S–M | yes |
| 10 | **Done** (`task::Latest`). `task::Latest<T>`: latest-wins background requests with debounce (`@` search) | `latest.rs` | S | yes |
| 11 | **Done** (`OMARCHY_FLTK_FRAME_LOG`, `testkit::frames`; docs/TESTKIT.md). Frame log (paint / wall / CPU per frame) and a warning when a UI callback blocks | `RUNE_FRAME_LOG` | S | yes |
| 12 | **Done** (`testkit::{tree, run_script, Gate}`, `OMARCHY_FLTK_SCRIPT`; docs/TESTKIT.md). Scripted agent harness (input by accessible name, element dump with rects) and a motion gate (no jumps on prepend/stream/follow) | `--script`, `--tree`, `motion_gate` | M | yes |

Not adopted: spring-driven growth of every row (each animation frame is a
repaint on the CPU backend).

The earlier chat-view comparison recorded: O(log n) height index (Fenwick) vs a
prefix-sum rebuild per height change; streaming keeps only the open tail's
source (1.5 B per Markdown byte) vs rebuilding and hashing the whole message
per update; idle 0 % CPU at 12.6–26.9 MB PSS vs ~34 MB RSS / 92–95 MB
footprint; a 1.78 MB minimal binary vs 9.3 MB; scrolls move pixels instead
of re-emitting every glyph; one-document transcript (selection, find, a11y
across messages); code/table actions, alerts, math in Markdown.

Those measurements belong to that earlier review's Rune snapshot and
workloads. They are not measurements of the current pinned Rune/GPUI/Flutter
revisions in [the renderer review](RENDERER-RESEARCH.md).


## 9. Lava UI reference and next work (2026-11-06)

The supplied `lava.mp4` is a useful composition acceptance case: reorderable
cards above a shader background, a dragged/falling row, translucent layers,
and a short effect on deletion. [SCENE.md](SCENE.md#video-reference-lavamp4-2026-11-06)
separates existing components from the missing rendering path.

Continue section 4 without changing the CPU default:

1. Shared painter: the first CPU slice now records fills, borders, text,
   alpha fills and balanced clips. Prove immediate/replay and damage parity.
2. Complete scene primitives (RGBA images and isolated translated opacity layers now implemented; paths, affine transforms, shadows,
   shader layers) and an invalidation lifecycle for widget caches.
3. One GL scene surface: low-level `scene::gpu::Renderer` now composes
   rectangles, portable shader commands and CPU texture fallbacks into a
   single framebuffer (read-back check added). Managed `scene::Surface` now owns CPU/GL lifecycle, snapshots, input,
   invalidation and paced shader frames. Stable keyed position/opacity updates
   now preserve initially culled/zero-alpha cards without preparation/texture uploads.
   Scoped keyed raising and rectangular clipped hit testing are now included.
   Managed node position/opacity transitions now use the shared clock,
   preserve retargeted poses and stop when settled/hidden/reduced motion.
   The `lava` example now implements live pointer/keyboard reorder,
   root drag previews, falling removal and a bounded GPU shader pulse.
   Its tests include full actual GPU read-back versus CPU replay.
   The CPU-widget overlay bridge now retains textures independently of the
   scene and routes native-child events through the top-level window.
   Consecutive compatible quads now use retained instanced attributes and
   ordered batches; shader-only frames reuse the instance buffer. Zero-area
   regions suspend GL and release resources, recovering on positive size.
   A bounded per-renderer image page now groups different fallback images.
   Next: full-window widget rendering, glyph atlases, residency across scenes and
   native isolated shader layers.
4. A live version of the interaction now runs in `examples/lava.rs`.
   [Measurements](SCENE.md#interactive-lava-example) cover the actual
   controller on Wayland: zero idle frames, retained resources during
   reorder, low GPU submission CPU, but cold-cache GPU PSS still exceeds
   100 MiB. Continue memory work, GPU/power measurements and normal/high-
   refresh resolution gates. Effects are bounded and respect reduced
   motion/visibility. The video remains a visual reference, not a benchmark.
5. Real o3 protocol spike and full-window renderer policy retain the gates
   above; the synthetic shell and static card demo do not close those gates.

The portal file dialogs and Rust application template/generator have also
been integrated into `o3-port`. The preceding `bab1db8` CI passed all six
jobs; that result predates the shared-painter changes.
