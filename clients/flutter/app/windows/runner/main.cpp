#include <flutter/dart_project.h>
#include <flutter/flutter_view_controller.h>
#include <windows.h>

#include <filesystem>
#include <string>
#include <vector>

#include "flutter_window.h"
#include "launch_links.h"
#include "utils.h"

namespace {

// Single instance (PKG-020, UI-022): a second start brings the running
// window to the front and hands it its arguments (`kinolune://` links,
// files from "Open with") instead of opening another copy on the same
// library database.
constexpr wchar_t kInstanceMutex[] = L"Local\\app.kinolune.Kinolune";
constexpr wchar_t kWindowClass[] = L"FLUTTER_RUNNER_WIN32_WINDOW";
constexpr wchar_t kWindowTitle[] = L"Kinolune";

// Relative file arguments are made absolute here: the running copy has
// another working directory.
std::vector<std::string> AbsoluteArguments(std::vector<std::string> args) {
  for (auto& arg : args) {
    if (arg.empty() || arg[0] == '-' || arg.find("://") != std::string::npos ||
        arg.rfind("file:", 0) == 0) {
      continue;
    }
    std::error_code error;
    const auto path = std::filesystem::u8path(arg);
    const auto absolute = std::filesystem::absolute(path, error);
    if (!error) {
      // u8string() is std::string before C++20 and std::u8string after;
      // copying by iterators works for both (plugins may switch to C++20).
      const auto utf8 = absolute.u8string();
      arg.assign(utf8.begin(), utf8.end());
    }
  }
  return args;
}

bool ActivateRunningInstance(const std::vector<std::string>& args) {
  HWND existing = ::FindWindow(kWindowClass, kWindowTitle);
  if (existing == nullptr) {
    return false;
  }
  if (::IsIconic(existing)) {
    ::ShowWindow(existing, SW_RESTORE);
  }
  ::SetForegroundWindow(existing);
  if (!args.empty()) {
    const std::string payload = launch_links::Pack(AbsoluteArguments(args));
    COPYDATASTRUCT data{};
    data.dwData = launch_links::kCopyDataId;
    data.cbData = static_cast<DWORD>(payload.size());
    data.lpData = const_cast<char*>(payload.data());
    ::SendMessageTimeout(existing, WM_COPYDATA, 0,
                         reinterpret_cast<LPARAM>(&data), SMTO_ABORTIFHUNG,
                         5000, nullptr);
  }
  return true;
}

}  // namespace

int APIENTRY wWinMain(_In_ HINSTANCE instance, _In_opt_ HINSTANCE prev,
                      _In_ wchar_t *command_line, _In_ int show_command) {
  std::vector<std::string> command_line_arguments =
      GetCommandLineArguments();

  HANDLE instance_mutex = ::CreateMutex(nullptr, TRUE, kInstanceMutex);
  if (instance_mutex != nullptr && ::GetLastError() == ERROR_ALREADY_EXISTS) {
    ActivateRunningInstance(command_line_arguments);
    ::CloseHandle(instance_mutex);
    return EXIT_SUCCESS;
  }

  // Attach to console when present (e.g., 'flutter run') or create a
  // new console when running with a debugger.
  if (!::AttachConsole(ATTACH_PARENT_PROCESS) && ::IsDebuggerPresent()) {
    CreateAndAttachConsole();
  }

  // Initialize COM, so that it is available for use in the library and/or
  // plugins.
  ::CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED);

  flutter::DartProject project(L"data");

  project.set_dart_entrypoint_arguments(
      AbsoluteArguments(std::move(command_line_arguments)));

  FlutterWindow window(project);
  Win32Window::Point origin(10, 10);
  Win32Window::Size size(1280, 720);
  if (!window.Create(kWindowTitle, origin, size)) {
    return EXIT_FAILURE;
  }
  window.SetQuitOnClose(true);

  ::MSG msg;
  while (::GetMessage(&msg, nullptr, 0, 0)) {
    ::TranslateMessage(&msg);
    ::DispatchMessage(&msg);
  }

  ::CoUninitialize();
  if (instance_mutex != nullptr) {
    ::ReleaseMutex(instance_mutex);
    ::CloseHandle(instance_mutex);
  }
  return EXIT_SUCCESS;
}
