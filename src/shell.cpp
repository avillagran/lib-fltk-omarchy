#include "fltk-omarchy/shell.h"

#include "fltk-omarchy/dialog.h"
#include "fltk-omarchy/icons.h"
#include "fltk-omarchy/paint.h"
#include "fltk-omarchy/text.h"
#include "fltk-omarchy/window.h"

#include <FL/Fl.H>
#include <FL/Fl_Window.H>
#include <FL/fl_draw.H>

#include <algorithm>
#include <cmath>

namespace fo {

namespace {

// Files rounds controls by 7 px (and cards by 11) when the theme rounds
// at all; square themes stay square.
int round7() { return theme().radius() > 0 ? 7 : 0; }

constexpr double kRestSeconds = 2.0; // Files' TOOLTIP_DELAY

} // namespace

// --- ToolBar ------------------------------------------------------------------

ToolBar::ToolBar(int x, int y, int w, int h) : Fl_Widget(x, y, w, h) {
  box(FL_NO_BOX);
  clear_visible_focus();
}

ToolBar::~ToolBar() {
  Fl::remove_timeout(rest, this);
  Tooltip::close_all();
}

void ToolBar::items(std::vector<Item> items) {
  if (items.size() != items_.size()) {
    hot_ = -1;
    extra_.assign(items.size(), Anim<double>(0.0));
  }
  items_ = std::move(items);
  redraw();
}

void ToolBar::set_on(int i, bool on) {
  if (i >= 0 && i < int(items_.size()) && items_[size_t(i)].on != on) {
    items_[size_t(i)].on = on;
    redraw();
  }
}

void ToolBar::set_enabled(int i, bool on) {
  if (i >= 0 && i < int(items_.size()) && items_[size_t(i)].enabled != on) {
    items_[size_t(i)].enabled = on;
    redraw();
  }
}

int ToolBar::width_of(int i) const {
  auto &a = const_cast<Anim<double> &>(extra_[size_t(i)]);
  return button_ + int(std::lround(std::max(0.0, a.value())));
}

int ToolBar::preferred_width() const {
  int n = int(items_.size());
  if (!n)
    return 0;
  int sum = (n - 1) * gap_;
  for (int i = 0; i < n; ++i)
    sum += width_of(i);
  return sum;
}

int ToolBar::item_at(int ex) const {
  int n = int(items_.size());
  if (plain_) {
    int cx = x();
    for (int i = 0; i < n; ++i) {
      int wi = width_of(i);
      if (ex >= cx && ex < cx + wi)
        return i;
      cx += wi + gap_;
    }
    return -1;
  }
  int cx = x() + w();
  for (int i = n - 1; i >= 0; --i) {
    int wi = width_of(i);
    if (ex >= cx - wi && ex < cx)
      return i;
    cx -= wi + gap_;
  }
  return -1;
}

bool ToolBar::item_rect(int i, int &rx, int &ry, int &rw, int &rh) const {
  int n = int(items_.size());
  if (i < 0 || i >= n)
    return false;
  ry = y();
  rh = h();
  if (plain_) {
    int cx = x();
    for (int k = 0; k < i; ++k)
      cx += width_of(k) + gap_;
    rx = cx;
    rw = width_of(i);
    return true;
  }
  int cx = x() + w();
  for (int k = n - 1; k > i; --k)
    cx -= width_of(k) + gap_;
  rw = width_of(i);
  rx = cx - rw;
  return true;
}

void ToolBar::tick(void *data) {
  auto *t = static_cast<ToolBar *>(data);
  bool moving = false;
  for (auto &a : t->extra_)
    moving = moving || a.running();
  if (t->relayout_)
    t->relayout_();
  t->redraw();
  if (moving)
    anim::request_frame(tick, t);
}

void ToolBar::rest(void *data) {
  auto *t = static_cast<ToolBar *>(data);
  if (t->hot_ < 0)
    return;
  const Item &it = t->items_[size_t(t->hot_)];
  if (it.shortcut.empty())
    return;
  int rx, ry, rw, rh;
  t->item_rect(t->hot_, rx, ry, rw, rh);
  if (Window *win = Window::of(t))
    Tooltip::open_at(win, rx, ry, rw, rh, it.shortcut);
}

void ToolBar::set_hot(int i) {
  if (i == hot_)
    return;
  hot_ = i;
  Fl::remove_timeout(rest, this);
  Tooltip::close_all();
  if (i >= 0)
    Fl::add_timeout(kRestSeconds, rest, this);
  if (!plain_) {
    for (size_t k = 0; k < items_.size(); ++k) {
      double target = 0;
      if (int(k) == hot_ && !items_[k].label.empty())
        target = std::min(double(allowance_), text::width(items_[k].label, text::REGULAR, 12) + 12);
      extra_[k].set(target, 160, anim::Ease::OutCubic);
    }
    anim::request_frame(tick, this);
    tick(this);
  }
  redraw();
}

int ToolBar::handle(int event) {
  switch (event) {
  case FL_ENTER:
  case FL_MOVE: {
    int i = item_at(Fl::event_x());
    set_hot(i);
    return 1;
  }
  case FL_LEAVE:
  case FL_HIDE:
    set_hot(-1);
    return event == FL_LEAVE ? 1 : 0;
  case FL_PUSH: {
    Fl::remove_timeout(rest, this);
    Tooltip::close_all();
    int i = item_at(Fl::event_x());
    if (i < 0 || Fl::event_button() != FL_LEFT_MOUSE)
      return 0;
    pressed_ = i;
    return 1;
  }
  case FL_RELEASE: {
    int i = item_at(Fl::event_x());
    int p = pressed_;
    pressed_ = -1;
    if (i == p && i >= 0 && Fl::event_inside(this)) {
      const Item &it = items_[size_t(i)];
      if (it.enabled && it.action) {
        auto fn = it.action; // the action may replace the items
        fn();
      }
    }
    return 1;
  }
  }
  return 0;
}

void ToolBar::draw() {
  const Theme &t = theme();
  int n = int(items_.size());
  for (int i = 0; i < n; ++i) {
    const Item &it = items_[size_t(i)];
    int rx, ry, rw, rh;
    item_rect(i, rx, ry, rw, rh);
    bool hot = i == hot_ && it.enabled;
    if (plain_) {
      Rgb c = !it.enabled ? t.muted() : hot ? t.accent() : t.foreground();
      icon::draw(it.icon, rx + (rw - 16) / 2, ry + (rh - 16) / 2, 16, c);
      continue;
    }
    if (it.on)
      paint::round_fill(rx, ry, rw, rh, round7(), t.card());
    Rgb c = !it.enabled ? t.muted() : it.on || hot ? t.accent() : t.dim();
    // The icon keeps the stable right edge; the label reveals to its left.
    icon::draw(it.icon, rx + rw - 26 + 3, ry + (rh - 18) / 2, 18, c);
    if (rw > button_ + 8) {
      fl_push_clip(rx + 8, ry, rw - 40, rh);
      text::draw(it.label, rx + 8, ry, rw - 40 + 4, rh, c, text::REGULAR, 12, FL_ALIGN_LEFT, text::Elide::None);
      fl_pop_clip();
    }
  }
}

// --- TabStrip -----------------------------------------------------------------

TabStrip::TabStrip(int x, int y, int w, int h) : Fl_Widget(x, y, w, h) {
  box(FL_NO_BOX);
  clear_visible_focus();
}

void TabStrip::tabs(std::vector<std::string> labels, int current) {
  labels_ = std::move(labels);
  current_ = std::max(0, std::min(current, int(labels_.size()) - 1));
  hover_ = hover_close_ = -1;
  redraw();
}

void TabStrip::layout() {
  slots_.clear();
  int n = int(labels_.size());
  plus_x_ = x() + w() - 40;
  int avail = plus_x_ - x() - 8;
  std::vector<int> want(static_cast<size_t>(n));
  int total = 0;
  for (int i = 0; i < n; ++i) {
    // label, padding 12 on each side, room for the close button
    int tw = int(text::width(labels_[size_t(i)], text::BOLD, 12)) + 24 + 22;
    want[size_t(i)] = std::min(220, std::max(80, tw));
    total += want[size_t(i)] + 2;
  }
  double k = total > avail && total > 0 ? double(avail) / total : 1.0;
  int cx = x() + 8;
  for (int i = 0; i < n; ++i) {
    int wi = std::max(56, int(want[size_t(i)] * k));
    slots_.push_back({cx, wi});
    cx += wi + 2;
  }
}

bool TabStrip::tab_rect(int i, int &tx, int &tw) const {
  const_cast<TabStrip *>(this)->layout();
  if (i < 0 || i >= int(slots_.size()))
    return false;
  tx = slots_[size_t(i)].x;
  tw = slots_[size_t(i)].w;
  return true;
}

int TabStrip::handle(int event) {
  layout();
  int ex = Fl::event_x(), ey = Fl::event_y();
  auto at = [&](int &close) {
    close = -1;
    for (size_t i = 0; i < slots_.size(); ++i)
      if (ex >= slots_[i].x && ex < slots_[i].x + slots_[i].w) {
        if (ex >= slots_[i].x + slots_[i].w - 24)
          close = int(i);
        return int(i);
      }
    return -1;
  };
  switch (event) {
  case FL_ENTER:
  case FL_MOVE: {
    int c;
    int i = at(c);
    bool plus = ex >= plus_x_ && ex < plus_x_ + 32 && ey >= y() && ey < y() + h();
    if (i != hover_ || c != hover_close_ || plus != hover_plus_) {
      hover_ = i, hover_close_ = c, hover_plus_ = plus;
      redraw();
    }
    return 1;
  }
  case FL_LEAVE:
    hover_ = hover_close_ = -1;
    hover_plus_ = false;
    redraw();
    return 1;
  case FL_PUSH:
    return 1;
  case FL_RELEASE: {
    int c;
    int i = at(c);
    if (!Fl::event_inside(this))
      return 1;
    if (Fl::event_button() == FL_RIGHT_MOUSE) {
      if (i >= 0 && context_)
        context_(i, ex, ey);
      return 1;
    }
    if (Fl::event_button() == FL_MIDDLE_MOUSE) {
      if (i >= 0 && close_)
        close_(i);
      return 1;
    }
    if (i < 0) {
      if ((ex >= plus_x_ || Fl::event_clicks()) && new_)
        new_();
      return 1;
    }
    if (c == i && (i == current_ || hover_close_ == i) && close_)
      close_(i);
    else if (select_)
      select_(i);
    return 1;
  }
  }
  return 0;
}

void TabStrip::draw() {
  const Theme &t = theme();
  layout();
  fl_push_clip(x(), y(), w(), h());
  paint::fill(x(), y(), w(), h(), t.background());
  for (size_t i = 0; i < slots_.size(); ++i) {
    const Slot &s = slots_[i];
    bool cur = int(i) == current_;
    if (cur)
      paint::round_fill(s.x, y() + 2, s.w, h() - 4, round7() ? 5 : 0, t.card());
    Rgb fg = cur ? t.bright_foreground() : int(i) == hover_ ? t.foreground() : t.dim();
    text::draw(labels_[i], s.x + 12, y(), s.w - 12 - 24, h(), fg, cur ? text::BOLD : text::REGULAR, 12);
    if (cur || int(i) == hover_) {
      Rgb xc = int(i) == hover_close_ ? t.red() : t.dim();
      text::draw("\xC3\x97", s.x + s.w - 24, y(), 18, h(), xc, text::REGULAR, 12, FL_ALIGN_CENTER,
                 text::Elide::None);
    }
  }
  if (current_ >= 0 && current_ < int(slots_.size())) {
    const Slot &s = slots_[size_t(current_)];
    RectF target{double(s.x), double(y() + h() - 2), double(s.w), 2};
    if (!line_.initialized())
      line_.jump(target);
    line_.set(target, anim::ms::INDICATOR);
    RectF l = line_.get(this);
    paint::fill(int(l.x), int(l.y), int(l.w), 2, t.accent());
  }
  icon::draw("plus", plus_x_ + 8, y() + (h() - 16) / 2, 16, hover_plus_ ? t.bright_foreground() : t.accent());
  paint::fill(x(), y() + h() - 1, w(), 1, t.card());
  if (current_ >= 0 && current_ < int(slots_.size())) {
    RectF l = line_.value();
    paint::fill(int(l.x), y() + h() - 2, int(l.w), 2, t.accent());
  }
  fl_pop_clip();
}

// --- StatusBar ----------------------------------------------------------------

StatusBar::StatusBar(int x, int y, int w, int h) : Fl_Widget(x, y, w, h) {
  box(FL_NO_BOX);
  clear_visible_focus();
}

void StatusBar::left(const std::string &text, bool error) {
  if (text != left_ || error != error_) {
    left_ = text, error_ = error;
    redraw();
  }
}
void StatusBar::hint(const std::string &text) {
  if (text != hint_) {
    hint_ = text;
    redraw();
  }
}
void StatusBar::right(const std::string &text) {
  if (text != right_) {
    right_ = text;
    redraw();
  }
}
void StatusBar::badge(const std::string &text, Rgb color) {
  if (text != badge_ || color != badge_color_) {
    badge_ = text, badge_color_ = color;
    redraw();
  }
}

int StatusBar::handle(int event) {
  if (event == FL_PUSH && hint_cb_ && Fl::event_x() > x() + 200 && Fl::event_x() < x() + w() - 300)
    return 1;
  if (event == FL_RELEASE && hint_cb_ && Fl::event_x() > x() + 200 && Fl::event_x() < x() + w() - 300) {
    hint_cb_();
    return 1;
  }
  return 0;
}

void StatusBar::draw() {
  const Theme &t = theme();
  paint::fill(x(), y(), w(), h(), t.background());
  paint::fill(x(), y(), w(), 1, t.card());
  const int fs = 12;
  int bw = badge_.empty() ? 0 : int(text::width(badge_, text::REGULAR, fs)) + 4;
  int rw = right_.empty() ? 0 : std::min(260, int(text::width(right_, text::REGULAR, fs)) + 4);
  int right_end = x() + w() - 16;
  if (bw)
    text::draw(badge_, right_end - bw, y(), bw, h(), badge_color_, text::REGULAR, fs, FL_ALIGN_RIGHT);
  int rx = right_end - bw - (bw ? 80 : 0) - rw;
  if (rw)
    text::draw(right_, rx, y(), rw, h(), t.dim(), text::REGULAR, fs, FL_ALIGN_LEFT);
  bool wide = w() >= 788;
  int lw = wide && !hint_.empty() ? 180 : std::max(100, rx - x() - 32);
  text::draw(left_, x() + 16, y(), lw, h(), error_ ? t.red() : t.dim(), text::REGULAR, fs);
  if (wide && !hint_.empty()) {
    int hx = x() + 208, hw = rx - 24 - hx;
    if (hw > 80)
      text::draw(hint_, hx, y(), hw, h(), t.dim(), text::REGULAR, fs);
  }
}

// --- SortHeader ---------------------------------------------------------------

SortHeader::SortHeader(int x, int y, int w, int h) : Fl_Widget(x, y, w, h) {
  box(FL_NO_BOX);
  clear_visible_focus();
}

void SortHeader::columns(std::vector<Column> c) {
  cols_ = std::move(c);
  redraw();
}

void SortHeader::sorted(int column, bool descending) {
  if (column != sorted_ || descending != desc_) {
    sorted_ = column, desc_ = descending;
    redraw();
  }
}

int SortHeader::column_at(int ex) const {
  for (size_t i = 0; i < cols_.size(); ++i)
    if (ex >= x() + cols_[i].x && ex < x() + cols_[i].x + cols_[i].w)
      return int(i);
  return -1;
}

int SortHeader::handle(int event) {
  switch (event) {
  case FL_ENTER:
  case FL_MOVE: {
    int c = column_at(Fl::event_x());
    if (c != hover_) {
      hover_ = c;
      redraw();
    }
    return 1;
  }
  case FL_LEAVE:
    hover_ = -1;
    redraw();
    return 1;
  case FL_PUSH:
    return column_at(Fl::event_x()) >= 0 ? 1 : 0;
  case FL_RELEASE: {
    int c = column_at(Fl::event_x());
    if (c >= 0 && sort_ && Fl::event_inside(this))
      sort_(c);
    return 1;
  }
  }
  return 0;
}

void SortHeader::draw() {
  const Theme &t = theme();
  paint::fill(x(), y(), w(), h(), t.background());
  int ch = std::min(h(), 26);
  int cy = y() + (h() - ch) / 2;
  for (size_t i = 0; i < cols_.size(); ++i) {
    const Column &c = cols_[i];
    bool sorted = int(i) == sorted_;
    if (i == 0)
      paint::round_fill(x() + c.x - 48, cy, c.w + 48, ch, round7() ? 4 : 0, t.card());
    Rgb fg = sorted ? t.accent() : int(i) == hover_ ? t.foreground() : t.dim();
    std::string label = text::upper(c.label);
    if (sorted)
      label += desc_ ? " \xE2\x86\x93" : " \xE2\x86\x91";
    int lw = int(text::width(label, text::REGULAR, 10)) + 2;
    int total = lw + 14;
    int lx = c.right ? x() + c.x + c.w - total : (i == 0 ? x() + c.x - 42 : x() + c.x);
    icon::draw(c.icon, lx, cy + (ch - 10) / 2, 10, fg);
    text::draw(label, lx + 14, cy, lw, ch, fg, text::REGULAR, 10, FL_ALIGN_LEFT, text::Elide::None);
  }
}

// --- Helpers ------------------------------------------------------------------

void draw_action_button(int x, int y, int w, int h, const std::string &label, bool on, bool hovered,
                        bool enabled) {
  const Theme &t = theme();
  Rgb fill = on ? t.card() : hovered ? mix(t.background(), t.card(), 0.6) : t.background();
  if (fill != t.background())
    paint::round_fill(x, y, w, h, round7(), fill);
  Rgb fg = !enabled ? t.muted() : on ? t.accent() : t.dim();
  text::draw(label, x + 10, y, w - 20, h, fg, text::REGULAR, 12, FL_ALIGN_LEFT);
}

void draw_section_label(const std::string &label, int x, int y, int w, int h) {
  text::draw(text::upper(label), x, y, w, h, theme().dim(), text::REGULAR, 10);
}

} // namespace fo
