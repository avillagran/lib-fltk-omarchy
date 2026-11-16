#include "fltk-omarchy/fit.h"

#include <algorithm>
#include <map>

namespace fo {
namespace fit {

int row_width(int count, int button, int gap) { return count > 0 ? count * button + (count - 1) * gap : 0; }

RowFold fold_row(const std::vector<int> &priority, const std::vector<int> &group, int button, int gap,
                 int available) {
  RowFold out;
  const int n = int(priority.size());
  if (available <= 0 || row_width(n, button, gap) <= available) {
    for (int i = 0; i < n; ++i)
      out.slots.push_back(i);
    return out;
  }
  // Units: a lone item, or every item of one nonzero group.
  struct Unit {
    std::vector<int> items;
    int priority = 0;
  };
  std::vector<Unit> units;
  std::map<int, size_t> by_group;
  for (int i = 0; i < n; ++i) {
    int g = i < int(group.size()) ? group[size_t(i)] : 0;
    if (g != 0) {
      auto it = by_group.find(g);
      if (it != by_group.end()) {
        Unit &u = units[it->second];
        u.items.push_back(i);
        u.priority = std::max(u.priority, priority[size_t(i)]);
        continue;
      }
      by_group[g] = units.size();
    }
    units.push_back(Unit{{i}, priority[size_t(i)]});
  }
  std::vector<size_t> order(units.size());
  for (size_t k = 0; k < order.size(); ++k)
    order[k] = k;
  std::stable_sort(order.begin(), order.end(),
                   [&](size_t a, size_t b) { return units[a].priority > units[b].priority; });
  // Something folds, so the overflow button always takes one slot.
  std::vector<bool> keep(size_t(n), false);
  int kept = 0;
  for (size_t k : order) {
    int size = int(units[k].items.size());
    if (row_width(kept + size + 1, button, gap) > available)
      continue; // a smaller unit further down may still fit
    for (int i : units[k].items)
      keep[size_t(i)] = true;
    kept += size;
  }
  bool placed = false;
  for (int i = 0; i < n; ++i) {
    if (keep[size_t(i)]) {
      out.slots.push_back(i);
      continue;
    }
    out.folded.push_back(i);
    if (!placed) {
      out.slots.push_back(-1);
      placed = true;
    }
  }
  return out;
}

StatusRow status_row(int x, int w, int left, int hint, int right, int badge) {
  StatusRow r;
  const int pad = 16, gap = 24, left_floor = 200, hint_floor = 160;
  const int inner = w - 2 * pad;
  if (inner <= 0)
    return r;
  left = std::max(0, left), hint = std::max(0, hint), right = std::max(0, right), badge = std::max(0, badge);
  const int lmin = std::min({left, left_floor, inner});
  int others = 0; // badge and right text, with their gaps
  if (badge > 0 && lmin + others + gap + badge <= inner) {
    r.badge_w = badge;
    others += gap + badge;
  }
  if (right > 0 && lmin + others + gap + right <= inner) {
    r.right_w = right;
    others += gap + right;
  }
  r.left_w = std::max(lmin, std::min(left, inner - others));
  int room = inner - others - r.left_w - (r.left_w > 0 ? gap : 0);
  if (hint > 0 && room >= hint_floor)
    r.hint_w = std::min(hint, room);
  r.left_x = x + pad;
  if (r.hint_w)
    r.hint_x = r.left_x + r.left_w + (r.left_w > 0 ? gap : 0);
  int end = x + w - pad;
  if (r.badge_w) {
    r.badge_x = end - r.badge_w;
    end = r.badge_x - gap;
  }
  if (r.right_w)
    r.right_x = end - r.right_w;
  return r;
}

CrumbRow crumbs(const std::vector<int> &chips, int available, int fold, int sep, int min_current) {
  CrumbRow r;
  const int n = int(chips.size());
  if (!n)
    return r;
  const int avail = std::max(0, available);
  const int natural = chips[size_t(n - 1)];
  int current = std::min(natural, avail);
  int used = current;
  r.first = n - 1;
  // Ancestors from the nearest, keeping room for "…" while more are hidden.
  while (r.first > 0) {
    int add = chips[size_t(r.first - 1)] + sep;
    int reserve = r.first - 1 > 0 ? fold + sep : 0;
    if (used + add + reserve > avail)
      break;
    used += add;
    --r.first;
  }
  if (r.first > 0) {
    int over = used + fold + sep - avail;
    if (over <= 0) {
      r.fold = true;
    } else if (r.first == n - 1 && current - over >= std::min(natural, min_current)) {
      current -= over; // the current chip gives up room for "…" and stays readable
      r.fold = true;
    }
  }
  for (int i = r.first; i < n - 1; ++i)
    r.widths.push_back(chips[size_t(i)]);
  r.widths.push_back(current);
  return r;
}

} // namespace fit
} // namespace fo
