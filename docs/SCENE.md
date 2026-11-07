# Window scene: shared painter foundation

The window scene is being built in stages (O3-PORT section 4). The first
stage records the existing shared painter and replays it on the CPU.
It does **not** enable GPU rendering for ordinary widgets.

## Record and replay

```rust,no_run
use fltk_omarchy::{paint, scene::{self, Rect, Scene}, theme};

// Main UI thread; text measurement still uses FLTK's current surface.
let (scene, ()) = Scene::record(|| {
    let t = theme::current();
    scene::clip(Rect::new(20, 30, 300, 100), || {
        paint::surface(20, 30, 300, 100, t.radius,
            Some(t.palette.lighter_background),
            Some((t.palette.accent, 1)));
    });
});
// In a draw callback, on its current surface:
scene.draw();
// Or redraw just the damaged area:
assert!(scene.draw_damage(Rect::new(20, 30, 40, 100)));
```

The list stores rounded fills, inner borders, text (including font weight,
elision, spacing and literal `@`), rectangular source-over alpha fills and
balanced nested rectangular clips. It keeps paint order. `commands()`
exposes a read-only list for backend development. `append` copies another
list; nested recordings are isolated. Recording and clip scopes restore
the previous state when unwinding.

Ordinary `paint` calls remain immediate CPU drawing. They check a
thread-local recorder but allocate no display list or copied strings when
recording is off. A scene creates no animation timer or GL context. Primitive-only full replay creates no
pixel backing store; explicit isolated layers retain their own rasters. Recording allocates commands and copies text; retain
and replay the list when unchanged rather than rebuilding it every frame.
Damage replay lazily retains an opaque RGB image (up to a 64 MiB RGBA-sized
budget); `clear_raster` releases it and `append` invalidates it. Native
rasterizers can vary edge antialiasing and scaled-image sampling when a
clip changes, so redrawing primitives independently per damage tile is
not pixel-exact on all platforms. The retained raster preserves the
original pixels, at an explicit memory cost, until the GPU backend lands.

Scenes contain resolved colors, fonts, translated strings and absolute
logical coordinates. Invalidate them on theme/font/language, scale or
layout changes. `examples/scene_cards.rs` demonstrates this lifecycle,
including live theme and language changes, and has `--snapshot` and
`--bench` modes. It is a static card reference, not the lava effect.

Damage replay copies from that raster within the requested clip and
leaves other pixels alone. The caller must supply a complete opaque
scene, including its background and every intersecting layer. Moving or
removing something requires damage covering its old and new bounds.
`draw_damage` returns false without painting if the combined bounds
exceed the budget or the surface fails. Check its return value. Release
the raster on surface-scale/font environment changes; record a new scene
on theme/layout/language changes.

`scene::Image::rgba(w, h, pixels)` creates a validated immutable RGBA8
resource without enabling an image decoder. `image.draw(rect)` draws it
source-over or records a shared reference; scaled pixels are not copied
into the command list. Zero dimensions, incorrect byte counts, overflow
and resources over 64 MiB are rejected. Decode/downsample through `media`
when needed. Alpha, scaling and clipping participate in parity checks.

## Isolated layers

`scene::Layer` retains a child scene with explicit bounds and composites
its finished result over the actual destination. Group opacity applies
once, so overlapping opaque children do not show through each other.

```rust,no_run
use fltk_omarchy::{paint, scene::{Layer, Rect, Scene}, theme};
let (card, ()) = Scene::record(|| {
    let t = theme::current();
    paint::fill(0, 0, 240, 64, t.radius, t.palette.lighter_background);
    // Record the card's text, borders and images here as well.
});
let card = Layer::new(Rect::new(0, 0, 240, 64), card)?;
// Inside a draw callback, or another Scene::record:
assert!(card.draw_at(40, 80, 0.5));
# Ok::<(), fltk_omarchy::scene::ImageError>(())
```

Drawing at `(x, y)` translates the child's bounds to that position. Layers
can be nested, clipped by the outer scene, and retained by layer commands;
cloning a handle shares its child and caches. Bounds are also the explicit
local crop. This is integer translation, not arbitrary affine transforms.

The CPU fallback renders two opaque native surfaces (black and white) to
recover RGBA, caches the result, and changes only the alpha bytes while
fading. It does not bake a theme background into the card, unlike the
existing `layer::Layer` widget fallback. Rasterization is at logical 1×;
native subpixel text coverage collapses to a single alpha channel. This
is a bounded CPU fallback, not a GPU layer or an efficiency claim.

A resource is limited to 64 MiB of RGBA; cold rendering needs temporary
black/white surfaces and buffers in addition. Keep layers to the affected
row/card, not the full window. Opacity changes reuse an upload buffer;
steady full opacity uses the cached image directly. `clear_raster` on a
layer or scene recursively releases child caches. Recreate the layer when
its theme, language or geometry changes. Transparent layers can be rendered
without an opaque background, but root `draw_damage` still requires one.
`draw_at` returns false on surface/coordinate failures; recorded calls
defer rasterization until replay. Check the return when drawing directly.

## Managed surface

`scene::Surface` hosts the same local-coordinate scene on CPU or GL without
application GL context calls. CPU is the default even with feature `gpu`;
`Backend::Auto` also stays on CPU until the measured window policy is ready.
Opt in with `backend(Backend::Gpu)` for an effect that needs the compositor.

```rust,no_run
use fltk::prelude::*;
use fltk_omarchy::{App, AppWindow, paint, scene::Surface};
let app = App::new("dev.example.scene");
let mut win = AppWindow::new("Scene", 420, 300);
let mut body = win.body();
let surface = Surface::new(380, 240).paint(|r, t| {
    paint::fill(0, 0, r.w, r.h, 0, t.palette.background);
    // scene::shader and isolated layers can be recorded here as well.
});
body.add(&*surface);
win.end();
win.show();
app.run();
```

