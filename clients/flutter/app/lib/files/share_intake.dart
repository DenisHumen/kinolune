// SPDX-License-Identifier: MPL-2.0
//
// UI-035: «Поделиться → Kinolune» on Android (ShareBridge.kt, channel
// `app.kinolune/share`). Another app shares photos or videos; the platform
// copies them into the app's cache and hands the paths here; the shell
// uploads them to «Личное». iOS needs a Share Extension target and an App
// Group (signing), which this repository does not build yet (ADR-0110).

import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

const _shareChannel = MethodChannel('app.kinolune/share');

/// Receives shared files and hands their local paths to [onShared].
class ShareIntake {
  ShareIntake({required this.onShared, this.channel = _shareChannel});

  final Future<void> Function(List<String> paths) onShared;
  final MethodChannel channel;
  bool _disposed = false;

  /// Listens for shares and takes one that started the app.
  Future<void> start() async {
    channel.setMethodCallHandler(_onCall);
    try {
      final first = await channel.invokeListMethod<String>('take');
      if (first != null && first.isNotEmpty && !_disposed) {
        unawaited(onShared(first));
      }
    } on MissingPluginException {
      return;
    } on PlatformException catch (e) {
      debugPrint('Share intake unavailable: $e');
    }
  }

  Future<Object?> _onCall(MethodCall call) async {
    if (call.method == 'shared' && call.arguments is List && !_disposed) {
      final paths = [
        for (final p in call.arguments as List)
          if (p is String) p,
      ];
      if (paths.isNotEmpty) unawaited(onShared(paths));
      return true;
    }
    throw MissingPluginException();
  }

  void dispose() {
    _disposed = true;
    channel.setMethodCallHandler(null);
  }
}
