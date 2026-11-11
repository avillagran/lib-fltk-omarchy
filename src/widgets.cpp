#include "fltk-omarchy/widgets.h"

#include "fltk-omarchy/app.h"
#include "fltk-omarchy/dialog.h"
#include "fltk-omarchy/i18n.h"
#include "fltk-omarchy/icons.h"
#include "fltk-omarchy/menu.h"
#include "fltk-omarchy/paint.h"
#include "fltk-omarchy/text.h"
#include "fltk-omarchy/window.h"

#include <FL/Fl.H>
#include <FL/Fl_Window.H>
#include <FL/fl_draw.H>

#include <algorithm>
#include <cmath>

namespace fo {

static bool has_focus(Fl_Widget *w) { return Fl::focus() == w; }

// --- Button -----------------------------------------------------------------

Button::Button(int x, int y, int w, int h, const char *label) : Fl_Button(x, y, w, h, label) {
  box(FL_NO_BOX);
  callback(cb, this);
  labelfont(text::REGULAR);
  labelsize(text::size::BODY);
}

void Button::cb(Fl_Widget *, void *data) {
  auto *b = static_cast<Button *>(data);
  if (b->click_)
    b->click_();
}

int Button::preferred_width() const {
  return std::max(88, int(text::width(label() ? label() : "", primary_ ? text::SEMIBOLD : text::REGULAR,
                                      text::size::BODY)) + 32);
}

int Button::handle(int event) {
  switch (event) {
  case FL_ENTER:
  case FL_LEAVE:
    hovered_ = event == FL_ENTER;
    redraw();
    Fl_Button::handle(event);
    return 1;
  case FL_FOCUS:
  case FL_UNFOCUS:
    redraw();
    return Fl_Button::handle(event);
  case FL_KEYBOARD:
    if (has_focus(this) && (Fl::event_key() == FL_Enter || Fl::event_key() == FL_KP_Enter)) {
      do_callback();
      return 1;
    }
    break;
  }
  return Fl_Button::handle(event);
}

void Button::draw() {
  const Theme &t = theme();
  Rgb target = primary_ ? (hovered_ ? mix(t.accent(), Rgb(255, 255, 255), 0.08) : t.accent())
                        : t.tint(t.foreground(), hovered_ || value() ? 0.10 : 0.05);
  fill_.set(target, anim::ms::BUTTON, anim::Ease::Linear);
  Rgb fill = fill_.get(this);
  if (!active_r())
    fill = t.tint(t.foreground(), 0.03);
  paint::round_fill(x(), y(), w(), h(), t.radius_small(), fill);
  if (has_focus(this) && focus_visible())
    paint::round_frame(x(), y(), w(), h(), t.radius_small(), 1,
                       primary_ ? t.accent_foreground() : t.accent());
  Rgb fg = !active_r() ? t.secondary_foreground() : primary_ ? t.accent_foreground() : t.foreground();
  text::draw(label() ? label() : "", x() + 8, y(), w() - 16, h(), fg, primary_ ? text::SEMIBOLD : text::REGULAR,
             labelsize(), FL_ALIGN_CENTER);
}

// --- CheckBox -----------------------------------------------------------------

CheckBox::CheckBox(int x, int y, int w, int h, const char *label) : Fl_Button(x, y, w, h, label) {
  box(FL_NO_BOX);
  type(FL_TOGGLE_BUTTON);
  labelfont(text::REGULAR);
  labelsize(text::size::BODY);
  callback(cb, this);
}

void CheckBox::cb(Fl_Widget *, void *data) {
  auto *c = static_cast<CheckBox *>(data);
  c->redraw();
  if (c->change_)
    c->change_(c->checked());
}

int CheckBox::preferred_width() const {
  return 16 + 10 + int(text::width(label() ? label() : "", text::REGULAR, text::size::BODY)) + 4;
}

int CheckBox::handle(int event) {
  switch (event) {
  case FL_ENTER:
  case FL_LEAVE:
    hovered_ = event == FL_ENTER;
    redraw();
    Fl_Button::handle(event);
    return 1;
  case FL_FOCUS:
  case FL_UNFOCUS:
    redraw();
    return Fl_Button::handle(event);
  case FL_KEYBOARD:
    if (has_focus(this) && (Fl::event_key() == FL_Enter || Fl::event_key() == FL_KP_Enter)) {
      value(!value());
      do_callback();
      return 1;
    }
    break;
  }
  return Fl_Button::handle(event);
}

void CheckBox::draw() {
  const Theme &t = theme();
  const int s = 16;
  int bx = x(), by = y() + (h() - s) / 2;
  int r = std::min(3, t.radius_small());
  if (value()) {
    paint::round_fill(bx, by, s, s, r, t.accent());
    paint::line(bx + 3.5, by + 8.5, bx + 6.5, by + 11.5, 2, t.accent_foreground());
    paint::line(bx + 6.5, by + 11.5, bx + 12.5, by + 4.5, 2, t.accent_foreground());
  } else {
    paint::round_fill(bx, by, s, s, r, t.dark_background());
    paint::round_frame(bx, by, s, s, r, 1, hovered_ ? t.foreground() : t.tint(t.foreground(), 0.35));
  }
  if (has_focus(this) && focus_visible())
    paint::round_frame(x() - 3, y(), w() + 6, h(), t.radius_small(), 1, t.accent());
  text::draw(label() ? label() : "", bx + s + 10, y(), w() - s - 10, h(), active_r() ? t.foreground() : t.secondary_foreground(),
             text::REGULAR, labelsize());
}

// --- IconButton ---------------------------------------------------------------

IconButton::IconButton(int x, int y, int w, int h, const char *icon_name, const char *tip)
    : Fl_Button(x, y, w, h), icon_(icon_name ? icon_name : ""), tip_(tip ? tip : "") {
  box(FL_NO_BOX);
  callback(cb, this);
  clear_visible_focus();
}

IconButton::~IconButton() { Fl::remove_timeout(show_tip, this); }

void IconButton::cb(Fl_Widget *, void *data) {
  auto *b = static_cast<IconButton *>(data);
  Tooltip::close_all();
  if (b->click_)
    b->click_();
}

void IconButton::show_tip(void *data) {
  auto *b = static_cast<IconButton *>(data);
  if (b->hovered_ && !b->tip_.empty())
    Tooltip::open_for(b, b->tip_);
}

int IconButton::handle(int event) {
  switch (event) {
  case FL_ENTER:
    hovered_ = true;
    redraw();
    Fl::remove_timeout(show_tip, this);
    Fl::add_timeout(anim::ms::TOOLTIP_DELAY / 1000.0, show_tip, this);
    Fl_Button::handle(event);
    return 1;
  case FL_LEAVE:
  case FL_HIDE:
    hovered_ = false;
    redraw();
    Fl::remove_timeout(show_tip, this);
    Tooltip::close_all();
    Fl_Button::handle(event);
    return 1;
  case FL_PUSH:
    Fl::remove_timeout(show_tip, this);
    Tooltip::close_all();
    break;
  case FL_KEYBOARD:
    if (has_focus(this) && (Fl::event_key() == FL_Enter || Fl::event_key() == FL_KP_Enter)) {
      do_callback();
      return 1;
    }
    break;
  case FL_FOCUS:
  case FL_UNFOCUS:
    redraw();
    break;
  }
  return Fl_Button::handle(event);
}

void IconButton::draw() {
  const Theme &t = theme();
  Rgb bg = t.background();
  Rgb target = active_ ? t.tint(t.accent(), 0.16)
               : (hovered_ || value()) && active_r() ? t.tint(t.foreground(), 0.08)
                                                   : bg;
  fill_.set(target, anim::ms::BUTTON, anim::Ease::Linear);
  Rgb fill = fill_.get(this);
  if (fill != bg)
    paint::round_fill(x(), y(), w(), h(), t.radius_small(), fill);
  if (has_focus(this) && focus_visible())
    paint::round_frame(x(), y(), w(), h(), t.radius_small(), 1, t.accent());
  Rgb fg = !active_r() ? t.tint(t.foreground(), 0.35) : active_ ? t.bright_foreground() : t.foreground();
  int s = std::min(18, std::min(w(), h()) - 8);
  icon::draw(icon_, x() + (w() - s) / 2, y() + (h() - s) / 2, s, fg);
}

// --- TextField ----------------------------------------------------------------

TextField::TextField(int x, int y, int w, int h, const char *label) : Fl_Input(x, y, w, h, label) {
  // Fl_Input_ erases changed text on minimal updates by drawing box() in
  // color(): a flat box, or typing leaves old glyphs behind. draw() paints
  // the rounded frame itself; the erase stays inside the text area.
  box(FL_FLAT_BOX);
  textfont(text::REGULAR);
  textsize(text::size::BODY);
  when(FL_WHEN_CHANGED);
  callback(cb, this);
}

void TextField::cb(Fl_Widget *, void *data) {
  auto *f = static_cast<TextField *>(data);
  if (f->change_)
    f->change_();
}

void TextField::secret(bool on) {
  input_type(on ? FL_SECRET_INPUT : FL_NORMAL_INPUT);
  redraw();
}

void TextField::set_text(const std::string &t, int from, int to) {
  value(t.c_str());
  int end = to < 0 ? size() : std::min(to, size());
  insert_position(end, std::min(std::max(from, 0), end));
}

int TextField::handle(int event) {
  if (event == FL_KEYBOARD && has_focus(this)) {
    int k = Fl::event_key();
    if (key_ && key_(k))
      return 1;
    if (k == FL_Escape && escape_) {
      escape_();
      return 1;
    }
    if ((k == FL_Enter || k == FL_KP_Enter) && enter_) {
      enter_();
      return 1;
    }
    if (k == FL_Tab && complete_ && !(Fl::event_state() & (FL_SHIFT | FL_CTRL | FL_ALT))) {
      std::string done = complete_(value());
      value(done.c_str());
      insert_position(size());
      if (change_)
        change_();
      return 1;
    }
  }
  if (event == FL_FOCUS || event == FL_UNFOCUS)
    redraw();
  return Fl_Input::handle(event);
}

void TextField::draw() {
  const Theme &t = theme();
  // Fl_Input_ erases the text area with color() and draws the selection
  // with selection_color(): keep them in the theme.
  color(t.dark_background().fl());
  textcolor(t.bright_foreground().fl());
  selection_color(t.accent().fl());
  cursor_color(t.accent().fl());
  if (damage() & FL_DAMAGE_ALL) {
    paint::round_fill(x(), y(), w(), h(), t.radius_small(), t.dark_background());
    paint::round_frame(x(), y(), w(), h(), t.radius_small(), 1,
                       has_focus(this) ? t.accent() : t.tint(t.foreground(), 0.18));
  }
  int px = 12, py = std::max(2, (h() - textsize() - 8) / 2);
  drawtext(x() + px, y() + py, w() - 2 * px, h() - 2 * py);
  if (size() == 0 && !placeholder_.empty() && !has_focus(this))
    text::draw(placeholder_, x() + px, y(), w() - 2 * px, h(), t.secondary_foreground(), textfont(), textsize());
  else if (size() == 0 && !placeholder_.empty())
    text::draw(placeholder_, x() + px + 2, y(), w() - 2 * px - 2, h(), t.secondary_foreground(), textfont(),
               textsize());
}

// --- Breadcrumbs ----------------------------------------------------------------

// Files' path chips (breadcrumb_ui.rs): 24 px chips on a card, the current
// folder on the accent, "›" between them, the ancestors that do not fit
// folded into a leading "…" chip that opens a menu of them.
static const char *kFold = "\xE2\x80\xA6";
static const char *kSep = "\xE2\x80\xBA";
static const int kChipH = 24, kSepW = 18, kChipPad = 8, kChipMax = 180, kCurrentMax = 260;

static int chip_round() { return theme().radius() > 0 ? 7 : 0; }

Breadcrumbs::Breadcrumbs(int x, int y, int w, int h) : Fl_Widget(x, y, w, h) {
  box(FL_NO_BOX);
  set_visible_focus();
}

void Breadcrumbs::segments(std::vector<Segment> s) {
  segs_ = std::move(s);
  cursor_ = -2;
  hover_ = -2;
  layout_hits();
  redraw();
}

void Breadcrumbs::layout_hits() {
  hits_.clear();
  int n = int(segs_.size());
  if (!n)
    return;
  auto chip_w = [&](int i) {
    int max = i == n - 1 ? kCurrentMax : kChipMax;
    return std::min(max, int(text::width(segs_[size_t(i)].label, text::REGULAR, 12) + 0.5) + 2 * kChipPad);
  };
  int fold = int(text::width(kFold, text::REGULAR, 12) + 0.5) + 2 * 10;
  // Keep as many trailing chips as fit; the rest fold into "…".
  first_ = n - 1;
  int used = chip_w(n - 1);
  while (first_ > 0) {
    int add = chip_w(first_ - 1) + kSepW;
    int reserve = first_ - 1 > 0 ? fold + kSepW : 0;
    if (used + add + reserve > w())
      break;
    used += add;
    --first_;
  }
  int cx = x();
  bool mirror = rtl();
  auto push = [&](int index, int width) {
    hits_.push_back(Hit{mirror ? x() + w() - (cx - x()) - width : cx, width, index});
    cx += width + kSepW;
  };
  if (first_ > 0)
    push(-1, fold);
  for (int i = first_; i < n; ++i)
    push(i, std::min(chip_w(i), std::max(24, x() + w() - cx)));
}

void Breadcrumbs::activate(int index) {
  if (index == -1) {
    Window *win = Window::of(this);
    if (!win)
      return;
    std::vector<MenuItem> items;
    for (int i = first_ - 1; i >= 0; --i)
      items.push_back(MenuItem(segs_[size_t(i)].label, "", [this, i] { activate(i); }).with_icon("folder"));
    for (const auto &hit : hits_)
      if (hit.index == -1)
        PopupMenu::open_below(win, hit.x, y() + (h() - kChipH) / 2, hit.w, kChipH, items);
    return;
  }
  if (index >= 0 && index < int(segs_.size()) - 1 && navigate_)
    navigate_(index);
}

void Breadcrumbs::open_fold() {
  for (const auto &hit : hits_)
    if (hit.index == -1) {
      activate(-1);
      return;
    }
}

int Breadcrumbs::handle(int event) {
  auto hit_at = [&](int ex) {
    for (const auto &h : hits_)
      if (ex >= h.x && ex < h.x + h.w)
        return h.index;
    return -2;
  };
  switch (event) {
  case FL_ENTER:
  case FL_MOVE: {
    int hv = hit_at(Fl::event_x());
    if (hv == int(segs_.size()) - 1)
      hv = -2;
    if (hv != hover_) {
      hover_ = hv;
      window()->cursor(hv != -2 ? FL_CURSOR_HAND : FL_CURSOR_DEFAULT);
      redraw();
    }
    return 1;
  }
  case FL_LEAVE:
    hover_ = -2;
    window()->cursor(FL_CURSOR_DEFAULT);
    redraw();
    return 1;
  case FL_PUSH:
    if (Fl::event_button() != FL_LEFT_MOUSE)
      return 0;
    // The current chip and the empty field belong to the owner (Files:
    // clicking the address edits the path).
    if (hit_at(Fl::event_x()) == -2 || hit_at(Fl::event_x()) == int(segs_.size()) - 1) {
      if (empty_click_) {
        empty_click_();
        return 1;
      }
      return 0;
    }
    return 1;
  case FL_RELEASE: {
    int hv = hit_at(Fl::event_x());
    if (hv != -2 && Fl::event_inside(this))
      activate(hv);
    return 1;
  }
  case FL_FOCUS:
    cursor_ = hits_.empty() ? -2 : hits_.back().index;
    redraw();
    return 1;
  case FL_UNFOCUS:
    redraw();
    return 1;
  case FL_KEYBOARD: {
    int k = Fl::event_key();
    // Alt+Left and the like belong to the window (back, forward).
    if (hits_.empty() || Fl::event_state(FL_CTRL | FL_ALT | FL_META))
      return 0;
    size_t at = hits_.size() - 1;
    for (size_t i = 0; i < hits_.size(); ++i)
      if (hits_[i].index == cursor_)
        at = i;
    // Left goes toward the start of the trail, which is on the right
    // when the language reads right to left.
    bool back = (k == FL_Left) != rtl();
    if ((k == FL_Left || k == FL_Right) && back && at > 0) {
      cursor_ = hits_[at - 1].index;
      redraw();
      return 1;
    }
    if ((k == FL_Left || k == FL_Right) && !back && at + 1 < hits_.size()) {
      cursor_ = hits_[at + 1].index;
      redraw();
      return 1;
    }
    if (k == FL_Enter || k == FL_KP_Enter || k == ' ') {
      activate(cursor_);
      return 1;
    }
    return 0;
  }
  }
  return 0;
}

void Breadcrumbs::draw() {
  const Theme &t = theme();
  layout_hits(); // cheap; follows resizes and font changes
  fl_push_clip(x(), y(), w(), h());
  int n = int(segs_.size());
  bool keyed = has_focus(this) && focus_visible();
  bool mirror = rtl();
  int cy = y() + (h() - kChipH) / 2;
  for (size_t i = 0; i < hits_.size(); ++i) {
    const Hit &hh = hits_[i];
    if (i > 0) // the separator before each chip, in reading order
      text::draw(kSep, mirror ? hh.x + hh.w : hh.x - kSepW, y(), kSepW, h(), t.dim(), text::REGULAR, 12,
                 FL_ALIGN_CENTER, text::Elide::None);
    bool current = hh.index == n - 1;
    Rgb fill = current ? t.accent() : hh.index == hover_ ? mix(t.background(), t.foreground(), 0.16) : t.card();
    paint::round_fill(hh.x, cy, hh.w, kChipH, chip_round(), fill);
    Rgb fg = current ? t.background() : hh.index == hover_ ? t.bright_foreground() : t.foreground();
    const std::string &label = hh.index == -1 ? std::string(kFold) : segs_[size_t(hh.index)].label;
    text::draw(label, hh.x + kChipPad, y(), hh.w - 2 * kChipPad, h(), fg, text::REGULAR, 12,
               hh.index == -1 ? FL_ALIGN_CENTER : (mirror ? FL_ALIGN_RIGHT : FL_ALIGN_LEFT));
    if (keyed && hh.index == cursor_)
      paint::round_frame(hh.x - 2, cy - 2, hh.w + 4, kChipH + 4, chip_round(), 1, t.accent());
  }
  fl_pop_clip();
}

// --- Rule -----------------------------------------------------------------------

void Rule::draw() { paint::fill(x(), y(), w(), h(), theme().hairline()); }

// --- SplitPane ------------------------------------------------------------------

SplitPane::SplitPane(int x, int y, int w, int h, Orientation o) : Fl_Group(x, y, w, h), o_(o) {
  box(FL_NO_BOX);
  split_ = (o == Horizontal ? w : h) / 3;
}

void SplitPane::apply(int px) {
  int total = o_ == Horizontal ? w() : h();
  if (!collapsed_)
    px = std::max(min1_, std::min(px, total - min2_));
  px = std::max(0, std::min(px, total));
  split_ = px;
  Fl_Widget *a = children() > 0 ? child(0) : nullptr;
  Fl_Widget *b = children() > 1 ? child(1) : nullptr;
  rtl_ = rtl();
  if (o_ == Horizontal && rtl_) { // the first pane on the right
    if (a) a->resize(x() + w() - px, y(), px, h());
    if (b) b->resize(x(), y(), std::max(0, w() - px - 1), h());
  } else if (o_ == Horizontal) {
    if (a) a->resize(x(), y(), px, h());
    if (b) b->resize(x() + px + 1, y(), std::max(0, w() - px - 1), h());
  } else {
    if (a) a->resize(x(), y(), w(), px);
    if (b) b->resize(x(), y() + px + 1, w(), std::max(0, h() - px - 1));
  }
  if (b) {
    if ((o_ == Horizontal ? b->w() : b->h()) <= 0) b->hide();
    else if (!b->visible()) b->show();
  }
  redraw();
}

void SplitPane::step(void *data) {
  auto *s = static_cast<SplitPane *>(data);
  s->apply(int(std::lround(s->motion_.value())));
  if (s->motion_.running())
    anim::request_frame(step, s);
}

void SplitPane::set_split(int px, bool animate) {
  if (animate && anim::enabled()) {
    motion_.jump(split_);
    motion_.set(px, anim::ms::LAYOUT);
    anim::request_frame(step, this);
  } else {
    apply(px);
  }
}

void SplitPane::collapse_second(bool on, bool animate) {
  if (on == collapsed_)
    return;
  int total = o_ == Horizontal ? w() : h();
  if (on) {
    before_collapse_ = split_;
    collapsed_ = true;
    set_split(total, animate);
  } else {
    collapsed_ = false;
    set_split(before_collapse_ ? before_collapse_ : total * 2 / 3, animate);
  }
}

void SplitPane::resize(int X, int Y, int W, int H) {
  int old_total = o_ == Horizontal ? w() : h();
  int second = old_total - split_;
  Fl_Widget::resize(X, Y, W, H);
  int total = o_ == Horizontal ? W : H;
  // The second pane keeps its size (a details pane), the first one grows.
  apply(collapsed_ ? total : total - second);
}

int SplitPane::handle(int event) {
  int pos = o_ == Horizontal ? (rtl_ ? x() + w() - Fl::event_x() : Fl::event_x() - x()) : Fl::event_y() - y();
  bool on_divider = !collapsed_ && std::abs(pos - split_) <= 3;
  switch (event) {
  case FL_MOVE:
  case FL_ENTER:
    if (on_divider != hover_) {
      hover_ = on_divider;
      window()->cursor(on_divider ? (o_ == Horizontal ? FL_CURSOR_WE : FL_CURSOR_NS) : FL_CURSOR_DEFAULT);
    }
    if (on_divider)
      return 1;
    break;
  case FL_LEAVE:
    if (hover_) {
      hover_ = false;
      window()->cursor(FL_CURSOR_DEFAULT);
    }
    break;
  case FL_PUSH:
    if (on_divider && Fl::event_button() == FL_LEFT_MOUSE) {
      dragging_ = true;
      grab_ = pos - split_;
      return 1;
    }
    break;
  case FL_DRAG:
    if (dragging_) {
      apply(pos - grab_);
      return 1;
    }
    break;
  case FL_RELEASE:
    if (dragging_) {
      dragging_ = false;
      if (change_)
        change_(split_);
      return 1;
    }
    break;
  }
  return Fl_Group::handle(event);
}

void SplitPane::draw() {
  if (o_ == Horizontal && rtl_ != rtl()) {
    // The language changed direction: swap the panes.
    apply(split_);
    damage(FL_DAMAGE_ALL);
  }
  Fl_Group::draw();
  const Theme &t = theme();
  if (collapsed_ && split_ >= (o_ == Horizontal ? w() : h()))
    return;
  Rgb line = dragging_ || hover_ ? t.accent() : t.hairline();
  if (o_ == Horizontal)
    paint::vline(rtl_ ? x() + w() - split_ - 1 : x() + split_, y(), h(), line);
  else
    paint::hline(x(), y() + split_, w(), line);
}

} // namespace fo
