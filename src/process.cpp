#include "fltk-omarchy/process.h"

#include <FL/Fl.H>

#include <atomic>
#include <condition_variable>
#include <cstring>
#include <deque>
#include <map>
#include <memory>
#include <mutex>
#include <string>
#include <thread>
#include <vector>

// Platforms: POSIX (Linux, macOS, the BSDs) through fork/exec, and
// Windows through CreateProcessW and a job object. Linux uses pipe2,
// execvpe and close_range; other POSIX systems (and Linux built with
// FO_PORTABLE_POSIX, which is how those paths are tested) use pipe +
// FD_CLOEXEC, a PATH search done before fork and a close() loop.
#if defined(_WIN32)
#define FO_HAVE_WIN32_PROCESS 1
#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>
#elif defined(__unix__) || defined(__APPLE__)
#include <cerrno>
#include <csignal>
#include <cstdlib>
#include <fcntl.h>
#include <poll.h>
#include <sys/resource.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>
#define FO_HAVE_POSIX 1
#if defined(__linux__) && !defined(FO_PORTABLE_POSIX)
#include <sys/syscall.h>
#define FO_LINUX_SPAWN 1
#endif
#if defined(__APPLE__)
#include <crt_externs.h>
#define FO_ENVIRON (*_NSGetEnviron())
#else
extern char **environ;
#define FO_ENVIRON environ
#endif
#endif

