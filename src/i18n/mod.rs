//! Translations: o3's flat JSON catalogs, the user's language per OS, live
//! locale switching and plural forms.
//!
//! Every user-visible string goes through here. Keys are the English source
//! strings with `{name}` placeholders; a missing translation falls back to
//! the language (`fr-ca` → `fr`), then English, then the key itself. The
//! toolkit's own strings ship in `i18n/toolkit.json` (20 locales); app
//! catalogs added with [`load`] sit on top and can override any of them.
//!
//! ```no_run
//! use fltk_omarchy::{i18n, tr, trn, widgets::*};
//! i18n::load(r#"{"es": {"Delete {name}?": "¿Eliminar {name}?",
//!                       "{count} files": "{count} archivos", "{count} files|one": "{count} archivo"}}"#).unwrap();
//! i18n::set_locale("es");
//! assert_eq!(tr!("Cancel"), "Cancelar");                         // toolkit catalog
//! assert_eq!(tr!("Delete {name}?", name = "a.txt"), "¿Eliminar a.txt?");
//! assert_eq!(trn!("{count} files", 1), "1 archivo");
//! // Re-labelled live when the locale changes, no rebuild:
//! let open = Button::new("").bind_text(&i18n::text("Open"));
//! let count = fltk_omarchy::State::new(3);
//! let label = Label::new("").bind(&i18n::map(&count, |n| trn!("{count} files", *n)));
//! i18n::set_locale("de");
//! ```
//!
//! The locale and the derived texts are [`State`]s of the UI thread: call
//! [`set_locale`] there. [`tr`] itself works on any thread.

pub mod catalog;
pub mod locale;
pub mod plural;

pub use catalog::{Catalog, CatalogError, Issue, IssueKind, Report};
pub use plural::Plural;

use crate::state::{State, Subscription, WeakState};
use fltk::prelude::*;
use std::cell::RefCell;
use std::fmt::Display;
use std::sync::{OnceLock, RwLock};

/// The toolkit catalog (`i18n/toolkit.json`): the strings the components
/// show by themselves plus common ones (Open, Save, Preferences…).
pub const TOOLKIT_JSON: &str = include_str!("../../i18n/toolkit.json");

/// The 20 locales of the toolkit catalog (o3's set) and their own names,
/// for a language picker.
pub const LANGUAGES: &[(&str, &str)] = &[
    ("en", "English"),
    ("es", "Español"),
    ("pt", "Português"),
    ("fr", "Français"),
    ("de", "Deutsch"),
    ("it", "Italiano"),
    ("nl", "Nederlands"),
    ("pl", "Polski"),
    ("ru", "Русский"),
    ("uk", "Українська"),
    ("ja", "日本語"),
    ("ko", "한국어"),
    ("zh", "中文"),
    ("ar", "العربية"),
    ("tr", "Türkçe"),
    ("sv", "Svenska"),
    ("da", "Dansk"),
    ("no", "Norsk"),
    ("fi", "Suomi"),
    ("cs", "Čeština"),
];

/// Writing direction of the current language (also `layout::Direction`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Direction {
    Ltr,
    Rtl,
}

impl Direction {
    pub fn is_rtl(self) -> bool {
        self == Direction::Rtl
    }
}

struct Shared {
    /// The app catalogs, merged in load order.
    app: Catalog,
    /// Normalized requested list (`fr-ca:en`), never empty.
    requested: String,
    chain: Vec<String>,
    resolved: String,
}

impl Shared {
    fn relocate(&mut self, requested: String) {
        self.chain = Catalog::chain(&requested);
        // The toolkit catalog has exactly the LANGUAGES locales (unit test),
        // so this needs no parsing: direction and language cost nothing
        // until a string is actually translated.
        let app = &self.app;
        self.resolved = Catalog::resolve_in(&requested, |t| {
            LANGUAGES.iter().any(|(l, _)| *l == t) || app.has_locale(t)
        });
        self.requested = requested;
        RTL.store(locale::is_rtl(&self.resolved), std::sync::atomic::Ordering::Relaxed);
    }
}

