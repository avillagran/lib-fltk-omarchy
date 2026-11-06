//! {{Name}}: count things from the keyboard.
//!
//! `backend` owns the state and the files (no UI), `ui` builds the window
//! and its keys, `cli` holds the commands agents and scripts run.

pub mod backend;
pub mod cli;
pub mod ui;

use fltk_omarchy::i18n;

/// The binary, package, `.desktop` file, icon and Wayland app id.
pub const NAME: &str = "{{name}}";
/// The name shown in the window title and the launcher.
pub const DISPLAY_NAME: &str = "{{Name}}";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Translations compiled into the binary, in Omarchy's per-language format
/// (`i18n/<code>.json`: English source → translation, counted strings as
/// `{"one": …, "other": …}`). Add a line per catalog you ship.
pub const CATALOGS: &[(&str, &str)] = &[
    ("en", include_str!("../i18n/en.json")),
    ("da", include_str!("../i18n/da.json")),
    ("de", include_str!("../i18n/de.json")),
    ("es", include_str!("../i18n/es.json")),
];

/// Loads [`CATALOGS`]. The language itself follows Omarchy's Setup >
/// Language live; the toolkit's own strings (Undo, Quit…) are built in.
pub fn load_catalogs() -> Result<(), i18n::CatalogError> {
    for (code, json) in CATALOGS {
        i18n::load_omarchy_catalog(code, json)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::CATALOGS;
    use std::collections::BTreeSet;

    fn placeholders(s: &str) -> BTreeSet<String> {
        s.split('{').skip(1).filter_map(|p| p.split_once('}')).map(|(n, _)| n.to_string()).collect()
    }

    /// Every catalog parses, translates the same strings as the others
    /// (English only needs the plural forms) and keeps the placeholders.
    #[test]
    fn catalogs_are_complete() {
        let mut keys: Option<BTreeSet<String>> = None;
        for (code, json) in CATALOGS {
            fltk_omarchy::i18n::Catalog::from_omarchy_json(code, json)
                .unwrap_or_else(|e| panic!("i18n/{code}.json: {}", e.0));
            let v: serde_json::Value = serde_json::from_str(json).unwrap();
            let obj = v.as_object().unwrap();
            for (k, t) in obj {
                let forms: Vec<&str> = match t {
                    serde_json::Value::Object(f) => {
                        assert!(f.contains_key("other"), "{code}: {k:?} has no \"other\" form");
                        f.values().filter_map(|x| x.as_str()).collect()
                    }
                    t => vec![t.as_str().unwrap()],
                };
                for f in forms {
                    assert_eq!(placeholders(f), placeholders(k), "{code}: {k:?} → {f:?}");
                }
            }
            if *code == "en" {
                continue;
            }
            let these: BTreeSet<String> = obj.keys().cloned().collect();
            match &keys {
                None => keys = Some(these),
                Some(k) => assert_eq!(&these, k, "i18n/{code}.json translates other strings"),
            }
        }
    }
}
