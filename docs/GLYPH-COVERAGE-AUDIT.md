# Glyph coverage audit metadata proposal

This is a **static, uncompiled diagnostic proposal** based on `8a2de43`.
The execution hold remains in force. No formatter, syntax checker, build,
test, native probe, GUI or CI was run to prepare it. Do not execute the
diagnostic or the commands in earlier guides without explicit authorization.

## Evidence gap

The receipt from `2d6c0ca` records exact whole-run replay but a maximum
channel difference of 18 for the ink/cluster partition with Noto Nastaliq
Urdu in the Wayland host-font case. It records glyph positions and group
sizes, but not the font ink rectangles, original geometry or logical-cluster
offsets used to investigate the boundaries. The discrepancy remains open.
Neither the historical receipt nor this patch establishes its cause.

The proposal adds those diagnostic inputs to each run's JSON. It does not
change the partition rule, padding, glyph positions, replay operations,
corpus, backend selection, output ownership or maximum delta gates.
No library, scene renderer, glyph cache, GPU atlas or platform backend changes.

## Additional report fields

`coverage_audit.version` is 1. The run still contains the existing `glyphs`
and `ink_group_sizes`; the following fields are additive:

| Field | Meaning |
| --- | --- |
| `coverage_audit.run_origin_surface` | Absolute run origin used for replay, in the diagnostic's logical-1x surface coordinates. |
| `coverage_audit.logical_rect_pango` | Pango run logical rectangle, as x/y/width/height. |
| `coverage_audit.baseline_pango` | Baseline returned by the layout iterator. |
| `coverage_audit.padding_pango` | Existing expansion per side, 1024 Pango units. |
| `coverage_audit.groups[].glyph_start` / `glyph_end` | Half-open indices into this run's original glyph sequence. |
| `coverage_audit.groups[].padded_run_ink_union_pango` | Bounding union of the group's padded font-metric rectangles, in left/top/right/bottom form relative to the run origin. |
| `coverage_audit.groups[].starts_inside_log_cluster` | Whether the boundary separates adjacent glyphs with the same original logical-cluster offset. |
| `glyphs[].advance_before_pango` | Accumulated pen advance before the original glyph. |
| `glyphs[].geometry_pango` | Original width and x/y offsets, before temporary replay adjustments. |
| `glyphs[].log_cluster` | Original Pango logical-cluster offset; not a Unicode grapheme index. |
| `glyphs[].font_ink_rect_pango` | Font ink rectangle before placement/padding, as x/y/width/height. |
| `glyphs[].padded_run_ink_pango` | The exact left/top/right/bottom rectangle used by the existing partition decision. |

Pango's [geometry contract](https://docs.gtk.org/Pango/struct.GlyphGeometry.html)
places a glyph at pen plus both offsets, then advances by width. This
proposal keeps that calculation. Its [font ink extents](https://docs.gtk.org/Pango/method.Font.get_glyph_extents.html)
use baseline-relative coordinates increasing right and down. See also the
[logical-cluster storage](https://docs.gtk.org/Pango/struct.GlyphString.html)
and [cluster-start attribute](https://docs.gtk.org/Pango/struct.GlyphVisAttr.html).

Each case also gains `group_delta_audit` version 1 with separate black/white
comparisons using the already captured group/native RGB buffers. These
record maximum channel delta, changed-pixel count, count exceeding the
existing threshold of 2 (`comparison_threshold`), and the changed pixels' half-open bounding box in
surface coordinates. Up to sixteen pixels at the maximum delta retain their
coordinates and both RGB values; `peak_pixel_count` records the full count.
No difference yields zero counts, an empty peak list and a null bounds field.
This adds localization without new drawing or changing any pass condition.

The font rectangle and its union are **not measured raster masks**. A false
`starts_inside_log_cluster` value does not prove equivalent compositing,
correct grapheme handling or disjoint pixel coverage. Font-metric overlap
and actual antialiased coverage still require independent comparison.

## Lifetime and scope

Original glyph and logical-cluster arrays are borrowed only while the owned
layout remains alive. Empty arrays use empty Rust slices without forming a
slice from a potentially NULL pointer. A nonempty run with NULL storage is
rejected before either array is borrowed. This is a source-level FFI
precondition, not a new runtime safety result.

The extra vectors and JSON belong only to the diagnostic; they are not a
bounded product cache and establish no efficiency improvement. Existing
native references, black/white comparisons and strict partition gates remain
required. After authorization and fixture review, an actual new report must
first establish whether the metadata matches rendering before using it to
choose a coverage fix. Then the full native-font corpus, atlas/tile alpha,
phase identities, budgets, DPI and platform gates remain outstanding.
