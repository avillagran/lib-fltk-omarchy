// Helpers for tests that need a display (the private Xvfb of
// tests/run-xvfb.sh).
#pragma once

#include "fltk-omarchy/theme.h"

class Fl_Window;

namespace fo_test {

/// True when DISPLAY is set and FLTK could open it; prints a skip notice
/// otherwise.
bool have_display();
/// Shows a window and waits until it is mapped and drawn.
void show(Fl_Window *w);
/// Runs the FLTK loop for a while so pending redraws and X events land.
void settle(double seconds = 0.15);
/// The color of one window pixel, as on screen.
fo::Rgb pixel(Fl_Window *w, int x, int y);
/// True when every channel is within `tolerance`.
bool near(fo::Rgb a, fo::Rgb b, int tolerance);
/// True when `c` lies clearly between `bg` and `tint` (at least a third of
/// the way), e.g. a hairline in the accent blended with the background.
bool tinted(fo::Rgb c, fo::Rgb bg, fo::Rgb tint);
/// Real input through the X server (XTest): available?
bool have_xtest();
/// Gives a window the X input focus (as a window manager would).
void focus_window(Fl_Window *w);
/// Presses and releases a key (an FLTK key code such as FL_Down).
void press_key(int fl_key);
/// Moves the pointer to window coordinates of `w`.
void move_pointer(Fl_Window *w, int x, int y);

} // namespace fo_test
