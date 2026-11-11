#include "fltk-omarchy/platform.h"

#include <FL/Fl.H>

#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <sys/stat.h>
#include <sys/types.h>

#if defined(_WIN32)
#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>
#else
#include <dirent.h>
#include <fcntl.h>
#include <unistd.h>
#endif

#if defined(__linux__) && !defined(FO_PORTABLE_POSIX)
#include <sys/inotify.h>
#define FO_HAVE_INOTIFY 1
#elif defined(__APPLE__) || defined(__FreeBSD__) || defined(__NetBSD__) || defined(__OpenBSD__) || \
    defined(__DragonFly__)
#include <sys/event.h>
#include <sys/time.h>
#define FO_HAVE_KQUEUE 1
#ifndef O_EVTONLY
#define O_EVTONLY O_RDONLY // macOS only: watch without keeping the volume busy
#endif
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

// --- FileWatch ---------------------------------------------------------------

namespace {

// FNV-1a over what identifies a version of a file.
struct Hash {
  uint64_t h = 1469598103934665603ULL;
  void add(const void *p, size_t n) {
    auto *b = static_cast<const unsigned char *>(p);
    for (size_t i = 0; i < n; ++i) {
      h ^= b[i];
      h *= 1099511628211ULL;
    }
  }
  template <class T> void add(const T &v) { add(&v, sizeof v); }
  void add(const std::string &s) { add(s.data(), s.size()); }
};

#if defined(_WIN32)
std::wstring widen(const std::string &s) {
  int n = MultiByteToWideChar(CP_UTF8, 0, s.data(), int(s.size()), nullptr, 0);
  std::wstring w(size_t(n > 0 ? n : 0), L'\0');
  if (n > 0)
    MultiByteToWideChar(CP_UTF8, 0, s.data(), int(s.size()), &w[0], n);
  return w;
}

void add_find_data(Hash &h, const WIN32_FIND_DATAW &d) {
  h.add(d.dwFileAttributes);
  h.add(d.ftLastWriteTime.dwLowDateTime);
  h.add(d.ftLastWriteTime.dwHighDateTime);
  h.add(d.nFileSizeLow);
  h.add(d.nFileSizeHigh);
  h.add(d.cFileName, wcslen(d.cFileName) * sizeof(wchar_t));
}

void add_path(Hash &h, const std::string &path) {
  std::wstring w = widen(path);
  WIN32_FILE_ATTRIBUTE_DATA a;
  if (!GetFileAttributesExW(w.c_str(), GetFileExInfoStandard, &a)) {
    h.add(uint8_t(0)); // missing
    return;
  }
  h.add(a.dwFileAttributes);
  h.add(a.ftLastWriteTime);
  h.add(a.nFileSizeLow);
  h.add(a.nFileSizeHigh);
  if (a.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY) {
    WIN32_FIND_DATAW d;
    HANDLE f = FindFirstFileW((w + L"\\*").c_str(), &d);
    if (f != INVALID_HANDLE_VALUE) {
      int n = 0;
      do
        add_find_data(h, d);
      while (++n < 1024 && FindNextFileW(f, &d));
      FindClose(f);
    }
  }
}
#else
void add_stat(Hash &h, const struct stat &st) {
  h.add(st.st_mode);
  h.add(st.st_ino);
  h.add(st.st_size);
#if defined(__APPLE__)
  h.add(st.st_mtimespec.tv_sec);
  h.add(st.st_mtimespec.tv_nsec);
  h.add(st.st_ctimespec.tv_nsec);
#else
  h.add(st.st_mtim.tv_sec);
  h.add(st.st_mtim.tv_nsec);
  h.add(st.st_ctim.tv_nsec);
#endif
}

void add_path(Hash &h, const std::string &path) {
  struct stat st;
  if (::stat(path.c_str(), &st) != 0) {
    h.add(uint8_t(0)); // missing
    return;
  }
  add_stat(h, st);
  if (!S_ISDIR(st.st_mode))
    return;
  // A directory: its entries too (a file inside rewritten in place does
  // not touch the directory's own times). Entries are not followed.
  if (DIR *d = ::opendir(path.c_str())) {
    int n = 0;
    while (struct dirent *e = ::readdir(d)) {
      std::string name = e->d_name;
      if (name == "." || name == "..")
        continue;
      h.add(name);
      struct stat es;
      if (::fstatat(::dirfd(d), e->d_name, &es, AT_SYMLINK_NOFOLLOW) == 0)
        add_stat(h, es);
      if (++n >= 1024)
        break;
    }
    ::closedir(d);
  }
}
#endif

} // namespace

