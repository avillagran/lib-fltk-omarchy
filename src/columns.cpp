#include "fltk-omarchy/columns.h"

#include "fltk-omarchy/i18n.h"
#include "fltk-omarchy/paint.h"
#include "fltk-omarchy/theme.h"

#include <FL/Fl.H>

#include <algorithm>
#include <cmath>

namespace fo {

namespace {
// The strip holding the columns. Left and Right that a column did not use
// belong to the view, even when FLTK's arrow-key focus navigation is on
// (fo::init() turns it off; plain FLTK apps may not).
class Strip : public Fl_Group {
public:
  Strip(int x, int y, int w, int h) : Fl_Group(x, y, w, h) {}
  int handle(int event) override {
    if (event == FL_KEYBOARD && (Fl::event_key() == FL_Left || Fl::event_key() == FL_Right))
      return 0;
    return Fl_Group::handle(event);
  }
};
} // namespace

ColumnView::ColumnView(int x, int y, int w, int h) : ScrollArea(x, y, w, h) {
  strip_ = new Strip(x, y, w, h);
  strip_->box(FL_NO_BOX);
  strip_->resizable(nullptr); // the view places the columns itself
  strip_->end();
  end();
  axes(Horizontal);
}

void ColumnView::column_width(int w) {
  col_w_ = std::max(40, w);
  relayout();
  if (active_ >= 0)
    reveal(active_, false);
}

int ColumnView::columns() const { return strip_->children(); }

Fl_Widget *ColumnView::column(int i) const { return i >= 0 && i < columns() ? strip_->child(i) : nullptr; }

int ColumnView::index_of(const Fl_Widget *w) const {
  for (int i = 0; i < columns(); ++i)
    if (strip_->child(i) == w || (w && w->inside(strip_->child(i))))
      return i;
  return -1;
}

int ColumnView::column_x(int i) const {
  // Right to left: the first column sits at the right end of the strip.
  int n = columns();
  int stride = col_w_ + 1;
  if (i18n::rtl())
    return std::max(n * stride, w()) - (i + 1) * stride + 1;
  return i * stride;
}

void ColumnView::relayout() {
  int n = columns();
  int total = std::max(n * (col_w_ + 1), w());
  if (strip_->w() != total || strip_->h() != h())
    strip_->size(total, h());
  for (int i = 0; i < n; ++i) {
    Fl_Widget *c = strip_->child(i);
    int cx = strip_->x() + column_x(i);
    if (c->x() != cx || c->y() != strip_->y() || c->w() != col_w_ || c->h() != h())
      c->resize(cx, strip_->y(), col_w_, h());
  }
  // The strip changed size: let the area clamp its offset.
  ScrollArea::resize(x(), y(), w(), h());
  redraw();
}

int ColumnView::add_column(Fl_Widget *w) {
  if (!w)
    return -1;
  strip_->add(w);
  relayout();
  int index = columns() - 1;
  if (follow_last_)
    reveal(index, true);
  return index;
}

void ColumnView::truncate(int count) {
  count = std::max(0, count);
  bool had_focus = false;
  while (columns() > count) {
    Fl_Widget *c = strip_->child(columns() - 1);
    if (Fl::focus() && (Fl::focus() == c || Fl::focus()->inside(c)))
      had_focus = true;
    strip_->remove(c);
    Fl::delete_widget(c);
  }
  relayout();
  if (active_ >= count) {
    int was = active_;
    active_ = count - 1;
    if (had_focus && active_ >= 0)
      strip_->child(active_)->take_focus();
    if (active_ != was && active_change_)
      active_change_(active_);
  }
  if (follow_last_ && count > 0)
    reveal(count - 1, true);
}

void ColumnView::set_active(int index, bool focus, bool animate) {
  if (index < 0 || index >= columns())
    return;
  if (focus)
    strip_->child(index)->take_focus();
  reveal(index, animate);
  if (index != active_) {
    active_ = index;
    if (active_change_)
      active_change_(index);
  }
}

void ColumnView::reveal(int index, bool animate) {
  if (index < 0 || index >= columns())
    return;
  ensure_visible_x(column_x(index), col_w_ + 1, animate);
}

void ColumnView::note_active() {
  int i = index_of(Fl::focus());
  if (i >= 0 && i != active_) {
    active_ = i;
    reveal(i, true);
    if (active_change_)
      active_change_(i);
  }
}

int ColumnView::handle(int event) {
  if (event == FL_KEYBOARD && keys_ && !Fl::event_state(FL_SHIFT | FL_CTRL | FL_ALT | FL_META)) {
    int k = Fl::event_key();
    if (k == FL_Left || k == FL_Right) {
      int from = index_of(Fl::focus());
      if (from < 0)
        from = active_;
      if (from >= 0) {
        int dir = (k == FL_Right) != i18n::rtl() ? 1 : -1;
        if (step_ && step_(from, dir))
          return 1;
        int to = from + dir;
        if (to >= 0 && to < columns()) {
          set_active(to);
          return 1;
        }
      }
    }
  }
  int r = ScrollArea::handle(event);
  if (event == FL_PUSH || event == FL_FOCUS || event == FL_KEYBOARD || event == FL_RELEASE)
    note_active();
  return r;
}

void ColumnView::resize(int X, int Y, int W, int H) {
  ScrollArea::resize(X, Y, W, H);
  relayout();
  int keep = active_ >= 0 ? active_ : columns() - 1;
  if (keep >= 0)
    reveal(keep, false);
}

void ColumnView::draw_over_content() {
  const Theme &t = theme();
  int n = columns();
  for (int i = 0; i < n; ++i) {
    // The hairline after each column, in reading order.
    int lx = i18n::rtl() ? strip_->x() + column_x(i) - 1 : strip_->x() + column_x(i) + col_w_;
    if (lx >= x() && lx < x() + w())
      paint::vline(lx, y(), h(), t.foreground(), 0.10);
  }
}

} // namespace fo