The painter is retained and re-recorded on size, theme, language, focus-ring
or explicit `invalidate()` changes. It receives a local rectangle and the
current theme. `set_scene` accepts an already recorded immutable scene;
its colors/layout are the caller's responsibility. This is an opaque
surface: it clears its background with the theme background before replay.
Translucent cards blend with earlier commands in that same scene.

`on_event` receives local pointer coordinates and normal FLTK events.
`on_event_with` also passes `&Surface`, so a callback can hit-test and
update its owner without capturing a strong clone of that same owner.
The surface is one keyboard stop; its GL child never owns logical focus.
Native input is delivered once even when an unhandled event traverses both
the group and GL child. Return true for handled actions, false to let app
shortcuts continue. Use `KeyboardPanel` + `focus::host` for virtual region
controls, stable selection and accessible rows. The canvas does not turn
painted labels into interactive widgets or a semantic accessibility tree.
Name it with `AccessibleExt` and bind all drawn text through `tr!`/i18n.

`animate(Some(Pace::Display))` advances top-level shader time through the
shared toolkit clock. GPU frames redraw only the GL child, not the CPU
parent/window background. CPU frames request at most `Pace::Low` (30 fps)
before the global pace policy is applied. Hidden views, reduced motion,
explicitly frozen time and scenes without top-level shaders stop requesting
frames. An event-loop check notices motion-setting changes and resumes a
stopped visible view without a polling timer. `set_time(Some(t))` freezes
phase; `set_time(None)` resumes automatic time when a pace is selected.
`animate(None)` stops automatic pacing and returns to recorded shader time.

`play_shader(pace, duration_ms)` restarts shader time at zero, clamps it to
the duration and stops frame requests at the end. It clears an explicit
time override and uses the same clock, visibility and reduced-motion rules.
The final shader phase should be visually quiet for a short interaction
effect. Node motion and shader pacing still share one frame request.

GL activation waits three short ticks after the parent maps (Wayland's
first-frame restriction). Static views install no repeating timer after
activation. Resize, hide/show and CPU/GPU policy switches prepare/recover
resources as needed. Preparation/renderer errors return to CPU and are
available from `backend_error`; `invalidate` permits a retry. Applications
do not own context creation/destruction. Native draw callbacks own the GL
resources; tracked weak handles let the host update them without extending
their lifetime beyond the native window. Known lost contexts abandon old
names rather than deleting them in a different context.

`AppWindow::snapshot` and widget-subtree `layer::capture` force scene surfaces
through CPU replay, even after GL activation. Attached roots use a childless
CPU draw proxy: FLTK otherwise traverses native subwindows a second time,
which would print the GPU result over the CPU reference and double-composite
overlays. The proxy leaves native visibility/context state unchanged.
Standalone native-window printing retains FLTK's behavior; use the explicit
AppWindow snapshot API for a CPU window reference.
Use `read_gpu_pixel` or `read_gpu_image`
for actual GL output. The latter asynchronously returns a top-to-bottom
RGBA `Image` in physical framebuffer pixels, limited to 64 MiB. Its callback
runs on a later UI turn outside native drawing; deleting the widget drops
queued requests. It returns false before activation or on CPU. A capture
requests one draw/read-back; it is a diagnostic, not a free screenshot API.
Other custom offscreen harnesses should render the local `Scene` directly;
a generic native widget capture is not automatically detectable everywhere.
The widget retains scene content across shader time changes. `SurfaceStats`
separates scene builds, parent surface draws, CPU frames, GPU submissions,
GPU preparations, scene uploads and retained texture bytes. Frame counts
are not presentation callbacks; scene uploads count installed revisions,
including resource recovery after context loss. Actual texture uploads and
program links have separate counters (see residency below).

## CPU widget overlays over a GPU surface

Toolkit overlays above a mapped `Surface` are now composed into its actual
GL framebuffer. This includes help, menus, dialogs, notifications and
`PopupCard` CPU widget content. They keep their existing widgets, focus,
keyboard/accessibility behavior and callbacks. A native-child event is
remapped to the top-level window before FLTK routes it, preserving normal
pressed/hover state and restoring coordinates afterward. Closing layers
keep drawing during their exit and stop receiving input immediately.

The bridge captures transparent premultiplied RGBA using black/white native
passes at one sampled animation time. It crops unused transparent margins.
Capture follows widget damage, layer identity/order, geometry, theme and
motion phase. Identical bytes avoid another upload. One texture is updated
independently of base scene resources, using the existing basic GL program.
Shader-only frames reuse it without CPU capture, parent drawing, program
creation or texture uploads. `overlay_captures`, `overlay_uploads` and
`overlay_texture_bytes` expose that work in `SurfaceStats`; total
`texture_bytes` includes the overlay. Closing the last overlay releases it.

The CPU capture budget is 64 MiB. Scene/overlay textures and the instance
buffer together share a separate 64 MiB GPU resource budget. Capture needs temporary native RGB surfaces and CPU buffers as
well. Modal scrims can make the crop cover the entire viewport; this stage
does not make large full-window bitmap fallbacks cheap. Native overlay
primitives/layers and atlases remain part of the full-window renderer work.
Fades and dirty CPU controls currently recapture their changed bitmap.
The fallback raster is at logical 1×; current full-image parity tests use
scale 1, and the native glyph/HiDPI pipeline still needs its own gates.

