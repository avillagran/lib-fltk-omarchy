#include "harness.h"

#include <ctime>

#include "fltk-omarchy/anim.h"
#include "fltk-omarchy/hyprland.h"
#include "fltk-omarchy/i18n.h"
#include "fltk-omarchy/icons.h"
#include "fltk-omarchy/json.h"
#include "fltk-omarchy/keys.h"
#include "fltk-omarchy/platform.h"
#include "fltk-omarchy/scroll.h"
#include "fltk-omarchy/text.h"
#include "fltk-omarchy/theme.h"

#include <FL/Enumerations.H>
#include <sys/stat.h>

using namespace fo;

// --- JSON -------------------------------------------------------------------

TEST(json_parses_objects_arrays_and_escapes) {
  json::Value v;
  CHECK(json::parse(R"({"a": [1, 2.5, -3e2], "s": "x\"é😀", "t": true, "n": null})", v));
  CHECK(v.is_object());
  const json::Value *a = v.get("a");
  CHECK(a && a->is_array() && a->items.size() == 3);
  CHECK_NEAR(a->items[2].number, -300, 1e-9);
  CHECK_STR(v.str("s"), "x\"\xC3\xA9\xF0\x9F\x98\x80");
  CHECK(v.flag("t"));
  CHECK(v.get("n")->is_null());
  CHECK(!v.get("missing"));
}

TEST(json_rejects_garbage) {
  json::Value v;
  CHECK(!json::parse("{", v));
  CHECK(!json::parse("{\"a\":1,}", v));
  CHECK(!json::parse("[1 2]", v));
  CHECK(!json::parse("\"unterminated", v));
  CHECK(!json::parse("{} trailing", v));
  CHECK(v.is_null());
}

// --- Theme ------------------------------------------------------------------

TEST(colors_toml_subset) {
  Palette p = parse_colors_toml("mode = \"light\"\n# comment\naccent = \"#7aa2f7\"\n  background='#1A1B26' # x\n"
                                "bad = \"#12345\"\nname = \"text\"\n");
  CHECK(p.light);
  CHECK_EQ(p.colors.size(), size_t(2));
  CHECK(p.colors["accent"] == Rgb(0x7a, 0xa2, 0xf7));
  CHECK(p.colors["background"] == Rgb(0x1a, 0x1b, 0x26));
}

TEST(theme_fallbacks_follow_folio) {
  Theme empty;
  CHECK(!empty.from_omarchy());
  CHECK(empty.accent() == Rgb(0xFF, 0xD6, 0x0A));
  CHECK(empty.color("no_such_key") == Rgb(0x88, 0x88, 0x88));

  Palette p;
  p.colors["background"] = Rgb(10, 20, 30);
  p.colors["foreground"] = Rgb(200, 200, 200);
  Theme t(p);
  // Derived backgrounds fall back to the background, foregrounds to foreground.
  CHECK(t.dark_background() == Rgb(10, 20, 30));
  CHECK(t.bright_foreground() == Rgb(200, 200, 200));
  // lighter_background equal to background: nudged toward the foreground.
  CHECK(t.lighter_background() == mix(Rgb(10, 20, 30), Rgb(200, 200, 200), 0.08));
  CHECK(t.secondary_foreground() == mix(Rgb(10, 20, 30), Rgb(200, 200, 200), 0.68));
  // Missing palette colors come from the fallback set.
  CHECK(t.red() == Theme::fallback_colors().at("red"));
}

TEST(theme_accent_foreground_and_radius) {
  Palette p;
  p.colors["accent"] = Rgb(0xFF, 0xD6, 0x0A);
  CHECK(Theme(p).accent_foreground() == Rgb(0, 0, 0));
  p.colors["accent"] = Rgb(0x20, 0x20, 0x80);
  Theme t(p);
  CHECK(t.accent_foreground() == Rgb(255, 255, 255));
  t.set_radius(10);
  CHECK_EQ(t.radius_small(), 4);
  t.set_radius(-3);
  CHECK_EQ(t.radius(), 0);
}

TEST(theme_loads_from_fixture_dir) {
  std::string dir = fo_test::private_dir("theme");
  mkdir((dir + "/theme").c_str(), 0700);
  fo_test::write_file(dir + "/theme/colors.toml", "accent = \"#112233\"\nlighter_background = \"#445566\"\n");
  Theme t = Theme::load(dir);
  CHECK(t.from_omarchy());
  CHECK(t.accent() == Rgb(0x11, 0x22, 0x33));
  CHECK(t.lighter_background() == Rgb(0x44, 0x55, 0x66));
  Theme missing = Theme::load(dir + "/nothing-here");
  CHECK(!missing.from_omarchy());
}

