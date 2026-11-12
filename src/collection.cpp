#include "fltk-omarchy/collection.h"

#include "fltk-omarchy/app.h"
#include "fltk-omarchy/i18n.h"
#include "fltk-omarchy/icons.h"
#include "fltk-omarchy/paint.h"
#include "fltk-omarchy/text.h"

#include <FL/Fl.H>
#include <FL/Fl_Window.H>
#include <FL/fl_draw.H>
#include <FL/fl_utf8.h>

#include <cstring>

#include <algorithm>
#include <cmath>

namespace fo {

Collection::Collection(int x, int y, int w, int h) : Fl_Widget(x, y, w, h) {
  box(FL_NO_BOX);
  set_visible_focus();
  bar_.place(x + w - 10, y, 10, h);
}

// Right to left the scrollbar sits on the left edge.
static void place_bar(ScrollBar &bar, int x, int y, int w, int h) { bar.place(rtl() ? x : x + w - 10, y, 10, h); }

// --- Geometry -------------------------------------------------------------------

static int pad_for(int pad, Collection::Layout l) { return pad >= 0 ? pad : l == Collection::Layout::Grid ? 24 : 12; }

void Collection::layout(Layout l) {
  if (l == layout_)
    return;
  layout_ = l;
  ring_ = Anim<RectF>();
  scroller_.jump(0);
  if (current_ >= 0)
    ensure_visible(current_, false);
  redraw();
}

void Collection::row_height(int h) { row_h_ = std::max(8, h); redraw(); }

void Collection::bands(std::vector<Band> b) {
  bands_ = std::move(b);
  geom_w_ = -1;
  redraw();
}

void Collection::band_geometry() const {
  int pad = pad_for(pad_, layout_);
  int W = std::max(1, w() - 2 * pad);
  if (geom_w_ == W && geom_.size() == bands_.size())
    return;
  geom_.clear();
  int y = 0, first = 0;
  for (const Band &b : bands_) {
    BandGeom g;
    g.first = first;
    g.top = y;
    y += b.header_h;
    g.items_top = y;
    g.cols = b.min_width > 0 ? std::max(1, (W + b.gap) / (b.min_width + b.gap)) : 1;
    g.cw = (W - b.gap * (g.cols - 1)) / g.cols;
    g.rows = (b.count + g.cols - 1) / g.cols;
    if (g.rows > 0)
      y += g.rows * (b.item_h + b.gap) - b.gap;
    y += b.after;
    first += b.count;
    geom_.push_back(g);
  }
  geom_h_ = y;
  geom_w_ = W;
}

int Collection::band_of(int index) const {
  if (layout_ != Layout::Bands || index < 0)
    return -1;
  band_geometry();
  for (size_t b = bands_.size(); b-- > 0;)
    if (index >= geom_[b].first)
      return index < geom_[b].first + bands_[b].count ? int(b) : -1;
  return -1;
}

void Collection::visible_range(int off, int &first, int &last) const {
  int n = count(), pad = pad_for(pad_, layout_);
  if (layout_ != Layout::Bands) {
    int unit = layout_ == Layout::List ? row_h_ : tile_h_;
    int cols = columns();
    first = std::max(0, (off - pad) / unit * cols);
    last = std::min(n - 1, ((off + h() - pad) / unit + 1) * cols - 1);
    return;
  }
  band_geometry();
  first = n, last = -1;
  int y0 = off - pad, y1 = off + h() - pad;
  for (size_t b = 0; b < bands_.size(); ++b) {
    const Band &bd = bands_[b];
    const BandGeom &g = geom_[b];
    int step = bd.item_h + bd.gap;
    int bottom = g.items_top + g.rows * step;
    if (bottom < y0 || g.items_top > y1 || bd.count == 0)
      continue;
    int r0 = std::max(0, (y0 - g.items_top) / step);
    int r1 = std::min(g.rows - 1, (y1 - g.items_top) / step);
    first = std::min(first, g.first + r0 * g.cols);
    last = std::max(last, std::min(g.first + bd.count - 1, g.first + (r1 + 1) * g.cols - 1));
  }
  last = std::min(last, n - 1);
}

int Collection::band_step(int cur, int dir) const {
  int b = band_of(cur);
  if (b < 0)
    return cur;
  const BandGeom &g = geom_[size_t(b)];
  int k = cur - g.first, cols = g.cols, col = k % cols;
  int to = cur + dir * cols;
  if (to >= g.first && to < g.first + bands_[size_t(b)].count)
    return to;
  if (dir > 0 && k / cols < g.rows - 1)
    return g.first + bands_[size_t(b)].count - 1; // the short last row
  // Into the neighbouring band, keeping the column where it can.
  int nb = b + dir;
  while (nb >= 0 && nb < int(bands_.size()) && bands_[size_t(nb)].count == 0)
    nb += dir;
  if (nb < 0 || nb >= int(bands_.size()))
    return cur;
  const BandGeom &h2 = geom_[size_t(nb)];
  int cnt = bands_[size_t(nb)].count;
  int c = std::min(col, h2.cols - 1);
  if (dir > 0)
    return h2.first + std::min(c, cnt - 1);
  int last_row = (h2.rows - 1) * h2.cols;
  return h2.first + std::min(last_row + c, cnt - 1);
}
void Collection::tile_size(int w, int h) { tile_w_ = std::max(16, w); tile_h_ = std::max(16, h); redraw(); }
void Collection::padding(int px) { pad_ = px; redraw(); }

int Collection::columns() const {
  if (layout_ != Layout::Grid)
    return 1;
  int pad = pad_for(pad_, layout_);
  return std::max(1, (w() - 2 * pad) / tile_w_);
}

void Collection::cell(int i, int &cx, int &cy, int &cw, int &ch) const {
  int pad = pad_for(pad_, layout_);
  if (layout_ == Layout::Bands) {
    int b = band_of(i);
    if (b < 0) {
      cx = cy = cw = ch = 0;
      return;
    }
    const Band &bd = bands_[size_t(b)];
    const BandGeom &g = geom_[size_t(b)];
    int k = i - g.first, col = k % g.cols;
    if (rtl())
      col = g.cols - 1 - col;
    cx = pad + col * (g.cw + bd.gap);
    cy = pad + g.items_top + (k / g.cols) * (bd.item_h + bd.gap);
    cw = g.cw;
    ch = bd.item_h;
    return;
  }
  if (layout_ == Layout::List) {
    cx = pad;
    cy = pad + i * row_h_;
    cw = w() - 2 * pad;
    ch = row_h_;
    return;
  }
  int cols = columns();
  cw = (w() - 2 * pad) / cols;
  ch = tile_h_;
  int col = i % cols;
  if (rtl())
    col = cols - 1 - col; // tiles flow from the right
  cx = pad + col * cw;
  cy = pad + (i / cols) * tile_h_;
}

int Collection::content_height() const {
  int n = count(), pad = pad_for(pad_, layout_);
  if (layout_ == Layout::Bands) {
    band_geometry();
    return 2 * pad + geom_h_;
  }
  if (layout_ == Layout::List)
    return 2 * pad + n * row_h_;
  int cols = columns();
  return 2 * pad + ((n + cols - 1) / cols) * tile_h_;
}

double Collection::max_offset() const { return std::max(0, content_height() - h()); }

bool Collection::item_rect(int index, int &X, int &Y, int &W, int &H) const {
  if (index < 0 || index >= count())
    return false;
  cell(index, X, Y, W, H);
  X += x();
  Y += y() - int(std::lround(pos_));
  return true;
}

int Collection::item_at(int ex, int ey) const {
  int n = count(), pad = pad_for(pad_, layout_);
  int cx = ex - x(), cy = ey - y() + int(std::lround(pos_));
  if (cx < pad || cx >= w() - pad || cy < pad)
    return -1;
  int i;
  if (layout_ == Layout::Bands) {
    band_geometry();
    int py = cy - pad, px = cx - pad;
    for (size_t b = 0; b < bands_.size(); ++b) {
      const Band &bd = bands_[b];
      const BandGeom &g = geom_[b];
      int step = bd.item_h + bd.gap;
      if (py < g.items_top || py >= g.items_top + g.rows * step)
        continue;
      int r = (py - g.items_top) / step;
      if ((py - g.items_top) % step >= bd.item_h)
        return -1;
      int c = px / (g.cw + bd.gap);
      if (c >= g.cols || px % (g.cw + bd.gap) >= g.cw)
        return -1;
      if (rtl())
        c = g.cols - 1 - c;
      int k = r * g.cols + c;
      return k < bd.count ? g.first + k : -1;
    }
    return -1;
  }
  if (layout_ == Layout::List) {
    i = (cy - pad) / row_h_;
  } else {
    int cols = columns(), cw = (w() - 2 * pad) / cols;
    int col = (cx - pad) / cw;
    if (col >= cols)
      return -1;
    if (rtl())
      col = cols - 1 - col;
    i = ((cy - pad) / tile_h_) * cols + col;
  }
  return i < n ? i : -1;
}

void Collection::resize(int X, int Y, int W, int H) {
  Fl_Widget::resize(X, Y, W, H);
  geom_w_ = -1;
  place_bar(bar_, X, Y, W, H);
  ring_ = Anim<RectF>(); // the grid reflows: no sliding from old places
}

// --- Selection ------------------------------------------------------------------

void Collection::reload(bool reset) {
  int n = count();
  if (reset) {
    sel_.assign(size_t(n), 0);
    current_ = n > 0 ? 0 : -1;
    anchor_ = current_;
    scroller_.jump(0);
    ring_ = Anim<RectF>();
  } else {
    sel_.resize(size_t(n), 0);
    if (current_ >= n)
      current_ = n - 1;
    if (anchor_ >= n)
      anchor_ = current_;
  }
  hover_ = prev_hover_ = -1;
  redraw();
}

bool Collection::selected(int i) const { return i >= 0 && i < int(sel_.size()) && sel_[size_t(i)]; }

void Collection::select(int i, bool on) {
  if (i < 0 || i >= count())
    return;
  sel_.resize(size_t(count()), 0);
  if (bool(sel_[size_t(i)]) != on) {
    sel_[size_t(i)] = on;
    changed();
  }
}

void Collection::select_all() {
  sel_.assign(size_t(count()), 1);
  changed();
}

void Collection::clear_selection() {
  sel_.assign(size_t(count()), 0);
  changed();
}

std::vector<int> Collection::selection() const {
  std::vector<int> v;
  for (size_t i = 0; i < sel_.size(); ++i)
    if (sel_[i])
      v.push_back(int(i));
  return v;
}

int Collection::selection_count() const { return int(std::count(sel_.begin(), sel_.end(), char(1))); }

void Collection::changed() {
  redraw();
  if (selchange_)
    selchange_();
}

void Collection::set_current(int index, bool sel) {
  int n = count();
  if (n == 0) {
    current_ = -1;
    return;
  }
  if (index < 0) {
    // No cursor: arrows start from the first item again.
    current_ = -1;
    anchor_ = -1;
    if (sel)
      sel_.assign(size_t(n), 0);
    ring_ = Anim<RectF>();
    changed();
    return;
  }
  index = std::max(0, std::min(index, n - 1));
  current_ = index;
  anchor_ = index;
  if (sel) {
    sel_.assign(size_t(n), 0);
    sel_[size_t(index)] = 1;
  }
  ensure_visible(index);
  changed();
}

void Collection::ensure_visible(int index, bool animate) {
  int cx, cy, cw, ch;
  if (index < 0 || index >= count())
    return;
  cell(index, cx, cy, cw, ch);
  double top = scroller_.target(), max = max_offset();
  double to = top;
  if (cy < top + 4)
    to = cy - (index < columns() ? pad_for(pad_, layout_) : 4);
  else if (cy + ch > top + h() - 4)
    to = cy + ch - h() + 4;
  if (to != top) {
    if (animate)
      scroller_.scroll_to(to, max);
    else
      scroller_.jump(std::min(std::max(to, 0.0), max));
    redraw();
  }
}

void Collection::scroll_to(double offset, bool animate) {
  if (animate)
    scroller_.scroll_to(offset, max_offset());
  else
    scroller_.jump(std::min(std::max(offset, 0.0), max_offset()));
  bar_.poke(this);
  redraw();
}

void Collection::move_cursor(int to, bool extend, bool keep) {
  int n = count();
  if (n == 0)
    return;
  to = std::max(0, std::min(to, n - 1));
  if (keep && !extend) {
    // Files' Ctrl+arrows: the cursor moves, the selection stays.
    set_current(to, false);
    ensure_visible(to);
    return;
  }
  if (extend && anchor_ >= 0) {
    if (!keep)
      sel_.assign(size_t(n), 0);
    sel_.resize(size_t(n), 0);
    for (int i = std::min(anchor_, to); i <= std::max(anchor_, to); ++i)
      sel_[size_t(i)] = 1;
    current_ = to;
    ensure_visible(to);
    changed();
  } else {
    set_current(to, true);
  }
}

void Collection::apply_band() {
  int n = count();
  sel_ = band_base_;
  sel_.resize(size_t(n), 0);
  int bx0 = std::min(band_x0_, band_x1_), bx1 = std::max(band_x0_, band_x1_);
  int by0 = std::min(band_y0_, band_y1_), by1 = std::max(band_y0_, band_y1_);
  // Only items whose cells can meet the band.
  int first = 0, last = n - 1;
  int pad = pad_for(pad_, layout_);
  if (layout_ != Layout::Bands) {
    int unit = layout_ == Layout::List ? row_h_ : tile_h_;
    int cols = columns();
    first = std::max(0, (by0 - pad) / unit * cols);
    last = std::min(n - 1, ((by1 - pad) / unit + 1) * cols);
  }
  int last_hit = -1;
  for (int i = first; i <= last; ++i) {
    int cx, cy, cw, ch;
    cell(i, cx, cy, cw, ch);
    // Tiles: the art and label, not the whole padded cell.
    int ix = cx + 6, iy = cy + 4, iw = cw - 12, ih = ch - 8;
    if (ix < bx1 && ix + iw > bx0 && iy < by1 && iy + ih > by0) {
      sel_[size_t(i)] = band_add_ ? !band_base_[size_t(i)] : 1;
      last_hit = i;
    }
  }
  if (last_hit >= 0)
    current_ = last_hit;
  changed();
}

// --- Input -------------------------------------------------------------------

void Collection::type_char(const char *t) {
  double now = anim::now_ms();
  if (now - typed_at_ > anim::ms::TYPEAHEAD)
    typed_.clear();
  typed_at_ = now;
  typed_ += t;
  int n = count();
  if (!n)
    return;
  int from = current_ < 0 ? 0 : (typed_.size() == strlen(t) ? current_ + 1 : current_);
  int found = -1;
  if (typeahead_) {
    found = typeahead_(typed_, from % n);
  } else {
    std::string want = typed_;
    for (auto &c : want)
      c = char(std::tolower(static_cast<unsigned char>(c)));
    for (int k = 0; k < n && found < 0; ++k) {
      int i = (from + k) % n;
      std::string s = item_text(i).substr(0, want.size());
      for (auto &c : s)
        c = char(std::tolower(static_cast<unsigned char>(c)));
      if (s == want)
        found = i;
    }
  }
  if (found >= 0)
    set_current(found, true);
}

bool Collection::key(int k) {
  if (key_ && key_(k))
    return true;
  int n = count();
  int state = Fl::event_state();
  bool shift = state & FL_SHIFT, ctrl = state & FL_CTRL;
  int cols = columns();
  int cur = current_ < 0 ? -1 : current_;
  // Right to left, Left goes forward in a grid.
  int left = rtl() ? 1 : -1;
  int page = std::max(1, h() / (layout_ == Layout::Grid ? tile_h_ : layout_ == Layout::Bands ? 40 : row_h_)) * cols;
  bool multi = layout_ == Layout::Grid;
  if (layout_ == Layout::Bands && cur >= 0) {
    int b = band_of(cur);
    multi = b >= 0 && geom_[size_t(b)].cols > 1;
  }
  if (ctrl && (k == 'a' || k == 'A')) {
    select_all();
    return true;
  }
  if (state & (FL_ALT | FL_META))
    return false;
  if (ctrl && k != FL_Up && k != FL_Down && k != FL_Left && k != FL_Right && k != FL_Home && k != FL_End)
    return false;
  if (n == 0) {
    // Nothing to move to, but the arrows are still ours: passing them on
    // would let FLTK move the focus to a neighbouring widget.
    switch (k) {
    case FL_Left:
    case FL_Right:
      return layout_ == Layout::Grid;
    case FL_Up:
    case FL_Down:
    case FL_Home:
    case FL_End:
    case FL_Page_Up:
    case FL_Page_Down: return true;
    }
    return false;
  }
  bool bands = layout_ == Layout::Bands;
  switch (k) {
  case FL_Up: move_cursor(cur < 0 ? n - 1 : bands ? band_step(cur, -1) : cur - cols, shift, ctrl); return true;
  case FL_Down: move_cursor(cur < 0 ? 0 : bands ? band_step(cur, 1) : cur + cols, shift, ctrl); return true;
  case FL_Left:
    if (!multi)
      return false;
    move_cursor(cur < 0 ? 0 : cur + left, shift, ctrl);
    return true;
  case FL_Right:
    if (!multi)
      return false;
    move_cursor(cur < 0 ? 0 : cur - left, shift, ctrl);
    return true;
  case FL_Home: move_cursor(0, shift, ctrl); return true;
  case FL_End: move_cursor(n - 1, shift, ctrl); return true;
  case FL_Page_Up: move_cursor(cur - page, shift); return true;
  case FL_Page_Down: move_cursor(cur + page, shift); return true;
  case FL_Enter:
  case FL_KP_Enter:
    if (cur >= 0 && activate_)
      activate_(cur);
    return true;
  }
  const char *t = Fl::event_text();
  if (t && Fl::event_length() > 0 && static_cast<unsigned char>(t[0]) >= 0x20 && t[0] != 0x7f &&
      !(t[0] == ' ' && typed_.empty())) {
    type_char(t);
    return true;
  }
  return false;
}

int Collection::handle(int event) {
  double max = max_offset();
  double p = pos_;
  place_bar(bar_, x(), y(), w(), h());
  if (bar_.handle(event, this, p, max, h())) {
    scroller_.jump(p);
    redraw();
    return 1;
  }
  int ex = Fl::event_x(), ey = Fl::event_y();
  int state = Fl::event_state();
  bool toggle = state & (FL_CTRL | FL_ALT);
  switch (event) {
  case FL_ENTER:
  case FL_MOVE: {
    int hv = item_at(ex, ey);
    if (hv != hover_) {
      prev_hover_ = hover_;
      hover_out_.jump(hover_in_.value());
      hover_out_.set(0, 120);
      hover_ = hv;
      hover_in_.jump(0);
      hover_in_.set(1, 120);
      redraw();
    }
    return 1;
  }
  case FL_LEAVE:
    if (hover_ >= 0) {
      prev_hover_ = hover_;
      hover_out_.jump(hover_in_.value());
      hover_out_.set(0, 120);
      hover_ = -1;
      redraw();
    }
    return 1;
  case FL_FOCUS:
  case FL_UNFOCUS:
    redraw();
    return 1;
  case FL_PUSH: {
    if (Fl::focus() != this)
      take_focus();
    int i = item_at(ex, ey);
    press_x_ = ex, press_y_ = ey;
    dragged_ = false;
    press_on_item_ = false;
    if (Fl::event_button() == FL_RIGHT_MOUSE) {
      if (i >= 0 && !selected(i))
        set_current(i, true);
      else if (i < 0)
        clear_selection();
      if (context_)
        context_(i, ex, ey);
      return 1;
    }
    if (Fl::event_button() != FL_LEFT_MOUSE)
      return 1;
    if (i < 0 && layout_ == Layout::Bands) {
      // A band header (Files: the new-folder button beside "FOLDERS").
      band_geometry();
      int pad = pad_for(pad_, layout_);
      int cy = ey - y() + int(std::lround(pos_)) - pad;
      for (size_t b = 0; b < bands_.size(); ++b)
        if (bands_[b].header_h > 0 && cy >= geom_[b].top && cy < geom_[b].top + bands_[b].header_h) {
          band_header_clicked(int(b), ex, ey);
          return 1;
        }
    }
    if (i >= 0) {
      if (Fl::event_clicks() > 0 && !toggle && !(state & FL_SHIFT)) {
        Fl::event_clicks(0);
        if (activate_)
          activate_(i);
        return 1;
      }
      if (state & FL_SHIFT) {
        move_cursor(i, true);
      } else if (toggle) {
        current_ = i;
        anchor_ = i;
        select(i, !selected(i));
      } else if (selected(i) && selection_count() > 1) {
        current_ = i; // keep the group for a drag; collapse on release
        press_on_item_ = true;
        redraw();
      } else {
        current_ = i;
        anchor_ = i;
        sel_.assign(size_t(count()), 0);
        if (i < int(sel_.size()))
          sel_[size_t(i)] = 1;
        press_on_item_ = true;
        changed();
      }
      return 1;
    }
    // Empty space: start a selection rectangle.
    banding_ = true;
    band_add_ = toggle;
    if (!band_add_)
      sel_.assign(size_t(count()), 0);
    band_base_ = sel_;
    band_x0_ = band_x1_ = ex - x();
    band_y0_ = band_y1_ = ey - y() + int(std::lround(pos_));
    changed();
    return 1;
  }
  case FL_DRAG:
    if (banding_) {
      // Past the edges the view scrolls along.
      if (ey < y())
        scroller_.jump(std::max(0.0, pos_ - (y() - ey) / 3.0 - 1));
      else if (ey > y() + h())
        scroller_.jump(std::min(max, pos_ + (ey - y() - h()) / 3.0 + 1));
      pos_ = scroller_.position(max);
      band_x1_ = std::max(0, std::min(ex - x(), w()));
      band_y1_ = int(std::max(0.0, std::min(double(ey - y()), double(h())) + pos_));
      apply_band();
      return 1;
    }
    if (press_on_item_ && !dragged_ && (std::abs(ex - press_x_) > 6 || std::abs(ey - press_y_) > 6)) {
      dragged_ = true;
      if (drag_)
        drag_();
    }
    return 1;
  case FL_RELEASE:
    if (banding_) {
      banding_ = false;
      redraw();
      return 1;
    }
    if (press_on_item_ && !dragged_ && !toggle && selection_count() > 1) {
      int i = item_at(ex, ey);
      if (i >= 0)
        set_current(i, true);
    }
    press_on_item_ = false;
    return 1;
  case FL_MOUSEWHEEL: {
    if (Fl::event_dy() == 0)
      return 0;
    double d = Fl::event_dy() * (layout_ == Layout::List ? 3.0 * row_h_ : layout_ == Layout::Bands ? 114.0 : tile_h_ * 0.5);
    if (scroller_.passes(d, max))
      return 1;
    scroller_.wheel(d, max);
    bar_.poke(this);
    redraw();
    return 1;
  }
  case FL_KEYBOARD:
    return key(Fl::event_key()) ? 1 : 0;
  case FL_DND_ENTER:
  case FL_DND_DRAG: {
    if (!accepts_)
      return 0;
    int i = item_at(ex, ey);
    int target = i >= 0 && accepts_(i) ? i : accepts_(-1) ? -1 : -2;
    if (target != drop_index_) {
      drop_index_ = target;
      redraw();
    }
    return 1;
  }
  case FL_DND_LEAVE:
    drop_index_ = -2;
    redraw();
    return 1;
  case FL_DND_RELEASE:
    return drop_index_ != -2 ? 1 : 0;
  case FL_PASTE: {
    int target = drop_index_;
    drop_index_ = -2;
    redraw();
    if (target != -2 && drop_)
      drop_(target, std::string(Fl::event_text(), size_t(Fl::event_length())));
    return 1;
  }
  }
  return 0;
}

// --- Drawing ------------------------------------------------------------------

void Collection::draw() {
  const Theme &t = theme();
  double max = max_offset();
  pos_ = scroller_.position(max);
  if (scroller_.running())
    anim::request_frame(this);
  place_bar(bar_, x(), y(), w(), h());
  int off = int(std::lround(pos_));
  fl_push_clip(x(), y(), w(), h());
  paint::fill(x(), y(), w(), h(), t.background());
  int n = count();
  if (int(sel_.size()) != n)
    sel_.resize(size_t(n), 0);
  int pad = pad_for(pad_, layout_);
  int first, last;
  visible_range(off, first, last);
  if (layout_ == Layout::Bands) {
    band_geometry();
    for (size_t b = 0; b < bands_.size(); ++b) {
      const Band &bd = bands_[b];
      int hy = y() + pad + geom_[b].top - off;
      if (bd.header_h > 0 && bd.count > 0 && hy + bd.header_h > y() && hy < y() + h())
        draw_band_header(int(b), x() + pad, hy, w() - 2 * pad, bd.header_h);
    }
  }
  bool focused = Fl::focus() == this;
  double hin = hover_in_.get(this), hout = hover_out_.get(this);
  for (int i = first; i <= last; ++i) {
    int cx, cy, cw, ch;
    cell(i, cx, cy, cw, ch);
    ItemState s;
    s.selected = sel_[size_t(i)];
    s.current = i == current_;
    s.hovered = i == hover_;
    s.focused = focused;
    s.drop_target = i == drop_index_;
    s.hover = i == hover_ ? hin : i == prev_hover_ ? hout : 0;
    draw_item(i, x() + cx, y() + cy - off, cw, ch, s);
  }
  // The focus ring slides from item to item.
  if ((always_ring_ || (focused && focus_visible())) && current_ >= 0 && current_ < n &&
      (ring_on_selected_ || !sel_[size_t(current_)])) {
    int cx, cy, cw, ch;
    cell(current_, cx, cy, cw, ch);
    RectF target{double(cx), double(cy), double(cw), double(ch)};
    if (!ring_.initialized())
      ring_.jump(target);
    else
      ring_.set(target, anim::ms::FOCUS);
    RectF r = ring_.get(this);
    int rr = t.radius() > 0 ? (ring_radius_ > 0 ? ring_radius_ : t.radius_small()) : 0;
    paint::round_frame(x() + int(std::lround(r.x)), y() + int(std::lround(r.y)) - off, int(std::lround(r.w)),
                       int(std::lround(r.h)), rr, 1, focused || !always_ring_ ? t.accent() : t.dim());
  }
  if (banding_) {
    int bx = std::min(band_x0_, band_x1_), by = std::min(band_y0_, band_y1_);
    int bw = std::abs(band_x1_ - band_x0_), bh = std::abs(band_y1_ - band_y0_);
    paint::fill(x() + bx, y() + by - off, bw, bh, t.accent(), 0.12);
    paint::round_frame(x() + bx, y() + by - off, bw, bh, 0, 1, t.accent(), 0.7);
  }
  if (drop_index_ == -1)
    paint::round_frame(x() + 2, y() + 2, w() - 4, h() - 4, t.radius_small(), 2, t.accent());
  bar_.draw(this, pos_, max, h());
  fl_pop_clip();
}

void Collection::draw_row(int X, int Y, int W, int H, const ItemState &s, const std::string &icn,
                          const std::string &name, const std::string &detail, Rgb icon_color, bool chevron) {
  const Theme &t = theme();
  int r = t.radius() > 0 ? 9 : 0;
  if (s.selected || s.drop_target)
    paint::round_fill(X, Y, W, H, r, s.drop_target ? t.tint(t.accent(), 0.24) : t.selection());
  else if (s.hover > 0)
    paint::round_fill(X, Y, W, H, r, mix(t.background(), t.card(), s.hover));
  if (s.current && s.focused)
    paint::fill(rtl() ? X + W - 3 : X + 1, Y + 4, 2, std::max(0, H - 8), t.accent());
  Rgb fg = t.bright_foreground();
  // Laid out left to right, then mirrored when the language reads right
  // to left: the icon at the right edge, the detail and chevron at the left.
  bool mirror = rtl();
  auto mx = [&](int x0, int w0) { return mirror ? X + W - (x0 - X) - w0 : x0; };
  int ix = X + 10;
  int is = std::min(16, H - 8);
  icon::draw(icn, mx(ix, is), Y + (H - is) / 2, is, icon_color == Rgb() ? t.blue() : icon_color);
  int lx = ix + is + 12;
  int right = X + W - 10;
  if (chevron) {
    text::draw("\xE2\x80\xBA", mx(right - 10, 10), Y, 10, H, s.selected ? t.accent() : t.dim(), text::REGULAR, 13,
               FL_ALIGN_CENTER, text::Elide::None);
    right -= 16;
  }
  if (!detail.empty()) {
    int dw = std::min(int(text::width(detail, text::REGULAR, 11)) + 1, (right - lx) / 2);
    text::draw(detail, mx(right - dw, dw), Y, dw, H, t.dim(), text::REGULAR, 11,
               mirror ? FL_ALIGN_LEFT : FL_ALIGN_RIGHT);
    right -= dw + 12;
  }
  text::draw(name, mx(lx, right - lx), Y, right - lx, H, fg, text::REGULAR, 12,
             mirror ? FL_ALIGN_RIGHT : FL_ALIGN_LEFT, text::Elide::Middle);
}

void Collection::draw_tile(int X, int Y, int W, int H, const ItemState &s, const std::string &icn,
                           const std::string &name, const std::string &subtitle, Rgb icon_color, bool folder_card,
                           int *art_x, int *art_y, int *art_w, int *art_h) {
  const Theme &t = theme();
  // Files' grid card: inset 4 x 3 in its cell, the art 55 px above the
  // bottom, an 11 px name and a 10 px detail under it.
  int r = t.radius() > 0 ? 8 : 0;
  int cx = X + 4, cy = Y + 3, cw = std::max(0, W - 8), ch = std::max(0, H - 6);
  if (s.selected || s.drop_target)
    paint::round_fill(cx, cy, cw, ch, r, s.drop_target ? t.tint(t.accent(), 0.24) : t.selection());
  else if (s.hover > 0)
    paint::round_fill(cx, cy, cw, ch, r, mix(t.background(), t.card(), s.hover));
  if (s.current && s.focused)
    paint::fill(rtl() ? cx + cw - 5 : cx + 3, cy + 6, 2, std::max(0, ch - 12), t.accent());
  int icon_h = std::max(1, H - 55);
  int ax = cx + 4, ay = cy + 3, aw = cw - 8, ah = icon_h - 4;
  if (folder_card) {
    int is = std::min(48, ah - 4);
    icon::draw("folder", ax + (aw - is) / 2, ay + (ah - is) / 2, is, t.blue());
  } else if (!icn.empty()) {
    int is = std::min(36, ah - 4);
    icon::draw(icn, ax + (aw - is) / 2, ay + (ah - is) / 2, is, icon_color == Rgb() ? t.dim() : icon_color);
  }
  if (art_x) *art_x = ax;
  if (art_y) *art_y = ay;
  if (art_w) *art_w = aw;
  if (art_h) *art_h = ah;
  Fl_Align al = rtl() ? FL_ALIGN_RIGHT : FL_ALIGN_LEFT;
  text::draw(name, cx + 7, cy + icon_h, cw - 14, 24, t.bright_foreground(), text::REGULAR, 11, al,
             text::Elide::Middle);
  if (!subtitle.empty())
    text::draw(subtitle, cx + 7, cy + icon_h + 23, cw - 14, 20, t.dim(), text::REGULAR, 10, al);
}

} // namespace fo