Native subwindows inside overlay content are not CPU-widget subtrees. An
incompatible capture or resource limit reports `backend_error` and falls
back to CPU, releasing old GPU resources. Overlay changes automatically
retry a temporary overlay failure, so closing the incompatible content
restores GL. Driver/base-scene errors retain the existing explicit retry
through `invalidate`. This bridge supports overlays on explicitly managed
scene regions; automatic ordinary full-window widget rendering is pending.

The lava diagnostic accepts `script/scene-bench --overlay` (with
`SCENE_BENCH_BIN` pointing to the built lava example). It adds a persistent
non-modal notification without requesting keyboard focus. On NODE's
RTX 3080 Ti, Wayland, 720×639, release, one sequential run per condition:
GPU with overlay used 1.94 % of a core / 72.4 MiB active PSS, the same binary
without the overlay 1.87 % / 70.0 MiB, and CPU with static floor/overlay
5.98 % / 34.2 MiB. Both idle phases had zero frames (≤0.018 % CPU).
The GPU notification used 47,120 texture bytes; reordering under it caused
zero captures, overlay uploads or scene preparations. All runs recorded
zero focus changes and completed 5-to-4 deletion. This small sample does
not establish a precise incremental overlay CPU cost.

The cache-disabled GPU overlay control used 117.5 MiB active PSS and
120.9 MiB after the interaction, so the cold-memory gate remains unmet.
Raw data: [scene-overlays-2026-11-06.json](scene-overlays-2026-11-06.json).

`downgrade()` returns a `WeakSurface` for app controllers and completion
handlers. `upgrade()` returns `None` once the native widget is deleted or
the surface state is gone. This avoids a controller/painter ownership cycle.

Try `examples/scene_surface.rs`: same painter and two shared translucent
cards over a portable gradient, live theme/language and snapshot support.
It animates only after GL activates; CPU-only/error fallback is static.

```sh
cargo run --example scene_surface -- --snapshot target/surface.png
cargo run --features gpu --example scene_surface
# Release process-CPU/frame diagnostic, inside a private display:
env -u WAYLAND_DISPLAY xvfb-run -a cargo run --release --features gpu \
  --example scene_surface -- --bench target/surface-bench.json
# Real Wayland diagnostic on Hyprland (no-focus spawn, focus event check):
cargo build --release --features gpu --example scene_surface
script/scene-bench
script/scene-bench --cpu
# --size WIDTHxHEIGHT changes window size. Reports go to build/scene-bench/.
```

The bench waits for activation, then samples idle, paced animation and idle
again, forcing motion on only in that process. It reports actual frame
rates, scene builds and parent redraws alongside process CPU. The two-card
workload does not close the lava/5,000-message/4K120 efficiency gates.

A sequential release run on 2026-11-06 used Wayland, a 1920×1080 scale-1
output and a 420×299 surface (420×340 window):

| Backend | Animated fps | Process CPU (% of one core) | Active PSS | Idle frames |
| --- | ---: | ---: | ---: | ---: |
| GPU, NVIDIA RTX 3080 Ti | 60.0 | 0.96% | 46.9 MiB | 0 |
| CPU shader interpreter | 29.9 | 74.6% | 28.0 MiB | 0 |

Both idle intervals used at most 0.014% CPU; neither backend rebuilt the
scene during animation. GPU frames also left the CPU parent untouched.
The monitor recorded zero focus-change events for both runs. These are
process CPU/PSS observations, not GPU utilization or power measurements;
the CPU row measures an animated shader, not ordinary static controls.
[Raw report](scene-surface-2026-11-06.json) includes device and phase data.
The earlier private-Xvfb parity check used an RTX 3070 Ti; those results
must not be treated as the same-device performance comparison. An initial
memory drop could not be attributed to heap trimming: a control without
that change reproduced it, so no allocator optimization was retained.

The managed surface still uses one native GL subwindow for the region. It
is not full-window integration of normal FLTK controls: a widget texture
bridge, CPU popup/native-child stacking integration and semantic virtual
controls remain work. Atlases, batched geometry and native isolated shader
layers are also pending. Prefer one surface for a whole composed region,
not a GL surface per row, and retain the CPU policy for ordinary app UI.

## Keyed card updates

Record an isolated layer with `layer.draw_keyed(NodeId(42), x, y, opacity)`
instead of `draw_at` when it needs to move or fade without a scene rebuild.
The ID belongs to the application and is unique among the keyed layers in
that scene. Child scenes inside isolated layers have their own ID scope.
`draw_keyed` returns a finite-opacity validation error, or the same native
draw success flag as `draw_at` outside recording.

```rust,no_run
use fltk_omarchy::{App, scene::{Layer, NodeId, Rect, Scene, Surface}};
let app = App::new("dev.example.keyed-scene");
let card = Layer::new(Rect::new(0, 0, 120, 50), Scene::default()).unwrap();
let (scene, ()) = Scene::record(|| {
    card.draw_keyed(NodeId(42), 20, 30, 1.0).unwrap();
});
let surface = Surface::new(400, 240);
surface.set_scene(scene);
surface.move_node(NodeId(42), 70, 90).unwrap();
surface.set_node_opacity(NodeId(42), 0.4).unwrap();
assert_eq!(surface.node_bounds(NodeId(42)).unwrap().x, 70);
```

The same `move_node` / `set_node_opacity` operations are available on
`Scene`, `gpu::PreparedScene` and `gpu::Renderer`. The ID maps to its
command/prepared draw in constant expected time after indexing. It does
not depend on how many earlier commands were clipped out. A keyed layer
that starts outside the viewport or at zero opacity is retained, so a
later update can make it visible without preparing/uploading a texture.
This retention uses the same 64 MiB resource budget: use keyed layers for
the composed region, not every row in a large virtual collection.

