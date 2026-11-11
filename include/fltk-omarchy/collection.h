// A virtual collection for big folders: draws only the visible items, in
// a list (rows) or a grid (tiles), with smooth scrolling, an animated
// focus ring that slides between items, hover, multi-selection, rubber-band
// selection, type-ahead and drag/drop hooks. The app owns the data:
// subclass, report count(), draw each item, and react to the callbacks.
//
//   class Files : public fo::Collection {
//     using Collection::Collection;
//     int count() const override { return int(entries.size()); }
//     std::string item_text(int i) const override { return entries[i].name; }
//     void draw_item(int i, int x, int y, int w, int h, const ItemState &s) override {
//       draw_row(x, y, w, h, s, entries[i].icon, entries[i].name, entries[i].size_text);
//     }
//   };
//
// Mouse: click selects; Ctrl+click or Alt+click toggles; Shift+click
// selects a range; double-click activates; dragging on empty space draws a
// selection rectangle (Ctrl/Alt adds); dragging a selected item calls
// on_drag_start (start Fl::dnd() there); right click calls on_context.
// Keyboard: arrows (grid-aware), Home/End, PageUp/PageDown, Shift extends,
// Ctrl+A selects all, Enter activates, typing jumps by name (type-ahead).
// on_key sees every key first (Backspace, Delete, F2, → into a folder...).
#pragma once

#include <FL/Fl_Widget.H>

#include <functional>
#include <string>
#include <vector>

#include "fltk-omarchy/anim.h"
#include "fltk-omarchy/scroll.h"

namespace fo {

class Collection : public Fl_Widget {
public:
  /// Bands: Files' Groups view. Consecutive runs of items, each with an
  /// optional header above it, laid out in as many columns as fit (cards)
  /// or in one (rows). See bands().
  enum class Layout { List, Grid, Bands };
  struct Band {
    int count = 0;     ///< items in this band (they follow the previous band's)
    int min_width = 0; ///< columns = width / min_width (0: one column)
    int item_h = 32;   ///< item height
    int gap = 0;       ///< space between items, both ways
    int header_h = 0;  ///< room for draw_band_header() above the items
    int after = 0;     ///< space after the band
  };
  struct ItemState {
    bool selected = false;
    bool current = false;  ///< the keyboard cursor
    bool hovered = false;
    bool focused = false;  ///< the collection has keyboard focus
    bool drop_target = false;
    double hover = 0;      ///< animated hover amount, 0..1
  };

  Collection(int x, int y, int w, int h);

  // --- The model (override) ---
  virtual int count() const = 0;
  /// Draws item `index` in its cell (already clipped to the widget).
  virtual void draw_item(int index, int x, int y, int w, int h, const ItemState &state) = 0;
  /// Text for type-ahead (and the default accessible name).
  virtual std::string item_text(int index) const { (void)index; return ""; }

  /// Call after the data changed (count or order). Keeps the selection
  /// size in step; clears it when `reset`.
  void reload(bool reset = false);

  // --- Layout ---
  void layout(Layout l);
  Layout layout() const { return layout_; }
  /// List row height (default 32).
  void row_height(int h);
  /// Grid tile size; the tiles stretch to fill each row (default 172x196).
  void tile_size(int w, int h);
  /// The bands of Layout::Bands (their counts should add up to count()).
  void bands(std::vector<Band> b);
  const std::vector<Band> &bands() const { return bands_; }
  /// The band holding an item (-1 outside every band).
  int band_of(int index) const;
  /// Draws band `band`'s header in its slot (Files: "FOLDERS · 3").
  virtual void draw_band_header(int band, int x, int y, int w, int h) { (void)band, (void)x, (void)y, (void)w, (void)h; }
  /// A click on a band header (x, y in window coordinates).
  virtual void band_header_clicked(int band, int x, int y) { (void)band, (void)x, (void)y; }
  /// Files keeps the focus ring on the focused item even without keyboard
  /// use (default: only while the keyboard is in use).
  void always_ring(bool on) { always_ring_ = on; redraw(); }
  /// Corner radius of the focus ring when the theme rounds (Files: 9 for
  /// rows, 8 for tiles, 11 for cards).
  void ring_radius(int px) { ring_radius_ = px; }
  /// Inner padding around the items (default 12 for lists, 24 for grids).
  void padding(int px);
  /// Whether the keyboard focus ring is drawn on a selected cursor item
  /// (default on). Off when the item draws its own selection clearly, as
  /// some tiles do; the ring then shows only on an unselected cursor.
  void focus_ring_on_selected(bool on) { ring_on_selected_ = on; redraw(); }
  /// Columns in the current layout (1 for lists).
  int columns() const;
  /// The item's rectangle in widget coordinates (scrolled), or false.
  bool item_rect(int index, int &x, int &y, int &w, int &h) const;
  /// Index at a point in widget coordinates, -1 for empty space.
  int item_at(int ex, int ey) const;

