// SPDX-License-Identifier: MPL-2.0
//
// Trailer banner (UI-036, ADR-0100): muted autoplay after ~1.5 s, pauses
// (scrolled away, app in the background, TV focus away, covered by the full
// screen), «вверх» on the TV remote → full screen → «Назад» to the same
// place, «никогда» and internet trailers off, the online trailer's frame
// that nothing covers, and the embed page. The WebView and the player are
// replaced by fake surfaces ([TrailerPlatform.instance]). With
// KINOLUNE_SCREENSHOTS_DIR set the frames are saved:
//   KINOLUNE_SCREENSHOTS_DIR=$PWD/../../../docs/screenshots/progress/ui-036 \
//     flutter test test/trailers_test.dart

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_library.dart';
import 'package:kinolune/integration/library.dart';
import 'package:kinolune/integration/player.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/screens/movie_detail_screen.dart';
import 'package:kinolune/services/app_services.dart';
import 'package:kinolune/services/preferences.dart';
import 'package:kinolune/services/secure_store.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:kinolune/trailers/trailer_banner.dart';
import 'package:kinolune/trailers/trailer_controller.dart';
import 'package:kinolune/trailers/trailer_platform.dart';
import 'package:kinolune/trailers/trailer_settings.dart';
import 'package:kinolune/widgets/focusable.dart';
import 'package:material_ui/material_ui.dart';
import 'package:pa_player/testing.dart';

import 'support/test_app.dart';

final _now = DateTime(2026, 11, 17, 20);

LibraryItem _movie(String id, String title) => LibraryItem(
  id: id,
  kind: ItemKind.movie,
  title: title,
  year: 2016,
  overview: 'A linguist is recruited to talk to visitors.',
  filePath: '/media/Movies/$title/$title.mkv',
  durationMs: 116 * 60000,
  addedAt: _now,
);

final _own = _movie('m-own', 'Arrival');
final _online = _movie('m-online', 'Dune');

const _ownDetails = ItemDetails(
  genres: ['Drama'],
  credits: [
    CreditPerson(name: 'Ann Example', character: 'Louise'),
    CreditPerson(name: 'Bob Example', character: 'Ian'),
  ],
  extras: [
    MediaExtra(
      title: 'Arrival Trailer',
      kind: MediaExtraKind.trailer,
      filePath: '/media/Movies/Arrival/Trailers/Arrival Trailer.mkv',
      durationMs: 150000,
    ),
  ],
);

const _youtube = Trailer(
  source: TrailerSource.youtube,
  url: 'https://www.youtube.com/watch?v=abcdefghijk',
  videoKey: 'abcdefghijk',
  name: 'Official Trailer',
  language: 'en',
);

class _FakeSurface implements TrailerSurface {
  _FakeSurface(this.trailer, {required this.muted});

  final Trailer trailer;
  final bool muted;
  bool playing = false;
  bool disposed = false;
  int pauses = 0;

  @override
  final ValueNotifier<bool> showing = ValueNotifier(false);
  @override
  final ValueNotifier<bool> failed = ValueNotifier(false);

  @override
  Widget build(BuildContext context) => ColoredBox(
    key: ValueKey(muted ? 'fake-banner-video' : 'fake-full-video'),
    color: const Color(0xFF2A4A6A),
  );

  @override
  Future<void> play() async {
    playing = true;
    showing.value = true;
  }

  @override
  Future<void> pause() async {
    playing = false;
    pauses++;
  }

  @override
  Future<void> dispose() async {
    disposed = true;
    playing = false;
  }
}

class _FakePlatform extends TrailerPlatform {
  final surfaces = <_FakeSurface>[];
  final opened = <Uri>[];
  bool web = true;
  bool saver = false;

  @override
  bool get webEmbed => web;

  @override
  Future<bool> powerSaving() async => saver;

  @override
  Future<bool> metered() async => false;

  @override
  TrailerSurface? surface(
    Trailer trailer, {
    required bool muted,
    required bool loop,
    required PlayerEngineFactory engines,
    String language = 'en',
  }) {
    if (trailer.source.online && !web) return null;
    final s = _FakeSurface(trailer, muted: muted);
    surfaces.add(s);
    return s;
  }

