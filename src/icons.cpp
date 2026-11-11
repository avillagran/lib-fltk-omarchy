#include "fltk-omarchy/icons.h"

#include "fltk-omarchy/text.h"

#include <FL/Fl_SVG_Image.H>
#include <FL/fl_draw.H>
#include <FL/fl_utf8.h>

#include <algorithm>
#include <cstdio>
#include <cstring>
#include <list>
#include <memory>
#include <unordered_map>

namespace fo {
namespace icon {

namespace {

struct Entry {
  const char *name;
  const char *body;
};

const Entry kIcons[] = {
#include "icons_data.inc"
};

struct Alias {
  const char *alias, *name;
};

const Alias kAliases[] = {
    {"archive-file", "file-archive"}, {"audio", "file-music"},   {"close", "x"},
    {"code", "file-code"},           {"columns", "columns-3"},   {"desktop", "monitor"},
    {"documents", "file-text"},      {"drive", "hard-drive"},    {"edit", "pencil"},
    {"grid", "layout-grid"},         {"hidden", "eye"},          {"home", "house"},
    {"more", "ellipsis"},            {"panel", "panel-right"},   {"pdf", "file-text"},
    {"pictures", "image"},           {"recent", "clock"},        {"refresh", "refresh-cw"},
    {"sidebar", "panel-left"},       {"sort", "arrow-down-a-z"}, {"split", "columns-2"},
    {"text", "file-text"},           {"trash", "trash-2"},       {"videos", "video"},
    {"downloads", "download"},       {"peer", "laptop"},         {"preview", "panel-right"},
};

const Entry *find(const std::string &name) {
  const Entry *begin = kIcons, *end = kIcons + sizeof kIcons / sizeof kIcons[0];
  const Entry *it = std::lower_bound(begin, end, name.c_str(),
                                     [](const Entry &e, const char *n) { return std::strcmp(e.name, n) < 0; });
  return it != end && name == it->name ? it : nullptr;
}

// A small LRU of rasters: lists draw each icon once per size and color.
struct Cache {
  using Key = std::string;
  std::list<std::pair<Key, std::unique_ptr<Fl_SVG_Image>>> order;
  std::unordered_map<Key, decltype(order)::iterator> map;
  size_t limit = 256;
};
Cache &cache() {
  static Cache c;
  return c;
}

} // namespace

std::string resolve(const std::string &name) {
  for (const auto &a : kAliases)
    if (name == a.alias)
      return a.name;
  return name;
}

bool exists(const std::string &name) {
  if (find(name))
    return true;
  return find(resolve(name)) != nullptr;
}

std::vector<std::string> names() {
  std::vector<std::string> v;
  for (const auto &e : kIcons)
    v.push_back(e.name);
  return v;
}

std::string svg(const std::string &name, Rgb color, double stroke, bool sharp) {
  const Entry *e = find(name);
  if (!e)
    e = find(resolve(name));
  if (!e)
    e = find("file");
  char head[320];
  std::snprintf(head, sizeof head,
                "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"24\" height=\"24\" viewBox=\"0 0 24 24\" "
                "fill=\"none\" stroke=\"%s\" stroke-width=\"%.2f\" stroke-linecap=\"%s\" "
                "stroke-linejoin=\"%s\">",
                to_hex(color).c_str(), stroke, sharp ? "square" : "round", sharp ? "miter" : "round");
  return std::string(head) + e->body + "</svg>";
}

void draw(const std::string &name, int x, int y, int size, Rgb color, double stroke) {
  if (size <= 0)
    return;
  if (glyph::available()) {
    const char *g = glyph::for_icon(name);
    if (*g) {
      glyph::draw(g, x, y, size, size, std::max(8, size * 7 / 8), color);
      return;
    }
  }
  if (stroke <= 0)
    stroke = size >= 20 ? 2.0 : 1.75;
  bool sharp = theme().radius() == 0;
  char key[160];
  std::snprintf(key, sizeof key, "%s|%d|%06x|%.2f|%d", name.c_str(), size,
                (color.r << 16) | (color.g << 8) | color.b, stroke, sharp ? 1 : 0);
  Cache &c = cache();
  Fl_SVG_Image *img = nullptr;
  auto it = c.map.find(key);
  if (it != c.map.end()) {
    c.order.splice(c.order.begin(), c.order, it->second);
    img = it->second->second.get();
  } else {
    std::string doc = svg(name, color, stroke, sharp);
    std::unique_ptr<Fl_SVG_Image> fresh(new Fl_SVG_Image(nullptr, doc.c_str()));
    if (fresh->fail())
      return;
    fresh->proportional = true;
    fresh->scale(size, size, 1, 1);
    img = fresh.get();
    c.order.emplace_front(key, std::move(fresh));
    c.map[key] = c.order.begin();
    while (c.order.size() > c.limit) {
      c.map.erase(c.order.back().first);
      c.order.pop_back();
    }
  }
  img->draw(x, y);
}

void clear_cache() {
  cache().order.clear();
  cache().map.clear();
}

size_t cache_entries() { return cache().order.size(); }

const char *license() {
  return "Lucide icons, ISC License\n\n"
         "Copyright (c) for portions of Lucide are held by Cole Bemis 2013-2022 as part of Feather (MIT). "
         "All other copyright (c) for Lucide are held by Lucide Contributors 2022.\n\n"
         "Permission to use, copy, modify, and/or distribute this software for any purpose with or without fee "
         "is hereby granted, provided that the above copyright notice and this permission notice appear in all "
         "copies.\n\n"
         "THE SOFTWARE IS PROVIDED \"AS IS\" AND THE AUTHOR DISCLAIMS ALL WARRANTIES WITH REGARD TO THIS "
         "SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE "
         "AUTHOR BE LIABLE FOR ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES "
         "WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN ACTION OF CONTRACT, NEGLIGENCE "
         "OR OTHER TORTIOUS ACTION, ARISING OUT OF OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS "
         "SOFTWARE.\n";
}

} // namespace icon

namespace glyph {

namespace {

bool g_available = false;

struct Glyph {
  const char *name;
  unsigned cp;
};

// Files' glyphs (crates/ui: toolbar.rs, view.rs, toolkit_menu.rs) and
// the closest Font Awesome / Material Design glyph for the other names.
const Glyph kGlyphs[] = {
    {"search", 0xf002},        {"x", 0xf00d},           {"close", 0xf00d},
    {"pencil", 0xf040},        {"edit", 0xf044},        {"square-pen", 0xf044},
    {"groups", 0xf00a},        {"list", 0xf0c9},        {"grid", 0xf009},
    {"layout-grid", 0xf009},   {"space", 0xf200},       {"pie-chart", 0xf200},
    {"columns", 0xf0db},       {"columns-3", 0xf0db},   {"columns-2", 0xf0db},
    {"split", 0xf0db},         {"info", 0xf05a},        {"inspector", 0xf05a},
    {"panel", 0xf05a},         {"panel-right", 0xf05a}, {"actions", 0xf0e7},
    {"zap", 0xf0e7},           {"settings", 0xf013},    {"arrow-left", 0xf060},
    {"arrow-right", 0xf061},   {"arrow-up", 0xf062},    {"arrow-down", 0xf063},
    {"check", 0xf00c},         {"star", 0xf005},        {"star-outline", 0xf006},
    {"recent", 0xf017},        {"clock", 0xf017},       {"history", 0xf1da},
    {"trash", 0xf1f8},         {"trash-2", 0xf1f8},     {"home", 0xf015},
    {"house", 0xf015},         {"downloads", 0xf019},   {"download", 0xf019},
    {"documents", 0xf15c},     {"file-text", 0xf15c},   {"text", 0xf15c},
    {"pictures", 0xf03e},      {"image", 0xf03e},       {"music", 0xf001},
    {"audio", 0xf001},         {"videos", 0xf008},      {"video", 0xf008},
    {"film", 0xf008},          {"drive", 0xf1c0},       {"hard-drive", 0xf0a0},
    {"database", 0xf1c0},      {"usb", 0xf287},         {"folder", 0xf07b},
    {"folder-open", 0xf07c},   {"folder-plus", 0xf0257}, {"eye", 0xf06e},
    {"hidden", 0xf06e},        {"eye-off", 0xf070},     {"terminal", 0xf120},
    {"globe", 0xf0ac},         {"network", 0xf0ac},     {"copy", 0xf0c5},
    {"scissors", 0xf0c4},      {"cut", 0xf0c4},         {"clipboard", 0xf0ea},
    {"paste", 0xf0ea},         {"undo", 0xf0e2},        {"redo", 0xf01e},
    {"refresh", 0xf021},       {"refresh-cw", 0xf021},  {"file", 0xf016},
    {"file-plus", 0xf016},     {"link", 0xf0c1},        {"plus", 0xf067},
    {"pin", 0xf08d},           {"chevron-right", 0xf054}, {"chevron-left", 0xf053},
    {"chevron-down", 0xf078},  {"chevron-up", 0xf077},  {"filter", 0xf0b0},
    {"calculator", 0xf1ec},    {"sort", 0xf15d},        {"arrow-down-a-z", 0xf15d},
    {"cloud", 0xf0c2},         {"eject", 0xf052},       {"share", 0xf1e0},
    {"share-2", 0xf1e0},       {"send", 0xf1d8},        {"keyboard", 0xf11c},
    {"help", 0xf059},          {"circle-help", 0xf059}, {"save", 0xf0c7},
    {"play", 0xf04b},          {"pause", 0xf04c},       {"external-link", 0xf08e},
    {"lock", 0xf023},          {"server", 0xf233},      {"laptop", 0xf109},
    {"peer", 0xf109},          {"monitor", 0xf108},     {"desktop", 0xf108},
    {"archive", 0xf187},       {"code", 0xf121},        {"file-code", 0xf1c9},
    {"pdf", 0xf1c1},           {"file-image", 0xf1c5},  {"file-music", 0xf1c7},
    {"file-video", 0xf1c8},    {"file-archive", 0xf1c6}, {"archive-file", 0xf1c6},
    {"warning", 0xf071},       {"triangle-alert", 0xf071}, {"more", 0xf141},
    {"ellipsis", 0xf141},      {"sidebar", 0xf0c9},     {"panel-left", 0xf0c9},
    {"tag", 0xf02b},           {"bookmark", 0xf02e},    {"select", 0xf012c},
    {"square-check", 0xf14a},
    {"layers", 0xf0c5},        {"open", 0xf07c},        {"preview", 0xf06e},
    {"templates", 0xf0c5},     {"public", 0xf0ac},      {"removable", 0xf287},
    {"sort-desc", 0xf063},     {"sort-asc", 0xf062},    {"filter-x", 0xf00d},
    {"compress", 0xf01a7},     {"extract", 0xf01da},    {"properties", 0xf05a},
};

} // namespace

bool available() { return g_available; }
void set_available(bool on) { g_available = on; }

const char *for_icon(const std::string &name) {
  // The UTF-8 for each glyph, built once.
  static std::unordered_map<std::string, std::string> table = [] {
    std::unordered_map<std::string, std::string> t;
    for (const auto &g : kGlyphs) {
      char buf[8];
      int n = fl_utf8encode(g.cp, buf);
      t.emplace(g.name, std::string(buf, size_t(n)));
    }
    return t;
  }();
  auto it = table.find(name);
  return it == table.end() ? "" : it->second.c_str();
}

void draw(const char *utf8, int x, int y, int w, int h, int size, Rgb color) {
  if (!utf8 || !*utf8 || w <= 0 || h <= 0)
    return;
  fl_font(text::REGULAR, size);
  int n = int(std::strlen(utf8));
  int dx = 0, dy = 0, gw = 0, gh = 0;
  fl_text_extents(utf8, n, dx, dy, gw, gh);
  fl_color(color.fl());
  // Center the ink, not the advance: Nerd glyphs sit unevenly in their cell.
  int tx = x + (w - gw) / 2 - dx;
  int ty = y + (h - gh) / 2 - dy;
  fl_draw(utf8, n, tx, ty);
}

} // namespace glyph
} // namespace fo
