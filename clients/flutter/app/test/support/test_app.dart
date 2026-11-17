// SPDX-License-Identifier: MPL-2.0

import 'dart:io';
import 'dart:ui' as ui;

import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/app.dart';
import 'package:kinolune/fakes/fake_device_sync.dart';
import 'package:kinolune/fakes/fake_library.dart';
import 'package:kinolune/fakes/fake_source_manager.dart';
import 'package:kinolune/fakes/fake_twitch.dart';
import 'package:kinolune/features/updates/update_controller.dart';
import 'package:kinolune/integration/library.dart'
    show Library, LibraryProfiles, PersonalMedia;
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/screens/home_screen.dart';
import 'package:kinolune/services/app_services.dart';
import 'package:kinolune/services/image_cache_control.dart';
import 'package:kinolune/services/preferences.dart';
import 'package:kinolune/services/secret_storage.dart';
import 'package:kinolune/services/secure_store.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:kinolune/sources/discovery.dart';
import 'package:kinolune/sources/jellyfin_link.dart';
import 'package:kinolune/sources/source_manager.dart';
import 'package:kinolune/sources/subsonic_link.dart';
import 'package:kinolune/sync/device_sync.dart';
import 'package:kinolune/twitch/twitch_archive.dart';
import 'package:kinolune/twitch/twitch_backend.dart';
import 'package:kinolune/twitch/twitch_downloads.dart';
import 'package:material_ui/material_ui.dart';
import 'package:pa_player/pa_player.dart' show PlayerEngineFactory;
import 'package:pa_player/testing.dart';

bool _fontsLoaded = false;

/// Loads Roboto and Material Icons from the Flutter SDK cache so screenshots
/// show real text instead of the test font boxes.
Future<void> loadAppFonts() async {
  if (_fontsLoaded) return;
  _fontsLoaded = true;
  // Inter from pa_design (fetched by `node design/scripts/fonts-fetch.mjs`).
  final inter = File(
    '../../../design/flutter/pa_design/fonts/InterVariable.ttf',
  );
  if (inter.existsSync()) {
    final loader = FontLoader('packages/pa_design/Inter')
      ..addFont(Future.value(ByteData.sublistView(inter.readAsBytesSync())));
    await loader.load();
  }
  final dir = _materialFontsDir();
  if (dir == null) return;
  Future<void> load(String family, List<String> files) async {
    final loader = FontLoader(family);
    for (final f in files) {
      final file = File('${dir.path}/$f');
      if (file.existsSync()) {
        loader.addFont(
          Future.value(ByteData.sublistView(file.readAsBytesSync())),
        );
      }
    }
    await loader.load();
  }

  await load('Roboto', [
    'Roboto-Regular.ttf',
    'Roboto-Medium.ttf',
    'Roboto-Bold.ttf',
    'Roboto-Black.ttf',
    'Roboto-Light.ttf',
  ]);
  await load('MaterialIcons', ['MaterialIcons-Regular.otf']);
}

Directory? _materialFontsDir() {
  final candidates = <String>[
    if (Platform.environment['FLUTTER_ROOT'] != null)
      '${Platform.environment['FLUTTER_ROOT']}/bin/cache/artifacts/material_fonts',
  ];
  // flutter_tester lives in <flutter>/bin/cache/artifacts/engine/<platform>/.
  var d = File(Platform.resolvedExecutable).parent;
  for (var i = 0; i < 4; i++) {
    candidates.add('${d.path}/material_fonts');
    candidates.add('${d.path}/artifacts/material_fonts');
    d = d.parent;
  }
  for (final c in candidates) {
    final dir = Directory(c);
    if (dir.existsSync()) return dir;
  }
  return null;
}

/// Demo services. Sources go through [FakeSourceManager] (like the Rust
/// source manager); with [reducedSources] through the v0 fallback
/// (`LibrarySourceManager`). [discovery] feeds the add flow.
Future<AppServices> demoServices({
  bool empty = false,
  bool reducedSources = false,
  SourceDiscovery discovery = const NoSourceDiscovery(),
  DeviceSync? sync,
  TwitchBackend? twitch,
  Preferences? preferences,
  SecretStorageInfo secretStorage = const SecretStorageInfo.unknown(),
  SourceManager Function(Library library)? sourceManager,
  PlayerEngineFactory? engineFactory,
  bool twitchNative = false,
  LibraryProfiles? profiles,
  TwitchDownloads? twitchDownloads,
  TwitchArchive? twitchArchive,
  JellyfinLink? jellyfin,
  SubsonicLink? subsonic,
  UpdateController? updates,
  FakeLibrary? library,
  PersonalMedia? personal,
}) async {
  library ??= empty
      ? FakeLibrary(scanStepDelay: Duration.zero)
      : FakeLibrary.demo(scanStepDelay: Duration.zero);
  final s = AppServices(
    library: library,
    secureStore: MemorySecureStore(),
    playerEngineFactory:
        engineFactory ?? ({bool video = true}) => FakePlayerEngine(),
    sourceManager: reducedSources
        ? null
        : (sourceManager ?? FakeSourceManager.new)(library),
    // NET-014: a Jellyfin client, when a test gives one.
    jellyfin: jellyfin,
    // NET-016: a Subsonic client, when a test gives one.
    subsonic: subsonic,
    discovery: discovery,
    // A family of three (UI-019) for the Devices screen.
    sync: sync ?? FakeDeviceSync.demo(),
    clock: () => demoNow,
    imageCache: demoImageCache(),
    preferences: preferences,
    // TWI-013: the fake Helix with made-up channels, signed in.
    twitchBackend: twitch ?? FakeTwitchBackend.demo(now: demoNow),
    secretStorage: secretStorage,
    // TWI-014: a full build whose signed config allows native playback.
    twitchFullBuild: twitchNative,
    twitchNativeAllowed: twitchNative,
    // TWI-017: the download queue, when a test gives one.
    twitchDownloads: twitchDownloads,
    // TWI-019: recordings, rules, saved files, when a test gives them.
    twitchArchive: twitchArchive,
    // UI-015: in-memory profiles (FakeProfiles) when given.
    profileStore: profiles,
    // PKG-018: in-app updates, when a test gives them.
    updates: updates,
    // UI-025: personal media, when a test gives it.
    personal: personal,
  );
  await s.init();
  return s;
}

