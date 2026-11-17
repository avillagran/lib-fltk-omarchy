#include "fltk-omarchy/dnd.h"

#include <FL/Fl.H>
#include <FL/Fl_Window.H>
#include <FL/platform.H>

#include <cerrno>
#include <csignal>
#include <cstring>
#include <ctime>
#include <fcntl.h>
#include <signal.h>
#include <unistd.h>

#if FLTK_USE_WAYLAND && defined(FO_HAVE_WAYLAND_CLIENT)
#include <wayland-client.h>
#define FO_WAYLAND_DND 1
#endif

namespace fo {
namespace dnd {

namespace {
bool g_active = false;
std::string g_uri, g_plain; // what the current drag offers

bool unreserved(unsigned char c) {
  return (c >= 'A' && c <= 'Z') || (c >= 'a' && c <= 'z') || (c >= '0' && c <= '9') || c == '-' || c == '.' ||
         c == '_' || c == '~' || c == '/';
}

void fall_back(const std::vector<std::string> &paths) {
  const std::string text = uri_list(paths);
  Fl::copy(text.c_str(), int(text.size()), 0);
  Fl::dnd();
}

#ifdef FO_WAYLAND_DND
// The toolkit's own Wayland objects, bound on FLTK's connection. FLTK keeps
// the target side (its data device is the first one, the one Hyprland sends
// drags to); this side only ever starts drags.
struct Wayland {
  bool tried = false;
  wl_seat *seat = nullptr;
  wl_data_device_manager *manager = nullptr;
  uint32_t manager_version = 0;
  wl_pointer *pointer = nullptr;
  wl_data_device *device = nullptr;
  wl_data_source *source = nullptr;
  uint32_t press_serial = 0; // of the latest button press: the drag's grab
} g;

const char *const kUriList = "text/uri-list";
const char *const kPlainUtf8 = "text/plain;charset=utf-8";
const char *const kPlain = "text/plain";

// write() that never raises SIGPIPE when the receiver already went away.
ssize_t write_quietly(int fd, const char *data, size_t size) {
  sigset_t pipe_set, old;
  sigemptyset(&pipe_set);
  sigaddset(&pipe_set, SIGPIPE);
  sigset_t pending;
  sigpending(&pending);
  const bool was_pending = sigismember(&pending, SIGPIPE);
  pthread_sigmask(SIG_BLOCK, &pipe_set, &old);
  ssize_t n = ::write(fd, data, size);
  const int saved = errno;
  if (n < 0 && saved == EPIPE && !was_pending) {
    const timespec zero{0, 0};
    sigtimedwait(&pipe_set, nullptr, &zero);
  }
  pthread_sigmask(SIG_SETMASK, &old, nullptr);
  errno = saved;
  return n;
}

struct Outgoing {
  int fd;
  std::string data;
  size_t done = 0;
};

// Writes what it can; true when the transfer is over (sent or failed).
bool pump(Outgoing *o) {
  while (o->done < o->data.size()) {
    ssize_t n = write_quietly(o->fd, o->data.data() + o->done, o->data.size() - o->done);
    if (n > 0) {
      o->done += size_t(n);
    } else if (n < 0 && errno == EINTR) {
      continue;
    } else if (n < 0 && (errno == EAGAIN || errno == EWOULDBLOCK)) {
      return false;
    } else {
      break;
    }
  }
  return true;
}

void writable(int fd, void *data) {
  auto *o = static_cast<Outgoing *>(data);
  if (!pump(o))
    return;
  Fl::remove_fd(fd, FL_WRITE);
  ::close(fd);
  delete o;
}

void send_async(int fd, const std::string &data) {
  ::fcntl(fd, F_SETFL, ::fcntl(fd, F_GETFL) | O_NONBLOCK);
  auto *o = new Outgoing{fd, data};
  if (pump(o)) {
    ::close(fd);
    delete o;
    return;
  }
  Fl::add_fd(fd, FL_WRITE, writable, o);
}

// The drag is over (dropped, refused or cancelled). The compositor kept the
// button release, so the pressed widget gets it now, as FLTK does at the end
// of its own drags.
void finish(wl_data_source *source) {
  wl_data_source_destroy(source);
  if (g.source != source)
    return;
  g.source = nullptr;
  g_active = false;
  Fl::e_state &= ~FL_BUTTONS;
  if (Fl_Widget *pushed = Fl::pushed()) {
    Fl::pushed(nullptr);
    const int old = Fl::e_number;
    pushed->handle(Fl::e_number = FL_RELEASE);
    Fl::e_number = old;
  }
}

void source_target(void *, wl_data_source *, const char *) {}
void source_send(void *, wl_data_source *, const char *mime, int fd) {
  if (mime && !std::strcmp(mime, kUriList))
    send_async(fd, g_uri);
  else if (mime && (!std::strcmp(mime, kPlainUtf8) || !std::strcmp(mime, kPlain)))
    send_async(fd, g_plain);
  else
    ::close(fd);
}
void source_cancelled(void *, wl_data_source *source) { finish(source); }
void source_dropped(void *, wl_data_source *) {}
void source_finished(void *, wl_data_source *source) { finish(source); }
void source_action(void *, wl_data_source *, uint32_t) {}
const wl_data_source_listener source_listener = {source_target, source_send,    source_cancelled,
                                                 source_dropped, source_finished, source_action};

void pointer_enter(void *, wl_pointer *, uint32_t, wl_surface *, wl_fixed_t, wl_fixed_t) {}
void pointer_leave(void *, wl_pointer *, uint32_t, wl_surface *) {}
void pointer_motion(void *, wl_pointer *, uint32_t, wl_fixed_t, wl_fixed_t) {}
void pointer_button(void *, wl_pointer *, uint32_t serial, uint32_t, uint32_t, uint32_t state) {
  if (state == WL_POINTER_BUTTON_STATE_PRESSED)
    g.press_serial = serial;
}
void pointer_axis(void *, wl_pointer *, uint32_t, uint32_t, wl_fixed_t) {}
void pointer_frame(void *, wl_pointer *) {}
void pointer_axis_source(void *, wl_pointer *, uint32_t) {}
void pointer_axis_stop(void *, wl_pointer *, uint32_t, uint32_t) {}
void pointer_axis_discrete(void *, wl_pointer *, uint32_t, int32_t) {}
wl_pointer_listener pointer_listener() {
  wl_pointer_listener l{};
  l.enter = pointer_enter;
  l.leave = pointer_leave;
  l.motion = pointer_motion;
  l.button = pointer_button;
  l.axis = pointer_axis;
  l.frame = pointer_frame;
  l.axis_source = pointer_axis_source;
  l.axis_stop = pointer_axis_stop;
  l.axis_discrete = pointer_axis_discrete;
  return l;
}
const wl_pointer_listener g_pointer_listener = pointer_listener(); // the seat is bound at v5 at most

void seat_capabilities(void *, wl_seat *seat, uint32_t caps) {
  const bool has = caps & WL_SEAT_CAPABILITY_POINTER;
  if (has && !g.pointer) {
    g.pointer = wl_seat_get_pointer(seat);
    wl_pointer_add_listener(g.pointer, &g_pointer_listener, nullptr);
  } else if (!has && g.pointer) {
    if (wl_pointer_get_version(g.pointer) >= WL_POINTER_RELEASE_SINCE_VERSION)
      wl_pointer_release(g.pointer);
    else
      wl_pointer_destroy(g.pointer);
    g.pointer = nullptr;
  }
}
void seat_name(void *, wl_seat *, const char *) {}
const wl_seat_listener seat_listener = {seat_capabilities, seat_name};

// This device never takes drops (FLTK's does): its offers go right away.
void device_offer(void *, wl_data_device *, wl_data_offer *offer) { wl_data_offer_destroy(offer); }
void device_enter(void *, wl_data_device *, uint32_t, wl_surface *, wl_fixed_t, wl_fixed_t, wl_data_offer *) {}
void device_leave(void *, wl_data_device *) {}
void device_motion(void *, wl_data_device *, uint32_t, wl_fixed_t, wl_fixed_t) {}
void device_drop(void *, wl_data_device *) {}
void device_selection(void *, wl_data_device *, wl_data_offer *) {}
const wl_data_device_listener device_listener = {device_offer,  device_enter, device_leave,
                                                 device_motion, device_drop,  device_selection};

void global(void *, wl_registry *registry, uint32_t name, const char *interface, uint32_t version) {
  if (!std::strcmp(interface, wl_seat_interface.name) && !g.seat) {
    g.seat = static_cast<wl_seat *>(wl_registry_bind(registry, name, &wl_seat_interface, version < 5 ? version : 5));
    wl_seat_add_listener(g.seat, &seat_listener, nullptr);
  } else if (!std::strcmp(interface, wl_data_device_manager_interface.name) && !g.manager) {
    g.manager_version = version < 3 ? version : 3;
    g.manager = static_cast<wl_data_device_manager *>(
        wl_registry_bind(registry, name, &wl_data_device_manager_interface, g.manager_version));
  }
}
void global_remove(void *, wl_registry *, uint32_t) {}
const wl_registry_listener registry_listener = {global, global_remove};

// Binds the seat and the data device manager once FLTK's display is open,
// on a private queue so FLTK's events are not dispatched from here, then
// hands everything to the default queue, which FLTK's loop dispatches.
void setup() {
  g.tried = true;
  wl_display *display = fl_wl_display();
  if (!display)
    return;
  wl_event_queue *queue = wl_display_create_queue(display);
  auto *wrapper = static_cast<wl_display *>(wl_proxy_create_wrapper(display));
  wl_proxy_set_queue(reinterpret_cast<wl_proxy *>(wrapper), queue);
  wl_registry *registry = wl_display_get_registry(wrapper);
  wl_proxy_wrapper_destroy(wrapper);
  wl_registry_add_listener(registry, &registry_listener, nullptr);
  wl_display_roundtrip_queue(display, queue); // the globals
  wl_display_roundtrip_queue(display, queue); // the seat's capabilities
  for (wl_proxy *p : {reinterpret_cast<wl_proxy *>(g.seat), reinterpret_cast<wl_proxy *>(g.manager),
                      reinterpret_cast<wl_proxy *>(g.pointer)})
    if (p)
      wl_proxy_set_queue(p, nullptr);
  wl_registry_destroy(registry);
  wl_event_queue_destroy(queue);
  if (g.seat && g.manager) {
    g.device = wl_data_device_manager_get_data_device(g.manager, g.seat);
    wl_data_device_add_listener(g.device, &device_listener, nullptr);
  }
}

bool start_wayland(const std::vector<std::string> &paths) {
  if (!g.tried)
    setup();
  if (!g.device || !g.press_serial)
    return false;
  Fl_Widget *pushed = Fl::pushed();
  Fl_Window *win = pushed ? pushed->top_window() : nullptr;
  wl_surface *origin = win && win->shown() ? fl_wl_surface(fl_wl_xid(win)) : nullptr;
  if (!origin)
    return false;
  if (g.source) { // a drag the compositor never closed
    wl_data_source_destroy(g.source);
    g.source = nullptr;
  }
  g_uri = uri_list(paths);
  g_plain = plain_text(paths);
  g.source = wl_data_device_manager_create_data_source(g.manager);
  wl_data_source_add_listener(g.source, &source_listener, nullptr);
  wl_data_source_offer(g.source, kUriList);
  wl_data_source_offer(g.source, kPlainUtf8);
  wl_data_source_offer(g.source, kPlain); // what FLTK's own drop target accepts
  if (g.manager_version >= WL_DATA_SOURCE_SET_ACTIONS_SINCE_VERSION)
    wl_data_source_set_actions(g.source, WL_DATA_DEVICE_MANAGER_DND_ACTION_COPY);
  wl_data_device_start_drag(g.device, g.source, origin, nullptr, g.press_serial);
  wl_display_flush(fl_wl_display());
  g_active = true;
  return true;
}
#endif
} // namespace

std::string uri_list(const std::vector<std::string> &paths) {
  static const char hex[] = "0123456789ABCDEF";
  std::string s;
  for (const auto &p : paths) {
    s += "file://";
    for (unsigned char c : p) {
      if (unreserved(c)) {
        s += char(c);
      } else {
        s += '%';
        s += hex[c >> 4];
        s += hex[c & 15];
      }
    }
    s += "\r\n";
  }
  return s;
}

std::string plain_text(const std::vector<std::string> &paths) {
  // No trailing newline: a terminal would run what it pastes.
  std::string s;
  for (size_t i = 0; i < paths.size(); ++i)
    s += (i ? "\n" : "") + paths[i];
  return s;
}

bool start_files(const std::vector<std::string> &paths) {
  if (paths.empty())
    return false;
#ifdef FO_WAYLAND_DND
  if (fl_wl_display() && start_wayland(paths))
    return true;
#endif
  fall_back(paths);
  return false;
}

bool active() { return g_active; }

bool intercept(int event, Fl_Window *window, int *result) {
#ifdef FO_WAYLAND_DND
  if (!g.tried && fl_wl_display())
    setup(); // before the first press, so its serial is known
  if (event == FL_PUSH)
    g_active = false; // a new press: no drag of ours is running
  if (event != FL_DND_RELEASE || !g_active)
    return false;
  // A drop of our own drag inside the application. FLTK would read the data
  // through the compositor, blocking on a pipe this same process has to
  // fill; the payload is here already, so the widget gets it directly and
  // FLTK just drops its offer (dispatch returns 0).
  if (Fl::handle_(FL_DND_RELEASE, window) && Fl::belowmouse()) {
    const std::string text = g_uri; // the handler may start another drag
    char *saved_text = Fl::e_text;
    const int saved_length = Fl::e_length, old = Fl::e_number;
    Fl::e_text = const_cast<char *>(text.c_str());
    Fl::e_length = int(text.size());
    Fl::belowmouse()->handle(Fl::e_number = FL_PASTE);
    Fl::e_number = old;
    Fl::e_text = saved_text;
    Fl::e_length = saved_length;
  }
  *result = 0;
  return true;
#else
  (void)event, (void)window, (void)result;
  return false;
#endif
}

} // namespace dnd
} // namespace fo
