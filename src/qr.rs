//! QR codes (crate feature `qr`, off by default): pairing links and
//! invites (o3-code's device pairing and OmaBridge invites).
//!
//! The code is drawn as crisp squares, dark modules on a light card made
//! from the theme (the background color on the foreground), with the
//! standard quiet zone, so phone cameras read it in dark and light themes.
//!
//! ```no_run
//! # #[cfg(feature = "qr")] {
//! use fltk_omarchy::{qr::Qr, State};
//! # let _app = fltk_omarchy::App::new("dev.example.qr");
//! let link = State::new("https://example.com/pair?code=123456".to_string());
//! let code = Qr::new(&link).accessible_label("Pairing code");
//! # }
//! ```

use crate::a11y::{self, Role};
use crate::paint;
use crate::state::State;
use crate::theme;
use fltk::{enums::*, prelude::*, widget::Widget};
use qrcode::{Color as Module, EcLevel, QrCode};
use std::cell::RefCell;
use std::rc::Rc;

/// Modules of the quiet zone around the code (the standard asks for 4).
pub const QUIET: usize = 4;

/// The modules of `text` as rows of booleans (true = dark), without the
/// quiet zone; `None` when the text is too long for a QR code.
pub fn modules(text: &str) -> Option<Vec<Vec<bool>>> {
    let code = QrCode::with_error_correction_level(text.as_bytes(), EcLevel::M).ok()?;
    let w = code.width();
    let colors = code.to_colors();
    Some(colors.chunks(w).map(|row| row.iter().map(|m| *m == Module::Dark).collect()).collect())
}

/// A QR code of a bound text, as large as fits its box in whole pixels per
/// module.
#[derive(Clone)]
pub struct Qr {
    inner: Widget,
    label: Rc<RefCell<String>>,
}
fltk::widget_extends!(Qr, Widget, inner);

impl Qr {
    pub fn new(text: &State<String>) -> Qr {
        let mut inner = Widget::default();
        inner.set_frame(FrameType::NoBox);
        inner.set_size(180, 180);
        let cache: Rc<RefCell<(String, Option<Vec<Vec<bool>>>)>> = Rc::new(RefCell::new((String::from("\u{0}"), None)));
        let (t, c) = (text.clone(), cache.clone());
        inner.draw(move |w| {
            let s = t.get();
            let mut c = c.borrow_mut();
            if c.0 != s {
                *c = (s.clone(), modules(&s));
            }
            theme::with(|th| {
                let (light, dark) = (th.palette.foreground, th.palette.background);
                let Some(m) = c.1.as_ref() else {
                    paint::text(&crate::tr!("Too long for a QR code"), w.x(), w.y(), w.w(), w.h(), th.font.caption, false, light, Align::Center | Align::Wrap);
                    return;
                };
                let n = m.len() + 2 * QUIET;
                let unit = (w.w().min(w.h()) / n as i32).max(1);
                let side = unit * n as i32;
                let (x0, y0) = (w.x() + (w.w() - side) / 2, w.y() + (w.h() - side) / 2);
                paint::fill(x0, y0, side, side, th.radius.min(unit * 2), light);
                fltk::draw::set_draw_color(dark.fl());
                for (r, row) in m.iter().enumerate() {
                    for (col, on) in row.iter().enumerate() {
                        if *on {
                            let x = x0 + (col + QUIET) as i32 * unit;
                            let y = y0 + (r + QUIET) as i32 * unit;
                            fltk::draw::draw_rectf(x, y, unit, unit);
                        }
                    }
                }
            });
        });
        let mut w2 = inner.clone();
        crate::state::bind_widget(&inner, text, false, move |_| w2.redraw());
        let label = Rc::new(RefCell::new(String::new()));
        let l = label.clone();
        a11y::describe(&inner, move |n| {
            n.role(Role::Image).name(&l.borrow());
        });
        Qr { inner, label }
    }

    /// The name screen readers read for the image ("Pairing code"); give
    /// the link itself in visible text next to it.
    pub fn accessible_label(self, name: &str) -> Self {
        *self.label.borrow_mut() = name.to_string();
        self
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn modules_are_square_with_finders() {
        let m = super::modules("https://example.com/pair?code=123456").unwrap();
        let n = m.len();
        assert!(n >= 21 && (n - 21) % 4 == 0, "QR sizes are 21 + 4k: {n}");
        assert!(m.iter().all(|r| r.len() == n));
        // Finder patterns: the 7×7 corners start with a dark border.
        for (r, c) in [(0, 0), (0, n - 7), (n - 7, 0)] {
            assert!((0..7).all(|i| m[r][c + i] && m[r + 6][c + i] && m[r + i][c] && m[r + i][c + 6]));
            assert!(!m[r + 1][c + 1] && m[r + 3][c + 3]);
        }
        assert!(super::modules(&"x".repeat(5000)).is_none());
    }
}
