// Dialogs, toasts, tooltips and the keyboard help, as layers of an
// fo::Window (Folio's FolioDialog, ConfirmDialog, HelpOverlay).
//
// A dialog dims the window (a scrim that fades in over 180 ms) and shows a
// card with a title, a message, an optional text field and two buttons.
// Enter confirms, Esc or a click on the scrim cancels. While it is open
// it keeps the keyboard: window shortcuts wait until it is answered.
//
//   fo::Dialog::confirm(win, "Empty the trash?", "This can't be undone.", "Empty",
//                       [](bool yes) { if (yes) empty_trash(); }, true /*danger*/);
//   fo::Dialog::prompt(win, "Rename", "", name, "Rename",
//                      [](bool ok, const std::string &s) { ... }, 0, stem_length);
//   fo::Toast::show(win, "Copied 3 items");
//   fo::HelpOverlay::show(win, {{"Ctrl+L", "Type a path"}, {"/", "Search"}});
#pragma once

#include <functional>
#include <string>
#include <utility>
#include <vector>

#include "fltk-omarchy/anim.h"
#include "fltk-omarchy/window.h"

class Fl_Widget_Tracker;

namespace fo {

class Button;
class TextField;

class Dialog : public Layer {
public:
  /// A yes/no question. `danger` draws the card's edge in red (deleting
  /// for good); the action button is the default either way.
  static Dialog *confirm(Window *win, const std::string &title, const std::string &message,
                         const std::string &action, std::function<void(bool)> done, bool danger = false);
  /// Asks for text. The field starts with `initial`, selected from
  /// `select_from` to `select_to` (-1: the end), e.g. a file name's stem.
  static Dialog *prompt(Window *win, const std::string &title, const std::string &message,
                        const std::string &initial, const std::string &action,
                        std::function<void(bool ok, const std::string &text)> done, int select_from = 0,
                        int select_to = -1);

  /// Answers it from code.
  void accept();
  void cancel();
  /// Shows a problem under the field (e.g. "That name is taken") and keeps
  /// the dialog open. Use from a prompt's validator.
  void set_error(const std::string &text);
  /// Called before accepting a prompt; return false to keep it open.
  void on_validate(std::function<bool(const std::string &)> fn) { validate_ = std::move(fn); }

  void draw() override;
  int handle(int event) override;
  void relayout() override;

private:
  Dialog() = default;
  void build(Window *win, bool with_field);
  void finish(bool ok);
  std::string title_, message_, error_;
  std::vector<std::string> lines_;
  bool danger_ = false;
  Button *ok_ = nullptr, *cancel_ = nullptr;
  TextField *field_ = nullptr;
  std::function<void(bool, const std::string &)> done_;
  std::function<bool(const std::string &)> validate_;
  int cx_ = 0, cy_ = 0, cw_ = 0, ch_ = 0;
  Anim<double> shown_;
};

/// A short notice at the bottom of the window that goes away by itself
/// (or on click). One per window: a new toast replaces the old one.
class Toast : public Layer {
public:
  enum Kind { Info, Error };
  static Toast *show(Window *win, const std::string &text, double seconds = 3.0, Kind kind = Info);
  bool modal() const override { return false; }
  void draw() override;
  int handle(int event) override;
  ~Toast() override;

private:
  Toast() = default;
  static void expire(void *);
  static void step(void *);
  std::string text_;
  Kind kind_ = Info;
  Anim<double> shown_;
  bool leaving_ = false;
  int cx_ = 0, cy_ = 0, cw_ = 0, ch_ = 0;
};

/// A small label under (or above) a widget. IconButton shows its own after
/// a 600 ms hover; other widgets call show()/hide().
class Tooltip : public Layer {
public:
  static void open_for(Fl_Widget *anchor, const std::string &text);
  static void close_all();
  bool modal() const override { return false; }
  void draw() override;
  int handle(int) override { return 0; }

private:
  Tooltip() = default;
  std::string text_;
  int ax_ = 0, ay_ = 0, aw_ = 0, ah_ = 0;
};

/// Every shortcut, one keypress away: a two-column table (key in the
/// accent, what it does beside it). Esc, ?, or a click closes it. Long
/// lists flow into a second pair of columns when the window is wide.
class HelpOverlay : public Layer {
public:
  using Row = std::pair<std::string, std::string>;
  static HelpOverlay *show(Window *win, std::vector<Row> rows, const std::string &title = "");
  void draw() override;
  int handle(int event) override;

private:
  HelpOverlay() = default;
  std::vector<Row> rows_;
  std::string title_;
  Anim<double> shown_;
};

} // namespace fo
