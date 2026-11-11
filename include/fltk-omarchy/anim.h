// Motion: Omarchy's durations and curves, retargetable tweens, and one
// shared frame timer that runs at the monitor's refresh rate only while
// something moves (an idle window costs no CPU).
//
// Motion never delays input: state changes take effect at once and the
// drawing catches up. When animations are off everything jumps to its
// final value.
//
//   fo::Anim<fo::Rgb> fill;                       // a member
//   fill.set(hovered ? hot : cold, fo::anim::ms::BUTTON);
//   ... in draw():  fo::Rgb c = fill.get(this);   // schedules the next frame
#pragma once

#include "fltk-omarchy/theme.h"

class Fl_Widget;

namespace fo {
namespace anim {

/// Monotonic milliseconds (frozen time in tests, see freeze()).
double now_ms();
/// Test clock: freeze at the current time, advance by hand, resume.
void freeze();
void advance(double ms);
void resume();

/// Easing curves used by Omarchy and o3.
enum class Ease {
  Linear,
  OutCubic,   ///< positions and sizes (Qt Easing.OutCubic)
  InCubic,    ///< leaving
  InOutCubic, ///< moves between two resting places
  OutQuint,
  OutBack,    ///< settles with a small overshoot (Qt's 1.70158)
  EaseOut,    ///< Omarchy Style.easeOut: cubic Bézier (0.23, 1, 0.32, 1)
};
/// Eased progress for t in [0, 1] (clamped).
double ease(Ease e, double t);
/// CSS/Qt cubic Bézier timing function through (x1,y1), (x2,y2).
double bezier(double x1, double y1, double x2, double y2, double x);

/// Durations in milliseconds, from Omarchy's shell and o3 (see
/// docs/ANIMATION.md for where each comes from).
namespace ms {
constexpr int BUTTON = 120;       ///< button color (Omarchy Ui/Button.qml)
constexpr int ROW = 60;           ///< row hover/press (PanelActionButton)
constexpr int FOCUS = 120;        ///< focus ring sliding between items
constexpr int LAYOUT = 190;       ///< layout changes, split panes
constexpr int POPUP_IN = 180;     ///< dialog/toast appearing (PopupCard)
constexpr int POPUP_OUT = 120;    ///< dialog/toast going
constexpr int INDICATOR = 160;    ///< selection indicators sliding
constexpr int REVEAL = 200;       ///< sections opening/closing
constexpr int SCROLL = 120;       ///< wheel glide to the exact destination
constexpr int SCROLLBAR_HIDE = 900; ///< scrollbar stays this long after scrolling
constexpr int TOOLTIP_DELAY = 600;  ///< hover before a tooltip shows
constexpr int TYPEAHEAD = 1000;     ///< type-ahead buffer resets after this
constexpr int SUBMENU_OPEN = 200;   ///< hover intent before a submenu opens
constexpr int SUBMENU_AIM = 300;    ///< grace while the pointer heads for an open submenu
} // namespace ms

/// Whether motion is on. Order: set_enabled() override, then
/// OMARCHY_FLTK_ANIM=on|off (1|0, true|false), then Hyprland's
/// `animations:enabled` (read at watch_system() and on configreloaded),
/// then on.
bool enabled();
/// -1 follows the environment/system again; 0 off; 1 on.
void set_enabled(int on);
/// 1 / 0 for on/off spellings, -1 for anything else.
int parse_switch(const char *text);

/// Reads Hyprland's `animations:enabled` and the focused monitor's refresh
/// rate (asynchronously) and re-reads them on `configreloaded`.
void watch_system();

/// Frames per second used by the frame timer (default 60).
double refresh_hz();
void set_refresh_hz(double hz);

/// Redraws `w` on the next frame. Safe if `w` is deleted meanwhile.
void request_frame(Fl_Widget *w);
/// Runs `fn(data)` on the next frame (once).
void request_frame(void (*fn)(void *), void *data);
/// True while any frame is pending.
bool busy();

} // namespace anim

/// A rectangle in floating point, for animated geometry.
struct RectF {
  double x = 0, y = 0, w = 0, h = 0;
  bool operator==(const RectF &o) const { return x == o.x && y == o.y && w == o.w && h == o.h; }
  bool operator!=(const RectF &o) const { return !(*this == o); }
};

inline double lerp(double a, double b, double t) { return a + (b - a) * t; }
inline Rgb lerp(Rgb a, Rgb b, double t) { return mix(a, b, t); }
inline RectF lerp(const RectF &a, const RectF &b, double t) {
  return RectF{lerp(a.x, b.x, t), lerp(a.y, b.y, t), lerp(a.w, b.w, t), lerp(a.h, b.h, t)};
}

/// A retargetable tween of T (double, Rgb, RectF; anything with lerp()).
/// set() starts from what is on screen now, so interrupting a motion never
/// jumps. The first set() (or any set() while animations are off) jumps.
template <class T> class Anim {
public:
  Anim() = default;
  explicit Anim(const T &v) : from_(v), to_(v), has_(true) {}

  /// Moves toward `target` over `ms` milliseconds.
  void set(const T &target, int ms, anim::Ease e = anim::Ease::OutCubic) {
    if (has_ && target == to_)
      return;
    if (!has_ || ms <= 0 || !anim::enabled()) {
      jump(target);
      return;
    }
    from_ = value();
    to_ = target;
    start_ = anim::now_ms();
    ms_ = ms;
    ease_ = e;
    running_ = true;
  }
  /// Sets the value at once.
  void jump(const T &v) {
    from_ = to_ = v;
    has_ = true;
    running_ = false;
  }
  /// Current value (stops the motion when it has arrived).
  T value() {
    if (!running_)
      return to_;
    double t = (anim::now_ms() - start_) / ms_;
    if (t >= 1 || !anim::enabled()) {
      running_ = false;
      return to_;
    }
    return lerp(from_, to_, anim::ease(ease_, t < 0 ? 0 : t));
  }
  /// Current value; while still moving, schedules a redraw of `w`.
  T get(Fl_Widget *w) {
    T v = value();
    if (running_)
      anim::request_frame(w);
    return v;
  }
  bool running() const { return running_; }
  bool initialized() const { return has_; }
  const T &target() const { return to_; }

private:
  T from_{}, to_{};
  double start_ = 0, ms_ = 1;
  anim::Ease ease_ = anim::Ease::OutCubic;
  bool has_ = false, running_ = false;
};

} // namespace fo
