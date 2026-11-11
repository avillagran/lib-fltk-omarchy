// Drawing primitives in FLTK coordinates: translucent fills, rounded
// rectangles and frames, hairlines, opacity groups.
//
// FLTK draws through cairo on Wayland (and on X11 when built with cairo);
// these functions use that same context for alpha and anti-aliased
// corners. Without cairo they fall back to opaque FLTK primitives, with
// translucent colors pre-mixed over `backdrop` (the theme background by
// default).
#pragma once

#include "fltk-omarchy/theme.h"

namespace fo {
namespace paint {

/// True when translucency and opacity groups are really composited.
bool has_alpha();

/// Fills a rectangle; `alpha` < 1 blends over what is drawn.
void fill(int x, int y, int w, int h, Rgb c, double alpha = 1.0);
/// Fills a rectangle with rounded corners (radius 0 = square).
void round_fill(int x, int y, int w, int h, int radius, Rgb c, double alpha = 1.0);
/// A `thickness`-wide frame just inside the rectangle.
void round_frame(int x, int y, int w, int h, int radius, int thickness, Rgb c, double alpha = 1.0);
/// A 1-unit horizontal / vertical hairline.
void hline(int x, int y, int w, Rgb c, double alpha = 1.0);
void vline(int x, int y, int h, Rgb c, double alpha = 1.0);
/// An anti-aliased stroked polyline segment.
void line(double x1, double y1, double x2, double y2, double width, Rgb c, double alpha = 1.0);
/// A filled circle.
void circle(double cx, double cy, double r, Rgb c, double alpha = 1.0);

/// Starts drawing into an offscreen group; pop_opacity() composites it
/// with `alpha`. Used to fade cards in and out. Pairs must nest.
void push_opacity();
void pop_opacity(double alpha);

/// The backdrop used to pre-mix translucent colors without cairo.
void set_backdrop(Rgb c);
Rgb backdrop();

} // namespace paint
} // namespace fo
