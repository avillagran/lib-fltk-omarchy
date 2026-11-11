// OS specifics behind one small interface. Linux is complete; macOS and
// the BSDs watch files with kqueue; Windows and anything else poll.
#pragma once

#include <functional>
#include <string>
#include <vector>

namespace fo {
namespace platform {

/// Environment variable, or `fallback` when unset or empty.
std::string env(const char *name, const std::string &fallback = "");

/// $HOME (never empty: "/" as a last resort).
std::string home_dir();
/// $XDG_CONFIG_HOME or ~/.config.
std::string config_dir();
/// $XDG_STATE_HOME or ~/.local/state.
std::string state_dir();

/// Reads at most `max_bytes` of a file. Returns false when it cannot be read.
bool read_file(const std::string &path, std::string &out, size_t max_bytes = 1 << 20);
bool exists(const std::string &path);
/// Parent directory of `path` ("/" for "/x", "." for "x").
std::string parent(const std::string &path);

/// Watches files and directories and calls `on_change` on the FLTK thread,
/// debounced (default 80 ms), after anything under them changes: inotify
/// on Linux and kqueue on macOS and the BSDs (both through `Fl::add_fd`),
/// and polling elsewhere (Windows) or when asked for: every
/// `poll_interval()` (1 s) each path's size and modification time, and a
/// directory's entries, are compared with the previous look.
///
/// Watches are not recursive. Replacing a watched file (as theme and
/// language switchers do) drops its watch, so callers usually watch the
/// parent directory too and call `set_paths` again from `on_change`.
class FileWatch {
public:
  enum class Backend {
    Auto,    ///< the OS's notifications, polling when there are none
    Polling, ///< always poll (any OS; tests, network file systems)
  };
  explicit FileWatch(std::function<void()> on_change, double debounce_s = 0.08, Backend backend = Backend::Auto);
  ~FileWatch();
  FileWatch(const FileWatch &) = delete;
  FileWatch &operator=(const FileWatch &) = delete;

  /// Replaces the watched set. Paths that do not exist are skipped by the
  /// notifying backends; polling notices them appearing.
  void set_paths(const std::vector<std::string> &paths);
  /// True when a backend is running (notifications or polling).
  bool active() const { return fd_ >= 0 || polling_; }
  /// "inotify", "kqueue", "polling" or "none".
  const char *backend() const;
  /// How often polling looks (seconds, default 1).
  void poll_interval(double seconds);

private:
  static void on_fd(int fd, void *self);
  static void on_timer(void *self);
  static void on_poll(void *self);
  unsigned long long snapshot() const;
  std::function<void()> on_change_;
  double debounce_;
  int fd_ = -1;
  std::vector<int> watches_;
  std::vector<std::string> paths_;
  bool polling_ = false;
  double poll_s_ = 1.0;
  unsigned long long last_ = 0;
};

} // namespace platform
} // namespace fo