Position/opacity updates preserve size, paint order and ancestor clips. Opacity applies
once to the isolated group and uses the same quantized 8-bit value on CPU
and GL. NaN/infinity, missing IDs and duplicate IDs produce `NodeError`;
duplicate detection precedes mutation, and GL preparation also rejects
duplicates. Appending a scene invalidates its ID index. CPU updates drop
the optional whole-scene damage backing but retain the isolated resources.
GL updates change retained draw values only, without context calls,
preparation, rasterization or uploads; a managed surface redraws its GL
child only. Updates also reach a prepared scene awaiting its first upload.

`raise_node(id)` moves a keyed layer to the end of its immediate clip
scope, above that scope's other content. A root node moves to the end of
the scene. Raising a child cannot escape its clip or pass later content
outside that scope. It preserves the layer's position/opacity/resources
and updates ID indexes in O(commands + nodes) on CPU or O(draws + nodes)
after preparation. Repeated raises, including before the first GL upload,
change ordering only; managed focus chrome stays above raised root nodes.
A drag preview that leaves a list's clip should be recorded as a separate
root keyed layer; the original item keeps its list clipping.

`Scene::hit_node(x, y)` / `Surface::hit_node(x, y)` return the last painted
keyed layer whose rectangular bounds contain the point, respecting all
ancestor clips and zero group opacity. Surface coordinates are local and
also constrained to its viewport. Hit testing scans the retained commands
without per-query allocations after ID validation. It deliberately uses
rectangular semantic targets: rounded/transparent texels do not shrink
them, unkeyed visuals do not block them, and nested isolated layers are
not traversed. Use the returned ID with a `KeyboardPanel`/app model for
selection, drag state and accessibility; drawing alone does not implement
those behaviors.

`Surface` updates affect the currently retained scene. A painter rebuild
or `set_scene` supplies new node properties: keep durable positions in app
state and use them in the painter. This is not automatic keyed layout or
selection, a reflow operation, a nested-node update API or affine
transformation. The interactive lava example below wires these primitives
to a keyed model and keyboard panel.

## Managed node motion

`Surface::animate_node(id, NodeMotion)` handles position and/or group
opacity with the shared toolkit clock. Defaults are 190 ms OutCubic, like
layout transitions. For a falling card, use an explicit duration/ease:

```rust,ignore
use fltk_omarchy::{anim::Ease, scene::{NodeId, NodeMotion}};
// Update durable final targets in app State/ListModel first.
surface.animate_node(NodeId(42), NodeMotion::new()
    .position(70, 360)
    .opacity(0.0)
    .duration_ms(350)
    .ease(Ease::InCubic)
    .on_done(|surface, id| {
        assert!(!surface.node_animating(id));
        // Remove the item from the app model, or start the next action.
    }))?;
```

Retargeting samples the previous transition at the current clock time and
starts there without a jump. Omitted properties remain unchanged. Immediate
`move_node` / `set_node_opacity` setters and `cancel_node_motion` hold the
current pose and cancel the old transition. Invalid IDs, opacity or easing
fail before interrupting it. Unchanged targets and zero duration settle
without starting a frame loop.

Only moving nodes are sampled; frame storage is reused. GPU frames update
the prepared draws and redraw the GL child at `Pace::Display`, without
rebuilding the painter, rasterizing or uploading. CPU requests are capped
at `Pace::Low` before the global pace policy. The faster of node and shader
pacing wins while both are active. After the last node finishes, a static
scene stops requesting frames; there is no per-node timer.

Reduced motion and hiding a widget settle transitions to their targets.
New transitions on an already hidden widget also settle immediately.
Frozen-clock offscreen snapshots can still sample intermediate frames;
`anim::clock::advance` plus a snapshot/read-back drives those tests without
a repeating timer. `Surface::set_time` affects shader time only; the shared
frozen clock also stops node timelines and automatic frame requests.

Active transitions survive automatic size/theme/language/focus rebuilds
and CPU/GPU switches. A new painter recording supplies layout/colors and
then the active transition supplies its current position/opacity. Missing
IDs cancel their transitions. Final durable properties belong to app state
and should be reflected in its painter, as for immediate keyed updates.
Calling `paint` or `set_scene` replaces content and cancels old transitions.

`on_done` runs once on a later UI turn, outside native draw callbacks and
state borrows. It can safely update its surface or app model. Completion
means the final draw was submitted, or an immediate/hidden/reduced-motion
target settled; it is not a compositor presentation acknowledgement.
Retargeting, immediate setters, cancellation, content replacement and
widget deletion suppress stale callbacks, even if already queued. The
callback receives its surface so it need not capture its owner's strong
clone. `node_bounds`, `node_opacity` and `node_animating` expose the current
sampled state. Motion preserves node size, paint order and ancestor clips;
use a root preview layer for a drag that leaves a list clip.

## Single-surface GL compositor spike

With feature `gpu`, `scene::gpu::{PreparedScene, Renderer}` composes a
retained scene into one current OpenGL 3.3 framebuffer. This remains the low-level
backend; `Surface` above owns its lifecycle for application code. Ordinary
AppWindow controls keep their CPU renderer.

1. Prepare the scene on the UI thread with `PreparedScene::new`, before
   entering a GL callback. It resolves CPU fallbacks and folds clip scopes.
2. In the current GL context, construct `Renderer` once. It uploads textures
   and compiles shader programs. It deduplicates shared image resources and
   shader sources. Textures and the retained instance buffer together are
   capped at 64 MiB.
   Install later prepared revisions with `Renderer::replace_scene` to reuse
   their still-shared immutable images and programs.
