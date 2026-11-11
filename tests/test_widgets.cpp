// Widget logic without a display: geometry, selection and keyboard,
// driven by synthetic FLTK events (nothing is drawn).
#include "harness.h"

#include <cstring>

#include "fltk-omarchy/collection.h"
#include "fltk-omarchy/sidebar.h"

#include <FL/Fl.H>

using namespace fo;

namespace {

class Names : public Collection {
public:
  Names(int w, int h) : Collection(0, 0, w, h) {}
  std::vector<std::string> names = {"alpha", "beta", "bravo", "charlie", "delta", "echo", "foxtrot", "golf",
                                    "hotel", "india"};
  int count() const override { return int(names.size()); }
  std::string item_text(int i) const override { return names[size_t(i)]; }
  void draw_item(int, int, int, int, int, const ItemState &) override {}
};

void key(Fl_Widget *w, int k, int state = 0, const char *text = "") {
  Fl::e_keysym = k;
  Fl::e_state = state;
  Fl::e_text = const_cast<char *>(text);
  Fl::e_length = int(strlen(text));
  w->handle(FL_KEYBOARD);
  Fl::e_state = 0;
}

} // namespace

TEST(collection_list_geometry_and_keys) {
  Names c(400, 200);
  c.reload(true);
  CHECK_EQ(c.current(), 0);
  CHECK_EQ(c.columns(), 1);
  int x, y, w, h;
  CHECK(c.item_rect(2, x, y, w, h));
  CHECK_EQ(y, 12 + 2 * 32);
  CHECK_EQ(c.item_at(50, 12 + 2 * 32 + 5), 2);
  CHECK_EQ(c.item_at(2, 20), -1); // padding
  key(&c, FL_Down);
  key(&c, FL_Down);
  CHECK_EQ(c.current(), 2);
  CHECK(c.selected(2) && !c.selected(0));
  key(&c, FL_Down, FL_SHIFT);
  key(&c, FL_Down, FL_SHIFT);
  CHECK_EQ(c.selection_count(), 3); // 2..4
  CHECK(c.selected(4));
  key(&c, FL_End);
  CHECK_EQ(c.current(), 9);
  CHECK_EQ(c.selection_count(), 1);
  key(&c, 'a', FL_CTRL, "a");
  CHECK_EQ(c.selection_count(), 10);
  int activated = -1;
  c.on_activate([&](int i) { activated = i; });
  key(&c, FL_Enter);
  CHECK_EQ(activated, 9);
  // on_key sees keys first.
  c.on_key([](int k) { return k == FL_Delete; });
  key(&c, FL_Delete);
  CHECK_EQ(c.current(), 9);
}

TEST(collection_grid_navigation) {
  Names c(24 * 2 + 172 * 3 + 10, 400); // 3 columns
  c.layout(Collection::Layout::Grid);
  c.reload(true);
  CHECK_EQ(c.columns(), 3);
  key(&c, FL_Right);
  key(&c, FL_Down);
  CHECK_EQ(c.current(), 4);
  key(&c, FL_Up);
  key(&c, FL_Left);
  CHECK_EQ(c.current(), 0);
  key(&c, FL_Up); // clamps
  CHECK_EQ(c.current(), 0);
  CHECK_EQ(c.item_at(24 + 172 + 5, 24 + 196 + 5), 4);
}

TEST(collection_type_ahead) {
  Names c(400, 200);
  c.reload(true);
  key(&c, 'c', 0, "c");
  CHECK_EQ(c.current(), 3); // charlie
  key(&c, 'b', 0, "b"); // within the timeout the buffer is "cb": no match, stays
  CHECK_EQ(c.current(), 3);
  Names d(400, 200);
  d.reload(true);
  key(&d, 'b', 0, "b");
  CHECK_EQ(d.current(), 1); // beta
  key(&d, 'r', 0, "r");
  CHECK_EQ(d.current(), 2); // "br": bravo
}

TEST(collection_reload_keeps_selection_in_range) {
  Names c(400, 200);
  c.reload(true);
  c.set_current(9);
  c.names.resize(4);
  c.reload();
  CHECK_EQ(c.current(), 3);
  CHECK_EQ(c.selection_count(), 0);
  c.select(1, true);
  CHECK_EQ(c.selection(), std::vector<int>{1});
}

TEST(sidebar_keyboard_and_active) {
  Sidebar s(0, 0, 240, 400);
  s.add_section("Home", "/home/me");
  s.add_row({"Documents", "file-text", "/home/me/Documents"});
  s.add_row({"Pictures", "image", "/home/me/Pictures"});
  int pinned = s.add_section("Pinned");
  s.add_row({"Work", "folder", "/work"});
  s.add_section("Views");
  s.add_row({"Trash", "trash-2", "trash://"});
  s.set_active("/home/me/Pictures");
  CHECK_EQ(s.active(), 1);
  std::string opened;
  bool finished = false;
  s.on_activate([&](const SidebarRow &r) { opened = r.id; });
  s.on_finished([&] { finished = true; });
  s.set_cursor(1);
  s.section_visible(pinned, false);
  key(&s, FL_Down); // skips the hidden section
  CHECK_EQ(s.cursor(), 3);
  key(&s, FL_Enter);
  CHECK_STR(opened, "trash://");
  CHECK(finished);
  key(&s, FL_Home);
  CHECK_EQ(s.cursor(), 0);
  CHECK_EQ(s.rows()[2].section, pinned);
}