/// A WebDAV and an SMB share and one of our servers "on the network", for
/// the add flow and the first-run setup (UI-004).
const demoDiscovery = StaticSourceDiscovery(
  [
    DiscoveredSource(
      type: SourceType.webdav,
      name: 'Living-room NAS',
      host: '192.168.1.20',
      port: 8080,
      path: '/dav/',
      url: 'http://192.168.1.20:8080/dav/',
      secure: false,
    ),
    DiscoveredSource(
      type: SourceType.smb,
      name: 'Study PC',
      host: '192.168.1.31',
      url: 'smb://192.168.1.31/',
    ),
  ],
  servers: [
    DiscoveredServer(name: 'Home server', host: 'media.local', port: 8487),
  ],
);

/// Demo services with the Streams section shown and five favourites.
Future<AppServices> twitchServices({
  TwitchBackend? twitch,
  DeviceSync? sync,
  PlayerEngineFactory? engineFactory,
  bool twitchNative = false,
}) async {
  final s = await demoServices(
    twitch: twitch,
    sync: sync,
    engineFactory: engineFactory,
    twitchNative: twitchNative,
  );
  await s.setShowTwitch(true);
  await seedDemoTwitchFavorites(s.twitch);
  return s;
}

/// "Now" of the demo services: a day after the demo data's reference time,
/// so "new" badges do not depend on the date the tests run.
final demoNow = DateTime(2026, 11, 14, 20);

final screenshotKey = GlobalKey();

Widget testApp(
  AppServices services, {
  FormFactor formFactor = FormFactor.desktop,
  Section section = Section.home,
}) {
  return RepaintBoundary(
    key: screenshotKey,
    child: KinoluneApp(
      services: services,
      formFactor: formFactor,
      initialSection: section,
    ),
  );
}

void setSurface(WidgetTester tester, Size logical, {double dpr = 1}) {
  tester.view.devicePixelRatio = dpr;
  tester.view.physicalSize = logical * dpr;
  addTearDown(tester.view.reset);
}

/// Pumps a few frames without waiting for repeating animations to settle.
Future<void> settle(WidgetTester tester, [int frames = 12]) async {
  for (var i = 0; i < frames; i++) {
    await tester.pump(const Duration(milliseconds: 100));
  }
}

Future<void> pressKey(
  WidgetTester tester,
  LogicalKeyboardKey key, {
  int times = 1,
}) async {
  for (var i = 0; i < times; i++) {
    await tester.sendKeyEvent(key);
    await tester.pump(const Duration(milliseconds: 80));
  }
  await settle(tester, 4);
}

/// Writes the current frame to `$KINOLUNE_SCREENSHOTS_DIR/<name>.png`.
Future<void> capture(WidgetTester tester, String name) async {
  final dir = Platform.environment['KINOLUNE_SCREENSHOTS_DIR'];
  if (dir == null || dir.isEmpty) return;
  final boundary =
      screenshotKey.currentContext!.findRenderObject()!
          as RenderRepaintBoundary;
  final dpr = tester.view.devicePixelRatio;
  final bytes = await tester.runAsync(() async {
    final ui.Image image = await boundary.toImage(pixelRatio: dpr);
    final data = await image.toByteData(format: ui.ImageByteFormat.png);
    image.dispose();
    return data!.buffer.asUint8List();
  });
  final file = File('$dir/$name.png');
  file.parent.createSync(recursive: true);
  file.writeAsBytesSync(bytes!);
}

/// Scrolls Home down until [finder] is on screen (UI-007: rows below the
/// Continue and Up Next rows start below the fold).
Future<void> scrollHomeTo(WidgetTester tester, Finder finder) async {
  await tester.scrollUntilVisible(
    finder,
    300,
    scrollable: find
        .descendant(
          of: find.byType(HomeScreen),
          matching: find.byType(Scrollable),
        )
        .first,
  );
  await settle(tester, 2);
}
