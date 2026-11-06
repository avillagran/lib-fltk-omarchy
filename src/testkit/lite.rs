//! Component descriptions for [`super::tree`]. With the `a11y` feature
//! they come from the accessibility registry; without it, `a11y::describe`,
//! `a11y::register` and `a11y::set_name` hand them here, and they are kept
//! only while the kit is armed (otherwise dropped at once, as before).

use crate::a11y::Node;
#[cfg(not(feature = "a11y"))]
use fltk::prelude::*;
use fltk::widget::Widget;
use std::rc::Rc;

pub(crate) type DescribeFn = Rc<dyn Fn(&mut Node)>;

/// What a widget says about itself.
#[derive(Default)]
pub(crate) struct Described {
    pub describe: Option<DescribeFn>,
    pub name: Option<String>,
}

#[cfg(not(feature = "a11y"))]
mod store {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::collections::HashMap;

    pub(super) struct Entry {
        pub w: Widget,
        pub describe: Option<DescribeFn>,
        pub name: Option<String>,
    }

    thread_local! {
        pub(super) static REG: RefCell<HashMap<usize, Entry>> = RefCell::new(HashMap::new());
        static INSERTS: Cell<u32> = const { Cell::new(0) };
    }

    pub(super) fn with_entry<W: WidgetExt + ?Sized>(w: &W, f: impl FnOnce(&mut Entry)) {
        let p = w.as_widget_ptr() as usize;
        REG.with(|reg| {
            let mut reg = reg.borrow_mut();
            if reg.get(&p).is_some_and(|e| e.w.was_deleted()) {
                reg.remove(&p);
            }
            if !reg.contains_key(&p) {
                // Bounded when widgets come and go (popups).
                let n = INSERTS.with(|c| c.replace(c.get().wrapping_add(1)));
                if n % 256 == 255 {
                    reg.retain(|_, e| !e.w.was_deleted());
                }
                // SAFETY: a live widget; the copy only tracks its deletion.
                let tracked = unsafe { Widget::from_widget_ptr(w.as_widget_ptr()) };
                reg.insert(p, Entry { w: tracked, describe: None, name: None });
            }
            if let Some(e) = reg.get_mut(&p) {
                f(e);
            }
        })
    }
}

/// Keeps `f` as the description of `widget` (only while armed).
#[cfg(not(feature = "a11y"))]
pub(crate) fn describe<W: WidgetExt + ?Sized>(widget: &W, f: DescribeFn) {
    if super::armed() {
        store::with_entry(widget, |e| e.describe = Some(f));
    }
}

/// Keeps the accessible name of `widget` (only while armed).
#[cfg(not(feature = "a11y"))]
pub(crate) fn set_name<W: WidgetExt + ?Sized>(widget: &W, name: &str) {
    if super::armed() {
        store::with_entry(widget, |e| e.name = (!name.is_empty()).then(|| name.to_string()));
    }
}

/// The description and name override of `w`.
pub(crate) fn described(w: &Widget) -> Described {
    #[cfg(feature = "a11y")]
    {
        let i = crate::a11y::imp::info(w);
        Described { describe: i.describe, name: i.name }
    }
    #[cfg(not(feature = "a11y"))]
    {
        let p = w.as_widget_ptr() as usize;
        store::REG.with(|reg| {
            let reg = reg.borrow();
            match reg.get(&p) {
                Some(e) if !e.w.was_deleted() => Described { describe: e.describe.clone(), name: e.name.clone() },
                _ => Described::default(),
            }
        })
    }
}
