#include "fltk-omarchy/shell.h"

#include "fltk-omarchy/dialog.h"
#include "fltk-omarchy/i18n.h"
#include "fltk-omarchy/icons.h"
#include "fltk-omarchy/paint.h"
#include "fltk-omarchy/text.h"
#include "fltk-omarchy/window.h"

#include <FL/Fl.H>
#include <FL/Fl_Window.H>
#include <FL/fl_draw.H>

#include <algorithm>
#include <cmath>
#include <cstdlib>

namespace fo {

namespace {

// Files rounds controls by 7 px (and cards by 11) when the theme rounds
// at all; square themes stay square.
int round7() { return theme().radius() > 0 ? 7 : 0; }

constexpr double kRestSeconds = 2.0; // Files' TOOLTIP_DELAY

} // namespace

// --- ToolBar ------------------------------------------------------------------

namespace {
constexpr double kHoverOpenSeconds = 0.12;  // resting on the overflow button
constexpr double kHoverCloseSeconds = 0.25; // grace after the pointer left
constexpr int kDrawerRow = 32, kDrawerPad = 4;
} // namespace

ToolBar::ToolBar(int x, int y, int w, int h) : Fl_Widget(x, y, w, h) {
  box(FL_NO_BOX);
  clear_visible_focus();
}

ToolBar::~ToolBar() {
  Fl::remove_timeout(rest, this);
  Fl::remove_timeout(hover_open, this);
  Fl::remove_timeout(hover_close, this);
  Tooltip::close_all();
  if (drawer_) {
    drawer_->owner_ = nullptr;
    drawer_->close();
    drawer_ = nullptr;
  }
}

void ToolBar::refold() {
  std::vector<int> pri, grp;
  for (const auto &it : items_) {
    pri.push_back(it.priority);
    grp.push_back(it.group);
  }
  fit::RowFold f = plain_ ? fit::fold_row(pri, grp, button_, gap_, 0) : fit::fold_row(pri, grp, button_, gap_, fit_);
  if (f.slots == fold_.slots && f.folded == fold_.folded && extra_.size() == f.slots.size())
    return;
  fold_ = std::move(f);
  hot_ = -1;
  extra_.assign(fold_.slots.size(), Anim<double>(0.0));
  if (drawer_ && !fold_.overflow())
    close_overflow();
  else if (drawer_)
    drawer_->relayout();
  redraw();
}

void ToolBar::items(std::vector<Item> items) {
  bool same = items.size() == items_.size();
  items_ = std::move(items);
  if (!same) {
    fold_ = {};
    extra_.clear();
  }
  refold();
  if (drawer_) {
    // The column shows the folded items as they are now.
    drawer_->entries_ = fold_.folded;
    drawer_->relayout();
  }
  redraw();
}

void ToolBar::fit_width(int px) {
  if (px == fit_)
    return;
  fit_ = px;
  refold();
}

bool ToolBar::folded(int i) const {
  return std::find(fold_.folded.begin(), fold_.folded.end(), i) != fold_.folded.end();
}

void ToolBar::set_on(int i, bool on) {
  if (i >= 0 && i < int(items_.size()) && items_[size_t(i)].on != on) {
    items_[size_t(i)].on = on;
    redraw();
    if (drawer_)
      drawer_->relayout();
  }
}

void ToolBar::set_enabled(int i, bool on) {
  if (i >= 0 && i < int(items_.size()) && items_[size_t(i)].enabled != on) {
    items_[size_t(i)].enabled = on;
    redraw();
  }
}

const ToolBar::Item *ToolBar::slot_item(int slot) const {
  if (slot < 0 || slot >= int(fold_.slots.size()))
    return nullptr;
  int i = fold_.slots[size_t(slot)];
  return i >= 0 && i < int(items_.size()) ? &items_[size_t(i)] : nullptr;
}

int ToolBar::slot_of(int item) const {
  for (size_t s = 0; s < fold_.slots.size(); ++s)
    if (fold_.slots[s] == item)
      return int(s);
  return -1;
}

int ToolBar::width_of(int slot) const {
  if (slot < 0 || slot >= int(extra_.size()))
    return button_;
  auto &a = const_cast<Anim<double> &>(extra_[size_t(slot)]);
  return button_ + int(std::lround(std::max(0.0, a.value())));
}

int ToolBar::preferred_width() const {
  int n = int(fold_.slots.size());
  if (!n)
    return 0;
  int sum = (n - 1) * gap_;
  for (int s = 0; s < n; ++s)
    sum += width_of(s);
  return sum;
}

int ToolBar::slot_at(int ex) const {
  int n = int(fold_.slots.size());
  for (int s = 0; s < n; ++s) {
    int rx = 0, ry = 0, rw = 0, rh = 0;
    slot_rect(s, rx, ry, rw, rh);
    if (ex >= rx && ex < rx + rw)
      return s;
  }
  return -1;
}

bool ToolBar::slot_rect(int slot, int &rx, int &ry, int &rw, int &rh) const {
  int n = int(fold_.slots.size());
  if (slot < 0 || slot >= n)
    return false;
  ry = y();
  rh = h();
  if (plain_) {
    int cx = x();
    for (int k = 0; k < slot; ++k)
      cx += width_of(k) + gap_;
    rx = cx;
    rw = width_of(slot);
    return true;
  }
  int cx = x() + w();
  for (int k = n - 1; k > slot; --k)
    cx -= width_of(k) + gap_;
  rw = width_of(slot);
  rx = cx - rw;
  return true;
}

bool ToolBar::item_rect(int i, int &rx, int &ry, int &rw, int &rh) const {
  if (i < 0 || i >= int(items_.size()))
    return false;
  int s = slot_of(folded(i) ? -1 : i);
  return slot_rect(s, rx, ry, rw, rh);
}

bool ToolBar::overflow_rect(int &rx, int &ry, int &rw, int &rh) const {
  return fold_.overflow() && slot_rect(slot_of(-1), rx, ry, rw, rh);
}

int ToolBar::hot() const {
  if (hot_ < 0 || hot_ >= int(fold_.slots.size()))
    return -1;
  return fold_.slots[size_t(hot_)];
}

bool ToolBar::overflow_hot() const { return hot_ >= 0 && hot_ < int(fold_.slots.size()) && fold_.slots[size_t(hot_)] == -1; }

std::string ToolBar::overflow_icon() const {
  for (int i : fold_.folded)
    if (items_[size_t(i)].on)
      return items_[size_t(i)].icon;
  return "more";
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
  const Item *it = t->slot_item(t->hot_);
  if (!it || it->shortcut.empty() || t->drawer_)
    return;
  int rx, ry, rw, rh;
  t->slot_rect(t->hot_, rx, ry, rw, rh);
  if (Window *win = Window::of(t))
    Tooltip::open_at(win, rx, ry, rw, rh, it->shortcut);
}

void ToolBar::hover_open(void *data) {
  auto *t = static_cast<ToolBar *>(data);
  if (t->overflow_hot() && !t->drawer_)
    t->open_drawer(false, false);
}

void ToolBar::hover_close(void *data) {
  auto *t = static_cast<ToolBar *>(data);
  if (t->drawer_ && !t->drawer_->held_ && !t->drawer_->inside_ && !t->overflow_hot())
    t->close_overflow();
}

void ToolBar::set_hot(int slot) {
  if (slot == hot_)
    return;
  hot_ = slot;
  Fl::remove_timeout(rest, this);
  Fl::remove_timeout(hover_open, this);
  Tooltip::close_all();
  if (slot >= 0 && slot_item(slot))
    Fl::add_timeout(kRestSeconds, rest, this);
  if (overflow_hot()) {
    Fl::remove_timeout(hover_close, this);
    if (!drawer_)
      Fl::add_timeout(kHoverOpenSeconds, hover_open, this);
  } else if (drawer_ && !drawer_->held_) {
    Fl::remove_timeout(hover_close, this);
    Fl::add_timeout(kHoverCloseSeconds, hover_close, this);
  }
  if (!plain_) {
    // Labels reveal within the room the row may take.
    int room = allowance_;
    if (fit_ > 0)
      room = std::max(0, std::min(room, fit_ - fit::row_width(int(fold_.slots.size()), button_, gap_)));
    for (size_t k = 0; k < fold_.slots.size(); ++k) {
      double target = 0;
      if (int(k) == hot_) {
        const Item *it = slot_item(int(k));
        std::string label = it ? it->label : overflow_label_.empty() ? tr("Show more") : overflow_label_;
        if (!label.empty() && !(fold_.slots[k] == -1 && drawer_))
          target = std::min(double(room), text::width(label, text::REGULAR, 12) + 12);
      }
      extra_[k].set(target, 160, anim::Ease::OutCubic);
    }
    anim::request_frame(tick, this);
    tick(this);
  }
  redraw();
}

void ToolBar::open_drawer(bool hold, bool keyboard) {
  Window *win = Window::of(this);
  if (!win || !fold_.overflow())
    return;
  Fl::remove_timeout(hover_open, this);
  Fl::remove_timeout(hover_close, this);
  Fl::remove_timeout(rest, this);
  Tooltip::close_all();
  if (!drawer_) {
    auto *d = new ToolDrawer();
    d->owner_ = this;
    d->entries_ = fold_.folded;
    drawer_ = d;
    d->open(win);
    d->fade_in(anim::ms::POPUP_IN, anim::Ease::OutCubic);
    // The overflow button keeps its icon while the column hangs from it.
    for (size_t k = 0; k < fold_.slots.size(); ++k)
      if (fold_.slots[k] == -1)
        extra_[k].set(0, 160, anim::Ease::OutCubic);
    anim::request_frame(tick, this);
  }
  if (hold)
    drawer_->hold(keyboard);
  redraw();
}

bool ToolBar::open_overflow(bool keyboard) {
  if (!fold_.overflow() || !Window::of(this))
    return false;
  open_drawer(true, keyboard);
  return true;
}

void ToolBar::close_overflow() {
  if (drawer_) {
    ToolDrawer *d = drawer_;
    drawer_ = nullptr;
    d->owner_ = nullptr;
    d->close();
  }
  redraw();
}

void ToolBar::drawer_closed() {
  drawer_ = nullptr;
  redraw();
}

void ToolBar::activate(int i) {
  if (i < 0 || i >= int(items_.size()))
    return;
  const Item &it = items_[size_t(i)];
  if (it.enabled && it.action) {
    auto fn = it.action; // the action may replace the items
    fn();
  }
}

int ToolBar::handle(int event) {
  switch (event) {
  case FL_ENTER:
  case FL_MOVE: {
    set_hot(slot_at(Fl::event_x()));
    return 1;
  }
  case FL_LEAVE:
  case FL_HIDE:
    set_hot(-1);
    if (event == FL_HIDE)
      close_overflow();
    return event == FL_LEAVE ? 1 : 0;
  case FL_PUSH: {
    Fl::remove_timeout(rest, this);
    Tooltip::close_all();
    int s = slot_at(Fl::event_x());
    if (s < 0 || Fl::event_button() != FL_LEFT_MOUSE)
      return 0;
    pressed_ = s;
    return 1;
  }
  case FL_RELEASE: {
    int s = slot_at(Fl::event_x());
    int p = pressed_;
    pressed_ = -1;
    if (s == p && s >= 0 && Fl::event_inside(this)) {
      int i = fold_.slots[size_t(s)];
      if (i == -1) {
        // A click holds the column open (or opens it held).
        open_drawer(true, false);
      } else {
        activate(i);
      }
    }
    return 1;
  }
  }
  return 0;
}

void ToolBar::draw() {
  const Theme &t = theme();
  int n = int(fold_.slots.size());
  for (int s = 0; s < n; ++s) {
    const int i = fold_.slots[size_t(s)];
    int rx, ry, rw, rh;
    slot_rect(s, rx, ry, rw, rh);
    if (i == -1) {
      // The overflow button: the folded "on" item's icon on a card (the
      // current view), or "more"; a small chevron says it drops down.
      std::string ic = overflow_icon();
      bool on = ic != "more" || drawer_ != nullptr;
      bool hot = s == hot_ || drawer_ != nullptr;
      if (on)
        paint::round_fill(rx, ry, rw, rh, round7(), t.card());
      Rgb c = on || hot ? t.accent() : t.dim();
      icon::draw(ic, rx + rw - 26 + 3, ry + (rh - 18) / 2 - 2, 18, c);
      icon::draw("chevron-down", rx + rw - 26 + 3 + 6, ry + rh - 11, 8, c);
      if (rw > button_ + 8) {
        const std::string label = overflow_label_.empty() ? tr("Show more") : overflow_label_;
        fl_push_clip(rx + 8, ry, rw - 40, rh);
        text::draw(label, rx + 8, ry, rw - 40 + 4, rh, c, text::REGULAR, 12, FL_ALIGN_LEFT, text::Elide::None);
        fl_pop_clip();
      }
      continue;
    }
    const Item &it = items_[size_t(i)];
    bool hot = s == hot_ && it.enabled;
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

// --- ToolDrawer -----------------------------------------------------------------

void ToolDrawer::relayout() {
  Window *win = host() ? host() : nullptr;
  if (!owner_ || !win)
    return;
  int ax, ay, aw, ah;
  if (!owner_->overflow_rect(ax, ay, aw, ah))
    return;
  shown_.clear();
  for (int i : entries_)
    shown_.push_back(owner_->items_[size_t(i)]);
  // As wide as the longest label and shortcut, in Files' menu metrics.
  int label_w = 0, keys_w = 0;
  for (const ToolBar::Item &it : shown_) {
    label_w = std::max(label_w, int(text::width(it.label, text::REGULAR, 12)) + 1);
    if (!it.shortcut.empty())
      keys_w = std::max(keys_w, int(text::width(it.shortcut, text::REGULAR, 10)) + 1);
  }
  cw_ = std::max(aw + 40, 12 + 26 + label_w + (keys_w ? 18 + keys_w : 0) + 30);
  cw_ = std::min(cw_, std::max(80, win->w() - 16));
  ch_ = 2 * kDrawerPad + int(entries_.size()) * kDrawerRow;
  cy_ = ay + ah + 4;
  if (cy_ + ch_ > win->h() - 4 && ay - 4 - ch_ >= 4)
    cy_ = ay - 4 - ch_; // no room below: up
  cx_ = rtl() ? ax : ax + aw - cw_;
  cx_ = std::max(8, std::min(cx_, win->w() - 8 - cw_));
  // Held, it covers the window to catch the click that closes it; opened
  // by hovering, only its card, so the pointer still reaches the header.
  if (held_)
    resize(0, 0, win->w(), win->h());
  else
    resize(cx_, cy_, cw_, ch_);
  if (hot_ >= int(entries_.size()))
    hot_ = -1;
  win->redraw();
}

void ToolDrawer::hold(bool keyboard) {
  held_ = true;
  if (keyboard && hot_ < 0 && !entries_.empty()) {
    hot_ = 0;
    // Start on the entry that is on (the current view), when one is.
    for (size_t k = 0; k < entries_.size(); ++k)
      if (owner_ && owner_->items_[size_t(entries_[k])].on)
        hot_ = int(k);
  }
  relayout();
}

void ToolDrawer::closing() {
  Fl::remove_timeout(ToolBar::hover_close, owner_);
  if (owner_)
    owner_->drawer_closed();
  owner_ = nullptr;
}

bool ToolDrawer::entry_rect(int i, int &x, int &y, int &w, int &h) const {
  if (i < 0 || i >= int(std::max(entries_.size(), shown_.size())))
    return false;
  x = cx_ + 2;
  y = cy_ + kDrawerPad + i * kDrawerRow;
  w = cw_ - 4;
  h = kDrawerRow;
  return true;
}

int ToolDrawer::entry_at(int ex, int ey) const {
  if (ex < cx_ || ex >= cx_ + cw_ || ey < cy_ + kDrawerPad || ey >= cy_ + ch_ - kDrawerPad)
    return -1;
  int i = (ey - cy_ - kDrawerPad) / kDrawerRow;
  return i >= 0 && i < int(entries_.size()) ? i : -1;
}

void ToolDrawer::move(int dir) {
  int n = int(entries_.size());
  if (!n || !owner_)
    return;
  int i = hot_;
  for (int step = 0; step < n; ++step) {
    i = i < 0 ? (dir > 0 ? 0 : n - 1) : (i + dir + n) % n;
    if (owner_->items_[size_t(entries_[size_t(i)])].enabled)
      break;
  }
  hot_ = i;
  redraw_all();
}

void ToolDrawer::choose(int i) {
  if (!owner_ || i < 0 || i >= int(entries_.size()))
    return;
  ToolBar *owner = owner_;
  int item = entries_[size_t(i)];
  if (!owner->items_[size_t(item)].enabled)
    return;
  owner->close_overflow(); // closes this layer (deleted later)
  owner->activate(item);
}

int ToolDrawer::handle(int event) {
  if (!owner_)
    return 0;
  const int ex = Fl::event_x(), ey = Fl::event_y();
  const bool in_card = ex >= cx_ && ex < cx_ + cw_ && ey >= cy_ && ey < cy_ + ch_;
  switch (event) {
  case FL_ENTER:
  case FL_MOVE:
  case FL_DRAG: {
    inside_ = in_card;
    if (in_card)
      Fl::remove_timeout(ToolBar::hover_close, owner_);
    int i = entry_at(ex, ey);
    if (i != hot_ && (in_card || !held_)) {
      hot_ = i;
      redraw_all();
    }
    if (!in_card && held_) {
      // Held over the window: the overflow button under it still shows hover.
      return 1;
    }
    return in_card ? 1 : 0;
  }
  case FL_LEAVE:
    inside_ = false;
    if (!held_) {
      hot_ = -1;
      Fl::remove_timeout(ToolBar::hover_close, owner_);
      Fl::add_timeout(kHoverCloseSeconds, ToolBar::hover_close, owner_);
      redraw_all();
    }
    return 1;
  case FL_PUSH:
    if (!in_card) {
      if (held_) {
        // A click outside closes it, as menus do (and is used up).
        owner_->close_overflow();
        return 1;
      }
      return 0;
    }
    pressed_inside_ = true;
    return 1;
  case FL_RELEASE: {
    bool was = pressed_inside_;
    pressed_inside_ = false;
    if (was && in_card)
      choose(entry_at(ex, ey));
    return 1;
  }
  case FL_KEYBOARD:
  case FL_SHORTCUT: {
    if (!held_)
      return 0;
    int k = Fl::event_key();
    switch (k) {
    case FL_Up: move(-1); return 1;
    case FL_Down: move(1); return 1;
    case FL_Home: hot_ = -1; move(1); return 1;
    case FL_End: hot_ = -1; move(-1); return 1;
    case FL_Enter: case FL_KP_Enter: case ' ': choose(hot_); return 1;
    case FL_Escape: owner_->close_overflow(); return 1;
    }
    return 0; // anything else closes it and reaches the app (passes_keys)
  }
  }
  return 0;
}

void ToolDrawer::draw() {
  const Theme &t = theme();
  // It drops down: the card's visible height follows the presence.
  double p = presence();
  int vis = std::max(1, int(std::lround(ch_ * p)));
  int r = round7();
  fl_push_clip(cx_, cy_, cw_, vis);
  paint::round_fill(cx_, cy_, cw_, ch_, r, t.accent());
  paint::round_fill(cx_ + 1, cy_ + 1, cw_ - 2, ch_ - 2, r, t.background());
  {
    // From the copy taken at the last layout, so the entries stay while
    // the card retracts after a choice.
    bool mirror = rtl();
    for (size_t k = 0; k < shown_.size(); ++k) {
      const ToolBar::Item &it = shown_[k];
      int x = 0, y = 0, w = 0, h = 0;
      entry_rect(int(k), x, y, w, h);
      bool hi = int(k) == hot_ && it.enabled;
      if (it.on || hi)
        paint::round_fill(x, y, w, h, r ? 5 : 0, t.card());
      Rgb fg = !it.enabled ? t.muted() : it.on ? t.accent() : hi ? t.bright_foreground() : t.foreground();
      auto mx = [&](int x0, int w0) { return mirror ? cx_ + cw_ - (x0 - cx_) - w0 : x0; };
      int lx = cx_ + 12;
      icon::draw(it.icon, mx(lx, 16), y + (h - 16) / 2, 16, it.on ? t.accent() : fg);
      lx += 26;
      int right = cx_ + cw_ - 12;
      if (it.on) {
        icon::draw("check", mx(right - 12, 12), y + (h - 12) / 2, 12, t.accent());
        right -= 20;
      }
      if (!it.shortcut.empty()) {
        int sw = int(text::width(it.shortcut, text::REGULAR, 10) + 1);
        text::draw(it.shortcut, mx(right - sw, sw), y, sw, h, t.dim(), text::REGULAR, 10,
                   mirror ? FL_ALIGN_LEFT : FL_ALIGN_RIGHT, text::Elide::None);
        right -= sw + 14;
      }
      text::draw(it.label, mx(lx, right - lx), y, right - lx, h, fg, text::REGULAR, 12,
                 mirror ? FL_ALIGN_RIGHT : FL_ALIGN_LEFT);
    }
  }
  fl_pop_clip();
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
    int tw = int(text::width(labels_[size_t(i)], text::REGULAR, 12)) + 24 + 22;
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
  case FL_PUSH: {
    int c = -1;
    pressed_ = Fl::event_button() == FL_LEFT_MOUSE ? at(c) : -1;
    if (c >= 0)
      pressed_ = -1; // Files: a press on the close button never drags
    press_x_ = ex, press_y_ = ey;
    dragging_ = false;
    drop_at_ = -1;
    return 1;
  }
  case FL_DRAG: {
    // Files: after 5 px of travel the tab is dragged; the drop point is
    // before or after the tab under the pointer (its halves), none outside
    // the strip.
    if (pressed_ < 0 || !reorder_)
      return 1;
    if (!dragging_ && std::max(std::abs(ex - press_x_), std::abs(ey - press_y_)) < 5)
      return 1;
    dragging_ = true;
    int c;
    int i = ey >= y() && ey < y() + h() ? at(c) : -1;
    int drop = i < 0 ? -1 : i + (ex >= slots_[size_t(i)].x + slots_[size_t(i)].w / 2 ? 1 : 0);
    if (drop != drop_at_) {
      drop_at_ = drop;
      redraw();
    }
    return 1;
  }
  case FL_RELEASE: {
    int c;
    int i = at(c);
    const bool dragged = dragging_;
    const int from = pressed_, drop = drop_at_;
    pressed_ = drop_at_ = -1;
    dragging_ = false;
    if (dragged) {
      redraw();
      if (from >= 0 && drop >= 0 && Fl::event_inside(this) && reorder_) {
        int to = drop - (from < drop ? 1 : 0);
        if (to != from)
          reorder_(from, to);
      }
      return 1;
    }
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
    text::draw(labels_[i], s.x + 12, y(), s.w - 12 - 24, h(), fg, text::REGULAR, 12);
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
  if (dragging_ && drop_at_ >= 0 && !slots_.empty()) {
    // Files: a 2 px accent line where the dragged tab would land.
    const Slot &s = slots_[size_t(std::min(drop_at_, int(slots_.size()) - 1))];
    int edge = drop_at_ >= int(slots_.size()) ? s.x + s.w : s.x;
    paint::fill(edge - 1, y() + 2, 2, std::max(1, h() - 4), t.accent());
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
  const bool on_hint = hint_cb_ && hint_w_ > 0 && Fl::event_x() >= hint_x_ && Fl::event_x() < hint_x_ + hint_w_;
  if (event == FL_PUSH && on_hint)
    return 1;
  if (event == FL_RELEASE && on_hint) {
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
  auto need = [&](const std::string &s) { return s.empty() ? 0 : int(text::width(s, text::REGULAR, fs)) + 4; };
  // By priority, never overlapping (fit::status_row): the counts, the
  // badge, the free space, then the tip in what is left.
  fit::StatusRow r = fit::status_row(x(), w(), need(left_), need(hint_), need(right_), need(badge_));
  if (r.badge_w)
    text::draw(badge_, r.badge_x, y(), r.badge_w, h(), badge_color_, text::REGULAR, fs, FL_ALIGN_RIGHT);
  if (r.right_w)
    text::draw(right_, r.right_x, y(), r.right_w, h(), t.dim(), text::REGULAR, fs, FL_ALIGN_LEFT);
  if (r.left_w)
    text::draw(left_, r.left_x, y(), r.left_w, h(), error_ ? t.red() : t.dim(), text::REGULAR, fs);
  hint_x_ = r.hint_x, hint_w_ = r.hint_w;
  if (r.hint_w)
    text::draw(hint_, r.hint_x, y(), r.hint_w, h(), t.dim(), text::REGULAR, fs);
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
