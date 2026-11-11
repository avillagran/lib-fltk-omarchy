// Files' places rail: a 52 px column of icons that expands to 170 px with
// the labels while the pointer rests on it (350 ms intent; it folds back
// 140 ms after the pointer leaves) or while it is pinned. The rail pushes
// the content beside it: the owner places it with desired_width() from
// on_width(). Section headers are hairlines in the rail and small labels
// expanded; footer rows (the trash) sit at the bottom, with the pin
// control. The row of the current place stays selected (a lighter fill)
// while you dig into it.
//
// Without rail() it is always expanded, at whatever width it is given.
//
// Keyboard: Up/Down/Home/End move the cursor (an accent outline), Enter or
// Right activates, Esc calls on_finished (hand the keyboard back to the
// files), Tab leaves as usual. Keyboard focus expands the rail. Clicking a
// row activates it and calls on_finished too. Long lists scroll smoothly.
//
// Drops: set accepts_drop / on_drop; while something is dragged over a
// row it is outlined, and FL_PASTE text (a text/uri-list) is delivered.
//
//   sb->rail(true);
//   sb->on_width([&] { layout(); });
//   sb->add_section("Places");
//   sb->add_row({"Documents", "documents", "/home/me/Documents"});
//   sb->on_activate([&](const fo::SidebarRow &r) { navigate(r.id); });
//   sb->set_active("/home/me/Documents");
#pragma once

#include <FL/Fl_Widget.H>

#include <functional>
#include <string>
#include <vector>

#include "fltk-omarchy/anim.h"
#include "fltk-omarchy/scroll.h"

namespace fo {

struct SidebarRow {
  std::string label;
  std::string icon;   ///< icon name (icons.h)
  std::string id;     ///< app data, usually a path; set_active() matches it
  std::string status; ///< a word on the right ("syncing"), "" for none
  Rgb status_color;   ///< its color (defaults to the secondary foreground)
  bool ejectable = false; ///< shows an eject button (calls on_eject)
  bool footer = false;    ///< kept at the bottom (Files: the trash)
  int section = 0;        ///< filled in by add_row

  SidebarRow() = default;
  SidebarRow(std::string label_, std::string icon_, std::string id_, std::string status_ = "",
             Rgb status_color_ = Rgb())
      : label(std::move(label_)), icon(std::move(icon_)), id(std::move(id_)), status(std::move(status_)),
        status_color(status_color_) {}
};

class Sidebar : public Fl_Widget {
public:
  Sidebar(int x, int y, int w, int h);
  ~Sidebar() override;

  /// Files' rail: `collapsed` px of icons, `expanded` px with labels.
  void rail(bool on, int collapsed = 52, int expanded = 170);
  bool is_rail() const { return rail_; }
  /// Expanded now (pinned, hovered or focused), or always without rail().
  bool expanded() const { return !rail_ || want_expanded_; }
  /// The width the rail wants this frame (animated while it opens).
  int desired_width();
  /// Called on every frame of an opening or closing (place it again).
  void on_width(std::function<void()> fn) { width_ = std::move(fn); }
  /// Pinned open (Files' pin control; it calls on_pin when clicked).
  void pinned(bool on);
  bool pinned() const { return pinned_; }
  void on_pin(std::function<void(bool)> fn) { pin_ = std::move(fn); }

  void clear();
  /// Starts a section; rows added next belong to it. `id` makes the header
  /// clickable (on_header), like a "Places" header that opens Home.
  int add_section(const std::string &title, const std::string &id = "");
  /// Hides a section's header and rows (e.g. an empty "Pinned").
  void section_visible(int section, bool on);
  int add_row(const SidebarRow &row);
  const std::vector<SidebarRow> &rows() const { return rows_; }
  SidebarRow &row(int i) { return rows_[size_t(i)]; }

  /// Lights the row with this id ("" for none).
  void set_active(const std::string &id);
  int active() const { return active_; }
  /// Keyboard cursor row.
  int cursor() const { return cursor_; }
  void set_cursor(int row);

  void on_activate(std::function<void(const SidebarRow &)> fn) { activate_ = std::move(fn); }
  void on_eject(std::function<void(const SidebarRow &)> fn) { eject_ = std::move(fn); }
  void on_header(std::function<void(const std::string &id)> fn) { header_ = std::move(fn); }
  void on_finished(std::function<void()> fn) { finished_ = std::move(fn); }
  /// Right click on a row (for a context menu).
  void on_context(std::function<void(const SidebarRow &, int x, int y)> fn) { context_ = std::move(fn); }
  void accepts_drop(std::function<bool(const SidebarRow &)> fn) { accepts_ = std::move(fn); }
  void on_drop(std::function<void(const SidebarRow &, const std::string &text)> fn) { dropped_ = std::move(fn); }

  void draw() override;
  int handle(int event) override;
  void resize(int x, int y, int w, int h) override;

private:
  struct Section {
    std::string title, id;
    bool visible = true;
  };
  struct Slot { // laid-out entry: header (row = -1) or row
    int section, row, y, h;
  };
  void layout();
  void expand(bool on);
  static void open_timer(void *);
  static void close_timer(void *);
  static void width_frame(void *);
  double reveal() const;
  bool on_pin_button(int ex, int ey) const;
  int pin_y() const;
  int slot_at(int ey) const;
  int row_y(int row) const;
  void reveal(int row);
  bool on_eject_button(int row, int ex) const;
  std::vector<Section> sections_;
  std::vector<SidebarRow> rows_;
  std::vector<Slot> slots_;
  int content_h_ = 0;
  int active_ = -1, cursor_ = 0, hover_ = -1, hover_header_ = -1, drop_row_ = -1;
  bool eject_hover_ = false, pin_hover_ = false;
  bool rail_ = false, pinned_ = false, want_expanded_ = false, hovering_ = false;
  int rail_w_ = 52, wide_w_ = 170, footer_h_ = 0;
  Anim<double> width_anim_;
  std::function<void()> width_;
  std::function<void(bool)> pin_;
  Scroller scroller_;
  ScrollBar bar_;
  double pos_ = 0;
  Anim<RectF> hover_fill_;
  std::function<void(const SidebarRow &)> activate_, eject_;
  std::function<void(const std::string &)> header_;
  std::function<void()> finished_;
  std::function<void(const SidebarRow &, int, int)> context_;
  std::function<bool(const SidebarRow &)> accepts_;
  std::function<void(const SidebarRow &, const std::string &)> dropped_;
};

} // namespace fo
