// Widget logic without a display: geometry, selection and keyboard,
// driven by synthetic FLTK events (nothing is drawn).
#include "harness.h"

#include <cstring>

#include "fltk-omarchy/collection.h"
#include "fltk-omarchy/dialog.h"
#include "fltk-omarchy/sidebar.h"
#include "fltk-omarchy/widgets.h"
#include "fltk-omarchy/window.h"

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

TEST(sidebar_click_hands_the_keyboard_back) {
  Sidebar s(0, 0, 240, 400);
  s.add_section("Home", "/home/me");
  s.add_row({"Documents", "file-text", "/home/me/Documents"});
  s.add_row({"Pictures", "image", "/home/me/Pictures"});
  std::string opened;
  int finished = 0;
  s.on_activate([&](const SidebarRow &r) { opened = r.id; });
  s.on_finished([&] { ++finished; });
  // Find a point on the second row: scan down until it opens.
  for (int y = 0; y < 400 && opened.empty(); y += 4) {
    Fl::e_x = 40;
    Fl::e_y = y;
    Fl::e_keysym = FL_Button + FL_LEFT_MOUSE;
    Fl::e_state = FL_BUTTON1;
    s.handle(FL_PUSH);
    if (!opened.empty() && opened != "/home/me/Pictures")
      opened.clear(), finished = 0;
  }
  Fl::e_state = 0;
  CHECK_STR(opened, "/home/me/Pictures");
  CHECK_EQ(finished, 1);
}

TEST(dialog_ask_returns_fields_button_and_check) {
  Window win(800, 600);
  win.end();
  DialogSpec spec;
  spec.title = "Sign in";
  spec.fields = {{"User", "me"}, {"Password", "", true}};
  spec.check = "Remember";
  spec.buttons = {"Cancel", "Skip", "Connect"};
  DialogAnswer got;
  int calls = 0;
  Dialog *d = Dialog::ask(&win, spec, [&](const DialogAnswer &a) {
    got = a;
    ++calls;
  });
  CHECK(d != nullptr);
  CHECK(win.modal_open());
  // The fields are the dialog's first children, in order.
  auto *pw = dynamic_cast<TextField *>(d->child(1));
  CHECK(pw != nullptr);
  pw->value("secret");
  auto *check = dynamic_cast<CheckBox *>(d->child(2));
  CHECK(check != nullptr);
  check->checked(true);
  d->choose(1);
  CHECK_EQ(calls, 1);
  CHECK_EQ(got.button, 1);
  CHECK_EQ(got.values.size(), size_t(2));
  CHECK_STR(got.values[0], "me");
  CHECK_STR(got.values[1], "secret");
  CHECK(got.checked);
  CHECK(!win.modal_open());
}

TEST(dialog_ask_escape_cancels_and_primary_validates) {
  Window win(800, 600);
  win.end();
  DialogAnswer got;
  got.button = 99;
  Dialog *d = Dialog::ask(&win, DialogSpec(), [&](const DialogAnswer &a) { got = a; });
  key(d, FL_Escape);
  CHECK_EQ(got.button, -1);

  bool answered = false;
  Dialog *p = Dialog::prompt(&win, "Rename", "", "a.txt", "Rename", [&](bool, const std::string &) { answered = true; });
  p->on_validate([](const std::string &s) { return s != "a.txt"; });
  p->accept(); // refused: the dialog stays
  CHECK(!answered);
  CHECK(win.modal_open());
  p->cancel();
  CHECK(answered);
}

TEST(window_run_shortcut_matches_the_current_key) {
  Window win(400, 300);
  win.end();
  int quit = 0;
  win.shortcut("q", [&] { ++quit; });
  Fl::e_keysym = 'q';
  Fl::e_state = 0;
  Fl::e_text = const_cast<char *>("q");
  Fl::e_length = 1;
  CHECK(win.run_shortcut());
  CHECK_EQ(quit, 1);
  Fl::e_keysym = 'w';
  Fl::e_text = const_cast<char *>("w");
  CHECK(!win.run_shortcut());
  CHECK_EQ(quit, 1);
}

TEST(window_remembers_the_focus_to_give_back) {
  // Giving the focus back needs a display (FLTK asks the window system
  // for the window's focus); the bookkeeping does not.
  Window win(400, 300);
  CheckBox first(0, 0, 200, 24, "First");
  auto *second = new CheckBox(0, 40, 200, 24, "Second");
  win.end();
  CheckBox outside(0, 0, 200, 24, "Outside");
  CHECK(win.remembered_focus() == nullptr);
  win.remember_focus(second);
  CHECK(win.remembered_focus() == second);
  win.remember_focus(&outside); // not in this window: ignored
  win.remember_focus(&win);
  CHECK(win.remembered_focus() == second);
  delete second; // a deleted widget is forgotten
  CHECK(win.remembered_focus() == nullptr);
  win.remember_focus(&first);
  CHECK(win.remembered_focus() == &first);
}

TEST(checkbox_toggles_from_the_keyboard) {
  CheckBox c(0, 0, 200, 24, "Apply to all");
  bool seen = false;
  c.on_change([&](bool on) { seen = on; });
  CHECK(!c.checked());
  Fl::focus(&c);
  key(&c, FL_Enter);
  CHECK(c.checked());
  CHECK(seen);
}

TEST(collection_empty_keeps_the_arrows) {
  Names c(400, 200);
  c.names.clear();
  c.reload(true);
  Fl::e_keysym = FL_Down;
  Fl::e_state = 0;
  Fl::e_text = const_cast<char *>("");
  Fl::e_length = 0;
  CHECK_EQ(c.handle(FL_KEYBOARD), 1); // FLTK would move the focus otherwise
  CHECK_EQ(c.current(), -1);
  Fl::e_keysym = FL_Left;
  CHECK_EQ(c.handle(FL_KEYBOARD), 0); // the list layout leaves Left to the app
  Fl::e_keysym = FL_Enter;
  CHECK_EQ(c.handle(FL_KEYBOARD), 0);
}

TEST(collection_no_cursor) {
  Names c(400, 200);
  c.reload(true);
  c.set_current(3);
  c.set_current(-1, true);
  CHECK_EQ(c.current(), -1);
  CHECK_EQ(c.selection_count(), 0);
  key(&c, FL_Down); // the first arrow lands on the first item
  CHECK_EQ(c.current(), 0);
  CHECK(c.selected(0));
}
