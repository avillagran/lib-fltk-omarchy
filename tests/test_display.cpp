// Paths that need a real display: what reaches the screen after a
// minimal update, focus rings, and the keyboard coming back after the
// window lost the window system's focus. ctest runs this binary inside a
// private Xvfb (tests/run-xvfb.sh); without DISPLAY every test skips.
#include "harness.h"
#include "display.h"

#include "fltk-omarchy/anim.h"
#include "fltk-omarchy/app.h"
#include "fltk-omarchy/collection.h"
#include "fltk-omarchy/theme.h"
#include "fltk-omarchy/widgets.h"
#include "fltk-omarchy/window.h"

#include <FL/Fl.H>
#include <FL/Fl_Input.H>
#include <FL/Fl_Window.H>
#include <FL/fl_draw.H>

#include <cstring>

using namespace fo;

namespace {

class Rows : public Collection {
public:
  Rows(int x, int y, int w, int h) : Collection(x, y, w, h) {}
  int n = 8;
  int count() const override { return n; }
  std::string item_text(int i) const override { return "item " + std::to_string(i); }
  void draw_item(int, int, int, int, int, const ItemState &) override {} // only the ring shows
};

void send_key(Fl_Widget *w, int k, const char *text = "") {
  Fl::e_keysym = k;
  Fl::e_state = 0;
  Fl::e_text = const_cast<char *>(text);
  Fl::e_length = int(std::strlen(text));
  w->handle(FL_KEYBOARD);
}

// Bright text pixels in a rectangle of the window.
int bright_pixels(Fl_Window *win, int x, int y, int w, int h, Rgb text) {
  int n = 0;
  for (int yy = y; yy < y + h; ++yy)
    for (int xx = x; xx < x + w; ++xx)
      if (fo_test::near(fo_test::pixel(win, xx, yy), text, 90))
        ++n;
  return n;
}

} // namespace

TEST(display_textfield_minimal_update_erases_old_text) {
  if (!fo_test::have_display())
    return;
  anim::set_enabled(0);
  Window win(420, 80);
  auto *field = new TextField(20, 20, 380, 38);
  win.end();
  fo_test::show(&win);
  field->take_focus();
  field->value("WWWWWWWWWW");
  field->insert_position(field->size());
  win.redraw();
  fo_test::settle();
  Rgb text = theme().bright_foreground();
  fl_font(field->textfont(), field->textsize());
  int start = field->x() + 12;
  int full = int(fl_width("WWWWWWWWWW"));
  int kept = int(fl_width("WWWWW"));
  CHECK(bright_pixels(&win, start + kept + 6, field->y() + 8, full - kept - 6, field->h() - 16, text) > 20);
  // Deleting with the keyboard redraws only what changed (a minimal update).
  for (int i = 0; i < 5; ++i)
    send_key(field, FL_BackSpace);
  fo_test::settle();
  CHECK_STR(field->value(), "WWWWW");
  CHECK_EQ(bright_pixels(&win, start + kept + 6, field->y() + 8, full - kept - 6, field->h() - 16, text), 0);

  // Sensitivity: without a box Fl_Input_ erases nothing and the deleted
  // glyphs stay on screen, which is the regression this test guards.
  field->box(FL_NO_BOX);
  field->value("WWWWWWWWWW");
  field->insert_position(field->size());
  win.redraw();
  fo_test::settle();
  for (int i = 0; i < 5; ++i)
    send_key(field, FL_BackSpace);
  fo_test::settle();
  CHECK(bright_pixels(&win, start + kept + 6, field->y() + 8, full - kept - 6, field->h() - 16, text) > 20);
  win.hide();
  anim::set_enabled(-1);
}

TEST(display_collection_focus_ring_on_selected) {
  if (!fo_test::have_display())
    return;
  anim::set_enabled(0);
  Window win(400, 300);
  auto *c = new Rows(0, 0, 400, 300);
  win.end();
  fo_test::show(&win);
  c->reload(true);
  c->take_focus();
  set_focus_visible(true);
  c->set_current(0); // selected cursor
  Rgb accent = theme().accent(), bg = theme().background();
  // Item 0 sits at (12, 12, 376, 32) in a list: the ring is its outline
  // (the items draw nothing). A 1 px frame may be blended with the
  // background, so look for an accent-tinted pixel: warmer than the
  // background, the way the yellow accent is.
  auto ring_at_top = [&] {
    win.redraw();
    fo_test::settle();
    for (int y = 10; y <= 14; ++y)
      if (fo_test::tinted(fo_test::pixel(&win, 200, y), bg, accent))
        return true;
    return false;
  };
  CHECK(Fl::focus() == c);
  CHECK(ring_at_top()); // default: the ring shows on a selected cursor
  c->focus_ring_on_selected(false);
  CHECK(!ring_at_top()); // the item shows its own selection
  c->select(0, false);
  CHECK(ring_at_top()); // an unselected cursor still gets the ring
  set_focus_visible(false);
  c->select(0, true);
  c->focus_ring_on_selected(true);
  CHECK(!ring_at_top()); // no ring while the mouse is in use
  win.hide();
  anim::set_enabled(-1);
}

TEST(display_window_restores_focus_after_losing_it) {
  if (!fo_test::have_display())
    return;
  if (!fo_test::have_xtest()) {
    std::printf("  skip: XTest is not available\n");
    return;
  }
  anim::set_enabled(0);
  Window a(10, 10, 400, 300, "a");
  auto *first = new CheckBox(10, 10, 200, 24, "First"); // a sidebar-like first child
  auto *files = new Rows(0, 40, 400, 260);
  a.end();
  Fl_Window b(500, 10, 200, 100, "b");
  new Fl_Input(10, 10, 180, 30);
  b.end();
  fo_test::show(&a);
  fo_test::show(&b);
  files->reload(true);
  fo_test::focus_window(&a);
  files->take_focus();
  fo_test::settle();
  CHECK(Fl::focus() == files);
  // The last key FLTK saw is an arrow: when the focus comes back, an
  // Fl_Group then skips its saved focus and picks its first child.
  fo_test::press_key(FL_Down);
  CHECK_EQ(files->current(), 1);
  // Another window takes the window system's focus...
  fo_test::focus_window(&b);
  CHECK(!a.contains(Fl::focus()));
  // ... and gives it back; the next arrow goes to the files, not to the
  // window's first child.
  fo_test::focus_window(&a);
  fo_test::press_key(FL_Down);
  CHECK(Fl::focus() == files);
  CHECK(Fl::focus() != first);
  CHECK_EQ(files->current(), 2);
  fo_test::press_key(FL_Down);
  CHECK_EQ(files->current(), 3);

  // Wayland compositors (and some X window managers) give the focus back
  // with the key that brought it: FL_FOCUS arrives while FLTK's last key
  // is an arrow, so a plain Fl_Group would pick its first child.
  Fl::focus(nullptr); // the window lost the focus
  Fl::e_keysym = FL_Down;
  Fl::e_state = 0;
  Fl::handle(FL_FOCUS, &a);
  CHECK(Fl::focus() == files);
  // A key that reaches the window while nothing in it has the keyboard
  // goes to the widget that had it.
  Fl::focus(nullptr);
  Fl::e_keysym = FL_Down;
  Fl::e_text = const_cast<char *>("");
  Fl::e_length = 0;
  Fl::handle(FL_KEYDOWN, &a);
  CHECK(Fl::focus() == files);
  CHECK_EQ(files->current(), 4);
  b.hide();
  a.hide();
  anim::set_enabled(-1);
}
