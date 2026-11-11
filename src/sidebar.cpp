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
const int kTop = 20, kInset = 12, kRowH = 34, kRowGap = 1, kHeaderH = 14, kHeaderTop = 21, kHeaderBottom = 6;
const int kPadX = 14, kIcon = 18, kIconGap = 14, kEject = 28;
const double kSpacing = 1.3;
} // namespace

Sidebar::Sidebar(int x, int y, int w, int h) : Fl_Widget(x, y, w, h) {
  box(FL_NO_BOX);
  set_visible_focus();
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
    if (!sections_[s].title.empty()) {
      int top = first ? 0 : kHeaderTop;
      slots_.push_back(Slot{int(s), -1, yy + top, kHeaderH});
      yy += top + kHeaderH + kHeaderBottom;
    } else if (!first) {
      yy += kHeaderTop / 2;
    }
    first = false;
    for (size_t r = 0; r < rows_.size(); ++r)
      if (rows_[r].section == int(s)) {
        slots_.push_back(Slot{int(s), int(r), yy, kRowH});
        yy += kRowH + kRowGap;
      }
  }
  content_h_ = yy + 20;
}

int Sidebar::row_y(int row) const {
  for (const auto &s : slots_)
    if (s.row == row)
      return s.y;
  return -1;
}

