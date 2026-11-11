// One call to set an app up the Omarchy way.
//
//   int main(int argc, char **argv) {
//     fo::init();                          // theme, fonts, language, motion
//     auto *win = new fo::Window(1100, 700, "Flo");
//     ...
//     win->end();
//     win->show(argc, argv);
//     return Fl::run();
//   }
#pragma once

#include <string>

namespace fo {

struct InitOptions {
  /// Omarchy's current theme directory ("" = ~/.local/state/omarchy/current).
  std::string theme_dir;
  /// Omarchy's language file ("" = ~/.config/omarchy/language).
  std::string language_file;
  bool watch_theme = true;
  bool watch_language = true;
  /// Follow Hyprland (rounding, animations switch, refresh rate).
  bool watch_hyprland = true;
  /// Window class for the compositor (window rules), e.g. "flo".
  std::string app_class;
};

/// Loads and follows the theme and language, applies the font, starts the
/// Hyprland watchers, enables Fl::awake() for background tasks, turns off
/// FLTK's arrow-key focus moves (arrows act inside controls) and installs
/// the keyboard-focus tracker used by focus_visible(). Call before
/// creating windows.
void init(const InitOptions &opts = InitOptions());

/// True while the keyboard is in use (the last input was a key, not a
/// click): focus rings are drawn only then, like CSS :focus-visible.
bool focus_visible();
void set_focus_visible(bool on);

} // namespace fo
