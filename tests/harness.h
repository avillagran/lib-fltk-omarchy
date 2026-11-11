// A tiny zero-dependency test harness.
#pragma once

#include <cmath>
#include <cstdio>
#include <string>
#include <vector>

namespace fo_test {

struct Case {
  const char *name;
  void (*fn)();
};
std::vector<Case> &cases();
extern int failures;
struct Register {
  Register(const char *name, void (*fn)()) { cases().push_back({name, fn}); }
};

/// A fresh exclusive 0700 directory under $TMPDIR, never deleted.
std::string private_dir(const char *label);
/// Writes a file inside a fixture directory (refuses paths outside $TMPDIR).
void write_file(const std::string &path, const std::string &text);
/// Runs the FLTK loop until `done` or `timeout_s` passes.
bool pump_until(bool &done, double timeout_s);

inline void fail(const char *file, int line, const std::string &what) {
  std::fprintf(stderr, "  FAIL %s:%d: %s\n", file, line, what.c_str());
  ++failures;
}

} // namespace fo_test

#define TEST(name)                                                                                 \
  static void name();                                                                              \
  static fo_test::Register reg_##name(#name, name);                                                \
  static void name()

#define CHECK(cond)                                                                                \
  do {                                                                                             \
    if (!(cond))                                                                                   \
      fo_test::fail(__FILE__, __LINE__, #cond);                                                    \
  } while (0)

#define CHECK_EQ(a, b)                                                                             \
  do {                                                                                             \
    auto va_ = (a);                                                                                \
    auto vb_ = (b);                                                                                \
    if (!(va_ == vb_))                                                                             \
      fo_test::fail(__FILE__, __LINE__, std::string(#a " == " #b));                                \
  } while (0)

#define CHECK_STR(a, b)                                                                            \
  do {                                                                                             \
    std::string va_ = (a), vb_ = (b);                                                              \
    if (va_ != vb_)                                                                                \
      fo_test::fail(__FILE__, __LINE__, std::string(#a " == \"") + vb_ + "\", got \"" + va_ + "\""); \
  } while (0)

#define CHECK_NEAR(a, b, eps)                                                                      \
  do {                                                                                             \
    double va_ = (a), vb_ = (b);                                                                   \
    if (std::fabs(va_ - vb_) > (eps))                                                              \
      fo_test::fail(__FILE__, __LINE__,                                                            \
                    std::string(#a " ~ " #b ": ") + std::to_string(va_) + " vs " + std::to_string(vb_)); \
  } while (0)