FileWatch::FileWatch(std::function<void()> on_change, double debounce_s, Backend backend)
    : on_change_(std::move(on_change)), debounce_(debounce_s) {
  if (backend == Backend::Auto) {
#if defined(FO_HAVE_INOTIFY)
    fd_ = inotify_init1(IN_NONBLOCK | IN_CLOEXEC);
#elif defined(FO_HAVE_KQUEUE)
    fd_ = kqueue();
    if (fd_ >= 0)
      ::fcntl(fd_, F_SETFD, FD_CLOEXEC);
#endif
    if (fd_ >= 0)
      Fl::add_fd(fd_, FL_READ, on_fd, this);
  }
  if (fd_ < 0) {
    polling_ = true;
    last_ = snapshot();
    Fl::add_timeout(poll_s_, on_poll, this);
  }
}

FileWatch::~FileWatch() {
  Fl::remove_timeout(on_timer, this);
  Fl::remove_timeout(on_poll, this);
#if defined(FO_HAVE_INOTIFY) || defined(FO_HAVE_KQUEUE)
  if (fd_ >= 0) {
    Fl::remove_fd(fd_);
#if defined(FO_HAVE_KQUEUE)
    for (int wfd : watches_)
      ::close(wfd);
#endif
    ::close(fd_);
  }
#endif
}

const char *FileWatch::backend() const {
  if (polling_)
    return "polling";
  if (fd_ < 0)
    return "none";
#if defined(FO_HAVE_INOTIFY)
  return "inotify";
#elif defined(FO_HAVE_KQUEUE)
  return "kqueue";
#else
  return "none";
#endif
}

void FileWatch::poll_interval(double seconds) {
  poll_s_ = seconds > 0.01 ? seconds : 0.01;
  if (polling_) {
    Fl::remove_timeout(on_poll, this);
    Fl::add_timeout(poll_s_, on_poll, this);
  }
}

unsigned long long FileWatch::snapshot() const {
  Hash h;
  for (const auto &p : paths_) {
    h.add(p);
    add_path(h, p);
  }
  return h.h;
}

void FileWatch::set_paths(const std::vector<std::string> &paths) {
  paths_ = paths;
  if (polling_) {
    last_ = snapshot();
    return;
  }
#if defined(FO_HAVE_INOTIFY)
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
#elif defined(FO_HAVE_KQUEUE)
  if (fd_ < 0)
    return;
  // kqueue watches open descriptors; closing one drops its filter.
  for (int wfd : watches_)
    ::close(wfd);
  watches_.clear();
  for (const auto &p : paths) {
    int wfd = ::open(p.c_str(), O_EVTONLY | O_CLOEXEC);
    if (wfd < 0)
      continue;
    struct kevent ev;
    EV_SET(&ev, uintptr_t(wfd), EVFILT_VNODE, EV_ADD | EV_CLEAR,
           NOTE_WRITE | NOTE_DELETE | NOTE_RENAME | NOTE_ATTRIB | NOTE_EXTEND | NOTE_LINK | NOTE_REVOKE, 0, nullptr);
    if (::kevent(fd_, &ev, 1, nullptr, 0, nullptr) == 0)
      watches_.push_back(wfd);
    else
      ::close(wfd);
  }
#endif
}

void FileWatch::on_fd(int fd, void *data) {
  auto *self = static_cast<FileWatch *>(data);
#if defined(FO_HAVE_INOTIFY)
  char buf[4096];
  while (::read(fd, buf, sizeof buf) > 0) {
  }
#elif defined(FO_HAVE_KQUEUE)
  struct kevent evs[16];
  struct timespec zero = {0, 0};
  while (::kevent(fd, nullptr, 0, evs, 16, &zero) > 0) {
  }
#else
  (void)fd;
#endif
  Fl::remove_timeout(on_timer, self);
  Fl::add_timeout(self->debounce_, on_timer, self);
}

void FileWatch::on_poll(void *data) {
  auto *self = static_cast<FileWatch *>(data);
  unsigned long long now = self->snapshot();
  if (now != self->last_) {
    self->last_ = now;
    Fl::remove_timeout(on_timer, self);
    Fl::add_timeout(self->debounce_, on_timer, self);
  }
  Fl::repeat_timeout(self->poll_s_, on_poll, data);
}

void FileWatch::on_timer(void *data) {
  auto *self = static_cast<FileWatch *>(data);
  if (self->on_change_)
    self->on_change_();
}

} // namespace platform
} // namespace fo
