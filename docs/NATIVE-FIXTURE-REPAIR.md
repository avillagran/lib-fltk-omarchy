# Native diagnostic fixture proposal

This is a **static, unvalidated** follow-up to `6b55aa6` on
`toolkit/unit-fixture-preservation`. The execution hold remains in force.
No Python/shell runner, syntax checker, formatter, build, Rust example,
test or GUI was executed while preparing this proposal. Its commands and
source files do not grant permission to resume validation.

The main checkout and the Files author's worktree are unchanged. This
proposal shares the still-unvalidated UI prerequisite documented in
[UNIT-FIXTURE-REPAIR.md](UNIT-FIXTURE-REPAIR.md).

The follow-up [integration review](FIXTURE-INTEGRATION-REVIEW.md) records
the pinned author checkpoint, unresolved shared-allocation and execution
profile requirements, and the dedicated accessibility evidence still needed.

## X11 wrappers

`script/x11-check` and `script/edit-input-check` no longer remove a shared
report or recursively delete a temporary driver directory. The new
`script/validation-fixture` utility provides two source-level operations:

- `allocate` requires a caller-supplied `OMARCHY_FLTK_UI_PARENT` or `TMPDIR`.
  It checks absolute paths, parent traversal, symlink components, Unix
  ownership and mode 0700 before creating a fresh private leaf. Existing
  ancestors must be owned by root or the current user and not writable by
  other users, except root-owned sticky directories. It prints
  `NATIVE_FIXTURE_RETAINED <path>` to stderr and returns the path on stdout.
- `x11` checks the fixture, binary and driver, rejects an existing report,
  creates private state/runtime/config/cache/temp children and exclusive
  log files, then launches a new Xvfb with `-displayfd` and no TCP listener.
  It removes inherited display/startup context from the child environment
  and assigns the newly reported display. Application and driver are passed
  as argument arrays, with their working directory inside the fixture.

Each run prepares a fresh 128-bit cookie in exclusive mode-0600 server and
client authority files. Xvfb receives `-auth`; the client file names the
display returned by `-displayfd`. No desktop authority file is read or
changed, and cookie bytes are not printed. Authority files remain private
artifacts; do not copy them into receipts or the development journal.
The encoding follows [libXau's record writer](https://raw.githubusercontent.com/freedesktop-unofficial-mirror/xorg__lib__libXau/master/AuWrite.c),
and the client entry follows its [family/display matching rules](https://raw.githubusercontent.com/freedesktop-unofficial-mirror/xorg__lib__libXau/master/AuGetBest.c).
The separate server entry relies on [Xorg's protocol/data loading](https://raw.githubusercontent.com/XQuartz/xorg-server/master/os/auth.c).
This is source review, not evidence that authentication succeeds or rejects
an unauthorized client on the installed Xvfb; both checks remain pending.

The wrappers build their respective example in the canonical `target/`
directory on each authorized invocation, retaining `build.log`. They retain
the Python driver, `xvfb.log`, `application.log`, `driver.log`, and `report.json`.
The old `bash -c` command interpolation and dependency on `xvfb-run` cleanup
are removed. The launcher closes its pipe descriptors and stops/reaps only
its own process handles on success or failure. It performs no fixture cleanup.

The existing drag/key sequences, accepted edit values and geometry checks
are unchanged in source. That does not prove their timing or acceptance
still passes: the process launcher and isolated environment require actual
review and runtime evidence after authorization. Xvfb still creates/retires
its own runtime sockets and locks. The launcher is **not a filesystem
sandbox**; a confined execution profile must address that lifecycle too.
These metadata checks do not prevent concurrent mutation by the same user.

## Bench snapshots

`navtree_bench` and `media_bench` include the existing retained Rust fixture
boundary. They require the same explicit private parent, print
`BENCH_FIXTURE_RETAINED <path>` to stderr, and preserve their JSON stdout.
NavTree keeps each of its eleven measured snapshots under a distinct frame
name. Media keeps `baseline.png`, `pending.png`, and `settled.png`.

No frame is deleted or reused as the next snapshot's output. File creation
replaces the old overwrite behavior in these benchmarks, so historical
timings must not be treated as measurements of this revision. Formatting,
compilation, native snapshots and new performance baselines are pending.

## Presentation markers

The async presentation probe inspects the synthetic marker with
`symlink_metadata`, requires a regular fixture file, and moves it into an
exclusively created per-request archive before injecting the synthetic key.
It no longer deletes the marker. The caller must still supply a fresh
private probe root; this edit does not admit every standalone invocation.

The synchronous private-Sway runner uses a distinct launcher script and
token file for each explicit-token case instead of unlinking/reusing them.
It retains the existing compositor-policy, honest-status and unchanged-env
checks. It uses only the local container image (`--pull=never`). Raw private
compositor token fixtures must stay in their private evidence directory,
not be copied to source receipts or the development journal.

The native presentation API, polling, cancellation, native-serial rules and
platform implementations are unchanged. These are diagnostic artifact
changes, not new native-input, focus, or installed Files acceptance evidence.

## Outstanding boundary

Other legacy runners still reuse/remove outputs, including the Hyprland
bench wrappers, OS-input/mouse runners and accessibility wrappers. Their
process ownership and filesystem paths require separate review. This patch
does not clear the execution hold, establish global absence of deletion,
approve the accumulated source proposals, or validate the original renderer
goal. Old binaries retain their prior behavior until an authorized rebuild.
