#include "fltk-omarchy/menu.h"

#include "fltk-omarchy/anim.h"
#include "fltk-omarchy/i18n.h"
#include "fltk-omarchy/icons.h"
#include "fltk-omarchy/paint.h"
#include "fltk-omarchy/text.h"

#include <FL/Fl.H>
#include <FL/fl_draw.H>

#include <algorithm>
#include <cctype>

namespace fo {

namespace {
// Files' popups: 28 px entries, a 1 px accent edge, hairline separators.
const int kItemH = 28, kSepH = 9, kPadY = 4, kMinW = 220;

bool usable(const MenuItem &m) { return !m.is_separator && m.enabled; }
} // namespace

PopupMenu::~PopupMenu() {
  Fl::remove_timeout(hover_timer, this);
  Fl::remove_timeout(aim_timer, this);
}

bool PopupMenu::card_rect(int card, int &X, int &Y, int &W, int &H) const {
  if (card < 0 || card >= int(cards_.size()))
    return false;
  const Card &c = cards_[size_t(card)];
  X = c.x, Y = c.y, W = c.w, H = c.h;
  return true;
}

bool PopupMenu::item_rect(int card, int index, int &X, int &Y, int &W, int &H) const {
  if (card < 0 || card >= int(cards_.size()))
    return false;
  const Card &c = cards_[size_t(card)];
  if (index < 0 || index >= int(c.items.size()))
    return false;
  X = c.x, Y = item_y(c, index), W = c.w;
  H = c.items[size_t(index)].is_separator ? kSepH : kItemH;
  return true;
}

bool PopupMenu::aiming(int fx, int fy, int tx, int ty, int sx, int sy, int sw, int sh, bool opened_right) {
  long ex = opened_right ? sx : sx + sw;
  long ax = fx, ay = fy, bx = ex, by = sy - 4, cx = ex, cy = sy + sh + 4;
  auto cross = [](long px, long py, long qx, long qy, long rx, long ry) {
    return (qx - px) * (ry - py) - (qy - py) * (rx - px);
  };
  long d1 = cross(ax, ay, bx, by, tx, ty), d2 = cross(bx, by, cx, cy, tx, ty), d3 = cross(cx, cy, ax, ay, tx, ty);
  bool neg = d1 < 0 || d2 < 0 || d3 < 0, pos = d1 > 0 || d2 > 0 || d3 > 0;
  return !(neg && pos);
}

PopupMenu *PopupMenu::open_at(Window *win, int x, int y, std::vector<MenuItem> items) {
  auto *m = new PopupMenu();
  m->cards_.push_back(Card{std::move(items)});
  m->anchor_x_ = x, m->anchor_y_ = y, m->anchor_w_ = 0, m->anchor_h_ = 0;
  m->anchor_below_ = false;
  m->open(win);
  Fl::focus(m);
  return m;
}

PopupMenu *PopupMenu::open_below(Window *win, int x, int y, int w, int h, std::vector<MenuItem> items,
                                 bool align_right) {
  auto *m = new PopupMenu();
  m->cards_.push_back(Card{std::move(items)});
  m->anchor_x_ = x, m->anchor_y_ = y, m->anchor_w_ = w, m->anchor_h_ = h;
  m->anchor_below_ = true;
  m->anchor_right_ = align_right;
  m->open(win);
  Fl::focus(m);
  return m;
}

void PopupMenu::measure(Card &c) {
  bool icons = false;
  int widest = 0;
  for (const auto &it : c.items)
    if (!it.icon.empty())
      icons = true;
  for (const auto &it : c.items) {
    if (it.is_separator)
      continue;
    int wl = int(text::width(it.label, text::REGULAR, 12));
    int ws = it.submenu.empty() ? int(text::width(it.shortcut, text::REGULAR, 10)) : 12;
    widest = std::max(widest, wl + ws + 48 + (icons ? 26 : 0) + (it.checked ? 20 : 0));
  }
  c.w = std::max(kMinW, widest);
  c.h = 2 * kPadY;
  for (const auto &it : c.items)
    c.h += it.is_separator ? kSepH : kItemH;
}

void PopupMenu::place(Card &c, int ax, int ay, int aw, int ah, bool below, bool right) {
  measure(c);
  int W = w(), H = h();
  // Right to left, menus open toward the left and align with the anchor's
  // right edge unless asked otherwise (which then means its left edge).
  bool mirror = rtl();
  if (below) {
    c.x = right != mirror ? ax + aw - c.w : ax;
    c.y = ay + ah + 4;
    if (c.y + c.h > H - 4 && ay - 4 - c.h >= 4)
      c.y = ay - 4 - c.h;
  } else if (aw > 0) { // a submenu beside its entry
    c.y = ay - kPadY;
    if (!mirror) {
      c.x = ax + aw - 2;
      if (c.x + c.w > W - 4)
        c.x = ax - c.w + 2;
    } else {
      c.x = ax - c.w + 2;
      if (c.x < 4)
        c.x = ax + aw - 2;
    }
  } else { // at a point
    c.y = ay;
    if (!mirror) {
      c.x = ax;
      if (c.x + c.w > W - 4)
        c.x = ax - c.w;
    } else {
      c.x = ax - c.w;
      if (c.x < 4)
        c.x = ax;
    }
    if (c.y + c.h > H - 4)
      c.y = ay - c.h;
  }
  c.x = std::max(4, std::min(c.x, W - 4 - c.w));
  c.y = std::max(4, std::min(c.y, H - 4 - c.h));
}

void PopupMenu::relayout() {
  if (cards_.empty())
    return;
  cards_.resize(1);
  place(cards_[0], anchor_x_, anchor_y_, anchor_w_, anchor_h_, anchor_below_, anchor_right_);
}

int PopupMenu::item_y(const Card &c, int index) const {
  int yy = c.y + kPadY;
  for (int i = 0; i < index; ++i)
    yy += c.items[size_t(i)].is_separator ? kSepH : kItemH;
  return yy;
}

int PopupMenu::item_at(const Card &c, int ex, int ey) const {
  if (ex < c.x || ex >= c.x + c.w || ey < c.y || ey >= c.y + c.h)
    return -2; // outside
  int yy = c.y + kPadY;
  for (size_t i = 0; i < c.items.size(); ++i) {
    int ih = c.items[i].is_separator ? kSepH : kItemH;
    if (ey >= yy && ey < yy + ih)
      return c.items[i].is_separator ? -1 : int(i);
    yy += ih;
  }
  return -1;
}

void PopupMenu::open_sub(size_t card, int index, bool keyboard) {
  cards_.resize(card + 1);
  Card &c = cards_[card];
  if (index < 0 || index >= int(c.items.size()) || c.items[size_t(index)].submenu.empty() ||
      !c.items[size_t(index)].enabled)
    return;
  Card sub;
  sub.items = c.items[size_t(index)].submenu;
  sub.parent_item = index;
  place(sub, c.x, item_y(c, index), c.w, kItemH, false, false);
  if (keyboard)
    for (size_t i = 0; i < sub.items.size(); ++i)
      if (usable(sub.items[i])) {
        sub.highlight = int(i);
        break;
      }
  cards_.push_back(std::move(sub));
  redraw_all();
}

void PopupMenu::move(Card &c, int dir) {
  int n = int(c.items.size());
  if (!n)
    return;
  int i = c.highlight;
  for (int step = 0; step < n; ++step) {
    i = i < 0 ? (dir > 0 ? 0 : n - 1) : (i + dir + n) % n;
    if (usable(c.items[size_t(i)])) {
      c.highlight = i;
      redraw_all();
      return;
    }
  }
}

void PopupMenu::choose(size_t card, int index) {
  Card &c = cards_[card];
  if (index < 0 || index >= int(c.items.size()))
    return;
  const MenuItem &it = c.items[size_t(index)];
  if (!usable(it))
    return;
  if (!it.submenu.empty()) {
    open_sub(card, index, true);
    return;
  }
  auto action = it.action;
  chosen_ = true;
  close();
  if (action)
    action();
}

void PopupMenu::hover_timer(void *data) {
  auto *m = static_cast<PopupMenu *>(data);
  if (m->pending_card_ >= 0 && m->pending_card_ < int(m->cards_.size()) &&
      m->cards_[size_t(m->pending_card_)].highlight == m->pending_item_)
    m->open_sub(size_t(m->pending_card_), m->pending_item_, false);
}

void PopupMenu::aim_timer(void *data) {
  auto *m = static_cast<PopupMenu *>(data);
  m->aiming_ = false;
  // The grace is over: what the pointer rests on now takes effect.
  if (m->have_last_)
    m->pointer(m->last_x_, m->last_y_, false);
}

void PopupMenu::pointer(int ex, int ey, bool aim) {
  int fx = last_x_, fy = last_y_;
  bool moved = have_last_ && (fx != ex || fy != ey);
  last_x_ = ex, last_y_ = ey;
  have_last_ = true;
  for (size_t ci = cards_.size(); ci-- > 0;) {
    Card &c = cards_[ci];
    int i = item_at(c, ex, ey);
    if (i == -2)
      continue;
    // Over a deeper card: the cards below stay on the entries that opened it.
    if (ci + 1 < cards_.size() && i == cards_[ci + 1].parent_item) {
      // Back on the open submenu's own entry: keep it.
      Fl::remove_timeout(aim_timer, this);
      Fl::remove_timeout(hover_timer, this);
      aiming_ = false;
      if (c.highlight != i) {
        c.highlight = i;
        redraw_all();
      }
      return;
    }
    if (ci + 1 < cards_.size() && aim && moved) {
      const Card &sub = cards_[ci + 1];
      bool right = sub.x + sub.w / 2 >= c.x + c.w / 2;
      if (aiming(fx, fy, ex, ey, sub.x, sub.y, sub.w, sub.h, right)) {
        // Heading for the open submenu: wait; if the pointer settles here
        // instead, act on where it is then.
        aiming_ = true;
        Fl::remove_timeout(hover_timer, this);
        Fl::remove_timeout(aim_timer, this);
        Fl::add_timeout(anim::ms::SUBMENU_AIM / 1000.0, aim_timer, this);
        return;
      }
    }
    if (aiming_ && aim && !moved)
      return; // a repeated event at the same place while waiting
    Fl::remove_timeout(aim_timer, this);
    aiming_ = false;
    if (i != c.highlight && (i < 0 || usable(c.items[size_t(i)]))) {
      c.highlight = i;
      // Leaving a submenu's entry closes the deeper cards.
      if (ci + 1 < cards_.size() && cards_[ci + 1].parent_item != i)
        cards_.resize(ci + 1);
      Fl::remove_timeout(hover_timer, this);
      if (i >= 0 && !c.items[size_t(i)].submenu.empty()) {
        pending_card_ = int(ci);
        pending_item_ = i;
        Fl::add_timeout(anim::ms::SUBMENU_OPEN / 1000.0, hover_timer, this);
      }
      redraw_all();
    }
    return;
  }
}

int PopupMenu::handle(int event) {
  int ex = Fl::event_x(), ey = Fl::event_y();
  switch (event) {
  case FL_FOCUS:
  case FL_UNFOCUS:
    return 1;
  case FL_ENTER:
  case FL_MOVE:
  case FL_DRAG:
    pointer(ex, ey, true);
    return 1;
  case FL_PUSH:
    for (size_t ci = cards_.size(); ci-- > 0;)
      if (item_at(cards_[ci], ex, ey) != -2)
        return 1;
    close(); // a click outside only closes the menu
    return 1;
  case FL_RELEASE:
    for (size_t ci = cards_.size(); ci-- > 0;) {
      int i = item_at(cards_[ci], ex, ey);
      if (i == -2)
        continue;
      if (i >= 0)
        choose(ci, i);
      return 1;
    }
    return 1;
  case FL_MOUSEWHEEL:
    return 1;
  case FL_KEYBOARD:
  case FL_SHORTCUT: {
    if (cards_.empty())
      return 0;
    // The keyboard wins over a pending pointer grace.
    Fl::remove_timeout(aim_timer, this);
    aiming_ = false;
    size_t ci = cards_.size() - 1;
    Card &c = cards_[ci];
    int k = Fl::event_key();
    int mods = Fl::event_state() & (FL_CTRL | FL_ALT | FL_META);
    bool bare = !(Fl::event_state() & (FL_CTRL | FL_ALT | FL_META | FL_SHIFT));
    if (k == FL_Escape) {
      if (ci > 0) {
        cards_.pop_back();
        redraw_all();
      } else {
        close();
      }
      return 1;
    }
    if (mods)
      return 0;
    if (k == FL_Down || (k == 'j' && bare)) { move(c, 1); return 1; }
    if (k == FL_Up || (k == 'k' && bare)) { move(c, -1); return 1; }
    if (k == FL_Home || k == FL_Page_Up) { c.highlight = -1; move(c, 1); return 1; }
    if (k == FL_End || k == FL_Page_Down) { c.highlight = -1; move(c, -1); return 1; }
    if (k == FL_Tab) return 1; // the Tab trap
    if (k == FL_Enter || k == FL_KP_Enter || k == ' ') {
      choose(ci, c.highlight);
      return 1;
    }
    // Into a submenu: Right, or Left when the menu is mirrored.
    bool mirror = rtl();
    if ((k == FL_Right && !mirror) || (k == FL_Left && mirror)) {
      if (c.highlight >= 0 && !c.items[size_t(c.highlight)].submenu.empty())
        open_sub(ci, c.highlight, true);
      return 1;
    }
    if (k == FL_Left || k == FL_Right) {
      if (ci > 0) {
        cards_.pop_back();
        redraw_all();
        return 1;
      }
      return 0;
    }
    // A letter jumps to the next entry starting with it.
    const char *t = Fl::event_text();
    if (bare && t && std::isalnum(static_cast<unsigned char>(t[0])) && !t[1]) {
      int n = int(c.items.size());
      for (int step = 1; step <= n; ++step) {
        int i = ((c.highlight < 0 ? -1 : c.highlight) + step + n) % n;
        const MenuItem &it = c.items[size_t(i)];
        if (usable(it) && !it.label.empty() &&
            std::tolower(static_cast<unsigned char>(it.label[0])) == std::tolower(static_cast<unsigned char>(t[0]))) {
          c.highlight = i;
          redraw_all();
          return 1;
        }
      }
      return 1;
    }
    return 0;
  }
  }
  return 0;
}

void PopupMenu::draw() {
  const Theme &t = theme();
  bool mirror = rtl();
  int r = t.radius() > 0 ? 7 : 0;
  for (const Card &c : cards_) {
    paint::round_fill(c.x, c.y, c.w, c.h, r, t.accent());
    paint::round_fill(c.x + 1, c.y + 1, c.w - 2, c.h - 2, r, t.background());
    bool icons = false;
    for (const auto &it : c.items)
      if (!it.icon.empty())
        icons = true;
    int yy = c.y + kPadY;
    for (size_t i = 0; i < c.items.size(); ++i) {
      const MenuItem &it = c.items[i];
      if (it.is_separator) {
        paint::hline(c.x + 12, yy + 4, c.w - 24, t.card());
        yy += kSepH;
        continue;
      }
      bool hi = int(i) == c.highlight && it.enabled;
      if (hi)
        paint::round_fill(c.x + 2, yy, c.w - 4, kItemH, r ? 5 : 0, t.card());
      Rgb fg = !it.enabled ? t.muted() : hi ? t.bright_foreground() : t.foreground();
      // Laid out left to right, mirrored for right-to-left languages.
      auto mx = [&](int x0, int w0) { return mirror ? c.x + c.w - (x0 - c.x) - w0 : x0; };
      int lx = c.x + 12;
      if (icons) {
        if (!it.icon.empty())
          icon::draw(it.icon, mx(lx, 14), yy + (kItemH - 14) / 2, 14, fg);
        lx += 26;
      }
      int right = c.x + c.w - 12;
      if (!it.submenu.empty()) {
        text::draw(mirror ? "\xE2\x80\xB9" : "\xE2\x80\xBA", mx(right - 10, 10), yy, 10, kItemH, fg, text::REGULAR, 13,
                   FL_ALIGN_CENTER, text::Elide::None);
        right -= 18;
      } else if (it.checked) {
        icon::draw("check", mx(right - 12, 12), yy + (kItemH - 12) / 2, 12, t.accent());
        right -= 20;
      }
      if (it.submenu.empty() && !it.shortcut.empty()) {
        int sw = int(text::width(it.shortcut, text::REGULAR, 10) + 1);
        text::draw(it.shortcut, mx(right - sw, sw), yy, sw, kItemH, t.dim(), text::REGULAR, 10,
                   mirror ? FL_ALIGN_LEFT : FL_ALIGN_RIGHT, text::Elide::None);
        right -= sw + 14;
      }
      text::draw(it.label, mx(lx, right - lx), yy, right - lx, kItemH, fg, text::REGULAR, 12,
                 mirror ? FL_ALIGN_RIGHT : FL_ALIGN_LEFT);
      yy += kItemH;
    }
  }
}

} // namespace fo
