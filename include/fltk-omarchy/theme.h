// The Omarchy palette, followed live.
//
// Colors come from `colors.toml` in Omarchy's current theme
// (~/.local/state/omarchy/current/theme/colors.toml), with the same keys
// and fallbacks as Folio: missing derived backgrounds fall back to
// `background`, missing foregrounds to `foreground`, anything else to a
// neutral dark palette with Omarchy's yellow accent (also used when
// Omarchy is not installed). Corners follow Hyprland's
// `decoration:rounding` (0, square, without Hyprland). The UI font is the
// Omarchy monospace font: fontconfig's "monospace" alias, which
// `omarchy-font-set` changes; OMARCHY_FLTK_FONT overrides it.
//
//   fo::watch_theme();                         // load + follow live
//   const fo::Theme &t = fo::theme();
//   fl_color(t.accent().fl());
//   int id = fo::on_theme_change([] { /* relayout */ });
#pragma once

#include <FL/Enumerations.H>

#include <cstdint>
#include <functional>
#include <map>
#include <string>

namespace fo {

/// An opaque 8-bit sRGB color.
struct Rgb {
  uint8_t r = 0, g = 0, b = 0;
  constexpr Rgb() = default;
  constexpr Rgb(uint8_t r_, uint8_t g_, uint8_t b_) : r(r_), g(g_), b(b_) {}
  /// FLTK color index for fl_color().
  Fl_Color fl() const { return fl_rgb_color(r, g, b); }
  bool operator==(const Rgb &o) const { return r == o.r && g == o.g && b == o.b; }
  bool operator!=(const Rgb &o) const { return !(*this == o); }
};

/// "#rrggbb" (or "rrggbb") to Rgb. Returns false on anything else.
bool parse_hex(const std::string &text, Rgb &out);
std::string to_hex(Rgb c);
/// Linear blend from `a` (t = 0) to `b` (t = 1), per channel in sRGB, as
/// Qt.alpha(b, t) over an opaque `a` would show.
Rgb mix(Rgb a, Rgb b, double t);
/// WCAG relative luminance, 0..1.
double luminance(Rgb c);

/// The parsed contents of a colors.toml: `key = "#rrggbb"` lines and
/// `mode = "light"`. Only this subset of TOML is read.
struct Palette {
  std::map<std::string, Rgb> colors;
  bool light = false;
};
Palette parse_colors_toml(const std::string &text);

class Theme {
public:
  Theme() = default;
  explicit Theme(Palette p) : palette_(std::move(p)) {}

  /// Any key of colors.toml, with Folio's fallbacks (see the file comment).
  Rgb color(const std::string &key) const;

  Rgb accent() const { return color("accent"); }
  Rgb background() const { return color("background"); }
  Rgb dark_background() const { return color("dark_background"); }
  Rgb foreground() const { return color("foreground"); }
  Rgb bright_foreground() const { return color("bright_foreground"); }
  Rgb red() const { return color("red"); }
  Rgb yellow() const { return color("yellow"); }
  Rgb green() const { return color("green"); }
  Rgb blue() const { return color("blue"); }
  Rgb magenta() const { return color("magenta"); }
  Rgb cyan() const { return color("cyan"); }
  Rgb orange() const { return color("orange"); }

  /// Cards, tiles, panels: the theme's lighter background, or the
  /// background nudged toward the foreground when the two are the same.
  Rgb lighter_background() const;
  /// Metadata and labels: foreground mixed toward background (~4.5:1).
  Rgb secondary_foreground() const;
  /// Black or white, whichever reads on the accent.
  Rgb accent_foreground() const;
  /// Hairlines and borders: foreground at 12 % over the background.
  Rgb hairline() const { return mix(background(), foreground(), 0.12); }
  /// `color` at `alpha` over the background (Qt.alpha(color, a) in Folio).
  Rgb tint(Rgb color, double alpha) const { return mix(background(), color, alpha); }

  bool light() const { return palette_.light; }
  /// Hyprland's window rounding in pixels (0 = square, Omarchy's default).
  int radius() const { return radius_; }
  /// Small controls (buttons, rows, fields) round at most 4 px.
  int radius_small() const { return radius_ < 4 ? radius_ : 4; }
  /// Pango family for the UI ("monospace" unless OMARCHY_FLTK_FONT).
  const std::string &font_family() const { return font_family_; }
  /// True when the palette came from a colors.toml (not the fallback).
  bool from_omarchy() const { return !palette_.colors.empty(); }
  const Palette &palette() const { return palette_; }

  void set_radius(int r) { radius_ = r < 0 ? 0 : r; }
  void set_font_family(const std::string &f) { font_family_ = f; }

  /// The built-in dark palette used off Omarchy.
  static const std::map<std::string, Rgb> &fallback_colors();
  /// Reads `<current_dir>/theme/colors.toml`; an empty palette (all
  /// fallbacks) when it is missing.
  static Theme load(const std::string &current_dir);

  bool operator==(const Theme &o) const {
    return palette_.colors == o.palette_.colors && palette_.light == o.palette_.light &&
           radius_ == o.radius_ && font_family_ == o.font_family_;
  }
  bool operator!=(const Theme &o) const { return !(*this == o); }

private:
  Palette palette_;
  int radius_ = 0;
  std::string font_family_ = "monospace";
};

/// The current theme (the fallback until watch_theme() or set_theme()).
const Theme &theme();

/// Makes `t` current: updates FLTK's default colors and fonts, redraws
/// every window and notifies listeners (only when something changed).
void set_theme(const Theme &t);

/// Loads the Omarchy theme from `current_dir` (default
/// ~/.local/state/omarchy/current) and follows it: inotify on the theme
/// directory and its parents (debounced 80 ms), and Hyprland's rounding
/// (re-read on `configreloaded`). Call once, after creating the app.
void watch_theme(const std::string &current_dir = "");

/// Called on the FLTK thread after the theme changed. Returns an id.
int on_theme_change(std::function<void()> fn);
void remove_theme_listener(int id);

} // namespace fo
