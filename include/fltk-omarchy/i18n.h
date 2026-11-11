// Translations and locale formatting, following Omarchy's language.
//
// The language is the code on the first line of ~/.config/omarchy/language
// (what `omarchy-language` writes), followed live; anything invalid, or no
// file, is English. Catalogs are embedded JSON objects mapping the English
// source to a translation, or to plural forms keyed by CLDR category:
//
//   { "Cancel": "Cancelar",
//     "%n items": { "one": "%n elemento", "other": "%n elementos" } }
//
// Lookups layer English, the base language and the regional one (pt, then
// pt-BR), so a missing string falls back one step at a time. The toolkit
// registers catalogs for its own strings; apps add theirs with
// add_catalog() before watch().
//
//   fo::i18n::add_catalog("es", kMyAppSpanish);
//   fo::i18n::watch();
//   label(fo::tr("Open"));
//   std::string s = fo::trn("%n items", count);
#pragma once

#include <cstdint>
#include <ctime>
#include <functional>
#include <string>

namespace fo {
namespace i18n {

/// Registers (merges) an embedded catalog for `lang` ("es", "pt-BR").
/// The JSON text must outlive nothing: it is parsed at the next reload.
void add_catalog(const std::string &lang, const char *json_text);

/// Follows the Omarchy language file (default ~/.config/omarchy/language).
void watch(const std::string &language_file = "");
/// Forces a language (tests, settings); "" goes back to the file.
void set_language(const std::string &code);
/// Current language code ("en" by default).
const std::string &language();
/// True for right-to-left languages (ar, he, fa, ur, and any language the
/// locale registry marks "rtl").
bool rtl();

/// The first line of a language file if it is a valid code, else "en".
std::string parse_language(const std::string &file_text);

/// Translation of `source`, or `source` itself. The pointer stays valid
/// until the language changes: translate at draw time, do not store it.
const char *tr(const char *source);
/// Plural form for `n` with "%n" replaced by the formatted number.
std::string trn(const char *source, long long n);
/// Replaces the lowest-numbered "%1".."%9" marker with `value` (Qt's arg()).
std::string arg(const std::string &text, const std::string &value);

/// CLDR cardinal plural category ("zero", "one", "two", "few", "many",
/// "other") of whole number `n` in `lang` (Omarchy's languages).
const char *plural_category(const std::string &lang, long long n);

/// Called on the FLTK thread after the language changed.
int on_change(std::function<void()> fn);
void remove_listener(int id);

// --- Locale formatting (Omarchy's locale registry when present) ---

/// "1,234.5" / "1.234,5" for the current language.
std::string format_number(double value, int decimals);
/// File sizes with decimal units, as Folio: "980 B", "31.6 KB", "4.2 GB".
std::string format_size(uint64_t bytes);
/// Short date ("11/10/26") and short time ("6:10 PM" / "18:10").
std::string format_date(std::time_t t);
std::string format_time(std::time_t t);
/// Folio's "when": the time for today, the date before that.
std::string format_when(std::time_t t, std::time_t now = 0);

} // namespace i18n

/// Shorthands.
inline const char *tr(const char *source) { return i18n::tr(source); }
/// True while the language reads right to left: the toolkit's widgets
/// mirror then (Sidebar, Breadcrumbs, Collection, menus, SplitPane,
/// ColumnView).
inline bool rtl() { return i18n::rtl(); }
inline std::string trn(const char *source, long long n) { return i18n::trn(source, n); }

} // namespace fo
