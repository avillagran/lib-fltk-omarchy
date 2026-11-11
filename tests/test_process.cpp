#include "harness.h"

#include "fltk-omarchy/platform.h"
#include "fltk-omarchy/process.h"

#include <FL/Fl.H>

#include <cerrno>
#include <chrono>
#include <csignal>
#include <cstdlib>
#include <cstring>
#include <thread>

using namespace fo;

static double seconds_since(std::chrono::steady_clock::time_point t) {
  return std::chrono::duration<double>(std::chrono::steady_clock::now() - t).count();
}

TEST(process_captures_and_reports_exit) {
  ProcessOptions o;
  o.argv = {"printf", "hello %s", "world"};
  ProcessResult r = run_sync(o);
  CHECK(r.started && r.ok());
  CHECK_STR(r.out, "hello world");
  o.argv = {"sh", "-c", "echo oops >&2; exit 3"};
  o.capture_stderr = true;
  r = run_sync(o);
  CHECK_EQ(r.exit_code, 3);
  CHECK_STR(r.err, "oops\n");
  CHECK(!r.ok());
}

TEST(process_exec_failure_is_reported) {
  ProcessOptions o;
  o.argv = {"fo-no-such-program-xyz"};
  ProcessResult r = run_sync(o);
  CHECK(!r.started);
  CHECK(r.error.find("fo-no-such-program-xyz") != std::string::npos);
  o.argv = {};
  CHECK(!run_sync(o).started);
}

TEST(process_exec_searches_path_and_reports_permission) {
  // A non-executable file found on PATH: "Permission denied", as execvp
  // reports it (also on the portable POSIX path, which searches itself).
  std::string dir = fo_test::private_dir("path");
  fo_test::write_file(dir + "/fo-not-executable", "#!/bin/sh\necho hi\n");
  std::string old = platform::env("PATH");
  setenv("PATH", (dir + ":" + old).c_str(), 1);
  ProcessOptions o;
  o.argv = {"fo-not-executable"};
  ProcessResult r = run_sync(o);
  CHECK(!r.started);
  CHECK(r.error.find(std::strerror(EACCES)) != std::string::npos);
  // With extra environment entries (execve with a new environment).
  o.argv = {"sh", "-c", "printf %s \"$FO_X\""};
  o.env = {"FO_X=found on PATH"};
  r = run_sync(o);
  CHECK(r.ok());
  CHECK_STR(r.out, "found on PATH");
  setenv("PATH", old.c_str(), 1);
}

TEST(process_timeout_kills_the_group) {
  ProcessOptions o;
  // The grandchild sleep must die too (killed as a process group).
  o.argv = {"sh", "-c", "sleep 30 & sleep 30"};
  o.timeout_s = 0.3;
  auto t0 = std::chrono::steady_clock::now();
  ProcessResult r = run_sync(o);
  CHECK(r.timed_out);
  CHECK_EQ(r.signal, SIGKILL);
  CHECK(seconds_since(t0) < 3);
}

TEST(process_stdout_cap) {
  ProcessOptions o;
  o.argv = {"head", "-c", "100000", "/dev/zero"};
  o.stdout_cap = 1000;
  o.kill_on_cap = false;
  ProcessResult r = run_sync(o);
  CHECK(r.truncated);
  CHECK_EQ(r.out.size(), size_t(1000));
  CHECK_EQ(r.exit_code, 0);
  // Runaway output is killed once it passes the cap.
  o.argv = {"yes"};
  o.kill_on_cap = true;
  o.timeout_s = 5;
  auto t0 = std::chrono::steady_clock::now();
  r = run_sync(o);
  CHECK(r.truncated && !r.timed_out);
  CHECK_EQ(r.signal, SIGKILL);
  CHECK(seconds_since(t0) < 3);
}

TEST(process_cwd_env_and_limits) {
  std::string dir = fo_test::private_dir("proc");
  ProcessOptions o;
  o.argv = {"sh", "-c", "pwd; echo $FO_TEST_VALUE"};
  o.cwd = dir;
  o.env = {"FO_TEST_VALUE=42"};
  ProcessResult r = run_sync(o);
  CHECK_STR(r.out, dir + "\n42\n");
  // A CPU limit stops a busy loop.
  o = ProcessOptions();
  o.argv = {"sh", "-c", "while :; do :; done"};
  o.limit_cpu_s = 1;
  o.timeout_s = 10;
  r = run_sync(o);
  CHECK(!r.timed_out);
  CHECK(r.signal == SIGXCPU || r.signal == SIGKILL);
  // An address-space limit makes a big allocation fail.
  o = ProcessOptions();
  o.argv = {"sh", "-c", "head -c 300000000 /dev/zero | tr '\\0' 'x' | (v=$(cat); echo ${#v})"};
  o.limit_address_space = 64ull << 20;
  o.timeout_s = 20;
  r = run_sync(o);
  CHECK(!r.ok());
}

TEST(process_async_delivers_on_the_loop) {
  bool done = false;
  ProcessResult got;
  ProcessOptions o;
  o.argv = {"sh", "-c", "echo async; sleep 0.1"};
  unsigned id = run(o, [&](ProcessResult r) {
    got = std::move(r);
    done = true;
  });
  CHECK(id != 0);
  CHECK_EQ(running_processes(), 1);
  CHECK(fo_test::pump_until(done, 5));
  CHECK(got.ok());
  CHECK_STR(got.out, "async\n");
  CHECK_EQ(running_processes(), 0);

  // Timeout and cancel.
  done = false;
  o.argv = {"sleep", "30"};
  o.timeout_s = 0.2;
  run(o, [&](ProcessResult r) {
    got = std::move(r);
    done = true;
  });
  CHECK(fo_test::pump_until(done, 5));
  CHECK(got.timed_out);
  done = false;
  o.timeout_s = 0;
  id = run(o, [&](ProcessResult r) {
    got = std::move(r);
    done = true;
  });
  cancel(id);
  CHECK(fo_test::pump_until(done, 5));
  CHECK_EQ(got.signal, SIGKILL);

  // Cap on the async path.
  done = false;
  o = ProcessOptions();
  o.argv = {"yes"};
  o.stdout_cap = 4096;
  run(o, [&](ProcessResult r) {
    got = std::move(r);
    done = true;
  });
  CHECK(fo_test::pump_until(done, 5));
  CHECK(got.truncated);
  CHECK_EQ(got.out.size(), size_t(4096));
}

TEST(tasks_run_off_thread_and_finish_on_loop) {
  bool done = false;
  std::thread::id worker, main_id = std::this_thread::get_id(), finished;
  tasks::run([&] { worker = std::this_thread::get_id(); },
             [&] {
               finished = std::this_thread::get_id();
               done = true;
             });
  CHECK(fo_test::pump_until(done, 5));
  CHECK(worker != main_id);
  CHECK(finished == main_id);
  CHECK_EQ(tasks::pending(), 0);
}
