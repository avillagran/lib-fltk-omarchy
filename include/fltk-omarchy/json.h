// A tiny JSON reader for the subset the toolkit needs: Hyprland replies
// (`hyprctl -j`), Omarchy's locale registry and translation catalogs.
// Parses RFC 8259 JSON into a tree; no writer, no streaming.
#pragma once

#include <string>
#include <vector>

namespace fo {
namespace json {

struct Value {
  enum Type { Null, Bool, Number, String, Array, Object };
  Type type = Null;
  bool boolean = false;
  double number = 0;
  std::string string;            // String
  std::vector<Value> items;      // Array items, or Object values
  std::vector<std::string> keys; // Object keys, parallel to `items`

  bool is_null() const { return type == Null; }
  bool is_bool() const { return type == Bool; }
  bool is_number() const { return type == Number; }
  bool is_string() const { return type == String; }
  bool is_array() const { return type == Array; }
  bool is_object() const { return type == Object; }

  /// Member `key` of an object, or nullptr (also when not an object).
  const Value *get(const std::string &key) const;
  /// Convenience readers with a fallback when missing or of another type.
  std::string str(const std::string &key, const std::string &fallback = "") const;
  double num(const std::string &key, double fallback = 0) const;
  bool flag(const std::string &key, bool fallback = false) const;
};

/// Parses `text`. Returns false (and leaves `out` Null) on any syntax error.
bool parse(const std::string &text, Value &out);

} // namespace json
} // namespace fo
