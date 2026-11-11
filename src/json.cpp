#include "fltk-omarchy/json.h"

#include <cstdlib>
#include <cstring>

namespace fo {
namespace json {

const Value *Value::get(const std::string &key) const {
  if (type != Object)
    return nullptr;
  for (size_t i = 0; i < keys.size(); ++i)
    if (keys[i] == key)
      return &items[i];
  return nullptr;
}

std::string Value::str(const std::string &key, const std::string &fallback) const {
  const Value *v = get(key);
  return v && v->is_string() ? v->string : fallback;
}

double Value::num(const std::string &key, double fallback) const {
  const Value *v = get(key);
  return v && v->is_number() ? v->number : fallback;
}

bool Value::flag(const std::string &key, bool fallback) const {
  const Value *v = get(key);
  if (v && v->is_bool())
    return v->boolean;
  if (v && v->is_number())
    return v->number != 0;
  return fallback;
}

namespace {

struct Parser {
  const char *p, *end;
  int depth = 0;

  void ws() {
    while (p < end && (*p == ' ' || *p == '\t' || *p == '\n' || *p == '\r'))
      ++p;
  }

  static void put_utf8(std::string &s, unsigned cp) {
    if (cp < 0x80) {
      s += char(cp);
    } else if (cp < 0x800) {
      s += char(0xC0 | (cp >> 6));
      s += char(0x80 | (cp & 0x3F));
    } else if (cp < 0x10000) {
      s += char(0xE0 | (cp >> 12));
      s += char(0x80 | ((cp >> 6) & 0x3F));
      s += char(0x80 | (cp & 0x3F));
    } else {
      s += char(0xF0 | (cp >> 18));
      s += char(0x80 | ((cp >> 12) & 0x3F));
      s += char(0x80 | ((cp >> 6) & 0x3F));
      s += char(0x80 | (cp & 0x3F));
    }
  }

  bool hex4(unsigned &out) {
    if (end - p < 4)
      return false;
    out = 0;
    for (int i = 0; i < 4; ++i) {
      char c = *p++;
      out <<= 4;
      if (c >= '0' && c <= '9') out |= unsigned(c - '0');
      else if (c >= 'a' && c <= 'f') out |= unsigned(c - 'a' + 10);
      else if (c >= 'A' && c <= 'F') out |= unsigned(c - 'A' + 10);
      else return false;
    }
    return true;
  }

  bool string(std::string &out) {
    if (p >= end || *p != '"')
      return false;
    ++p;
    while (p < end) {
      char c = *p++;
      if (c == '"')
        return true;
      if ((unsigned char)c < 0x20)
        return false;
      if (c != '\\') {
        out += c;
        continue;
      }
      if (p >= end)
        return false;
      char e = *p++;
      switch (e) {
      case '"': out += '"'; break;
      case '\\': out += '\\'; break;
      case '/': out += '/'; break;
      case 'b': out += '\b'; break;
      case 'f': out += '\f'; break;
      case 'n': out += '\n'; break;
      case 'r': out += '\r'; break;
      case 't': out += '\t'; break;
      case 'u': {
        unsigned cp;
        if (!hex4(cp))
          return false;
        if (cp >= 0xD800 && cp <= 0xDBFF) {
          unsigned lo;
          if (end - p < 6 || p[0] != '\\' || p[1] != 'u')
            return false;
          p += 2;
          if (!hex4(lo) || lo < 0xDC00 || lo > 0xDFFF)
            return false;
          cp = 0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00);
        }
        put_utf8(out, cp);
        break;
      }
      default:
        return false;
      }
    }
    return false;
  }

  bool literal(const char *word) {
    size_t n = std::strlen(word);
    if (size_t(end - p) < n || std::strncmp(p, word, n) != 0)
      return false;
    p += n;
    return true;
  }

  bool number(double &out) {
    const char *start = p;
    if (p < end && *p == '-') ++p;
    if (p >= end || !(*p >= '0' && *p <= '9')) return false;
    while (p < end && ((*p >= '0' && *p <= '9') || *p == '.' || *p == 'e' || *p == 'E' || *p == '+' || *p == '-'))
      ++p;
    std::string s(start, p);
    char *stop = nullptr;
    out = std::strtod(s.c_str(), &stop);
    return stop && *stop == 0;
  }

  bool value(Value &v) {
    if (++depth > 64)
      return false;
    ws();
    if (p >= end)
      return false;
    bool ok = false;
    switch (*p) {
    case '{': {
      ++p;
      v.type = Value::Object;
      ws();
      if (p < end && *p == '}') { ++p; ok = true; break; }
      for (;;) {
        ws();
        std::string key;
        if (!string(key)) return false;
        ws();
        if (p >= end || *p != ':') return false;
        ++p;
        Value child;
        if (!value(child)) return false;
        v.keys.push_back(std::move(key));
        v.items.push_back(std::move(child));
        ws();
        if (p < end && *p == ',') { ++p; continue; }
        if (p < end && *p == '}') { ++p; ok = true; }
        break;
      }
      break;
    }
    case '[': {
      ++p;
      v.type = Value::Array;
      ws();
      if (p < end && *p == ']') { ++p; ok = true; break; }
      for (;;) {
        Value child;
        if (!value(child)) return false;
        v.items.push_back(std::move(child));
        ws();
        if (p < end && *p == ',') { ++p; continue; }
        if (p < end && *p == ']') { ++p; ok = true; }
        break;
      }
      break;
    }
    case '"':
      v.type = Value::String;
      ok = string(v.string);
      break;
    case 't': v.type = Value::Bool; v.boolean = true; ok = literal("true"); break;
    case 'f': v.type = Value::Bool; ok = literal("false"); break;
    case 'n': ok = literal("null"); break;
    default:
      v.type = Value::Number;
      ok = number(v.number);
    }
    --depth;
    return ok;
  }
};

} // namespace

bool parse(const std::string &text, Value &out) {
  Parser ps{text.data(), text.data() + text.size()};
  Value v;
  if (!ps.value(v))
    return out = Value(), false;
  ps.ws();
  if (ps.p != ps.end)
    return out = Value(), false;
  out = std::move(v);
  return true;
}

} // namespace json
} // namespace fo
