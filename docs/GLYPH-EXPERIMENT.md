# Native positioned glyph experiment

`scene_glyph_check` is a Linux diagnostic for the pinned fltk-rs/fltk-sys
1.5.23 Cairo driver. It does not change `PreparedScene`, allocate an atlas,
or add a dependency to the library. The current renderer retains its native
whole-command text fallback.

Execution is currently **on hold** following the fixture-deletion report;
see the [static fixture audit](TEST-FIXTURES-AUDIT.md). The commands below are
reference instructions, not authorization to resume them. The recorded
receipt belongs to checkpoint `2d6c0ca`; subsequent output-preservation changes
have source review only and have not been compiled or executed during the hold.

The experiment compares four replays against independent native FLTK
black/white captures at logical 1x:

1. A complete Pango layout for each line emitted by FLTK's layout callback.
2. Native font runs, preserving shaped glyph indices, advances, offsets,
   visual order, and fallback fonts.
3. A candidate partition into overlapping ink/cluster groups.
4. The same positioned glyphs drawn individually, without reshaping them.

All comparisons use the same native `ImageSurface` backend. A Cairo memory
surface was not an interchangeable reference: even complete layouts differed
from X11 native rasterization. FLTK's half-pixel driver translation cancels
its half-pixel text translation at logical 1x. Its cfltk `draw_text2` wrapper
passes `draw_symbols=0`; the callback must match that setting as well.

The native Cairo font-description method is called only after an RTTI check.
No private FLTK object offsets are read. Pango C structures and borrowed
glyphs are consumed while the owned layout/context remain alive. The probe
requires Pango >=1.50.6 and validates FLTK's corresponding metrics branch
by native pixel comparison. This is not a portable product ABI promise.

## Running the gates

Build in the repository's canonical target directory:

```sh
cargo build --offline --example scene_glyph_check
env -u WAYLAND_DISPLAY OMARCHY_FLTK_A11Y=off xvfb-run -a \
  target/debug/examples/scene_glyph_check target/glyph-check
script/glyph-wayland-check
GLYPH_CHECK_FONTS=/usr/share/fonts script/glyph-wayland-check
```

The output path must be new. The probe creates that directory exclusively,
with mode 0700, before FLTK initialization; an existing directory, file or
final symlink is rejected without opening its artifacts. Choose another path
for each invocation. No automatic cleanup is performed. The Wayland runner
already creates a unique private fixture and now passes a new `glyphs/` child
to the probe; its compositor/application logs remain alongside that child.

The Wayland runner uses the existing local
`localhost/o3-qml-wayland-tests:local` image, rootless headless Sway, no
network and no host display/input sockets. It mounts the probe and a new
private fixture directory. The optional font directory is mounted read-only;
its fontconfig cache stays inside the fixture. `GLYPH_CHECK_BIN` and
`PRESENT_WAYLAND_IMAGE` override the binary and local image respectively.

The default exit status gates **positioned native runs**. JSON/stdout report
the individual and ink-group results separately, even when the run gate
passes. To require either experimental partition on X11:

```sh
env -u WAYLAND_DISPLAY xvfb-run -a target/debug/examples/scene_glyph_check \
  target/glyph-groups --require-groups
env -u WAYLAND_DISPLAY xvfb-run -a target/debug/examples/scene_glyph_check \
  target/glyph-individual --require-individual
```

Every gate keeps the same maximum channel-delta threshold of 2. The probe
writes full positioning/font reports and PNGs before returning a failed gate.
On other operating systems the diagnostic reports unsupported (exit 2).

## Recorded result and implications

The [receipt](evidence/native-glyph-layout-20261106.json) distinguishes X11,
the container's own fonts, and Wayland with the host fonts mounted read-only.
Whole layouts and native runs reproduce FLTK exactly in all recorded cases,
including proportional-font ligatures, combining marks, RTL/mixed text,
fallback scripts, color emoji where available, wrapping, clipping, bold,
and a missing-glyph box. The report records actual fonts; a missing emoji
rendered as a box is not evidence for color emoji support.

Independent glyph painting fails the threshold in connected scripts despite
preserving native positions. Grouping overlapping ink and intact clusters
fixes the X11 corpus, but **does not pass all fonts**: Noto Nastaliq Urdu in
the Wayland host-font run still differs by 18. Whole-run replay remains exact
in that same case. Splitting one native glyph operation into several source-
over operations is therefore not admitted as an equivalent atlas path.
The candidate partition is a quadratic diagnostic reference, not a hot-path
algorithm or an approved fallback policy.

Next work must preserve native coverage for connected glyphs, then validate
actual bounded tile rasterization, alpha/color handling, stable font/glyph/
phase identities, CPU cache eviction, GL page residency and uploads, context
loss, theme/font invalidation, mixed DPI, and CoreText/Windows backends.
This experiment establishes the positioned-layout bridge; it establishes
no GPU performance, PSS, power, or complete glyph-atlas result.

Primary API references: [run extents](https://docs.gtk.org/Pango/method.LayoutIter.get_run_extents.html),
[run baseline](https://docs.gtk.org/Pango/method.LayoutIter.get_run_baseline.html),
[native glyph replay](https://docs.gtk.org/PangoCairo/func.show_glyph_string.html),
and the [Pango 1.58.2 renderer](https://github.com/GNOME/pango/blob/1.58.2/pango/pangocairo-render.c).