TEST(hex_and_mix) {
  Rgb c;
  CHECK(parse_hex("#a0B1c2", c) && c == Rgb(0xa0, 0xb1, 0xc2));
  CHECK(!parse_hex("#a0B1c", c));
  CHECK(!parse_hex("#zzzzzz", c));
  CHECK_STR(to_hex(Rgb(1, 2, 255)), "#0102ff");
  CHECK(mix(Rgb(0, 0, 0), Rgb(200, 100, 50), 0.5) == Rgb(100, 50, 25));
  CHECK_NEAR(luminance(Rgb(255, 255, 255)), 1.0, 1e-9);
}

// --- Animation --------------------------------------------------------------

TEST(easing_curves) {
  using anim::Ease;
  for (Ease e : {Ease::Linear, Ease::OutCubic, Ease::InCubic, Ease::InOutCubic, Ease::OutQuint, Ease::EaseOut}) {
    CHECK_NEAR(anim::ease(e, 0), 0, 1e-6);
    CHECK_NEAR(anim::ease(e, 1), 1, 1e-6);
  }
  CHECK_NEAR(anim::ease(Ease::OutCubic, 0.5), 0.875, 1e-9);
  CHECK_NEAR(anim::ease(Ease::InOutCubic, 0.5), 0.5, 1e-9);
  CHECK(anim::ease(Ease::OutBack, 0.8) > 1.0); // overshoots
  // Omarchy's easeOut starts fast: well past half at a quarter of the time.
  CHECK(anim::ease(Ease::EaseOut, 0.25) > 0.6);
  CHECK_NEAR(anim::bezier(0.25, 0.25, 0.75, 0.75, 0.3), 0.3, 1e-4); // linear control points
  CHECK_NEAR(anim::ease(Ease::Linear, 2.0), 1, 1e-9);                 // clamped
}

TEST(anim_retargets_from_what_is_on_screen) {
  anim::set_enabled(1);
  anim::freeze();
  Anim<double> a;
  a.set(0, 100);
  CHECK(!a.running()); // first value jumps
  a.set(100, 100, anim::Ease::Linear);
  CHECK(a.running());
  anim::advance(50);
  CHECK_NEAR(a.value(), 50, 1e-6);
  a.set(0, 100, anim::Ease::Linear); // retarget mid-way: starts at 50
  CHECK_NEAR(a.value(), 50, 1e-6);
  anim::advance(50);
  CHECK_NEAR(a.value(), 25, 1e-6);
  anim::advance(60);
  CHECK_NEAR(a.value(), 0, 1e-6);
  CHECK(!a.running());

  Anim<Rgb> c(Rgb(0, 0, 0));
  c.set(Rgb(200, 100, 0), 100, anim::Ease::Linear);
  anim::advance(50);
  CHECK(c.value() == Rgb(100, 50, 0));

  Anim<RectF> r(RectF{0, 0, 10, 10});
  r.set(RectF{100, 0, 10, 20}, 100, anim::Ease::Linear);
  anim::advance(25);
  RectF v = r.value();
  CHECK_NEAR(v.x, 25, 1e-6);
  CHECK_NEAR(v.h, 12.5, 1e-6);

  anim::set_enabled(0); // off: everything jumps
  Anim<double> off(1);
  off.set(5, 100);
  CHECK(!off.running());
  CHECK_NEAR(off.value(), 5, 1e-9);
  anim::set_enabled(-1);
  anim::resume();
}

TEST(anim_switch_parsing) {
  CHECK_EQ(anim::parse_switch("off"), 0);
  CHECK_EQ(anim::parse_switch("FALSE"), 0);
  CHECK_EQ(anim::parse_switch("0"), 0);
  CHECK_EQ(anim::parse_switch("On"), 1);
  CHECK_EQ(anim::parse_switch("1"), 1);
  CHECK_EQ(anim::parse_switch("maybe"), -1);
  CHECK_EQ(anim::parse_switch(nullptr), -1);
}

