// Basic controls in Folio's look: quiet surfaces, square corners unless
// Hyprland rounds windows, the accent for the main choice and for focus.
// Every control works from the keyboard; focus rings show only while the
// keyboard is in use (fo::focus_visible()).
#pragma once

#include <FL/Fl_Button.H>
#include <FL/Fl_Group.H>
#include <FL/Fl_Input.H>
#include <FL/Fl_Widget.H>

#include <functional>
#include <string>
#include <vector>

#include "fltk-omarchy/anim.h"

namespace fo {

/// A dialog/toolbar button with a text label (Folio's PlainButton): the
/// accent when primary, a faint fill otherwise; 36 px high. Enter or Space
/// activates it when focused.
class Button : public Fl_Button {
public:
  Button(int x, int y, int w, int h, const char *label = nullptr);
  /// The main choice of a dialog: accent fill, accent-foreground text.
  void primary(bool on) { primary_ = on; redraw(); }
  bool primary() const { return primary_; }
  /// Called on click / Enter / Space.
  void on_click(std::function<void()> fn) { click_ = std::move(fn); }
  /// Width that fits the label (at least 88, Folio's minimum).
  int preferred_width() const;
  void draw() override;
  int handle(int event) override;

private:
  static void cb(Fl_Widget *, void *);
  std::function<void()> click_;
  bool primary_ = false, hovered_ = false;
  Anim<Rgb> fill_;
};

/// A square toolbar button holding one icon (Folio's IconButton). The
/// tooltip text is also its accessible name. `active` shows a toggled
/// state (accent tint). Not a Tab stop unless focusable(true).
class IconButton : public Fl_Button {
public:
  IconButton(int x, int y, int w, int h, const char *icon_name, const char *tooltip_text = nullptr);
  void icon(const std::string &name) { icon_ = name; redraw(); }
  const std::string &icon() const { return icon_; }
  void tooltip_text(const std::string &t) { tip_ = t; }
  const std::string &tooltip_text() const { return tip_; }
  void active_state(bool on) { active_ = on; redraw(); }
  bool active_state() const { return active_; }
  void focusable(bool on) { on ? set_visible_focus() : clear_visible_focus(); }
  void on_click(std::function<void()> fn) { click_ = std::move(fn); }
  void draw() override;
  int handle(int event) override;
  ~IconButton() override;

private:
  static void cb(Fl_Widget *, void *);
  static void show_tip(void *);
  std::string icon_, tip_;
  std::function<void()> click_;
  bool active_ = false, hovered_ = false;
  Anim<Rgb> fill_;
};

/// A check box with a label (Folio's dialogs: "Apply to all"): a small
/// square that fills with the accent when checked. Click, Space or Enter
/// toggles it.
class CheckBox : public Fl_Button {
public:
  CheckBox(int x, int y, int w, int h, const char *label = nullptr);
  bool checked() const { return value() != 0; }
  void checked(bool on) { value(on ? 1 : 0); redraw(); }
  /// Called after every toggle.
  void on_change(std::function<void(bool)> fn) { change_ = std::move(fn); }
  /// Width that fits the box and the label.
  int preferred_width() const;
  void draw() override;
  int handle(int event) override;

private:
  static void cb(Fl_Widget *, void *);
  std::function<void(bool)> change_;
  bool hovered_ = false;
};

/// A single-line text field (Folio's PlainField): a dark box with an
/// accent edge while typing and a placeholder that stays inside it.
///
///   field->on_enter([&] { go(field->value()); });
///   field->on_escape([&] { cancel(); });
///   field->on_complete([](const std::string &t) { return complete_path(t); }); // Tab
class TextField : public Fl_Input {
public:
  TextField(int x, int y, int w, int h, const char *label = nullptr);
  void placeholder(const std::string &text) { placeholder_ = text; redraw(); }
  void on_enter(std::function<void()> fn) { enter_ = std::move(fn); }
  void on_escape(std::function<void()> fn) { escape_ = std::move(fn); }
  /// Every edit.
  void on_change(std::function<void()> fn) { change_ = std::move(fn); }
  /// Tab: returns the completed text (the cursor goes to its end).
  void on_complete(std::function<std::string(const std::string &)> fn) { complete_ = std::move(fn); }
  /// First look at every key; return true to consume it (Down to results...).
  void on_key(std::function<bool(int key)> fn) { key_ = std::move(fn); }
  /// Sets the text and selects [from, to) (to = -1: the end).
  void set_text(const std::string &text, int from = 0, int to = -1);
  /// Hides what is typed (passwords): dots instead of characters.
  void secret(bool on);
  void draw() override;
  int handle(int event) override;

private:
  static void cb(Fl_Widget *, void *);
  std::string placeholder_;
  std::function<void()> enter_, escape_, change_;
  std::function<std::string(const std::string &)> complete_;
  std::function<bool(int)> key_;
};

/// The path bar's breadcrumbs: "~ / Pictures / Lisbon 2026". The last
/// segment is bright and bold; others are clickable. When the path does
/// not fit, the first segments fold into "…", which opens a menu of them
/// (inside an fo::Window). Keyboard: Left/Right move between segments,
/// Enter goes there.
class Breadcrumbs : public Fl_Widget {
public:
  struct Segment {
    std::string label;
    std::string path;
  };
  Breadcrumbs(int x, int y, int w, int h);
  void segments(std::vector<Segment> s);
  const std::vector<Segment> &segments() const { return segs_; }
  /// Called with the segment index when one is chosen.
  void on_navigate(std::function<void(int index)> fn) { navigate_ = std::move(fn); }
  /// Index of the first segment shown (0 unless folded).
  int first_visible() const { return first_; }
  void draw() override;
  int handle(int event) override;

private:
  struct Hit {
    int x, w, index; // index -1: the fold "…"
  };
  void layout_hits();
  void activate(int index);
  std::vector<Segment> segs_;
  std::vector<Hit> hits_;
  std::function<void(int)> navigate_;
  int first_ = 0, hover_ = -2, cursor_ = -2;
};

/// A hairline (the theme foreground at 12 %) filling its box: under a
/// toolbar, between panes.
class Rule : public Fl_Widget {
public:
  Rule(int x, int y, int w, int h) : Fl_Widget(x, y, w, h) { box(FL_NO_BOX); }
  void draw() override;
};

/// Two panes and a draggable divider (horizontal: left | right).
/// Keyboard: none (the divider is a mouse affordance); set_split() moves it
/// from code, animated over 190 ms when asked.
class SplitPane : public Fl_Group {
public:
  enum Orientation { Horizontal, Vertical };
  SplitPane(int x, int y, int w, int h, Orientation o = Horizontal);
  /// The first pane's size in pixels.
  int split() const { return split_; }
  void set_split(int px, bool animate = false);
  void min_sizes(int first, int second) { min1_ = first, min2_ = second; }
  /// Called after the user dragged the divider.
  void on_change(std::function<void(int)> fn) { change_ = std::move(fn); }
  /// The second pane collapses to nothing (hidden) and back.
  void collapse_second(bool on, bool animate = true);
  void resize(int x, int y, int w, int h) override;
  int handle(int event) override;
  void draw() override;

private:
  static void step(void *);
  void apply(int px);
  Orientation o_;
  int split_ = 200, min1_ = 120, min2_ = 120;
  bool dragging_ = false, hover_ = false, collapsed_ = false;
  int grab_ = 0, before_collapse_ = 0;
  Anim<double> motion_;
  std::function<void(int)> change_;
};

} // namespace fo
