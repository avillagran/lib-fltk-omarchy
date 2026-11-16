// A small embedded set of Lucide icons (ISC license, see license()) drawn
// with FLTK's Fl_SVG_Image: tinted with any color, rasterized at the
// screen's real resolution, cached per (name, size, color, stroke).
//
// Names are Lucide's ("folder", "file-text", "chevron-right") plus
// file-manager aliases: home, desktop, documents, downloads, pictures,
// music, videos, recent, trash, drive, usb, network, cloud, grid, columns,
// more, close, refresh, sort, sidebar, panel, split, edit, code, archive,
// pdf, text, audio. Unknown names draw "file".
//
// On Omarchy's square theme (radius 0) strokes use square caps and miter
// joins; rounded themes keep Lucide's round ends.
//
// When the UI font is a Nerd Font (Omarchy's default, JetBrainsMono Nerd
// Font), the names Files uses draw as that font's glyphs instead, as
// Files' header, rail and menus do: fo::glyph maps them to code points of
// the Font Awesome and Material Design ranges. Without a Nerd Font every
// name falls back to the Lucide drawing.
//
//   fo::icon::draw("folder", x, y, 18, fo::theme().accent());
//
// Icon styles: Style::Files (the default) is the above. Style::Folio
// draws Folio's line icons instead (github.com/omacom/folio, MIT): its
// own drawings for the names it has (folder, documents, downloads,
// pictures, music, videos, recent, trash, drive, eject, peer, the network,
// back/forward, filter, grid, list, columns, preview, sorting, play, open,
// check, close and a page for any file), and the Lucide drawing in the same
// 1.6 stroke for the rest, never the Nerd Font glyphs, so every icon is one
// line family. draw_folder() is Folio's flat folder silhouette.
#pragma once

#include "fltk-omarchy/theme.h"

#include <string>
#include <vector>

namespace fo {
namespace icon {

/// True for a Lucide name or an alias that the set contains.
bool exists(const std::string &name);
/// Every embedded Lucide name (without aliases), sorted.
std::vector<std::string> names();
/// The Lucide name an alias stands for (the name itself otherwise).
std::string resolve(const std::string &name);

/// Draws icon `name` in the `size` x `size` box at (x, y). `stroke` is the
/// stroke width in icon units on Lucide's 24 grid (0: 2 at >= 20 px, a
/// lighter 1.75 below).
void draw(const std::string &name, int x, int y, int size, Rgb color, double stroke = 0);

/// The full SVG document for an icon, ready for Fl_SVG_Image.
std::string svg(const std::string &name, Rgb color, double stroke = 2, bool sharp = false);

enum class Style { Files, Folio };
/// Chooses the icon family for every draw() (see the file comment).
void set_style(Style s);
Style style();

/// Folio's flat folder (a card with a tab at its top left, from Folio's
/// FileTile.qml) fitted in the box at its 128:92 shape: filled with `fill`,
/// outlined with `edge` when `edge_width` > 0. Corners follow the theme.
void draw_folder(int x, int y, int w, int h, Rgb fill, Rgb edge, double edge_width = 0);

/// Folio's drawings as SVG path data on the 24-unit grid.
namespace folio {
bool has(const std::string &name);
/// `round`: the rounded variant (themes that round windows).
std::string path(const std::string &name, bool round);
/// The folder silhouette on a 128 x 92 box, its corners rounded by `r`.
std::string folder_silhouette(double r);
const char *license();
} // namespace folio

/// Drops every cached raster (the theme listener calls this on changes).
void clear_cache();
size_t cache_entries();

/// The icons' license texts (Lucide's ISC, then Folio's MIT), to show in
/// an about box.
const char *license();

} // namespace icon

namespace glyph {

/// True when the UI font has the Nerd Font glyphs (set by the theme when
/// it resolves the font; tests may force it).
bool available();
void set_available(bool on);
/// The UTF-8 Nerd Font glyph for an icon name or alias ("" for none).
const char *for_icon(const std::string &name);
/// Draws UTF-8 text (a glyph, or a glyph and a label) centered in the box
/// with the UI font at `size`.
void draw(const char *utf8, int x, int y, int w, int h, int size, Rgb color);

} // namespace glyph
} // namespace fo
