// ScrollArea and ScrollBar on both axes, without a display (synthetic
// wheel and mouse events; nothing is drawn).
#include "harness.h"

#include "fltk-omarchy/anim.h"
#include "fltk-omarchy/scroll.h"

#include <FL/Fl.H>
#include <FL/Fl_Box.H>
#include <FL/Fl_Group.H>

using namespace fo;

namespace {

int wheel(Fl_Widget *w, int dx, int dy, int ex, int ey, int state = 0) {
  Fl::e_x = ex;
  Fl::e_y = ey;
  Fl::e_dx = dx;
  Fl::e_dy = dy;
  Fl::e_state = state;
  int r = w->handle(FL_MOUSEWHEEL);
  Fl::e_dx = Fl::e_dy = 0;
  Fl::e_state = 0;
  return r;
}

} // namespace

TEST(scroll_area_vertical_by_default_follows_width) {
  anim::set_enabled(0);
  ScrollArea a(0, 0, 300, 200);
  auto *content = new Fl_Group(0, 0, 100, 1000);
  content->end();
  a.end();
  a.resize(0, 0, 320, 200);
  CHECK_EQ(content->w(), 320); // follows the width
  CHECK_EQ(content->h(), 1000);
  CHECK_NEAR(a.max_offset(), 800, 0.1);
  CHECK_NEAR(a.max_offset_x(), 0, 0.1);
  CHECK_EQ(wheel(&a, 0, 2, 10, 10), 1);
  CHECK_NEAR(a.target_offset(), 96, 0.1);
  // Horizontal steps have nowhere to go: passed on.
  CHECK_EQ(wheel(&a, 3, 0, 10, 10), 0);
  a.scroll_to(5000, false);
  CHECK_NEAR(a.offset(), 800, 0.1);
  CHECK_EQ(content->y(), -800);
  anim::set_enabled(-1);
}

TEST(scroll_area_horizontal) {
  anim::set_enabled(0);
  ScrollArea a(0, 0, 300, 200);
  auto *strip = new Fl_Group(0, 0, 1200, 50);
  strip->end();
  a.end();
  a.axes(ScrollArea::Horizontal);
  CHECK_EQ(strip->h(), 200); // follows the height
  CHECK_EQ(strip->w(), 1200);
  CHECK_NEAR(a.max_offset(), 0, 0.1);
  CHECK_NEAR(a.max_offset_x(), 900, 0.1);
  // A vertical step scrolls sideways in a Horizontal-only area.
  CHECK_EQ(wheel(&a, 0, 1, 10, 10), 1);
  CHECK_NEAR(a.target_offset_x(), 48, 0.1);
  CHECK_EQ(wheel(&a, 2, 0, 10, 10), 1);
  CHECK_NEAR(a.target_offset_x(), 144, 0.1);
  a.ensure_visible_x(1000, 100, false);
  CHECK_NEAR(a.offset_x(), 800, 0.1);
  CHECK_EQ(strip->x(), -800);
  a.ensure_visible_x(100, 50, false);
  CHECK_NEAR(a.offset_x(), 100, 0.1);
  // Wider than the view: its start wins.
  a.ensure_visible_x(400, 500, false);
  CHECK_NEAR(a.offset_x(), 400, 0.1);
  anim::set_enabled(-1);
}

TEST(scroll_area_both_axes_and_shift_wheel) {
  anim::set_enabled(0);
  ScrollArea a(0, 0, 300, 200);
  auto *content = new Fl_Group(0, 0, 900, 700);
  content->end();
  a.end();
  a.axes(ScrollArea::Both);
  a.resize(0, 0, 300, 200);
  CHECK_EQ(content->w(), 900); // keeps its own size
  CHECK_EQ(content->h(), 700);
  CHECK_EQ(wheel(&a, 0, 1, 10, 10), 1);
  CHECK_NEAR(a.target_offset(), 48, 0.1);
  CHECK_NEAR(a.target_offset_x(), 0, 0.1);
  CHECK_EQ(wheel(&a, 0, 1, 10, 10, FL_SHIFT), 1); // Shift turns it sideways
  CHECK_NEAR(a.target_offset(), 48, 0.1);
  CHECK_NEAR(a.target_offset_x(), 48, 0.1);
  a.scroll_to(100, false);
  a.scroll_to_x(250, false);
  CHECK_EQ(content->x(), -250);
  CHECK_EQ(content->y(), -100);
  // Shrinking the content clamps the offsets.
  content->size(400, 250);
  a.resize(0, 0, 300, 200);
  CHECK_NEAR(a.offset(), 50, 0.1);
  CHECK_NEAR(a.offset_x(), 100, 0.1);
  anim::set_enabled(-1);
}

TEST(scroll_area_nested_scroller_gets_the_wheel_first) {
  anim::set_enabled(0);
  ScrollArea outer(0, 0, 300, 200);
  auto *strip = new Fl_Group(0, 0, 1200, 200);
  auto *inner = new ScrollArea(0, 0, 250, 200);
  auto *col = new Fl_Group(0, 0, 250, 900);
  col->end();
  inner->end();
  strip->end();
  outer.end();
  outer.axes(ScrollArea::Horizontal);
  // Over the inner column: it scrolls vertically, the strip stays.
  CHECK_EQ(wheel(&outer, 0, 1, 20, 20), 1);
  CHECK_NEAR(inner->target_offset(), 48, 0.1);
  CHECK_NEAR(outer.target_offset_x(), 0, 0.1);
  // Past the column, the strip takes it.
  CHECK_EQ(wheel(&outer, 0, 1, 280, 20), 1);
  CHECK_NEAR(outer.target_offset_x(), 48, 0.1);
  anim::set_enabled(-1);
}

TEST(scroll_bar_horizontal_drag) {
  anim::set_enabled(0);
  Fl_Box owner(0, 0, 400, 100);
  ScrollBar bar(ScrollBar::Horizontal);
  CHECK(bar.orientation() == ScrollBar::Horizontal);
  bar.place(0, 90, 400, 10);
  double pos = 0, max = 1200, view = 400;
  // Thumb: 400 * 400 / 1600 = 100 px at the left. Grab its middle.
  Fl::e_x = 52;
  Fl::e_y = 95;
  Fl::e_keysym = FL_Button + FL_LEFT_MOUSE;
  CHECK(bar.handle(FL_PUSH, &owner, pos, max, view));
  CHECK(bar.dragging());
  CHECK_NEAR(pos, 0, 0.5);
  Fl::e_x = 52 + 148; // half of the free track (400 - 4 - 100 = 296)
  CHECK(bar.handle(FL_DRAG, &owner, pos, max, view));
  CHECK_NEAR(pos, 600, 5);
  CHECK(bar.handle(FL_RELEASE, &owner, pos, max, view));
  // A press outside the strip is not the bar's.
  Fl::e_y = 40;
  CHECK(!bar.handle(FL_PUSH, &owner, pos, max, view));
  anim::set_enabled(-1);
}
