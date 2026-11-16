// The window shell of Files: the header's compact icon buttons, the tab
// strip under it, the status bar at the bottom and the sortable column
// header of a list. Metrics and colors are Files' (crates/ui toolbar.rs,
// view.rs, tab_strip.rs): 36 px buttons 4 px apart, 32 px tabs and status
// bar, the window background with 8 % foreground cards (Theme::card()).
//
//   auto *tools = new fo::ToolBar(0, 10, 400, 36);
//   tools->items({{"search", fo::tr("Search"), "Ctrl+F", false, true, [] { filter(); }},
//                 {"settings", fo::tr("Preferences"), "Ctrl+,", false, true, [] { prefs(); }}});
//   tools->on_relayout([&] { layout_header(); }); // a label is being revealed
//   tools->fit_width(room);  // narrow window: low-priority buttons fold
#pragma once

#include <FL/Fl_Widget.H>

#include <functional>
#include <string>
#include <vector>

#include "fltk-omarchy/anim.h"
#include "fltk-omarchy/fit.h"
#include "fltk-omarchy/theme.h"
#include "fltk-omarchy/window.h"

namespace fo {

class ToolDrawer;

/// A row of compact icon buttons (Files' header). Hovering a button
/// reveals its label to the left of the icon (160 ms), taking room from
/// the left; resting on it for two seconds shows its shortcut under it.
/// Toggled buttons sit on a card; hovered and toggled ones turn the
/// accent. Right-anchored by default: the buttons end at the widget's
/// right edge and preferred_width() grows while a label shows, so the
/// owner places it again from on_relayout().
///
/// The back/forward/up buttons are a ToolBar too, with plain(true): no
/// labels, no cards, left-anchored, the foreground when enabled and the
/// muted color when not.
///
/// Narrow rows: with fit_width() set, the buttons that do not fit fold
/// into one overflow button, lowest priority first (items sharing a
/// group, such as a view switcher, fold together; see fit::fold_row).
/// The overflow button sits where the first folded button was and shows
/// the icon of a folded button that is on (the current view), or "more".
/// Hovering it drops the folded buttons down under it as a column with
/// their labels and shortcuts, which goes away when the pointer leaves
/// both; a click keeps the column open until a choice, a click outside or
/// Esc; open_overflow(true) opens it from the keyboard (Up/Down, Home/End,
/// Enter or Space, Esc). Hover labels never grow the row past fit_width().
class ToolBar : public Fl_Widget {
public:
  struct Item {
    std::string icon;     ///< icon name (Nerd glyph when available)
    std::string label;    ///< revealed on hover; also the accessible name
    std::string shortcut; ///< shown under the button after a rest ("" none)
    bool on = false;      ///< toggled (a view that is shown, an open menu)
    bool enabled = true;
    std::function<void()> action;
    int priority = 0; ///< higher stays longer in a narrow row
    int group = 0;    ///< items sharing a nonzero group fold together
  };

  ToolBar(int x, int y, int w, int h);
  ~ToolBar() override;

  /// Replaces the buttons (the hover state stays when the count is equal).
  void items(std::vector<Item> items);
  const std::vector<Item> &items() const { return items_; }
  Item &item(int i) { return items_[size_t(i)]; }
  void set_on(int i, bool on);
  void set_enabled(int i, bool on);
  /// Plain: back/forward/up (see the class comment).
  void plain(bool on) { plain_ = on; redraw(); }
  /// Button size and gap (Files: 36 x 36, 4 apart; plain 30, 6 apart).
  void metrics(int button, int gap) { button_ = button, gap_ = gap; refold(); }
  /// The most a label may take (Files: up to 160 px, less when the
  /// header is narrow).
  void allowance(int px) { allowance_ = px; }
  /// The most room the row may take, labels included (0: no limit).
  void fit_width(int px);
  int fit_width() const { return fit_; }
  /// The overflow button's label (default: the toolkit's "Show more").
  void overflow_label(std::string label) { overflow_label_ = std::move(label); }
  /// Something is folded into the overflow button.
  bool overflowing() const { return fold_.overflow(); }
  /// Item `i` is in the overflow column rather than in the row.
  bool folded(int i) const;
  /// The current fold (slots in visual order; -1 is the overflow button).
  const fit::RowFold &fold() const { return fold_; }
  /// Opens the column of folded buttons, held open; `keyboard` highlights
  /// the first entry for the arrows. False when nothing is folded.
  bool open_overflow(bool keyboard);
  void close_overflow();
  bool overflow_open() const { return drawer_ != nullptr; }
  /// The open column (for tests and screenshots), or nullptr.
  ToolDrawer *drawer() const { return drawer_; }
  /// Width of the buttons now, labels included.
  int preferred_width() const;
  /// Called on every frame of a label reveal (place the widget again).
  void on_relayout(std::function<void()> fn) { relayout_ = std::move(fn); }
  /// The button's rectangle in window coordinates (to open a menu under
  /// it); a folded item gives the overflow button's rectangle.
  bool item_rect(int i, int &x, int &y, int &w, int &h) const;
  /// The overflow button's rectangle; false when nothing is folded.
  bool overflow_rect(int &x, int &y, int &w, int &h) const;
  /// The item under the pointer, -1 for none (or the overflow button).
  int hot() const;
  /// The pointer is over the overflow button.
  bool overflow_hot() const;