/// Direction of `Shared::resolved`, read on every draw (no lock).
static RTL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static SHARED: OnceLock<RwLock<Shared>> = OnceLock::new();
static TOOLKIT: OnceLock<Catalog> = OnceLock::new();

/// The toolkit catalog, parsed at the first lookup.
fn toolkit() -> &'static Catalog {
    // Checked by a unit test; an empty catalog keeps English working regardless.
    TOOLKIT.get_or_init(|| Catalog::from_json(TOOLKIT_JSON).unwrap_or_default())
}

fn shared() -> &'static RwLock<Shared> {
    SHARED.get_or_init(|| {
        let mut s = Shared {
            app: Catalog::new(),
            requested: String::new(),
            chain: Vec::new(),
            resolved: String::new(),
        };
        s.relocate(requested_or_en(&locale::system().join(":")));
        RwLock::new(s)
    })
}

fn read<R>(f: impl FnOnce(&Shared) -> R) -> R {
    f(&shared().read().unwrap_or_else(|e| e.into_inner()))
}

fn write<R>(f: impl FnOnce(&mut Shared) -> R) -> R {
    f(&mut shared().write().unwrap_or_else(|e| e.into_inner()))
}

fn requested_or_en(list: &str) -> String {
    let l = locale::parse_list(list).join(":");
    if l.is_empty() {
        "en".into()
    } else {
        l
    }
}

type Refresher = Box<dyn FnMut() -> bool>;

thread_local! {
    static LOCALE: State<String> = {
        let s = State::new(read(|sh| sh.requested.clone()));
        // First listener: catalogs and live texts are current before any
        // app listener runs.
        s.subscribe(|v: &String| {
            write(|sh| sh.relocate(v.clone()));
            refresh();
        }).forever();
        s
    };
    static LIVE: RefCell<Vec<Refresher>> = const { RefCell::new(Vec::new()) };
}

/// Re-translates the live texts and repaints the windows (toolkit strings
/// are translated when drawn).
fn refresh() {
    let mut list = LIVE.with(|l| std::mem::take(&mut *l.borrow_mut()));
    list.retain_mut(|f| f());
    LIVE.with(|l| {
        let mut l = l.borrow_mut();
        let added = std::mem::take(&mut *l);
        *l = list;
        l.extend(added);
    });
    if let Some(wins) = fltk::app::windows() {
        for mut w in wins {
            w.redraw();
        }
    }
}

/// The current locale as a State (normalized list, e.g. `es` or
/// `fr-ca:en`). Subscribe to react to language changes.
pub fn locale() -> State<String> {
    LOCALE.with(|s| s.clone())
}

/// Switches the language (`"es"`, `"pt_BR.UTF-8"`, `"fr-CA:en"`). Bound
/// texts re-translate and windows repaint. UI thread.
pub fn set_locale(tag: &str) {
    FOLLOW.store(false, std::sync::atomic::Ordering::Relaxed);
    let v = requested_or_en(tag);
    LOCALE.with(|s| s.set(v));
}