TEST(scroller_glides_and_reverses) {
  anim::set_enabled(1);
  anim::freeze();
  Scroller s;
  s.wheel(100, 1000);
  CHECK(s.running());
  CHECK_NEAR(s.target(), 100, 1e-9);
  anim::advance(60);
  double mid = s.position(1000);
  CHECK(mid > 50 && mid < 100);
  s.wheel(100, 1000); // same direction accumulates
  CHECK_NEAR(s.target(), 200, 1e-9);
  s.wheel(-10, 1000); // reversal drops pending travel
  CHECK_NEAR(s.target(), s.position(1000) - 10, 1e-6);
  anim::advance(200);
  s.position(1000);
  CHECK(!s.running());
  s.wheel(5000, 1000); // clamped at max
  anim::advance(200);
  CHECK_NEAR(s.position(1000), 1000, 1e-9);
  CHECK(s.passes(10, 1000));
  CHECK(!s.passes(-10, 1000));
  anim::set_enabled(-1);
  anim::resume();
}

TEST(hyprland_reply_parsers) {
  int r = -1;
  CHECK(hypr::parse_option_int(R"({"option":"decoration:rounding","int":8,"set":true})", r) && r == 8);
  bool on = false;
  CHECK(hypr::parse_option_bool(R"({"option":"animations:enabled","bool":true})", on) && on);
  CHECK(hypr::parse_option_bool(R"({"int":0})", on) && !on);
  CHECK(!hypr::parse_option_int("garbage", r));
  CHECK_NEAR(hypr::parse_refresh_rate(R"([{"id":0,"refreshRate":60.0,"focused":false},
                                          {"id":1,"refreshRate":143.98,"focused":true}])"),
             143.98, 1e-9);
  CHECK_NEAR(hypr::parse_refresh_rate(R"([{"id":0,"refreshRate":75}])"), 75, 1e-9);
  CHECK_NEAR(hypr::parse_refresh_rate("[]"), 0, 1e-9);
  CHECK(!hypr::available()); // the test env has no Hyprland
}

// --- Text -------------------------------------------------------------------

// Every code point is 1 unit wide.
static double units(const char *t, int n) {
  double w = 0;
  for (int i = 0; i < n; ++i)
    if ((static_cast<unsigned char>(t[i]) & 0xC0) != 0x80)
      w += 1;
  return w;
}

TEST(elide_end_and_middle) {
  text::Measure m = units;
  CHECK_STR(text::elide_end("short", 10, m), "short");
  CHECK_STR(text::elide_end("abcdefghij", 6, m), "abcde\xE2\x80\xA6");
  CHECK_STR(text::elide_end("ab cdefgh", 4, m), "ab\xE2\x80\xA6"); // trailing space trimmed
  // Middle keeps the extension.
  std::string mid = text::elide_middle("holiday-photo-2026-final.jpeg", 16, m);
  CHECK(units(mid.c_str(), int(mid.size())) <= 16);
  CHECK(mid.find("\xE2\x80\xA6") != std::string::npos);
  CHECK(mid.size() > 5 && mid.substr(mid.size() - 5) == ".jpeg");
  CHECK(mid.compare(0, 4, "holi") == 0);
  // UTF-8 never split.
  std::string u = text::elide_middle("\xC3\xA9\xC3\xA9\xC3\xA9\xC3\xA9\xC3\xA9\xC3\xA9\xC3\xA9\xC3\xA9.txt", 8, m);
  CHECK(units(u.c_str(), int(u.size())) <= 8);
  CHECK(text::utf8_offsets(u).size() == 9);
  // Too narrow for anything: just the ellipsis.
  CHECK_STR(text::elide_end("abc", 1, m), "\xE2\x80\xA6");
}

TEST(wrap_two_lines) {
  text::Measure m = units;
  auto lines = text::wrap("Sintra day trip photos", 10, m, 2);
  CHECK_EQ(lines.size(), size_t(2));
  CHECK_STR(lines[0], "Sintra day");
  CHECK(units(lines[1].c_str(), int(lines[1].size())) <= 10);
  auto one = text::wrap("tiny", 10, m, 2);
  CHECK_EQ(one.size(), size_t(1));
  CHECK_STR(text::upper("Home \xC3\xA9"), "HOME \xC3\x89");
}

// --- Keys -------------------------------------------------------------------

