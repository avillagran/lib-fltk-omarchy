cmake_minimum_required(VERSION 3.16)
project(lib-fltk-omarchy VERSION 0.1.0 LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)
if(NOT CMAKE_BUILD_TYPE AND NOT CMAKE_CONFIGURATION_TYPES)
  set(CMAKE_BUILD_TYPE Release CACHE STRING "Build type" FORCE)
endif()

option(FO_BUILD_EXAMPLES "Build the gallery example" ON)
option(FO_BUILD_TESTS "Build the unit tests" ON)
option(FLTK_OMARCHY_SHARED_FLTK "Link FLTK's shared libraries instead of the static ones" OFF)

find_package(FLTK 1.4 CONFIG REQUIRED)
find_package(Threads REQUIRED)

# FLTK is linked statically by default, so apps run without the fltk
# package installed (it is a build-time dependency only).
if(FLTK_OMARCHY_SHARED_FLTK AND TARGET fltk::fltk-shared AND TARGET fltk::images-shared)
  set(FO_FLTK_LIBS fltk::images-shared fltk::fltk-shared)
else()
  set(FO_FLTK_LIBS fltk::images fltk::fltk)
endif()

# cairo is what FLTK draws with on Wayland (and on X11 when built with
# FLTK_GRAPHICS_CAIRO); the toolkit uses the same context for translucency
# and anti-aliased rounded shapes. Optional: without it, drawing falls back
# to opaque FLTK primitives.
find_package(PkgConfig QUIET)
if(PkgConfig_FOUND)
  pkg_check_modules(FO_CAIRO QUIET IMPORTED_TARGET cairo)
endif()

add_library(fltk-omarchy STATIC
  src/anim.cpp
  src/collection.cpp
  src/dialog.cpp
  src/hyprland.cpp
  src/i18n.cpp
  src/icons.cpp
  src/json.cpp
  src/keys.cpp
  src/menu.cpp
  src/paint.cpp
  src/platform.cpp
  src/process.cpp
  src/scroll.cpp
  src/sidebar.cpp
  src/text.cpp
  src/theme.cpp
  src/widgets.cpp
  src/window.cpp
)
add_library(fltk-omarchy::fltk-omarchy ALIAS fltk-omarchy)
target_include_directories(fltk-omarchy PUBLIC
  $<BUILD_INTERFACE:${CMAKE_CURRENT_SOURCE_DIR}/include>
  $<INSTALL_INTERFACE:include>)
target_link_libraries(fltk-omarchy PUBLIC ${FO_FLTK_LIBS} Threads::Threads)
if(FO_CAIRO_FOUND)
  target_link_libraries(fltk-omarchy PRIVATE PkgConfig::FO_CAIRO)
  target_compile_definitions(fltk-omarchy PRIVATE FO_HAVE_CAIRO=1)
endif()
if(CMAKE_CXX_COMPILER_ID MATCHES "GNU|Clang")
  target_compile_options(fltk-omarchy PRIVATE -Wall -Wextra)
endif()

if(FO_BUILD_EXAMPLES)
  add_executable(gallery examples/gallery.cpp)
  target_link_libraries(gallery PRIVATE fltk-omarchy::fltk-omarchy)
  if(CMAKE_CXX_COMPILER_ID MATCHES "GNU|Clang")
    target_compile_options(gallery PRIVATE -Wall -Wextra)
  endif()
endif()

if(FO_BUILD_TESTS)
  enable_testing()
  add_executable(fo_tests
    tests/main.cpp
    tests/test_core.cpp
    tests/test_process.cpp
    tests/test_widgets.cpp
  )
  target_link_libraries(fo_tests PRIVATE fltk-omarchy::fltk-omarchy)
  target_include_directories(fo_tests PRIVATE src)
  if(CMAKE_CXX_COMPILER_ID MATCHES "GNU|Clang")
    target_compile_options(fo_tests PRIVATE -Wall -Wextra)
  endif()
  # The runner never sees the real user environment: HOME, TMPDIR and the
  # XDG directories point into the build tree before the binary starts, and
  # the binary then creates a fresh exclusive 0700 run directory under
  # test-runs/ (never deleted) and moves every variable inside it.
  set(_fo_runs "${CMAKE_BINARY_DIR}/test-runs")
  add_test(NAME fo_tests COMMAND ${CMAKE_COMMAND} -E env
    "FO_TEST_RUNS=${_fo_runs}"
    "HOME=${_fo_runs}/unset-home"
    "TMPDIR=${_fo_runs}/unset-tmp"
    "XDG_CONFIG_HOME=${_fo_runs}/unset-config"
    "XDG_DATA_HOME=${_fo_runs}/unset-data"
    "XDG_CACHE_HOME=${_fo_runs}/unset-cache"
    "XDG_STATE_HOME=${_fo_runs}/unset-state"
    "XDG_RUNTIME_DIR=${_fo_runs}/unset-runtime"
    --unset=HYPRLAND_INSTANCE_SIGNATURE --unset=OMARCHY_PATH
    --unset=OMARCHY_FLTK_ANIM --unset=OMARCHY_FLTK_FONT --unset=DISPLAY
    --unset=WAYLAND_DISPLAY
    $<TARGET_FILE:fo_tests>)
endif()
