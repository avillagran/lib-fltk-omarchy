// Narrow rows (fit.h): pure layout rules, nothing drawn. The ToolBar's
// overflow column is tested with a display (test_display.cpp).
#include "harness.h"

#include "fltk-omarchy/fit.h"

#include <FL/Fl.H>

#include <algorithm>

using namespace fo;

namespace {

bool overlaps(int ax, int aw, int bx, int bw) { return aw > 0 && bw > 0 && ax < bx + bw && bx < ax + aw; }

} // namespace

TEST(fit_row_keeps_everything_that_fits) {
  fit::RowFold f = fit::fold_row({1, 2, 3}, {}, 36, 4, 116);
  CHECK_EQ(f.slots.size(), size_t(3));
  CHECK(!f.overflow());
  f = fit::fold_row({1, 2, 3}, {}, 36, 4, 0); // no limit
  CHECK(!f.overflow());
}

TEST(fit_row_folds_low_priority_and_groups_together) {
  // search, edit, 5 views (group 1), inspector, actions, preferences
  std::vector<int> pri = {100, 60, 40, 40, 40, 40, 40, 50, 90, 95};
  std::vector<int> grp = {0, 0, 1, 1, 1, 1, 1, 0, 0, 0};
  fit::RowFold f = fit::fold_row(pri, grp, 36, 4, 346); // 640 px window
  // The view group folds whole; the overflow sits where it was.
  CHECK(f.overflow());
  std::vector<int> want = {0, 1, -1, 7, 8, 9};
  CHECK(f.slots == want);
  std::vector<int> folded = {2, 3, 4, 5, 6};
  CHECK(f.folded == folded);
  CHECK(fit::row_width(int(f.slots.size()), 36, 4) <= 346);
  // 420 px: only the two most important stay, beside the overflow.
  f = fit::fold_row(pri, grp, 36, 4, 126);
  want = {0, -1, 9};
  CHECK(f.slots == want);
  // No room even for the overflow button: it is all there is.
  f = fit::fold_row(pri, grp, 36, 4, 20);
  CHECK_EQ(f.slots.size(), size_t(1));
  CHECK_EQ(f.slots[0], -1);
  CHECK_EQ(f.folded.size(), pri.size());
  // Every width from 0 to the full row: never wider than the room.
  for (int a = 36; a < 400; ++a) {
    fit::RowFold g = fit::fold_row(pri, grp, 36, 4, a);
    CHECK(fit::row_width(int(g.slots.size()), 36, 4) <= a);
    CHECK_EQ(g.slots.size() - (g.overflow() ? 1 : 0) + g.folded.size(), pri.size());
    // The group is never split.
    int in = 0;
    for (int i : g.folded)
      in += grp[size_t(i)] == 1;
    CHECK(in == 0 || in == 5);
  }
}

TEST(fit_status_parts_never_overlap) {
  for (int w = 100; w <= 1900; w += 7) {
    fit::StatusRow r = fit::status_row(0, w, 140, 600, 150, 100);
    CHECK(r.left_w >= std::min(120, w - 32));
    CHECK(!overlaps(r.left_x, r.left_w, r.right_x, r.right_w));
    CHECK(!overlaps(r.left_x, r.left_w, r.badge_x, r.badge_w));
    CHECK(!overlaps(r.left_x, r.left_w, r.hint_x, r.hint_w));
    CHECK(!overlaps(r.hint_x, r.hint_w, r.right_x, r.right_w));
    CHECK(!overlaps(r.hint_x, r.hint_w, r.badge_x, r.badge_w));
    CHECK(!overlaps(r.right_x, r.right_w, r.badge_x, r.badge_w));
    CHECK(r.right_w == 0 || r.right_w == 150); // whole or not at all
    CHECK(r.badge_w == 0 || r.badge_w == 100);
    CHECK(r.hint_w == 0 || r.hint_w >= 160);
    int end = std::max({r.left_x + r.left_w, r.hint_x + r.hint_w, r.right_x + r.right_w, r.badge_x + r.badge_w});
    CHECK(end <= w - 16);
  }
  // Wide: everything shows; the badge outlives the free space, which
  // outlives the tip.
  fit::StatusRow r = fit::status_row(0, 1900, 140, 600, 150, 100);
  CHECK(r.hint_w == 600 && r.right_w == 150 && r.badge_w == 100);
  r = fit::status_row(0, 420, 140, 600, 150, 100);
  CHECK(r.badge_w == 100);
  CHECK(r.hint_w == 0);
  CHECK_EQ(r.left_w, 140);
}

TEST(fit_crumbs_always_show_the_current_folder) {
  std::vector<int> chips = {40, 60, 80, 120};
  fit::CrumbRow r = fit::crumbs(chips, 1000, 28, 14, 58);
  CHECK_EQ(r.first, 0);
  CHECK(!r.fold);
  // Narrow: the current chip keeps a readable width; "…" only with room.
  for (int a = 0; a < 400; ++a) {
    r = fit::crumbs(chips, a, 28, 14, 58);
    CHECK(!r.widths.empty());
    int cur = r.widths.back();
    CHECK(cur == std::min(120, a) || cur >= 58);
    int used = (r.fold ? 28 + 14 : 0);
    for (size_t k = 0; k < r.widths.size(); ++k)
      used += r.widths[k] + (k ? 14 : 0);
    CHECK(used <= std::max(a, 0));
    if (r.fold)
      CHECK(r.first > 0);
  }
  // 70 px: no room for "…" and a readable chip, so the current one takes it all.
  r = fit::crumbs(chips, 70, 28, 14, 58);
  CHECK(!r.fold);
  CHECK_EQ(r.first, 3);
  CHECK_EQ(r.widths.back(), 70);
  // 110 px: "…" plus a squeezed but readable current chip.
  r = fit::crumbs(chips, 110, 28, 14, 58);
  CHECK(r.fold);
  CHECK_EQ(r.widths.back(), 110 - 42);
}


// Folio's icon line (icons.h Style::Folio): its own drawings for the names
// it has, Lucide for the rest, never an empty drawing.
#include "fltk-omarchy/icons.h"

TEST(icons_folio_style_covers_every_name) {
  using namespace fo::icon;
  CHECK(folio::has("folder") && folio::has("documents") && folio::has("arrow-left") && folio::has("file-image"));
  CHECK(!folio::has("search") && !folio::has("settings"));
  CHECK(folio::path("folder", false) == "M3 6h7l2 2h9v11H3z");
  CHECK(folio::path("folder", true).find('Q') != std::string::npos);
  CHECK(folio::path("pictures", false).find("M3 16l5-5") != std::string::npos);
  CHECK(folio::path("file", false) == "M6 3h8l4 4v14H6zM14 3v4h4");
  CHECK(folio::folder_silhouette(0) == "M4.5 88.5V0.5H46L50 6.5H123.5V88.5Z");
  CHECK(folio::folder_silhouette(6).find("Q4.5 0.5") != std::string::npos);
  for (const char *n : {"actions", "groups", "space", "search", "settings", "pencil", "info", "star", "pin", "plus",
                        "bookmark", "home", "calculator", "save", "tag", "help", "cut", "paste", "undo", "redo",
                        "compress", "extract", "select", "sigma", "terminal", "keyboard", "share"})
    CHECK(folio::has(n) || exists(n));
  CHECK(std::string(license()).find("David Heinemeier Hansson") != std::string::npos);
  CHECK(style() == Style::Files);
  set_style(Style::Folio);
  CHECK(style() == Style::Folio);
  set_style(Style::Files);
}
