# Validation fixture audit and execution hold

Static review on 2026-11-06 against `2d6c0ca`. No test, GUI runner, probe,
build, formatter, or alternate validation harness was executed during this
review. The user reported overwritten captures under `build/test-ui` and
stopped validation. No deletion outside that directory has been established
by this review; source inspection cannot prove what a past process deleted.

**Tests and GUI validation remain stopped.** Repair sources can be reviewed
statically, but restarting requires the user's explicit authorization. Do
not retry through unit-only tests, another runner, another backend, or CI.
Historical green results are historical evidence, not admission of the
repaired sources. Candidate control fix `23a5b7f` is not validated or adopted.

The Files author owns the separate UI/control repair worktree. This audit
does not edit those sources or claim their repair has been completed.

## Recursive deletion in test fixtures

| Location at the reviewed checkpoint | Purpose | Review consequence |
|---|---|---|
| `tests/ui.rs:71` | Delete the shared `build/test-ui` root before the harness | Deletes prior run artifacts; replace with an exclusively created private run root and preserve it. |
| `tests/ui.rs:1393`, `:1431` | Simulate replacement/missing theme directories | Preserve the old theme under a new evidence path instead of recursive deletion. |
| `tests/tooltip_ui/mod.rs:103`, `:106` | Clear the next/current tooltip theme directories | Use unique fixture paths and preserve prior directories. |
| `src/glass.rs:326`, `:341`, `:354` (`cfg(test)`) | Reuse and clean process-ID temp fixtures | Outside the UI harness; a UI-only patch does not cover these unit tests. |
| `src/platform/poll.rs:108`, `:122` (`cfg(test)`) | Reuse and clean a process-ID temp fixture | Also contains single-file deletion to simulate disappearance. |
| `src/platform/portal/mod.rs:337`, `:397` (`cfg(test)`) | Fake bus cleanup in `Drop` and before creation | Destruction can run after a test scope returns; inspect teardown as well as test bodies. |

There are 12 literal `remove_dir_all` calls across these five source files
at this checkpoint. This count is an inventory of these matched calls, not
proof that indirect deletion APIs or subprocesses have been exhausted.

## Additional direct test cleanup and fixed output paths

`tests/ui.rs` also removes user override files at lines 347, 372, 990 and
1444. `tests/tooltip_ui/mod.rs:335` removes a settings file, and
`tests/typo_ui/mod.rs:353` removes the language fixture. A repair promising
no fixture deletion needs to cover single-file operations too.

These modules bypass the main harness root and must receive the new root:

| Module | Fixed output path |
|---|---|
| `tests/playback_ui/mod.rs:19` | `build/test-ui/playback` |
| `tests/typo_ui/mod.rs:328` | `build/test-ui/xdg-config` |
| `tests/diff_ui/mod.rs:749`, `:774` | `build/test-ui/shots/a11y-diff*.png` |
| `tests/ui.rs:987` | A shared `omarchy-fltk-unused.png` in the system temp directory |

Further direct cleanup occurs in test sections of `src/instance.rs`,
`src/frame/file.rs`, `src/frame/surface.rs`, `src/platform/frame/mod.rs`,
`src/media/cache.rs`, `src/media/playback/proc.rs`, and in `tests/frame.rs`
and `tests/instance.rs`. These include file/empty-directory cleanup rather
than recursive deletion. Their path ownership, evidence retention and any
fixture links still need separate review before a whole-suite restart.

## Native diagnostics and runtime ownership

Legacy runners also remove or reuse report files, including `x11-check`,
`edit-input-check`, `real-window`, `os-input-check`, `os-mouse-check`,
`frame-bench`, `motion-bench`, `gpu-bench`, `richtext-bench`, `edit-bench`,
`richtext-a11y-bench`, `stream-bench`, `gfx-bench`, and `a11y-check`.
`x11-check` and `edit-input-check` have recursive temporary-driver cleanup.
Diagnostic examples `async_present_check`, `navtree_bench`, and `media_bench`
also contain direct file removal. None was run during this audit.

The glyph diagnostic introduced in `2d6c0ca` reused a caller-supplied output
directory and could overwrite its PNG/JSON files. The accompanying static
patch requires an exclusively created private output directory, rejects
existing final entries before initializing FLTK, and moves Wayland probe
artifacts into a fresh child of its already unique fixture. This patch has
**not been compiled or executed**; old binaries retain their old behavior.

Deletion in production resource lifecycle code is a separate category:
`src/platform/instance/unix.rs` retires an owned socket, and platform frame
backends retire owned files/shared-memory names. Finding an unlink does not
alone prove an unsafe operation. Do not disable these runtime contracts as
a blanket test-cleanup fix. Tests that exercise such lifecycle behavior need
an explicit ownership/confinement review as well.

## Review boundary before resuming

The pending patch needs a complete path review, including root creation,
all nested fixtures, aliases/symlinks, output collisions, failure and Drop
paths, and subprocesses. Preserve evidence on success and failure. Verify
that a rename used to simulate absence cannot replace an existing evidence
destination. Updating only the harness's initial cleanup does not establish
that its entire execution is free of fixture deletion or artifact reuse.

This document is source-review evidence only. It establishes no new test
result or permission to execute repaired or alternative validation paths.
