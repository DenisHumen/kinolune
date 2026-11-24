// SPDX-License-Identifier: MPL-2.0
//
// BUG-015: seeking in the player broke the semantics tree (`Null check
// operator` in `_updateSemanticsNodeGeometry`) on every live QA run on
// Linux, where the GTK accessibility bridge keeps semantics on. The same
// path here: the film page, «Смотреть», the remote's fast-forward key and a
// jump to 3:00, with semantics on.

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_library.dart';
import 'package:kinolune/integration/library.dart';
import 'package:kinolune/screens/movie_detail_screen.dart';
import 'package:kinolune/services/app_services.dart';
import 'package:kinolune/services/secret_storage.dart';
import 'package:kinolune/services/secure_store.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:material_ui/material_ui.dart';
import 'package:pa_player/pa_player.dart';
import 'package:pa_player/testing.dart';

import 'support/test_app.dart';

final _now = DateTime(2026, 11, 20, 12);

/// Frames of 16 ms for [d]; the playhead moves with them while [playing].
Future<void> _frames(
  WidgetTester t,
  Duration d, {
  FakePlayerEngine? playing,
}) async {
  const step = Duration(milliseconds: 16);
  for (var at = Duration.zero; at < d; at += step) {
    playing?.advance(step);
    await t.pump(step);
    expect(t.takeException(), isNull);
  }
}

void main() {
  for (final (platform, banner) in [
    (TargetPlatform.linux, true),
    (TargetPlatform.linux, false),
    (TargetPlatform.android, false),
  ]) {
    testWidgets('seek by key and to 3:00 with semantics on ($platform, '
        'banner $banner)', (
      t,
    ) async {
      debugDefaultTargetPlatformOverride = platform;
      final semantics = t.ensureSemantics();
      final engine = FakePlayerEngine(
        loadedDuration: const Duration(minutes: 10),
      );
      try {
        final s = AppServices(
          library: FakeLibrary.items([
            LibraryItem(
              id: 'm-bunny',
              kind: ItemKind.movie,
              title: 'Big Buck Bunny',
              year: 2008,
              filePath: '/media/Movies/Big Buck Bunny (2008).mkv',
              durationMs: 10 * 60000,
              addedAt: _now,
            ),
          ]),
          secureStore: MemorySecureStore(),
          playerEngineFactory: ({bool video = true}) => engine,
          clock: () => _now,
          secretStorage: banner
              ? const SecretStorageInfo(
                  backend: 'encrypted-file',
                  fallbackKind: SecretFallbackKind.unavailable,
                  fallbackReason: 'no Secret Service',
                )
              : const SecretStorageInfo.unknown(),
        );
        await s.init();
        setSurface(t, const Size(1280, 720));
        await t.pumpWidget(testApp(s, section: Section.movies));
        await settle(t, 10);
        await t.tap(find.text('Big Buck Bunny').first);
        await settle(t, 10);
        expect(find.byType(MovieDetailScreen), findsOneWidget);
        await t.tap(find.byKey(detailPlayKey));
        await _frames(t, const Duration(seconds: 1));
        expect(find.byType(PlayerScreen), findsOneWidget);

        await _frames(t, const Duration(seconds: 10), playing: engine);
        await t.sendKeyEvent(
          LogicalKeyboardKey.mediaFastForward,
          platform: 'android',
        );
        await _frames(t, const Duration(milliseconds: 1500), playing: engine);
        await engine.seek(const Duration(minutes: 3));
        await _frames(t, const Duration(seconds: 5), playing: engine);
        expect(
          engine.state.value.position,
          greaterThanOrEqualTo(const Duration(minutes: 3)),
        );

        await t.sendKeyEvent(LogicalKeyboardKey.escape);
        await _frames(t, const Duration(seconds: 2));
        await t.pumpWidget(const SizedBox());
        await settle(t, 2);
      } finally {
        semantics.dispose();
        debugDefaultTargetPlatformOverride = null;
      }
    });
  }
}
