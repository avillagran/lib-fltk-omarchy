// Paths that need a real display: what reaches the screen after a
// minimal update, focus rings, and the keyboard coming back after the
// window lost the window system's focus. ctest runs this binary inside a
// private Xvfb (tests/run-xvfb.sh); without DISPLAY every test skips.
#include "harness.h"
#include "display.h"

#include "fltk-omarchy/anim.h"
#include "fltk-omarchy/app.h"
#include "fltk-omarchy/collection.h"
#include "fltk-omarchy/dialog.h"
#include "fltk-omarchy/columns.h"
#include "fltk-omarchy/i18n.h"
#include "fltk-omarchy/menu.h"
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

namespace {

void move(PopupMenu *m, int x, int y) {
  Fl::e_x = x;
  Fl::e_y = y;
  Fl::e_state = 0;
  m->handle(FL_MOVE);
}

void center_of(PopupMenu *m, int card, int index, int &x, int &y) {
  int ix = 0, iy = 0, iw = 0, ih = 0;
  m->item_rect(card, index, ix, iy, iw, ih);
  x = ix + iw / 2;
  y = iy + ih / 2;
}

// Hovers entry 1 ("Sort by") until its submenu opens.
void open_sort(PopupMenu *m) {
  int x, y;
  center_of(m, 0, 1, x, y);
  move(m, x - 20, y);
  move(m, x, y);
  fo_test::settle((anim::ms::SUBMENU_OPEN + 100) / 1000.0);
}

} // namespace

TEST(display_menu_submenu_aim_grace) {
  if (!fo_test::have_display())
    return;
  Window win(800, 600);
  win.end();
  fo_test::show(&win);
  std::vector<MenuItem> items = {
      {"Open"},
      {"Sort by", "", nullptr, {{"Name"}, {"Size"}, {"Modified"}}},
      {"Rename"},
      {"Delete"},
      {"Properties"},
  };
  PopupMenu *m = PopupMenu::open_at(&win, 100, 100, items);
  open_sort(m);
  CHECK_EQ(m->depth(), 2);
  CHECK_EQ(m->highlighted(0), 1);
  int cx, cy, cw, ch, sx, sy, sw, sh;
  m->card_rect(0, cx, cy, cw, ch);
  m->card_rect(1, sx, sy, sw, sh);
  CHECK(sx > cx); // opened to the right
  int x1, y1, x2, y2;
  center_of(m, 0, 1, x1, y1);
  center_of(m, 0, 2, x2, y2);

  // Diagonally toward the submenu across "Rename": it stays open.
  move(m, cx + cw - 40, y1);
  move(m, cx + cw - 8, y2 - 10);
  CHECK_EQ(m->depth(), 2);
  CHECK_EQ(m->highlighted(0), 1);
  // Into the submenu: its entries take the pointer.
  int nx, ny;
  center_of(m, 1, 1, nx, ny);
  move(m, nx, ny);
  CHECK_EQ(m->depth(), 2);
  CHECK_EQ(m->highlighted(1), 1);
  CHECK_EQ(m->highlighted(0), 1);

  // Settling on "Rename" instead: after the grace it takes over.
  move(m, x1, y1);
  move(m, cx + cw - 40, y1);
  move(m, cx + cw - 8, y2 - 10);
  CHECK_EQ(m->depth(), 2);
  fo_test::settle((anim::ms::SUBMENU_AIM + 120) / 1000.0);
  CHECK_EQ(m->depth(), 1);
  CHECK_EQ(m->highlighted(0), 2);

  // Moving away from the submenu closes it at once.
  open_sort(m);
  CHECK_EQ(m->depth(), 2);
  move(m, cx + 30, y2);
  CHECK_EQ(m->depth(), 1);
  CHECK_EQ(m->highlighted(0), 2);

  // A key during the grace wins over the pointer.
  open_sort(m);
  move(m, cx + cw - 40, y1);
  move(m, cx + cw - 8, y2 - 10);
  send_key(m, FL_Escape); // closes the submenu
  fo_test::settle((anim::ms::SUBMENU_AIM + 120) / 1000.0);
  CHECK_EQ(m->depth(), 1);
  CHECK_EQ(m->highlighted(0), 1); // still on "Sort by", not "Rename"
  m->close();
  fo_test::settle();
  win.hide();
}

namespace {
int brightness(Rgb c) { return int(c.r) + int(c.g) + int(c.b); }
bool is_child(Window &w, Fl_Widget *c) { return w.find(c) < w.children(); }
} // namespace

