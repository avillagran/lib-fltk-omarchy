#include "fltk-omarchy/window.h"

#include "fltk-omarchy/anim.h"
#include "fltk-omarchy/app.h"
#include "fltk-omarchy/i18n.h"
#include "fltk-omarchy/icons.h"
#include "fltk-omarchy/paint.h"
#include "fltk-omarchy/theme.h"

#include <FL/Fl.H>
#include <FL/fl_draw.H>

#include <algorithm>

namespace fo {

// ---------------------------------------------------------------------------
// App setup and keyboard-focus tracking.

namespace {
bool g_focus_visible = false;
bool g_initialized = false;

int dispatch(int event, Fl_Window *w) {
  if (event == FL_KEYDOWN || event == FL_SHORTCUT) {
    int k = Fl::event_key();
    bool modifier_only = k == FL_Shift_L || k == FL_Shift_R || k == FL_Control_L || k == FL_Control_R ||
                         k == FL_Alt_L || k == FL_Alt_R || k == FL_Meta_L || k == FL_Meta_R;
    if (!modifier_only && !g_focus_visible) {
      g_focus_visible = true;
      if (w)
        w->redraw();
    }
  } else if (event == FL_PUSH && g_focus_visible) {
    g_focus_visible = false;
    if (w)
      w->redraw();
  }
  return Fl::handle_(event, w);
}
} // namespace

bool focus_visible() { return g_focus_visible; }
void set_focus_visible(bool on) { g_focus_visible = on; }

void init(const InitOptions &o) {
  if (g_initialized)
    return;
  g_initialized = true;
  Fl::lock(); // Fl::awake() for fo::tasks
  Fl::option(Fl::OPTION_ARROW_FOCUS, false);
  Fl::option(Fl::OPTION_VISIBLE_FOCUS, true);
  Fl::scheme("none");
  if (!Fl::event_dispatch())
    Fl::event_dispatch(dispatch);
  if (!o.app_class.empty()) {
    static std::string cls = o.app_class;
    Fl_Window::default_xclass(cls.c_str());
  }
  if (o.watch_theme)
    watch_theme(o.theme_dir);
  else
    set_theme(theme());
  if (o.watch_language)
    i18n::watch(o.language_file);
  if (o.watch_hyprland)
    anim::watch_system();
  on_theme_change([] { icon::clear_cache(); });
}

// ---------------------------------------------------------------------------
// Layers.

Layer::Layer() : Fl_Group(0, 0, 10, 10) {
  end();
  box(FL_NO_BOX);
}

Layer::~Layer() {
  Fl::remove_timeout(fade_step, this);
  if (fade_host_) {
    auto &f = fade_host_->fading_;
    f.erase(std::remove(f.begin(), f.end(), this), f.end());
  }
  delete restore_;
}

void Layer::fade_in(int in_ms, anim::Ease in, int out_ms, anim::Ease out) {
  fades_ = true;
  fade_out_ms_ = out_ms;
  fade_out_ease_ = out;
  presence_.jump(0);
  presence_.set(1, in_ms, in);
  redraw_all();
}

double Layer::presence() {
  if (!fades_)
    return 1;
  return presence_.get(this);
}

void Layer::fade_step(void *data) {
  auto *l = static_cast<Layer *>(data);
  l->presence_.value();
  if (l->presence_.running() && anim::enabled() && l->fade_host_) {
    l->fade_host_->redraw();
    Fl::repeat_timeout(1.0 / std::max(10.0, anim::refresh_hz()), fade_step, data);
    return;
  }
  l->finish_close();
}

void Layer::finish_close() {
  Fl::remove_timeout(fade_step, this);
  if (Window *win = fade_host_) {
    auto &f = win->fading_;
    f.erase(std::remove(f.begin(), f.end(), this), f.end());
    fade_host_ = nullptr;
    win->remove(this);
    win->redraw();
  }
  Fl::delete_widget(this);
}

void Layer::open(Window *win) {
  if (!win || host_)
    return;
  Fl_Group::current(nullptr);
  resize(0, 0, win->w(), win->h());
  host_ = win;
  if (modal()) {
    delete restore_;
    restore_ = Fl::focus() ? new Fl_Widget_Tracker(Fl::focus()) : nullptr;
  }
  win->add_layer(this);
  relayout();
  redraw_all();
}

void Layer::close() {
  if (!host_)
    return;
  closing();
  Window *win = host_;
  bool had_focus = Fl::focus() && (Fl::focus() == this || Fl::focus()->inside(this));
  // From where it is (a layer closed while still fading in).
  double shown = fades_ ? presence_.value() : 0;
  int ms = int(fade_out_ms_ * shown + 0.5);
  bool fade = fades_ && ms > 0 && anim::enabled() && win->shown() && visible();
  win->drop_layer(this, !fade);
  host_ = nullptr;
  if (modal() && had_focus) {
    if (restore_ && !restore_->deleted() && restore_->widget()->visible_r())
      restore_->widget()->take_focus();
    else
      win->take_focus();
    if (Fl::focus() == this || (Fl::focus() && Fl::focus()->inside(this)))
      Fl::focus(win);
  }
  win->redraw();
  if (!fade) {
    Fl::delete_widget(this);
    return;
  }
  // Still drawn above the content, but no longer taking events (output
  // widgets get none, and nothing is drawn greyed out).
  set_output();
  fade_host_ = win;
  win->fading_.push_back(this);
  presence_.jump(shown);
  presence_.set(0, ms, fade_out_ease_);
  Fl::add_timeout(1.0 / std::max(10.0, anim::refresh_hz()), fade_step, this);
}

void Layer::redraw_all() {
  if (host_)
    host_->redraw();
}

// ---------------------------------------------------------------------------
// Window.

static void window_setup(Window *w) {
  w->box(FL_FLAT_BOX);
  w->color(theme().background().fl());
}

Window::Window(int w, int h, const char *title) : Fl_Double_Window(w, h, title) { window_setup(this); }
Window::Window(int x, int y, int w, int h, const char *title) : Fl_Double_Window(x, y, w, h, title) {
  window_setup(this);
}

Window::~Window() {
  // Layers are children: Fl_Group's destructor deletes them.
  layers_.clear();
  for (Layer *l : fading_)
    l->fade_host_ = nullptr;
  fading_.clear();
  delete last_focus_;
}

void Window::remember_focus(Fl_Widget *w) {
  if (w == this || (w && !contains(w)))
    return;
  if (last_focus_ && last_focus_->widget() == w)
    return;
  delete last_focus_;
  last_focus_ = w ? new Fl_Widget_Tracker(w) : nullptr;
}

void Window::note_focus() {
  Fl_Widget *f = Fl::focus();
  if (f && f != this && contains(f) && !top_layer())
    remember_focus(f);
}

// FLTK forgets the focused widget when the window loses the window
// system's focus, and gives the window's first child the keyboard when it
// comes back during a key (a Group skips its saved focus for arrow and Tab
// keys). For a window whose first child is a sidebar that sends arrows to
// the wrong place, so the last focused widget comes back instead.
bool Window::restore_focus() {
  Fl_Widget *w = remembered_focus();
  if (!w || !contains(w) || !w->visible_focus() || !w->active_r())
    return false;
  for (Fl_Widget *p = w; p && p != this; p = p->parent())
    if (!p->visible())
      return false;
  if (!w->handle(FL_FOCUS))
    return false;
  if (!w->contains(Fl::focus()))
    Fl::focus(w);
  return true;
}

void Window::shortcut(const std::string &spec, std::function<void()> fn) {
  remove_shortcut(spec);
  shortcuts_.push_back(Shortcut{parse_key(spec), spec, std::move(fn)});
}

void Window::remove_shortcut(const std::string &spec) {
  shortcuts_.erase(std::remove_if(shortcuts_.begin(), shortcuts_.end(),
                                  [&](const Shortcut &s) { return s.spec == spec; }),
                   shortcuts_.end());
}

std::vector<std::string> Window::shortcut_specs() const {
  std::vector<std::string> v;
  for (const auto &s : shortcuts_)
    v.push_back(s.spec);
  return v;
}

bool Window::run_shortcut() {
  if (modal_open())
    return false;
  for (auto &s : shortcuts_)
    if (event_is(s.key)) {
      auto fn = s.fn;
      fn();
      return true;
    }
  return false;
}

bool Window::modal_open() const {
  for (Layer *l : layers_)
    if (l->modal())
      return true;
  return false;
}

Window *Window::of(Fl_Widget *w) {
  Fl_Window *win = w ? (w->as_window() ? w->as_window() : w->window()) : nullptr;
  while (win) {
    if (auto *fw = dynamic_cast<Window *>(win))
      return fw;
    win = win->window();
  }
  return nullptr;
}

void Window::add_layer(Layer *l) {
  layers_.push_back(l);
  add(l); // last child: drawn last, gets events first
}

void Window::drop_layer(Layer *l, bool remove_child) {
  layers_.erase(std::remove(layers_.begin(), layers_.end(), l), layers_.end());
  if (remove_child)
    remove(l);
}

int Window::handle(int event) {
  Layer *top = nullptr;
  for (auto it = layers_.rbegin(); it != layers_.rend(); ++it)
    if ((*it)->modal()) {
      top = *it;
      break;
    }
  if (top && (event == FL_KEYBOARD || event == FL_SHORTCUT)) {
    // The modal layer sees the key first; then it either swallows it or
    // closes and lets it through (menus).
    if (top->handle(event))
      return 1;
    if (event == FL_KEYBOARD)
      return 0; // FLTK retries as FL_SHORTCUT
    if (!top->passes_keys())
      return 1;
    top->close();
    top = nullptr;
  }
  if ((event == FL_KEYBOARD || event == FL_SHORTCUT) && !top && !contains(Fl::focus()) &&
      Fl::event_key() != FL_Tab && restore_focus()) {
    // A key reached the window while nothing in it had the keyboard (the
    // window system never gave the window the focus, or took it away):
    // it goes to the widget that had it last, as a keystroke.
    for (Fl_Widget *w = Fl::focus(); w && w != this; w = w->parent())
      if (w->handle(FL_KEYBOARD))
        return 1;
  }
  if (event == FL_SHORTCUT && !top) {
    for (auto &s : shortcuts_)
      if (event_is(s.key)) {
        auto fn = s.fn;
        fn();
        return 1;
      }
  }
  if (event == FL_FOCUS && top)
    return top->handle(event);
  if (event == FL_FOCUS && !contains(Fl::focus())) {
    // Tab (with or without Shift) keeps FLTK's order; anything else
    // (the window coming back, a stray arrow) restores the last widget.
    bool tabbing = Fl::event_key() == FL_Tab && !Fl::event_state(FL_CTRL | FL_ALT | FL_META);
    if (!tabbing && restore_focus())
      return 1;
  }
  note_focus();
  int r = Fl_Double_Window::handle(event);
  note_focus();
  return r;
}

void Window::draw() {
  note_focus(); // focus moved from outside an event (right after show())
  color(theme().background().fl());
  if ((!layers_.empty() || !fading_.empty()) && damage())
    clear_damage(FL_DAMAGE_ALL); // translucent layers need what is below them redrawn
  Fl_Double_Window::draw();
}

void Window::resize(int x, int y, int w, int h) {
  Fl_Double_Window::resize(x, y, w, h);
  for (Layer *l : layers_) {
    l->resize(0, 0, w, h);
    l->relayout();
  }
  for (Layer *l : fading_)
    l->resize(0, 0, w, h);
}

} // namespace fo
