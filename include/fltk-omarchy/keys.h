// Key specs: "Ctrl+Shift+N", "Alt+Left", "F2", "Mod+,", "?", "Space".
//
// `Mod` is the platform's primary modifier (Ctrl on Linux and Windows, Cmd
// on macOS); `Ctrl` is always Control. Letters match case-insensitively
// with the Shift state as written; a symbol written without Shift ("?",
// "/") matches the character typed, whatever Shift it took.
#pragma once

#include <string>

namespace fo {

struct Key {
  int key = 0;  ///< FLTK key code (FL_Left, 'n', FL_F + 2, ...)
  int mods = 0; ///< FL_SHIFT | FL_CTRL | FL_ALT | FL_META
  bool valid() const { return key != 0; }
  bool operator==(const Key &o) const { return key == o.key && mods == o.mods; }
};

/// Parses a spec; an invalid Key (key == 0) when it cannot.
Key parse_key(const std::string &spec);
/// Human label in the toolkit's spelling: "Ctrl+Shift+N", "Alt+←", "⏎".
std::string key_label(const Key &k);
/// True when the event (key code, FLTK state, typed text) is `k`.
bool key_matches(const Key &k, int event_key, int event_state, const char *event_text);
/// key_matches() for the current FLTK event.
bool event_is(const Key &k);
bool event_is(const char *spec);

} // namespace fo