  @override
  Future<bool> openExternal(Uri url) async {
    opened.add(url);
    return true;
  }

  _FakeSurface? get banner => surfaces.where((s) => s.muted).lastOrNull;
}

late _FakePlatform _platform;

Future<(AppServices, FakeLibrary)> _services({
  Map<String, String> prefs = const {},
}) async {
  final lib = FakeLibrary.items(
    [_own, _online],
    details: {'m-own': _ownDetails},
  );
  lib.onlineTrailers['m-online'] = _youtube;
  final p = MemoryPreferences()..values.addAll(prefs);
  final s = AppServices(
    library: lib,
    secureStore: MemorySecureStore(),
    preferences: p,
    playerEngineFactory: ({bool video = true}) => FakePlayerEngine(),
    clock: () => _now,
  );
  await s.init();
  return (s, lib);
}

Future<void> _open(
  WidgetTester t,
  AppServices s,
  String title, {
  FormFactor ff = FormFactor.desktop,
  Size size = const Size(1280, 800),
}) async {
  setSurface(t, size);
  await t.pumpWidget(testApp(s, formFactor: ff, section: Section.movies));
  await settle(t, 10);
  await t.tap(find.text(title).first);
  // Less than the autoplay delay.
  await settle(t, 5);
}

Future<void> _close(WidgetTester t) async {
  await t.pumpWidget(const SizedBox());
  await settle(t, 2);
}

/// The Android back button / the TV remote's «Назад».
Future<void> _systemBack(WidgetTester t) async {
  await t.binding.defaultBinaryMessenger.handlePlatformMessage(
    'flutter/navigation',
    const JSONMethodCodec().encodeMethodCall(const MethodCall('popRoute')),
    (_) {},
  );
  await settle(t, 6);
}

KButton? _focusedButton() => FocusManager.instance.primaryFocus?.context
    ?.findAncestorWidgetOfExactType<KButton>();

ScrollPosition _pageScroll(WidgetTester t) => t
    .state<ScrollableState>(
      find
          .descendant(
            of: find.byType(MovieDetailScreen),
            matching: find.byType(Scrollable),
          )
          .first,
    )
    .position;

