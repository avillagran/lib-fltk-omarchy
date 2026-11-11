// Language following and plural rules after Folio's translator.cpp.
#include "fltk-omarchy/i18n.h"

#include "fltk-omarchy/json.h"
#include "fltk-omarchy/platform.h"

#include <FL/Fl.H>
#include <FL/Fl_Window.H>

#include <clocale>
#include <cmath>
#include <cstdio>
#include <cstring>
#include <map>
#include <memory>
#include <vector>

// POSIX locales (newlocale, nl_langinfo_l, strftime_l) give the system's
// formats when the locale is installed; otherwise, and on Windows, a
// built-in table of short date and time patterns per language is used.
#if (defined(__unix__) || defined(__APPLE__)) && !defined(FO_NO_LOCALE_T)
#include <langinfo.h>
#include <locale.h>
#if defined(__APPLE__)
#include <xlocale.h>
#endif
#define FO_HAVE_LOCALE_T 1
#endif

namespace fo {
namespace i18n {

namespace {

#include "i18n_catalogs.inc"

struct Catalogs {
  std::map<std::string, std::vector<const char *>> sources;
  std::map<std::string, std::string> strings;                    // source -> translation
  std::map<std::string, std::map<std::string, std::string>> plurals; // source -> category -> form
  std::string language = "en";
  std::string forced;
  std::string file;
  bool loaded = false;
  std::unique_ptr<platform::FileWatch> watch;
  std::map<int, std::function<void()>> listeners;
  int next_id = 1;
  // Formatting
  std::string decimal = ".", group = ",";
  bool rtl = false;
  bool system_locales = true;
#ifdef FO_HAVE_LOCALE_T
  locale_t loc = (locale_t)0;
#endif
};

Catalogs &cat() {
  static Catalogs c;
  static bool init = false;
  if (!init) {
    init = true;
    for (const auto &tc : kToolkitCatalogs) {
      c.sources[tc.lang].push_back(tc.json);
      // Omarchy's Spanish is es-MX; plain "es" gets the same strings.
      if (std::string(tc.lang) == "es-MX")
        c.sources["es"].push_back(tc.json);
    }
  }
  return c;
}

std::string base_of(const std::string &code) { return code.substr(0, code.find('-')); }

bool in(const std::string &code, std::initializer_list<const char *> codes) {
  for (const char *c : codes)
    if (code == c)
      return true;
  return false;
}

std::string registry_path() {
  std::string p = platform::env("OMARCHY_PATH");
  if (!p.empty())
    return p + "/default/i18n/locales.json";
  for (std::string dir : {std::string("/usr/share/omarchy"), platform::home_dir() + "/.local/share/omarchy"}) {
    std::string f = dir + "/default/i18n/locales.json";
    if (platform::exists(f))
      return f;
  }
  return "";
}

void load_format(Catalogs &c) {
  const std::string &lang = c.language;
  // Defaults: decimal comma for most of Europe and Latin America.
  bool comma = in(base_of(lang), {"es", "pt", "fr", "de", "it", "nl", "da", "sv", "no", "fi", "pl",
                                  "cs", "sk", "sl", "hr", "sr", "ru", "uk", "ro", "tr", "el", "hu",
                                  "bg", "lt", "lv", "et", "ca", "id", "is", "uz"});
  c.decimal = comma ? "," : ".";
  c.group = comma ? "." : ",";
  if (in(base_of(lang), {"fr", "sv", "no", "fi", "pl", "cs", "sk", "ru", "uk", "bg", "lt", "lv", "et", "hu"}))
    c.group = "\xE2\x80\xAF"; // narrow no-break space
  c.rtl = in(base_of(lang), {"ar", "he", "fa", "ur"});
  std::string locale_name;
  std::string text;
  std::string reg = registry_path();
  json::Value root;
  if (!reg.empty() && platform::read_file(reg, text, 1 << 20) && json::parse(text, root)) {
    if (const json::Value *e = root.get(lang)) {
      locale_name = e->str("locale");
      std::string d = e->str("decimal");
      if (!d.empty())
        c.decimal = d;
      if (e->str("direction") == "rtl")
        c.rtl = true;
    }
  }
#ifdef FO_HAVE_LOCALE_T
  if (c.loc) {
    freelocale(c.loc);
    c.loc = (locale_t)0;
  }
  if (locale_name.empty() && lang == "en")
    locale_name = "en_US.UTF-8";
  if (!locale_name.empty() && c.system_locales)
    c.loc = newlocale(LC_ALL_MASK, locale_name.c_str(), (locale_t)0);
  if (c.loc) {
    const char *dp = nl_langinfo_l(RADIXCHAR, c.loc);
    const char *ts = nl_langinfo_l(THOUSEP, c.loc);
    if (dp && *dp)
      c.decimal = dp;
    if (ts)
      c.group = ts;
  }
#endif
}

void reload() {
  Catalogs &c = cat();
  std::string lang = c.forced;
  if (lang.empty()) {
    std::string text;
    lang = platform::read_file(c.file, text, 4096) ? parse_language(text) : "en";
  }
  if (c.loaded && lang == c.language)
    return;
  c.language = lang;
  c.loaded = true;
  c.strings.clear();
  c.plurals.clear();
  std::vector<std::string> codes{"en"};
  if (base_of(lang) != "en")
    codes.push_back(base_of(lang));
  if (lang != codes.back())
    codes.push_back(lang);
  for (const auto &code : codes) {
    auto it = c.sources.find(code);
    if (it == c.sources.end())
      continue;
    for (const char *src : it->second) {
      json::Value v;
      if (!json::parse(src, v) || !v.is_object())
        continue;
      for (size_t i = 0; i < v.keys.size(); ++i) {
        const json::Value &item = v.items[i];
        if (item.is_string() && !item.string.empty()) {
          c.strings[v.keys[i]] = item.string;
          c.plurals.erase(v.keys[i]);
        } else if (item.is_object()) {
          auto &forms = c.plurals[v.keys[i]];
          forms.clear();
          for (size_t k = 0; k < item.keys.size(); ++k)
            if (item.items[k].is_string())
              forms[item.keys[k]] = item.items[k].string;
          c.strings.erase(v.keys[i]);
        }
      }
    }
  }
  load_format(c);
  auto copy = c.listeners;
  for (auto &kv : copy)
    kv.second();
  for (Fl_Window *w = Fl::first_window(); w; w = Fl::next_window(w))
    w->redraw();
}

void rearm() {
  Catalogs &c = cat();
  if (c.watch)
    c.watch->set_paths({platform::parent(c.file), c.file});
  reload();
}

void ensure_loaded() {
  Catalogs &c = cat();
  if (!c.loaded) {
    if (c.file.empty())
      c.file = platform::config_dir() + "/omarchy/language";
    reload();
  }
}

std::string format_fixed(double v, int decimals, bool grouping) {
  Catalogs &c = cat();
  char buf[64];
  std::snprintf(buf, sizeof buf, "%.*f", decimals < 0 ? 0 : decimals, std::fabs(v));
  std::string s = buf;
  size_t dot = s.find('.');
  std::string ip = s.substr(0, dot), fp = dot == std::string::npos ? "" : s.substr(dot + 1);
  if (grouping && ip.size() > 4) {
    std::string g;
    int n = 0;
    for (size_t i = ip.size(); i-- > 0;) {
      g.insert(0, 1, ip[i]);
      if (++n % 3 == 0 && i > 0)
        g.insert(0, c.group);
    }
    ip = g;
  }
  bool negative = v < 0 && std::fabs(v) >= std::pow(10.0, -decimals) / 2;
  return (negative ? "-" : "") + ip + (fp.empty() ? "" : c.decimal + fp);
}

void local_tm(std::time_t t, std::tm &out) {
#if defined(_WIN32)
  localtime_s(&out, &t);
#else
  localtime_r(&t, &out);
#endif
}

// Short date patterns (CLDR's, simplified) for when no system locale
// answers: D/DD day, M/MM month, YY/YYYY year. `h12`: a 12-hour clock.
struct DatePattern {
  const char *code, *date;
  bool h12;
};
const DatePattern kDatePatterns[] = {
    {"en", "M/D/YY", true},        {"ar", "D/M/YYYY", true},     {"bg", "D.MM.YY", false},
    {"bn", "D/M/YY", true},        {"ca", "D/M/YY", false},      {"cs", "DD.MM.YY", false},
    {"da", "DD.MM.YYYY", false},   {"de", "DD.MM.YY", false},    {"el", "D/M/YY", true},
    {"es", "DD/MM/YY", false},     {"et", "DD.MM.YY", false},    {"fa", "YYYY/M/D", false},
    {"fi", "D.M.YYYY", false},     {"fil", "M/D/YY", true},      {"fr", "DD/MM/YYYY", false},
    {"ga", "DD/MM/YYYY", false},   {"he", "D.M.YYYY", false},    {"hi", "D/M/YY", true},
    {"hr", "DD. MM. YYYY.", false}, {"hu", "YYYY. MM. DD.", false}, {"id", "DD/MM/YY", false},
    {"is", "D.M.YYYY", false},     {"it", "DD/MM/YY", false},    {"ja", "YYYY/MM/DD", false},
    {"ko", "YY. M. D.", true},     {"lt", "YYYY-MM-DD", false},  {"lv", "DD.MM.YY", false},
    {"ms", "D/MM/YY", true},       {"nl", "DD-MM-YYYY", false},  {"no", "DD.MM.YYYY", false},
    {"pl", "DD.MM.YYYY", false},   {"pt", "DD/MM/YY", false},    {"pt-BR", "DD/MM/YYYY", false},
    {"ro", "DD.MM.YYYY", false},   {"ru", "DD.MM.YYYY", false},  {"si", "YYYY-MM-DD", false},
    {"sk", "D. M. YYYY", false},   {"sl", "D. M. YY", false},    {"sr", "D.M.YY.", false},
    {"sv", "YYYY-MM-DD", false},   {"sw", "DD/MM/YYYY", false},  {"ta", "D/M/YY", true},
    {"th", "D/M/YY", false},       {"tr", "DD.MM.YYYY", false},  {"uk", "DD.MM.YY", false},
    {"ur", "D/M/YY", true},        {"uz", "DD/MM/YY", false},    {"vi", "DD/MM/YYYY", false},
    {"zh-CN", "YYYY/M/D", false},  {"zh-TW", "YYYY/M/D", true},
};

const DatePattern &date_pattern(const std::string &lang) {
  static const DatePattern iso{"", "YYYY-MM-DD", false};
  for (const auto &p : kDatePatterns)
    if (lang == p.code)
      return p;
  std::string base = base_of(lang);
  for (const auto &p : kDatePatterns)
    if (base == p.code)
      return p;
  return iso;
}

std::string two(int v) { return (v < 10 ? "0" : "") + std::to_string(v); }

std::string apply_date_pattern(const char *pat, const std::tm &tm) {
  std::string out;
  for (const char *p = pat; *p;) {
    if (!std::strncmp(p, "YYYY", 4)) { out += std::to_string(tm.tm_year + 1900); p += 4; }
    else if (!std::strncmp(p, "YY", 2)) { out += two((tm.tm_year + 1900) % 100); p += 2; }
    else if (!std::strncmp(p, "MM", 2)) { out += two(tm.tm_mon + 1); p += 2; }
    else if (*p == 'M') { out += std::to_string(tm.tm_mon + 1); ++p; }
    else if (!std::strncmp(p, "DD", 2)) { out += two(tm.tm_mday); p += 2; }
    else if (*p == 'D') { out += std::to_string(tm.tm_mday); ++p; }
    else out += *p++;
  }
  return out;
}

} // namespace

std::vector<std::string> toolkit_languages() {
  std::vector<std::string> v;
  for (const auto &tc : kToolkitCatalogs)
    v.push_back(tc.lang);
  return v;
}

void add_catalog(const std::string &lang, const char *json_text) {
  Catalogs &c = cat();
  c.sources[lang].push_back(json_text);
  if (c.loaded) {
    c.loaded = false;
    reload();
  }
}

void use_system_locales(bool on) {
  Catalogs &c = cat();
  if (c.system_locales == on)
    return;
  c.system_locales = on;
  if (c.loaded)
    load_format(c);
}

void watch(const std::string &language_file) {
  Catalogs &c = cat();
  c.file = language_file.empty() ? platform::config_dir() + "/omarchy/language" : language_file;
  if (!c.watch)
    c.watch.reset(new platform::FileWatch(rearm));
  c.loaded = false;
  rearm();
}

void set_language(const std::string &code) {
  Catalogs &c = cat();
  c.forced = code;
  if (c.file.empty())
    c.file = platform::config_dir() + "/omarchy/language";
  c.loaded = false;
  reload();
}

const std::string &language() {
  ensure_loaded();
  return cat().language;
}

bool rtl() {
  ensure_loaded();
  return cat().rtl;
}

std::string parse_language(const std::string &text) {
  std::string line = text.substr(0, text.find('\n'));
  while (!line.empty() && (line.back() == ' ' || line.back() == '\r' || line.back() == '\t'))
    line.pop_back();
  while (!line.empty() && (line[0] == ' ' || line[0] == '\t'))
    line.erase(0, 1);
  // ^[a-z]{2,3}(-[A-Za-z]{2})?$
  size_t i = 0;
  while (i < line.size() && line[i] >= 'a' && line[i] <= 'z')
    ++i;
  if (i < 2 || i > 3)
    return "en";
  if (i == line.size())
    return line;
  if (line[i] != '-' || line.size() != i + 3)
    return "en";
  for (size_t k = i + 1; k < line.size(); ++k) {
    char ch = line[k];
    if (!((ch >= 'a' && ch <= 'z') || (ch >= 'A' && ch <= 'Z')))
      return "en";
  }
  return line;
}

const char *tr(const char *source) {
  if (!source)
    return "";
  ensure_loaded();
  Catalogs &c = cat();
  auto it = c.strings.find(source);
  return it != c.strings.end() ? it->second.c_str() : source;
}

const char *plural_category(const std::string &code, long long count) {
  const long long n = count < 0 ? -count : count, n10 = n % 10, n100 = n % 100;
  const bool millions = n != 0 && n % 1000000 == 0;
  const bool teens = n100 >= 12 && n100 <= 14;
  if (in(code, {"ja", "ko", "zh-CN", "zh-TW", "th", "vi", "id", "ms"})) return "other";
  if (in(code, {"hi", "bn", "fa", "si"})) return n <= 1 ? "one" : "other";
  if (in(code, {"fr", "pt-BR"})) return n <= 1 ? "one" : millions ? "many" : "other";
  if (in(code, {"ca", "es", "es-MX", "it", "pt"})) return n == 1 ? "one" : millions ? "many" : "other";
  if (code == "fil") return (n10 == 4 || n10 == 6 || n10 == 9) ? "other" : "one";
  if (code == "is") return n10 == 1 && n100 != 11 ? "one" : "other";
  if (in(code, {"cs", "sk"})) return n == 1 ? "one" : n >= 2 && n <= 4 ? "few" : "other";
  if (code == "pl") return n == 1 ? "one" : n10 >= 2 && n10 <= 4 && !teens ? "few" : "many";
  if (in(code, {"ru", "uk"})) return n10 == 1 && n100 != 11 ? "one" : n10 >= 2 && n10 <= 4 && !teens ? "few" : "many";
  if (in(code, {"hr", "sr"})) return n10 == 1 && n100 != 11 ? "one" : n10 >= 2 && n10 <= 4 && !teens ? "few" : "other";
  if (code == "sl") return n100 == 1 ? "one" : n100 == 2 ? "two" : n100 >= 3 && n100 <= 4 ? "few" : "other";
  if (code == "lt") return n100 >= 11 && n100 <= 19 ? "other" : n10 == 1 ? "one" : n10 >= 2 ? "few" : "other";
  if (code == "lv") return n10 == 0 || (n100 >= 11 && n100 <= 19) ? "zero" : n10 == 1 ? "one" : "other";
  if (code == "ro") return n == 1 ? "one" : n == 0 || (n100 >= 1 && n100 <= 19) ? "few" : "other";
  if (code == "ar") {
    if (n <= 2) return n == 0 ? "zero" : n == 1 ? "one" : "two";
    return n100 >= 3 && n100 <= 10 ? "few" : n100 >= 11 ? "many" : "other";
  }
  if (code == "he") return n == 1 ? "one" : n == 2 ? "two" : "other";
  if (code == "ga") return n == 1 ? "one" : n == 2 ? "two" : n >= 3 && n <= 6 ? "few" : n >= 7 && n <= 10 ? "many" : "other";
  return n == 1 ? "one" : "other";
}

std::string trn(const char *source, long long n) {
  ensure_loaded();
  Catalogs &c = cat();
  std::string form = source ? source : "";
  auto it = c.plurals.find(form);
  if (it != c.plurals.end()) {
    auto f = it->second.find(plural_category(c.language, n));
    if (f == it->second.end() || f->second.empty())
      f = it->second.find("other");
    if (f != it->second.end())
      form = f->second;
  } else if (auto s = c.strings.find(form); s != c.strings.end()) {
    form = s->second;
  }
  std::string num = format_fixed(double(n), 0, true);
  for (size_t p; (p = form.find("%n")) != std::string::npos;)
    form.replace(p, 2, num);
  return form;
}

std::string arg(const std::string &text, const std::string &value) {
  for (char d = '1'; d <= '9'; ++d) {
    char marker[3] = {'%', d, 0};
    size_t p = text.find(marker);
    if (p != std::string::npos) {
      std::string out = text;
      out.replace(p, 2, value);
      return out;
    }
  }
  return text;
}

int on_change(std::function<void()> fn) {
  Catalogs &c = cat();
  int id = c.next_id++;
  c.listeners[id] = std::move(fn);
  return id;
}

void remove_listener(int id) { cat().listeners.erase(id); }

std::string format_number(double value, int decimals) {
  ensure_loaded();
  return format_fixed(value, decimals, true);
}

std::string format_size(uint64_t bytes) {
  ensure_loaded();
  static const char *units[] = {"B", "KB", "MB", "GB", "TB"};
  double v = double(bytes);
  int i = 0;
  while (v >= 1000 && i < 4) {
    v /= 1000;
    ++i;
  }
  // 999.95 KB would print as "1000.0 KB": move up a unit instead.
  if (i > 0 && i < 4 && v >= 999.95) {
    v /= 1000;
    ++i;
  }
  return format_fixed(v, i == 0 ? 0 : 1, false) + " " + tr(units[i]);
}

#ifdef FO_HAVE_LOCALE_T
static std::string strftime_loc(const char *fmt, const std::tm &tm) {
  char buf[128];
  size_t n = 0;
#ifdef FO_HAVE_LOCALE_T
  Catalogs &c = cat();
  if (c.loc)
    n = strftime_l(buf, sizeof buf, fmt, &tm, c.loc);
  else
#endif
    n = std::strftime(buf, sizeof buf, fmt, &tm);
  return std::string(buf, n);
}
#endif

std::string format_date(std::time_t t) {
  ensure_loaded();
  std::tm tm{};
  local_tm(t, tm);
#ifdef FO_HAVE_LOCALE_T
  if (cat().loc) {
    std::string fmt = nl_langinfo_l(D_FMT, cat().loc);
    for (size_t p; (p = fmt.find("%Y")) != std::string::npos;)
      fmt.replace(p, 2, "%y");
    return strftime_loc(fmt.c_str(), tm);
  }
#endif
  return apply_date_pattern(date_pattern(cat().language).date, tm);
}

std::string format_time(std::time_t t) {
  ensure_loaded();
  std::tm tm{};
  local_tm(t, tm);
  bool h12 = date_pattern(cat().language).h12;
#ifdef FO_HAVE_LOCALE_T
  if (cat().loc) {
    std::string native = nl_langinfo_l(T_FMT, cat().loc);
    h12 = native.find("%r") != std::string::npos || native.find("%p") != std::string::npos ||
          native.find("%I") != std::string::npos || native.find("%l") != std::string::npos;
    if (h12)
      return strftime_loc("%-I:%M %p", tm);
  }
#endif
  if (!h12)
    return two(tm.tm_hour) + ":" + two(tm.tm_min);
  int h = tm.tm_hour % 12;
  return std::to_string(h ? h : 12) + ":" + two(tm.tm_min) + " " + tr(tm.tm_hour < 12 ? "AM" : "PM");
}

std::string format_when(std::time_t t, std::time_t now) {
  if (!now)
    now = std::time(nullptr);
  std::tm a{}, b{};
  local_tm(t, a);
  local_tm(now, b);
  bool today = a.tm_year == b.tm_year && a.tm_yday == b.tm_yday;
  return today ? format_time(t) : format_date(t);
}

} // namespace i18n
} // namespace fo