TEST(key_specs) {
  Key k = parse_key("Ctrl+Shift+N");
  CHECK(k.key == 'n' && k.mods == (FL_CTRL | FL_SHIFT));
  CHECK_STR(key_label(k), "Ctrl+Shift+N");
  CHECK(parse_key("Alt+Left").key == FL_Left);
  CHECK_STR(key_label(parse_key("alt+left")), "Alt+\xE2\x86\x90");
  CHECK(parse_key("F2").key == FL_F + 2);
  CHECK(parse_key("Ctrl++").key == '+');
  CHECK(!parse_key("Hyper+X").valid());
  CHECK(!parse_key("").valid());
  CHECK(parse_key("Mod+,").mods == FL_CTRL);

  CHECK(key_matches(parse_key("Ctrl+L"), 'l', FL_CTRL, "\x0c"));
  CHECK(key_matches(parse_key("Ctrl+L"), 'L', FL_CTRL | FL_NUM_LOCK, ""));
  CHECK(!key_matches(parse_key("Ctrl+L"), 'l', FL_CTRL | FL_SHIFT, ""));
  CHECK(key_matches(parse_key("?"), '/', FL_SHIFT, "?"));  // symbol as typed
  CHECK(!key_matches(parse_key("?"), '/', 0, "/"));
  CHECK(key_matches(parse_key("Enter"), FL_KP_Enter, 0, "\r"));
  CHECK(key_matches(parse_key("Delete"), FL_Delete, 0, ""));
}

// --- i18n -------------------------------------------------------------------

TEST(plural_categories) {
  CHECK_STR(i18n::plural_category("en", 1), "one");
  CHECK_STR(i18n::plural_category("en", 0), "other");
  CHECK_STR(i18n::plural_category("fr", 0), "one");
  CHECK_STR(i18n::plural_category("fr", 1000000), "many");
  CHECK_STR(i18n::plural_category("es-MX", 1000000), "many");
  CHECK_STR(i18n::plural_category("pl", 3), "few");
  CHECK_STR(i18n::plural_category("pl", 13), "many");
  CHECK_STR(i18n::plural_category("pl", 22), "few");
  CHECK_STR(i18n::plural_category("ru", 21), "one");
  CHECK_STR(i18n::plural_category("ru", 11), "many");
  CHECK_STR(i18n::plural_category("ar", 0), "zero");
  CHECK_STR(i18n::plural_category("ar", 2), "two");
  CHECK_STR(i18n::plural_category("ar", 105), "few");
  CHECK_STR(i18n::plural_category("ar", 111), "many");
  CHECK_STR(i18n::plural_category("ja", 1), "other");
  CHECK_STR(i18n::plural_category("sl", 102), "two");
  CHECK_STR(i18n::plural_category("lv", 10), "zero");
  CHECK_STR(i18n::plural_category("ga", 7), "many");
  CHECK_STR(i18n::plural_category("cs", 4), "few");
}

TEST(language_file_parsing) {
  CHECK_STR(i18n::parse_language("es-MX\n"), "es-MX");
  CHECK_STR(i18n::parse_language("  pt-BR  \nother"), "pt-BR");
  CHECK_STR(i18n::parse_language("fil"), "fil");
  CHECK_STR(i18n::parse_language("../../etc"), "en");
  CHECK_STR(i18n::parse_language("english"), "en");
  CHECK_STR(i18n::parse_language(""), "en");
  CHECK_STR(i18n::parse_language("es-MXX"), "en");
}

TEST(translations_layer_and_pluralize) {
  // No registry: formatting uses the built-in separators.
  std::string reg = fo_test::private_dir("omarchy");
  setenv("OMARCHY_PATH", reg.c_str(), 1);
  i18n::add_catalog("pt", R"({"Hello": "Olá", "Bye": "Tchau", "%n files": {"one": "%n arquivo", "other": "%n arquivos"}})");
  i18n::add_catalog("pt-BR", R"({"Bye": "Falou"})");
  i18n::set_language("pt-BR");
  CHECK_STR(i18n::language(), "pt-BR");
  CHECK_STR(tr("Hello"), "Olá");     // from the base language
  CHECK_STR(tr("Bye"), "Falou");     // regional wins
  CHECK_STR(tr("Untranslated"), "Untranslated");
  CHECK_STR(tr("Cancel"), "Cancelar"); // toolkit catalog
  CHECK_STR(trn("%n files", 0), "0 arquivo"); // pt-BR: 0 is "one"
  CHECK_STR(trn("%n files", 2), "2 arquivos");
  CHECK_STR(trn("%n files", 12345), "12.345 arquivos");
  CHECK_STR(i18n::format_size(31600), "31,6 KB");
  i18n::set_language("en");
  CHECK_STR(tr("Hello"), "Hello");
  CHECK_STR(trn("%n items", 1), "1 item");
  CHECK_STR(i18n::format_size(999), "999 B");
  CHECK_STR(i18n::format_size(31600), "31.6 KB");
  CHECK_STR(i18n::format_size(999960), "1.0 MB");
  CHECK_STR(i18n::format_size(4200000000ull), "4.2 GB");
  CHECK_STR(i18n::arg("Search %1 and below", "Pictures"), "Search Pictures and below");
  CHECK(!i18n::rtl());
  unsetenv("OMARCHY_PATH");
}

