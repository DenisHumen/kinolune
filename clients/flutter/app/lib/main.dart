// SPDX-License-Identifier: MPL-2.0

import 'dart:async';
import 'dart:io' show Platform;

import 'package:material_ui/material_ui.dart';
import 'package:pa_core/pa_core.dart' as core;
import 'package:pa_platform_secrets/pa_platform_secrets.dart';
import 'package:path_provider/path_provider.dart';

import 'app.dart';
import 'branding.dart';
import 'build_info.dart';
import 'config.dart';
import 'fakes/fake_device_sync.dart';
import 'fakes/fake_library.dart';
import 'fakes/fake_source_manager.dart';
import 'fakes/fake_twitch.dart';
import 'flavor.dart';
import 'integration/library.dart';
import 'integration/player.dart';
import 'licenses.dart';
import 'platform/form_factor.dart';
import 'platform/libmpv_check.dart';
import 'screens/twitch/twitch_embed_screen.dart';
import 'services/app_services.dart';
import 'services/image_cache_control.dart';
import 'services/preferences.dart';
import 'services/secure_store.dart';
import 'sources/core_discovery.dart';
import 'sources/core_source_manager.dart';
import 'sources/discovery.dart';
import 'sources/source_manager.dart';
import 'sync/device_info.dart';
import 'sync/device_sync.dart';
import 'twitch/core_twitch_backend.dart';
import 'twitch/twitch_backend.dart';
import 'widgets/artwork_image.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  ensurePaPlayerInitialized();
  registerAppLicenses();

  final formFactor = await detectFormFactor();
  final dir = await getApplicationSupportDirectory();
  final Library library;
  SourceManager? sourceManager;
  DeviceSync sync = const NoDeviceSync();
  SourceDiscovery discovery = const NoSourceDiscovery();
  var rescan = false;
  var buildInfo = const BuildInfo();
  ImageCacheControl? imageCache;
  TwitchBackend? twitchBackend;
  var twitchNative = false;
  if (AppConfig.demo) {
    library = FakeLibrary.demo();
    sourceManager = FakeSourceManager(library);
    sync = FakeDeviceSync.demo();
    imageCache = demoImageCache();
    twitchBackend = FakeTwitchBackend.demo(now: DateTime.now());
  } else {
    // Rust core by default (UI-024); the v0 database is imported on the first
    // start and kept as kinolune.db.v0.bak. v0 is the fallback.
    final opened = await openAppLibrary(dir.path, rustCore: AppConfig.rustCore);
    library = opened.library;
    rescan = opened.importedV0;
    if (opened.fallbackReason != null) {
      debugPrint('Library on the v0 fallback: ${opened.fallbackReason}');
    }
    if (opened.onRustCore) {
      _checkCoreFlavor();
      buildInfo = _coreBuildInfo();
    }
    // CORE-015: on Android the core keeps passwords and tokens in the
    // Keystore through this callback; desktop uses the OS keychain itself.
    // Before CoreSources.open, which takes the store opened here.
    if (opened.onRustCore) await _openCoreSecrets(dir.path);
    // LIB-018: the core scans and streams SMB and WebDAV sources itself,
    // with the passwords of the store opened above.
    final lib = library;
    if (lib is RustLibrary) await lib.connectSources(dir.path);
    // Posters and backdrops through the core's disk cache (LIB-013); without
    // the core they load straight from the network.
    try {
      final cache = await getApplicationCacheDirectory();
      final artwork = await RustArtworkCache.open('${cache.path}/artwork');
      appArtworkCache = artwork;
      imageCache = RustImageCacheControl(artwork);
    } catch (e) {
      debugPrint('Artwork cache unavailable: $e');
    }
    // UI-005: the Rust source manager over the same database. Without it
    // (v0 fallback, or the secret store cannot open) the Sources screen
    // runs reduced: local folders only.
    if (opened.onRustCore) {
      try {
        sourceManager = await CoreSourceManager.open(
          dbPath: '${dir.path}/$coreDbFileName',
          dataDir: dir.path,
        );
      } catch (e) {
        debugPrint('Source manager unavailable: $e');
      }
      // UI-019: pairing and LAN sync over the same database.
      try {
        sync = await CoreDeviceSync.open(
          library: (library as RustLibrary).core,
          dataDir: '${dir.path}/sync',
          deviceName: await syncDeviceName(formFactor),
          deviceClass: switch (formFactor) {
            FormFactor.tv => CoreDeviceClass.tv,
            FormFactor.phone => CoreDeviceClass.mobile,
            FormFactor.desktop => CoreDeviceClass.desktop,
          },
        );
        await holdMulticastLock(true);
      } catch (e) {
        debugPrint('Device sync unavailable: $e');
      }
      // TWI-013: the Streams section over the same database (favourites,
      // cache, progress); Helix only with a client id.
      try {
        twitchBackend = await CoreTwitchBackend.open(
          dbPath: '${dir.path}/$coreDbFileName',
          dataDir: dir.path,
          clientId: AppConfig.twitchClientId.isEmpty
              ? null
              : AppConfig.twitchClientId,
        );
      } catch (e) {
        debugPrint('Twitch section on local favourites only: $e');
      }
      // TWI-004: native Twitch playback only in full builds and only while
      // the signed remote config allows it.
      if (AppFlavor.twitchNative) twitchNative = _twitchNativeAllowed(dir.path);
      // NET-020: shares and servers on the LAN for the add-source flow
      // (mDNS through the core; browses only while that flow is open).
      if (sourceManager != null) {
        try {
          discovery = await CoreSourceDiscovery.open();
        } catch (e) {
          debugPrint('LAN discovery unavailable: $e');
        }
      }
    }
  }
  final services = AppServices(
    library: library,
    secureStore: PlatformSecureStore(),
    preferences: FilePreferences('${dir.path}/preferences.json'),
    sourceManager: sourceManager,
    discovery: discovery,
    sync: sync,
    buildInfo: buildInfo,
    imageCache: imageCache,
    twitchBackend: twitchBackend,
    twitchNativeAllowed: twitchNative,
  );
  await services.init();
  if (AppConfig.demo) {
    await seedDemoTwitchFavorites(services.twitch);
  } else {
    // TWI-005: the official embed where a WebView exists (the demo's
    // channels are made up: it keeps "Open on Twitch").
    registerTwitchEmbed(services.twitch);
  }
  // Music tags and network-share items of an imported v0 library come with
  // a scan.
  if (rescan) unawaited(services.scan());
  runApp(
    KinoluneApp(
      services: services,
      formFactor: formFactor,
      libmpvMissing: libmpvMissing(),
    ),
  );
}

