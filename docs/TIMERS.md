# Owned UI timers

Create timers after `App::new`, on the UI thread. Callbacks execute there
and can update `State` directly. Keep callbacks short; blocking work belongs
in `task::Task`. Widgets already animate through `anim` and need no app timer.

```rust,no_run
use fltk_omarchy::{timer, App, State};
use std::time::Duration;

let app = App::new("dev.example.timers");
let ready = State::new(false);
let value = ready.clone();
let pending = timer::after(Duration::from_millis(250), move || value.set(true));
// pending.cancel() releases the callback and its captures immediately.
// Dropping `pending` alone leaves the one-shot scheduled.
app.run();
```

`timer::every(interval, callback)` repeats while the callback returns `true`.
Returning `false`, panicking, or calling `Timer::cancel` terminates it. A
repeat retains one callback allocation for its lifetime. Its cadence uses
the previous deadline; an overdue repeat resumes at least 1 ms later instead
of catching up in a burst. A zero repeat interval is rejected. Timers are
best-effort deadlines, not a real-time clock.

`Timer` is a detached, stable ID. Canceling a stale ID returns `false` and
cannot affect a newer timer. All handle operations require the UI thread.
For cancellation when an owner leaves scope, retain `timer.guard()`;
`TimerGuard` cannot leave the UI thread and cancels on drop. Calling
`guard.detach()` returns the detached `Timer` and keeps the job alive.
As with any stored callback, capturing a strong reference to an owner which
holds its guard can form a cycle; capture a weak reference instead.

Canceling an executing callback removes its ID immediately. Its captures
drop after that invocation returns; a subsequent repeat request cannot
resurrect it. Capture destructors run outside the scheduler's registry
borrow and can schedule or cancel other timers. Nested modal event loops
can run other timers, but cannot reenter the currently executing callback.
Panics terminate that job and do not unwind across the native callback ABI;
the normal panic hook still reports them.

The scheduler keeps exact deadline entries in one ordered queue, with no
canceled tombstones, and arms one native timeout for the earliest entry.
It has no polling thread or periodic idle wakeup. Ready-job bursts yield
after 64 callbacks to let input and painting proceed. The animation frame
clock uses its own fixed native callback without allocating one closure
per frame. Some existing components deliberately poll while alive (for
example shader backend discovery); this does not remove those policies.

`task::Latest` also cancels its pending debounce timer when a request is
replaced or canceled, or when the final `Latest` owner is dropped. Pending
work is freed immediately. Running work still requires cooperative `Ctx`
cancellation. Dropping one of several clones keeps the shared owner alive.

## Verification and limits

`testkit::timer_stats()` exposes live `pending`, `queued`, and `executing`
counts, lifetime `scheduled`, `invoked`, `completed`, and `canceled` counts,
the `peak_pending` high-water mark, and `native_armed`. Read it on the UI
thread. At idle, all live counts and `native_armed` are zero and
`scheduled == completed + canceled`. A canceled callback can still be
executing until it returns. Raw frame-clock and sheet-retention timers are
separate and are not counted here.

The pinned fltk-rs 1.5.23 `add_timeout3` bridge allocates a callback box
without reclaiming it after firing or `remove_timeout3`. The toolkit uses
the owned scheduler for its internal closure timers. Plain FLTK APIs remain
available; calling `fltk::app::add_timeout3` directly retains upstream behavior.

Reproduce on a private display:

```sh
env -u WAYLAND_DISPLAY xvfb-run -a cargo test --offline --test timer
env -u WAYLAND_DISPLAY xvfb-run -a cargo run --offline --example timer_ownership_check
```

The diagnostic intentionally exercises upstream retention in its short-lived
process. With 128 fired and 128 canceled callbacks, each capturing 4096 bytes,
upstream drops 0 captures and retains 1,048,576 payload bytes; the owned
scheduler drops all 256 captures and retains zero payload bytes. These are
ownership counts, not RSS/PSS, allocator residency, driver memory, or a
power measurement. They do not establish the renderer's cold-memory gate.

The main-thread runtime harness also checks burst cleanup, stable stale IDs,
repetition, self-cancellation, nested event loops, destructor reentrancy,
scoped guards, panic cleanup, `Latest` replacement/final-owner cancellation,
and 64 native frame deadlines delivering redraws without owned timer jobs.
