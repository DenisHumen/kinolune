// SPDX-License-Identifier: MPL-2.0
//
// MUS-005 in the app: the mini player is in every section (a capsule above
// the tabs on a phone, a bar on the desktop, a plate on TV), the video
// player hides it and it comes back after; Now Playing shows the format of
// the stream from the library.

import 'dart:async';

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/features/music/music_queue_store.dart';
import 'package:kinolune/integration/library.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/screens/playback.dart';
import 'package:kinolune/services/app_services.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:material_ui/material_ui.dart';
import 'package:pa_player/pa_player.dart';

import 'support/test_app.dart';

const _bar = ValueKey('now-playing-bar');

/// Plays the first album of the demo library («Nocturne Avenue», FLAC
/// 24/96).
Future<void> _playAlbum(AppServices s) async {
  final tracks = await libraryMusic(s.library).musicTracks().first;
  final album = tracks.where((x) => x.item.album == 'Nocturne Avenue');
  await s.music.play([for (final x in album.take(3)) musicRequest(x)]);
}

Future<void> _done(WidgetTester t, AppServices s) async {
  s.music.pause();
  await t.pumpWidget(const SizedBox());
  await settle(t, 2);
}

void main() {
  for (final (name, size, form, styleKey) in [
    ('phone', const Size(390, 844), FormFactor.phone, 'mini-player-capsule'),
    ('desktop', const Size(1280, 800), FormFactor.desktop, 'mini-player-bar'),
  ]) {
    testWidgets('$name: the mini player is in every section', (t) async {
      setSurface(t, size);
      final s = await demoServices();
      await _playAlbum(s);
      await t.pumpWidget(testApp(s, formFactor: form));
      await settle(t);
      final shell = t.state<AppShellState>(find.byType(AppShell));
      for (final section in shell.menuSections) {
        shell.select(section);
        await settle(t, 6);
        expect(
          find.byKey(_bar),
          findsOneWidget,
          reason: 'mini player in ${section.name}',
        );
        expect(find.byKey(ValueKey(styleKey)), findsOneWidget);
      }
      await capture(t, 'mus-005-mini-player-$name');
      await _done(t, s);
    });
  }

  testWidgets('the video player hides the mini player, closing it brings '
      'it back', (t) async {
    setSurface(t, const Size(1280, 800));
    final s = await demoServices();
    await _playAlbum(s);
    await t.pumpWidget(testApp(s, section: Section.movies));
    await settle(t);
    expect(find.byKey(_bar), findsOneWidget);

    final movie = (await s.library.movies().first).firstWhere(
      (i) => i.title == 'The Quiet Orbit',
    );
    final context = t.element(find.byType(AppShell));
    unawaited(playVideo(context, movie));
    await settle(t);
    expect(find.byType(PlayerScreen), findsOneWidget);
    expect(find.byKey(_bar), findsNothing, reason: 'hidden in the player');
    expect(s.music.playing, isFalse, reason: 'the video paused the music');

    Navigator.of(
      t.element(find.byType(PlayerScreen)),
      rootNavigator: true,
    ).pop();
    await settle(t);
    expect(find.byType(PlayerScreen), findsNothing);
    expect(find.byKey(_bar), findsOneWidget, reason: 'back after the video');
    await _done(t, s);
  });

  testWidgets('Now Playing shows the format of the stream', (t) async {
    setSurface(t, const Size(1280, 800));
    final s = await demoServices();
    await _playAlbum(s);
    await t.pumpWidget(testApp(s, section: Section.music));
    await settle(t);
    // The desktop bar shows it too.
    expect(find.text('FLAC · 24/96'), findsOneWidget);
    await t.tap(find.byKey(const ValueKey('bar-title')));
    await settle(t);
    expect(find.byType(NowPlayingScreen), findsOneWidget);
    expect(
      t.widget<Text>(find.byKey(const ValueKey('np-format'))).data,
      'FLAC · 24/96',
    );
    await capture(t, 'mus-005-now-playing-desktop');
    await _done(t, s);
  });

  testWidgets('TV: a plate the remote focuses; OK opens Now Playing', (
    t,
  ) async {
    setSurface(t, const Size(1920, 1080));
    final s = await demoServices();
    await _playAlbum(s);
    await t.pumpWidget(testApp(s, formFactor: FormFactor.tv));
    await settle(t);
    final plate = find.byKey(const ValueKey('mini-player-tv'));
    expect(plate, findsOneWidget);

    Focus.of(t.element(plate)).requestFocus();
    await settle(t, 3);
    final box = t.widget<AnimatedContainer>(plate);
    expect(
      (box.decoration! as BoxDecoration).color,
      const Color(0xFFFFFFFF),
      reason: 'the focused plate is drawn white',
    );
    await capture(t, 'mus-005-mini-player-tv');

    await t.sendKeyEvent(LogicalKeyboardKey.select);
    await settle(t);
    expect(find.byType(NowPlayingScreen), findsOneWidget);
    await _done(t, s);
  });
}
