// Minimal Hyprland IPC: queries on the request socket and the event
// stream, both non-blocking on the FLTK thread. Only reads state; never
// edits the user's Hyprland configuration. Without Hyprland (or off
// Linux) `available()` is false and requests fail at once.
#pragma once

#include <functional>
#include <string>

namespace fo {
namespace hypr {

/// True when HYPRLAND_INSTANCE_SIGNATURE is set and its socket exists.
bool available();
/// Request socket path (or the event socket with `events`), "" without Hyprland.
std::string socket_path(bool events = false);

using Reply = std::function<void(bool ok, const std::string &reply)>;

/// Sends `request` (for example "j/getoption decoration:rounding") and
/// delivers the reply on the FLTK thread. Gives up after `timeout_s`.
void request(const std::string &request, Reply done, double timeout_s = 0.5);

/// Blocking variant for worker threads and tools.
bool request_sync(const std::string &request, std::string &reply, int timeout_ms = 500);

/// Subscribes to the event socket. `fn(event, data)` runs on the FLTK
/// thread for every `event>>data` line. Returns an id (0 when Hyprland is
/// not running).
int subscribe(std::function<void(const std::string &event, const std::string &data)> fn);
void unsubscribe(int id);

/// `getoption` reply parsers: {"int": N} or {"bool": b} (newer Hyprland).
bool parse_option_int(const std::string &json, int &out);
bool parse_option_bool(const std::string &json, bool &out);
/// Refresh rate of the focused monitor in a `j/monitors` reply (else the
/// first one); 0 when unknown.
double parse_refresh_rate(const std::string &monitors_json);

} // namespace hypr
} // namespace fo
