//! Rejected requests must not map or operate on an invalid target.
use super::*;
use fltk_omarchy::native::{present, PresentError, PresentOptions};
type Check<'a> = &'a dyn Fn(&Path) -> Result<(), String>;
pub fn checks(run: &mut dyn FnMut(&str, Check)) {
    run(
        "presentation: validate before mapping, reject child/deleted/worker targets",
        &|_| {
            let mut win = fltk::window::DoubleWindow::new(0, 0, 300, 200, "presentation-errors");
            let mut child = fltk::window::DoubleWindow::new(0, 0, 100, 80, "child");
            child.end();
            win.end();
            let invalid = present(
                &mut win,
                PresentOptions {
                    startup_id: Some("bad\0hint"),
                    ..Default::default()
                },
            );
            check(
                matches!(invalid, Err(PresentError::InvalidHint)) && !win.shown(),
                "invalid hint mapped the target",
            )?;
            check(
                matches!(
                    present(&mut child, PresentOptions::default()),
                    Err(PresentError::ChildWindow)
                ) && !child.shown(),
                "child target accepted",
            )?;
            let mut worker = win.clone();
            let rejected = std::thread::spawn(move || {
                matches!(
                    present(&mut worker, PresentOptions::default()),
                    Err(PresentError::WrongThread)
                )
            })
            .join()
            .map_err(|_| "worker panicked")?;
            check(
                rejected && !win.shown(),
                "worker request touched the window",
            )?;
            let mut deleted = fltk::window::DoubleWindow::new(0, 0, 100, 80, "deleted");
            deleted.end();
            let alias = deleted.clone();
            fltk::app::delete_widget(alias);
            check(
                matches!(
                    present(&mut deleted, PresentOptions::default()),
                    Err(PresentError::DeletedWindow)
                ),
                "deleted target accepted",
            )
        },
    );
}
