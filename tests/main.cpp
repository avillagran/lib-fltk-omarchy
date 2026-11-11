// Test runner. Before any test runs it creates a fresh exclusive 0700
// run directory under $FO_TEST_RUNS (inside the build tree, set by ctest)
// and points TMPDIR, HOME and every XDG directory into it. Nothing is ever
// deleted; each run keeps its fixtures for inspection.
#include "harness.h"

#include <FL/Fl.H>

#include <cerrno>
#include <cstdlib>
#include <cstring>
#include <sys/stat.h>
#include <unistd.h>

namespace fo_test {

std::vector<Case> &cases() {
  static std::vector<Case> c;
  return c;
}
int failures = 0;

static std::string g_tmp;

std::string private_dir(const char *label) {
  std::string tmpl = g_tmp + "/" + label + "-XXXXXX";
  std::vector<char> buf(tmpl.begin(), tmpl.end());
  buf.push_back(0);
  if (!mkdtemp(buf.data())) {
    std::fprintf(stderr, "mkdtemp failed: %s\n", std::strerror(errno));
    std::exit(2);
  }
  return buf.data();
}

void write_file(const std::string &path, const std::string &text) {
  if (path.compare(0, g_tmp.size() + 1, g_tmp + "/") != 0 || path.find("/../") != std::string::npos) {
    std::fprintf(stderr, "refusing to write outside the test root: %s\n", path.c_str());
    std::exit(2);
  }
  FILE *f = std::fopen(path.c_str(), "wb");
  if (!f) {
    std::fprintf(stderr, "cannot write %s\n", path.c_str());
    std::exit(2);
  }
  std::fwrite(text.data(), 1, text.size(), f);
  std::fclose(f);
}

bool pump_until(bool &done, double timeout_s) {
  double waited = 0;
  while (!done && waited < timeout_s) {
    Fl::wait(0.01);
    waited += 0.01;
  }
  return done;
}

} // namespace fo_test

static void mk(const std::string &p) {
  if (mkdir(p.c_str(), 0700) != 0 && errno != EEXIST) {
    std::fprintf(stderr, "mkdir %s: %s\n", p.c_str(), std::strerror(errno));
    std::exit(2);
  }
}

int main(int argc, char **argv) {
  const char *runs = std::getenv("FO_TEST_RUNS");
  if (!runs || !*runs || runs[0] != '/') {
    std::fprintf(stderr, "FO_TEST_RUNS must be an absolute directory (run through ctest)\n");
    return 2;
  }
  mk(runs);
  std::string tmpl = std::string(runs) + "/run-XXXXXX";
  std::vector<char> buf(tmpl.begin(), tmpl.end());
  buf.push_back(0);
  if (!mkdtemp(buf.data())) {
    std::perror("mkdtemp");
    return 2;
  }
  std::string root = buf.data();
  const char *sub[] = {"tmp", "home", "config", "data", "cache", "state", "runtime"};
  for (const char *s : sub)
    mk(root + "/" + s);
  setenv("TMPDIR", (root + "/tmp").c_str(), 1);
  setenv("HOME", (root + "/home").c_str(), 1);
  setenv("XDG_CONFIG_HOME", (root + "/config").c_str(), 1);
  setenv("XDG_DATA_HOME", (root + "/data").c_str(), 1);
  setenv("XDG_CACHE_HOME", (root + "/cache").c_str(), 1);
  setenv("XDG_STATE_HOME", (root + "/state").c_str(), 1);
  setenv("XDG_RUNTIME_DIR", (root + "/runtime").c_str(), 1);
  unsetenv("HYPRLAND_INSTANCE_SIGNATURE");
  unsetenv("OMARCHY_FLTK_ANIM");
  fo_test::g_tmp = root + "/tmp";
  std::printf("test root: %s\n", root.c_str());

  const char *only = argc > 1 ? argv[1] : nullptr;
  int ran = 0;
  for (const auto &c : fo_test::cases()) {
    if (only && !std::strstr(c.name, only))
      continue;
    int before = fo_test::failures;
    c.fn();
    ++ran;
    std::printf("%s %s\n", fo_test::failures == before ? "ok  " : "FAIL", c.name);
  }
  std::printf("%d tests, %d failed checks\n", ran, fo_test::failures);
  return fo_test::failures ? 1 : 0;
}
