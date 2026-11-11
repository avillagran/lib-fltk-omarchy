#include "fltk-omarchy/process.h"

#include <FL/Fl.H>

#include <atomic>
#include <condition_variable>
#include <cstring>
#include <deque>
#include <map>
#include <mutex>
#include <thread>

#if defined(__unix__) || defined(__APPLE__)
#include <cerrno>
#include <csignal>
#include <fcntl.h>
#include <poll.h>
#include <sys/resource.h>
#include <sys/syscall.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>
#define FO_HAVE_POSIX 1
extern char **environ;
#endif

namespace fo {

#ifdef FO_HAVE_POSIX

namespace {

double mono_s() {
  timespec t;
  clock_gettime(CLOCK_MONOTONIC, &t);
  return double(t.tv_sec) + t.tv_nsec / 1e9;
}

struct Spawned {
  pid_t pid = -1;
  int out = -1, err = -1;
  std::string error;
};

void close_from(int lowest, int keep) {
#if defined(SYS_close_range)
  if (keep > lowest)
    syscall(SYS_close_range, lowest, keep - 1, 0);
  if (syscall(SYS_close_range, keep + 1, ~0U, 0) == 0)
    return;
#endif
  for (int fd = lowest; fd < 1024; ++fd)
    if (fd != keep)
      ::close(fd);
}

Spawned spawn(const ProcessOptions &o) {
  Spawned s;
  if (o.argv.empty() || o.argv[0].empty()) {
    s.error = "empty argv";
    return s;
  }
  // Everything the child needs is prepared before fork: after it, only
  // async-signal-safe calls.
  std::vector<char *> argv;
  for (const auto &a : o.argv)
    argv.push_back(const_cast<char *>(a.c_str()));
  argv.push_back(nullptr);
  std::vector<std::string> env_store;
  std::vector<char *> envp;
  if (!o.env.empty()) {
    for (char **e = environ; e && *e; ++e) {
      const char *eq = std::strchr(*e, '=');
      size_t n = eq ? size_t(eq - *e) : std::strlen(*e);
      bool overridden = false;
      for (const auto &x : o.env)
        if (x.compare(0, n, *e, n) == 0 && x.size() > n && x[n] == '=')
          overridden = true;
      if (!overridden)
        envp.push_back(*e);
    }
    for (const auto &x : o.env)
      envp.push_back(const_cast<char *>(x.c_str()));
    envp.push_back(nullptr);
  }

  int out[2] = {-1, -1}, err[2] = {-1, -1}, status[2] = {-1, -1};
  if (::pipe2(out, O_CLOEXEC) != 0 || ::pipe2(status, O_CLOEXEC) != 0 ||
      (o.capture_stderr && ::pipe2(err, O_CLOEXEC) != 0)) {
    s.error = std::strerror(errno);
    for (int fd : {out[0], out[1], err[0], err[1], status[0], status[1]})
      if (fd >= 0)
        ::close(fd);
    return s;
  }
  const char *cwd = o.cwd.empty() ? nullptr : o.cwd.c_str();
  pid_t pid = ::fork();
  if (pid == 0) {
    ::setpgid(0, 0);
    sigset_t none;
    sigemptyset(&none);
    sigprocmask(SIG_SETMASK, &none, nullptr);
    ::signal(SIGPIPE, SIG_DFL);
    if (o.limit_address_space) {
      rlimit r{rlim_t(o.limit_address_space), rlim_t(o.limit_address_space)};
      setrlimit(RLIMIT_AS, &r);
    }
    if (o.limit_cpu_s) {
      rlimit r{rlim_t(o.limit_cpu_s), rlim_t(o.limit_cpu_s)};
      setrlimit(RLIMIT_CPU, &r);
    }
    int code = 0;
    if (cwd && ::chdir(cwd) != 0)
      code = errno;
    int devnull = ::open("/dev/null", O_RDWR);
    if (!code) {
      ::dup2(devnull, 0);
      ::dup2(out[1], 1);
      ::dup2(o.capture_stderr ? err[1] : devnull, 2);
      close_from(3, status[1]);
      if (!envp.empty())
        ::execvpe(argv[0], argv.data(), envp.data());
      else
        ::execvp(argv[0], argv.data());
      code = errno;
    }
    ssize_t w = ::write(status[1], &code, sizeof code);
    (void)w;
    ::_exit(127);
  }
  ::close(out[1]);
  ::close(status[1]);
  if (err[1] >= 0)
    ::close(err[1]);
  if (pid < 0) {
    s.error = std::strerror(errno);
    ::close(out[0]);
    ::close(status[0]);
    if (err[0] >= 0)
      ::close(err[0]);
    return s;
  }
  int code = 0;
  ssize_t n;
  do
    n = ::read(status[0], &code, sizeof code);
  while (n < 0 && errno == EINTR);
  ::close(status[0]);
  if (n == ssize_t(sizeof code)) {
    // exec failed: the child already exited with 127.
    s.error = std::string(o.argv[0]) + ": " + std::strerror(code);
    int st;
    ::waitpid(pid, &st, 0);
    ::close(out[0]);
    if (err[0] >= 0)
      ::close(err[0]);
    return s;
  }
  s.pid = pid;
  s.out = out[0];
  s.err = err[0];
  ::fcntl(s.out, F_SETFL, O_NONBLOCK);
  if (s.err >= 0)
    ::fcntl(s.err, F_SETFL, O_NONBLOCK);
  return s;
}

/// Reads what is available. Returns false at EOF (or on error).
bool drain(int fd, std::string &buf, size_t cap, bool &over) {
  char tmp[16384];
  for (;;) {
    ssize_t n = ::read(fd, tmp, sizeof tmp);
    if (n > 0) {
      size_t room = buf.size() < cap ? cap - buf.size() : 0;
      if (size_t(n) > room)
        over = true;
      buf.append(tmp, size_t(n) < room ? size_t(n) : room);
      continue;
    }
    if (n < 0 && errno == EINTR)
      continue;
    if (n < 0 && errno == EAGAIN)
      return true;
    return false;
  }
}

void decode_status(int st, ProcessResult &r) {
  if (WIFEXITED(st))
    r.exit_code = WEXITSTATUS(st);
  else if (WIFSIGNALED(st))
    r.signal = WTERMSIG(st);
}

} // namespace

ProcessResult run_sync(const ProcessOptions &o) {
  ProcessResult r;
  Spawned s = spawn(o);
  if (s.pid < 0) {
    r.error = s.error;
    return r;
  }
  r.started = true;
  double deadline = o.timeout_s > 0 ? mono_s() + o.timeout_s : 0;
  bool out_open = true, err_open = s.err >= 0, killed = false;
  bool err_over = false;
  auto kill_group = [&] {
    if (!killed) {
      ::kill(-s.pid, SIGKILL);
      killed = true;
    }
  };
  while (out_open || err_open) {
    int wait_ms = -1;
    if (deadline) {
      double left = deadline - mono_s();
      if (left <= 0) {
        r.timed_out = true;
        kill_group();
        break;
      }
      wait_ms = int(left * 1000) + 1;
    }
    pollfd p[2];
    int n = 0;
    if (out_open)
      p[n++] = {s.out, POLLIN, 0};
    if (err_open)
      p[n++] = {s.err, POLLIN, 0};
    if (::poll(p, nfds_t(n), wait_ms) < 0 && errno != EINTR)
      break;
    if (out_open && !drain(s.out, r.out, o.stdout_cap, r.truncated))
      out_open = false;
    if (err_open && !drain(s.err, r.err, o.stderr_cap, err_over))
      err_open = false;
    if (r.truncated && o.kill_on_cap) {
      kill_group();
      break;
    }
  }
  ::close(s.out);
  if (s.err >= 0)
    ::close(s.err);
  // The output is closed; the child may still be running (it closed
  // stdout early). Wait for it within the deadline.
  int st = 0;
  for (;;) {
    pid_t w = ::waitpid(s.pid, &st, WNOHANG);
    if (w == s.pid || (w < 0 && errno != EINTR))
      break;
    if (deadline && mono_s() >= deadline) {
      r.timed_out = true;
      kill_group();
      ::waitpid(s.pid, &st, 0);
      break;
    }
    ::usleep(2000);
  }
  decode_status(st, r);
  return r;
}

// ---------------------------------------------------------------------------
// Asynchronous children on the FLTK loop.

namespace {

struct Job {
  unsigned id;
  Spawned sp;
  size_t out_cap, err_cap;
  bool kill_on_cap;
  bool out_open = true, err_open = false, killed = false, err_over = false;
  ProcessResult res;
  std::function<void(ProcessResult)> done;
};

std::map<unsigned, Job *> &jobs() {
  static std::map<unsigned, Job *> j;
  return j;
}
unsigned next_job = 1;

void kill_job(Job *j) {
  if (!j->killed) {
    ::kill(-j->sp.pid, SIGKILL);
    j->killed = true;
  }
}

void on_timeout(void *data);
void on_poll(void *data);

void finish(Job *j, int st) {
  Fl::remove_timeout(on_timeout, j);
  Fl::remove_timeout(on_poll, j);
  decode_status(st, j->res);
  jobs().erase(j->id);
  auto done = std::move(j->done);
  ProcessResult res = std::move(j->res);
  delete j;
  if (done)
    done(std::move(res));
}

void try_reap(Job *j) {
  if (j->out_open || j->err_open)
    return;
  int st = 0;
  pid_t w = ::waitpid(j->sp.pid, &st, WNOHANG);
  if (w == j->sp.pid || (w < 0 && errno != EINTR))
    return finish(j, st);
  Fl::remove_timeout(on_poll, j);
  Fl::add_timeout(0.01, on_poll, j);
}

void on_poll(void *data) { try_reap(static_cast<Job *>(data)); }

void on_timeout(void *data) {
  auto *j = static_cast<Job *>(data);
  j->res.timed_out = true;
  kill_job(j);
}

void close_fd(Job *j, bool err) {
  int &fd = err ? j->sp.err : j->sp.out;
  Fl::remove_fd(fd);
  ::close(fd);
  fd = -1;
  (err ? j->err_open : j->out_open) = false;
}

void on_out(int, void *data) {
  auto *j = static_cast<Job *>(data);
  if (!drain(j->sp.out, j->res.out, j->out_cap, j->res.truncated))
    close_fd(j, false);
  if (j->res.truncated && j->kill_on_cap) {
    kill_job(j);
    if (j->out_open)
      close_fd(j, false);
  }
  try_reap(j);
}

void on_err(int, void *data) {
  auto *j = static_cast<Job *>(data);
  if (!drain(j->sp.err, j->res.err, j->err_cap, j->err_over))
    close_fd(j, true);
  try_reap(j);
}

} // namespace

unsigned run(const ProcessOptions &o, std::function<void(ProcessResult)> done) {
  Spawned s = spawn(o);
  if (s.pid < 0) {
    ProcessResult r;
    r.error = s.error;
    if (done)
      done(std::move(r));
    return 0;
  }
  auto *j = new Job{next_job++, s, o.stdout_cap, o.stderr_cap, o.kill_on_cap, true, false, false, false, ProcessResult(), nullptr};
  if (!next_job)
    next_job = 1;
  j->res.started = true;
  j->err_open = s.err >= 0;
  j->done = std::move(done);
  jobs()[j->id] = j;
  Fl::add_fd(s.out, FL_READ, on_out, j);
  if (s.err >= 0)
    Fl::add_fd(s.err, FL_READ, on_err, j);
  if (o.timeout_s > 0)
    Fl::add_timeout(o.timeout_s, on_timeout, j);
  return j->id;
}

void cancel(unsigned id) {
  auto it = jobs().find(id);
  if (it != jobs().end())
    kill_job(it->second);
}

int running_processes() { return int(jobs().size()); }

#else // no POSIX: processes are unavailable (a later Windows port fills this in)

ProcessResult run_sync(const ProcessOptions &) {
  ProcessResult r;
  r.error = "processes are not supported on this platform yet";
  return r;
}
unsigned run(const ProcessOptions &o, std::function<void(ProcessResult)> done) {
  if (done)
    done(run_sync(o));
  return 0;
}
void cancel(unsigned) {}
int running_processes() { return 0; }

#endif

// ---------------------------------------------------------------------------
// Thread pool.

namespace tasks {

namespace {

struct Item {
  std::function<void()> work, done;
};

struct Pool {
  std::mutex m;
  std::condition_variable cv;
  std::deque<Item> queue;
  int threads = 0, idle = 0;
  std::atomic<int> pending{0};
};

Pool &pool() {
  static Pool *p = new Pool; // never destroyed: workers may outlive main()
  return *p;
}

void on_awake(void *data) {
  auto *fn = static_cast<std::function<void()> *>(data);
  (*fn)();
  delete fn;
}

void worker() {
  Pool &p = pool();
  for (;;) {
    Item item;
    {
      std::unique_lock<std::mutex> lock(p.m);
      ++p.idle;
      p.cv.wait(lock, [&] { return !p.queue.empty(); });
      --p.idle;
      item = std::move(p.queue.front());
      p.queue.pop_front();
    }
    if (item.work)
      item.work();
    auto done = std::move(item.done);
    post([done] {
      pool().pending--;
      if (done)
        done();
    });
  }
}

bool &locked() {
  static bool l = false;
  return l;
}

} // namespace

void post(std::function<void()> fn) {
  Fl::awake(on_awake, new std::function<void()>(std::move(fn)));
}

void run(std::function<void()> work, std::function<void()> done) {
  if (!locked()) {
    Fl::lock(); // enables Fl::awake() messages; the FLTK loop unlocks while waiting
    locked() = true;
  }
  Pool &p = pool();
  p.pending++;
  std::lock_guard<std::mutex> lock(p.m);
  p.queue.push_back(Item{std::move(work), std::move(done)});
  unsigned hw = std::thread::hardware_concurrency();
  int max_threads = hw >= 4 || hw == 0 ? 4 : int(hw);
  if (p.idle == 0 && p.threads < max_threads) {
    ++p.threads;
    std::thread(worker).detach();
  }
  p.cv.notify_one();
}

int pending() { return pool().pending; }

} // namespace tasks

} // namespace fo
