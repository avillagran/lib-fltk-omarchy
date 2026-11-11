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

/// Drops every cached raster (the theme listener calls this on changes).
void clear_cache();
size_t cache_entries();

/// Lucide's ISC license text, to show in an about box.
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
