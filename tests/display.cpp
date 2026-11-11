#include "display.h"

#include <FL/Enumerations.H>
#include <FL/Fl.H>
#include <FL/Fl_Window.H>
#include <FL/fl_draw.H>
#include <FL/platform.H>

#include <cstdio>
#include <cstdlib>

#if FLTK_USE_X11
#include <X11/Xlib.h>
#include <X11/keysym.h>
#ifdef FO_HAVE_XTEST
#include <X11/extensions/XTest.h>
#endif
#endif

namespace fo_test {

bool have_display() {
  static int state = -1;
  if (state < 0) {
    const char *d = std::getenv("DISPLAY");
    state = d && *d ? 1 : 0;
    if (state) {
      fl_open_display();
#if FLTK_USE_X11
      state = fl_x11_display() ? 1 : 0;
#endif
    }
  }
  if (!state)
    std::printf("  skip: no display (run through ctest's private Xvfb)\n");
  return state == 1;
}

void settle(double seconds) {
  for (double t = 0; t < seconds; t += 0.01)
    Fl::wait(0.01);
  Fl::flush();
#if FLTK_USE_X11
  if (fl_x11_display())
    XSync(fl_x11_display(), False);
#endif
  Fl::check();
}

void show(Fl_Window *w) {
  w->show();
  for (int i = 0; i < 200 && !w->visible(); ++i)
    Fl::wait(0.01);
  settle(0.2);
}

fo::Rgb pixel(Fl_Window *w, int x, int y) {
  Fl::flush();
  w->make_current();
  uchar *p = fl_read_image(nullptr, x, y, 1, 1);
  fo::Rgb c;
  if (p) {
    c = fo::Rgb(p[0], p[1], p[2]);
    delete[] p;
  }
  return c;
}

bool near(fo::Rgb a, fo::Rgb b, int tol) {
  return std::abs(int(a.r) - int(b.r)) <= tol && std::abs(int(a.g) - int(b.g)) <= tol &&
         std::abs(int(a.b) - int(b.b)) <= tol;
}

bool tinted(fo::Rgb c, fo::Rgb bg, fo::Rgb tint) {
  // Project c - bg on tint - bg.
  double dx = tint.r - bg.r, dy = tint.g - bg.g, dz = tint.b - bg.b;
  double len2 = dx * dx + dy * dy + dz * dz;
  if (len2 <= 0)
    return false;
  double ux = c.r - bg.r, uy = c.g - bg.g, uz = c.b - bg.b;
  double t = (ux * dx + uy * dy + uz * dz) / len2;
  double ox = ux - t * dx, oy = uy - t * dy, oz = uz - t * dz;
  return t >= 0.33 && ox * ox + oy * oy + oz * oz < 0.1 * len2;
}

bool have_xtest() {
#if FLTK_USE_X11 && defined(FO_HAVE_XTEST)
  int ev, err, major, minor;
  return fl_x11_display() && XTestQueryExtension(fl_x11_display(), &ev, &err, &major, &minor);
#else
  return false;
#endif
}

void focus_window(Fl_Window *w) {
#if FLTK_USE_X11
  if (Display *d = fl_x11_display()) {
    XSetInputFocus(d, fl_x11_xid(w), RevertToParent, CurrentTime);
    XSync(d, False);
  }
#endif
  settle(0.15);
}

void press_key(int fl_key) {
#if FLTK_USE_X11 && defined(FO_HAVE_XTEST)
  if (Display *d = fl_x11_display()) {
    KeySym sym = fl_key == FL_Down    ? XK_Down
                 : fl_key == FL_Up    ? XK_Up
                 : fl_key == FL_Left  ? XK_Left
                 : fl_key == FL_Right ? XK_Right
                 : fl_key == FL_Enter ? XK_Return
                 : fl_key == FL_Escape ? XK_Escape
                                       : KeySym(fl_key);
    KeyCode code = XKeysymToKeycode(d, sym);
    XTestFakeKeyEvent(d, code, True, CurrentTime);
    XTestFakeKeyEvent(d, code, False, CurrentTime);
    XSync(d, False);
  }
#else
  (void)fl_key;
#endif
  settle(0.15);
}

void move_pointer(Fl_Window *w, int x, int y) {
#if FLTK_USE_X11 && defined(FO_HAVE_XTEST)
  if (Display *d = fl_x11_display()) {
    XTestFakeMotionEvent(d, -1, w->x_root() + x, w->y_root() + y, CurrentTime);
    XSync(d, False);
  }
#else
  (void)w, (void)x, (void)y;
#endif
  settle(0.05);
}

void click(Fl_Window *w, int x, int y) {
  move_pointer(w, x, y);
#if FLTK_USE_X11 && defined(FO_HAVE_XTEST)
  if (Display *d = fl_x11_display()) {
    XTestFakeButtonEvent(d, 1, True, CurrentTime);
    XTestFakeButtonEvent(d, 1, False, CurrentTime);
    XSync(d, False);
  }
#endif
  settle(0.1);
}

void drag(Fl_Window *w, int x0, int y0, int x1, int y1, int steps) {
  move_pointer(w, x0, y0);
#if FLTK_USE_X11 && defined(FO_HAVE_XTEST)
  if (Display *d = fl_x11_display()) {
    const int rx = w->x_root(), ry = w->y_root();
    XTestFakeButtonEvent(d, 1, True, CurrentTime);
    XSync(d, False);
    settle(0.05);
    for (int i = 1; i <= steps; ++i) {
      XTestFakeMotionEvent(d, -1, rx + x0 + (x1 - x0) * i / steps, ry + y0 + (y1 - y0) * i / steps,
                           CurrentTime);
      XSync(d, False);
      settle(0.03);
    }
    XTestFakeButtonEvent(d, 1, False, CurrentTime);
    XSync(d, False);
  }
#else
  (void)x1, (void)y1, (void)steps;
#endif
  settle(0.1);
}

} // namespace fo_test