TEST(display_layers_fade_out_on_close) {
  if (!fo_test::have_display())
    return;
  anim::set_enabled(1);
  anim::freeze();
  Window win(600, 400);
  win.end();
  fo_test::show(&win);
  Rgb bg = fo_test::pixel(&win, 5, 5);
  int answered = 0;
  Dialog *d = Dialog::confirm(&win, "Delete?", "It goes for good.", "Delete", [&](bool) { ++answered; });
  anim::advance(anim::ms::POPUP_IN + 20);
  win.redraw();
  fo_test::settle();
  Rgb dim = fo_test::pixel(&win, 5, 5); // the scrim, fully shown
  CHECK(brightness(dim) < brightness(bg) - 15);

  Fl_Widget_Tracker alive(d);
  d->cancel();
  CHECK_EQ(answered, 1);
  // Closed at once for the app: not modal, not on top, no events...
  CHECK(!win.modal_open());
  CHECK(win.top_layer() == nullptr);
  CHECK(d->closing_out());
  CHECK(!d->takesevents());
  CHECK(!d->is_open());
  // ... but still drawn while it fades out.
  CHECK(is_child(win, d));
  anim::advance(15);
  win.redraw();
  fo_test::settle();
  CHECK(!alive.deleted() && is_child(win, d));
  Rgb mid = fo_test::pixel(&win, 5, 5);
  CHECK(brightness(mid) > brightness(dim));
  CHECK(brightness(mid) < brightness(bg));
  // Done: removed and deleted, the window shows through again.
  anim::advance(anim::ms::POPUP_OUT);
  fo_test::settle(0.2);
  CHECK(alive.deleted());
  CHECK(fo_test::near(fo_test::pixel(&win, 5, 5), bg, 2));

  // Toasts and the help overlay fade out the same way.
  Toast *t = Toast::show(&win, "Copied", 0);
  Fl_Widget_Tracker toast(t);
  anim::advance(anim::ms::POPUP_IN + 20);
  fo_test::settle();
  t->close();
  CHECK(!toast.deleted() && t->closing_out());
  anim::advance(anim::ms::POPUP_OUT + 20);
  fo_test::settle(0.2);
  CHECK(toast.deleted());
  HelpOverlay *h = HelpOverlay::show(&win, {{"?", "Help"}});
  Fl_Widget_Tracker help(h);
  anim::advance(anim::ms::POPUP_IN + 20);
  fo_test::settle();
  send_key(h, FL_Escape);
  CHECK(!help.deleted() && h->closing_out());
  // Closing again while it fades is a no-op.
  h->close();
  anim::advance(anim::ms::POPUP_OUT + 20);
  fo_test::settle(0.2);
  CHECK(help.deleted());

  // A dialog closed while still fading in fades out from where it was.
  Dialog *q = Dialog::confirm(&win, "Again?", "", "Yes", nullptr);
  Fl_Widget_Tracker quick(q);
  anim::advance(10);
  q->cancel();
  CHECK(!quick.deleted());
  anim::advance(anim::ms::POPUP_OUT / 2);
  fo_test::settle(0.2);
  CHECK(quick.deleted());

  // With animations off they go at once.
  anim::set_enabled(0);
  Dialog *e = Dialog::confirm(&win, "Now?", "", "Yes", nullptr);
  e->cancel();
  CHECK(!is_child(win, e));
  fo_test::settle();
  anim::resume();
  anim::set_enabled(-1);
  win.hide();
}