3. `Renderer::move_to` and `set_opacity` update a prepared card/sprite
   without reflow, rasterization or texture uploads. Changed instance
   records are uploaded on the next draw. Indexes use visible prepared
   paint order, which can change after raising a node; prefer the keyed
   methods above for application identities.
   Top-level shader opacity changes are not yet supported.
4. `Renderer::draw(logical_size, framebuffer_size, time_override)` submits
   quads in scene order to the bound framebuffer, using scissor clips and
   premultiplied source-over. Changing shader time requires no new text
   rasterization, shader compilation or texture upload.
5. Destroy the renderer while its GL context is still current. Its unsafe
   API documents the caller's context/state/lifetime responsibilities. It
   creates no window or animation timer and does not restore GL state.

Simple opaque and alpha rectangles and top-level portable shaders run
natively in GL. Uniform locations are resolved once per linked program
and shared by its draws, including the overlay pass. Frames perform no
uniform-name queries; context recovery rebuilds the locations together
with their programs. A zero-width/height surface unmaps its native child
and releases its textures outside drawing; restoring positive dimensions
recovers the GPU without an application invalidation or backend error.
Consecutive solid/image quads with the same texture and clip use instanced
batches. Per-instance rectangles and colors use a retained 48-byte record (rectangle, color and UV region)
per prepared primitive, plus one reserved overlay record. CPU storage is
retained and resized on scene replacement; the matching GPU store is allocated
lazily when a batch first needs it and updated after instance properties/order
change. A changed primitive count releases the old GPU store before resizing.
Isolated quads use the simple uniform path, without instance-store
allocation/uploads or attribute-offset updates. Textures and reserved
instance data share the aggregate 64 MiB GPU resource budget. Batches stop
at shaders, texture changes, clips and culled entries; paint order and
keyed raising remain unchanged. `Renderer::frame_stats` exposes submitted
primitives and actual GL draw calls. `SurfaceStats::gpu_primitives` and
`gpu_draw_calls` accumulate those counts across frames, including overlays.
These counters measure submissions, not physical presentation.
`instance_bytes` reports the retained GPU buffer; `gpu_instance_upload_bytes`
counts its cumulative uploads. Shader time alone does not update this buffer.
`scene::shader(rect, &program, &uniforms)` records a shared
`gfx::sl` program and local uniforms; CPU replay evaluates the same program
into bounded RGBA. Logical size follows the target rectangle. On the GPU,
the renderer can override time independently of the retained scene.

Text runs, rounded primitives and isolated layers currently use native
CPU raster fallbacks uploaded once as RGBA textures.
Source images use the same deterministic pixel-center bilinear resampler
in CPU replay and GPU preparation. Filtering happens in premultiplied alpha;
returned public resources remain straight RGBA. CPU draws the result 1:1
in logical units, avoiding a destination-dependent native scaling transform.
`Image::resampled(w,h)` is also usable from workers. One result up to 8 MiB
is shared across clones; output is limited to 64 MiB, larger results are
uncached, and `Image::clear_cache` releases the optional shared result.
Direct native fallback textures preserve the black pass's
premultiplied RGB instead of unpremultiplying and premultiplying it again;
that round trip otherwise loses color precision at overlapping edges.
Public `Image` and isolated-layer resources remain straight RGBA. GL
textures use premultiplied colors before filtering, which avoids halos from
transparent texels. The image page does not provide a glyph atlas or native GPU layer rendering. A shader inside an
isolated CPU layer is flattened at preparation time; draw-time animation
currently affects only top-level shader commands. Rebuild preparation when
layout/theme/language/fonts/scale change. CPU fallbacks are at logical 1×.

Validation command (a real GL context, window inside a private Xvfb):

```sh
env -u WAYLAND_DISPLAY xvfb-run -a -s "-screen 0 1280x800x24" \
  cargo run --features gpu --example scene_gpu_check
```

The example compares full CPU and GPU RGB images at 256×192 (maximum
channel error ≤2), verifies shader-time changes, card movement/fading without texture uploads,
and checks GL errors.
It includes a gradient background shader, translucent rounded card with
text, nested clipping, alpha rectangles and a scaled image with transparent
texels. CPU/GPU PNGs and JSON go to `target/scene-gpu-check`. On NODE,
NVIDIA RTX 3070 Ti: max channel error 2, GL error 0, 8 prepared primitives,
70,296 bytes of retained textures. This is a correctness result, not a CPU,
GPU or memory efficiency benchmark. CI runs the same check in Xvfb; a
software GL renderer can prove correctness but not power consumption.

## Current limits and next stages

- Only shared `paint` helpers are recorded. Raw `fltk::draw` operations,
  arbitrary widget subtrees and `gfx::Canvas` painting are not captured.
  Use `scene::clip` rather than raw push/pop clip calls when recording.
- Text is still logical text rendered by FLTK on replay, not shaped glyph
  runs or a GPU atlas. Recording text needs the UI thread and a surface
  for measurement. This API does not make FLTK objects thread-safe.
- Scene clips are rectangular. Paths, shadows, rounded alpha
  surfaces and affine transforms still need common representations and
  CPU/GPU parity checks; GPU isolated-layer shaders remain pending.
- CPU-widget overlays now have an automatic retained texture bridge over
  managed surfaces. Automatic per-widget scene caches, a full-window
  widget renderer/damage tracker and renderer-selection policy remain
  pending. The managed Surface hosts explicitly recorded scene content.
  Existing GPU shaders remain `gfx::View`/`GpuView`, with their documented
  subwindow/composition limits.

Next: integrate full-window widget rendering, quad batching and text/image
atlases, then native isolated shader layers. Stable keyed updates and node
motion are implemented. Extend the remaining common primitives. Measure real hardware at 4K/120 Hz before changing the
default. Compare total process/driver memory and CPU, not submission time
alone; software GL is useful for correctness, not efficiency claims.