  // --- Selection and cursor ---
  int current() const { return current_; }
  /// Moves the cursor, scrolls it into view; `select` replaces the selection.
  /// -1 leaves no cursor (and, with `select`, nothing selected).
  void set_current(int index, bool select = true);
  bool selected(int index) const;
  void select(int index, bool on);
  void select_all();
  void clear_selection();
  std::vector<int> selection() const;
  int selection_count() const;
  void ensure_visible(int index, bool animate = true);

  // --- Scrolling ---
  double scroll_offset() const { return pos_; }
  void scroll_to(double offset, bool animate = true);

  // --- Callbacks ---
  void on_activate(std::function<void(int index)> fn) { activate_ = std::move(fn); }
  void on_selection_change(std::function<void()> fn) { selchange_ = std::move(fn); }
  /// index -1: the click was on empty space. x, y in window coordinates.
  void on_context(std::function<void(int index, int x, int y)> fn) { context_ = std::move(fn); }
  void on_drag_start(std::function<void()> fn) { drag_ = std::move(fn); }
  /// First look at every key press; return true to consume it.
  void on_key(std::function<bool(int key)> fn) { key_ = std::move(fn); }
  /// Type-ahead: return the index to jump to for `prefix` searching from
  /// `from`, or -1. Default: first item_text() starting with it
  /// (case-insensitive), wrapping around.
  void on_type_ahead(std::function<int(const std::string &prefix, int from)> fn) { typeahead_ = std::move(fn); }
  /// Drops: may `index` (-1: the folder itself) take what is dragged?
  void accepts_drop(std::function<bool(int index)> fn) { accepts_ = std::move(fn); }
  void on_drop(std::function<void(int index, const std::string &text)> fn) { drop_ = std::move(fn); }

  // --- Drawing helpers in Files' look, for draw_item() ---
  /// A list row: icon, middle-elided name, optional detail on the right.
  /// Marked rows take the theme's selection, hovered ones a fading card;
  /// the cursor row an accent bar on its start edge.
  void draw_row(int x, int y, int w, int h, const ItemState &s, const std::string &icon,
                const std::string &name, const std::string &detail = "", Rgb icon_color = Rgb(),
                bool chevron = false);
  /// A grid tile (Files' Grid): a card holding a large icon (or the art
  /// box for a thumbnail) on top, the name and a small subtitle under it,
  /// left-aligned. Returns the art box through art_*.
  void draw_tile(int x, int y, int w, int h, const ItemState &s, const std::string &icon,
                 const std::string &name, const std::string &subtitle = "", Rgb icon_color = Rgb(),
                 bool folder_card = false, int *art_x = nullptr, int *art_y = nullptr, int *art_w = nullptr,
                 int *art_h = nullptr);

  void draw() override;
  int handle(int event) override;
  void resize(int x, int y, int w, int h) override;

protected:
  double max_offset() const;
  int content_height() const;

private:
  void cell(int index, int &cx, int &cy, int &cw, int &ch) const; // content coordinates
  struct BandGeom {
    int first, top, items_top, cols, cw, rows;
  };
  void band_geometry() const;
  void visible_range(int off, int &first, int &last) const;
  int band_step(int cur, int dir) const; // Up/Down in Bands
  void move_cursor(int to, bool extend);
  void apply_band();
  void changed();
  void type_char(const char *text);
  bool key(int k);
  Layout layout_ = Layout::List;
  int row_h_ = 32, tile_w_ = 172, tile_h_ = 196, pad_ = -1;
  bool ring_on_selected_ = true, always_ring_ = false;
  int ring_radius_ = 0;
  std::vector<Band> bands_;
  mutable std::vector<BandGeom> geom_;
  mutable int geom_w_ = -1, geom_h_ = 0;
  std::vector<char> sel_;
  int current_ = -1, anchor_ = -1, hover_ = -1, prev_hover_ = -1, drop_index_ = -2;
  Scroller scroller_;
  ScrollBar bar_;
  double pos_ = 0;
  Anim<RectF> ring_;
  Anim<double> hover_in_, hover_out_;
  // Rubber band (content coordinates).
  bool banding_ = false, band_add_ = false, press_on_item_ = false, dragged_ = false;
  int band_x0_ = 0, band_y0_ = 0, band_x1_ = 0, band_y1_ = 0, press_x_ = 0, press_y_ = 0;
  std::vector<char> band_base_;
  std::string typed_;
  double typed_at_ = -1e9;
  std::function<void(int)> activate_;
  std::function<void()> selchange_, drag_;
  std::function<void(int, int, int)> context_;
  std::function<bool(int)> key_;
  std::function<int(const std::string &, int)> typeahead_;
  std::function<bool(int)> accepts_;
  std::function<void(int, const std::string &)> drop_;
};

} // namespace fo
