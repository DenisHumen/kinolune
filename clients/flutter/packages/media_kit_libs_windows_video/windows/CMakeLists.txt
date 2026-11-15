# SPDX-License-Identifier: MPL-2.0
#
# Windows native libraries for package:media_kit_video (PKG-020, ADR-NNNN):
# our LGPL-2.1 libmpv + FFmpeg (tools/mpv) and ANGLE, prepared in libs/ by
# packaging/app/windows/mpv-libs.sh. Nothing is downloaded here.
cmake_minimum_required(VERSION 3.14)

# Read by media_kit_video's CMakeLists.txt (this package is configured first:
# plugins are added in alphabetical order).
option(MEDIA_KIT_LIBS_AVAILABLE "package:media_kit libraries are available." ON)

set(PROJECT_NAME "media_kit_libs_windows_video")
project(${PROJECT_NAME} LANGUAGES CXX)

set(PA_LIBS_DIR "${CMAKE_CURRENT_SOURCE_DIR}/libs")
if(NOT EXISTS "${PA_LIBS_DIR}/libmpv/libmpv.dll.a"
   OR NOT EXISTS "${PA_LIBS_DIR}/libmpv/libmpv-2.dll"
   OR NOT EXISTS "${PA_LIBS_DIR}/ANGLE/lib/libEGL.dll.lib")
  message(FATAL_ERROR
    "media_kit_libs_windows_video: no libmpv/ANGLE in ${PA_LIBS_DIR}. Run "
    "packaging/app/windows/mpv-libs.sh "
    "clients/flutter/packages/media_kit_libs_windows_video/windows/libs "
    "<libmpv-windows-x86_64> first (see packaging/app/windows/README.md).")
endif()

# media_kit_video expects exactly these two directories.
set(LIBMPV_SRC "${CMAKE_BINARY_DIR}/libmpv")
set(ANGLE_SRC "${CMAKE_BINARY_DIR}/ANGLE")
file(REMOVE_RECURSE "${LIBMPV_SRC}" "${ANGLE_SRC}")
file(COPY "${PA_LIBS_DIR}/libmpv/" DESTINATION "${LIBMPV_SRC}")
file(COPY "${PA_LIBS_DIR}/ANGLE/" DESTINATION "${ANGLE_SRC}")
file(GLOB PA_LIBMPV_DLLS "${LIBMPV_SRC}/*.dll")
file(GLOB PA_ANGLE_DLLS "${ANGLE_SRC}/*.dll")

set(PLUGIN_NAME "media_kit_libs_windows_video_plugin")

add_library(
  ${PLUGIN_NAME} SHARED
  "include/media_kit_libs_windows_video/media_kit_libs_windows_video_plugin_c_api.h"
  "media_kit_libs_windows_video_plugin_c_api.cpp"
)

apply_standard_settings(${PLUGIN_NAME})

set_target_properties(${PLUGIN_NAME} PROPERTIES CXX_VISIBILITY_PRESET hidden)
target_compile_definitions(${PLUGIN_NAME} PRIVATE FLUTTER_PLUGIN_IMPL)
target_include_directories(${PLUGIN_NAME} INTERFACE "${CMAKE_CURRENT_SOURCE_DIR}/include")
target_link_libraries(${PLUGIN_NAME} PRIVATE flutter flutter_wrapper_plugin)

# Copied next to Kinolune.exe by the runner's install step.
set(
  media_kit_libs_windows_video_bundled_libraries
  ${PA_LIBMPV_DLLS}
  ${PA_ANGLE_DLLS}
  PARENT_SCOPE
)