## Video reference: lava.mp4 (2026-11-06)

Reviewed the supplied 9.12 s, 1800×1600 video across its full duration.
It shows a five-task list, rounded dark cards with category badges,
dragging a raised card through reordered rows, dropping the card into a
lava horizon, disappearance with the list compacting from five to four,
and a continuously moving floor, embers, flame and smoke effect.

| Visible behavior | Toolkit status |
|---|---|
| Cards, labels, badges, controls and themed styling | Existing custom row renderers, shared `paint`, glyphs/icons |
| Drag reorder and closing the gap after removal | Existing `List::reorderable`, `ListModel` diffs and row motion; the app owns drop semantics |
| A lifted or falling copy of a row | The `lava` example uses a root keyed preview sharing its row layer, soft shadow and managed position/opacity motion |
| Fade/reveal and a short drop-triggered effect | Example wires a 450 ms bounded GPU shader pulse to drag/drop; CPU fallback keeps the floor static |
| Animated lava floor with particles behind translucent cards | Explicit `scene::Surface` now composes top-level shaders and retained translucent card textures in one GL child; native shader layers and the full-window widget bridge remain pending |
| Quiet idle after interaction | Continuous decorative animation is not idle; pause/freeze it when hidden, reduced motion or saver mode is active |

The toolkit can now build the interactive composition in one explicit
canvas. This does not establish the full video's continuously animated
floor, smoke, affine card rotation or low-power budget at every resolution.

## Interactive lava example

```sh
cargo run --release --features gpu --example lava
cargo run --release --example lava -- --cpu
cargo run --example lava -- --snapshot target/lava.png
# Private, no-focus Wayland diagnostic with the actual app controller:
cargo build --release --features gpu --example lava
SCENE_BENCH_BIN="$PWD/target/release/examples/lava" script/scene-bench --size 720x680
SCENE_BENCH_BIN="$PWD/target/release/examples/lava" script/scene-bench --cpu --size 720x680
```

One `ListModel` and one `KeyboardPanel` own the five tasks. Dragging previews
the keyed order and moves a separate root copy, so the row can leave its
list clip. Release above the floor settles into its slot; release over the
floor falls, fades, removes the model row once and closes the gap. Rows and
previews share card resources. A retained soft shadow follows the preview.
Theme, corners, text and right-to-left layout follow toolkit settings;
the example adds English source keys and a Spanish catalog.

Use arrows to select, Space to pick up/drop, arrows while held to reorder,
Escape to restore the original slot, Enter/Delete/Backspace to complete,
Mod+Shift+Up/Down to reorder and Mod+R to reset. Rapid input during settling
waits for the current flight. F1 help uses the existing overlay on CPU and
is composed above the live scene with GPU selected.

GPU lava pulses only during interaction and stops after 450 ms. CPU lava
is a cached static specialization of the same base shader, avoiding shader
interpretation during each card-motion frame. This is an explicit energy
choice; animated CPU/GPU effects are not the same workload. Reduced motion
settles immediately, and static/finished scenes request no animation frames.

The UI suite imports the example's actual controller/painter. It checks
pointer and keyboard reorder, cancel, falling 5-to-4 removal, reset during
flight, all rows removed, resource reuse during motion and stopped GPU
frames afterward. A frozen actual GPU read-back of the complete surface
is compared against CPU replay with maximum RGB channel error 2, including
the animated shader. This parity test renders the same GPU scene on CPU;
the ordinary CPU demo deliberately chooses the static shader instead.
Capture callbacks can close their window safely on a later UI turn;
deletion suppresses subsequent queued capture callbacks.

The benchmark exercises that same controller without taking focus: repeated
350 ms reorder cycles, then one falling deletion, with idle phases on both
sides. Shader pulses overlap during repeated GPU interactions. It runs in
a private hidden special workspace and checks compositor focus events.
Frame counts are submissions, including input-triggered redraws, not
display presentation rates. Hidden-workspace timings do not measure GPU
power or establish visible display pacing, the 5,000-message o3 budget,
or the 4K/120 gate.

Release measurements on NODE (2026-11-06), native Wayland on NVIDIA
RTX 3080 Ti, 1920×1080 at 60 Hz/scale 1, 720×639 surface, sequential runs
on a hidden special workspace:

| Workload | CPU (% of one core) | Draw submissions/s | Active PSS |
|---|---:|---:|---:|
| CPU cards, cached static floor | 5.80 | 24.0 | 34.7 MiB |
| GPU cards and overlapping bounded pulses, warm disk cache (two runs) | 1.17–1.22 | 64.0–64.7 | 71.8–71.9 MiB |
| Same GPU scene, disk shader cache disabled | 1.34 | 64.3 | 118.6 MiB |

Both idle phases submitted **zero frames**, at 0.006–0.014 % process CPU.
Reordering caused zero scene rebuilds, preparations or uploads; input and
completion caused eight parent draws in the three-second animation sample.
The GPU retained 1,357,824 texture bytes. All runs finished the falling
deletion with four stable IDs and no flight, and recorded zero focus changes.
The default first GPU run at a compositor-selected 1100×693 also reached
132.3 MiB active PSS; a later warm floating run at that same size used
73.6 MiB. Controlling size alone did not explain the memory difference.

