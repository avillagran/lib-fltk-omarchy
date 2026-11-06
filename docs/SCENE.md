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
recording is off. A scene creates no animation timer or GL context. Full replay creates no
pixel backing store. Recording allocates commands and copies text; retain
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

## Current limits and next stages

- Only shared `paint` helpers are recorded. Raw `fltk::draw` operations,
  arbitrary widget subtrees and `gfx::Canvas` painting are not captured.
  Use `scene::clip` rather than raw push/pop clip calls when recording.
- Text is still logical text rendered by FLTK on replay, not shaped glyph
  runs or a GPU atlas. Recording text needs the UI thread and a surface
  for measurement. This API does not make FLTK objects thread-safe.
- Scene clips are rectangular. Paths, shadows, rounded alpha
  surfaces, transforms, isolated opacity layers and shader commands still
  need common representations and CPU/GPU parity checks.
- There is no automatic per-widget scene cache, damage tracker, GL scene
  renderer, plain-FLTK texture bridge or renderer-selection policy yet.
  Existing GPU shaders remain `gfx::View`/`GpuView`, with their documented
  subwindow/composition limits.

Next: extend the common primitives, add a single GL scene surface with
quad batching and text/image atlases, then compose widget fallbacks and
shader layers. Measure real hardware at 4K/120 Hz before changing the
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
weights, spacing and alpha over different backgrounds. Unit checks cover
nested/unwinding recorders and overflowing rectangle intersections.
