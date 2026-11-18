// SPDX-License-Identifier: MPL-2.0
#ifndef RUNNER_LAUNCH_LINKS_H_
#define RUNNER_LAUNCH_LINKS_H_

// UI-022 (ADR-NNNN): the arguments of a second start travel to the running
// window as one WM_COPYDATA message: UTF-8 strings, each ended by '\0'. The
// window hands them to Dart on the channel `app.kinolune/links`
// (lib/platform/app_links.dart).

#include <string>
#include <vector>

namespace launch_links {

// COPYDATASTRUCT::dwData of the message ("KLNK").
constexpr unsigned long kCopyDataId = 0x4B4C4E4B;

inline std::string Pack(const std::vector<std::string>& args) {
  std::string out;
  for (const auto& arg : args) {
    out.append(arg);
    out.push_back('\0');
  }
  return out;
}

inline std::vector<std::string> Unpack(const char* data, size_t size) {
  std::vector<std::string> out;
  std::string current;
  for (size_t i = 0; i < size; ++i) {
    if (data[i] == '\0') {
      out.push_back(current);
      current.clear();
    } else {
      current.push_back(data[i]);
    }
  }
  if (!current.empty()) {
    out.push_back(current);
  }
  return out;
}

}  // namespace launch_links

#endif  // RUNNER_LAUNCH_LINKS_H_