int Sidebar::slot_at(int ey) const {
  int cy = ey - y() + int(std::lround(pos_));
  for (size_t i = 0; i < slots_.size(); ++i) {
    const Slot &s = slots_[i];
    int pad = s.row < 0 ? 4 : 0;
    if (cy >= s.y - pad && cy < s.y + s.h + pad)
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
  if (row < 0 || !rows_[size_t(row)].ejectable)
    return false;
  // The eject button is at the end of the row: the left edge right to left.
  return rtl() ? ex < x() + kInset + kEject + 4 : ex >= x() + w() - kInset - kEject - 4;
}

void Sidebar::resize(int X, int Y, int W, int H) {
  Fl_Widget::resize(X, Y, W, H);
  bar_.place(rtl() ? X : X + W - 10, Y, 10, H);
}

int Sidebar::handle(int event) {
  double max = std::max(0, content_h_ - h());
  bar_.place(rtl() ? x() : x() + w() - 10, y(), 10, h());
  double p = pos_;
  if (bar_.handle(event, this, p, max, h())) {
    scroller_.jump(p);
    redraw();
    return 1;
  }
  int ex = Fl::event_x(), ey = Fl::event_y();
  switch (event) {
  case FL_ENTER:
  case FL_MOVE: {
    int si = slot_at(ey);
    int hv = si >= 0 ? slots_[size_t(si)].row : -1;
    int hh = si >= 0 && slots_[size_t(si)].row < 0 && !sections_[size_t(slots_[size_t(si)].section)].id.empty()
                 ? slots_[size_t(si)].section
                 : -1;
    bool ej = on_eject_button(hv, ex);
    if (hv != hover_ || hh != hover_header_ || ej != eject_hover_) {
      hover_ = hv;
      hover_header_ = hh;
      eject_hover_ = ej;
      window()->cursor(hh >= 0 ? FL_CURSOR_HAND : FL_CURSOR_DEFAULT);
      redraw();
    }
    return 1;
  }
  case FL_LEAVE:
    hover_ = hover_header_ = -1;
    eject_hover_ = false;
    window()->cursor(FL_CURSOR_DEFAULT);
    redraw();
    return 1;
  case FL_PUSH: {
    int si = slot_at(ey);
    if (si < 0)
      return 1;
    const Slot &s = slots_[size_t(si)];
    if (Fl::event_button() == FL_RIGHT_MOUSE) {
      if (s.row >= 0 && context_)
        context_(rows_[size_t(s.row)], ex, ey);
      return 1;
    }
    if (s.row < 0) {
      const std::string &id = sections_[size_t(s.section)].id;
      if (!id.empty() && header_)
        header_(id);
      return 1;
    }
    cursor_ = s.row;
    if (on_eject_button(s.row, ex)) {
      if (eject_)
        eject_(rows_[size_t(s.row)]);
    } else {
      // Like Enter: open the row and hand the keyboard back to the files,
      // so the next arrow moves there and not here.
      if (activate_)
        activate_(rows_[size_t(s.row)]);
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
    redraw();
    return 1;
  case FL_UNFOCUS:
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
    auto step = [&](int from, int dir) {
      for (int r = from + dir; r >= 0 && r < n; r += dir)
        if (visible_row(r))
          return r;
      return from;
    };
    switch (k) {
    case FL_Up: set_cursor(step(cursor_, -1)); return 1;
    case FL_Down: set_cursor(step(cursor_, 1)); return 1;
    case FL_Home: set_cursor(step(-1, 1)); return 1;
    case FL_End: set_cursor(step(n, -1)); return 1;
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
    int si = slot_at(ey);
    int r = si >= 0 ? slots_[size_t(si)].row : -1;
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
  double max = std::max(0, content_h_ - h());
  pos_ = scroller_.position(max);
  if (scroller_.running())
    anim::request_frame(this);
  bool mirror = rtl();
  bar_.place(mirror ? x() : x() + w() - 10, y(), 10, h());
  fl_push_clip(x(), y(), w(), h());
  paint::fill(x(), y(), w(), h(), t.background());
  int off = y() - int(std::lround(pos_));
  int rx = x() + kInset, rw = w() - 2 * kInset;
  // Rows are laid out left to right and mirrored for right-to-left
  // languages: icon at the right, label right-aligned, status and eject
  // button at the left.
  auto mx = [&](int x0, int w0) { return mirror ? rx + rw - (x0 - rx) - w0 : x0; };
  int r = t.radius_small();
  bool keyed = Fl::focus() == this && focus_visible();

  // The hover tint slides between rows.
  if (hover_ >= 0 && hover_ != active_) {
    RectF target{double(rx), double(row_y(hover_)), double(rw), double(kRowH)};
    if (!hover_fill_.initialized() || hover_fill_.target().h == 0)
      hover_fill_.jump(target);
    else
      hover_fill_.set(target, anim::ms::ROW * 2);
    RectF hf = hover_fill_.get(this);
    paint::round_fill(int(std::lround(hf.x)), off + int(std::lround(hf.y)), int(std::lround(hf.w)), kRowH, r,
                      t.foreground(), 0.06);
  } else {
    hover_fill_.jump(RectF{});
  }

  for (const Slot &s : slots_) {
    int sy = off + s.y;
    if (sy + s.h < y() || sy > y() + h())
      continue;
    if (s.row < 0) {
      const Section &sec = sections_[size_t(s.section)];
      Rgb c = s.section == hover_header_ ? t.foreground() : t.secondary_foreground();
      if (!sec.id.empty() && active_ >= 0 && rows_[size_t(active_)].id == sec.id)
        c = t.accent();
      fl_font(text::REGULAR, text::size::SMALL);
      std::string title = text::upper(sec.title);
      int hx = rx + kPadX;
      if (mirror)
        hx = rx + rw - kPadX - text::width_spaced(title, kSpacing, text::REGULAR, text::size::SMALL);
      text::draw_spaced(title, hx, sy + kHeaderH - fl_descent(), kSpacing, c, text::REGULAR, text::size::SMALL);
      continue;
    }
    const SidebarRow &row = rows_[size_t(s.row)];
    bool active = s.row == active_, drop = s.row == drop_row_;
    if (active || drop)
      paint::round_fill(rx, sy, rw, kRowH, r, t.accent(), 0.16);
    if (drop)
      paint::round_frame(rx, sy, rw, kRowH, r, 2, t.accent());
    else if (keyed && s.row == cursor_)
      paint::round_frame(rx, sy, rw, kRowH, r, 1, t.accent());
    Rgb fg = active ? t.bright_foreground() : t.foreground();
    int ix = rx + kPadX;
    icon::draw(row.icon.empty() ? "folder" : row.icon, mx(ix, kIcon), sy + (kRowH - kIcon) / 2, kIcon, fg);
    int lx = ix + kIcon + kIconGap;
    int right = rx + rw - kPadX;
    if (row.ejectable) {
      int ex = rx + rw - kEject - 3;
      if (s.row == hover_ && eject_hover_)
        paint::round_fill(mx(ex, kEject), sy + (kRowH - kEject) / 2, kEject, kEject, r, t.foreground(), 0.08);
      icon::draw("eject", mx(ex + (kEject - 16) / 2, 16), sy + (kRowH - 16) / 2, 16,
                 s.row == hover_ ? t.foreground() : t.secondary_foreground());
      right = ex - 6;
    }
    if (!row.status.empty()) {
      int sw = int(text::width(row.status, text::REGULAR, text::size::SMALL)) + 1;
      Rgb sc = row.status_color == Rgb() ? t.secondary_foreground() : row.status_color;
      text::draw(row.status, mx(right - sw, sw), sy, sw, kRowH, sc, text::REGULAR, text::size::SMALL,
                 mirror ? FL_ALIGN_LEFT : FL_ALIGN_RIGHT, text::Elide::None);
      right -= sw + 8;
    }
    text::draw(row.label, mx(lx, right - lx), sy, right - lx, kRowH, fg, text::REGULAR, text::size::BODY,
               mirror ? FL_ALIGN_RIGHT : FL_ALIGN_LEFT);
  }
  bar_.draw(this, pos_, max, h());
  fl_pop_clip();
}

} // namespace fo
