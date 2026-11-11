#include "fltk-omarchy/icons.h"

#include <FL/Fl_SVG_Image.H>

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
} // namespace fo
