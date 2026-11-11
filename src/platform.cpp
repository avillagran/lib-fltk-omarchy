#include "fltk-omarchy/platform.h"

#include <FL/Fl.H>

#include <cstdio>
#include <cstdlib>
#include <sys/stat.h>

#if defined(__linux__)
#include <fcntl.h>
#include <sys/inotify.h>
#include <unistd.h>
#define FO_HAVE_INOTIFY 1
#endif

namespace fo {
namespace platform {

std::string env(const char *name, const std::string &fallback) {
  const char *v = std::getenv(name);
  return v && *v ? std::string(v) : fallback;
}

std::string home_dir() {
  std::string h = env("HOME");
#if defined(_WIN32)
  if (h.empty())
    h = env("USERPROFILE");
#endif
  return h.empty() ? std::string("/") : h;
}

std::string config_dir() { return env("XDG_CONFIG_HOME", home_dir() + "/.config"); }
std::string state_dir() { return env("XDG_STATE_HOME", home_dir() + "/.local/state"); }

bool read_file(const std::string &path, std::string &out, size_t max_bytes) {
  out.clear();
  FILE *f = std::fopen(path.c_str(), "rb");
  if (!f)
    return false;
  char buf[8192];
  size_t n;
  while (out.size() < max_bytes && (n = std::fread(buf, 1, sizeof buf, f)) > 0)
    out.append(buf, n < max_bytes - out.size() ? n : max_bytes - out.size());
  std::fclose(f);
  return true;
}

bool exists(const std::string &path) {
  struct stat st;
  return ::stat(path.c_str(), &st) == 0;
}

std::string parent(const std::string &path) {
  size_t end = path.size();
  while (end > 1 && path[end - 1] == '/')
    --end;
  size_t slash = path.rfind('/', end - 1);
  if (slash == std::string::npos)
    return ".";
  if (slash == 0)
    return "/";
  return path.substr(0, slash);
}

FileWatch::FileWatch(std::function<void()> on_change, double debounce_s)
    : on_change_(std::move(on_change)), debounce_(debounce_s) {
#ifdef FO_HAVE_INOTIFY
  fd_ = inotify_init1(IN_NONBLOCK | IN_CLOEXEC);
  if (fd_ >= 0)
    Fl::add_fd(fd_, FL_READ, on_fd, this);
#endif
}

FileWatch::~FileWatch() {
  Fl::remove_timeout(on_timer, this);
#ifdef FO_HAVE_INOTIFY
  if (fd_ >= 0) {
    Fl::remove_fd(fd_);
    ::close(fd_);
  }
#endif
}

void FileWatch::set_paths(const std::vector<std::string> &paths) {
#ifdef FO_HAVE_INOTIFY
  if (fd_ < 0)
    return;
  for (int wd : watches_)
    inotify_rm_watch(fd_, wd);
  watches_.clear();
  const uint32_t mask = IN_CLOSE_WRITE | IN_MOVED_TO | IN_MOVED_FROM | IN_CREATE | IN_DELETE |
                        IN_DELETE_SELF | IN_MOVE_SELF | IN_ATTRIB | IN_MODIFY;
  for (const auto &p : paths) {
    int wd = inotify_add_watch(fd_, p.c_str(), mask);
    if (wd >= 0)
      watches_.push_back(wd);
  }
#else
  (void)paths;
#endif
}

void FileWatch::on_fd(int fd, void *data) {
  auto *self = static_cast<FileWatch *>(data);
#ifdef FO_HAVE_INOTIFY
  char buf[4096];
  while (::read(fd, buf, sizeof buf) > 0) {
  }
#else
  (void)fd;
#endif
  Fl::remove_timeout(on_timer, self);
  Fl::add_timeout(self->debounce_, on_timer, self);
}

void FileWatch::on_timer(void *data) {
  auto *self = static_cast<FileWatch *>(data);
  if (self->on_change_)
    self->on_change_();
}

} // namespace platform
} // namespace fo
