// Miller columns (the macOS Finder's column view, Files' Columns):
// several columns side by side in a horizontal ScrollArea, a hairline
// between them, the newest or the active column kept in view, and
// Left/Right moving the keyboard between columns.
//
// The columns are any widgets, usually fo::Collection subclasses in the
// List layout (which leave Left and Right to their parents). The app owns
// what each column shows: it adds a column when a folder is opened,
// truncates the trail when the user goes back up, and decides what Right
// means through on_step (open the selected folder in a new column).
//
//   auto *cols = new fo::ColumnView(x, y, w, h);
//   cols->add_column(new Files(0, 0, 0, 0, home));        // positioned by the view
//   cols->on_step([&](int from, int dir) {
//     if (dir > 0 && from == cols->columns() - 1 && selected_is_folder(from)) {
//       cols->add_column(new Files(0, 0, 0, 0, selected_path(from)));
//       cols->set_active(from + 1);
//       return true;
//     }
//     return false;                                    // default: move between columns
//   });
//   cols->truncate(2);                                 // back up: drop columns 2...
#pragma once

#include <functional>

#include "fltk-omarchy/scroll.h"

namespace fo {

class ColumnView : public ScrollArea {
public:
  ColumnView(int x, int y, int w, int h);

  /// Width of every column (default 260); a 1 px hairline separates them.
  void column_width(int w);
  int column_width() const { return col_w_; }

  /// Appends a column; the view owns it, places it and sizes it to the
  /// view's height. Returns its index. With follow_last() (the default)
  /// the view glides so the new column is in view.
  int add_column(Fl_Widget *w);
  /// Removes and deletes the columns from index `count` on. If the
  /// keyboard was in one of them it moves to the last column left.
  void truncate(int count);
  void clear() { truncate(0); }
  int columns() const;
  Fl_Widget *column(int index) const;
  /// The column that is `w` or contains it, or -1.
  int index_of(const Fl_Widget *w) const;

  /// The column holding the keyboard (the last one focused), -1 for none.
  int active() const { return active_; }
  /// Makes column `index` the active one: gives it the keyboard when
  /// `focus` (if it takes focus) and scrolls it into view.
  void set_active(int index, bool focus = true, bool animate = true);
  /// Called when the active column changes (keyboard or click).
  void on_active_change(std::function<void(int index)> fn) { active_change_ = std::move(fn); }

  /// Scrolls so column `index` is in view.
  void reveal(int index, bool animate = true);
  /// Keep the newest column in view as columns are added (default on).
  void follow_last(bool on) { follow_last_ = on; }
  bool follow_last() const { return follow_last_; }

  /// Left/Right that the focused column did not use move between columns
  /// (default on). `on_step` sees them first: `dir` is -1 for the
  /// previous column and +1 for the next, in reading order (Left is +1
  /// when fo::rtl()); return true to consume the key.
  void keyboard_navigation(bool on) { keys_ = on; }
  void on_step(std::function<bool(int from, int dir)> fn) { step_ = std::move(fn); }

  int handle(int event) override;
  void resize(int x, int y, int w, int h) override;

protected:
  void draw_over_content() override;

private:
  void relayout();
  void note_active();
  int column_x(int index) const; ///< content x of a column
  Fl_Group *strip_ = nullptr;
  int col_w_ = 260;
  int active_ = -1;
  bool follow_last_ = true, keys_ = true;
  std::function<void(int)> active_change_;
  std::function<bool(int, int)> step_;
};

} // namespace fo
