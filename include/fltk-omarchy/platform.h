// OS specifics behind one small interface. Linux is complete; other
// systems compile with fallbacks (FileWatch does nothing, directories come
// from the usual environment variables).
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
/// debounced (default 80 ms), after anything under them changes. Uses
/// inotify through `Fl::add_fd` on Linux; elsewhere it never fires.
///
/// Watches are not recursive. Replacing a watched file (as theme and
/// language switchers do) drops its watch, so callers usually watch the
/// parent directory too and call `set_paths` again from `on_change`.
class FileWatch {
public:
  explicit FileWatch(std::function<void()> on_change, double debounce_s = 0.08);
  ~FileWatch();
  FileWatch(const FileWatch &) = delete;
  FileWatch &operator=(const FileWatch &) = delete;

  /// Replaces the watched set. Paths that do not exist are skipped.
  void set_paths(const std::vector<std::string> &paths);
  /// True when the OS backend is available (inotify initialized).
  bool active() const { return fd_ >= 0; }

private:
  static void on_fd(int fd, void *self);
  static void on_timer(void *self);
  std::function<void()> on_change_;
  double debounce_;
  int fd_ = -1;
  std::vector<int> watches_;
};

} // namespace platform
} // namespace fo