void main() {
  setUp(() {
    _platform = _FakePlatform();
    TrailerPlatform.instance = _platform;
  });
  tearDown(() => TrailerPlatform.instance = TrailerPlatform());

  group('settings', () {
    test('defaults: always, internet on; stored as names', () async {
      final p = MemoryPreferences();
      var s = await TrailerSettings.read(p);
      expect(s.autoplay, TrailerAutoplay.always);
      expect(s.internet, isTrue);
      await TrailerSettings.writeAutoplay(p, TrailerAutoplay.wifiOnly);
      await TrailerSettings.writeInternet(p, false);
      s = await TrailerSettings.read(p);
      expect(s.autoplay, TrailerAutoplay.wifiOnly);
      expect(s.internet, isFalse);
      await TrailerSettings.writeAutoplay(p, TrailerAutoplay.always);
      await TrailerSettings.writeInternet(p, true);
      expect(p.values, isEmpty);
      expect(TrailerAutoplay.parse('bogus'), TrailerAutoplay.always);
    });
  });

  group('embed page', () {
    test('YouTube through youtube-nocookie, no download, escaped', () {
      final html = trailerEmbedHtml(
        TrailerSource.youtube,
        'abcdefghijk',
        muted: true,
        loop: true,
        language: 'ru-RU',
      );
      expect(html, contains("host: 'https://www.youtube-nocookie.com'"));
      expect(html, contains('https://www.youtube.com/iframe_api'));
      expect(html, contains('"key":"abcdefghijk"'));
      expect(html, contains('"muted":true'));
      expect(html, isNot(contains('googlevideo')));
      // A hostile id cannot close the script element.
      final evil = trailerEmbedHtml(
        TrailerSource.youtube,
        '</script><script>x',
        muted: false,
        loop: false,
      );
      expect(evil, isNot(contains('</script><script>x')));
    });

    test('Vimeo in its own player, muted background mode', () {
      final html = trailerEmbedHtml(
        TrailerSource.vimeo,
        '123456789',
        muted: true,
        loop: true,
      );
      expect(html, contains('https://player.vimeo.com/video/'));
      expect(html, contains('"site":"vimeo"'));
    });
  });

  testWidgets('own trailer: still, then muted autoplay after ~1.5 s', (
    t,
  ) async {
    final (s, _) = await _services();
    await _open(t, s, 'Arrival');
    expect(_platform.surfaces, isEmpty, reason: 'still backdrop first');
    expect(find.byKey(trailerButtonKey), findsOneWidget);
    await settle(t, 12);
    final banner = _platform.banner!;
    expect(banner.muted, isTrue);
    expect(banner.playing, isTrue);
    expect(banner.trailer.source, TrailerSource.file);
    expect(find.byKey(const ValueKey('fake-banner-video')), findsOneWidget);
    await capture(t, 'banner_own_trailer_desktop');
    await _close(t);
    expect(banner.disposed, isTrue);
  });

  testWidgets('pauses when scrolled away and in the background', (t) async {
    final (s, _) = await _services();
    await _open(t, s, 'Arrival');
    await settle(t, 12);
    final banner = _platform.banner!;
    expect(banner.playing, isTrue);

    _pageScroll(t).jumpTo(700);
    await settle(t, 2);
    expect(banner.playing, isFalse, reason: 'scrolled away');
    _pageScroll(t).jumpTo(0);
    await settle(t, 2);
    expect(banner.playing, isTrue);

    t.binding.handleAppLifecycleStateChanged(AppLifecycleState.inactive);
    t.binding.handleAppLifecycleStateChanged(AppLifecycleState.hidden);
    t.binding.handleAppLifecycleStateChanged(AppLifecycleState.paused);
    await settle(t, 2);
    expect(banner.playing, isFalse, reason: 'app in the background');
    t.binding.handleAppLifecycleStateChanged(AppLifecycleState.hidden);
    t.binding.handleAppLifecycleStateChanged(AppLifecycleState.inactive);
    t.binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
    await settle(t, 2);
    expect(banner.playing, isTrue);
    await _close(t);
  });

  testWidgets('Battery Saver and reduce motion: no autoplay', (t) async {
    _platform.saver = true;
    final (s, _) = await _services();
    await _open(t, s, 'Arrival');
    await settle(t, 20);
    expect(_platform.surfaces, isEmpty);
    expect(find.byKey(trailerButtonKey), findsOneWidget);
    await _close(t);

    _platform.saver = false;
    t.platformDispatcher.accessibilityFeaturesTestValue =
        const FakeAccessibilityFeatures(disableAnimations: true);
    addTearDown(t.platformDispatcher.clearAccessibilityFeaturesTestValue);
    final (s2, _) = await _services();
    await _open(t, s2, 'Arrival');
    await settle(t, 20);
    expect(_platform.surfaces, isEmpty, reason: 'reduce motion');
    await _close(t);
  });

  testWidgets('«никогда»: no autoplay, the button still plays it', (t) async {
    final (s, _) = await _services(
      prefs: {TrailerSettings.autoplayKey: TrailerAutoplay.never.name},
    );
    await _open(t, s, 'Dune');
    await settle(t, 20);
    expect(_platform.surfaces, isEmpty);
    expect(
      find.byKey(const ValueKey('trailer-live-backdrop')),
      findsOneWidget,
    );
    await t.tap(find.byKey(trailerButtonKey));
    await settle(t, 6);
    expect(find.byType(TrailerFullScreen), findsOneWidget);
    final full = _platform.surfaces.single;
    expect(full.muted, isFalse, reason: 'full screen has sound');
    expect(full.playing, isTrue);
    await capture(t, 'full_screen_online');
    await _close(t);
  });

  testWidgets('internet trailers off: the core is asked without internet', (
    t,
  ) async {
    final (s, lib) = await _services(
      prefs: {TrailerSettings.internetKey: 'false'},
    );
    await _open(t, s, 'Dune');
    await settle(t, 20);
    expect(lib.trailerRequests, [('m-online', false)]);
    expect(find.byKey(trailerButtonKey), findsNothing);
    expect(_platform.surfaces, isEmpty);
    await _close(t);
  });

  testWidgets('online trailer plays in a frame nothing covers', (t) async {
    final (s, _) = await _services();
    await _open(t, s, 'Dune');
    await settle(t, 12);
    final banner = _platform.banner!;
    expect(banner.trailer.source, TrailerSource.youtube);
    expect(banner.playing, isTrue);
    final frame = t.getRect(find.byKey(const ValueKey('trailer-frame')));
    expect(frame.height, greaterThanOrEqualTo(trailerFrameMinHeight));
    expect(frame.width / frame.height, closeTo(16 / 9, .01));
    for (final f in [
      find.byKey(detailPlayKey),
      find.byKey(trailerButtonKey),
      find.byKey(const ValueKey('detail-title')),
      find.byKey(const ValueKey('detail-meta')),
    ]) {
      if (f.evaluate().isEmpty) continue;
      expect(
        frame.overlaps(t.getRect(f)),
        isFalse,
        reason: '$f must not cover the player',
      );
    }
    await capture(t, 'banner_online_frame_desktop');
    await _close(t);
  });

  testWidgets('no WebView: the button opens the video in its app', (t) async {
    _platform.web = false;
    final (s, _) = await _services();
    await _open(t, s, 'Dune');
    await settle(t, 12);
    expect(_platform.surfaces, isEmpty);
    expect(
      find.byKey(const ValueKey('trailer-live-backdrop')),
      findsOneWidget,
    );
    await t.tap(find.byKey(trailerButtonKey));
    await settle(t, 2);
    expect(_platform.opened, [Uri.parse(_youtube.url!)]);
    await _close(t);
  });

  testWidgets('TV: focus on Play, «вверх» → full screen → «Назад» back', (
    t,
  ) async {
    final (s, _) = await _services();
    await _open(t, s, 'Dune', ff: FormFactor.tv, size: const Size(960, 540));
    expect(_focusedButton()?.key, detailPlayKey);
    await settle(t, 12);
    final banner = _platform.banner;
    await capture(t, 'banner_online_tv');
    final offset = _pageScroll(t).pixels;

    await pressKey(t, LogicalKeyboardKey.arrowUp);
    expect(find.byType(TrailerFullScreen), findsOneWidget);
    final full = _platform.surfaces.firstWhere((x) => !x.muted);
    expect(full.playing, isTrue);
    if (banner != null) {
      expect(banner.playing, isFalse, reason: 'covered by the full screen');
    }

    await _systemBack(t);
    expect(find.byType(TrailerFullScreen), findsNothing);
    expect(full.disposed, isTrue);
    expect(_focusedButton()?.key, detailPlayKey, reason: 'same place');
    expect(_pageScroll(t).pixels, offset);
    if (banner != null) expect(banner.playing, isTrue);
    await _close(t);
  });

  testWidgets('TV: own trailer full screen in the player; back returns', (
    t,
  ) async {
    final (s, _) = await _services();
    await _open(t, s, 'Arrival', ff: FormFactor.tv, size: const Size(960, 540));
    await settle(t, 12);
    final banner = _platform.banner!;
    expect(banner.playing, isTrue);
    await capture(t, 'banner_own_trailer_tv');

    // Focus moves down out of the top part: the banner pauses.
    await pressKey(t, LogicalKeyboardKey.arrowDown, times: 3);
    expect(banner.playing, isFalse, reason: 'focus left the top');
    await pressKey(t, LogicalKeyboardKey.arrowUp, times: 3);
    expect(_focusedButton()?.key, detailPlayKey);
    expect(banner.playing, isTrue);

    await pressKey(t, LogicalKeyboardKey.arrowUp);
    expect(find.byType(PlayerScreen), findsOneWidget);
    expect(banner.playing, isFalse);
    await _systemBack(t);
    expect(find.byType(PlayerScreen), findsNothing);
    expect(_focusedButton()?.key, detailPlayKey);
    await _close(t);
  });

  testWidgets('phone: a tap on the banner opens the trailer', (t) async {
    final (s, _) = await _services();
    await _open(
      t,
      s,
      'Arrival',
      ff: FormFactor.phone,
      size: const Size(390, 844),
    );
    await settle(t, 12);
    await capture(t, 'banner_own_trailer_phone');
    await t.tapAt(const Offset(300, 120));
    await settle(t, 6);
    expect(find.byType(PlayerScreen), findsOneWidget);
    await _close(t);
  });
}
