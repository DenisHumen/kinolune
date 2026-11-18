#include "flutter_window.h"

#include <flutter/method_result_functions.h>
#include <flutter/standard_method_codec.h>

#include <optional>

#include "flutter/generated_plugin_registrant.h"
#include "launch_links.h"

FlutterWindow::FlutterWindow(const flutter::DartProject& project)
    : project_(project) {}

FlutterWindow::~FlutterWindow() {}

bool FlutterWindow::OnCreate() {
  if (!Win32Window::OnCreate()) {
    return false;
  }

  RECT frame = GetClientArea();

  // The size here must match the window dimensions to avoid unnecessary surface
  // creation / destruction in the startup path.
  flutter_controller_ = std::make_unique<flutter::FlutterViewController>(
      frame.right - frame.left, frame.bottom - frame.top, project_);
  // Ensure that basic setup of the controller was successful.
  if (!flutter_controller_->engine() || !flutter_controller_->view()) {
    return false;
  }
  RegisterPlugins(flutter_controller_->engine());
  SetChildContent(flutter_controller_->view()->GetNativeWindow());

  // UI-022: `take` gives Dart the arguments of second starts that came
  // before it listened; new ones arrive as `open` (OpenLinks).
  links_channel_ =
      std::make_unique<flutter::MethodChannel<flutter::EncodableValue>>(
          flutter_controller_->engine()->messenger(), "app.kinolune/links",
          &flutter::StandardMethodCodec::GetInstance());
  links_channel_->SetMethodCallHandler(
      [this](const flutter::MethodCall<flutter::EncodableValue>& call,
             std::unique_ptr<flutter::MethodResult<flutter::EncodableValue>>
                 result) {
        if (call.method_name() != "take") {
          result->NotImplemented();
          return;
        }
        flutter::EncodableList list;
        for (const auto& arg : pending_links_) {
          list.emplace_back(arg);
        }
        pending_links_.clear();
        result->Success(flutter::EncodableValue(list));
      });

  flutter_controller_->engine()->SetNextFrameCallback([&]() {
    this->Show();
  });

  // Flutter can complete the first frame before the "show window" callback is
  // registered. The following call ensures a frame is pending to ensure the
  // window is shown. It is a no-op if the first frame hasn't completed yet.
  flutter_controller_->ForceRedraw();

  return true;
}

void FlutterWindow::OpenLinks(std::vector<std::string> args) {
  if (args.empty()) {
    return;
  }
  if (!links_channel_) {
    pending_links_.insert(pending_links_.end(), args.begin(), args.end());
    return;
  }
  flutter::EncodableList list;
  for (const auto& arg : args) {
    list.emplace_back(arg);
  }
  // Dart answers true once it took them; otherwise they wait for `take`.
  auto keep = [this, args]() {
    pending_links_.insert(pending_links_.end(), args.begin(), args.end());
  };
  links_channel_->InvokeMethod(
      "open", std::make_unique<flutter::EncodableValue>(list),
      std::make_unique<flutter::MethodResultFunctions<flutter::EncodableValue>>(
          [keep](const flutter::EncodableValue* result) {
            const bool* taken =
                result ? std::get_if<bool>(result) : nullptr;
            if (taken == nullptr || !*taken) {
              keep();
            }
          },
          [keep](const std::string&, const std::string&,
                 const flutter::EncodableValue*) { keep(); },
          [keep]() { keep(); }));
}

void FlutterWindow::OnDestroy() {
  links_channel_ = nullptr;
  if (flutter_controller_) {
    flutter_controller_ = nullptr;
  }

  Win32Window::OnDestroy();
}

LRESULT
FlutterWindow::MessageHandler(HWND hwnd, UINT const message,
                              WPARAM const wparam,
                              LPARAM const lparam) noexcept {
  // Give Flutter, including plugins, an opportunity to handle window messages.
  if (flutter_controller_) {
    std::optional<LRESULT> result =
        flutter_controller_->HandleTopLevelWindowProc(hwnd, message, wparam,
                                                      lparam);
    if (result) {
      return *result;
    }
  }

  switch (message) {
    case WM_FONTCHANGE:
      flutter_controller_->engine()->ReloadSystemFonts();
      break;
    case WM_COPYDATA: {
      // UI-022: a second start of the app (main.cpp) hands over its
      // arguments.
      const auto* data = reinterpret_cast<const COPYDATASTRUCT*>(lparam);
      if (data != nullptr && data->dwData == launch_links::kCopyDataId) {
        OpenLinks(launch_links::Unpack(
            static_cast<const char*>(data->lpData), data->cbData));
        return TRUE;
      }
      break;
    }
  }

  return Win32Window::MessageHandler(hwnd, message, wparam, lparam);
}
