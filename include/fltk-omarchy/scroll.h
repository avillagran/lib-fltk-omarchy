// Smooth scrolling: wheel/touchpad glide to the exact destination (120 ms
// OutCubic, retargeted by every step), a thin scrollbar (vertical or
// horizontal) that appears while scrolling or hovered and fades out, and a
// ScrollArea container that scrolls vertically, horizontally or both.
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
/// hover, fades out ~0.9 s after the last scroll. Vertical by default (a
/// strip on the right edge); horizontal bars run along the bottom edge.
class ScrollBar {
public:
  enum Orientation { Vertical, Horizontal };
  ScrollBar() = default;
  explicit ScrollBar(Orientation o) : o_(o) {}
  void orientation(Orientation o) { o_ = o; }
  Orientation orientation() const { return o_; }
  /// Track: the strip on the right edge (vertical) or the bottom edge
  /// (horizontal) of the owner (x, y, w, h).
  void place(int x, int y, int w, int h) { x_ = x, y_ = y, w_ = w, h_ = h; }
  /// Marks scrolling activity (shows the bar).
  void poke(Fl_Widget *owner);
  void draw(Fl_Widget *owner, double pos, double max, double view);
  /// Mouse handling. Returns true when it used the event; while dragging
  /// sets `pos` to the new position.
  bool handle(int event, Fl_Widget *owner, double &pos, double max, double view);
  bool dragging() const { return dragging_; }

private:
  /// Thumb start and length along the track's axis.
  bool thumb(double pos, double max, double view, int &ty, int &th) const;
  int start() const { return o_ == Vertical ? y_ : x_; }
  int length() const { return o_ == Vertical ? h_ : w_; }
  Orientation o_ = Vertical;
  int x_ = 0, y_ = 0, w_ = 10, h_ = 0;
  double last_ = -1e9;
  bool hovered_ = false, dragging_ = false;
  int grab_dy_ = 0;
  Anim<double> alpha_;
};

/// A scrolling container for one child. Vertical by default: the child
/// follows the area's width and keeps its own height (set it to the
/// content's height). Horizontal: the child follows the area's height and
/// keeps its own width. Both: the child keeps its own size and scrolls
/// either way.
///
///   auto *area = new fo::ScrollArea(x, y, w, h);
///   auto *content = new Fl_Group(x, y, w - 10, 2000);   // the content
///   ...; content->end(); area->end();
///
///   area->axes(fo::ScrollArea::Horizontal);              // a strip of columns
///
/// Wheel: vertical steps scroll vertically (horizontally in a
/// Horizontal-only area), horizontal steps (touchpads, tilt wheels) and
/// Shift+wheel scroll horizontally. A nested scroller under the pointer
/// gets the wheel first.
class ScrollArea : public Fl_Group {
public:
  enum Axes { Vertical = 1, Horizontal = 2, Both = 3 };
  ScrollArea(int x, int y, int w, int h);
  /// Which ways the content scrolls (default Vertical).
  void axes(Axes a);
  Axes axes() const { return axes_; }

  /// Vertical scroll offset in pixels.
  double offset() const { return pos_; }
  void scroll_to(double offset, bool animate = true);
  /// Scrolls so the content rectangle (y relative to the content top,
  /// height h) is visible.
  void ensure_visible(int content_y, int h, bool animate = true);
  double max_offset() const;

  /// Horizontal scroll offset in pixels.
  double offset_x() const { return pos_x_; }
  void scroll_to_x(double offset, bool animate = true);
  /// Scrolls so the content columns [content_x, content_x + w) are visible.
  void ensure_visible_x(int content_x, int w, bool animate = true);
  double max_offset_x() const;
  /// The offsets the view is gliding to (equal to the offsets at rest).
  double target_offset() const { return scroller_.target(); }
  double target_offset_x() const { return scroller_x_.target(); }

  int handle(int event) override;
  void draw() override;
  void resize(int x, int y, int w, int h) override;

protected:
  Fl_Widget *content() const { return children() ? child(0) : nullptr; }
  /// Moves the content to the current offsets.
  void place_content();
  /// Drawn over the content, under the scrollbars (clipped to the area).
  virtual void draw_over_content() {}

private:
  void place_bars();
  bool wheel(double dx, double dy);
  Axes axes_ = Vertical;
  Scroller scroller_, scroller_x_;
  ScrollBar bar_, bar_x_{ScrollBar::Horizontal};
  double pos_ = 0, pos_x_ = 0;
};

} // namespace fo