/// Back to the user's system languages (on Omarchy, the language picked
/// under Setup > Language, followed live).
pub fn follow_system() {
    let v = requested_or_en(&locale::system().join(":"));
    LOCALE.with(|s| s.set(v));
    FOLLOW.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// The system language may have changed (Omarchy's language file): switch
/// to it unless the app chose a language with [`set_locale`]. UI thread.
pub(crate) fn system_changed() {
    refresh_system_language();
}

/// Re-reads the system language (on Omarchy, `~/.config/omarchy/language`)
/// and switches to it unless the app picked a language with
/// [`set_locale`]. The toolkit calls it when Omarchy's files change. UI thread.
pub fn refresh_system_language() {
    if !FOLLOW.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    let v = requested_or_en(&locale::system().join(":"));
    if v != current() {
        LOCALE.with(|s| s.set(v));
    }
}

/// True until the app picks a language itself.
static FOLLOW: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// The OS preferred languages, best first (`["es-cl", "en-us"]`; empty when
/// the system has none, e.g. `LANG=C`).
pub fn system_locales() -> Vec<String> {
    locale::system()
}

/// The requested locale list (same as `locale().get()`, any thread).
pub fn current() -> String {
    read(|s| s.requested.clone())
}

/// The language in use: the first requested locale the catalogs know
/// (`fr-ca`), else the first requested.
pub fn resolved() -> String {
    read(|s| s.resolved.clone())
}

/// Base language in use (`fr` for `fr-ca`, `no` for `nb`).
pub fn language() -> String {
    read(|s| locale::language(&s.resolved).to_string())
}

/// Writing direction of the language in use. Layouts and components
/// mirror with it (see [`crate::layout::direction_of`] for per-container
/// overrides).
pub fn direction() -> Direction {
    if is_rtl() {
        Direction::Rtl
    } else {
        Direction::Ltr
    }
}

/// True when the language in use is written right to left.
pub fn is_rtl() -> bool {
    shared();
    RTL.load(std::sync::atomic::Ordering::Relaxed)
}

/// Adds an app catalog (o3's flat JSON) on top of the toolkit and earlier
/// ones; its strings win.
pub fn load(json: &str) -> Result<(), CatalogError> {
    add(Catalog::from_json(json)?);
    Ok(())
}

/// Adds one language of an Omarchy-style app catalog (`i18n/da.json`:
/// English source → translation, plural forms as objects), as Omarchy's
/// own apps ship them. See [`Catalog::from_omarchy_json`].
pub fn load_omarchy_catalog(lang: &str, json: &str) -> Result<(), CatalogError> {
    add(Catalog::from_omarchy_json(lang, json)?);
    Ok(())
}

/// [`load_omarchy_catalog`] for every `<code>.json` in `dir` (`da.json`,
/// `pt-BR.json`); returns how many languages were loaded.
pub fn load_omarchy_dir(dir: impl AsRef<std::path::Path>) -> Result<usize, CatalogError> {
    let dir = dir.as_ref();
    let rd = std::fs::read_dir(dir).map_err(|e| CatalogError(format!("{}: {e}", dir.display())))?;
    let mut n = 0;
    let mut files: Vec<_> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "json")).collect();
    files.sort();
    for p in files {
        let Some(code) = p.file_stem().and_then(|s| s.to_str()).map(str::to_string) else { continue };
        let s = std::fs::read_to_string(&p).map_err(|e| CatalogError(format!("{}: {e}", p.display())))?;
        load_omarchy_catalog(&code, &s).map_err(|e| CatalogError(format!("{}: {}", p.display(), e.0)))?;
        n += 1;
    }
    Ok(n)
}

/// One of Omarchy's languages, from its registry
/// (`$OMARCHY_PATH/default/i18n/locales.json`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanguageInfo {
    /// Registry code (`da`, `pt-BR`).
    pub code: String,
    /// The language's own name (`Dansk`).
    pub name: String,
    pub english_name: String,
    pub rtl: bool,
    /// Decimal separator (`,`).
    pub decimal: char,
    /// The digit zero the language writes numbers with (`۰` in Persian).
    pub zero_digit: char,
}

/// Omarchy's languages (empty without Omarchy), for a language picker.
pub fn languages() -> Vec<LanguageInfo> {
    let Some(path) = crate::platform::dirs::omarchy_locales_registry() else { return Vec::new() };
    let Ok(text) = std::fs::read_to_string(path) else { return Vec::new() };
    let Ok(serde_json::Value::Object(root)) = serde_json::from_str::<serde_json::Value>(&text) else { return Vec::new() };
    root.into_iter()
        .filter_map(|(code, v)| {
            let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
            let ch = |k: &str, d: char| v.get(k).and_then(|x| x.as_str()).and_then(|x| x.chars().next()).unwrap_or(d);
            Some(LanguageInfo {
                name: s("name"),
                english_name: s("englishName"),
                rtl: s("direction") == "rtl",
                decimal: ch("decimal", '.'),
                zero_digit: ch("zeroDigit", '0'),
                code,
            })
        })
        .collect()
}

