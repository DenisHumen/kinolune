// SPDX-License-Identifier: MPL-2.0
//
// UI-014: the track panels. Audio · Subtitles · Video are tabs of one
// panel that opens beside the picture on a desktop, as a bottom sheet on a
// phone and across the top on a TV; a remote moves through it without
// getting stuck, Back leaves a subtitle page for its tab, the video tab
// switches video streams, the subtitles tab changes the subtitle style,
// and the tracks picked in the tabs carry over to the next episode.
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:pa_player/pa_player.dart';
import 'package:pa_player/testing.dart';

const _en = MediaTrack(id: '1', language: 'eng', channels: 6, isDefault: true);
const _ru = MediaTrack(id: '2', language: 'rus', title: 'Дубляж', channels: 2);
const _subEn = MediaTrack(id: '5', language: 'eng');
const _subRu = MediaTrack(id: '6', language: 'rus');
const _main = MediaTrack(
  id: '1',
  codec: 'hevc',
  width: 3840,
  height: 2160,
  fps: 23.976,
);
const _lite = MediaTrack(id: '2', codec: 'h264', width: 1280, height: 720);

class _Harness {
  _Harness({
    this.tv = false,
    this.loadNext,
    this.request = const PlayRequest(
      uri: '/tv/e1.mkv',
      title: 'E1',
      subtitles: [],
      seriesKey: 's1',
    ),
  });

  final bool tv;
  final Future<UpNextItem?> Function()? loadNext;
  final PlayRequest request;
  final engine = FakePlayerEngine();
  final settings = ValueNotifier(const SubtitleSettings());
  final memories = <(String, TrackMemory)>[];

  Widget app() => MaterialApp(
    home: PlayerScreen(
      request: request,
      onProgress: (_, _) {},
      engineFactory: ({bool video = true}) => engine,
      loadNext: loadNext,
      captionStyleLoader: () async => null,
      subtitleSettings: settings,
      onSubtitleSettings: (s) => settings.value = s,
      onSubtitleMemory: (k, m) => memories.add((k, m)),
      tvLayout: tv,
    ),
  );

  PlayerScreenState state(WidgetTester t) =>
      t.state<PlayerScreenState>(find.byType(PlayerScreen));
}

void _surface(WidgetTester t, Size size) {
  t.view.devicePixelRatio = 1;
  t.view.physicalSize = size;
  addTearDown(t.view.reset);
}

Future<_Harness> _open(WidgetTester t, _Harness h) async {
  h.engine
    ..openAudioTracks = const [_en, _ru]
    ..embeddedSubtitles = const [_subEn, _subRu];
  await t.pumpWidget(h.app());
  await t.pump();
  await t.pump(const Duration(seconds: 2));
  return h;
}

Rect _panel(WidgetTester t) =>
    t.getRect(find.byKey(const ValueKey('osd-panel')));

String? _focusedKey() {
  for (
    BuildContext? c = FocusManager.instance.primaryFocus?.context;
    c != null;
  ) {
    final w = c.widget;
    if (w.key is ValueKey<String>) return (w.key! as ValueKey<String>).value;
    BuildContext? parent;
    c.visitAncestorElements((e) {
      parent = e;
      return false;
    });
    c = parent;
  }
  return null;
}

Future<void> _key(WidgetTester t, LogicalKeyboardKey k) async {
  await t.sendKeyEvent(k);
  await t.pumpAndSettle();
}

