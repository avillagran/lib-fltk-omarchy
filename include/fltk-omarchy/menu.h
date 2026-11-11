// Popup and context menus, drawn inside an fo::Window as a layer (no
// separate OS window, so they behave the same on Wayland and X11).
//
// Folio's look: a hairline-bordered card in the background color, 32 px
// entries, the shortcut on the right, an arrow for submenus, a check for
// checked entries, the highlighted entry tinted with the accent. No
// opening animation, as in Folio.
//
// Pointer: a submenu opens after 200 ms of hover. While it is open, moving
// the pointer diagonally toward it across other entries keeps it open
// (the "aim" triangle of Qt and GTK, with a 300 ms grace: when the pointer
// settles on another entry instead, that entry takes over).
//
// Keyboard: Up/Down (j/k), Home/End, Enter or Space activate, Right opens
// a submenu and Left closes it, a letter jumps to the next entry starting
// with it, Esc closes. Any other key closes the menu and goes on to the
// app (so window shortcuts work while a menu is open).
//
//   std::vector<fo::MenuItem> items = {
//     {"Open", "⏎", [&] { open(); }},
//     fo::MenuItem::separator(),
//     {"Sort by", "", nullptr, {{"Name", "", by_name}, {"Size", "", by_size}}},
//   };
//   fo::PopupMenu::open_at(win, Fl::event_x(), Fl::event_y(), items);
#pragma once

#include <functional>
#include <string>
#include <vector>

#include "fltk-omarchy/window.h"

namespace fo {

struct MenuItem {
  std::string label;
  std::string shortcut; ///< shown on the right ("Ctrl+C", "F2")
  std::function<void()> action;
  std::vector<MenuItem> submenu;
  std::string icon; ///< optional icon name (see icons.h)
  bool enabled = true;
  bool checked = false;
  bool is_separator = false;

  MenuItem() = default;
  MenuItem(std::string l, std::string keys = "", std::function<void()> fn = nullptr,
           std::vector<MenuItem> sub = {})
      : label(std::move(l)), shortcut(std::move(keys)), action(std::move(fn)), submenu(std::move(sub)) {}
  static MenuItem separator() {
    MenuItem m;
    m.is_separator = true;
    return m;
  }
  MenuItem &disabled(bool off = true) { enabled = !off; return *this; }
  MenuItem &check(bool on = true) { checked = on; return *this; }
  MenuItem &with_icon(const std::string &name) { icon = name; return *this; }
};

class PopupMenu : public Layer {
public:
  /// Opens at a point (the pointer for context menus), flipped and clamped
  /// to stay inside the window.
  static PopupMenu *open_at(Window *win, int x, int y, std::vector<MenuItem> items);
  /// Opens below a rectangle (a toolbar button), or above it when there
  /// is no room below; right-aligned with it when `align_right`.
  static PopupMenu *open_below(Window *win, int x, int y, int w, int h, std::vector<MenuItem> items,
                               bool align_right = false);
  /// Called when the menu closes without a choice (Esc, click outside).
  void on_dismiss(std::function<void()> fn) { dismiss_ = std::move(fn); }

  /// Open cards: 1 for the menu, one more per open submenu.
  int depth() const { return int(cards_.size()); }
  /// The highlighted entry of card `card` (0: the menu), -1 for none.
  int highlighted(int card) const {
    return card >= 0 && card < int(cards_.size()) ? cards_[size_t(card)].highlight : -1;
  }
  /// Card `card`'s rectangle in window coordinates; false when not open.
  bool card_rect(int card, int &x, int &y, int &w, int &h) const;
  /// The rectangle of entry `index` of card `card`, in window coordinates.
  bool item_rect(int card, int index, int &x, int &y, int &w, int &h) const;

  /// True when a pointer moving from (fx, fy) to (tx, ty) heads for a
  /// submenu card (sx, sy, sw, sh): (tx, ty) lies in the triangle between
  /// (fx, fy) and the card's near edge (its left edge when it opened to
  /// the right of its parent, its right edge otherwise).
  static bool aiming(int fx, int fy, int tx, int ty, int sx, int sy, int sw, int sh, bool opened_right);

  bool passes_keys() const override { return true; }
  void draw() override;
  int handle(int event) override;
  void relayout() override;
  ~PopupMenu() override;

protected:
  void closing() override {
    if (!chosen_ && dismiss_)
      dismiss_();
  }

private:
  struct Card {
    std::vector<MenuItem> items;
    int x = 0, y = 0, w = 0, h = 0;
    int highlight = -1;
    int parent_item = -1; // in the previous card
  };
  PopupMenu() = default;
  void place(Card &c, int ax, int ay, int aw, int ah, bool below, bool right);
  void measure(Card &c);
  int item_at(const Card &c, int ex, int ey) const;
  int item_y(const Card &c, int index) const;
  void open_sub(size_t card, int index, bool keyboard);
  void move(Card &c, int dir);
  void choose(size_t card, int index);
  static void hover_timer(void *);
  static void aim_timer(void *);
  void pointer(int ex, int ey, bool aim);
  std::vector<Card> cards_;
  std::function<void()> dismiss_;
  int pending_card_ = -1, pending_item_ = -1;
  int last_x_ = 0, last_y_ = 0;
  bool have_last_ = false, aiming_ = false;
  bool chosen_ = false;
  int anchor_x_ = 0, anchor_y_ = 0, anchor_w_ = 0, anchor_h_ = 0;
  bool anchor_below_ = false, anchor_right_ = false;
};

} // namespace fo
