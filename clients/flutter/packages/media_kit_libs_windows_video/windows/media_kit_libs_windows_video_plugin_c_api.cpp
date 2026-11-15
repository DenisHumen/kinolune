// SPDX-License-Identifier: MPL-2.0
//
// Nothing to register: the package only carries libmpv, FFmpeg and ANGLE,
// which media_kit_video links and the app bundles (PKG-020).
#include "include/media_kit_libs_windows_video/media_kit_libs_windows_video_plugin_c_api.h"

void MediaKitLibsWindowsVideoPluginCApiRegisterWithRegistrar(
    FlutterDesktopPluginRegistrarRef registrar) {
  (void)registrar;
}
