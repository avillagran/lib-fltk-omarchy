// The places sidebar (Folio's Sidebar): sections with small uppercase
// headers, rows with an icon, a label, an optional status word and an
// optional eject button. The row holding the current place stays lit
// (accent tint) while you dig into it.
//
// Keyboard: Up/Down/Home/End move the cursor (an accent outline), Enter or
// Right activates, Esc calls on_finished (hand the keyboard back to the
// files), Tab leaves as usual. Long lists scroll smoothly.
//
// Drops: set accepts_drop / on_drop; while something is dragged over a
// row it is outlined, and FL_PASTE text (a text/uri-list) is delivered.
//
//   sb->add_section("Home", "home");
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

  void clear();
  /// Starts a section; rows added next belong to it. `id` makes the header
  /// clickable (on_header), like Folio's "Home" label.
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
  int slot_at(int ey) const;
  int row_y(int row) const;
  void reveal(int row);
  bool on_eject_button(int row, int ex) const;
  std::vector<Section> sections_;
  std::vector<SidebarRow> rows_;
  std::vector<Slot> slots_;
  int content_h_ = 0;
  int active_ = -1, cursor_ = 0, hover_ = -1, hover_header_ = -1, drop_row_ = -1;
  bool eject_hover_ = false;
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
