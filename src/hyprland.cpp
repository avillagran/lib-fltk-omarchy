#include "fltk-omarchy/hyprland.h"

#include "fltk-omarchy/json.h"
#include "fltk-omarchy/platform.h"

#include <FL/Fl.H>

#include <cerrno>
#include <cstring>
#include <map>

#if defined(__linux__)
#include <fcntl.h>
#include <poll.h>
#include <sys/socket.h>
#include <sys/un.h>
#include <time.h>
#include <unistd.h>
#define FO_HAVE_UNIX_SOCKETS 1
#endif

namespace fo {
namespace hypr {

std::string socket_path(bool events) {
  std::string sig = platform::env("HYPRLAND_INSTANCE_SIGNATURE");
  if (sig.empty() || sig.find('/') != std::string::npos)
    return "";
  const char *name = events ? "/.socket2.sock" : "/.socket.sock";
  std::string runtime = platform::env("XDG_RUNTIME_DIR");
  if (!runtime.empty()) {
    std::string p = runtime + "/hypr/" + sig + name;
    if (platform::exists(p))
      return p;
  }
  std::string p = "/tmp/hypr/" + sig + name;
  return platform::exists(p) ? p : "";
}

bool available() { return !socket_path().empty(); }

bool parse_option_int(const std::string &text, int &out) {
  json::Value v;
  if (!json::parse(text, v) || !v.is_object())
    return false;
  if (const json::Value *i = v.get("int"); i && i->is_number()) {
    out = int(i->number);
    return true;
  }
  if (const json::Value *b = v.get("bool"); b && b->is_bool()) {
    out = b->boolean ? 1 : 0;
    return true;
  }
  return false;
}

bool parse_option_bool(const std::string &text, bool &out) {
  int v;
  if (!parse_option_int(text, v))
    return false;
  out = v != 0;
  return true;
}

double parse_refresh_rate(const std::string &text) {
  json::Value v;
  if (!json::parse(text, v) || !v.is_array() || v.items.empty())
    return 0;
  const json::Value *pick = &v.items[0];
  for (const auto &m : v.items)
    if (m.flag("focused")) {
      pick = &m;
      break;
    }
  double hz = pick->num("refreshRate", 0);
  return hz > 1 ? hz : 0;
}

#ifdef FO_HAVE_UNIX_SOCKETS

namespace {

int connect_to(const std::string &path) {
  if (path.empty() || path.size() >= sizeof(sockaddr_un::sun_path))
    return -1;
  int fd = ::socket(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC | SOCK_NONBLOCK, 0);
  if (fd < 0)
    return -1;
  sockaddr_un addr{};
  addr.sun_family = AF_UNIX;
  std::memcpy(addr.sun_path, path.c_str(), path.size());
  if (::connect(fd, reinterpret_cast<sockaddr *>(&addr), sizeof addr) != 0 && errno != EINPROGRESS) {
    ::close(fd);
    return -1;
  }
  return fd;
}

struct Pending {
  int fd;
  std::string reply;
  Reply done;
};

void finish(Pending *p, bool ok) {
  Fl::remove_fd(p->fd);
  ::close(p->fd);
  Reply done = std::move(p->done);
  std::string reply = std::move(p->reply);
  delete p;
  if (done)
    done(ok, reply);
}

void on_timeout(void *data) { finish(static_cast<Pending *>(data), false); }

void on_readable(int fd, void *data) {
  auto *p = static_cast<Pending *>(data);
  char buf[8192];
  for (;;) {
    ssize_t n = ::read(fd, buf, sizeof buf);
    if (n > 0) {
      p->reply.append(buf, size_t(n));
      if (p->reply.size() > (8u << 20)) {
        Fl::remove_timeout(on_timeout, p);
        finish(p, false);
        return;
      }
      continue;
    }
    if (n < 0 && (errno == EAGAIN || errno == EINTR))
      return;
    Fl::remove_timeout(on_timeout, p);
    finish(p, n == 0);
    return;
  }
}

struct Events {
  int fd = -1;
  std::string partial;
  int next_id = 1;
  std::map<int, std::function<void(const std::string &, const std::string &)>> subs;
};

Events &events() {
  static Events e;
  return e;
}

void on_event(int fd, void *) {
  Events &e = events();
  char buf[4096];
  for (;;) {
    ssize_t n = ::read(fd, buf, sizeof buf);
    if (n > 0) {
      e.partial.append(buf, size_t(n));
      continue;
    }
    if (n == 0 || (errno != EAGAIN && errno != EINTR)) {
      Fl::remove_fd(fd);
      ::close(fd);
      e.fd = -1;
    }
    break;
  }
  size_t nl;
  while ((nl = e.partial.find('\n')) != std::string::npos) {
    std::string line = e.partial.substr(0, nl);
    e.partial.erase(0, nl + 1);
    size_t sep = line.find(">>");
    std::string ev = sep == std::string::npos ? line : line.substr(0, sep);
    std::string data = sep == std::string::npos ? "" : line.substr(sep + 2);
    auto subs = e.subs; // callbacks may unsubscribe
    for (auto &kv : subs)
      kv.second(ev, data);
  }
}

} // namespace

void request(const std::string &req, Reply done, double timeout_s) {
  int fd = connect_to(socket_path());
  if (fd < 0) {
    if (done)
      done(false, "");
    return;
  }
  // The request is tiny; a fresh Unix socket always has room for it.
  if (::write(fd, req.data(), req.size()) != ssize_t(req.size())) {
    ::close(fd);
    if (done)
      done(false, "");
    return;
  }
  auto *p = new Pending{fd, "", std::move(done)};
  Fl::add_fd(fd, FL_READ, on_readable, p);
  Fl::add_timeout(timeout_s, on_timeout, p);
}

bool request_sync(const std::string &req, std::string &reply, int timeout_ms) {
  reply.clear();
  int fd = connect_to(socket_path());
  if (fd < 0)
    return false;
  bool ok = ::write(fd, req.data(), req.size()) == ssize_t(req.size());
  timespec start;
  clock_gettime(CLOCK_MONOTONIC, &start);
  while (ok) {
    timespec now;
    clock_gettime(CLOCK_MONOTONIC, &now);
    int left = timeout_ms - int((now.tv_sec - start.tv_sec) * 1000 + (now.tv_nsec - start.tv_nsec) / 1000000);
    if (left <= 0) {
      ok = false;
      break;
    }
    pollfd pfd{fd, POLLIN, 0};
    if (::poll(&pfd, 1, left) <= 0)
      continue;
    char buf[8192];
    ssize_t n = ::read(fd, buf, sizeof buf);
    if (n > 0)
      reply.append(buf, size_t(n));
    else if (n == 0)
      break;
    else if (errno != EAGAIN && errno != EINTR)
      ok = false;
  }
  ::close(fd);
  return ok;
}

int subscribe(std::function<void(const std::string &, const std::string &)> fn) {
  Events &e = events();
  if (e.fd < 0) {
    e.fd = connect_to(socket_path(true));
    if (e.fd < 0)
      return 0;
    Fl::add_fd(e.fd, FL_READ, on_event, nullptr);
  }
  int id = e.next_id++;
  e.subs[id] = std::move(fn);
  return id;
}

void unsubscribe(int id) {
  Events &e = events();
  e.subs.erase(id);
  if (e.subs.empty() && e.fd >= 0) {
    Fl::remove_fd(e.fd);
    ::close(e.fd);
    e.fd = -1;
  }
}

#else // no Unix sockets: Hyprland is never available

void request(const std::string &, Reply done, double) {
  if (done)
    done(false, "");
}
bool request_sync(const std::string &, std::string &reply, int) {
  reply.clear();
  return false;
}
int subscribe(std::function<void(const std::string &, const std::string &)>) { return 0; }
void unsubscribe(int) {}

#endif

} // namespace hypr
} // namespace fo