/// The registry entry of `code` (`pt-BR`, or its base language `pt`).
pub fn language_info(code: &str) -> Option<LanguageInfo> {
    let want = locale::normalize(code);
    let all = languages();
    all.iter()
        .find(|l| locale::normalize(&l.code) == want)
        .or_else(|| all.iter().find(|l| locale::normalize(&l.code) == locale::language(&want)))
        .cloned()
}

/// `value` with `decimals` decimals in the language in use: its decimal
/// separator and digits from Omarchy's registry (`3,5` in Danish, `۳٫۵` in
/// Persian); `.` and ASCII digits without Omarchy.
pub fn format_number(value: f64, decimals: usize) -> String {
    let info = language_info(&resolved());
    let (dec, zero) = info.map_or(('.', '0'), |i| (i.decimal, i.zero_digit));
    format_number_with(value, decimals, dec, zero)
}

/// [`format_number`] with an explicit decimal separator and zero digit.
pub fn format_number_with(value: f64, decimals: usize, dec: char, zero: char) -> String {
    let s = format!("{value:.decimals$}");
    s.chars()
        .map(|c| match c {
            '.' => dec,
            '0'..='9' => char::from_u32(zero as u32 + (c as u32 - '0' as u32)).unwrap_or(c),
            other => other,
        })
        .collect()
}

/// [`load`] from a file.
pub fn load_file(path: impl AsRef<std::path::Path>) -> Result<(), CatalogError> {
    add(Catalog::from_file(path)?);
    Ok(())
}

pub fn add(catalog: Catalog) {
    write(|s| {
        s.app.merge(&catalog);
        s.relocate(s.requested.clone());
    });
    if LOCALE.try_with(|_| ()).is_ok() {
        refresh();
    }
}

/// Toolkit + app catalogs as merged for lookups (checks, tooling).
pub fn catalog() -> Catalog {
    let mut c = toolkit().clone();
    read(|s| c.merge(&s.app));
    c
}

/// The toolkit catalog alone.
pub fn toolkit_catalog() -> Catalog {
    toolkit().clone()
}

/// Translation of `source` in the current language.
pub fn tr(source: &str) -> String {
    tr_with(source, &[])
}

/// Translation with `{name}` placeholders filled: `tr_with("Delete {name}?",
/// &[("name", &file)])`. See also the [`tr!`](crate::tr) macro.
pub fn tr_with(source: &str, args: &[(&str, &dyn Display)]) -> String {
    render(source, None, args)
}

/// Plural translation: the `source|category` form for `count` in the
/// current language, else `source`. `{count}` is filled with `count`.
pub fn trn(source: &str, count: i64, args: &[(&str, &dyn Display)]) -> String {
    if args.iter().any(|(k, _)| *k == "count") {
        return render(source, Some(count), args);
    }
    let mut all: Vec<(&str, &dyn Display)> = Vec::with_capacity(args.len() + 1);
    all.push(("count", &count));
    all.extend_from_slice(args);
    render(source, Some(count), &all)
}

fn render(source: &str, count: Option<i64>, args: &[(&str, &dyn Display)]) -> String {
    read(
        |s| match Catalog::lookup_in(&[&s.app, toolkit()], &s.chain, source, count) {
            Some((loc, t)) => catalog::format(t, args, locale::is_rtl(loc)),
            None => catalog::format(source, args, false),
        },
    )
}