TEST(display_columns_keyboard_and_clicks) {
  if (!fo_test::have_display())
    return;
  if (!fo_test::have_xtest()) {
    std::printf("  skip: XTest is not available\n");
    return;
  }
  anim::set_enabled(0);
  Window win(10, 10, 600, 300, "columns");
  auto *view = new ColumnView(0, 0, 600, 300);
  view->end();
  win.end();
  view->column_width(250);
  std::vector<Rows *> cols;
  for (int i = 0; i < 4; ++i) {
    cols.push_back(new Rows(0, 0, 10, 10));
    cols.back()->reload(true);
    view->add_column(cols.back());
  }
  fo_test::show(&win);
  fo_test::focus_window(&win);
  view->set_active(3);
  fo_test::settle();
  CHECK(Fl::focus() == cols[3]);
  CHECK_NEAR(view->offset_x(), 4 * 251 - 600, 0.5);
  // Real key presses: the list leaves Left to its parents, the view moves.
  fo_test::press_key(FL_Left);
  fo_test::press_key(FL_Left);
  CHECK(Fl::focus() == cols[1]);
  CHECK_EQ(view->active(), 1);
  // Column 1 starts at 251 in the strip: scrolled into view.
  CHECK(view->offset_x() <= 251 + 0.5);
  CHECK_EQ(cols[1]->x(), 251 - int(view->offset_x()));
  // Up and Down still move inside the column.
  fo_test::press_key(FL_Down);
  CHECK_EQ(cols[1]->current(), 1);
  // A click in another column makes it the active one.
  fo_test::click(&win, cols[2]->x() + 60, 30);
  CHECK(Fl::focus() == cols[2]);
  CHECK_EQ(view->active(), 2);
  win.hide();
  anim::set_enabled(-1);
}

namespace {
struct RtlScope {
  RtlScope() { i18n::set_language("ar"); }
  ~RtlScope() { i18n::set_language(""); }
};

class IconRows : public Collection {
public:
  IconRows(int x, int y, int w, int h) : Collection(x, y, w, h) {}
  int count() const override { return 3; }
  void draw_item(int, int x, int y, int w, int h, const ItemState &s) override {
    draw_row(x, y, w, h, s, "folder", "", "", theme().accent());
  }
};

// Non-background pixels in a column strip of a row.
int ink(Fl_Window *win, int x0, int x1, int y0, int y1) {
  Rgb bg = theme().background();
  int n = 0;
  for (int y = y0; y < y1; y += 2)
    for (int x = x0; x < x1; ++x)
      if (!fo_test::near(fo_test::pixel(win, x, y), bg, 12))
        ++n;
  return n;
}
} // namespace

TEST(display_rtl_rows_breadcrumbs_and_menus) {
  if (!fo_test::have_display())
    return;
  anim::set_enabled(0);
  RtlScope rtl;
  Window win(600, 400);
  auto *rows = new IconRows(0, 0, 600, 120);
  auto *crumbs = new Breadcrumbs(20, 140, 560, 40);
  win.end();
  fo_test::show(&win);
  rows->reload(true);
  rows->clear_selection();
  win.redraw();
  fo_test::settle();
  // Row 0 is at y 12..44: the icon is drawn at the right end, not the left.
  CHECK(ink(&win, 600 - 12 - 40, 600 - 12, 14, 42) > 10);
  CHECK_EQ(ink(&win, 12, 12 + 40, 14, 42), 0);

  // Breadcrumbs: the trail starts at the right edge.
  int went = -1;
  crumbs->segments({{"~", "/home/me"}, {"Pictures", "/home/me/Pictures"}, {"Lisbon", "/l"}});
  crumbs->on_navigate([&](int i) { went = i; });
  Fl::e_x = crumbs->x() + crumbs->w() - 4;
  Fl::e_y = crumbs->y() + 20;
  crumbs->handle(FL_RELEASE);
  CHECK_EQ(went, 0);
  went = -1;
  Fl::e_x = crumbs->x() + 4;
  crumbs->handle(FL_RELEASE);
  CHECK_EQ(went, -1); // nothing at the left end of a short trail

  // Menus open to the left of the pointer, submenus too, and Left opens
  // a submenu.
  std::vector<MenuItem> items = {{"Open"}, {"Sort by", "", nullptr, {{"Name"}, {"Size"}}}, {"Rename"}};
  PopupMenu *m = PopupMenu::open_at(&win, 580, 60, items);
  int cx, cy, cw, ch;
  CHECK(m->card_rect(0, cx, cy, cw, ch));
  CHECK_EQ(cx + cw, 580);
  send_key(m, FL_Down);
  send_key(m, FL_Down);
  CHECK_EQ(m->highlighted(0), 1);
  send_key(m, FL_Right); // closes nothing, opens nothing
  CHECK_EQ(m->depth(), 1);
  send_key(m, FL_Left);
  CHECK_EQ(m->depth(), 2);
  int sx, sy, sw, sh;
  CHECK(m->card_rect(1, sx, sy, sw, sh));
  CHECK(sx + sw <= cx + 2);
  send_key(m, FL_Right); // back out
  CHECK_EQ(m->depth(), 1);
  m->close();
  fo_test::settle();
  win.hide();
  anim::set_enabled(-1);
}
