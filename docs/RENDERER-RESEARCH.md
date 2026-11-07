# Renderer optimization review: Rune, GPUI and Flutter

Reviewed on 2026-11-06. The implementation target remains the complete
[o3/toolkit plan](O3-PORT.md), with CPU as the default and explicit CPU,
memory, idle and high-refresh gates. This review supplies implementation
ideas; it is not a benchmark comparing the frameworks.

## Source snapshots

| Framework | Revision inspected | Primary implementation sources |
|---|---|---|
| Rune | `4ca683561a100e16b1ce32f3576f4ade61bd2924` | [Renderer and isolated scopes](https://github.com/anomalyco/rune/blob/4ca683561a100e16b1ce32f3576f4ade61bd2924/crates/rune/src/render.rs), [glyph atlas](https://github.com/anomalyco/rune/blob/4ca683561a100e16b1ce32f3576f4ade61bd2924/crates/rune/src/text/atlas.rs) |
| GPUI / Zed | `e231594b6b8818ea8d668ee2eb73e1fb09300654` | [Scene order and batching](https://github.com/zed-industries/zed/blob/e231594b6b8818ea8d668ee2eb73e1fb09300654/crates/gpui/src/scene.rs), [window invalidation](https://github.com/zed-industries/zed/blob/e231594b6b8818ea8d668ee2eb73e1fb09300654/crates/gpui/src/window.rs), [wgpu renderer](https://github.com/zed-industries/zed/blob/e231594b6b8818ea8d668ee2eb73e1fb09300654/crates/gpui_wgpu/src/wgpu_renderer.rs), [atlas uploads](https://github.com/zed-industries/zed/blob/e231594b6b8818ea8d668ee2eb73e1fb09300654/crates/gpui_wgpu/src/wgpu_atlas.rs) |
| Flutter | `fabacd663126fbcbf9629b5365b093be08369d7a` | [Raster cache](https://github.com/flutter/flutter/blob/fabacd663126fbcbf9629b5365b093be08369d7a/engine/src/flutter/flow/raster_cache.cc), [cache admission](https://github.com/flutter/flutter/blob/fabacd663126fbcbf9629b5365b093be08369d7a/engine/src/flutter/flow/layers/display_list_raster_cache_item.cc), [damage diff](https://github.com/flutter/flutter/blob/fabacd663126fbcbf9629b5365b093be08369d7a/engine/src/flutter/flow/diff_context.cc) |

The source snapshots above are the evidence for the following observations.
The proposed applications are our own design decisions and still require
implementation, parity checks and measured results.

## Findings and application

| Area | Observed design | Application to this toolkit | Evidence required |
|---|---|---|---|
| Retained boundaries | Flutter separates repaint regions; a boundary can retain a costly static subtree while nearby content changes. [RepaintBoundary](https://api.flutter.dev/flutter/widgets/RepaintBoundary-class.html) | Keep scene data, CPU widget textures and shader time independent. The overlay bridge implements this separation: static popups reuse their texture while the underlying GL shader runs. | Actual GL read-back parity; zero recaptures/uploads/parent draws during a static overlay's shader frames; no idle frames. |
| Quad submission | Rune uses instanced quads for shared primitive kinds, with additional passes for isolated scopes/effects. GPUI emits ordered batches of primitive kinds and texture pages. | Cache uniform locations, batch compatible consecutive quads, then use instanced geometry. Preserve overlapping paint order, ancestor clips and keyed raising. | Identical images before/after movement, overlap, clips and raising; draw/bind counts and CPU at a fixed workload. |
| Atlas residency | Rune bounds glyph pages, tracks which pages are used, and marks changed rows. GPUI queues tile uploads and tracks texture generations. | Bounded glyph/image pages, padding against filtered bleed, partial uploads and generation-safe identities. Keep a small steady-state budget and release unused pages. | Mixed scripts/emoji/font weights and device scales; eviction/reuse correctness; bounded CPU/GPU memory and upload bytes across long scrolls. |
| Layer composition | Rune's scopes describe opacity, blend, filtering and clipping, using lazy offscreen targets. | Native isolated shader layers, bounded target reuse and affine transforms. Motion should change composition properties without re-rasterizing static text. | CPU/GPU group-opacity parity, overlap, nested clipping, rotation/scaling, changing shader time and context recovery. |
| Invalidation | GPUI tracks dirty views and presentation separately. Flutter's diff records changed paint regions; its raster cache admits reusable content and evicts unused entries. | Separate layout, paint, composition and presentation changes. Retain resources across scene revisions; virtualize the 5,000-message workload. | A single changed row avoids full layout/texture replacement; final pixels match full redraw; deletion/context loss releases resources. |
| Shader preparation | Impeller prepares shaders/reflection ahead of time and controls pipeline caching explicitly. [Impeller architecture](https://docs.flutter.dev/perf/impeller) | Investigate cold compilation separately from warm rendering. Keep program creation out of frames and reuse pipelines across scene revisions. Assess portable offline/binary caching against the GL 3.3 fallback contract. | Matched cold-cache controls, startup latency and total PSS; unsupported-driver recovery; no runtime frame compilation. |

Our GL compositor has different platform and dependency constraints from
wgpu/Metal/Vulkan implementations. Offline shader support in another engine
does not establish that the same approach is available on every supported
OpenGL driver. Preserve native font shaping, theme behavior, keyboard/IME,
accessibility and CPU rendering while changing submission and caching.

## Implementation and verification order

1. Finish the automatic widget/overlay bridge, including native-child
   event routing, translucent stacking, fade phases, resize/theme changes,
   fallback/recovery and resource release. Its CPU reference must exclude
   FLTK's extra native-window print pass.
2. Measure cold memory by allocation category; retain only optimizations
   that improve a matched control. Current lava PSS is about 72 MiB warm
   and 119 MiB with shader disk caching disabled. The cold ≤100 MiB gate
   remains open. A shader-compiler release hint did not improve it.
3. Cache pipeline state and uniform locations, add ordered batching and
   instancing, and measure actual submissions/binds/uploads. Uniform
   locations and bounded consecutive solid/image instancing are now
   implemented; the latter preserves shaders, clips, texture boundaries
   and keyed raising, with actual primitive/draw-call counters. Preserve
   updates across native re-entrant message dispatch instead of nesting
   mutable draw state; GPUI's window frame callback explicitly defers such
   draws and keeps presentation demand.
4. Add bounded glyph/image atlases and persistent resources across scene
   revisions. Validate native shaping, Unicode, DPI, eviction and reuse.
   Image pages and current-scene image/program residency are now implemented
   ([SCENE.md](SCENE.md#resources-across-scene-revisions)): stable identities,
   retirement, bounded pressure compaction and native readback checks.
   Native Unicode layers reuse their rasters; unretained text still prepares
   a new fallback image. Glyph atlases and mixed-DPI gates remain outstanding.
5. Add native isolated shader scopes, remaining common primitives,
   affine transforms and bounded offscreen-target reuse.
6. Integrate the common painter with full-window controls and virtual
   document rendering. Cover every outstanding item in O3-PORT's P0/P1
   requirements, including the sheet and notice-card behaviors, rather
   than treating a status-table entry as proof of all its subrequirements.
7. Run the real o3 protocol spike and the complete idle/streaming/memory
   and 5,000-message 4K/120 Hz gates before changing renderer policy.
   Physical presentation rate, GPU power and CPU submission rate are
   separate measurements. Verify hidden and zero-area regions as well.

The existing merged APIs and synthetic shell are inputs to this work.
The interactive lava example proves a composed region; it does not finish
the full-window GPU renderer, atlas/layer work or real o3 migration gates.

## Scope retained from the user request

The 2026-11-06 instruction is to implement the complete documented plan,
using Rune, GPUI and Flutter as references. Keep all remaining requirements
and gates visible until their current-state evidence proves completion.
Work assigned to o3 in O3-PORT section 5 remains part of the protocol-spike
integration boundary; the external terminal, bar and on-demand browser
helpers keep the contracts specified there.
