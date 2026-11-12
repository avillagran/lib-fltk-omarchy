// Theme watching follows Folio's theme.cpp (itself after Omacut).
#include "fltk-omarchy/theme.h"

#include "fltk-omarchy/hyprland.h"
#include "fltk-omarchy/icons.h"
#include "fltk-omarchy/platform.h"
#include "fltk-omarchy/text.h"

#include <FL/Fl.H>
#include <FL/Fl_Window.H>

#include <algorithm>
#include <cmath>
#include <cstdio>
#include <memory>

namespace fo {

bool parse_hex(const std::string &text, Rgb &out) {
  size_t i = !text.empty() && text[0] == '#' ? 1 : 0;
  if (text.size() - i != 6)
    return false;
  unsigned v = 0;
  for (; i < text.size(); ++i) {
    char c = text[i];
    v <<= 4;
    if (c >= '0' && c <= '9') v |= unsigned(c - '0');
    else if (c >= 'a' && c <= 'f') v |= unsigned(c - 'a' + 10);
    else if (c >= 'A' && c <= 'F') v |= unsigned(c - 'A' + 10);
    else return false;
  }
  out = Rgb(uint8_t(v >> 16), uint8_t(v >> 8), uint8_t(v));
  return true;
}

std::string to_hex(Rgb c) {
  char buf[8];
  std::snprintf(buf, sizeof buf, "#%02x%02x%02x", c.r, c.g, c.b);
  return buf;
}

Rgb mix(Rgb a, Rgb b, double t) {
  if (t <= 0) return a;
  if (t >= 1) return b;
  auto ch = [t](uint8_t x, uint8_t y) { return uint8_t(std::lround(x + (y - x) * t)); };
  return Rgb(ch(a.r, b.r), ch(a.g, b.g), ch(a.b, b.b));
}

double luminance(Rgb c) {
  auto lin = [](uint8_t v8) {
    double v = v8 / 255.0;
    return v <= .04045 ? v / 12.92 : std::pow((v + .055) / 1.055, 2.4);
  };
  return .2126 * lin(c.r) + .7152 * lin(c.g) + .0722 * lin(c.b);
}

Palette parse_colors_toml(const std::string &text) {
  Palette p;
  size_t pos = 0;
  while (pos < text.size()) {
    size_t eol = text.find('\n', pos);
    if (eol == std::string::npos)
      eol = text.size();
    std::string line = text.substr(pos, eol - pos);
    pos = eol + 1;
    size_t i = 0;
    auto skip = [&] { while (i < line.size() && (line[i] == ' ' || line[i] == '\t')) ++i; };
    skip();
    size_t k0 = i;
    while (i < line.size() && ((line[i] >= 'a' && line[i] <= 'z') || line[i] == '_'))
      ++i;
    if (i == k0)
      continue;
    std::string key = line.substr(k0, i - k0);
    skip();
    if (i >= line.size() || line[i] != '=')
      continue;
    ++i;
    skip();
    if (i >= line.size() || (line[i] != '"' && line[i] != '\''))
      continue;
    char q = line[i++];
    size_t close = line.find(q, i);
    if (close == std::string::npos)
      continue;
    std::string value = line.substr(i, close - i);
    if (key == "mode") {
      p.light = value == "light";
      continue;
    }
    Rgb c;
    if (value.size() == 7 && parse_hex(value, c))
      p.colors[key] = c;
  }
  return p;
}

const std::map<std::string, Rgb> &Theme::fallback_colors() {
  // Files' fallback outside Omarchy: Tokyo Night.
  static const std::map<std::string, Rgb> colors = {
      {"accent", Rgb(0x7a, 0xa2, 0xf7)},
      {"selection", Rgb(0x29, 0x2e, 0x42)},
      {"muted", Rgb(0x41, 0x48, 0x68)},
      {"background", Rgb(0x1a, 0x1b, 0x26)},
      {"dark_background", Rgb(0x13, 0x14, 0x1c)},
      {"darker_background", Rgb(0x0e, 0x0e, 0x14)},
      {"lighter_background", Rgb(0x24, 0x28, 0x3b)},
      {"foreground", Rgb(0xa9, 0xb1, 0xd6)},
      {"dark_foreground", Rgb(0x56, 0x5f, 0x89)},
      {"bright_foreground", Rgb(0xc0, 0xca, 0xf5)},
      {"red", Rgb(0xf7, 0x76, 0x8e)},
      {"yellow", Rgb(0xe0, 0xaf, 0x68)},
      {"orange", Rgb(0xeb, 0x92, 0x7b)},
      {"green", Rgb(0x9e, 0xce, 0x6a)},
      {"cyan", Rgb(0x44, 0x9d, 0xab)},
      {"blue", Rgb(0x7a, 0xa2, 0xf7)},
      {"magenta", Rgb(0xad, 0x8e, 0xe6)},
  };
  return colors;
}

static bool ends_with(const std::string &s, const char *suffix) {
  size_t n = std::char_traits<char>::length(suffix);
  return s.size() >= n && s.compare(s.size() - n, n, suffix) == 0;
}

Rgb Theme::color(const std::string &key) const {
  const auto &c = palette_.colors;
  if (auto it = c.find(key); it != c.end())
    return it->second;
  if (ends_with(key, "background")) {
    if (auto it = c.find("background"); it != c.end())
      return it->second;
  }
  if (ends_with(key, "foreground")) {
    if (auto it = c.find("foreground"); it != c.end())
      return it->second;
  }
  const auto &f = fallback_colors();
  if (auto it = f.find(key); it != f.end())
    return it->second;
  return Rgb(0x88, 0x88, 0x88);
}

Rgb Theme::lighter_background() const {
  Rgb lighter = color("lighter_background");
  Rgb bg = background();
  if (std::fabs(luminance(lighter) - luminance(bg)) > 0.004)
    return lighter;
  return mix(bg, foreground(), 0.08);
}

Rgb Theme::dim() const {
  // Files' dark_foreground: the foreground with its brightest channel
  // divided by 1.4 (the Rust toolkit's Rgb::darker(1.4)).
  Rgb f = foreground();
  auto ch = [](uint8_t v) { return uint8_t(std::lround(std::min(255.0, v / 1.4))); };
  return Rgb(ch(f.r), ch(f.g), ch(f.b));
}

Rgb Theme::muted() const {
  const auto &c = palette_.colors;
  if (auto it = c.find("muted"); it != c.end())
    return it->second;
  if (auto it = c.find("color8"); it != c.end())
    return it->second;
  return from_omarchy() ? Rgb(0x70, 0x78, 0x80) : fallback_colors().at("muted");
}

Rgb Theme::selection() const {
  const auto &c = palette_.colors;
  if (auto it = c.find("selection"); it != c.end())
    return it->second;
  return from_omarchy() ? mix(background(), foreground(), 0.18) : fallback_colors().at("selection");
}

Rgb Theme::secondary_foreground() const { return mix(background(), foreground(), 0.68); }

Rgb Theme::accent_foreground() const {
  return luminance(accent()) > .179 ? Rgb(0, 0, 0) : Rgb(255, 255, 255);
}

Theme Theme::load(const std::string &current_dir) {
  std::string text;
  Palette p;
  if (platform::read_file(current_dir + "/theme/colors.toml", text, 256 * 1024))
    p = parse_colors_toml(text);
  return Theme(std::move(p));
}

// ---------------------------------------------------------------------------
// Current theme and live following.

namespace {

struct State {
  Theme current;
  bool applied = false;
  std::map<int, std::function<void()>> listeners;
  int next_id = 1;
  std::string dir;
  std::unique_ptr<platform::FileWatch> watch;
  int hypr_sub = 0;
  bool rounding_pending = false;
  bool follow_hypr = false;
  // The followed theme before Files' overrides, and the overrides.
  Theme live;
  bool has_live = false;
  int forced_radius = -1;
  bool local_palette = false;
};

State &state() {
  static State s;
  return s;
}

void apply_to_fltk(const Theme &t) {
  Rgb bg = t.background(), fg = t.foreground(), bg2 = t.dark_background();
  Fl::background(bg.r, bg.g, bg.b);
  Fl::background2(bg2.r, bg2.g, bg2.b);
  Fl::foreground(fg.r, fg.g, fg.b);
  Fl::set_color(FL_SELECTION_COLOR, t.accent().fl());
  Fl::set_color(FL_INACTIVE_COLOR, t.secondary_foreground().fl());
  text::apply_fonts(t.font_family());
  for (Fl_Window *w = Fl::first_window(); w; w = Fl::next_window(w))
    w->redraw();
}

void notify() {
  auto copy = state().listeners;
  for (auto &kv : copy)
    kv.second();
}

// The followed theme with the overrides on top becomes current.
void publish(const Theme &live) {
  State &s = state();
  s.live = live;
  s.has_live = true;
  Theme t = s.local_palette ? Theme() : live;
  t.set_font_family(live.font_family());
  t.set_radius(s.forced_radius >= 0 ? s.forced_radius : live.radius());
  set_theme(t);
}

void refresh_rounding() {
  State &s = state();
  if (!s.follow_hypr || s.rounding_pending || !hypr::available())
    return;
  s.rounding_pending = true;
  hypr::request("j/getoption decoration:rounding", [](bool ok, const std::string &reply) {
    State &st = state();
    st.rounding_pending = false;
    int r;
    if (!ok || !hypr::parse_option_int(reply, r) || r < 0)
      return;
    Theme t = st.has_live ? st.live : st.current;
    if (r != t.radius()) {
      t.set_radius(r);
      publish(t);
    }
  });
}

void rearm_and_reload() {
  State &s = state();
  // A theme switch replaces symlinks and files, so watch the parents too
  // and re-arm after every change. Watching the nearest existing ancestor
  // means installing Omarchy later works without restarting the app.
  std::string ancestor = s.dir;
  while (!platform::exists(ancestor) && ancestor != "/" && ancestor != ".")
    ancestor = platform::parent(ancestor);
  s.watch->set_paths({ancestor, platform::parent(ancestor), s.dir, s.dir + "/theme",
                      s.dir + "/theme/colors.toml"});
  Theme t = Theme::load(s.dir);
  const Theme &before = s.has_live ? s.live : s.current;
  t.set_radius(before.radius());
  t.set_font_family(before.font_family());
  publish(t);
  refresh_rounding();
}

} // namespace

const Theme &theme() { return state().current; }

void set_theme(const Theme &t) {
  State &s = state();
  if (s.applied && t == s.current)
    return;
  bool first = !s.applied;
  s.current = t;
  s.applied = true;
  apply_to_fltk(t);
  if (!first)
    notify();
}

void watch_theme(const std::string &current_dir) {
  State &s = state();
  s.dir = current_dir.empty() ? platform::home_dir() + "/.local/state/omarchy/current" : current_dir;
  s.follow_hypr = current_dir.empty();
  Theme t = s.current;
  std::string forced = platform::env("OMARCHY_FLTK_FONT", "");
  bool nerd = false;
  std::string family = text::ui_family(nerd);
  if (!forced.empty()) {
    family = forced;
    nerd = forced.find("Nerd") != std::string::npos;
  }
  t.set_font_family(family);
  glyph::set_available(nerd);
  s.current = t;
  if (s.has_live)
    s.live.set_font_family(family);
  if (!s.watch)
    s.watch.reset(new platform::FileWatch(rearm_and_reload));
  rearm_and_reload();
  // Rounding follows the live compositor only for the real theme.
  if (current_dir.empty() && !s.hypr_sub)
    s.hypr_sub = hypr::subscribe([](const std::string &ev, const std::string &) {
      if (ev == "configreloaded")
        refresh_rounding();
    });
}

void theme_overrides(int radius, bool local_palette) {
  State &s = state();
  s.forced_radius = radius < 0 ? -1 : radius;
  s.local_palette = local_palette;
  publish(s.has_live ? s.live : s.current);
}

int on_theme_change(std::function<void()> fn) {
  State &s = state();
  int id = s.next_id++;
  s.listeners[id] = std::move(fn);
  return id;
}

void remove_theme_listener(int id) { state().listeners.erase(id); }

} // namespace fo
