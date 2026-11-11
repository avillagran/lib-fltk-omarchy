# Animation

Motion follows Omarchy: short transitions on state changes, glides
instead of jumps, nothing that delays input. State changes take effect at
once and the drawing catches up; when animations are off everything is
drawn in its final state.

## On or off

`fo::anim::enabled()` answers, in this order:

1. The app: `anim::set_enabled(1)` / `set_enabled(0)`; `set_enabled(-1)`
   follows the user and the system again.
2. The user: `OMARCHY_FLTK_ANIM=on|off` (also `1|0`, `true|false`,
   `yes|no`; the older `OMARCHY_FLTK_ANIMATIONS=0` still works).
3. The system: Hyprland's `animations:enabled` (the Omarchy Animations
   switch), read over Hyprland's request socket by `anim::watch_system()`
   (called by `fo::init`) and again on every `configreloaded` event.
4. Nothing answered: on.

## Frames

One FLTK timeout drives every motion, at the refresh rate of the focused
monitor (`refreshRate` from `j/monitors`, re-read when monitors change or
the config reloads; 60 Hz without Hyprland). It is armed only by
`anim::request_frame(widget)` or `request_frame(fn, data)` and stops as
soon as a frame passes with nothing requested, so an idle window costs no
CPU. Requests for deleted widgets are dropped safely
(`Fl_Widget_Tracker`).

Widgets use `fo::Anim<T>`:

```cpp
fo::Anim<fo::Rgb> fill_;                       // member
void draw() override {
  fill_.set(hovered ? hot : cold, fo::anim::ms::BUTTON, fo::anim::Ease::Linear);
  fo::Rgb c = fill_.get(this);                 // asks for the next frame while moving
  ...
}
```

`set()` starts from the value on screen, so interrupting a motion never
jumps; the first `set()` and any `set()` while animations are off jump.

## What moves, and for how long

| What | Duration and curve | From |
|---|---|---|
| Button and IconButton fill | 120 ms linear (`ms::BUTTON`) | Omarchy `Ui/Button.qml` |
| Sidebar hover tint sliding between rows | 120 ms OutCubic (2 × `ms::ROW`) | Omarchy `PanelActionButton` (60 ms) |
| Collection hover fade in/out | 120 ms OutCubic | — |
| Collection focus ring sliding to the new item | 120 ms OutCubic (`ms::FOCUS`) | — |
| Wheel / touchpad scrolling | 120 ms OutCubic to the exact destination, retargeted by every step, reversal drops pending travel (`ms::SCROLL`) | Rust toolkit `scroll.rs` |
| Scrollbar | shows on scroll or hover (90 ms), fades out 0.9 s after the last scroll (200 ms) | Folio `FolioScrollBar` (appears/disappears) |
| SplitPane `set_split(px, true)`, `collapse_second` | 190 ms OutCubic (`ms::LAYOUT`) | o3 `PinnedKeyboardPanel` |
| Dialog and help: scrim and card fade in, card grows from 97 % | 180 ms Omarchy easeOut (`ms::POPUP_IN`) | Omarchy `Ui/PopupCard.qml` |
| Dialog and help: fade and shrink out on close | 120 ms easeOut (`ms::POPUP_OUT`), from where the fade-in was | Omarchy `PopupCard` |
| Toast in / out | 180 ms easeOut / 120 ms InCubic, rising 8 px | Omarchy `PopupCard` |
| Tooltip | after 600 ms of hover, no fade (`ms::TOOLTIP_DELAY`) | Folio `IconButton` |
| Menus and submenus | no animation (Folio); submenus open after 200 ms of hover intent (`ms::SUBMENU_OPEN`), at once from the keyboard | Folio `FolioMenu`, GTK menu delay |
| Submenu aim | moving diagonally toward an open submenu keeps it open; settling elsewhere takes over after 300 ms (`ms::SUBMENU_AIM`) | Qt/GTK safe triangle, Rust toolkit `popups.rs` |
| Type-ahead buffer | resets after 1 s (`ms::TYPEAHEAD`) | — |

Layers that fade in (`Layer::fade_in()`, read with `presence()` in
`draw()`) fade out on `close()`: the layer leaves `top_layer()`, gives the
keyboard back and stops taking events at once, keeps drawing while
`presence()` falls to 0, and is deleted when it reaches it. The scale
(`Layer::card_scale`) needs cairo (Wayland, or FLTK built with cairo on
X11); elsewhere the card only fades.

Easing curves: `anim::Ease::EaseOut` is Omarchy's `Style.easeOut`, the
cubic Bézier (0.23, 1, 0.32, 1): it starts at once and settles without a
bounce. The others are Qt's curves of the same names.

## Tests

`anim::freeze()`, `anim::advance(ms)` and `anim::resume()` drive the clock
by hand, so tweens and the scroller are tested without waiting.
