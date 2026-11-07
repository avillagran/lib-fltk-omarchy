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

pub fn async_checks(run: &mut dyn FnMut(&str, Check)) {
    use fltk_omarchy::native::begin_present;
    use std::task::Poll;
    run("presentation async: validation before mapping", &|_| {
        let mut win = fltk::window::DoubleWindow::new(0, 0, 300, 200, "async-validation");
        let mut child = fltk::window::DoubleWindow::new(0, 0, 100, 80, "child");
        child.end();
        win.end();
        check(
            matches!(
                begin_present(
                    &mut win,
                    PresentOptions {
                        startup_id: Some("bad\0hint"),
                        ..Default::default()
                    }
                ),
                Err(PresentError::InvalidHint)
            ) && !win.shown(),
            "invalid hint mapped target",
        )?;
        check(
            matches!(
                begin_present(&mut child, PresentOptions::default()),
                Err(PresentError::ChildWindow)
            ) && !child.shown(),
            "child accepted or mapped",
        )?;
        let mut worker = win.clone();
        let rejected = std::thread::spawn(move || {
            matches!(
                begin_present(&mut worker, PresentOptions::default()),
                Err(PresentError::WrongThread)
            )
        })
        .join()
        .map_err(|_| "worker panicked")?;
        check(rejected && !win.shown(), "worker touched target")?;
        let mut deleted = fltk::window::DoubleWindow::new(0, 0, 100, 80, "deleted");
        deleted.end();
        fltk::app::delete_widget(deleted.clone());
        check(
            matches!(
                begin_present(&mut deleted, PresentOptions::default()),
                Err(PresentError::DeletedWindow)
            ),
            "deleted target accepted",
        )
    });
    run(
        "presentation async: cancellation and lease lifecycle",
        &|_| {
            let mut a = fltk::window::DoubleWindow::new(0, 0, 100, 80, "async-a");
            a.end();
            let mut b = fltk::window::DoubleWindow::new(0, 0, 100, 80, "async-b");
            b.end();
            let mut first =
                begin_present(&mut a, PresentOptions::default()).map_err(|e| e.to_string())?;
            check(
                matches!(
                    begin_present(&mut b, PresentOptions::default()),
                    Err(PresentError::Busy)
                ),
                "second request did not respect lease",
            )?;
            check(
                first.cancel() && !first.cancel(),
                "cancellation was not one-shot",
            )?;
            check(
                matches!(first.poll(), Poll::Ready(Err(PresentError::Cancelled)))
                    && matches!(first.poll(), Poll::Ready(Err(PresentError::Cancelled))),
                "cancelled result was not cached",
            )?;
            let second =
                begin_present(&mut b, PresentOptions::default()).map_err(|e| e.to_string())?;
            drop(first);
            check(
                matches!(
                    begin_present(&mut a, PresentOptions::default()),
                    Err(PresentError::Busy)
                ),
                "old handle released new lease",
            )?;
            drop(second);
            let third =
                begin_present(&mut a, PresentOptions::default()).map_err(|e| e.to_string())?;
            drop(third);
            a.hide();
            b.hide();
            Ok(())
        },
    );
    run("presentation async: completed result is cached", &|_| {
        let mut win = fltk::window::DoubleWindow::new(0, 0, 100, 80, "async-complete");
        win.end();
        let mut request =
            begin_present(&mut win, PresentOptions::default()).map_err(|e| e.to_string())?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let result = loop {
            match request.poll() {
                Poll::Ready(result) => break result,
                Poll::Pending if std::time::Instant::now() < deadline => {
                    let _ = fltk::app::wait_for(0.01);
                }
                Poll::Pending => return Err("request did not complete".into()),
            }
        };
        let again = request.poll();
        check(
            matches!(again, Poll::Ready(ref value) if format!("{value:?}") == format!("{result:?}"))
                && !request.cancel(),
            "terminal result was not cached",
        )?;
        win.hide();
        Ok(())
    });
    run(
        "presentation async: hidden and deleted pending targets",
        &|_| {
            let mut a = fltk::window::DoubleWindow::new(0, 0, 100, 80, "async-hide");
            a.end();
            let mut first =
                begin_present(&mut a, PresentOptions::default()).map_err(|e| e.to_string())?;
            a.hide();
            check(
                matches!(first.poll(), Poll::Ready(Err(PresentError::Cancelled)))
                    && matches!(first.poll(), Poll::Ready(Err(PresentError::Cancelled)))
                    && !first.cancel(),
                "hidden target did not terminate",
            )?;
            let mut b = fltk::window::DoubleWindow::new(0, 0, 100, 80, "async-delete");
            b.end();
            let mut second =
                begin_present(&mut b, PresentOptions::default()).map_err(|e| e.to_string())?;
            fltk::app::delete_widget(b.clone());
            check(
                matches!(second.poll(), Poll::Ready(Err(PresentError::DeletedWindow)))
                    && matches!(second.poll(), Poll::Ready(Err(PresentError::DeletedWindow)))
                    && !second.cancel(),
                "deleted target did not terminate",
            )
        },
    );
}