  void draw() override;
  int handle(int event) override;

private:
  friend class ToolDrawer;
  static void tick(void *);
  static void rest(void *);
  static void hover_open(void *);
  static void hover_close(void *);
  void refold();
  int width_of(int slot) const;
  int slot_at(int ex) const;
  bool slot_rect(int slot, int &x, int &y, int &w, int &h) const;
  int slot_of(int item) const;
  void set_hot(int slot);
  const Item *slot_item(int slot) const;
  std::string overflow_icon() const;
  void open_drawer(bool hold, bool keyboard);
  void drawer_closed();
  void activate(int item);
  std::vector<Item> items_;
  fit::RowFold fold_;
  std::vector<Anim<double>> extra_; ///< per slot
  std::function<void()> relayout_;
  std::string overflow_label_;
  ToolDrawer *drawer_ = nullptr;
  int hot_ = -1, pressed_ = -1, button_ = 36, gap_ = 4, allowance_ = 160, fit_ = 0;
  bool plain_ = false;
};

/// The column of folded buttons under a ToolBar's overflow button (a
/// window layer). Opened by the ToolBar; public for tests.
class ToolDrawer : public Layer {
public:
  /// Items shown, as indexes into the owner's items.
  const std::vector<int> &entries() const { return entries_; }
  /// The highlighted entry (-1 none).
  int highlighted() const { return hot_; }
  /// Held open (a click or the keyboard opened it): it takes the keys and
  /// a click outside closes it. Opened by hovering otherwise.
  bool held() const { return held_; }
  /// The card's rectangle in window coordinates.
  void card(int &x, int &y, int &w, int &h) const { x = cx_, y = cy_, w = cw_, h = ch_; }
  /// Entry `i`'s rectangle in window coordinates.
  bool entry_rect(int i, int &x, int &y, int &w, int &h) const;

  bool modal() const override { return held_; }
  bool passes_keys() const override { return true; }
  void draw() override;
  int handle(int event) override;
  void relayout() override;

protected:
  void closing() override;

private:
  friend class ToolBar;
  ToolDrawer() = default;
  void hold(bool keyboard);
  void choose(int i);
  void move(int dir);
  int entry_at(int ex, int ey) const;
  ToolBar *owner_ = nullptr;
  std::vector<int> entries_;
  std::vector<ToolBar::Item> shown_; ///< the entries as last laid out
  int hot_ = -1, cx_ = 0, cy_ = 0, cw_ = 0, ch_ = 0;
  bool held_ = false, inside_ = false, pressed_inside_ = false;
};

/// Files' tab strip: one tab per folder, the current one on a card with
/// an accent line under it and a close button, the others dimmed (their
/// close button shows on hover), and "+" at the right. Middle click or
/// the close button closes a tab, a click on empty space or "+" opens one.
class TabStrip : public Fl_Widget {
public:
  TabStrip(int x, int y, int w, int h);
  void tabs(std::vector<std::string> labels, int current);
  int current() const { return current_; }
  int count() const { return int(labels_.size()); }
  void on_select(std::function<void(int)> fn) { select_ = std::move(fn); }
  void on_close(std::function<void(int)> fn) { close_ = std::move(fn); }
  void on_new(std::function<void()> fn) { new_ = std::move(fn); }
  /// Right click on a tab (index) for a context menu; x, y in the window.
  void on_context(std::function<void(int, int, int)> fn) { context_ = std::move(fn); }
  /// Files' tab drag: after 5 px of travel an accent line shows where the
  /// tab would go (before or after the tab under the pointer); releasing
  /// inside the strip asks to move it there as a remove-then-insert
  /// (from, to), leaving the strip cancels. The owner reorders its tabs and
  /// calls tabs() again. Without a handler, tabs do not move.
  void on_reorder(std::function<void(int, int)> fn) { reorder_ = std::move(fn); }
  /// Where a dragged tab would land now (0..count), -1 when not dragging.
  int drop_at() const { return dragging_ ? drop_at_ : -1; }
  /// A tab's rectangle (widget coordinates are window coordinates).
  bool tab_rect(int i, int &x, int &w) const;

