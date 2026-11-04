//! Locale tags and the user's preferred languages per OS.
//!
//! Self-contained (std only, no crate imports) so it can move to a platform
//! layer later. Every function falls back instead of failing: no panics, an
//! empty list means "no preference" (callers use English).
//!
//! - Linux/BSD and other Unix: gettext's order. `LC_ALL`, else
//!   `LC_MESSAGES`, else `LANG` is the locale; `C`/`POSIX` there means no
//!   translation. Otherwise `LANGUAGE` (a `:` list) comes first.
//! - macOS: an explicit `LC_ALL`/`LC_MESSAGES`/`LANGUAGE` wins (terminals set
//!   `LANG` to whatever, so `LANG` alone does not); then CoreFoundation's
//!   `CFLocaleCopyPreferredLanguages` (System Settings > Language); then `LANG`.
//! - Windows: an explicit `LC_ALL`/`LC_MESSAGES`/`LANGUAGE`/`LANG` wins; then
//!   `GetUserPreferredUILanguages`, then `GetUserDefaultLocaleName`.

/// Normalized tag: `fr_CA.UTF-8@euro` → `fr-ca`, `zh-Hans-CN` → `zh-hans-cn`.
/// `C`, `POSIX` and empty give `""`. Only the first entry of a `:` list.
pub fn normalize(tag: &str) -> String {
    let first = tag.split(':').next().unwrap_or("");
    let t = first
        .split(['.', '@'])
        .next()
        .unwrap_or("")
        .trim()
        .replace('_', "-")
        .to_lowercase();
    match t.as_str() {
        "c" | "posix" => String::new(),
        _ => t,
    }
}

/// Language of a normalized tag, with Norwegian Bokmål/Nynorsk as `no`
/// (the catalogs use `no`) and the old Hebrew code `iw` as `he`.
pub fn language(tag: &str) -> &str {
    let b = tag.split('-').next().unwrap_or("");
    match b {
        "nb" | "nn" => "no",
        "iw" => "he",
        _ => b,
    }
}

/// A `:` or `,` separated list (`LANGUAGE`, `fr_CA.UTF-8:en`), normalized,
/// without empty, `C`/`POSIX` or repeated entries.
pub fn parse_list(list: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for part in list.split([':', ',']) {
        let n = normalize(part.trim());
        if !n.is_empty() && !out.contains(&n) {
            out.push(n);
        }
    }
    out
}

/// Lookup order for one tag: itself, then shorter prefixes, then the
/// language: `zh-hans-cn` → `zh-hans-cn`, `zh-hans`, `zh`; `nb-no` →
/// `nb-no`, `nb`, `no`.
pub fn fallbacks(tag: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut t = tag;
    while !t.is_empty() {
        if !out.iter().any(|o| o == t) {
            out.push(t.to_string());
        }
        t = match t.rfind('-') {
            Some(i) => &t[..i],
            None => "",
        };
    }
    let lang = language(tag);
    if !lang.is_empty() && !out.iter().any(|o| o == lang) {
        out.push(lang.to_string());
    }
    out
}

/// True for right-to-left languages (Arabic, Hebrew, Persian, Urdu, and a
/// few others written in Arabic or Hebrew script).
pub fn is_rtl(tag: &str) -> bool {
    matches!(
        language(tag),
        "ar" | "he" | "fa" | "ur" | "ps" | "sd" | "ug" | "yi" | "ckb" | "dv"
    )
}

/// Preferred languages from environment variables, gettext order. `get`
/// reads a variable (injectable for tests).
pub fn from_env_with(get: impl Fn(&str) -> Option<String>) -> Vec<String> {
    let set = |k: &str| get(k).filter(|v| !v.trim().is_empty());
    let primary = set("LC_ALL")
        .or_else(|| set("LC_MESSAGES"))
        .or_else(|| set("LANG"));
    if let Some(p) = &primary {
        if normalize(p).is_empty() {
            // C/POSIX: untranslated (gettext ignores LANGUAGE then).
            return Vec::new();
        }
    }
    let mut out = set("LANGUAGE").map(|l| parse_list(&l)).unwrap_or_default();
    if let Some(p) = primary {
        for t in parse_list(&p) {
            if !out.contains(&t) {
                out.push(t);
            }
        }
    }
    out
}

fn env(k: &str) -> Option<String> {
    std::env::var(k).ok()
}

/// The user's preferred languages, best first (may be empty).
pub fn system() -> Vec<String> {
    #[cfg(target_os = "macos")]
    {
        // An explicit choice in the environment; LANG alone is not one.
        let explicit = from_env_with(|k| if k == "LANG" { None } else { env(k) });
        if !explicit.is_empty() {
            return explicit;
        }
        let os = mac::preferred();
        if !os.is_empty() {
            return os;
        }
        from_env_with(env)
    }
    #[cfg(windows)]
    {
        let explicit = from_env_with(env);
        if !explicit.is_empty() {
            return explicit;
        }
        win::preferred()
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        from_env_with(env)
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use std::ffi::{c_char, c_void, CStr};

    type CFIndex = isize;
    const UTF8: u32 = 0x0800_0100; // kCFStringEncodingUTF8

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFLocaleCopyPreferredLanguages() -> *const c_void;
        fn CFArrayGetCount(array: *const c_void) -> CFIndex;
        fn CFArrayGetValueAtIndex(array: *const c_void, index: CFIndex) -> *const c_void;
        fn CFStringGetCString(
            s: *const c_void,
            buf: *mut c_char,
            size: CFIndex,
            encoding: u32,
        ) -> u8;
        fn CFRelease(cf: *const c_void);
    }

    /// BCP 47 tags from System Settings, e.g. `["es-CL", "en-US"]`.
    pub fn preferred() -> Vec<String> {
        let mut out = Vec::new();
        // SAFETY: CoreFoundation "Copy" rule: we own the array and release
        // it once; values are borrowed from it while it is alive.
        unsafe {
            let array = CFLocaleCopyPreferredLanguages();
            if array.is_null() {
                return out;
            }
            for i in 0..CFArrayGetCount(array).max(0) {
                let s = CFArrayGetValueAtIndex(array, i);
                let mut buf = [0 as c_char; 128];
                if !s.is_null()
                    && CFStringGetCString(s, buf.as_mut_ptr(), buf.len() as CFIndex, UTF8) != 0
                {
                    let tag = super::normalize(&CStr::from_ptr(buf.as_ptr()).to_string_lossy());
                    if !tag.is_empty() && !out.contains(&tag) {
                        out.push(tag);
                    }
                }
            }
            CFRelease(array);
        }
        out
    }
}

