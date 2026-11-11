// Child processes and background work, without blocking the UI.
//
// Heavy or risky work (decoders, thumbnailers, mounts) belongs in a child
// process so the app itself stays small: run() spawns argv directly (no
// shell), with optional resource limits, a timeout and a cap on captured
// output, and calls back on the FLTK thread when the child is done.
//
//   fo::ProcessOptions o;
//   o.argv = {"file", "--brief", "--mime-type", path};
//   o.timeout_s = 2;
//   fo::run(o, [](fo::ProcessResult r) { if (r.ok()) use(r.out); });
//
// Blocking work in-process goes to the small thread pool:
//
//   fo::tasks::run([=] { heavy(); }, [=] { update_ui(); });
#pragma once

#include <cstdint>
#include <functional>
#include <string>
#include <vector>

namespace fo {

struct ProcessOptions {
  std::vector<std::string> argv; ///< argv[0] is looked up in PATH
  std::string cwd;               ///< working directory ("" = inherit)
  std::vector<std::string> env;  ///< extra "NAME=value" entries
  double timeout_s = 0;          ///< kill (SIGKILL, whole group) after this; 0 = none
  size_t stdout_cap = 1 << 20;   ///< bytes of stdout kept
  bool kill_on_cap = true;       ///< kill the child once stdout passes the cap
  bool capture_stderr = false;   ///< otherwise stderr goes to /dev/null
  size_t stderr_cap = 64 * 1024;
  uint64_t limit_address_space = 0; ///< RLIMIT_AS in bytes; 0 = unlimited
  uint64_t limit_cpu_s = 0;         ///< RLIMIT_CPU in seconds; 0 = unlimited
};

struct ProcessResult {
  bool started = false;   ///< exec succeeded
  int exit_code = -1;     ///< valid when it exited normally
  int signal = 0;         ///< terminating signal, 0 if it exited
  bool timed_out = false;
  bool truncated = false; ///< stdout passed the cap (and was cut)
  std::string out, err;
  std::string error;      ///< why it did not start, when !started
  bool ok() const { return started && signal == 0 && exit_code == 0 && !timed_out && !truncated; }
};

/// Spawns and waits (blocking). For worker threads and tools.
ProcessResult run_sync(const ProcessOptions &opts);

/// Spawns and returns at once; `done` runs on the FLTK thread (through
/// Fl::add_fd) when the child has exited and its output is read. Returns an
/// id for cancel(), 0 when it could not start (then `done` already ran).
unsigned run(const ProcessOptions &opts, std::function<void(ProcessResult)> done);
/// Kills a running child started with run(); its `done` runs with signal set.
void cancel(unsigned id);
/// Number of children started with run() still running.
int running_processes();

namespace tasks {
/// Runs `work` on a worker thread (pool of up to 4) and then `done` on the
/// FLTK thread. Call from the FLTK thread. Calls Fl::lock() the first time.
void run(std::function<void()> work, std::function<void()> done = {});
/// Runs `fn` on the FLTK thread soon (callable from any thread).
void post(std::function<void()> fn);
/// Work items queued or running.
int pending();
} // namespace tasks

} // namespace fo
