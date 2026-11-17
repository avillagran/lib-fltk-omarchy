// fo::dnd: dragging local files out of a widget, so other applications
// accept them.
//
// FLTK 1.4's Fl::dnd() offers only "text/plain;charset=utf-8" on Wayland,
// so file managers, browsers, chats and editors refuse a drag of files
// (they take "text/uri-list"). fo::dnd::start_files() runs that drag with
// its own Wayland data source, which offers:
//
//   text/uri-list               file:// URIs, CRLF-terminated (RFC 2483)
//   text/plain;charset=utf-8    the paths, one per line
//   text/plain                  the same
//
// Only the copy action is offered: the receiver copies (or moves on its own
// terms), and nothing on this side ever deletes the dragged files.
//
// Drops inside the same application never cross the compositor: the
// widget under the pointer gets FL_DND_RELEASE and then FL_PASTE with
// Fl::event_text() holding the text/uri-list payload, exactly as with
// FLTK's own Fl::dnd() (fo::init() installs the hook). Off Wayland it falls
// back to Fl::copy() + Fl::dnd().
//
// While it runs, a card follows the pointer: the item's icon and name (or
// the caller's label), with the number of items when there are several.
//
// Call it from the FL_DRAG that starts the drag (Collection::on_drag_start).
#pragma once

#include "fltk-omarchy/theme.h"

#include <cstdint>
#include <optional>
#include <string>
#include <vector>

class Fl_Window;

namespace fo {
namespace dnd {

// What the card under the pointer shows.
struct Preview {
  std::string label;         // "" = the first path's name
  std::string icon = "file"; // an fo::icon name
  std::optional<Rgb> color;  // the icon's color (default: the accent)
};

// Starts dragging `paths` (absolute local paths). Returns true when the
// drag started with the toolkit's data source, false when it fell back to
// Fl::dnd() (no Wayland, or the compositor lacks what it needs).
bool start_files(const std::vector<std::string> &paths, const Preview &preview = {});

// True while a start_files() drag is in progress.
bool active();

// The card as premultiplied ARGB8888 pixels at `scale` pixels per point
// (needs the display open; empty when FLTK can't draw offscreen). Exposed
// for tests and screenshots.
std::vector<uint32_t> card_pixels(const std::vector<std::string> &paths, const Preview &preview, int scale, int &w,
                                  int &h);

// The payloads it offers, exposed for tests.
std::string uri_list(const std::vector<std::string> &paths);
std::string plain_text(const std::vector<std::string> &paths);

// Toolkit internal, called by fo::init's event dispatch for every event:
// prepares the Wayland objects once the display is open and delivers the
// FL_DND_RELEASE of the toolkit's own drag inside the application. Returns
// true when it handled the event; `*result` is then what the dispatch
// returns.
bool intercept(int event, Fl_Window *window, int *result);

} // namespace dnd
} // namespace fo
