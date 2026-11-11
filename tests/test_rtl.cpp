// Right-to-left mirroring without a display: geometry, hit testing and
// keys of Collection grids, SplitPane and Sidebar while fo::rtl().
#include "harness.h"

#include "fltk-omarchy/collection.h"
#include "fltk-omarchy/i18n.h"
#include "fltk-omarchy/sidebar.h"
#include "fltk-omarchy/widgets.h"

#include <FL/Fl.H>
#include <FL/Fl_Box.H>

#include <cstring>

using namespace fo;

namespace {

struct Rtl {
  Rtl() { i18n::set_language("ar"); }
  ~Rtl() { i18n::set_language(""); }
};

class Tiles : public Collection {
public:
  Tiles(int w, int h) : Collection(0, 0, w, h) {}
  int count() const override { return 10; }
  void draw_item(int, int, int, int, int, const ItemState &) override {}
};

void key(Fl_Widget *w, int k) {
  Fl::e_keysym = k;
  Fl::e_state = 0;
  Fl::e_text = const_cast<char *>("");
  Fl::e_length = 0;
  w->handle(FL_KEYBOARD);
}

} // namespace

TEST(rtl_collection_grid_flows_from_the_right) {
  Rtl rtl;
  CHECK(fo::rtl());
  Tiles c(24 * 2 + 172 * 3 + 10, 400); // 3 columns
  c.layout(Collection::Layout::Grid);
  c.reload(true);
  int x0, y0, w0, h0, x1, y1, w1, h1;
  CHECK(c.item_rect(0, x0, y0, w0, h0));
  CHECK(c.item_rect(1, x1, y1, w1, h1));
  CHECK(x0 > x1); // the first tile is at the right
  CHECK_EQ(y0, y1);
  CHECK_EQ(c.item_at(x0 + 5, y0 + 5), 0);
  CHECK_EQ(c.item_at(x1 + 5, y1 + 5), 1);
  // Left moves forward in reading order, Right back.
  key(&c, FL_Left);
  CHECK_EQ(c.current(), 1);
  key(&c, FL_Left);
  key(&c, FL_Right);
  CHECK_EQ(c.current(), 1);
  key(&c, FL_Down);
  CHECK_EQ(c.current(), 4);
}

TEST(rtl_split_pane_puts_the_first_pane_on_the_right) {
  Rtl rtl;
  SplitPane s(0, 0, 600, 300);
  auto *a = new Fl_Box(0, 0, 10, 10);
  auto *b = new Fl_Box(0, 0, 10, 10);
  s.end();
  s.set_split(200);
  CHECK_EQ(a->x(), 400);
  CHECK_EQ(a->w(), 200);
  CHECK_EQ(b->x(), 0);
  CHECK_EQ(b->w(), 399);
  // Dragging the divider (at x 399) to the left grows the first pane.
  Fl::e_x = 399;
  Fl::e_y = 100;
  Fl::e_keysym = FL_Button + FL_LEFT_MOUSE;
  CHECK_EQ(s.handle(FL_PUSH), 1);
  Fl::e_x = 299;
  s.handle(FL_DRAG);
  s.handle(FL_RELEASE);
  CHECK_EQ(s.split(), 300);
  CHECK_EQ(a->x(), 300);
  // The second pane keeps its size when the pane grows.
  s.resize(0, 0, 800, 300);
  CHECK_EQ(b->w(), 299);
  CHECK_EQ(a->x(), 300);
  CHECK_EQ(a->w(), 500);
}

TEST(rtl_sidebar_eject_and_keys) {
  Rtl rtl;
  Sidebar s(0, 0, 240, 400);
  s.add_section("Devices");
  SidebarRow usb{"KINGSTON", "usb", "/run/media/me/KINGSTON"};
  usb.ejectable = true;
  s.add_row(usb);
  std::string ejected, opened;
  bool finished = false;
  s.on_eject([&](const SidebarRow &r) { ejected = r.id; });
  s.on_activate([&](const SidebarRow &r) { opened = r.id; });
  s.on_finished([&] { finished = true; });
  // The eject button is at the row's left end now: find the row's height
  // by scanning, clicking near the left edge.
  for (int y = 0; y < 200 && ejected.empty() && opened.empty(); y += 4) {
    Fl::e_x = 20;
    Fl::e_y = y;
    Fl::e_keysym = FL_Button + FL_LEFT_MOUSE;
    s.handle(FL_PUSH);
  }
  CHECK_STR(ejected, "/run/media/me/KINGSTON");
  CHECK(opened.empty());
  // Toward the files is Left: it opens the row and hands the keyboard back.
  Fl::e_keysym = FL_Right;
  CHECK_EQ(s.handle(FL_KEYBOARD), 0);
  key(&s, FL_Left);
  CHECK_STR(opened, "/run/media/me/KINGSTON");
  CHECK(finished);
}
