# Retained fixture integration review

Static review on 2026-11-07. **Execution remains on hold.** No formatter,
syntax checker, build, test, native probe, GUI or CI was run for this review.
This document does not admit either candidate or authorize a restart.

## Pinned inputs

| Input | Reviewed scope |
| --- | --- |
| Author `a85c591f3088ad9b7031ff375b128d85fee1ef39` | Committed common fixture boundary, native Windows owner/ACL helper, CI changes and disabled-control regressions. |
| Own `714a31acc59af695cf518b1b6604519f49f0f65f` | Unit adapter, IPC preflights and selected native diagnostic wrappers. |
| Main `8a2de43d1710b7a0fa86085cdfa8299a57f7018b` | Unchanged checkout; no candidate integration. |

The author's worktree has additional uncommitted edits in the shared helper,
glass, poll and portal. Those are not included in the pinned review. Do not
copy those edits or implement the same repair in that worktree. The own
branch still inherits `d2bfb5a`; it does not contain `a85c591`.

## Integration findings

1. **New unit children must use the same Windows creation policy.** The own
   `unit_fixture::directory` creates a case with a raw `DirBuilder`, then
   asks the shared boundary to validate the existing path. At `a85c591`,
   `create_private` normalizes the owner only immediately after an exclusive
   creation. Validating an existing directory deliberately does not do that.
   Therefore a new case with an elevated token's default administrator owner
   can be rejected even when the run root was correctly normalized. Route
   exclusive case allocation through one shared creation operation when
   integrating. Do not fix this by weakening owner checks or normalizing an
   existing path. This finding concerns the combination, not a claim that
   the author's UI-only directory creation has this defect.
2. **The current CI parent does not admit the deliberate Unix fallback
   regression.** The own `require_private_socket_fallback` requires a retained
   run directly under a real, current-user-owned, mode-0700 `/tmp`. The
   candidate CI only creates a private child under the system temp directory.
   Combining those sources leaves the long-runtime test's precondition
   unsatisfied. Keep the real fallback regression; review a confined native
   execution profile before running it. Never change the host `/tmp` mode,
   ownership or symlink to satisfy this check.
3. **The dedicated inactive-Input accessibility regression is still absent
   at the pinned author commit.** `TextField` describes the child Input's
   inactive state and rejects its accessibility actions before mutation.
   The committed disabled-control checks exercise pixels, caret/draft,
   gestures, keyboard and retained Sheet content, but do not dispatch an AT
   `SetValue` while only the Input is inactive. Required evidence remains:
   active Group/inactive Input reports disabled; SetValue leaves the draft
   unchanged; reactivation restores accepted editing. Source guards alone
   do not establish that behavior through the accessibility dispatcher.

## Windows source assessment

The candidate rejects linked/reparse paths, unsupported namespaces and NULL
DACLs; it checks existing ancestors separately from private directories.
The descriptor allocation is retained while owner/DACL/ACE aliases are
used, and is released with `LocalFree`, matching the documented
[GetSecurityInfo ownership contract](https://learn.microsoft.com/en-us/windows/win32/api/aclapi/nf-aclapi-getsecurityinfo).
Its bounded alias inspection is consistent with the returned
[self-relative descriptor layout](https://learn.microsoft.com/en-us/windows/desktop/SecAuthZ/absolute-and-self-relative-security-descriptors).
Those observations do not prove the FFI declarations compile or that the
ACL policy accepts real Windows runner directories.

Before execution can be considered, the fixture review must account for
ordinary/elevated ownership, untrusted operational and inherited grants,
NULL/unsupported ACLs, links/reparses, existing-path refusal and retention
after failure. A path/ACL checker does not confine a process or exclude
mutations by the same user or privileged administrators.

## Native driver follow-up

The own X11 wrappers now explicitly reject a failed private display
connection before sending gestures. Their embedded drivers declare display
pointer argument types and close only their own connection in `finally`.
There is no fallback to the user's display. Gesture sequences, accepted
edit values and geometry expectations are unchanged; runtime success and
unauthorized-client rejection remain unverified.

## Next integration decision

Wait for the author's committed handoff, reconcile one shared allocation
contract, and review the actual combined sources before exporting anything.
Keep UI/control acceptance, fixture retention, native input and renderer
performance as separate claims. Legacy runners and runtime socket/lock
lifecycle still need review; neither this review nor a clean Git tree lifts
the execution hold. The full renderer/o3 goal remains incomplete.
