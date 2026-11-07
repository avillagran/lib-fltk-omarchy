# Retained library fixture proposal

This is a **static, unvalidated** follow-up to UI fixture checkpoint
`d2bfb5ac433ad8786c11b5c320d9e1b6ba4af481`. Validation is on hold after the
fixture-deletion report. No build, formatter, test, GUI, probe, or alternate
runner was executed while preparing this proposal. It is not authorization
to resume validation or to adopt the earlier disabled-controls candidate.

The proposal lives on `toolkit/unit-fixture-preservation` in a separate
worktree. The main checkout and the Files author's worktree are unchanged.
Its prerequisite includes the author's still-unvalidated UI/control changes;
do not apply the unit follow-up without reviewing that prerequisite.

## Changes

All Rust changes are under `cfg(test)`:

- Library tests register the existing `platform::test_fixture` boundary.
  Its source is unchanged from the prerequisite. The UI harness includes
  the same source in its own test process.
- `platform::unit_fixture` initializes one retained run with `OnceLock`,
  allocates case directories exclusively, and assigns checked atomic IDs
  for parallel tests. A collision fails instead of reusing or clearing a
  case. Initialization errors remain errors for that process.
- Glass fixtures no longer clear a process-ID directory or remove it at
  the end. Each case gets a fresh retained directory.
- The poll fixture moves its watched file to a new evidence directory to
  make the original path absent. It checks that the retired bytes remain.
- The fake portal creates its Unix socket in a retained case. Its former
  recursive-cleanup `Drop` and pre-creation deletion are removed. A socket
  path longer than 100 bytes fails with a request for a shorter private
  parent before binding.
- Frame transport and surface unit cases use retained directories and no
  longer enumerate directories for cleanup. The file-transport inspection
  uses `DirEntry::file_type` and rejects unexpected aliases instead of
  following them or ignoring directory-read errors.
- The frame-file watcher keeps `initial.bmp` before atomically replacing
  the active `frame.bmp` pathname. This retains the initial bytes without
  introducing an absence gap into the original replacement test. Both
  fixture versions stay in the case directory after the watcher stops.
- Media-cache and playback fixtures use private cases. FFmpeg generates a
  new `clip.mp4` with `-n` and `-nostdin`; missing optional tools still skip,
  but encoder failures now fail instead of appearing to be missing tools.
  Clips, junk input and bounded-read image fixtures are retained.
- The cross-process frame integration binary includes the same retained
  boundary and adapter sources and removes its directory cleanup. Its
  producer child remains selected by `--exact child_producer`; the adapter's
  own `cfg(test)` checks are also present in the integration binary's full
  test list.

These edits remove seven literal `remove_dir_all` calls and the poll test's
single-file removal. The proposal also adds source-level regression cases
for distinct directories, retention of two file versions, root/traversal
rejection, and a fixture alias whose target is another private fixture.
None of those cases has run.

The frame/media follow-up removes thirteen additional direct file/directory
cleanup calls across six files. It leaves atomic replacement of the watched
active pathname and the runtime's resource lifecycle behavior in place;
retention of fixture evidence is not a claim that these tests execute no
runtime unlink operations.

## Parent and retention contract

Before an authorized test process starts, its caller must supply a private
absolute parent through `OMARCHY_FLTK_UI_PARENT` or `TMPDIR`, matching the
shared boundary's existing contract. On Unix it must belong to the caller
with mode 0700 and contain no symlink components. Windows inherits the
caller-supplied parent's ACL; that platform still needs its own review and
execution evidence. Fake portal tests also require a short parent for Unix
socket paths. No fallback selects a shared system temp directory.

The process prints `UNIT_FIXTURE_RETAINED <path>` once. Evidence remains
after success, assertion failure, or normal object destruction. No automatic
cleanup is provided. The boundary rejects a request to preserve the run
root, traversal paths, or a linked path; it creates a new retained destination
for each accepted move. The new adapter does not change environment variables.

## Review still required

The proposal covers glass/poll/fake-portal, frame/media unit fixtures and
the frame integration fixture. Direct cleanup in instance unit/integration
fixtures, shared legacy runner outputs, and resource lifecycle operations
reached by tests remain outside this patch. Production socket/shared-memory
ownership behavior is unchanged. These source changes do not establish that
a whole-suite run is confined or free of fixture cleanup.

Before any execution, review the shared prerequisite, every path created or
moved here, the added cases, parallel initialization, target platforms, and
the remaining inventory. Formatting, compilation, MSRV/cross checks and
runtime regressions are all pending. Restart requires the user's explicit
authorization after that review; no alternate harness or CI bypass is implied.
