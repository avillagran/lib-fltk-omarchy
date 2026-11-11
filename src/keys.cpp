#include "fltk-omarchy/keys.h"

#include <FL/Enumerations.H>
#include <FL/Fl.H>

#include <cstring>

namespace fo {

namespace {

struct Named {
  const char *name;
  int key;
  const char *label;
};

const Named kNamed[] = {
    {"left", FL_Left, "←"},         {"right", FL_Right, "→"},     {"up", FL_Up, "↑"},
    {"down", FL_Down, "↓"},         {"home", FL_Home, "Home"},    {"end", FL_End, "End"},
    {"pageup", FL_Page_Up, "PgUp"}, {"pagedown", FL_Page_Down, "PgDn"},
    {"enter", FL_Enter, "⏎"},       {"return", FL_Enter, "⏎"},    {"escape", FL_Escape, "Esc"},
    {"esc", FL_Escape, "Esc"},      {"tab", FL_Tab, "Tab"},       {"space", ' ', "Space"},
    {"backspace", FL_BackSpace, "Backspace"}, {"delete", FL_Delete, "Delete"},
    {"del", FL_Delete, "Delete"},   {"insert", FL_Insert, "Insert"}, {"menu", FL_Menu, "Menu"},
    {"plus", '+', "+"},
};

std::string lower(const std::string &s) {
  std::string out = s;
  for (auto &c : out)
    if (c >= 'A' && c <= 'Z')
      c = char(c + 32);
  return out;
}

int primary_mod() {
#if defined(__APPLE__)
  return FL_META;
#else
  return FL_CTRL;
#endif
}

} // namespace

Key parse_key(const std::string &spec) {
  Key k;
  if (spec.empty())
    return k;
  // Split on '+', keeping a trailing "+" key ("Ctrl++").
  std::string rest = spec;
  for (;;) {
    size_t plus = rest.find('+');
    if (plus == std::string::npos || plus == rest.size() - 1 || plus == 0)
      break;
    std::string mod = lower(rest.substr(0, plus));
    rest = rest.substr(plus + 1);
    if (mod == "ctrl" || mod == "control") k.mods |= FL_CTRL;
    else if (mod == "shift") k.mods |= FL_SHIFT;
    else if (mod == "alt" || mod == "option") k.mods |= FL_ALT;
    else if (mod == "super" || mod == "meta" || mod == "cmd" || mod == "win") k.mods |= FL_META;
    else if (mod == "mod" || mod == "primary") k.mods |= primary_mod();
    else return Key();
  }
  std::string name = lower(rest);
  for (const auto &n : kNamed)
    if (name == n.name) {
      k.key = n.key;
      return k;
    }
  if (name.size() >= 2 && name[0] == 'f' && name[1] >= '1' && name[1] <= '9') {
    int num = std::atoi(name.c_str() + 1);
    if (num >= 1 && num <= 24) {
      k.key = FL_F + num;
      return k;
    }
  }
  if (rest.size() == 1 && (unsigned char)rest[0] >= 0x20 && (unsigned char)rest[0] < 0x7f) {
    char c = rest[0];
    k.key = (c >= 'A' && c <= 'Z') ? c + 32 : c;
    return k;
  }
  return Key();
}

std::string key_label(const Key &k) {
  std::string s;
  if (k.mods & FL_CTRL) s += "Ctrl+";
#if defined(__APPLE__)
  if (k.mods & FL_META) s += "Cmd+";
#else
  if (k.mods & FL_META) s += "Super+";
#endif
  if (k.mods & FL_ALT) s += "Alt+";
  if (k.mods & FL_SHIFT) s += "Shift+";
  for (const auto &n : kNamed)
    if (n.key == k.key && std::strcmp(n.name, "return") && std::strcmp(n.name, "esc") &&
        std::strcmp(n.name, "del") && std::strcmp(n.name, "plus"))
      return s + n.label;
  if (k.key > FL_F && k.key <= FL_F + 24)
    return s + "F" + std::to_string(k.key - FL_F);
  if (k.key >= 'a' && k.key <= 'z')
    return s + char(k.key - 32);
  if (k.key > 0 && k.key < 0x7f)
    return s + char(k.key);
  return s + "?";
}

bool key_matches(const Key &k, int event_key, int event_state, const char *event_text) {
  if (!k.valid())
    return false;
  const int mask = FL_SHIFT | FL_CTRL | FL_ALT | FL_META;
  int state = event_state & mask;
  bool symbol = k.key > 0x20 && k.key < 0x7f && !(k.key >= 'a' && k.key <= 'z') &&
                !(k.key >= '0' && k.key <= '9');
  if (symbol && !(k.mods & FL_SHIFT)) {
    // "?" or "/" as typed: compare the text, ignore Shift.
    if ((state & ~FL_SHIFT) != k.mods)
      return false;
    if (event_text && event_text[0] == char(k.key) && event_text[1] == 0)
      return true;
    return event_key == k.key && !(state & FL_SHIFT);
  }
  int ek = event_key;
  if (ek >= 'A' && ek <= 'Z')
    ek += 32;
  if (ek == FL_KP_Enter)
    ek = FL_Enter;
  return ek == k.key && state == k.mods;
}

bool event_is(const Key &k) { return key_matches(k, Fl::event_key(), Fl::event_state(), Fl::event_text()); }
bool event_is(const char *spec) { return event_is(parse_key(spec)); }

} // namespace fo
