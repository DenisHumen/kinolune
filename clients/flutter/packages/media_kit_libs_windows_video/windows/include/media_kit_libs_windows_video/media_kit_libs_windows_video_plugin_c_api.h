// SPDX-License-Identifier: MPL-2.0
//
// Registration entry point the Flutter tool calls from the generated plugin
// registrant (PKG-020). The package only carries native libraries.
#ifndef MEDIA_KIT_LIBS_WINDOWS_VIDEO_PLUGIN_C_API_H_
#define MEDIA_KIT_LIBS_WINDOWS_VIDEO_PLUGIN_C_API_H_

#include <flutter_plugin_registrar.h>

#ifdef FLUTTER_PLUGIN_IMPL
#define FLUTTER_PLUGIN_EXPORT __declspec(dllexport)
#else
#define FLUTTER_PLUGIN_EXPORT __declspec(dllimport)
#endif

#if defined(__cplusplus)
extern "C" {
#endif

FLUTTER_PLUGIN_EXPORT void MediaKitLibsWindowsVideoPluginCApiRegisterWithRegistrar(
    FlutterDesktopPluginRegistrarRef registrar);

#if defined(__cplusplus)
}  // extern "C"
#endif

#endif  // MEDIA_KIT_LIBS_WINDOWS_VIDEO_PLUGIN_C_API_H_
