// Fitting a row into the room it has: the rules the header, the path chips
// and the status bar use when the window is narrow. Pure functions over
// widths (no FLTK, no drawing), so the narrow-window behaviour is testable
// and shared by every widget that needs it.
//
//   fo::fit::RowFold f = fo::fit::fold_row({100, 60, 40, 40}, {0, 0, 1, 1}, 36, 4, 120);
//   // f.slots == {0, -1, 1}: the two grouped buttons went into the overflow
//   // button, which sits where the first of them was.
#pragma once

#include <vector>

namespace fo {
namespace fit {

/// Which buttons of a row stay visible and which fold into one overflow
/// button. `slots` is the visual order: item indexes, and -1 for the
/// overflow button (placed where the first folded item was). `folded`
/// lists the folded items in their original order.
struct RowFold {
  std::vector<int> slots;
  std::vector<int> folded;
  bool overflow() const { return !folded.empty(); }
};

/// The width of `count` buttons `button` px wide and `gap` px apart.
int row_width(int count, int button, int gap);

/// Folds a row of equal buttons into `available` px. Everything stays when
/// it fits (or when `available` <= 0, meaning no limit). Otherwise the
/// buttons with the highest `priority` stay (ties: the earlier one), one
/// overflow button takes the rest, and items sharing a nonzero `group`
/// (a view switcher) stay or fold together. `group` may be empty.
RowFold fold_row(const std::vector<int> &priority, const std::vector<int> &group, int button, int gap,
                 int available);

/// Where the four parts of a status bar go: what is marked and listed
/// (left), the tip (hint), the filesystem and free space (right) and the
/// badge at the far right (Files: PSS). Widths 0 mean "not shown".
struct StatusRow {
  int left_x = 0, left_w = 0;
  int hint_x = 0, hint_w = 0;
  int right_x = 0, right_w = 0;
  int badge_x = 0, badge_w = 0;
};

/// Lays a status bar out by priority so nothing overlaps: the left text
/// keeps at least min(its width, 120) px (elided below its width), then
/// the badge and the right text appear only when they fit whole, then the
/// left text grows to its width, and the hint takes what is left when that
/// is at least 160 px (elided). 16 px margins, 24 px between parts.
/// Arguments are the widths each part needs (0: nothing to show).
StatusRow status_row(int x, int w, int left, int hint, int right, int badge);

/// Which path chips show. `chips` are the natural widths, the current
/// folder last. The current chip always keeps a slot; ancestors are added
/// from the nearest while they fit; when some are hidden a `fold` px "…"
/// chip leads, unless that would squeeze the current chip under
/// min(its width, `min_current`), in which case the hidden ancestors hang
/// from the current chip instead (fold = false, first > 0).
struct CrumbRow {
  int first = 0;           ///< index of the first chip shown
  bool fold = false;       ///< a leading "…" chip is shown
  std::vector<int> widths; ///< widths of chips first..last
};
CrumbRow crumbs(const std::vector<int> &chips, int available, int fold, int sep, int min_current);

} // namespace fit
} // namespace fo