TEST(language_file_is_read) {
  std::string cfg = fo_test::private_dir("lang");
  fo_test::write_file(cfg + "/language", "fr\n");
  i18n::set_language("");
  i18n::watch(cfg + "/language");
  CHECK_STR(i18n::language(), "fr");
  CHECK_STR(tr("Close"), "Fermer");
  i18n::set_language("en");
}

// --- Icons and platform -----------------------------------------------------

TEST(icons_resolve_and_build_svg) {
  CHECK(icon::exists("folder"));
  CHECK(icon::exists("home"));
  CHECK(icon::exists("eject"));
  CHECK(!icon::exists("no-such-icon"));
  CHECK_STR(icon::resolve("trash"), "trash-2");
  auto names = icon::names();
  CHECK(names.size() > 90);
  for (size_t i = 1; i < names.size(); ++i)
    CHECK(names[i - 1] < names[i]); // sorted for the binary search
  std::string s = icon::svg("folder", Rgb(255, 0, 0), 1.75, true);
  CHECK(s.find("stroke=\"#ff0000\"") != std::string::npos);
  CHECK(s.find("stroke-linecap=\"square\"") != std::string::npos);
  CHECK(s.find("<path") != std::string::npos);
  CHECK(icon::svg("nope", Rgb(), 2, false) == icon::svg("file", Rgb(), 2, false));
}

TEST(platform_paths) {
  CHECK_STR(platform::parent("/a/b/c"), "/a/b");
  CHECK_STR(platform::parent("/a"), "/");
  CHECK_STR(platform::parent("a"), ".");
  CHECK_STR(platform::parent("/a/b/"), "/a");
  // The runner moved HOME into the private run directory.
  CHECK(platform::home_dir().find("/home") != std::string::npos);
  CHECK(platform::config_dir().find("/config") != std::string::npos);
}

namespace {
void watch_case(platform::FileWatch::Backend backend) {
  std::string dir = fo_test::private_dir("watch");
  std::string file = dir + "/colors.toml";
  fo_test::write_file(file, "accent = \"#111111\"\n");
  int fired = 0;
  bool done = false;
  platform::FileWatch w([&] { ++fired, done = true; }, 0.02, backend);
  CHECK(w.active());
  w.poll_interval(0.05);
  w.set_paths({dir, file});
  // A new entry in the directory.
  fo_test::write_file(dir + "/new.txt", "x");
  CHECK(fo_test::pump_until(done, 3));
  // A file rewritten in place (same name, new size).
  done = false;
  fo_test::write_file(file, "accent = \"#222222\" # changed\n");
  CHECK(fo_test::pump_until(done, 3));
  // Nothing changes: nothing fires.
  done = false;
  int before = fired;
  fo_test::pump_until(done, 0.3);
  CHECK_EQ(fired, before);
}
} // namespace

TEST(file_watch_notifies) {
  platform::FileWatch probe(nullptr);
  std::printf("  backend: %s\n", probe.backend());
  CHECK(probe.active());
  watch_case(platform::FileWatch::Backend::Auto);
}

TEST(file_watch_polling) {
  platform::FileWatch probe(nullptr, 0.08, platform::FileWatch::Backend::Polling);
  CHECK_STR(probe.backend(), "polling");
  watch_case(platform::FileWatch::Backend::Polling);
}

