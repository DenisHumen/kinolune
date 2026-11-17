// SPDX-License-Identifier: MPL-2.0
//
// PKG-021: media_kit_video's macOS GPU output asks CGL for an *accelerated*
// OpenGL 3.2 pixel format and force-unwraps the result
// (OpenGLHelpers.createPixelFormat). Where there is no accelerated renderer
// (virtual machines such as the CI runners, some remote sessions) that is
// nil and the app dies with SIGTRAP as soon as a video opens. Ask CGL the
// same question first and use the CPU output when the answer is no.

import 'dart:ffi';
import 'dart:io' show Platform;

import 'package:flutter/foundation.dart' show debugPrint;

typedef _ChooseC =
    Int32 Function(Pointer<Int32>, Pointer<Pointer<Void>>, Pointer<Int32>);
typedef _ChooseDart =
    int Function(Pointer<Int32>, Pointer<Pointer<Void>>, Pointer<Int32>);
typedef _ReleaseC = Void Function(Pointer<Void>);
typedef _ReleaseDart = void Function(Pointer<Void>);
typedef _MallocC = Pointer<Void> Function(IntPtr);
typedef _MallocDart = Pointer<Void> Function(int);
typedef _FreeC = Void Function(Pointer<Void>);
typedef _FreeDart = void Function(Pointer<Void>);

/// The attributes of media_kit_video 2.0's createPixelFormat (CGLTypes.h
/// values), zero-terminated.
const _attributes = <int>[
  99, 0x3200, // kCGLPFAOpenGLProfile, kCGLOGLPVersion_3_2_Core
  73, // kCGLPFAAccelerated
  5, // kCGLPFADoubleBuffer
  8, 64, // kCGLPFAColorSize 64
  58, // kCGLPFAColorFloat
  76, // kCGLPFABackingStore
  96, // kCGLPFAAllowOfflineRenderers
  101, // kCGLPFASupportsAutomaticGraphicsSwitching
  0,
];

bool? _gpuOutput;

/// Whether media_kit_video may use its GPU (OpenGL) output. Always true off
/// macOS; on macOS true when CGL has an accelerated renderer for it.
bool gpuVideoOutputAvailable() {
  if (!Platform.isMacOS) return true;
  return _gpuOutput ??= _probe();
}

bool _probe() {
  try {
    final gl = DynamicLibrary.open(
      '/System/Library/Frameworks/OpenGL.framework/OpenGL',
    );
    final choose = gl.lookupFunction<_ChooseC, _ChooseDart>(
      'CGLChoosePixelFormat',
    );
    final release = gl.lookupFunction<_ReleaseC, _ReleaseDart>(
      'CGLReleasePixelFormat',
    );
    final libc = DynamicLibrary.process();
    final malloc = libc.lookupFunction<_MallocC, _MallocDart>('malloc');
    final free = libc.lookupFunction<_FreeC, _FreeDart>('free');

    final attrs = malloc(_attributes.length * sizeOf<Int32>()).cast<Int32>();
    final pix = malloc(sizeOf<Pointer<Void>>()).cast<Pointer<Void>>();
    final npix = malloc(sizeOf<Int32>()).cast<Int32>();
    try {
      for (var i = 0; i < _attributes.length; i++) {
        attrs[i] = _attributes[i];
      }
      pix.value = nullptr;
      final err = choose(attrs, pix, npix);
      final ok = err == 0 && pix.value != nullptr;
      if (pix.value != nullptr) release(pix.value);
      if (!ok) {
        debugPrint(
          'pa_player: no accelerated OpenGL renderer (CGL error $err), '
          'video uses the CPU output',
        );
      }
      return ok;
    } finally {
      free(attrs.cast());
      free(pix.cast());
      free(npix.cast());
    }
  } catch (e) {
    // Cannot ask: keep media_kit's default.
    debugPrint('pa_player: OpenGL probe failed: $e');
    return true;
  }
}
