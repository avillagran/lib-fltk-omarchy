#include "fltk-omarchy/paint.h"

#include <FL/Fl.H>
#include <FL/Fl_Graphics_Driver.H>
#include <FL/fl_draw.H>

#include <cmath>

#if defined(FO_HAVE_CAIRO) && FLTK_USE_CAIRO
#include <cairo.h>
#define FO_CAIRO 1
#endif

namespace fo {
namespace paint {

namespace {
bool g_has_backdrop = false;
Rgb g_backdrop;
int g_groups = 0; // opacity groups pushed without cairo (no-ops)

#ifdef FO_CAIRO
cairo_t *cr() {
  // With FLTK_USE_CAIRO every display driver (Wayland, X11-cairo) and the
  // image surface draw into a cairo context, returned by gc().
  return static_cast<cairo_t *>(fl_graphics_driver->gc());
}
void source(cairo_t *c, Rgb col, double a) {
  cairo_set_source_rgba(c, col.r / 255.0, col.g / 255.0, col.b / 255.0, a);
}
void round_path(cairo_t *c, double x, double y, double w, double h, double r) {
  if (r <= 0) {
    cairo_rectangle(c, x, y, w, h);
    return;
  }
  r = std::min(r, std::min(w, h) / 2);
  cairo_new_sub_path(c);
  cairo_arc(c, x + w - r, y + r, r, -M_PI / 2, 0);
  cairo_arc(c, x + w - r, y + h - r, r, 0, M_PI / 2);
  cairo_arc(c, x + r, y + h - r, r, M_PI / 2, M_PI);
  cairo_arc(c, x + r, y + r, r, M_PI, 3 * M_PI / 2);
  cairo_close_path(c);
}
#endif

Rgb flat(Rgb c, double alpha) { return alpha >= 1 ? c : mix(backdrop(), c, alpha); }

void fl_round_fill(int x, int y, int w, int h, int r, Rgb c) {
  fl_color(c.fl());
  if (r <= 0) {
    fl_rectf(x, y, w, h);
    return;
  }
  r = std::min(r, std::min(w, h) / 2);
  fl_rectf(x + r, y, w - 2 * r, h);
  fl_rectf(x, y + r, r, h - 2 * r);
  fl_rectf(x + w - r, y + r, r, h - 2 * r);
  fl_pie(x, y, 2 * r, 2 * r, 90, 180);
  fl_pie(x + w - 2 * r, y, 2 * r, 2 * r, 0, 90);
  fl_pie(x, y + h - 2 * r, 2 * r, 2 * r, 180, 270);
  fl_pie(x + w - 2 * r, y + h - 2 * r, 2 * r, 2 * r, 270, 360);
}
} // namespace

void set_backdrop(Rgb c) {
  g_backdrop = c;
  g_has_backdrop = true;
}
Rgb backdrop() { return g_has_backdrop ? g_backdrop : theme().background(); }

bool has_alpha() {
#ifdef FO_CAIRO
  return fl_graphics_driver && fl_graphics_driver->gc() != nullptr;
#else
  return false;
#endif
}

void fill(int x, int y, int w, int h, Rgb c, double alpha) {
  if (w <= 0 || h <= 0 || alpha <= 0)
    return;
#ifdef FO_CAIRO
  if (alpha < 1 && has_alpha()) {
    cairo_t *c2 = cr();
    cairo_save(c2);
    source(c2, c, alpha);
    cairo_rectangle(c2, x, y, w, h);
    cairo_fill(c2);
    cairo_restore(c2);
    return;
  }
#endif
  fl_color(flat(c, alpha).fl());
  fl_rectf(x, y, w, h);
}

void round_fill(int x, int y, int w, int h, int radius, Rgb c, double alpha) {
  if (w <= 0 || h <= 0 || alpha <= 0)
    return;
  if (radius <= 0)
    return fill(x, y, w, h, c, alpha);
#ifdef FO_CAIRO
  if (has_alpha()) {
    cairo_t *c2 = cr();
    cairo_save(c2);
    source(c2, c, alpha);
    round_path(c2, x, y, w, h, radius);
    cairo_fill(c2);
    cairo_restore(c2);
    return;
  }
#endif
  fl_round_fill(x, y, w, h, radius, flat(c, alpha));
}

void round_frame(int x, int y, int w, int h, int radius, int t, Rgb c, double alpha) {
  if (w <= 0 || h <= 0 || alpha <= 0 || t <= 0)
    return;
#ifdef FO_CAIRO
  if (has_alpha()) {
    cairo_t *c2 = cr();
    cairo_save(c2);
    source(c2, c, alpha);
    cairo_set_fill_rule(c2, CAIRO_FILL_RULE_EVEN_ODD);
    round_path(c2, x, y, w, h, radius);
    round_path(c2, x + t, y + t, w - 2 * t, h - 2 * t, radius > t ? radius - t : 0);
    cairo_fill(c2);
    cairo_restore(c2);
    return;
  }
#endif
  Rgb f = flat(c, alpha);
  fl_color(f.fl());
  for (int i = 0; i < t; ++i)
    fl_rect(x + i, y + i, w - 2 * i, h - 2 * i);
}

void hline(int x, int y, int w, Rgb c, double alpha) { fill(x, y, w, 1, c, alpha); }
void vline(int x, int y, int h, Rgb c, double alpha) { fill(x, y, 1, h, c, alpha); }

void line(double x1, double y1, double x2, double y2, double width, Rgb c, double alpha) {
#ifdef FO_CAIRO
  if (has_alpha()) {
    cairo_t *c2 = cr();
    cairo_save(c2);
    source(c2, c, alpha);
    cairo_set_line_width(c2, width);
    cairo_set_line_cap(c2, CAIRO_LINE_CAP_ROUND);
    cairo_move_to(c2, x1, y1);
    cairo_line_to(c2, x2, y2);
    cairo_stroke(c2);
    cairo_restore(c2);
    return;
  }
#endif
  fl_color(flat(c, alpha).fl());
  fl_line_style(FL_SOLID, int(width + 0.5));
  fl_line(int(x1), int(y1), int(x2), int(y2));
  fl_line_style(0);
}

void circle(double cx, double cy, double r, Rgb c, double alpha) {
#ifdef FO_CAIRO
  if (has_alpha()) {
    cairo_t *c2 = cr();
    cairo_save(c2);
    source(c2, c, alpha);
    cairo_new_sub_path(c2);
    cairo_arc(c2, cx, cy, r, 0, 2 * M_PI);
    cairo_fill(c2);
    cairo_restore(c2);
    return;
  }
#endif
  fl_color(flat(c, alpha).fl());
  fl_pie(int(cx - r), int(cy - r), int(2 * r), int(2 * r), 0, 360);
}

void push_opacity() {
#ifdef FO_CAIRO
  if (has_alpha()) {
    cairo_push_group(cr());
    return;
  }
#endif
  ++g_groups;
}

void pop_opacity(double alpha) {
#ifdef FO_CAIRO
  if (has_alpha()) {
    cairo_t *c2 = cr();
    cairo_pop_group_to_source(c2);
    cairo_paint_with_alpha(c2, alpha < 0 ? 0 : alpha > 1 ? 1 : alpha);
    return;
  }
#endif
  (void)alpha;
  if (g_groups > 0)
    --g_groups;
}

} // namespace paint
} // namespace fo