/// CORE-013: the Dart and Rust halves must be the same flavor. A mismatch
/// is a packaging error: fatal in debug builds, logged in release builds.
void _checkCoreFlavor() {
  final problem = checkCoreFlavor(core.buildFlavor());
  if (problem == null) return;
  debugPrint('Build flavor mismatch: $problem');
  assert(false, 'Build flavor mismatch: $problem');
}

/// About (UI-016): the core's own version, flavor and capabilities.
BuildInfo _coreBuildInfo() {
  try {
    return BuildInfo(
      coreVersion: core.coreVersion(),
      coreFlavor: core.buildFlavor(),
      coreCapabilities: core.buildCapabilities(),
    );
  } catch (e) {
    debugPrint('Core build info not read: $e');
    return const BuildInfo();
  }
}

Future<void> _openCoreSecrets(String dataDir) async {
  try {
    final info = await openPlatformSecrets(dataDir: dataDir);
    if (info != null) debugPrint('Core secrets: ${info.backend}');
  } catch (e) {
    // Sources with passwords fail with a `secrets` error until the next start.
    debugPrint('Core secret store unavailable: $e');
  }
}

/// TWI-004: the signed remote config allows native Twitch playback
/// (`twitch.gql_playback`, `player = native`, a GQL client id, no
/// kill-switch). Reads the cached or built-in copy; no network.
bool _twitchNativeAllowed(String dataDir) {
  try {
    final config = core.CoreRemoteConfig.open(
      cacheDir: '$dataDir/remote-config',
      appVersion: kAppVersion,
      platform: Platform.operatingSystem,
    );
    final flags = config.twitch();
    return flags.gqlPlayback &&
        flags.player == core.TwitchPlayer.native &&
        (flags.gqlClientId ?? '').isNotEmpty;
  } catch (e) {
    debugPrint('Remote config unavailable: $e');
    return false;
  }
}
