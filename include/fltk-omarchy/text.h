// Text: the theme font, measuring, eliding and drawing.
//
// apply_fonts() maps FLTK's built-in faces to the Omarchy font, so stock
// FLTK widgets match too: FL_HELVETICA regular, FL_HELVETICA_BOLD bold,
// FL_TIMES medium, FL_TIMES_BOLD semibold, FL_COURIER* the same family.
// Sizes are FLTK units (logical pixels); FLTK scales them on HiDPI
// screens, so measured widths and drawing stay consistent at any scale.
#pragma once

#include <FL/Enumerations.H>

#include <functional>
#include <string>
#include <vector>

namespace fo {

struct Rgb;

namespace text {

constexpr Fl_Font REGULAR = FL_HELVETICA;
constexpr Fl_Font BOLD = FL_HELVETICA_BOLD;
constexpr Fl_Font MEDIUM = FL_TIMES;
constexpr Fl_Font SEMIBOLD = FL_TIMES_BOLD;

/// Folio's type scale, in FLTK units.
namespace size {
constexpr int SMALL = 11;   ///< metadata, section labels, shortcut hints
constexpr int LABEL = 12;   ///< tile names, secondary text
constexpr int BODY = 13;    ///< rows, menus, fields
constexpr int TITLE = 14;   ///< path bar
constexpr int HEADING = 16; ///< dialog titles
} // namespace size

/// Points FLTK's faces at `family` (a Pango/fontconfig family name).
void apply_fonts(const std::string &family);

/// Width of `n` bytes of UTF-8 text. Used to make the eliding testable.
using Measure = std::function<double(const char *text, int n)>;
/// Measure with FLTK's current font after fl_font(font, size).
Measure fltk_measure(Fl_Font font, int size);

/// Byte offsets of every UTF-8 code point start, plus the end.
std::vector<int> utf8_offsets(const std::string &s);

/// "a long name…" so it fits in `max_w`. Unchanged when it fits.
std::string elide_end(const std::string &s, double max_w, const Measure &m);
/// "a long…name.jpg": keeps the start and the end (with the extension,
/// when there is a short one) and puts "…" in the middle. For file names.
std::string elide_middle(const std::string &s, double max_w, const Measure &m);
/// Breaks `s` into at most `max_lines` lines of `max_w` (at spaces, '-',
/// '_', '.' when possible, anywhere otherwise); the last line is elided.
std::vector<std::string> wrap(const std::string &s, double max_w, const Measure &m, int max_lines);

/// Uppercase (ASCII and the Unicode letters fl_toupper knows).
std::string upper(const std::string &s);

/// Width with fl_font(font, size).
double width(const std::string &s, Fl_Font font, int size);

enum class Elide { None, End, Middle };

/// Draws `s` in the box, vertically centered, aligned by `align`
/// (FL_ALIGN_LEFT / CENTER / RIGHT), elided to fit.
void draw(const std::string &s, int x, int y, int w, int h, Rgb color, Fl_Font font = REGULAR,
          int size = size::BODY, Fl_Align align = FL_ALIGN_LEFT, Elide elide = Elide::End);
/// Draws with extra spacing between letters (section labels); returns the width.
int draw_spaced(const std::string &s, int x, int baseline, double spacing, Rgb color, Fl_Font font, int size);
/// Width that draw_spaced() would take.
int width_spaced(const std::string &s, double spacing, Fl_Font font, int size);

} // namespace text
} // namespace fo
