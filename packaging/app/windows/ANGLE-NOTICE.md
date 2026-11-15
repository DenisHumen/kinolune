<!-- SPDX-License-Identifier: MPL-2.0 -->
# ANGLE in the Windows build of Kinolune

The video output of media_kit on Windows draws through OpenGL ES on Direct3D 11
with [ANGLE](https://chromium.googlesource.com/angle/angle). Kinolune ships the
prebuilt ANGLE that media_kit uses (archive `ANGLE.7z` of
`alexmercerind/flutter-windows-ANGLE-OpenGL-ES` v1.0.1, pinned by SHA-256 in
`packaging/app/windows/mpv-libs.sh`), only the DLLs it needs:

| File | Component | Licence |
|---|---|---|
| `libEGL.dll`, `libGLESv2.dll` | ANGLE | BSD-3-Clause (`LICENSE` in this folder) |
| `libc++.dll` (if present) | LLVM libc++ | Apache-2.0 WITH LLVM-exception |
| `zlib.dll` (if present) | zlib | Zlib |

`d3dcompiler_47.dll` is not shipped: Windows 10 and 11 include it.

Texts of Apache-2.0 and the other licences: `kinolune-<version>-licenses.tar.gz`
of the same GitHub Release.
