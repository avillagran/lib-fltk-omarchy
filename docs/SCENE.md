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

GL activation waits three short ticks after the parent maps (Wayland's
first-frame restriction). Static views install no repeating timer after
activation. Resize, hide/show and CPU/GPU policy switches prepare/recover
resources as needed. Preparation/renderer errors return to CPU and are
available from `backend_error`; `invalidate` permits a retry. Applications
do not own context creation/destruction. Native draw callbacks own the GL
resources; tracked weak handles let the host update them without extending
their lifetime beyond the native window. Known lost contexts abandon old
names rather than deleting them in a different context.

`AppWindow::snapshot` and `layer::capture` force scene surfaces through CPU
replay, even after GL activation. Use `read_gpu_pixel` for actual GL output.
Other custom offscreen harnesses should render the local `Scene` directly;
a generic native widget capture is not automatically detectable everywhere.
The widget retains scene content across shader time changes. `SurfaceStats`
separates scene builds, parent surface draws, CPU frames, GPU submissions,
GPU preparations, scene uploads and retained texture bytes. Frame counts
are not presentation callbacks; scene uploads count renderer creation,
including resource recovery after context loss.

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
transformation. Automatic motion and the complete falling/reorder
interaction remain work.

## Single-surface GL compositor spike

With feature `gpu`, `scene::gpu::{PreparedScene, Renderer}` composes a
retained scene into one current OpenGL 3.3 framebuffer. This remains the low-level
backend; `Surface` above owns its lifecycle for application code. Ordinary
AppWindow controls keep their CPU renderer.

1. Prepare the scene on the UI thread with `PreparedScene::new`, before
   entering a GL callback. It resolves CPU fallbacks and folds clip scopes.
2. In the current GL context, construct `Renderer` once. It uploads textures
   and compiles shader programs. It deduplicates shared image resources and
   shader sources and caps retained textures at 64 MiB.
3. `Renderer::move_to` and `set_opacity` update a prepared card/sprite
   without reflow, rasterization or uploads. Indexes use visible prepared
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
natively in GL. `scene::shader(rect, &program, &uniforms)` records a shared
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
transparent texels. This is not a glyph/image
atlas, geometry batching or native GPU layer rendering. A shader inside an
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
- There is no automatic per-widget scene cache, damage tracker, full-window
  widget renderer, plain-FLTK texture bridge or renderer-selection policy
  yet. The managed Surface hosts explicitly recorded scenes only.
  Existing GPU shaders remain `gfx::View`/`GpuView`, with their documented
  subwindow/composition limits.

Next: add quad batching and text/image atlases, then native isolated
shader layers, stable keyed scene updates and widget/overlay texture
fallbacks. Extend the remaining common primitives. Measure real hardware at 4K/120 Hz before changing the
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
| A lifted or falling copy of a row | `layer::capture`, offscreen layers and the shared animation clock are building blocks; no complete drag-flight component |
| Fade/reveal and a short drop-triggered effect | Existing layer opacity/effects and bounded `gfx::View` shaders; needs app wiring and measurements |
| Animated lava floor with particles behind translucent cards | No efficient complete composition path yet; requires the window scene and shader/alpha layers |
| Quiet idle after interaction | Continuous decorative animation is not idle; pause/freeze it when hidden, reduced motion or saver mode is active |

The current toolkit can build the task UI and bounded effects. It cannot
yet promise the complete video's rendering at display refresh within the
low-power budget. A prerecorded video background is a possible app choice,
but it neither proves a native scene renderer nor replaces interactive
composition. Do not count it as completion of the GPU plan.

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
