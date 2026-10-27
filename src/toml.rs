//! Minimal reader for the TOML subset Omarchy writes in `colors.toml` and
//! `shell.toml`: `[section]` headers and `key = value` lines with strings,
//! numbers and booleans. Keys are flattened to `"section.key"`.

use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Str(String),
    Num(f64),
    Bool(bool),
}

impl Value {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Num(n) => Some(*n),
            Value::Str(s) => s.trim().parse().ok(),
            Value::Bool(_) => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            Value::Str(s) => match s.trim().to_ascii_lowercase().as_str() {
                "true" | "1" | "yes" | "on" => Some(true),
                "false" | "0" | "no" | "off" => Some(false),
                _ => None,
            },
            Value::Num(n) => Some(*n != 0.0),
        }
    }
}

pub type Table = HashMap<String, Value>;

pub fn parse(text: &str) -> Table {
    let mut out = Table::new();
    let mut section = String::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            if let Some(end) = line.find(']') {
                section = line[1..end].trim().to_string();
            }
            continue;
        }
        let Some((key, rest)) = line.split_once('=') else { continue };
        let key = key.trim().trim_matches('"');
        let rest = rest.trim();
        let value = if let Some(q) = rest.chars().next().filter(|c| *c == '"' || *c == '\'') {
            let body = &rest[1..];
            match body.find(q) {
                Some(end) => Value::Str(body[..end].to_string()),
                None => Value::Str(body.to_string()),
            }
        } else {
            let bare = rest.split('#').next().unwrap_or("").trim();
            match bare {
                "true" => Value::Bool(true),
                "false" => Value::Bool(false),
                _ => bare.parse::<f64>().map(Value::Num).unwrap_or_else(|_| Value::Str(bare.to_string())),
            }
        };
        let full = if section.is_empty() { key.to_string() } else { format!("{section}.{key}") };
        out.insert(full, value);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sections_strings_numbers_comments() {
        let t = parse("top = \"#e68e0d\"  # c\n[font]\nbase-size = 12 # x\n[menu]\nborder = \"hyprland.active-border\"\nflag = true\nalpha = 0.95\n");
        assert_eq!(t["top"].as_str(), Some("#e68e0d"));
        assert_eq!(t["font.base-size"].as_f64(), Some(12.0));
        assert_eq!(t["menu.border"].as_str(), Some("hyprland.active-border"));
        assert_eq!(t["menu.flag"].as_bool(), Some(true));
        assert_eq!(t["menu.alpha"].as_f64(), Some(0.95));
    }
}