#[cfg(windows)]
mod win {
    const MUI_LANGUAGE_NAME: u32 = 0x8;
    const LOCALE_NAME_MAX_LENGTH: usize = 85;

    #[link(name = "kernel32")]
    extern "system" {
        fn GetUserPreferredUILanguages(
            flags: u32,
            num: *mut u32,
            buffer: *mut u16,
            len: *mut u32,
        ) -> i32;
        fn GetUserDefaultLocaleName(name: *mut u16, len: i32) -> i32;
    }

    /// UI languages (`es-CL`, `en-US`), else the user locale name.
    pub fn preferred() -> Vec<String> {
        let mut out = Vec::new();
        // SAFETY: two-call pattern; the second call writes at most `len` u16s.
        unsafe {
            let (mut num, mut len) = (0u32, 0u32);
            if GetUserPreferredUILanguages(
                MUI_LANGUAGE_NAME,
                &mut num,
                std::ptr::null_mut(),
                &mut len,
            ) != 0
                && len > 0
            {
                let mut buf = vec![0u16; len as usize];
                if GetUserPreferredUILanguages(
                    MUI_LANGUAGE_NAME,
                    &mut num,
                    buf.as_mut_ptr(),
                    &mut len,
                ) != 0
                {
                    // Double-NUL-terminated list of NUL-terminated names.
                    for name in buf.split(|c| *c == 0).filter(|s| !s.is_empty()) {
                        let tag = super::normalize(&String::from_utf16_lossy(name));
                        if !tag.is_empty() && !out.contains(&tag) {
                            out.push(tag);
                        }
                    }
                }
            }
            if out.is_empty() {
                let mut buf = [0u16; LOCALE_NAME_MAX_LENGTH];
                let n = GetUserDefaultLocaleName(buf.as_mut_ptr(), buf.len() as i32);
                if n > 1 {
                    let tag = super::normalize(&String::from_utf16_lossy(&buf[..(n - 1) as usize]));
                    if !tag.is_empty() {
                        out.push(tag);
                    }
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let m: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |k| m.get(k).cloned()
    }

    #[test]
    fn normalizes_posix_and_bcp47_tags() {
        assert_eq!(normalize("fr_CA.UTF-8:en"), "fr-ca");
        assert_eq!(normalize("de_DE@euro"), "de-de");
        assert_eq!(normalize("zh-Hans-CN"), "zh-hans-cn");
        assert_eq!(normalize("C.UTF-8"), "");
        assert_eq!(normalize("POSIX"), "");
        assert_eq!(normalize(""), "");
        assert_eq!(language("nb-no"), "no");
        assert_eq!(language("nn"), "no");
        assert_eq!(language("iw-il"), "he");
        assert_eq!(
            parse_list("fr_CA.UTF-8:en,es::C"),
            vec!["fr-ca", "en", "es"]
        );
        assert_eq!(fallbacks("zh-hans-cn"), vec!["zh-hans-cn", "zh-hans", "zh"]);
        assert_eq!(fallbacks("nb-no"), vec!["nb-no", "nb", "no"]);
        assert!(
            is_rtl("ar-eg")
                && is_rtl("he")
                && is_rtl("fa")
                && is_rtl("ur")
                && !is_rtl("en")
                && !is_rtl("ja")
        );
    }

    #[test]
    fn environment_follows_gettext() {
        assert_eq!(
            from_env_with(env_of(&[("LANG", "es_CL.UTF-8")])),
            vec!["es-cl"]
        );
        assert_eq!(
            from_env_with(env_of(&[
                ("LANG", "es_CL.UTF-8"),
                ("LC_MESSAGES", "de_DE.UTF-8")
            ])),
            vec!["de-de"]
        );
        assert_eq!(
            from_env_with(env_of(&[
                ("LANG", "es_CL.UTF-8"),
                ("LC_ALL", "ja_JP.UTF-8"),
                ("LC_MESSAGES", "de_DE")
            ])),
            vec!["ja-jp"]
        );
        assert_eq!(
            from_env_with(env_of(&[("LANG", "pt_BR.UTF-8"), ("LANGUAGE", "uk:ru")])),
            vec!["uk", "ru", "pt-br"]
        );
        // C/POSIX turns translation off, LANGUAGE included.
        assert!(from_env_with(env_of(&[("LC_ALL", "C"), ("LANGUAGE", "fr")])).is_empty());
        assert_eq!(from_env_with(env_of(&[("LANGUAGE", "fr")])), vec!["fr"]);
        assert!(from_env_with(env_of(&[])).is_empty());
        // Never panics, whatever the OS gives.
        let _ = system();
    }
}
