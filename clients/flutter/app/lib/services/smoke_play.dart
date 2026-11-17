// SPDX-License-Identifier: MPL-2.0
//
// PKG-021: release smoke test of playback. With KINOLUNE_SMOKE_PLAY=<file>
// in the environment (desktop), the app opens the player on that local
// file a few seconds after the first frame, the way the file browser does,
// and prints one line:
//
//   KINOLUNE_SMOKE_PLAY open /path/to/sample.avi
//
// The release workflow starts the packaged app like this and checks that it
// survives playback (alpha.11 on macOS crashed as soon as a video started).
// Off by default: without the variable nothing is registered.

import 'dart:async';
import 'dart:io';

import 'package:material_ui/material_ui.dart';

import '../integration/player.dart';
import 'app_services.dart';

/// Opens the player on `KINOLUNE_SMOKE_PLAY` when it is set; call once
/// from the app's root state.
void armSmokePlay(
  GlobalKey<NavigatorState> navigator,
  AppServices Function() services,
) {
  final path = Platform.environment['KINOLUNE_SMOKE_PLAY'] ?? '';
  if (path.isEmpty) return;
  WidgetsBinding.instance.addPostFrameCallback((_) {
    Timer(const Duration(seconds: 3), () {
      final nav = navigator.currentState;
      if (nav == null) return;
      final s = services();
      // ignore: avoid_print
      print('KINOLUNE_SMOKE_PLAY open $path');
      unawaited(
        nav.push(
          MaterialPageRoute<void>(
            fullscreenDialog: true,
            builder: (_) => PlayerScreen(
              request: PlayRequest(
                uri: path,
                title: path.split(Platform.pathSeparator).last,
              ),
              onProgress: (_, _) {},
              engineFactory: s.playerEngineFactory,
              subtitleSettings: s.subtitleSettings,
              subtitleLanguages: s.subtitleLanguages,
              seekStep: Duration(seconds: s.skipSeconds.value),
            ),
          ),
        ),
      );
    });
  });
}