The cache-disabled control isolates a cold-compilation cost retained in
the process; it does not prove the ownership of every allocated page.
An experiment requesting [`glReleaseShaderCompiler`](https://registry.khronos.org/OpenGL/specs/gl/glspec45.core.withchanges.pdf)
after linking changed 118.6 to 118.5 MiB, effectively no improvement, so
it was not retained.
No heap-trimming workaround or global driver configuration was added.
The ≤100 MiB budget therefore remains **unmet for this cold GPU run**.
Use `script/scene-bench --no-shader-cache` to repeat that control without
deleting or changing the user's shader cache. These results establish
bounded interactive composition, not GPU default policy or power savings.
Raw reports and diagnostic metadata: [lava-scene-2026-11-06.json](lava-scene-2026-11-06.json).

## Validation

`scene_ui` compares entire deterministic PNGs for immediate drawing,
retained replay and a reconstruction through disjoint damage tiles. The
fixture includes nested clips, rounded borders, Unicode and literal `@`,
weights, spacing and alpha over different backgrounds. Isolated-layer checks cover group
opacity at overlaps, changing striped backgrounds, rounded transparent
corners, negative child origins, nesting and translated damage replay.
Unit checks cover
nested/unwinding recorders and overflowing rectangle intersections.


Managed-surface checks additionally cover cached CPU painting, explicit and
language invalidation, keyboard input, one local pointer delivery through a
mapped GL child, mapped snapshots, hide/show context recovery, backend
switches, motion-off/hidden clock stopping and motion-on resume. A GPU clock
check asserts frames advance without parent draws or scene rebuilds.

## Ordered batch validation (49f013e, 2026-11-06)

The ordered solid/image workload submits 310 visible quads in 8 GL draw
calls, with a 9,952-byte instance store, on NVIDIA RTX 3070 Ti and Mesa
llvmpipe. Whole-frame read-back stays within 2 RGB levels of CPU replay
before/after keyed movement, opacity and raising. Clip/shader barriers and
overlapping colors are covered. Changing shader time uploads no instances;
zero width/height releases textures and the instance buffer, then positive
size recovers GL. The low-level FBO check starts with a shader before any
instanced draw: initial/keyed/deferred maximum differences are 1/2/2 and
GL error is zero on both drivers.

At checkpoint `49f013e`, the real lava controller was a different workload: its card textures are
distinct, so it currently gains no batching. It takes the singleton uniform
path with zero instance-store allocation/uploads. A matched release control
from `81ca273` uses the same controller and overlay at 720×639 on Wayland,
RTX 3080 Ti, hidden workspace, no focus. Each case below is one run:

| Cache condition | Control CPU/core | Adaptive CPU/core | Control active PSS | Adaptive active PSS |
|---|---:|---:|---:|---:|
| Repeated normal run | 2.05% | 1.60% | 93.7 MiB | 94.3 MiB |
| Disk caching disabled | 1.31% | 2.48% | 137.5 MiB | 139.8 MiB |
| First normal run | 1.63% | 1.92% | 150.8 MiB | 150.8 MiB |

CPU variation does not establish a precise improvement or regression.
Anonymous PSS is approximately 30.1/67.1/79.1 MiB for repeated/disabled/first
runs in both renderers. Earlier measurements at different times are not a
matched memory comparison. The cold ≤100 MiB gate remains unmet. Uniform
arrays and always-instanced quads were explored and discarded; these
controls do not prove a shader-layout cause for the compilation memory.
No heap tuning or compiler-release hint is retained.

All runs finish the 5→4 removal with no flight left, zero focus changes,
zero idle frames and no idle instance uploads. Submission rates around
64/s include input-driven frames and are not presentation rates. A dense
workload CPU comparison, image/glyph atlases, cold-memory work, native
isolated layers, full-window integration and the real o3/high-refresh/power
gates remain. [Raw controls and submission counts](scene-batching-2026-11-06.json)
include the exploratory variants without claiming their results for the
retained renderer.


## Bounded image page

The renderer can pack a batch of distinct fallback/image resources into a
page of at most 2048×2048 (also limited by the driver's maximum texture
size). A deterministic shelf plan chooses the smallest fitting candidate;
allocation including padding must remain within twice the packed source
bytes plus 4 KiB, and the combined 64 MiB resource budget. A single resource,
images too large for the page, a non-fitting set or a budget conflict use
individual textures. Pages belong to a renderer/context. The residency
policy below can retain several pages across revisions; this is not a glyph
atlas.

Each tile has a one-pixel extruded gutter with premultiplied edge colors.
The page receives individual padded tile uploads, without a full-size CPU
page allocation. Normalized UV rectangles are retained with each draw and
instance. Different images on the same page can batch while preserving
paint order, clips, opacity, raising and independent overlay updates.
Current filters are linear with no mipmaps. Context recovery rebuilds the
page and closing/hiding releases it with the renderer.

The real lava reorder interval now submits 2,210 primitives in 1,380–1,381
GL calls, versus 2,210 calls in the earlier non-atlas observations. Its
logical texture allocation is 1,537,520 bytes and the active instance store
is 864 bytes. No scene preparation, texture/overlay upload or CPU parent
rebuild is added during that interval (the existing controller still draws
its parent eight times). Warm/first-normal/cache-disabled runs measured
2.18/1.79/1.98% of one core and 96.6/153.3/141.2 MiB active PSS, respectively.
These are one observation per condition, not proof of a CPU/PSS saving.
Idle phases submit zero frames or instance uploads and all runs keep focus
unchanged. Cold memory still exceeds the budget. [Raw atlas observations](scene-atlas-2026-11-06.json).

## Resources across scene revisions

Managed `Surface` now replaces a prepared scene in its existing renderer.
Low-level users can call `Renderer::replace_scene(prepared)` with that
renderer’s original GL context current. Shared immutable `Image` resources
and cached `Layer` rasters reuse textures, page UVs and shader programs.
New images upload individually or into a new bounded page. Programs absent
from the new scene are deleted; still-used source programs, uniform
locations, basic pipeline and VAO remain. Instance storage is resized safely
when the primitive count changes. Overlay capture/upload remains independent.

Resources have monotonic identities independent of pixel addresses. Clones
and cached resampling preserve the corresponding immutable identity;
`Layer::clear_raster`, a new image, or an uncached resample gets a new one.
GPU residency metadata does not pin an extra CPU pixel copy. Removed IDs
are forgotten immediately. A page survives only while at least one current
tile uses it; the last removal deletes it. Vacant slots are not reused under
an old identity. If holes prevent a new scene fitting in the combined
64 MiB texture/instance/overlay budget, the current images are repacked
before new allocations. Individual textures avoid padding that would exceed
the budget. This bounds live resource accounting, not driver-internal
transients, total process PSS or shader compiler memory.

Preparation still rasterizes unretained native text/borders into new
fallback images; it is not a glyph atlas or automatic content-hash cache.
Reuse requires an immutable resource shared with the previous scene.
Explicitly cached `Layer` content must be invalidated for theme/font/scale
changes as before. Hiding, zero area, deleting, switching backend or losing
the context releases/abandons the context-local cache; recovery uploads
again. No cache spans unrelated contexts or holds unused past scenes.

`Renderer::resource_stats()` describes the last scene installation:
`texture_uploads` (unique image payloads), `texture_upload_bytes` (including
gutters), `reused_images`, `program_links` and pressure `repacks`. Its
`resident_images`, `texture_objects` and `shader_programs` describe current
residency; the last includes the basic program. `SurfaceStats` accumulates
`gpu_texture_uploads`, `gpu_texture_upload_bytes`, `gpu_image_reuses`,
`gpu_program_links` and `gpu_image_repacks`. The older `gpu_scene_uploads`
counts installed revisions, even when no texture upload occurs. Overlay
uploads have their own counter. An invalid instance budget leaves the
renderer unchanged; a later installation/allocation error clears scene
images/draws/programs, and another valid replacement can recover.

The native gate prepares all CPU references before GL, then checks complete
readback pixels in 256 revisions with changing images, overlapping native
Unicode text layers and varying instance counts. A same-binary
`--recreate` control discards the renderer each revision. It also exercises
63 MiB of live texture data, pressure compaction, rejection of 64 MiB plus
instances, recovery and final empty-scene release:

```sh
env -u WAYLAND_DISPLAY xvfb-run -a cargo run --offline --features gpu --example scene_residency_check
env -u WAYLAND_DISPLAY xvfb-run -a cargo run --offline --features gpu --example scene_residency_check -- --recreate
```

The control isolates renderer lifetime within the current binary; it is
not a historical-binary performance comparison. Readback wall time includes
driver synchronization and pixel comparison. Upload/link counts establish
less resource work, without proving CPU, PSS, power or presentation savings.
Glyph residency, full-window rendering and the cold-memory gate remain open.

On NVIDIA RTX 3070 Ti and Mesa llvmpipe LLVM 23.1.3, the 255 replacements
uploaded 587,520 bytes in 255 image uploads, reused 510 resources and linked
zero programs. The recreation control uploaded 7,254,240 bytes in 765 uploads
and linked 510 programs. Both modes/drivers passed complete readback with
maximum RGB delta 1, pressure compaction and rejection/recovery. Small-scene
texture accounting stayed at 33,024 bytes with retention, matching the initial
page plus one replacement tile, and returned to zero on empty replacement.
[Raw residency observations and source hashes](scene-residency-2026-11-06.json).


## Post-compilation heap diagnostic

With `SCENE_BENCH_BIN` pointing to the built lava example,
`script/scene-bench --overlay --no-shader-cache --trim-heap` requests a
one-time allocator trim in the lava benchmark 250 ms after GPU activation.
It is a diagnostic option, not a renderer policy. The script rejects a
binary that does not acknowledge the option. A marker before the operation
allows the parent to sample PSS; allocator counters are sampled immediately
around it. Mapping results contain aggregate classes, not addresses/paths.

Two ABBA stages (control/trim/trim/control) found only a transient benefit.
Trim took approximately 0.19–0.27 ms. With caches disabled, idle anonymous
PSS was about 68.7 MiB in controls and 66.8 MiB after trimming. After the
5→4 deletion both returned to about 69.1 MiB. Reported occupied arena space
was approximately 55.1 MiB, with 2.3 MiB free; trimming did not change the
occupied count. In two normal-cache observations it was about 18.3 MiB
occupied and anonymous PSS about 31.8 MiB. These allocator counters do not
identify the allocation owner or measure resident memory. File-backed
sharing also changed total PSS substantially, including one early trim
case, so a total-PSS difference alone is not evidence of heap reclamation.

The optional `platform::memory::allocator_snapshot` dynamically resolves
and verifies the glibc provider for `mallinfo2`, returning `None` on other
platforms, older libraries or interposed providers. Use it after allocator
setup, without concurrent allocator reconfiguration. Counter semantics:
[GNU allocator statistics](https://sourceware.org/glibc/manual/latest/html_node/Statistics-of-Malloc.html).

No heap trim or tuning was added to `Surface`, `Renderer` or normal app
startup. The cold-memory gate remains unmet. Further work must investigate
occupied allocations and compilation/context caches rather than assume
large amounts of freed heap are available. The Khronos
[parallel-compile hint](https://raw.githubusercontent.com/KhronosGroup/OpenGL-Registry/main/extensions/KHR/KHR_parallel_shader_compile.txt)
is a possible separate experiment; its queried state is a request, not an
actual worker count or guarantee, and no policy has been selected here.
[Raw staged controls](scene-heap-2026-11-06.json) preserve all observations.