/// `{name}` substitution without translation.
pub fn format(template: &str, args: &[(&str, &dyn Display)]) -> String {
    catalog::format(template, args, false)
}

/// A State holding `f()`, recomputed whenever the locale (or a catalog)
/// changes: `i18n::live(|| tr!("Hello, {name}", name = user))`.
pub fn live(f: impl Fn() -> String + 'static) -> State<String> {
    LOCALE.with(|_| ());
    let out = State::new(f());
    let weak: WeakState<String> = out.downgrade();
    LIVE.with(|l| {
        l.borrow_mut().push(Box::new(move || match weak.upgrade() {
            Some(s) => {
                s.set(f());
                true
            }
            None => false,
        }))
    });
    out
}

/// The translation of `source` as a live State, for `bind_text`/`bind`.
pub fn text(source: &str) -> State<String> {
    let s = source.to_string();
    live(move || tr(&s))
}

/// A live text derived from another State: recomputed when the locale or
/// `state` changes. `i18n::map(&count, |n| trn!("{count} files", *n))`.
pub fn map<T: Clone + PartialEq + 'static>(
    state: &State<T>,
    f: impl Fn(&T) -> String + 'static,
) -> State<String> {
    let f = std::rc::Rc::new(f);
    let (src, f2) = (state.clone(), f.clone());
    let out = live(move || src.with(|v| f2(v)));
    let weak = out.downgrade();
    state
        .subscribe(move |v| {
            if let Some(o) = weak.upgrade() {
                o.set(f(v));
            }
        })
        .forever();
    out
}

/// Runs `f` now and after every locale change while the widget `w` exists,
/// for components without a bind method:
/// `i18n::bind(&*tabs, move || tabs2.set_tabs(&[&tr("Home"), &tr("Work")].map(|s| s.as_str())))`.
pub fn bind<W: WidgetExt>(w: &W, mut f: impl FnMut() + 'static) {
    crate::state::bind_widget(w, &locale(), true, move |_| f());
}

/// Calls `f` on every locale change (not now).
pub fn on_change(f: impl FnMut(&String) + 'static) -> Subscription {
    locale().subscribe(f)
}

/// Translates with named placeholders: `tr!("Open")`,
/// `tr!("Delete {name}?", name = file)`. Arguments are anything `Display`.
#[macro_export]
macro_rules! tr {
    ($src:expr $(,)?) => {
        $crate::i18n::tr($src)
    };
    ($src:expr, $($name:ident = $val:expr),+ $(,)?) => {
        $crate::i18n::tr_with($src, &[$((stringify!($name), &$val as &dyn ::std::fmt::Display)),+])
    };
}

/// Plural translation: `trn!("{count} files", n)` picks the `|one`,
/// `|few`… form of the current language; `{count}` is `n`. More named
/// arguments may follow: `trn!("{count} files in {dir}", n, dir = path)`.
#[macro_export]
macro_rules! trn {
    ($src:expr, $n:expr $(, $name:ident = $val:expr)* $(,)?) => {
        $crate::i18n::trn($src, ($n) as i64, &[$((stringify!($name), &$val as &dyn ::std::fmt::Display)),*])
    };
}