TEST(i18n_date_and_time_fallback_patterns) {
  // The built-in patterns used without a system locale (and on Windows).
  i18n::use_system_locales(false);
  std::tm tm{};
  tm.tm_year = 2026 - 1900;
  tm.tm_mon = 10; // November
  tm.tm_mday = 3;
  tm.tm_hour = 18;
  tm.tm_min = 5;
  tm.tm_isdst = -1;
  std::time_t t = std::mktime(&tm);
  i18n::set_language("en");
  CHECK_STR(i18n::format_date(t), "11/3/26");
  CHECK_STR(i18n::format_time(t), "6:05 PM");
  i18n::set_language("de");
  CHECK_STR(i18n::format_date(t), "03.11.26");
  CHECK_STR(i18n::format_time(t), "18:05");
  i18n::set_language("ja");
  CHECK_STR(i18n::format_date(t), "2026/11/03");
  i18n::set_language("pt-BR");
  CHECK_STR(i18n::format_date(t), "03/11/2026");
  i18n::set_language("es-MX"); // the base language's pattern
  CHECK_STR(i18n::format_date(t), "03/11/26");
  i18n::set_language("xx");
  CHECK_STR(i18n::format_date(t), "2026-11-03");
  i18n::set_language("");
  i18n::use_system_locales(true);
}

namespace {
#include "i18n_catalogs.inc"
} // namespace

TEST(toolkit_catalogs_cover_omarchy_languages) {
  // Omarchy's locale registry (default/i18n/locales.json), 50 languages.
  const char *omarchy[] = {"en", "da", "ar", "fi", "fr", "el", "hu", "hi", "is", "ja", "ko", "es-MX", "fil",
                           "pt", "sv", "tr", "vi", "ur", "bn", "ca", "si", "ta", "th", "uz", "it", "zh-CN",
                           "pl", "lt", "ga", "nl", "no", "ru", "id", "de", "sw", "fa", "ms", "uk", "ro", "cs",
                           "sr", "he", "bg", "sk", "hr", "sl", "zh-TW", "et", "lv", "pt-BR"};
  std::vector<std::string> have = i18n::toolkit_languages();
  CHECK_EQ(have.size(), sizeof omarchy / sizeof *omarchy);
  json::Value en;
  CHECK(json::parse(kToolkitCatalogs[0].json, en));
  CHECK_STR(kToolkitCatalogs[0].lang, "en");
  for (const char *code : omarchy) {
    const ToolkitCatalog *tc = nullptr;
    for (const auto &c : kToolkitCatalogs)
      if (std::string(c.lang) == code)
        tc = &c;
    if (!tc) {
      fo_test::fail(__FILE__, __LINE__, std::string("no catalog for ") + code);
      continue;
    }
    json::Value v;
    if (!json::parse(tc->json, v) || !v.is_object()) {
      fo_test::fail(__FILE__, __LINE__, std::string("catalog does not parse: ") + code);
      continue;
    }
    // Every string English has, translated and not empty.
    for (size_t i = 0; i < en.keys.size(); ++i) {
      const json::Value *t = v.get(en.keys[i]);
      bool ok = t && ((t->is_string() && !t->string.empty()) || t->is_object());
      if (!ok)
        fo_test::fail(__FILE__, __LINE__, std::string(code) + " lacks \"" + en.keys[i] + "\"");
    }
    // Every plural category the language uses has its own form.
    const json::Value *items = v.get("%n items");
    std::vector<long long> ns;
    for (long long n = 0; n <= 230; ++n)
      ns.push_back(n);
    ns.push_back(1000000);
    for (long long n : ns) {
      std::string cat = i18n::plural_category(code, n);
      const json::Value *form = items ? items->get(cat) : nullptr;
      if (!form || !form->is_string() || form->string.empty()) {
        fo_test::fail(__FILE__, __LINE__, std::string(code) + " lacks the \"" + cat + "\" form");
        break;
      }
    }
    // And through the real lookup.
    i18n::set_language(code);
    CHECK(std::string(tr("Cancel")).size() > 0);
    std::string five = trn("%n items", 5);
    CHECK(five.find("%n") == std::string::npos);
    CHECK(five.find('5') != std::string::npos);
  }
  i18n::set_language("pt");
  CHECK_STR(tr("Delete"), "Eliminar"); // European Portuguese
  i18n::set_language("pt-BR");
  CHECK_STR(tr("Delete"), "Excluir");
  i18n::set_language("es");
  CHECK_STR(tr("Cancel"), "Cancelar"); // Omarchy's es-MX strings
  i18n::set_language("ru");
  CHECK_STR(trn("%n items", 3), "3 элемента");
  i18n::set_language("");
}
