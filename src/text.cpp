#include "fltk-omarchy/text.h"

#include "fltk-omarchy/theme.h"

#include <FL/Fl.H>
#include <FL/fl_draw.H>
#include <FL/fl_utf8.h>

#include <string>

namespace fo {
namespace text {

void apply_fonts(const std::string &family) {
  // Fl::set_font keeps the pointer: keep the strings alive for good.
  static std::string names[12];
  const char *suffix[12] = {"", " Bold", " Italic", " Bold Italic", "", " Bold", " Italic",
                            " Bold Italic", " Medium", " SemiBold", " Medium Italic", " SemiBold Italic"};
  const Fl_Font faces[12] = {FL_HELVETICA, FL_HELVETICA_BOLD, FL_HELVETICA_ITALIC, FL_HELVETICA_BOLD_ITALIC,
                             FL_COURIER, FL_COURIER_BOLD, FL_COURIER_ITALIC, FL_COURIER_BOLD_ITALIC,
                             FL_TIMES, FL_TIMES_BOLD, FL_TIMES_ITALIC, FL_TIMES_BOLD_ITALIC};
  for (int i = 0; i < 12; ++i) {
    std::string n = family + suffix[i];
    if (names[i] == n)
      continue;
    names[i] = n;
    Fl::set_font(faces[i], names[i].c_str());
  }
  FL_NORMAL_SIZE = size::BODY;
}

Measure fltk_measure(Fl_Font font, int sz) {
  return [font, sz](const char *t, int n) {
    fl_font(font, sz);
    return fl_width(t, n);
  };
}

std::vector<int> utf8_offsets(const std::string &s) {
  std::vector<int> o;
  o.reserve(s.size() + 1);
  for (size_t i = 0; i < s.size(); ++i)
    if ((static_cast<unsigned char>(s[i]) & 0xC0) != 0x80)
      o.push_back(int(i));
  o.push_back(int(s.size()));
  return o;
}

static const char kEllipsis[] = "\xE2\x80\xA6"; // …

std::string elide_end(const std::string &s, double max_w, const Measure &m) {
  if (m(s.c_str(), int(s.size())) <= max_w)
    return s;
  std::vector<int> o = utf8_offsets(s);
  int chars = int(o.size()) - 1;
  double ew = m(kEllipsis, 3);
  // Largest prefix whose width plus the ellipsis fits.
  int lo = 0, hi = chars;
  while (lo < hi) {
    int mid = (lo + hi + 1) / 2;
    if (m(s.c_str(), o[size_t(mid)]) + ew <= max_w)
      lo = mid;
    else
      hi = mid - 1;
  }
  std::string head = s.substr(0, size_t(o[size_t(lo)]));
  while (!head.empty() && head.back() == ' ')
    head.pop_back();
  return head + kEllipsis;
}

std::string elide_middle(const std::string &s, double max_w, const Measure &m) {
  if (m(s.c_str(), int(s.size())) <= max_w)
    return s;
  std::vector<int> o = utf8_offsets(s);
  int chars = int(o.size()) - 1;
  // Keep a short extension (".jpg", ".tar.gz" counts its last part) whole.
  int ext = 0;
  size_t dot = s.rfind('.');
  if (dot != std::string::npos && dot > 0 && s.size() - dot <= 6) {
    for (size_t i = 0; i + 1 < o.size(); ++i)
      if (size_t(o[i]) >= dot)
        ++ext;
  }
  double ew = m(kEllipsis, 3);
  auto fits = [&](int head, int tail) {
    std::string t = s.substr(0, size_t(o[size_t(head)])) + kEllipsis +
                    s.substr(size_t(o[size_t(chars - tail)]));
    return m(t.c_str(), int(t.size())) <= max_w;
  };
  (void)ew;
  // Total kept characters k: tail gets the extension plus ~1/3 of the rest.
  auto split = [&](int k, int &head, int &tail) {
    int rest = k - ext;
    if (rest < 0) {
      tail = k;
      head = 0;
      return;
    }
    tail = ext + rest / 3;
    head = k - tail;
  };
  int lo = 0, hi = chars - 1, best = -1;
  while (lo <= hi) {
    int mid = (lo + hi) / 2, h, t;
    split(mid, h, t);
    if (fits(h, t)) {
      best = mid;
      lo = mid + 1;
    } else {
      hi = mid - 1;
    }
  }
  if (best < 0)
    return elide_end(s, max_w, m);
  int h, t;
  split(best, h, t);
  if (h == 0)
    return elide_end(s, max_w, m);
  return s.substr(0, size_t(o[size_t(h)])) + kEllipsis + s.substr(size_t(o[size_t(chars - t)]));
}

std::vector<std::string> wrap(const std::string &s, double max_w, const Measure &m, int max_lines) {
  std::vector<std::string> lines;
  std::string rest = s;
  while (!rest.empty() && int(lines.size()) < max_lines) {
    if (int(lines.size()) == max_lines - 1 || m(rest.c_str(), int(rest.size())) <= max_w) {
      lines.push_back(elide_end(rest, max_w, m));
      break;
    }
    std::vector<int> o = utf8_offsets(rest);
    int chars = int(o.size()) - 1;
    int lo = 1, hi = chars;
    while (lo < hi) {
      int mid = (lo + hi + 1) / 2;
      if (m(rest.c_str(), o[size_t(mid)]) <= max_w)
        lo = mid;
      else
        hi = mid - 1;
    }
    int cut = o[size_t(lo)];
    // Prefer breaking after a space or punctuation in the last 60 %.
    bool at_space = size_t(cut) < rest.size() && rest[size_t(cut)] == ' ';
    for (int i = lo; !at_space && i > lo * 2 / 5; --i) {
      char c = rest[size_t(o[size_t(i)] - 1)];
      if (c == ' ' || c == '-' || c == '_' || c == '.') {
        cut = o[size_t(i)];
        break;
      }
    }
    std::string line = rest.substr(0, size_t(cut));
    while (!line.empty() && line.back() == ' ')
      line.pop_back();
    lines.push_back(line);
    rest = rest.substr(size_t(cut));
    while (!rest.empty() && rest[0] == ' ')
      rest.erase(0, 1);
  }
  return lines;
}

std::string upper(const std::string &s) {
  std::string out;
  out.reserve(s.size());
  const char *p = s.c_str(), *end = p + s.size();
  while (p < end) {
    int len = 0;
    unsigned cp = fl_utf8decode(p, end, &len);
    if (len <= 0)
      len = 1;
    char buf[8];
    int n = fl_utf8encode(unsigned(fl_toupper(cp)), buf);
    out.append(buf, size_t(n));
    p += len;
  }
  return out;
}

double width(const std::string &s, Fl_Font font, int sz) {
  fl_font(font, sz);
  return fl_width(s.c_str(), int(s.size()));
}

void draw(const std::string &s, int x, int y, int w, int h, Rgb color, Fl_Font font, int sz, Fl_Align align,
          Elide elide) {
  if (w <= 0 || s.empty())
    return;
  fl_font(font, sz);
  std::string t = s;
  Measure m = [](const char *p, int n) { return fl_width(p, n); };
  if (elide == Elide::End)
    t = elide_end(s, w, m);
  else if (elide == Elide::Middle)
    t = elide_middle(s, w, m);
  double tw = fl_width(t.c_str(), int(t.size()));
  int tx = x;
  if (align & FL_ALIGN_RIGHT)
    tx = x + w - int(tw + 0.5);
  else if (!(align & FL_ALIGN_LEFT))
    tx = x + (w - int(tw + 0.5)) / 2;
  int base = y + (h + fl_height()) / 2 - fl_descent();
  fl_color(color.fl());
  fl_draw(t.c_str(), int(t.size()), tx, base);
}

int width_spaced(const std::string &s, double spacing, Fl_Font font, int sz) {
  fl_font(font, sz);
  std::vector<int> o = utf8_offsets(s);
  double w = 0;
  for (size_t i = 0; i + 1 < o.size(); ++i)
    w += fl_width(s.c_str() + o[i], o[i + 1] - o[i]) + (i + 2 < o.size() ? spacing : 0);
  return int(w + 0.5);
}

int draw_spaced(const std::string &s, int x, int baseline, double spacing, Rgb color, Fl_Font font, int sz) {
  fl_font(font, sz);
  fl_color(color.fl());
  std::vector<int> o = utf8_offsets(s);
  double cx = x;
  for (size_t i = 0; i + 1 < o.size(); ++i) {
    int n = o[i + 1] - o[i];
    fl_draw(s.c_str() + o[i], n, float(cx), float(baseline));
    cx += fl_width(s.c_str() + o[i], n) + spacing;
  }
  return int(cx - spacing - x + 0.5);
}

} // namespace text
} // namespace fo