  void draw() override;
  int handle(int event) override;

private:
  void layout();
  struct Slot {
    int x, w;
  };
  std::vector<std::string> labels_;
  std::vector<Slot> slots_;
  int current_ = 0, hover_ = -1, hover_close_ = -1, plus_x_ = 0;
  bool hover_plus_ = false;
  Anim<RectF> line_;
  std::function<void(int)> select_, close_;
  std::function<void()> new_;
  std::function<void(int, int, int)> context_;
  std::function<void(int, int)> reorder_;
  int pressed_ = -1, press_x_ = 0, press_y_ = 0, drop_at_ = -1;
  bool dragging_ = false;
};

/// Files' status bar: what is marked and listed on the left (red for
/// problems), a hint in the middle, the filesystem and its free space on
/// the right, and a short colored badge at the far right (Files: the
/// window's PSS in green). When the bar is narrow the parts give way by
/// priority and never overlap (fit::status_row): the left text stays
/// (elided if it must), the badge and the free space show only whole, and
/// the hint takes what is left, elided, or nothing.
class StatusBar : public Fl_Widget {
public:
  StatusBar(int x, int y, int w, int h);
  void left(const std::string &text, bool error = false);
  void hint(const std::string &text);
  void right(const std::string &text);
  void badge(const std::string &text, Rgb color);
  /// Clicking the hint (e.g. to open the path editor).
  void on_hint(std::function<void()> fn) { hint_cb_ = std::move(fn); }
  void draw() override;
  int handle(int event) override;

private:
  std::string left_, hint_, right_, badge_;
  Rgb badge_color_;
  int hint_x_ = 0, hint_w_ = 0; ///< where the hint was drawn (its click area)
  bool error_ = false;
  std::function<void()> hint_cb_;
};

/// The column header of Files' List view: small uppercase labels with
/// glyphs; the sorted column in the accent with an arrow, the name column
/// on a card. Columns are laid out by the owner (x relative to the widget).
class SortHeader : public Fl_Widget {
public:
  struct Column {
    std::string label, icon;
    int x = 0, w = 0;
    bool right = false; ///< right-aligned (sizes)
  };
  SortHeader(int x, int y, int w, int h);
  void columns(std::vector<Column> c);
  /// Which column is sorted and how.
  void sorted(int column, bool descending);
  void on_sort(std::function<void(int column)> fn) { sort_ = std::move(fn); }
  void draw() override;
  int handle(int event) override;

private:
  int column_at(int ex) const;
  std::vector<Column> cols_;
  int sorted_ = 0, hover_ = -1;
  bool desc_ = false;
  std::function<void(int)> sort_;
};

/// Files' button look, for panels such as the inspector: a card when
/// `on` (its main action, in the accent), the background otherwise, the
/// label in dim text (muted when unavailable). Draw-only helper.
void draw_action_button(int x, int y, int w, int h, const std::string &label, bool on, bool hovered,
                        bool enabled = true);

/// Files' section label: "CARPETAS · 3", 10 px dim uppercase.
void draw_section_label(const std::string &text, int x, int y, int w, int h);

} // namespace fo