void main() {
  testWidgets('a desktop window: the panel opens beside the picture', (
    t,
  ) async {
    _surface(t, const Size(1280, 720));
    final h = await _open(t, _Harness());
    await t.tap(find.byKey(const ValueKey('osd-audio')));
    await t.pumpAndSettle();
    final r = _panel(t);
    // 380 wide with a 24 px margin.
    expect(r.right, 1280);
    expect(r.width, 380 + 48);
    for (final tab in ['tab-audio', 'tab-subtitles', 'tab-video']) {
      expect(find.byKey(ValueKey(tab)), findsOneWidget);
    }
    expect(find.byKey(const ValueKey('panel-audio')), findsOneWidget);
    // A click on a tab opens it in the same panel.
    await t.tap(find.byKey(const ValueKey('tab-subtitles')));
    await t.pumpAndSettle();
    expect(find.byKey(const ValueKey('panel-subtitles')), findsOneWidget);
    expect(h.state(t).osd.panel, OsdPanel.subtitles);
    await t.pumpWidget(const SizedBox());
  });

  testWidgets('a phone: a bottom sheet that a swipe down closes', (t) async {
    _surface(t, const Size(390, 844));
    final h = await _open(t, _Harness());
    await t.tap(find.byKey(const ValueKey('osd-subtitles')));
    await t.pumpAndSettle();
    final r = _panel(t);
    expect(r.bottom, 844);
    expect(r.width, 390);
    expect(r.height, closeTo(844 * 0.6, 1));
    expect(find.byKey(const ValueKey('panel-subtitles')), findsOneWidget);
    // Every row of the subtitles tab is reachable.
    await t.scrollUntilVisible(
      find.byKey(const ValueKey('sub-background')),
      100,
      scrollable: find.descendant(
        of: find.byKey(const ValueKey('panel-subtitles')),
        matching: find.byType(Scrollable),
      ),
    );
    await t.fling(
      find.byKey(const ValueKey('tab-subtitles')),
      const Offset(0, 300),
      1000,
    );
    await t.pumpAndSettle();
    expect(h.state(t).osd.panel, OsdPanel.none);
    expect(find.byKey(const ValueKey('osd-panel')), findsNothing);
    await t.pumpWidget(const SizedBox());
  });

  testWidgets('a phone in landscape keeps the sheet', (t) async {
    _surface(t, const Size(844, 390));
    await _open(t, _Harness());
    await t.tap(find.byKey(const ValueKey('osd-audio')));
    await t.pumpAndSettle();
    final r = _panel(t);
    expect(r.bottom, 390);
    expect(r.height, closeTo(390 * 0.85, 1));
    expect(t.takeException(), isNull);
    await t.pumpWidget(const SizedBox());
  });

  testWidgets('a TV: tabs across the top, the remote never gets stuck', (
    t,
  ) async {
    _surface(t, const Size(1920, 1080));
    final h = await _open(t, _Harness(tv: true));
    final s = h.state(t);
    s.osd.openPanel(OsdPanel.audio);
    await t.pumpAndSettle();
    final r = _panel(t);
    // Across the top with 48 px TV margins at the sides.
    expect(r.top, 0);
    expect(r.width, 1920);
    expect(r.height, closeTo(1080 * 0.55 + 24, 1));
    // Focus starts on the track playing.
    expect(_focusedKey(), 'audio-1');

    // Up reaches the tabs; Right opens Subtitles without a press.
    await _key(t, LogicalKeyboardKey.arrowUp);
    expect(_focusedKey(), 'track-tabs');
    expect(s.osd.panel, OsdPanel.audio);
    await _key(t, LogicalKeyboardKey.arrowRight);
    expect(_focusedKey(), 'track-tabs');
    expect(s.osd.panel, OsdPanel.subtitles);
    await _key(t, LogicalKeyboardKey.arrowRight);
    expect(s.osd.panel, OsdPanel.video);
    // Past the last tab: the close button, and back.
    await _key(t, LogicalKeyboardKey.arrowRight);
    expect(_focusedKey(), 'panel-close');
    await _key(t, LogicalKeyboardKey.arrowLeft);
    expect(_focusedKey(), 'track-tabs');
    expect(s.osd.panel, OsdPanel.video);
    await _key(t, LogicalKeyboardKey.arrowLeft);
    expect(s.osd.panel, OsdPanel.subtitles);

    // Down goes into the list; Select picks a track and closes the panel.
    await _key(t, LogicalKeyboardKey.arrowDown);
    expect(_focusedKey(), startsWith('sub-'));
    var guard = 0;
    while (_focusedKey() != 'sub-5' && guard++ < 5) {
      await _key(t, LogicalKeyboardKey.arrowDown);
    }
    expect(_focusedKey(), 'sub-5');
    await _key(t, LogicalKeyboardKey.select);
    expect(h.engine.calls, contains('sub:5'));
    expect(s.osd.panel, OsdPanel.none);

    // Down through the whole subtitles tab ends at its last row, and Back
    // closes the panel from anywhere.
    s.osd.openPanel(OsdPanel.subtitles);
    await t.pumpAndSettle();
    for (var i = 0; i < 20; i++) {
      await _key(t, LogicalKeyboardKey.arrowDown);
    }
    expect(_focusedKey(), 'sub-background');
    await _key(t, LogicalKeyboardKey.escape);
    expect(s.osd.panel, OsdPanel.none);
    await t.pumpWidget(const SizedBox());
  });

  testWidgets('Back from a subtitle page returns to the subtitles tab', (
    t,
  ) async {
    _surface(t, const Size(1280, 720));
    final h = await _open(t, _Harness());
    final s = h.state(t);
    s.osd.openPanel(OsdPanel.subtitles);
    await t.pumpAndSettle();
    await t.ensureVisible(find.byKey(const ValueKey('sub-file')));
    await t.tap(find.byKey(const ValueKey('sub-file')));
    await t.pumpAndSettle();
    expect(s.osd.panel, OsdPanel.subtitleFile);
    await _key(t, LogicalKeyboardKey.escape);
    expect(s.osd.panel, OsdPanel.subtitles);
    // The arrow in the header does the same.
    s.osd.openPanel(OsdPanel.subtitleFile);
    await t.pumpAndSettle();
    await t.tap(find.byKey(const ValueKey('panel-back')));
    await t.pumpAndSettle();
    expect(s.osd.panel, OsdPanel.subtitles);
    await _key(t, LogicalKeyboardKey.escape);
    expect(s.osd.panel, OsdPanel.none);
    await t.pumpWidget(const SizedBox());
  });

  testWidgets('the video tab switches the video stream', (t) async {
    _surface(t, const Size(1280, 720));
    final h = _Harness();
    h.engine.openVideoTracks = const [_main, _lite];
    await _open(t, h);
    h.state(t).osd.openPanel(OsdPanel.video);
    await t.pumpAndSettle();
    expect(find.text('3840×2160 · hevc · 23.976 fps'), findsOneWidget);
    await t.tap(find.byKey(const ValueKey('video-2')));
    await t.pumpAndSettle();
    expect(h.engine.calls, contains('video:2'));
    expect(h.engine.state.value.videoTrack, _lite);
    // Not remembered for the series: streams differ between files.
    expect(h.memories, isEmpty);
    await t.pumpWidget(const SizedBox());
  });

  testWidgets('one video stream is shown, with the zoom modes', (t) async {
    _surface(t, const Size(1280, 720));
    final h = _Harness();
    h.engine.openVideoTracks = const [_main];
    await _open(t, h);
    h.state(t).osd.openPanel(OsdPanel.video);
    await t.pumpAndSettle();
    expect(find.byKey(const ValueKey('video-current')), findsOneWidget);
    expect(find.byKey(const ValueKey('video-1')), findsNothing);
    await t.tap(find.byKey(const ValueKey('zoom-fill')));
    await t.pumpAndSettle();
    expect(h.state(t).zoom.value, VideoZoom.fill);
    await t.pumpWidget(const SizedBox());
  });

  testWidgets('the subtitles tab changes size, height and background', (
    t,
  ) async {
    _surface(t, const Size(1280, 900));
    final h = await _open(t, _Harness());
    h.state(t).osd.openPanel(OsdPanel.subtitles);
    await t.pumpAndSettle();
    Future<void> tap(String key) async {
      await t.ensureVisible(find.byKey(ValueKey(key)));
      await t.pumpAndSettle();
      await t.tap(find.byKey(ValueKey(key)));
      await t.pumpAndSettle();
    }

    expect(find.text('System'), findsOneWidget);
    // The first step leaves the system style at the normal size.
    await tap('sub-size-more');
    expect(h.settings.value.useSystemStyle, isFalse);
    expect(h.settings.value.scale, 1.0);
    await tap('sub-size-more');
    expect(h.settings.value.scale, 1.25);
    expect(find.text('125 %'), findsOneWidget);
    await tap('sub-size-less');
    await tap('sub-size-less');
    expect(h.settings.value.scale, 0.75);

    await tap('sub-height-more');
    expect(h.settings.value.position, closeTo(0.10, 1e-9));
    await tap('sub-height-less');
    await tap('sub-height-less');
    expect(h.settings.value.position, closeTo(0.06, 1e-9));

    await tap('sub-background');
    expect(h.settings.value.background, SubtitleBackground.translucent);
    expect(find.text('Background: translucent'), findsOneWidget);
    // The panel stays open while the style is tuned.
    expect(h.state(t).osd.panel, OsdPanel.subtitles);
    await t.pumpWidget(const SizedBox());
  });

  testWidgets('no quick style without a way to store it', (t) async {
    _surface(t, const Size(1280, 720));
    final h = _Harness();
    h.engine
      ..openAudioTracks = const [_en]
      ..embeddedSubtitles = const [_subEn];
    await t.pumpWidget(
      MaterialApp(
        home: PlayerScreen(
          request: h.request,
          onProgress: (_, _) {},
          engineFactory: ({bool video = true}) => h.engine,
          captionStyleLoader: () async => null,
          subtitleSettings: h.settings,
        ),
      ),
    );
    await t.pump(const Duration(seconds: 2));
    h.state(t).osd.openPanel(OsdPanel.subtitles);
    await t.pumpAndSettle();
    expect(find.byKey(const ValueKey('sub-size-more')), findsNothing);
    await t.pumpWidget(const SizedBox());
  });

  // ROADMAP UI-014 «done when»: a track chosen for a series is kept and
  // applied to the next episode.
  testWidgets('tracks picked in the tabs carry over to the next episode', (
    t,
  ) async {
    _surface(t, const Size(1920, 1080));
    final h = _Harness(
      tv: true,
      loadNext: () async => UpNextItem(
        request: const PlayRequest(
          uri: '/tv/e2.mkv',
          title: 'E2',
          subtitles: [],
          seriesKey: 's1',
        ),
        onProgress: (_, _) {},
      ),
    );
    await _open(t, h);
    final s = h.state(t);
    s.osd.openPanel(OsdPanel.audio);
    await t.pumpAndSettle();
    await t.tap(find.byKey(const ValueKey('audio-2')));
    await t.pumpAndSettle();
    s.osd.openPanel(OsdPanel.audio);
    await t.pumpAndSettle();
    await t.tap(find.byKey(const ValueKey('tab-subtitles')));
    await t.pumpAndSettle();
    await t.tap(find.byKey(const ValueKey('sub-5')));
    await t.pumpAndSettle();
    final remembered = h.memories.last;
    expect(remembered.$1, 's1');
    expect(remembered.$2.audio!.language, 'rus');
    expect(remembered.$2.choice!.language, 'eng');

    // The next file lists its tracks in another order and with other ids.
    h.engine
      ..openAudioTracks = const [
        MediaTrack(id: '7', language: 'eng', isDefault: true),
        MediaTrack(id: '8', language: 'rus'),
      ]
      ..embeddedSubtitles = const [
        MediaTrack(id: '9', language: 'rus'),
        MediaTrack(id: '10', language: 'eng'),
      ];
    h.engine.finish();
    await t.pump();
    s.upNext.playNow();
    await t.pump();
    await t.pump(const Duration(seconds: 2));
    expect(s.request.title, 'E2');
    expect(h.engine.state.value.audioTrack!.id, '8');
    expect(h.engine.state.value.subtitleTrack!.id, '10');

    // A new player for the next episode (the app opened it from the
    // library) starts with what the app stored for the series.
    await t.pumpWidget(const SizedBox());
    final next = _Harness(
      tv: true,
      request: PlayRequest(
        uri: '/tv/e3.mkv',
        title: 'E3',
        subtitles: const [],
        seriesKey: 's1',
        subtitleMemory: remembered.$2,
      ),
    );
    await _open(t, next);
    expect(next.engine.state.value.audioTrack, _ru);
    expect(next.engine.state.value.subtitleTrack, _subEn);
    await t.pumpWidget(const SizedBox());
  });
}
