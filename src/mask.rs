//! Input masks for fixed-format fields: `"99:99"` (time of day),
//! `"9999-99-99"` (date), `"9999-99-99T99:99"` (o3-code's scheduled tasks
//! and snooze dates). Use them through [`TextField::mask`](crate::widgets::TextField::mask).
//!
//! In a mask `9` is a digit, `a` a letter, `*` a letter or digit; any
//! other character is a literal, typed for the user between the characters
//! they enter (`\` makes the next character literal: `"\9"`). Text that
//! does not fit (a letter in a digit slot, characters past the end) is
//! dropped, so pasting `"0730"` into `"99:99"` gives `"07:30"`.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    Digit,
    Letter,
    Alnum,
    Literal(char),
}

impl Slot {
    fn accepts(self, c: char) -> bool {
        match self {
            Slot::Digit => c.is_ascii_digit(),
            Slot::Letter => c.is_alphabetic(),
            Slot::Alnum => c.is_alphanumeric(),
            Slot::Literal(_) => false,
        }
    }
}

/// A parsed input mask. See the [module docs](self).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mask {
    slots: Vec<Slot>,
    spec: String,
}

impl Mask {
    pub fn new(spec: &str) -> Mask {
        let mut slots = Vec::new();
        let mut chars = spec.chars();
        while let Some(c) = chars.next() {
            slots.push(match c {
                '9' => Slot::Digit,
                'a' => Slot::Letter,
                '*' => Slot::Alnum,
                '\\' => Slot::Literal(chars.next().unwrap_or('\\')),
                other => Slot::Literal(other),
            });
        }
        Mask { slots, spec: spec.to_string() }
    }

    /// The mask as written.
    pub fn spec(&self) -> &str {
        &self.spec
    }

    /// A hint for the placeholder: the mask with `9` → `0`, `a` → `A`,
    /// `*` → `_` (`"00:00"`). Apps usually show their own ("HH:MM").
    pub fn hint(&self) -> String {
        self.slots
            .iter()
            .map(|s| match s {
                Slot::Digit => '0',
                Slot::Letter => 'A',
                Slot::Alnum => '_',
                Slot::Literal(c) => *c,
            })
            .collect()
    }

    /// Formats `raw` (what the field holds after an edit) to the mask and
    /// maps the caret (a byte offset in `raw`) to the formatted text.
    pub fn format(&self, raw: &str, caret: usize) -> (String, usize) {
        let mut out = String::new();
        let mut new_caret = None;
        let mut m = 0;
        for (i, c) in raw.char_indices() {
            if new_caret.is_none() && i >= caret {
                new_caret = Some(out.len());
            }
            if m >= self.slots.len() {
                break;
            }
            // Literals before the next slot: the user typed it, or we do.
            loop {
                match self.slots.get(m) {
                    Some(Slot::Literal(l)) if *l == c => {
                        out.push(c);
                        m += 1;
                        break;
                    }
                    Some(Slot::Literal(l)) => {
                        out.push(*l);
                        m += 1;
                    }
                    Some(s) if s.accepts(c) => {
                        out.push(c);
                        m += 1;
                        break;
                    }
                    // Not accepted here: drop it.
                    _ => break,
                }
            }
        }
        let caret = new_caret.unwrap_or(out.len()).min(out.len());
        (out, caret)
    }

    /// True when `text` fills every slot of the mask.
    pub fn complete(&self, text: &str) -> bool {
        let chars: Vec<char> = text.chars().collect();
        chars.len() == self.slots.len()
            && self.slots.iter().zip(&chars).all(|(s, c)| match s {
                Slot::Literal(l) => l == c,
                s => s.accepts(*c),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::Mask;

    fn f(m: &Mask, raw: &str) -> String {
        m.format(raw, raw.len()).0
    }

    #[test]
    fn time_of_day() {
        let m = Mask::new("99:99");
        assert_eq!(f(&m, "0"), "0");
        assert_eq!(f(&m, "07"), "07");
        assert_eq!(f(&m, "073"), "07:3");
        assert_eq!(f(&m, "07:3"), "07:3");
        assert_eq!(f(&m, "0730"), "07:30");
        assert_eq!(f(&m, "07305"), "07:30", "past the end is dropped");
        assert_eq!(f(&m, "07:"), "07:", "a typed literal stays");
        assert_eq!(f(&m, "0x7"), "07", "letters in digit slots are dropped");
        assert_eq!(f(&m, "7:30"), "73:0", "a literal where a digit goes is dropped");
        assert!(m.complete("07:30"));
        assert!(!m.complete("07:3"));
        assert!(!m.complete("07-30"));
        assert_eq!(m.hint(), "00:00");
    }

    #[test]
    fn iso_date_time_with_letter_literal() {
        let m = Mask::new("9999-99-99T99:99");
        assert_eq!(f(&m, "202611061430"), "2026-11-06T14:30");
        assert_eq!(f(&m, "2026-11-06T14:30"), "2026-11-06T14:30");
        assert!(m.complete("2026-11-06T14:30"));
        assert_eq!(f(&m, "20261106"), "2026-11-06");
    }

    #[test]
    fn caret_follows_inserted_literals() {
        let m = Mask::new("99:99");
        // Typing "3" after "07" (caret at the end) → "07:3", caret at the end.
        assert_eq!(m.format("073", 3), ("07:3".into(), 4));
        // Caret inside: after "0" in "0730" → after "0" in "07:30".
        assert_eq!(m.format("0730", 1), ("07:30".into(), 1));
        assert_eq!(m.format("0730", 3), ("07:30".into(), 4));
        // Escaped mask characters are literals.
        let e = Mask::new("\\9-99");
        assert_eq!(f(&e, "12"), "9-12");
    }
}
