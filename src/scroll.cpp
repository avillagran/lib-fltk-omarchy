#include "fltk-omarchy/scroll.h"

#include "fltk-omarchy/paint.h"

#include <FL/Fl.H>
#include <FL/fl_draw.H>

#include <algorithm>
#include <cmath>

namespace fo {

// --- Scroller --------------------------------------------------------------

void Scroller::jump(double pos) {
  value_ = from_ = target_ = std::max(0.0, pos);
  running_ = false;
}

void Scroller::wheel(double delta, double max) {
  if (!std::isfinite(delta) || delta == 0)
    return;
  position(max);
  double remaining = target_ - value_;
  double base = remaining * delta < 0 ? value_ : target_; // reversal drops pending travel
  scroll_to(base + delta, max);
}

void Scroller::scroll_to(double pos, double max) {
  position(max);
  pos = std::min(std::max(pos, 0.0), std::max(max, 0.0));
  if (!anim::enabled()) {
    jump(pos);
    return;
  }
  from_ = value_;
  target_ = pos;
  start_ = anim::now_ms();
  running_ = std::fabs(target_ - value_) > 0.01;
  if (!running_)
    value_ = target_;
}

double Scroller::position(double max) {
  max = std::max(max, 0.0);
  target_ = std::min(std::max(target_, 0.0), max);
  from_ = std::min(std::max(from_, 0.0), max);
  if (running_) {
    double t = (anim::now_ms() - start_) / anim::ms::SCROLL;
    if (t >= 1 || !anim::enabled()) {
      value_ = target_;
      running_ = false;
    } else {
      value_ = from_ + (target_ - from_) * anim::ease(anim::Ease::OutCubic, t);
    }
  }
  value_ = std::min(std::max(value_, 0.0), max);
  if (!running_)
    value_ = target_;
  return value_;
}

bool Scroller::passes(double delta, double max) const {
  if (delta == 0)
    return true;
  double at = running_ ? target_ : value_;
  return (delta < 0 && at <= 0) || (delta > 0 && at >= max);
}

// --- ScrollBar ------------------------------------------------------------

void ScrollBar::poke(Fl_Widget *owner) {
  last_ = anim::now_ms();
  owner->redraw();
}

bool ScrollBar::thumb(double pos, double max, double view, int &ty, int &th) const {
  int len = length();
  if (max <= 0 || view <= 0 || len <= 8)
    return false;
  double total = max + view;
  th = std::max(24, int(len * view / total));
  th = std::min(th, len - 4);
  ty = start() + 2 + int((len - 4 - th) * (pos / max) + 0.5);
  return true;
}

static void hide_later(void *w) { static_cast<Fl_Widget *>(w)->redraw(); }

void ScrollBar::draw(Fl_Widget *owner, double pos, double max, double view) {
  double now = anim::now_ms();
  bool recent = now - last_ < anim::ms::SCROLLBAR_HIDE;
  bool show = max > 0 && (recent || hovered_ || dragging_);
  alpha_.set(show ? 1.0 : 0.0, show ? 90 : 200, anim::Ease::OutCubic);
  double a = alpha_.get(owner);
  if (recent && !hovered_ && !dragging_) {
    Fl::remove_timeout(hide_later, owner);
    Fl::add_timeout((anim::ms::SCROLLBAR_HIDE - (now - last_)) / 1000.0 + 0.01, hide_later, owner);
  }
  int ty, th;
  if (a <= 0.01 || !thumb(pos, max, view, ty, th))
    return;
  const Theme &t = theme();
  int bw = hovered_ || dragging_ ? 6 : 4;
  int radius = std::min(2, t.radius()) > 0 ? bw / 2 : 0;
  double alpha = (dragging_ ? 0.5 : 0.28) * a;
  if (o_ == Vertical)
    paint::round_fill(x_ + w_ - bw - 2, ty, bw, th, radius, t.foreground(), alpha);
  else
    paint::round_fill(ty, y_ + h_ - bw - 2, th, bw, radius, t.foreground(), alpha);
}

bool ScrollBar::handle(int event, Fl_Widget *owner, double &pos, double max, double view) {
  int ex = Fl::event_x(), ey = Fl::event_y();
  bool inside = max > 0 && ex >= x_ && ex < x_ + w_ && ey >= y_ && ey < y_ + h_;
  int along = o_ == Vertical ? ey : ex;
  int s = start(), len = length();
  switch (event) {
  case FL_MOVE:
  case FL_ENTER:
    if (inside != hovered_) {
      hovered_ = inside;
      owner->redraw();
    }
    return false;
  case FL_LEAVE:
    if (hovered_) {
      hovered_ = false;
      owner->redraw();
    }
    return false;
  case FL_PUSH: {
    if (!inside || Fl::event_button() != FL_LEFT_MOUSE)
      return false;
    int ty, th;
    if (!thumb(pos, max, view, ty, th))
      return false;
    if (along < ty || along >= ty + th) // page to the click
      ty = std::min(std::max(along - th / 2, s + 2), s + len - 2 - th);
    grab_dy_ = along - ty;
    dragging_ = true;
    pos = (ty - s - 2) * max / std::max(1, len - 4 - th);
    owner->redraw();
    return true;
  }
  case FL_DRAG: {
    if (!dragging_)
      return false;
    int ty = 0, th = 0;
    thumb(pos, max, view, ty, th);
    double p = (along - grab_dy_ - s - 2) * max / std::max(1, len - 4 - th);
    pos = std::min(std::max(p, 0.0), max);
    last_ = anim::now_ms();
    owner->redraw();
    return true;
  }
  case FL_RELEASE:
    if (!dragging_)
      return false;
    dragging_ = false;
    last_ = anim::now_ms();
    owner->redraw();
    return true;
  }
  return false;
}

// --- ScrollArea -------------------------------------------------------------

ScrollArea::ScrollArea(int x, int y, int w, int h) : Fl_Group(x, y, w, h) {
  box(FL_NO_BOX);
  clip_children(1);
  place_bars();
}

void ScrollArea::axes(Axes a) {
  axes_ = a;
  if (!(a & Vertical))
    scroller_.jump(0);
  if (!(a & Horizontal))
    scroller_x_.jump(0);
  resize(x(), y(), w(), h());
  redraw();
}

void ScrollArea::place_bars() {
  // With both bars the vertical one stops short of the horizontal one.
  bool both = (axes_ & Vertical) && (axes_ & Horizontal) && max_offset() > 0 && max_offset_x() > 0;
  bar_.place(x() + w() - 10, y(), 10, h() - (both ? 10 : 0));
  bar_x_.place(x(), y() + h() - 10, w() - (both ? 10 : 0), 10);
}

double ScrollArea::max_offset() const {
  Fl_Widget *c = content();
  return c && (axes_ & Vertical) ? std::max(0, c->h() - h()) : 0;
}

double ScrollArea::max_offset_x() const {
  Fl_Widget *c = content();
  return c && (axes_ & Horizontal) ? std::max(0, c->w() - w()) : 0;
}

void ScrollArea::place_content() {
  Fl_Widget *c = content();
  if (!c)
    return;
  int nx = x() - int(std::lround(pos_x_));
  int ny = y() - int(std::lround(pos_));
  if (c->x() != nx || c->y() != ny) {
    c->position(nx, ny);
    if (Fl_Group *g = c->as_group())
      g->init_sizes();
  }
}

void ScrollArea::scroll_to(double offset, bool animate) {
  if (animate)
    scroller_.scroll_to(offset, max_offset());
  else
    scroller_.jump(std::min(std::max(offset, 0.0), max_offset()));
  pos_ = scroller_.position(max_offset());
  place_content();
  bar_.poke(this);
  redraw();
}

void ScrollArea::scroll_to_x(double offset, bool animate) {
  if (animate)
    scroller_x_.scroll_to(offset, max_offset_x());
  else
    scroller_x_.jump(std::min(std::max(offset, 0.0), max_offset_x()));
  pos_x_ = scroller_x_.position(max_offset_x());
  place_content();
  bar_x_.poke(this);
  redraw();
}

void ScrollArea::ensure_visible(int cy, int ch, bool animate) {
  double top = scroller_.target();
  if (cy < top)
    scroll_to(cy, animate);
  else if (cy + ch > top + h())
    scroll_to(cy + ch - h(), animate);
}

void ScrollArea::ensure_visible_x(int cx, int cw, bool animate) {
  double left = scroller_x_.target();
  if (cx < left)
    scroll_to_x(cx, animate);
  else if (cx + cw > left + w())
    scroll_to_x(std::min(double(cx), double(cx + cw - w())), animate);
}

void ScrollArea::resize(int x, int y, int w, int h) {
  Fl_Widget::resize(x, y, w, h); // the content keeps its own size on the scrolling axes
  Fl_Widget *c = content();
  if (c) {
    int cw = axes_ & Horizontal ? c->w() : w;
    int ch = axes_ & Vertical ? c->h() : h;
    if (c->w() != cw || c->h() != ch)
      c->size(cw, ch);
  }
  if (scroller_.target() > max_offset())
    scroller_.jump(max_offset());
  if (scroller_x_.target() > max_offset_x())
    scroller_x_.jump(max_offset_x());
  pos_ = scroller_.position(max_offset());
  pos_x_ = scroller_x_.position(max_offset_x());
  place_bars();
  place_content();
}

// FLTK's groups offer the wheel to children outside the pointer too, so a
// list next to the pointer could take it. The wheel goes down the widgets
// under the pointer instead: plain groups are passed through, and the
// first scrolling area or leaf widget there gets it.
static int wheel_under_pointer(Fl_Widget *w) {
  while (w && w->takesevents() && Fl::event_inside(w)) {
    Fl_Group *g = w->as_group();
    if (!g || g->as_window() || dynamic_cast<ScrollArea *>(g))
      return w->handle(FL_MOUSEWHEEL);
    Fl_Widget *next = nullptr;
    for (int i = g->children(); i-- > 0;) {
      Fl_Widget *c = g->child(i);
      if (c->takesevents() && Fl::event_inside(c)) {
        next = c;
        break;
      }
    }
    w = next;
  }
  return 0;
}

bool ScrollArea::wheel(double dx, double dy) {
  bool used = false;
  double maxy = max_offset(), maxx = max_offset_x();
  if (dx != 0 && maxx > 0 && !scroller_x_.passes(dx, maxx)) {
    scroller_x_.wheel(dx, maxx);
    bar_x_.poke(this);
    used = true;
  }
  if (dy != 0 && maxy > 0 && !scroller_.passes(dy, maxy)) {
    scroller_.wheel(dy, maxy);
    bar_.poke(this);
    used = true;
  }
  if (used)
    redraw();
  return used;
}

int ScrollArea::handle(int event) {
  double max = max_offset(), maxx = max_offset_x();
  if (event == FL_MOUSEWHEEL) {
    double dx = Fl::event_dx() * 48.0, dy = Fl::event_dy() * 48.0;
    // Shift+wheel and a Horizontal-only area turn vertical steps sideways.
    bool shifted = false;
    if (dx == 0 && (Fl::event_state(FL_SHIFT) || axes_ == Horizontal)) {
      shifted = Fl::event_state(FL_SHIFT) && (axes_ & Horizontal);
      dx = dy;
      dy = 0;
    }
    if (!(axes_ & Vertical))
      dy = 0;
    if (!(axes_ & Horizontal))
      dx = 0;
    bool can = (dy != 0 && !scroller_.passes(dy, max)) || (dx != 0 && !scroller_x_.passes(dx, maxx));
    // Let a nested scroller under the pointer use it first (not a
    // Shift+wheel meant for this area: nested lists would scroll down).
    if (!shifted && wheel_under_pointer(content()))
      return 1;
    return can && wheel(dx, dy) ? 1 : 0;
  }
  double p = pos_;
  if ((axes_ & Vertical) && bar_.handle(event, this, p, max, h())) {
    scroller_.jump(p);
    redraw();
    return 1;
  }
  double px = pos_x_;
  if ((axes_ & Horizontal) && bar_x_.handle(event, this, px, maxx, w())) {
    scroller_x_.jump(px);
    redraw();
    return 1;
  }
  if (event == FL_ENTER || event == FL_MOVE || event == FL_LEAVE)
    Fl_Group::handle(event);
  else
    return Fl_Group::handle(event);
  return 1;
}

void ScrollArea::draw() {
  double max = max_offset(), maxx = max_offset_x();
  pos_ = scroller_.position(max);
  pos_x_ = scroller_x_.position(maxx);
  if (scroller_.running() || scroller_x_.running())
    anim::request_frame(this);
  place_bars();
  place_content();
  fl_push_clip(x(), y(), w(), h());
  paint::fill(x(), y(), w(), h(), theme().background());
  draw_children();
  draw_over_content();
  if (axes_ & Vertical)
    bar_.draw(this, pos_, max, h());
  if (axes_ & Horizontal)
    bar_x_.draw(this, pos_x_, maxx, w());
  fl_pop_clip();
}

} // namespace fo
