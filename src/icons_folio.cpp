// Folio's line icons (github.com/omacom/folio, src/Glyph.qml and the folder
// silhouette of src/FileTile.qml), ported as SVG path data on the same
// 24-unit grid with the same 1.6 stroke, square ends on square themes and
// round ends when the theme rounds, and the boxes' corners rounded by 2
// then. Folio is MIT licensed: Copyright (c) 2026 David Heinemeier Hansson;
// the notice is in third_party/folio/LICENSE and in icon::license().
#include "fltk-omarchy/icons.h"

#include <cstdio>
#include <string>

namespace fo {
namespace icon {
namespace folio {

namespace {

std::string num(double v) {
  char b[32];
  std::snprintf(b, sizeof b, "%g", v);
  return b;
}

std::string circle(double cx, double cy, double r) {
  return "M" + num(cx - r) + " " + num(cy) + "a" + num(r) + " " + num(r) + " 0 1 0 " + num(2 * r) + " 0a" + num(r) +
         " " + num(r) + " 0 1 0 " + num(-2 * r) + " 0";
}

std::string box(double x, double y, double w, double h, double rad) {
  if (rad <= 0)
    return "M" + num(x) + " " + num(y) + "h" + num(w) + "v" + num(h) + "h" + num(-w) + "z";
  double x2 = x + w, y2 = y + h;
  return "M" + num(x + rad) + " " + num(y) + "H" + num(x2 - rad) + "Q" + num(x2) + " " + num(y) + " " + num(x2) + " " +
         num(y + rad) + "V" + num(y2 - rad) + "Q" + num(x2) + " " + num(y2) + " " + num(x2 - rad) + " " + num(y2) +
         "H" + num(x + rad) + "Q" + num(x) + " " + num(y2) + " " + num(x) + " " + num(y2 - rad) + "V" + num(y + rad) +
         "Q" + num(x) + " " + num(y) + " " + num(x + rad) + " " + num(y) + "Z";
}

std::string page(bool round) {
  return round ? "M8 3H14L18 7V19Q18 21 16 21H8Q6 21 6 19V5Q6 3 8 3ZM14 3v4h4" : "M6 3h8l4 4v14H6zM14 3v4h4";
}

std::string folder_box(bool round) {
  return round ? "M3 8Q3 6 5 6H10L12 8H19Q21 8 21 10V17Q21 19 19 19H5Q3 19 3 17Z" : "M3 6h7l2 2h9v11H3z";
}

// The names apps use (Files' and the toolkit's) for Folio's drawings.
const char *folio_name(const std::string &n) {
  struct Alias {
    const char *from, *to;
  };
  static const Alias kAliases[] = {
      {"documents", "documents"},  {"document", "documents"},   {"file-text", "documents"}, {"text", "documents"},
      {"pdf", "documents"},        {"file-code", "documents"},  {"code", "documents"},
      {"downloads", "downloads"},  {"download", "downloads"},
      {"pictures", "pictures"},    {"picture", "pictures"},     {"image", "pictures"},      {"file-image", "pictures"},
      {"music", "music"},          {"file-music", "music"},
      {"videos", "videos"},        {"video", "videos"},         {"film", "videos"},         {"file-video", "videos"},
      {"recent", "recent"},        {"clock", "recent"},         {"history", "recent"},
      {"trash", "trash"},          {"trash-2", "trash"},
      {"drive", "drive"},          {"hard-drive", "drive"},     {"database", "drive"},      {"usb", "drive"},
      {"removable", "drive"},
      {"eject", "eject"},
      {"peer", "peer"},            {"laptop", "peer"},          {"monitor", "peer"},        {"desktop", "peer"},
      {"server", "peer"},
      {"network", "tailnet"},      {"tailnet", "tailnet"},      {"globe", "tailnet"},
      {"folder", "folder"},        {"folder-open", "folder"},   {"open-folder", "folder"},
      {"back", "back"},            {"arrow-left", "back"},      {"chevron-left", "back"},
      {"forward", "forward"},      {"arrow-right", "forward"},  {"chevron-right", "forward"},
      {"down", "down"},            {"chevron-down", "down"},
      {"filter", "filter"},
      {"grid", "grid"},            {"layout-grid", "grid"},
      {"list", "list"},
      {"columns", "columns"},      {"columns-3", "columns"},    {"columns-2", "columns"},   {"split", "columns"},
      {"preview", "preview"},      {"panel", "preview"},        {"panel-right", "preview"}, {"inspector", "preview"},
      {"sort", "sort-name"},       {"arrow-down-a-z", "sort-name"}, {"sort-name", "sort-name"},
      {"sort-kind", "sort-kind"},  {"sort-newest", "sort-newest"}, {"sort-oldest", "sort-oldest"},
      {"sort-size", "sort-size"},
      {"play", "play"},            {"pause", "pause"},
      {"open", "open"},            {"external-link", "open"},
      {"audio", "audio"},
      {"check", "check"},
      {"close", "close"},          {"x", "close"},              {"filter-x", "close"},
      {"file", "file"},            {"file-plus", "file"},       {"file-archive", "file"},   {"archive-file", "file"},
      {"archive", "file"},
  };
  for (const auto &a : kAliases)
    if (n == a.from)
      return a.to;
  return nullptr;
}

} // namespace

bool has(const std::string &name) { return folio_name(name) != nullptr; }

std::string path(const std::string &name, bool round) {
  const char *f = folio_name(name);
  if (!f)
    return {};
  const std::string n = f;
  const double r = round ? 2 : 0;
  if (n == "documents") return page(round) + "M9 12h6M9 16h6";
  if (n == "downloads") return "M12 4v11M7 10l5 5 5-5M4 20h16";
  if (n == "pictures") return box(3, 5, 18, 14, r) + "M3 16l5-5 4 4 3-3 6 6" + circle(16, 9, 1.5);
  if (n == "music") return "M9 17V5l11-2v12" + circle(6.5, 17, 2.5) + circle(17.5, 15, 2.5);
  if (n == "videos") return box(3, 5, 18, 14, r) + "M10 9v6l5-3z";
  if (n == "recent") return circle(12, 12, 9) + "M12 7v5l3 2";
  if (n == "trash") return "M4 7h16M9 7V4h6v3M6 7l1 13h10l1-13";
  if (n == "drive") return box(3, 6, 18, 12, r) + "M7 14h2";
  if (n == "eject") return "M12 5l7 8H5zM5 18h14";
  if (n == "peer") return box(4, 4, 16, 11, r) + "M9 20h6M12 15v5";
  if (n == "tailnet")
    return circle(12, 5, 2.5) + circle(5, 18, 2.5) + circle(19, 18, 2.5) + "M10.8 7.2L6.2 15.8M13.2 7.2l4.6 8.6M7.5 18h9";
  if (n == "folder") return folder_box(round);
  if (n == "back") return "M15 6l-6 6 6 6";
  if (n == "forward") return "M9 6l6 6-6 6";
  if (n == "down") return "M6 9l6 6 6-6";
  if (n == "filter") return "M4 5h16l-6 7v6l-4 2v-8z";
  if (n == "grid")
    return box(4, 4, 6, 6, r / 2) + box(14, 4, 6, 6, r / 2) + box(4, 14, 6, 6, r / 2) + box(14, 14, 6, 6, r / 2);
  if (n == "list") return "M4 6h16M4 12h16M4 18h16";
  if (n == "columns") return box(3, 4, 18, 16, r) + "M9 4v16M15 4v16";
  if (n == "preview") return box(3, 5, 18, 14, r) + "M14 5v14";
  if (n == "sort-name") return "M6 4v16M3 17l3 3 3-3M12 11l3-7 3 7M13.2 8.4h3.6M12 14h6l-6 6h6";
  if (n == "sort-kind") return "M6 4v16M3 17l3 3 3-3" + circle(16.5, 7.5, 3) + box(13.5, 14, 6, 6, r / 2);
  if (n == "sort-newest") return "M6 4v16M3 17l3 3 3-3" + circle(16.5, 12, 5) + "M16.5 9v3.2l2.2 1.4";
  if (n == "sort-oldest") return "M6 20V4M3 7l3-3 3 3" + circle(16.5, 12, 5) + "M16.5 9v3.2l2.2 1.4";
  if (n == "sort-size") return "M6 4v16M3 17l3 3 3-3M12 6h9M12 12h6.5M12 18h4";
  if (n == "play") return "M8 5v14l11-7z";
  if (n == "pause") return "M8 5v14M16 5v14";
  if (n == "open") return "M14 4h6v6M20 4l-9 9M18 14v6H4V6h6";
  if (n == "audio") return "M4 10v4M8 7v10M12 4v16M16 8v8M20 11v2";
  if (n == "check") return "M5 12l5 5 9-10";
  if (n == "close") return "M6 6l12 12M18 6L6 18";
  return page(round); // "file" and anything else: a page
}

std::string folder_silhouette(double r) {
  // FileTile.qml folderPath(): a card with a tab on its top left edge, on a
  // 128 x 92 box.
  if (r <= 0)
    return "M4.5 88.5V0.5H46L50 6.5H123.5V88.5Z";
  const double t = r < 3 ? r : 3;
  return "M4.5 " + num(88.5 - r) + "V" + num(0.5 + r) + "Q4.5 0.5 " + num(4.5 + r) + " 0.5" + "H" + num(46 - t) +
         "Q46 0.5 " + num(46 + t * 0.6) + " " + num(0.5 + t * 0.9) + "L" + num(50 - t * 0.6) + " " + num(6.5 - t * 0.9) +
         "Q50 6.5 " + num(50 + t) + " 6.5H" + num(123.5 - r) + "Q123.5 6.5 123.5 " + num(6.5 + r) + "V" + num(88.5 - r) +
         "Q123.5 88.5 " + num(123.5 - r) + " 88.5H" + num(4.5 + r) + "Q4.5 88.5 4.5 " + num(88.5 - r) + "Z";
}

const char *license() {
  return "Folio icons (github.com/omacom/folio), MIT License\n\n"
         "Copyright (c) 2026 David Heinemeier Hansson\n\n"
         "Permission is hereby granted, free of charge, to any person obtaining a copy of this software and "
         "associated documentation files (the \"Software\"), to deal in the Software without restriction, including "
         "without limitation the rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell "
         "copies of the Software, and to permit persons to whom the Software is furnished to do so, subject to the "
         "following conditions:\n\n"
         "The above copyright notice and this permission notice shall be included in all copies or substantial "
         "portions of the Software.\n\n"
         "THE SOFTWARE IS PROVIDED \"AS IS\", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT "
         "LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO "
         "EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN "
         "AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE "
         "OR OTHER DEALINGS IN THE SOFTWARE.\n";
}

} // namespace folio
} // namespace icon
} // namespace fo