namespace fo {

namespace {
// Fl::awake() needs Fl::lock() to have been called once on the FLTK thread.
void ensure_fl_lock() {
  static bool locked = false;
  if (!locked) {
    Fl::lock(); // the FLTK loop unlocks while waiting
    locked = true;
  }
}
} // namespace

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

// Closes every descriptor from `lowest` up except `keep`. `max_fd` comes
// from before fork (sysconf is not async-signal-safe).
void close_from(int lowest, int keep, int max_fd) {
#if defined(FO_LINUX_SPAWN) && defined(SYS_close_range)
  if (keep > lowest)
    syscall(SYS_close_range, lowest, keep - 1, 0);
  if (syscall(SYS_close_range, keep + 1, ~0U, 0) == 0)
    return;
#endif
  for (int fd = lowest; fd < max_fd; ++fd)
    if (fd != keep)
      ::close(fd);
}

// A pipe whose ends are closed on exec.
int make_pipe(int fds[2]) {
#ifdef FO_LINUX_SPAWN
  return ::pipe2(fds, O_CLOEXEC);
#else
  // Not atomic: a fork on another thread in between could inherit these
  // ends (it closes them on its own exec).
  if (::pipe(fds) != 0)
    return -1;
  ::fcntl(fds[0], F_SETFD, FD_CLOEXEC);
  ::fcntl(fds[1], F_SETFD, FD_CLOEXEC);
  return 0;
#endif
}

#ifndef FO_LINUX_SPAWN
// Where execvp would look for `name`, in order (computed before fork).
std::vector<std::string> exec_candidates(const std::string &name) {
  std::vector<std::string> v;
  if (name.find('/') != std::string::npos) {
    v.push_back(name);
    return v;
  }
  const char *path = std::getenv("PATH");
  std::string p = path && *path ? path : "/usr/bin:/bin:/usr/sbin:/sbin";
  size_t start = 0;
  for (;;) {
    size_t colon = p.find(':', start);
    std::string dir = p.substr(start, colon == std::string::npos ? std::string::npos : colon - start);
    v.push_back((dir.empty() ? std::string(".") : dir) + "/" + name);
    if (colon == std::string::npos)
      break;
    start = colon + 1;
  }
  return v;
}
#endif

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
    for (char **e = FO_ENVIRON; e && *e; ++e) {
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

#ifndef FO_LINUX_SPAWN
  std::vector<std::string> candidates = exec_candidates(o.argv[0]);
  char **exec_env = envp.empty() ? FO_ENVIRON : envp.data();
#endif
  long open_max = ::sysconf(_SC_OPEN_MAX);
  int max_fd = open_max > 0 && open_max < 65536 ? int(open_max) : 65536;
  (void)max_fd;

  int out[2] = {-1, -1}, err[2] = {-1, -1}, status[2] = {-1, -1};
  if (make_pipe(out) != 0 || make_pipe(status) != 0 || (o.capture_stderr && make_pipe(err) != 0)) {
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
      close_from(3, status[1], max_fd);
#ifdef FO_LINUX_SPAWN
      if (!envp.empty())
        ::execvpe(argv[0], argv.data(), envp.data());
      else
        ::execvp(argv[0], argv.data());
      code = errno;
#else
      // execvp's search, with execve so the environment can be replaced:
      // EACCES anywhere wins over "not found", other errors stop.
      bool denied = false;
      code = ENOENT;
      for (const auto &c : candidates) {
        ::execve(c.c_str(), argv.data(), exec_env);
        if (errno == EACCES)
          denied = true;
        else if (errno != ENOENT && errno != ENOTDIR) {
          code = errno;
          break;
        }
      }
      if (denied && code == ENOENT)
        code = EACCES;
#endif
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

#elif defined(FO_HAVE_WIN32_PROCESS)

// Windows: CreateProcessW with the child in a job object (killing the job
// kills its whole tree, like a POSIX process group; the job also carries
// the memory and CPU limits). Output is read by one thread per pipe, and
// run() waits on a thread of its own and delivers `done` through
// Fl::awake. Untested on real Windows so far: it is compile-checked only.

namespace {

std::wstring widen(const std::string &s) {
  if (s.empty())
    return std::wstring();
  int n = MultiByteToWideChar(CP_UTF8, 0, s.data(), int(s.size()), nullptr, 0);
  std::wstring w(size_t(n > 0 ? n : 0), L'\0');
  if (n > 0)
    MultiByteToWideChar(CP_UTF8, 0, s.data(), int(s.size()), &w[0], n);
  return w;
}

std::string last_error_text() {
  DWORD e = GetLastError();
  char buf[256];
  DWORD n = FormatMessageA(FORMAT_MESSAGE_FROM_SYSTEM | FORMAT_MESSAGE_IGNORE_INSERTS, nullptr, e, 0, buf,
                           sizeof buf, nullptr);
  while (n > 0 && (buf[n - 1] == '\n' || buf[n - 1] == '\r' || buf[n - 1] == ' '))
    --n;
  return n ? std::string(buf, n) : "error " + std::to_string(unsigned(e));
}

// One argument quoted the way the Microsoft C runtime parses it.
void append_quoted(std::wstring &cmd, const std::wstring &arg) {
  if (!arg.empty() && arg.find_first_of(L" \t\n\v\"") == std::wstring::npos) {
    cmd += arg;
    return;
  }
  cmd += L'"';
  for (size_t i = 0;; ++i) {
    size_t slashes = 0;
    while (i < arg.size() && arg[i] == L'\\') {
      ++i;
      ++slashes;
    }
    if (i == arg.size()) {
      cmd.append(slashes * 2, L'\\');
      break;
    }
    if (arg[i] == L'"') {
      cmd.append(slashes * 2 + 1, L'\\');
      cmd += L'"';
    } else {
      cmd.append(slashes, L'\\');
      cmd += arg[i];
    }
  }
  cmd += L'"';
}

// The parent's environment with the "NAME=value" overrides applied (names
// compare without case, as Windows does).
std::vector<wchar_t> environment_block(const std::vector<std::string> &extra) {
  std::vector<std::wstring> vars;
  if (LPWCH env = GetEnvironmentStringsW()) {
    for (LPWCH p = env; *p; p += wcslen(p) + 1)
      vars.emplace_back(p);
    FreeEnvironmentStringsW(env);
  }
  auto name_of = [](const std::wstring &v) {
    size_t eq = v.find(L'=', 1); // "=C:=C:\\" entries start with '='
    return v.substr(0, eq);
  };
  for (const auto &x : extra) {
    std::wstring w = widen(x);
    std::wstring name = name_of(w);
    for (auto it = vars.begin(); it != vars.end();) {
      if (CompareStringOrdinal(name_of(*it).c_str(), -1, name.c_str(), -1, TRUE) == CSTR_EQUAL)
        it = vars.erase(it);
      else
        ++it;
    }
    vars.push_back(w);
  }
  std::vector<wchar_t> block;
  for (const auto &v : vars) {
    block.insert(block.end(), v.begin(), v.end());
    block.push_back(L'\0');
  }
  block.push_back(L'\0');
  return block;
}

struct Child {
  HANDLE process = nullptr, job = nullptr, out = nullptr, err = nullptr;
  std::string error;
};

Child spawn(const ProcessOptions &o) {
  Child c;
  if (o.argv.empty() || o.argv[0].empty()) {
    c.error = "empty argv";
    return c;
  }
  std::wstring cmd;
  for (size_t i = 0; i < o.argv.size(); ++i) {
    if (i)
      cmd += L' ';
    append_quoted(cmd, widen(o.argv[i]));
  }
  SECURITY_ATTRIBUTES sa{sizeof(SECURITY_ATTRIBUTES), nullptr, TRUE};
  HANDLE out_r = nullptr, out_w = nullptr, err_r = nullptr, err_w = nullptr;
  HANDLE nul = CreateFileW(L"NUL", GENERIC_READ | GENERIC_WRITE, FILE_SHARE_READ | FILE_SHARE_WRITE, &sa,
                           OPEN_EXISTING, 0, nullptr);
  auto close_all = [&] {
    for (HANDLE h : {out_r, out_w, err_r, err_w, nul})
      if (h && h != INVALID_HANDLE_VALUE)
        CloseHandle(h);
  };
  if (!CreatePipe(&out_r, &out_w, &sa, 0) || (o.capture_stderr && !CreatePipe(&err_r, &err_w, &sa, 0))) {
    c.error = last_error_text();
    close_all();
    return c;
  }
  // Only the child's ends are inherited.
  SetHandleInformation(out_r, HANDLE_FLAG_INHERIT, 0);
  if (err_r)
    SetHandleInformation(err_r, HANDLE_FLAG_INHERIT, 0);

  HANDLE job = CreateJobObjectW(nullptr, nullptr);
  if (job) {
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION li{};
    li.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    if (o.limit_address_space) {
      li.BasicLimitInformation.LimitFlags |= JOB_OBJECT_LIMIT_PROCESS_MEMORY;
      li.ProcessMemoryLimit = SIZE_T(o.limit_address_space);
    }
    if (o.limit_cpu_s) {
      li.BasicLimitInformation.LimitFlags |= JOB_OBJECT_LIMIT_PROCESS_TIME;
      li.BasicLimitInformation.PerProcessUserTimeLimit.QuadPart = LONGLONG(o.limit_cpu_s) * 10000000LL;
    }
    SetInformationJobObject(job, JobObjectExtendedLimitInformation, &li, sizeof li);
  }

  STARTUPINFOW si{};
  si.cb = sizeof si;
  si.dwFlags = STARTF_USESTDHANDLES;
  si.hStdInput = nul;
  si.hStdOutput = out_w;
  si.hStdError = o.capture_stderr ? err_w : nul;
  PROCESS_INFORMATION pi{};
  std::vector<wchar_t> env;
  if (!o.env.empty())
    env = environment_block(o.env);
  std::wstring cwd = widen(o.cwd);
  std::vector<wchar_t> cmdline(cmd.begin(), cmd.end());
  cmdline.push_back(L'\0');
  DWORD flags = CREATE_SUSPENDED | CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT;
  BOOL ok = CreateProcessW(nullptr, cmdline.data(), nullptr, nullptr, TRUE, flags, env.empty() ? nullptr : env.data(),
                           cwd.empty() ? nullptr : cwd.c_str(), &si, &pi);
  if (!ok) {
    c.error = o.argv[0] + ": " + last_error_text();
    close_all();
    if (job)
      CloseHandle(job);
    return c;
  }
  if (job)
    AssignProcessToJobObject(job, pi.hProcess);
  ResumeThread(pi.hThread);
  CloseHandle(pi.hThread);
  CloseHandle(out_w);
  if (err_w)
    CloseHandle(err_w);
  CloseHandle(nul);
  c.process = pi.hProcess;
  c.job = job;
  c.out = out_r;
  c.err = err_r;
  return c;
}

void kill_child(const Child &c) {
  if (c.job)
    TerminateJobObject(c.job, 1);
  else if (c.process)
    TerminateProcess(c.process, 1);
}

// Reads a pipe to its end, keeping at most `cap` bytes; calls `over` once
// when the cap is passed.
void read_pipe(HANDLE h, std::string &buf, size_t cap, bool &truncated, std::function<void()> over) {
  char tmp[16384];
  DWORD n = 0;
  bool told = false;
  while (ReadFile(h, tmp, sizeof tmp, &n, nullptr) && n > 0) {
    size_t room = buf.size() < cap ? cap - buf.size() : 0;
    if (size_t(n) > room) {
      truncated = true;
      if (!told && over) {
        told = true;
        over();
      }
    }
    buf.append(tmp, size_t(n) < room ? size_t(n) : room);
  }
}

// Runs a spawned child to the end. `cancelled` is polled while waiting.
ProcessResult wait_child(Child c, const ProcessOptions &o, std::atomic<bool> *cancelled) {
  ProcessResult r;
  r.started = true;
  bool err_over = false;
  std::atomic<bool> killed{false};
  auto kill_once = [&] {
    if (!killed.exchange(true))
      kill_child(c);
  };
  std::thread out_reader([&] {
    read_pipe(c.out, r.out, o.stdout_cap, r.truncated, o.kill_on_cap ? std::function<void()>(kill_once) : nullptr);
  });
  std::thread err_reader;
  if (c.err)
    err_reader = std::thread([&] { read_pipe(c.err, r.err, o.stderr_cap, err_over, nullptr); });
  ULONGLONG deadline = o.timeout_s > 0 ? GetTickCount64() + ULONGLONG(o.timeout_s * 1000) : 0;
  for (;;) {
    DWORD wait = 50;
    if (deadline) {
      ULONGLONG now = GetTickCount64();
      if (now >= deadline) {
        r.timed_out = true;
        kill_once();
        break;
      }
      wait = DWORD(std::min<ULONGLONG>(50, deadline - now));
    }
    if (WaitForSingleObject(c.process, wait) == WAIT_OBJECT_0)
      break;
    if (cancelled && *cancelled) {
      kill_once();
      break;
    }
  }
  WaitForSingleObject(c.process, INFINITE);
  // Something else in the job may still hold the pipes: once the deadline
  // passed or the child was killed, stop reading.
  if (killed)
    TerminateJobObject(c.job, 1);
  if (killed || r.timed_out) {
    CancelIoEx(c.out, nullptr);
    if (c.err)
      CancelIoEx(c.err, nullptr);
  }
  out_reader.join();
  if (err_reader.joinable())
    err_reader.join();
  DWORD code = 0;
  GetExitCodeProcess(c.process, &code);
  if (killed)
    r.signal = 9; // as SIGKILL on POSIX: stopped by the toolkit
  else
    r.exit_code = int(code);
  CloseHandle(c.out);
  if (c.err)
    CloseHandle(c.err);
  CloseHandle(c.process);
  if (c.job)
    CloseHandle(c.job);
  return r;
}

struct AsyncJob {
  std::atomic<bool> cancelled{false};
};

std::mutex &jobs_mutex() {
  static std::mutex m;
  return m;
}
std::map<unsigned, std::shared_ptr<AsyncJob>> &jobs() {
  static std::map<unsigned, std::shared_ptr<AsyncJob>> j;
  return j;
}
unsigned next_job = 1;

} // namespace

ProcessResult run_sync(const ProcessOptions &o) {
  Child c = spawn(o);
  if (!c.process) {
    ProcessResult r;
    r.error = c.error;
    return r;
  }
  return wait_child(c, o, nullptr);
}

unsigned run(const ProcessOptions &o, std::function<void(ProcessResult)> done) {
  Child c = spawn(o);
  if (!c.process) {
    ProcessResult r;
    r.error = c.error;
    if (done)
      done(std::move(r));
    return 0;
  }
  ensure_fl_lock();
  unsigned id = next_job++;
  if (!next_job)
    next_job = 1;
  auto job = std::make_shared<AsyncJob>();
  {
    std::lock_guard<std::mutex> lock(jobs_mutex());
    jobs()[id] = job;
  }
  std::thread([c, o, job, id, done]() mutable {
    ProcessResult r = wait_child(c, o, &job->cancelled);
    auto shared = std::make_shared<ProcessResult>(std::move(r));
    tasks::post([id, done, shared] {
      {
        std::lock_guard<std::mutex> lock(jobs_mutex());
        jobs().erase(id);
      }
      if (done)
        done(std::move(*shared));
    });
  }).detach();
  return id;
}

void cancel(unsigned id) {
  std::lock_guard<std::mutex> lock(jobs_mutex());
  auto it = jobs().find(id);
  if (it != jobs().end())
    it->second->cancelled = true;
}

int running_processes() {
  std::lock_guard<std::mutex> lock(jobs_mutex());
  return int(jobs().size());
}

#else // neither POSIX nor Windows: processes are unavailable

ProcessResult run_sync(const ProcessOptions &) {
  ProcessResult r;
  r.error = "processes are not supported on this platform";
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

} // namespace

void post(std::function<void()> fn) {
  Fl::awake(on_awake, new std::function<void()>(std::move(fn)));
}

void run(std::function<void()> work, std::function<void()> done) {
  ensure_fl_lock(); // enables Fl::awake() messages
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
