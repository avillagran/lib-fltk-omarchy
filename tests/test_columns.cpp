// ColumnView without a display: placement, following the newest column,
// truncating, and Left/Right between columns.
#include "harness.h"

#include "fltk-omarchy/anim.h"
#include "fltk-omarchy/collection.h"
#include "fltk-omarchy/columns.h"
#include "fltk-omarchy/i18n.h"

#include <FL/Fl.H>

#include <cstring>

using namespace fo;

namespace {

class Col : public Collection {
public:
  Col() : Collection(0, 0, 10, 10) {}
  int count() const override { return 5; }
  void draw_item(int, int, int, int, int, const ItemState &) override {}
};

int key(Fl_Widget *w, int k) {
  Fl::e_keysym = k;
  Fl::e_state = 0;
  Fl::e_text = const_cast<char *>("");
  Fl::e_length = 0;
  return w->handle(FL_KEYBOARD);
}

} // namespace

TEST(columns_place_follow_and_truncate) {
  anim::set_enabled(0);
  ColumnView v(0, 0, 500, 300);
  v.column_width(200);
  std::vector<Col *> cols;
  for (int i = 0; i < 4; ++i) {
    cols.push_back(new Col());
    CHECK_EQ(v.add_column(cols.back()), i);
  }
  CHECK_EQ(v.columns(), 4);
  CHECK_NEAR(v.max_offset_x(), 4 * 201 - 500, 0.1);
  // The newest column is in view.
  CHECK_NEAR(v.offset_x(), 4 * 201 - 500, 0.1);
  CHECK_EQ(cols[3]->x(), 3 * 201 - (4 * 201 - 500));
  CHECK_EQ(cols[3]->w(), 200);
  CHECK_EQ(cols[3]->h(), 300);
  CHECK_EQ(v.index_of(cols[2]), 2);
  // Making the first one active brings it back.
  int changed = -1;
  v.on_active_change([&](int i) { changed = i; });
  v.set_active(0, false);
  CHECK_EQ(v.active(), 0);
  CHECK_EQ(changed, 0);
  CHECK_NEAR(v.offset_x(), 0, 0.1);
  CHECK_EQ(cols[0]->x(), 0);
  // Going back up drops the deeper columns; the strip fits again.
  v.set_active(3, false);
  v.truncate(2);
  CHECK_EQ(v.columns(), 2);
  CHECK_EQ(v.active(), 1);
  CHECK_EQ(changed, 1);
  CHECK_NEAR(v.max_offset_x(), 0, 0.1);
  CHECK_NEAR(v.offset_x(), 0, 0.1);
  Fl::do_widget_deletion();
  // A wider view: nothing to scroll.
  v.resize(0, 0, 900, 300);
  CHECK_EQ(cols[1]->h(), 300);
  CHECK_NEAR(v.max_offset_x(), 0, 0.1);
  anim::set_enabled(-1);
}

TEST(columns_left_right_between_columns) {
  anim::set_enabled(0);
  ColumnView v(0, 0, 500, 300);
  v.column_width(200);
  Col *a = new Col(), *b = new Col(), *c = new Col();
  v.add_column(a);
  v.add_column(b);
  v.add_column(c);
  v.set_active(1, false);
  // A list Collection leaves Left/Right to its parents, as FLTK does with
  // a key the focused widget did not use.
  a->layout(Collection::Layout::List);
  CHECK_EQ(key(b, FL_Right), 0);
  CHECK_EQ(key(&v, FL_Right), 1);
  CHECK_EQ(v.active(), 2);
  CHECK_EQ(key(&v, FL_Right), 0); // the last column: not used
  CHECK_EQ(key(&v, FL_Left), 1);
  CHECK_EQ(key(&v, FL_Left), 1);
  CHECK_EQ(v.active(), 0);
  CHECK_EQ(key(&v, FL_Left), 0);
  // on_step decides first (e.g. Right opens a folder in a new column).
  int steps = 0;
  v.on_step([&](int from, int dir) {
    ++steps;
    if (dir > 0 && from == v.columns() - 1) {
      v.add_column(new Col());
      v.set_active(from + 1);
      return true;
    }
    return false;
  });
  v.set_active(2); // the keys start from the focused column
  CHECK(Fl::focus() == c);
  CHECK_EQ(key(&v, FL_Right), 1);
  CHECK_EQ(v.columns(), 4);
  CHECK_EQ(v.active(), 3);
  CHECK_EQ(steps, 1);
  // Off: the keys are not the view's.
  v.keyboard_navigation(false);
  CHECK_EQ(key(&v, FL_Left), 0);
  CHECK_EQ(v.active(), 3);
  anim::set_enabled(-1);
}

TEST(columns_right_to_left) {
  anim::set_enabled(0);
  i18n::set_language("ar");
  CHECK(i18n::rtl());
  ColumnView v(0, 0, 500, 300);
  v.column_width(200);
  Col *a = new Col(), *b = new Col(), *c = new Col();
  v.add_column(a);
  v.add_column(b);
  v.add_column(c);
  // The first column is on the right; the newest one, at the left, is
  // in view.
  CHECK(a->x() > b->x());
  CHECK(b->x() > c->x());
  CHECK(c->x() >= 0);
  CHECK(c->x() + 200 <= 500);
  // Left goes deeper in reading order.
  v.set_active(1, false);
  CHECK_EQ(key(&v, FL_Left), 1);
  CHECK_EQ(v.active(), 2);
  CHECK_EQ(key(&v, FL_Right), 1);
  CHECK_EQ(v.active(), 1);
  i18n::set_language("");
  anim::set_enabled(-1);
}
