// The app window and its layers.
//
// fo::Window is a double-buffered FLTK window painted in the theme
// background. Besides the normal content it hosts layers: menus, dialogs,
// toasts, tooltips and the help overlay are drawn inside the window, above
// the content, with translucency where cairo is available. While a modal
// layer is open it receives the keyboard; other window shortcuts wait.
//
//   auto *win = new fo::Window(1100, 700, "Flo");
//   win->shortcut("Ctrl+L", [] { path_bar->edit(); });
//   win->shortcut("?", [win] { fo::HelpOverlay::show(win, rows); });
#pragma once

#include <FL/Fl_Double_Window.H>
#include <FL/Fl_Group.H>

#include <functional>
#include <string>
#include <vector>

#include "fltk-omarchy/keys.h"

class Fl_Widget_Tracker;

namespace fo {

class Window;

/// Something drawn above a window's content. Layers cover the whole
/// window; what they draw (a card, a menu) and which events they take is
/// up to them. Subclasses implement draw() and handle() as usual, and
/// call close() to go away (deleted safely afterwards).
class Layer : public Fl_Group {
public:
  Layer();
  ~Layer() override;

  /// Adds the layer on top of `win`. Modal layers take the keyboard and
  /// remember the focused widget to give focus back on close.
  void open(Window *win);
  /// Removes the layer and deletes it once events are done with it.
  void close();
  bool is_open() const { return host_ != nullptr; }
  Window *host() const { return host_; }

  /// Modal layers get every key and click while open.
  virtual bool modal() const { return true; }
  /// A key the modal layer did not use: return true to close the layer
  /// and let the key reach the app (menus), false to swallow it (dialogs).
  virtual bool passes_keys() const { return false; }
  /// The window resized: lay the card out again.
  virtual void relayout() {}

protected:
  /// Called when the layer is closing (before removal).
  virtual void closing() {}
  void redraw_all();

private:
  friend class Window;
  Window *host_ = nullptr;
  Fl_Widget_Tracker *restore_ = nullptr;
};

class Window : public Fl_Double_Window {
public:
  Window(int w, int h, const char *title = nullptr);
  Window(int x, int y, int w, int h, const char *title = nullptr);
  ~Window() override;

  /// Runs `fn` when the key is pressed anywhere in the window and nothing
  /// focused used it. See keys.h for the spec syntax.
  void shortcut(const std::string &spec, std::function<void()> fn);
  void remove_shortcut(const std::string &spec);
  /// Runs the shortcut matching the current key event, as if nothing had
  /// used the key. For widgets that would otherwise take a plain key
  /// (type-ahead) but want window keys such as "q" or "?" to win. Returns
  /// false when none matches or a modal layer is open.
  bool run_shortcut();
  /// The registered shortcuts, for a help overlay.
  std::vector<std::string> shortcut_specs() const;

  /// The topmost layer, or nullptr.
  Layer *top_layer() const { return layers_.empty() ? nullptr : layers_.back(); }
  bool modal_open() const;

  /// The fo::Window that contains `w` (nullptr if it is in another kind).
  static Window *of(Fl_Widget *w);

  int handle(int event) override;
  void draw() override;
  void resize(int x, int y, int w, int h) override;

private:
  friend class Layer;
  struct Shortcut {
    Key key;
    std::string spec;
    std::function<void()> fn;
  };
  void add_layer(Layer *l);
  void drop_layer(Layer *l);
  std::vector<Shortcut> shortcuts_;
  std::vector<Layer *> layers_;
};

} // namespace fo
