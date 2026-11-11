#include "fltk-omarchy/anim.h"

#include "fltk-omarchy/hyprland.h"
#include "fltk-omarchy/platform.h"

#include <FL/Fl.H>
#include <FL/Fl_Widget.H>

#include <chrono>
#include <cmath>
#include <cstring>
#include <memory>
#include <vector>

namespace fo {
namespace anim {

namespace {

struct Clock {
  bool frozen = false;
  double at = 0;
};
Clock &clock_state() {
  static Clock c;
  return c;
}

double real_ms() {
  using namespace std::chrono;
  static const steady_clock::time_point origin = steady_clock::now();
  return duration<double, std::milli>(steady_clock::now() - origin).count();
}

struct Switch {
  int app = -1;    // set_enabled
  int env = -2;    // -2: not read yet
  int system = -1; // Hyprland animations:enabled
  double hz = 60;
  int sub = 0;
};
Switch &sw() {
  static Switch s;
  return s;
}

} // namespace

double now_ms() {
  Clock &c = clock_state();
  return c.frozen ? c.at : real_ms();
}
void freeze() {
  Clock &c = clock_state();
  c.at = real_ms();
  c.frozen = true;
}
void advance(double ms) {
  Clock &c = clock_state();
  if (!c.frozen)
    freeze();
  c.at += ms;
}
void resume() { clock_state().frozen = false; }

double bezier(double x1, double y1, double x2, double y2, double x) {
  if (x <= 0 || x >= 1)
    return x <= 0 ? 0 : 1;
  auto c = [](double a, double b, double s) {
    double u = 1 - s;
    return 3 * u * u * s * a + 3 * u * s * s * b + s * s * s;
  };
  auto dc = [](double a, double b, double s) {
    double u = 1 - s;
    return 3 * u * u * a + 6 * u * s * (b - a) + 3 * s * s * (1 - b);
  };
  // Newton on x(s) = x, bisection when the slope is flat.
  double s = x;
  for (int i = 0; i < 8; ++i) {
    double err = c(x1, x2, s) - x;
    if (std::fabs(err) < 1e-6)
      return c(y1, y2, s);
    double d = dc(x1, x2, s);
    if (std::fabs(d) < 1e-6)
      break;
    s = std::min(1.0, std::max(0.0, s - err / d));
  }
  double lo = 0, hi = 1;
  s = x;
  for (int i = 0; i < 40; ++i) {
    double v = c(x1, x2, s);
    if (std::fabs(v - x) < 1e-7)
      break;
    (v < x ? lo : hi) = s;
    s = (lo + hi) / 2;
  }
  return c(y1, y2, s);
}

double ease(Ease e, double t) {
  t = t < 0 ? 0 : t > 1 ? 1 : t;
  switch (e) {
  case Ease::Linear: return t;
  case Ease::OutCubic: return 1 - std::pow(1 - t, 3);
  case Ease::InCubic: return t * t * t;
  case Ease::InOutCubic: return t < 0.5 ? 4 * t * t * t : 1 - std::pow(-2 * t + 2, 3) / 2;
  case Ease::OutQuint: return 1 - std::pow(1 - t, 5);
  case Ease::OutBack: {
    const double s = 1.70158;
    double u = t - 1;
    return 1 + u * u * ((s + 1) * u + s);
  }
  case Ease::EaseOut: return bezier(0.23, 1.0, 0.32, 1.0, t);
  }
  return t;
}

int parse_switch(const char *text) {
  if (!text)
    return -1;
  std::string v;
  for (const char *p = text; *p; ++p)
    v += char(*p >= 'A' && *p <= 'Z' ? *p + 32 : *p);
  while (!v.empty() && (v.back() == ' ' || v.back() == '\n'))
    v.pop_back();
  if (v == "1" || v == "on" || v == "true" || v == "yes")
    return 1;
  if (v == "0" || v == "off" || v == "false" || v == "no")
    return 0;
  return -1;
}

bool enabled() {
  Switch &s = sw();
  if (s.app >= 0)
    return s.app == 1;
  if (s.env == -2) {
    s.env = parse_switch(platform::env("OMARCHY_FLTK_ANIM").c_str());
    if (s.env < 0 && platform::env("OMARCHY_FLTK_ANIMATIONS") == "0")
      s.env = 0;
  }
  if (s.env >= 0)
    return s.env == 1;
  if (s.system >= 0)
    return s.system == 1;
  return true;
}

void set_enabled(int on) { sw().app = on < 0 ? -1 : on ? 1 : 0; }

double refresh_hz() { return sw().hz; }
void set_refresh_hz(double hz) { sw().hz = hz >= 20 && hz <= 1000 ? hz : 60; }

static void read_system() {
  hypr::request("j/getoption animations:enabled", [](bool ok, const std::string &reply) {
    bool on;
    if (ok && hypr::parse_option_bool(reply, on))
      sw().system = on ? 1 : 0;
  });
  hypr::request("j/monitors", [](bool ok, const std::string &reply) {
    double hz = ok ? hypr::parse_refresh_rate(reply) : 0;
    if (hz > 0)
      set_refresh_hz(hz);
  });
}

void watch_system() {
  if (!hypr::available())
    return;
  read_system();
  if (!sw().sub)
    sw().sub = hypr::subscribe([](const std::string &ev, const std::string &) {
      if (ev == "configreloaded" || ev == "monitoradded" || ev == "monitorremoved")
        read_system();
    });
}

// ---------------------------------------------------------------------------
// Frame scheduling: one timer, at the refresh rate, only while requested.

namespace {

struct Frames {
  std::vector<std::unique_ptr<Fl_Widget_Tracker>> widgets;
  std::vector<std::pair<void (*)(void *), void *>> calls;
  bool armed = false;
};
Frames &frames() {
  static Frames f;
  return f;
}

void tick(void *) {
  Frames &f = frames();
  f.armed = false;
  auto widgets = std::move(f.widgets);
  auto calls = std::move(f.calls);
  f.widgets.clear();
  f.calls.clear();
  for (auto &c : calls)
    c.first(c.second);
  for (auto &t : widgets)
    if (!t->deleted())
      t->widget()->redraw();
}

void arm() {
  Frames &f = frames();
  if (f.armed)
    return;
  f.armed = true;
  Fl::add_timeout(1.0 / refresh_hz(), tick);
}

} // namespace

void request_frame(Fl_Widget *w) {
  if (!w)
    return;
  Frames &f = frames();
  for (auto &t : f.widgets)
    if (t->widget() == w)
      return arm();
  f.widgets.emplace_back(new Fl_Widget_Tracker(w));
  arm();
}

void request_frame(void (*fn)(void *), void *data) {
  frames().calls.emplace_back(fn, data);
  arm();
}

bool busy() { return frames().armed; }

} // namespace anim
} // namespace fo
