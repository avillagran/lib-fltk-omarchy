// Smooth scrolling: wheel/touchpad glide to the exact destination (120 ms
// OutCubic, retargeted by every step), a thin scrollbar that appears while
// scrolling or hovered and fades out, and a ScrollArea container.
#pragma once

#include <FL/Fl_Group.H>

#include "fltk-omarchy/anim.h"

namespace fo {

/// Scroll position math shared by every scrolling widget. Accumulates
/// wheel deltas and eases toward the exact destination; reversing
/// direction drops the pending travel. Positions are in pixels, 0..max.
class Scroller {
public:
  /// Adopts a position at once (keyboard, scrollbar drag).
  void jump(double pos);
  /// Adds a wheel step of `delta` pixels (positive = down).
  void wheel(double delta, double max);
  /// Glides to `pos` (or jumps when animations are off).
  void scroll_to(double pos, double max);
  /// Position to draw now, clamped to [0, max]. Stops at the target.
  double position(double max);
  double target() const { return target_; }
  bool running() const { return running_; }
  /// True when a step of `delta` has nowhere to go (nested scrolling
  /// hands the wheel to the parent then).
  bool passes(double delta, double max) const;

private:
  double value_ = 0, from_ = 0, target_ = 0, start_ = 0;
  bool running_ = false;
};

/// The thin overlay scrollbar (Folio's FolioScrollBar): 4 px, rounded with
/// the theme, foreground at 28 % (50 % while dragged). Shows on scroll or
/// hover, fades out ~0.9 s after the last scroll.
class ScrollBar {
public:
  /// Track: the strip on the right edge of the owner (x, y, w, h).
  void place(int x, int y, int w, int h) { x_ = x, y_ = y, w_ = w, h_ = h; }
  /// Marks scrolling activity (shows the bar).
  void poke(Fl_Widget *owner);
  void draw(Fl_Widget *owner, double pos, double max, double view);
  /// Mouse handling. Returns true when it used the event; while dragging
  /// sets `pos` to the new position.
  bool handle(int event, Fl_Widget *owner, double &pos, double max, double view);
  bool dragging() const { return dragging_; }

private:
  bool thumb(double pos, double max, double view, int &ty, int &th) const;
  int x_ = 0, y_ = 0, w_ = 10, h_ = 0;
  double last_ = -1e9;
  bool hovered_ = false, dragging_ = false;
  int grab_dy_ = 0;
  Anim<double> alpha_;
};

/// A vertical scrolling container for one child of any height. The child
/// keeps its own width when narrower; set its height to its content.
///
///   auto *area = new fo::ScrollArea(x, y, w, h);
///   auto *content = new Fl_Group(x, y, w - 10, 2000);   // the content
///   ...; content->end(); area->end();
class ScrollArea : public Fl_Group {
public:
  ScrollArea(int x, int y, int w, int h);
  /// Scroll offset in pixels.
  double offset() const { return pos_; }
  void scroll_to(double offset, bool animate = true);
  /// Scrolls so the content rectangle (y relative to the content top,
  /// height h) is visible.
  void ensure_visible(int content_y, int h, bool animate = true);
  double max_offset() const;
  int handle(int event) override;
  void draw() override;
  void resize(int x, int y, int w, int h) override;

private:
  Fl_Widget *content() const { return children() ? child(0) : nullptr; }
  void place_content();
  Scroller scroller_;
  ScrollBar bar_;
  double pos_ = 0;
};

} // namespace fo