#[cfg(test)]
mod tests {
    #[test]
    fn omarchy_catalogs_and_number_formats() {
        let c = Catalog::from_omarchy_json("da", r#"{"Save": "Gem", "%n clips": {"one": "%n klip", "other": "%n klip!"}}"#).unwrap();
        let mut probe = Catalog::new();
        probe.merge(&c);
        assert_eq!(probe.get("da", "Save"), Some("Gem"));
        assert_eq!(probe.get("da", "%n clips|one"), Some("%n klip"));
        assert_eq!(probe.get("da", "%n clips"), Some("%n klip!"));
        assert!(Catalog::from_omarchy_json("da", "[1]").is_err());
        assert!(Catalog::from_omarchy_json("da", r#"{"x": 3}"#).is_err());
        assert_eq!(format_number_with(3.5, 1, ',', '0'), "3,5");
        assert_eq!(format_number_with(-12.25, 2, '\u{066b}', '\u{06f0}'), "-\u{06f1}\u{06f2}\u{066b}\u{06f2}\u{06f5}");
        assert_eq!(format_number_with(7.0, 0, '.', '0'), "7");
    }

    use super::*;

    #[test]
    fn toolkit_catalog_is_complete() {
        let c = Catalog::from_json(TOOLKIT_JSON).expect("i18n/toolkit.json parses");
        let locales: Vec<String> = LANGUAGES.iter().map(|(l, _)| l.to_string()).collect();
        let mut have = c.locales();
        have.sort();
        let mut want = locales.clone();
        want.sort();
        assert_eq!(have, want, "the toolkit catalog has o3's 20 locales");
        let r = c.check("en");
        assert!(r.is_clean(), "toolkit catalog issues:\n{r}");
        // English values equal their keys (plural forms aside).
        for k in c.keys("en") {
            if !k.contains('|') {
                assert_eq!(c.get("en", &k), Some(k.as_str()));
            }
        }
    }

    #[test]
    fn toolkit_sources_use_catalog_keys() {
        // Every literal the crate passes to tr/tr!/tr_with/trn/text is a key.
        let c = Catalog::from_json(TOOLKIT_JSON).unwrap();
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut missing = Vec::new();
        let mut found = 0;
        let mut stack = vec![dir];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                    continue;
                }
                // The i18n module itself holds test strings.
                if p.extension().map_or(true, |x| x != "rs")
                    || p.parent().is_some_and(|d| d.ends_with("i18n"))
                {
                    continue;
                }
                let src = std::fs::read_to_string(&p).unwrap();
                for key in catalog::source_keys(&src) {
                    found += 1;
                    if c.get("en", &key).is_none() {
                        missing.push(format!("{}: {key}", p.display()));
                    }
                }
            }
        }
        assert!(found >= 8, "found only {found} toolkit strings");
        assert!(
            missing.is_empty(),
            "strings not in i18n/toolkit.json: {missing:#?}"
        );
    }

    #[test]
    fn global_lookup_switches_language() {
        // The only unit test touching the process-wide locale.
        add(Catalog::from_json(r#"{"es":{"Cancel":"Anular","{count} apps":"{count} apps","{count} apps|one":"{count} app"},"en":{"{count} apps":"{count} apps","{count} apps|one":"{count} app"}}"#).unwrap());
        set_locale("es_CL.UTF-8");
        assert_eq!(current(), "es-cl");
        assert_eq!(language(), "es");
        assert_eq!(tr("Cancel"), "Anular", "app catalog overrides the toolkit");
        assert_eq!(tr("Open"), "Abrir", "toolkit string");
        assert_eq!(
            tr!("No matches for “{query}”", query = "x"),
            "Sin coincidencias para «x»"
        );
        assert_eq!(trn!("{count} apps", 1), "1 app");
        assert_eq!(trn!("{count} items", 2), "2 elementos");
        let open = text("Open");
        let n = State::new(1);
        let items = map(&n, |n| trn!("{count} items", *n));
        assert_eq!(items.get(), "1 elemento");
        set_locale("ar");
        assert_eq!(direction(), Direction::Rtl);
        assert_eq!(open.get(), "فتح");
        n.set(2);
        assert_eq!(items.get(), "عنصران");
        n.set(5);
        assert_eq!(items.get(), "\u{2068}5\u{2069} عناصر");
        set_locale("zz");
        assert_eq!(tr("Open"), "Open");
        assert_eq!(direction(), Direction::Ltr);
        set_locale("");
        assert_eq!(current(), "en");
        drop(open);
        drop(items);
        set_locale("de");
        assert!(
            LIVE.with(|l| l.borrow().is_empty()),
            "dropped live texts are pruned"
        );
        set_locale("en");
    }
}
