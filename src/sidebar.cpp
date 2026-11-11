#include "fltk-omarchy/sidebar.h"

#include "fltk-omarchy/app.h"
#include "fltk-omarchy/i18n.h"
#include "fltk-omarchy/icons.h"
#include "fltk-omarchy/paint.h"
#include "fltk-omarchy/text.h"

#include <FL/Fl.H>
#include <FL/Fl_Window.H>
#include <FL/fl_draw.H>

#include <algorithm>
#include <cmath>

namespace fo {

namespace {
// Files' rail (places_sidebar.rs, view.rs): 34 px rows from 8 px down,
// fills inset 5 px, icons centered on the rail's 26 px axis, labels from
// 53 px, 20 px section slots (a hairline in the rail, a label expanded).
const int kTop = 8, kInset = 5, kRowH = 34, kRowGap = 0, kHeaderH = 20, kFillH = 30;
const int kIconAxis = 26, kLabelX = 53, kIcon = 16, kEject = 26, kBottom = 6;
const int kOpenMs = 350, kCloseMs = 140;

int round7() { return theme().radius() > 0 ? 7 : 0; }
Rgb rail_fill(const Theme &t) { return mix(t.background(), t.foreground(), 0.04); }
Rgb rail_edge(const Theme &t) { return mix(t.background(), t.foreground(), 0.15); }
Rgb row_selected(const Theme &t) { return mix(t.background(), t.foreground(), 0.21); }
Rgb row_hover(const Theme &t) { return mix(t.background(), t.foreground(), 0.12); }
} // namespace

Sidebar::Sidebar(int x, int y, int w, int h) : Fl_Widget(x, y, w, h) {
  box(FL_NO_BOX);
  set_visible_focus();
}

Sidebar::~Sidebar() {
  Fl::remove_timeout(open_timer, this);
  Fl::remove_timeout(close_timer, this);
}

void Sidebar::rail(bool on, int collapsed, int expanded) {
  rail_ = on;
  rail_w_ = collapsed;
  wide_w_ = std::max(collapsed, expanded);
  want_expanded_ = pinned_;
  width_anim_.jump(want_expanded_ ? wide_w_ : rail_w_);
  redraw();
}

int Sidebar::desired_width() {
  if (!rail_)
    return w();
  return int(std::lround(width_anim_.value()));
}

double Sidebar::reveal() const {
  if (!rail_)
    return 1.0;
  if (wide_w_ <= rail_w_)
    return 0.0;
  return std::max(0.0, std::min(1.0, double(w() - rail_w_) / (wide_w_ - rail_w_)));
}

void Sidebar::width_frame(void *data) {
  auto *sb = static_cast<Sidebar *>(data);
  if (sb->width_)
    sb->width_();
  sb->redraw();
  if (sb->width_anim_.running())
    anim::request_frame(width_frame, sb);
}

void Sidebar::expand(bool on) {
  if (!rail_)
    return;
  on = on || pinned_;
  if (on == want_expanded_)
    return;
  want_expanded_ = on;
  width_anim_.set(on ? wide_w_ : rail_w_, anim::ms::LAYOUT, anim::Ease::OutCubic);
  anim::request_frame(width_frame, this);
  width_frame(this);
}

void Sidebar::open_timer(void *data) {
  auto *sb = static_cast<Sidebar *>(data);
  if (sb->hovering_)
    sb->expand(true);
}

void Sidebar::close_timer(void *data) {
  auto *sb = static_cast<Sidebar *>(data);
  if (!sb->hovering_ && Fl::focus() != sb)
    sb->expand(false);
}

void Sidebar::pinned(bool on) {
  pinned_ = on;
  if (rail_)
    expand(on || hovering_ || Fl::focus() == this);
  redraw();
}

int Sidebar::pin_y() const {
  // In the rail the pin is its own icon above the footer rows; expanded it
  // sits at the end of the last footer row.
  return y() + h() - kBottom - footer_h_ - (reveal() < 0.5 ? kRowH : 0);
}

bool Sidebar::on_pin_button(int ex, int ey) const {
  if (!rail_)
    return false;
  int py = pin_y();
  if (reveal() < 0.5)
    return ey >= py && ey < py + kRowH;
  int fy = y() + h() - kBottom - kRowH;
  return ey >= fy && ey < fy + kRowH && ex >= x() + w() - 40;
}

void Sidebar::clear() {
  sections_.clear();
  rows_.clear();
  active_ = -1;
  cursor_ = 0;
  hover_ = -1;
  layout();
  redraw();
}

int Sidebar::add_section(const std::string &title, const std::string &id) {
  sections_.push_back(Section{title, id, true});
  layout();
  redraw();
  return int(sections_.size()) - 1;
}

void Sidebar::section_visible(int section, bool on) {
  if (section < 0 || section >= int(sections_.size()))
    return;
  sections_[size_t(section)].visible = on;
  layout();
  redraw();
}

int Sidebar::add_row(const SidebarRow &row) {
  if (sections_.empty())
    add_section("");
  rows_.push_back(row);
  rows_.back().section = int(sections_.size()) - 1;
  layout();
  redraw();
  return int(rows_.size()) - 1;
}

void Sidebar::set_active(const std::string &id) {
  int a = -1;
  for (size_t i = 0; i < rows_.size(); ++i)
    if (!id.empty() && rows_[i].id == id)
      a = int(i);
  if (a != active_) {
    active_ = a;
    redraw();
  }
}

void Sidebar::set_cursor(int row) {
  if (rows_.empty())
    return;
  cursor_ = std::max(0, std::min(row, int(rows_.size()) - 1));
  reveal(cursor_);
  redraw();
}

void Sidebar::layout() {
  slots_.clear();
  int yy = kTop;
  bool first = true;
  for (size_t s = 0; s < sections_.size(); ++s) {
    if (!sections_[s].visible)
      continue;
    bool any = false;
    for (const auto &r : rows_)
      any = any || (r.section == int(s) && !r.footer);
    if (!any && sections_[s].title.empty())
      continue;
    if (!first || !sections_[s].title.empty()) {
      slots_.push_back(Slot{int(s), -1, yy, kHeaderH});
      yy += kHeaderH;
    }
    first = false;
    for (size_t r = 0; r < rows_.size(); ++r)
      if (rows_[r].section == int(s) && !rows_[r].footer) {
        slots_.push_back(Slot{int(s), int(r), yy, kRowH});
        yy += kRowH + kRowGap;
      }
  }
  footer_h_ = 0;
  for (const auto &r : rows_)
    footer_h_ += r.footer ? kRowH : 0;
  content_h_ = yy + 8;
}

int Sidebar::row_y(int row) const {
  for (const auto &s : slots_)
    if (s.row == row)
      return s.y;
  return -1;
}

int Sidebar::slot_at(int ey) const {
  // Footer rows: fixed at the bottom, outside the scrolled list. They are
  // reported as negative slot numbers: -2 - their row.
  int fy = y() + h() - kBottom - footer_h_;
  if (ey >= fy) {
    for (size_t r = 0; r < rows_.size(); ++r)
      if (rows_[r].footer) {
        if (ey >= fy && ey < fy + kRowH)
          return -2 - int(r);
        fy += kRowH;
      }
    return -1;
  }
  int cy = ey - y() + int(std::lround(pos_));
  for (size_t i = 0; i < slots_.size(); ++i) {
    const Slot &s = slots_[i];
    if (cy >= s.y && cy < s.y + s.h)
      return int(i);
  }
  return -1;
}

void Sidebar::reveal(int row) {
  int ry = row_y(row);
  if (ry < 0)
    return;
  double max = std::max(0, content_h_ - h());
  double top = scroller_.target();
  if (ry < top + 8)
    scroller_.scroll_to(std::max(0, ry - 20), max);
  else if (ry + kRowH > top + h() - 8)
    scroller_.scroll_to(ry + kRowH - h() + 20, max);
  redraw();
}

bool Sidebar::on_eject_button(int row, int ex) const {
  if (row < 0 || !rows_[size_t(row)].ejectable || reveal() < 0.5)
    return false;
  // The eject button is at the end of the row: the left edge right to left.
  return rtl() ? ex < x() + kInset + kEject + 4 : ex >= x() + w() - kInset - kEject - 4;
}

void Sidebar::resize(int X, int Y, int W, int H) {
  Fl_Widget::resize(X, Y, W, H);
  bar_.place(rtl() ? X : X + W - 10, Y, 10, H);
}

int Sidebar::handle(int event) {
  double max = std::max(0, content_h_ - (h() - footer_h_ - kBottom));
  int list_h = h() - footer_h_ - kBottom - (rail_ && reveal() < 0.5 ? kRowH : 0);
  bar_.place(rtl() ? x() : x() + w() - 6, y(), 6, list_h);
  double p = pos_;
  if (bar_.handle(event, this, p, max, list_h)) {
    scroller_.jump(p);
    redraw();
    return 1;
  }
  int ex = Fl::event_x(), ey = Fl::event_y();
  auto row_of = [&](int si) {
    if (si <= -2)
      return -2 - si;
    return si >= 0 ? slots_[size_t(si)].row : -1;
  };
  switch (event) {
  case FL_ENTER:
    hovering_ = true;
    Fl::remove_timeout(close_timer, this);
    if (rail_ && !want_expanded_)
      Fl::add_timeout(kOpenMs / 1000.0, open_timer, this);
    // fall through
  case FL_MOVE: {
    int si = slot_at(ey);
    int hv = row_of(si);
    int hh = si >= 0 && slots_[size_t(si)].row < 0 && !sections_[size_t(slots_[size_t(si)].section)].id.empty()
                 ? slots_[size_t(si)].section
                 : -1;
    bool ej = on_eject_button(hv, ex);
    bool pin = on_pin_button(ex, ey);
    if (pin)
      hv = -1;
    if (hv != hover_ || hh != hover_header_ || ej != eject_hover_ || pin != pin_hover_) {
      hover_ = hv;
      hover_header_ = hh;
      eject_hover_ = ej;
      pin_hover_ = pin;
      window()->cursor(hh >= 0 ? FL_CURSOR_HAND : FL_CURSOR_DEFAULT);
      redraw();
    }
    return 1;
  }
  case FL_LEAVE:
    hovering_ = false;
    Fl::remove_timeout(open_timer, this);
    if (rail_ && want_expanded_ && !pinned_)
      Fl::add_timeout(kCloseMs / 1000.0, close_timer, this);
    hover_ = hover_header_ = -1;
    eject_hover_ = pin_hover_ = false;
    window()->cursor(FL_CURSOR_DEFAULT);
    redraw();
    return 1;
  case FL_PUSH: {
    if (on_pin_button(ex, ey)) {
      pinned(!pinned_);
      if (pin_)
        pin_(pinned_);
      return 1;
    }
    int si = slot_at(ey);
    if (si == -1)
      return 1;
    int row = row_of(si);
    if (Fl::event_button() == FL_RIGHT_MOUSE) {
      if (row >= 0 && context_)
        context_(rows_[size_t(row)], ex, ey);
      return 1;
    }
    if (row < 0) {
      const std::string &id = sections_[size_t(slots_[size_t(si)].section)].id;
      if (!id.empty() && header_)
        header_(id);
      return 1;
    }
    cursor_ = row;
    if (on_eject_button(row, ex)) {
      if (eject_)
        eject_(rows_[size_t(row)]);
    } else {
      // Like Enter: open the row and hand the keyboard back to the files,
      // so the next arrow moves there and not here.
      if (activate_)
        activate_(rows_[size_t(row)]);
      if (finished_)
        finished_();
    }
    redraw();
    return 1;
  }
  case FL_MOUSEWHEEL: {
    if (max <= 0 || Fl::event_dy() == 0)
      return 0;
    double d = Fl::event_dy() * 40.0;
    if (scroller_.passes(d, max))
      return 0;
    scroller_.wheel(d, max);
    bar_.poke(this);
    redraw();
    return 1;
  }
  case FL_FOCUS:
    if (active_ >= 0)
      cursor_ = active_;
    reveal(cursor_);
    expand(true);
    redraw();
    return 1;
  case FL_UNFOCUS:
    if (!hovering_)
      expand(false);
    redraw();
    return 1;
  case FL_KEYBOARD: {
    if (Fl::event_state() & (FL_CTRL | FL_ALT | FL_META))
      return 0;
    int k = Fl::event_key();
    int n = int(rows_.size());
    if (!n)
      return 0;
    auto visible_row = [&](int r) {
      return sections_[size_t(rows_[size_t(r)].section)].visible;
    };
    // Keyboard order: the list, then the footer rows.
    std::vector<int> order;
    for (int r = 0; r < n; ++r)
      if (visible_row(r) && !rows_[size_t(r)].footer)
        order.push_back(r);
    for (int r = 0; r < n; ++r)
      if (visible_row(r) && rows_[size_t(r)].footer)
        order.push_back(r);
    if (order.empty())
      return 0;
    size_t at = 0;
    for (size_t i = 0; i < order.size(); ++i)
      if (order[i] == cursor_)
        at = i;
    switch (k) {
    case FL_Up: set_cursor(order[at > 0 ? at - 1 : 0]); return 1;
    case FL_Down: set_cursor(order[std::min(order.size() - 1, at + 1)]); return 1;
    case FL_Home: set_cursor(order.front()); return 1;
    case FL_End: set_cursor(order.back()); return 1;
    case FL_Left:
    case FL_Right:
      // Toward the files: Right, or Left when the sidebar is mirrored.
      if ((k == FL_Right) == rtl())
        return 0;
      // fall through
    case FL_Enter:
    case FL_KP_Enter:
    case ' ':
      if (activate_ && cursor_ >= 0 && cursor_ < n)
        activate_(rows_[size_t(cursor_)]);
      if (finished_)
        finished_();
      return 1;
    case FL_Escape:
      if (finished_) {
        finished_();
        return 1;
      }
      return 0;
    }
    return 0;
  }
  case FL_DND_ENTER:
  case FL_DND_DRAG: {
    int r = row_of(slot_at(ey));
    if (r >= 0 && accepts_ && !accepts_(rows_[size_t(r)]))
      r = -1;
    if (!accepts_ && !dropped_)
      return 0;
    if (r != drop_row_) {
      drop_row_ = r;
      redraw();
    }
    return 1;
  }
  case FL_DND_LEAVE:
    drop_row_ = -1;
    redraw();
    return 1;
  case FL_DND_RELEASE:
    return drop_row_ >= 0 ? 1 : 0;
  case FL_PASTE: {
    int r = drop_row_;
    drop_row_ = -1;
    redraw();
    if (r >= 0 && dropped_)
      dropped_(rows_[size_t(r)], std::string(Fl::event_text(), size_t(Fl::event_length())));
    return 1;
  }
  }
  return 0;
}

void Sidebar::draw() {
  const Theme &t = theme();
  double rv = reveal();
  bool labels = rv > 0.35;
  int list_h = h() - footer_h_ - kBottom - (rail_ && !labels ? kRowH : 0);
  double max = std::max(0, content_h_ - list_h);
  pos_ = scroller_.position(max);
  if (scroller_.running())
    anim::request_frame(this);
  bool mirror = rtl();
  bar_.place(mirror ? x() : x() + w() - 6, y(), 6, list_h);
  fl_push_clip(x(), y(), w(), h());
  paint::fill(x(), y(), w(), h(), rail_ ? rail_fill(t) : t.background());
  if (rail_)
    paint::vline(mirror ? x() : x() + w() - 1, y(), h(), rail_edge(t));
  int rx = x() + kInset, rw = w() - 2 * kInset - (rail_ ? 1 : 0);
  // Rows are laid out left to right and mirrored for right-to-left
  // languages: icon at the right, label right-aligned, status and eject
  // button at the left.
  auto mx = [&](int x0, int w0) { return mirror ? x() + w() - (x0 - x()) - w0 : x0; };
  int r = round7();
  bool keyed = Fl::focus() == this && focus_visible();
  // Labels fade in with the reveal.
  auto label_color = [&](Rgb c) { return mix(rail_ ? rail_fill(t) : t.background(), c, std::min(1.0, rv * 1.5)); };

  auto draw_row = [&](int ri, int sy) {
    const SidebarRow &row = rows_[size_t(ri)];
    bool active = ri == active_, drop = ri == drop_row_, hov = ri == hover_;
    int fy = sy + (kRowH - kFillH) / 2;
    if (active || hov || drop)
      paint::round_fill(mx(rx, rw), fy, rw, kFillH, r, active ? row_selected(t) : row_hover(t));
    if (drop)
      paint::round_frame(mx(rx, rw), fy, rw, kFillH, r, 2, t.accent());
    else if (keyed && ri == cursor_)
      paint::round_frame(mx(rx, rw), fy, rw, kFillH, r, 1, t.accent());
    Rgb fg = active ? t.bright_foreground() : t.foreground();
    std::string ic = row.icon.empty() ? "folder" : row.icon;
    icon::draw(ic, mx(x() + kIconAxis - kIcon / 2, kIcon), sy + (kRowH - kIcon) / 2, kIcon,
               row.icon == "star" ? t.accent() : fg);
    if (!labels)
      return;
    int lx = x() + kLabelX;
    int right = x() + w() - kInset - 8;
    if (row.footer && rail_)
      right -= 30; // the pin
    if (row.ejectable) {
      int ex = x() + w() - kInset - kEject - 2;
      if (hov && eject_hover_)
        paint::round_fill(mx(ex, kEject), sy + (kRowH - kEject) / 2, kEject, kEject, r, row_selected(t));
      icon::draw("eject", mx(ex + (kEject - 14) / 2, 14), sy + (kRowH - 14) / 2, 14,
                 label_color(hov ? t.foreground() : t.dim()));
      right = ex - 6;
    }
    if (!row.status.empty()) {
      int sw = int(text::width(row.status, text::REGULAR, 10)) + 1;
      Rgb sc = row.status_color == Rgb() ? t.dim() : row.status_color;
      text::draw(row.status, mx(right - sw, sw), sy, sw, kRowH, label_color(sc), text::REGULAR, 10,
                 mirror ? FL_ALIGN_LEFT : FL_ALIGN_RIGHT, text::Elide::None);
      right -= sw + 6;
    }
    text::draw(row.label, mx(lx, right - lx), sy, right - lx, kRowH, label_color(fg),
               active ? text::BOLD : text::REGULAR, 12, mirror ? FL_ALIGN_RIGHT : FL_ALIGN_LEFT);
  };

  fl_push_clip(x(), y(), w(), list_h);
  int off = y() - int(std::lround(pos_));
  for (const Slot &s : slots_) {
    int sy = off + s.y;
    if (sy + s.h < y() || sy > y() + list_h)
      continue;
    if (s.row < 0) {
      const Section &sec = sections_[size_t(s.section)];
      if (!labels || sec.title.empty()) {
        // In the rail a section starts after a hairline.
        paint::hline(x() + 10, sy + s.h / 2, w() - 20 - (rail_ ? 1 : 0), rail_edge(t));
        continue;
      }
      Rgb c = s.section == hover_header_ ? t.foreground() : t.dim();
      if (!sec.id.empty() && active_ >= 0 && rows_[size_t(active_)].id == sec.id)
        c = t.accent();
      text::draw(sec.title, mx(x() + 12, w() - 24), sy, w() - 24, s.h, label_color(c), text::REGULAR, 11,
                 mirror ? FL_ALIGN_RIGHT : FL_ALIGN_LEFT);
      continue;
    }
    draw_row(s.row, sy);
  }
  bar_.draw(this, pos_, max, list_h);
  fl_pop_clip();

  // Footer rows and the pin control.
  int fy = y() + h() - kBottom - footer_h_;
  for (size_t ri = 0; ri < rows_.size(); ++ri)
    if (rows_[ri].footer) {
      draw_row(int(ri), fy);
      fy += kRowH;
    }
  if (rail_) {
    Rgb pc = pinned_ ? t.accent() : pin_hover_ ? t.bright_foreground() : t.dim();
    if (!labels) {
      int py = pin_y();
      icon::draw("pin", mx(x() + kIconAxis - 7, 14), py + (kRowH - 14) / 2, 14, pc);
    } else {
      int py = y() + h() - kBottom - kRowH;
      icon::draw("pin", mx(x() + w() - kInset - 28, 14), py + (kRowH - 14) / 2, 14, pc);
    }
  }
  fl_pop_clip();
}

} // namespace fo
