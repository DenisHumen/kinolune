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
import 'diagnostics/core_diagnostics.dart';
import 'diagnostics/diagnostics.dart';
import 'features/updates/update_setup.dart';
import 'fakes/fake_device_sync.dart';
import 'fakes/fake_library.dart';
import 'fakes/fake_offline.dart';
import 'fakes/fake_personal_media.dart';
import 'fakes/fake_review_inbox.dart';
import 'fakes/fake_source_manager.dart';
import 'fakes/fake_twitch.dart';
import 'flavor.dart';
import 'integration/library.dart';
import 'integration/player.dart';
import 'l10n/l10n.dart';
import 'licenses.dart';
import 'offline/background_downloads.dart';
import 'offline/network_watch.dart';
import 'offline/offline_downloads.dart';
import 'platform/app_links.dart';
import 'platform/form_factor.dart';
import 'platform/libmpv_check.dart';
import 'screens/twitch/twitch_embed_screen.dart';
import 'screens/twitch/twitch_player_screen.dart';
import 'services/app_services.dart';
import 'services/image_cache_control.dart';
import 'services/library_auto_check.dart';
import 'services/preferences.dart';
import 'services/secret_storage.dart';
import 'services/secure_store.dart';
import 'services/startup_probe.dart';
import 'sources/core_discovery.dart';
import 'sources/core_source_manager.dart';
import 'sources/discovery.dart';
import 'sources/jellyfin_link.dart';
import 'sources/plex_link.dart';
import 'sources/subsonic_link.dart';
import 'sources/source_manager.dart';
import 'sync/device_info.dart';
import 'sync/device_sync.dart';
import 'sync/sync_pace.dart';
import 'twitch/core_live_source.dart';
import 'twitch/core_twitch_backend.dart';
import 'twitch/core_twitch_native.dart';
import 'twitch/live_notifications.dart';
import 'twitch/live_notifications_platform.dart';
import 'twitch/twitch_archive.dart';
import 'twitch/twitch_backend.dart';
import 'twitch/twitch_downloads.dart';
import 'twitch/twitch_native.dart';
import 'twitch/twitch_service.dart';
import 'widgets/artwork_image.dart';

Future<void> main(List<String> args) async {
  WidgetsFlutterBinding.ensureInitialized();
  // UI-022: `kinolune://` links and files of the command line (desktop).
  LaunchLinks.initial = args;
  ensurePaPlayerInitialized();
  registerAppLicenses();

  final formFactor = await detectFormFactor();
  final dir = await getApplicationSupportDirectory();
  // CORE-008: opt-in logs and crash reports (nothing is written without it).
  if (!AppConfig.demo) await startDiagnostics(dir.path);
  final Library library;
  SourceManager? sourceManager;
  JellyfinLink? jellyfin;
  PlexLink? plex;
  SubsonicLink? subsonic;
  DeviceSync sync = const NoDeviceSync();
  SourceDiscovery discovery = const NoSourceDiscovery();
  var rescan = false;
  var v0SourceIds = const <String, String>{};
  var buildInfo = const BuildInfo();
  ImageCacheControl? imageCache;
  TwitchBackend? twitchBackend;
  var twitchNative = false;
  var twitchNativeKilled = false;
  TwitchDownloads? twitchDownloads;
  CoreTwitchArchive? twitchArchive;
  OfflineController? offline;
  var secretStorage = const SecretStorageInfo.unknown();
  TwitchNativeSource? twitchNativeSource;
  if (AppConfig.demo) {
    library = FakeLibrary.demo();
    sourceManager = FakeSourceManager(library);
    sync = FakeDeviceSync.demo();
    imageCache = demoImageCache();
    twitchBackend = FakeTwitchBackend.demo(now: DateTime.now());
    offline = OfflineController(FakeOfflineBackend.demo());
  } else {
    // Rust core by default (UI-024); the v0 database is imported on the first
    // start and kept as kinolune.db.v0.bak. v0 is the fallback.
    final opened = await openAppLibrary(dir.path, rustCore: AppConfig.rustCore);
    library = opened.library;
    rescan = opened.importedV0;
    v0SourceIds = opened.v0SourceIds;
    if (opened.fallbackReason != null) {
      debugPrint('Library on the v0 fallback: ${opened.fallbackReason}');
    }
    if (opened.onRustCore) {
      _checkCoreFlavor();
      buildInfo = _coreBuildInfo();
    }
    // CORE-015: on Android the core keeps passwords and tokens in the
    // Keystore through this callback; desktop uses the OS keychain or, when
    // it does not work, an encrypted file (CORE-007). Before
    // CoreSources.open, which takes the store opened here.
    if (opened.onRustCore) secretStorage = await _openCoreSecrets(dir.path);
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
      // NET-014: Jellyfin sign-in, playback method and reports.
      try {
        jellyfin = await CoreJellyfinLink.open(
          library: (library as RustLibrary).core,
          dataDir: dir.path,
        );
      } catch (e) {
        debugPrint('Jellyfin unavailable: $e');
      }
      // NET-012: Plex sign-in with a code at plex.tv/link.
      try {
        plex = await CorePlexLink.open(
          library: (library as RustLibrary).core,
          dataDir: dir.path,
        );
      } catch (e) {
        debugPrint('Plex unavailable: $e');
      }
      // NET-016: Subsonic server checks and scrobbles.
      try {
        subsonic = await CoreSubsonicLink.open(
          library: (library as RustLibrary).core,
          dataDir: dir.path,
        );
      } catch (e) {
        debugPrint('Subsonic unavailable: $e');
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
      // the signed remote config allows it (the player: TWI-014).
      if (AppFlavor.twitchNative &&
          twitchLabOrigin(AppConfig.twitchLab) == null) {
        final (source, killed) = _twitchNativeSource(
          dir.path,
          (await runningVersion()).text,
        );
        twitchNativeSource = source;
        twitchNative = source != null;
        twitchNativeKilled = killed;
      }
      // TWI-017: VOD downloads, full builds only, while the signed remote
      // config allows them (`twitch.archive`) and someone is signed in.
      if (AppFlavor.twitchArchive) {
        if ((twitchBackend, library) case (
          final CoreTwitchBackend backend,
          final RustLibrary rust,
        )) {
          final queue = await _twitchDownloads(backend, rust, dir.path);
          twitchDownloads = queue;
          // TWI-019: recordings, rules, saved files on the same queue; rule
          // changes carry the library's parent token (ADR-0097).
          if (queue != null) {
            twitchArchive = CoreTwitchArchive(
              queue.coreQueue,
              parentToken: () => rust.parentToken,
            );
          }
        }
      }
      // NET-019: offline copies of network items, on the job queue of
      // the library database.
      if (library case final RustLibrary rust) {
        final backend = await CoreOfflineBackend.open(
          library: rust.core,
          folder: await offlineFolder(dir.path),
        );
        if (backend != null) {
          offline = OfflineController(backend);
          watchNetwork(offline);
        }
      }
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
  // TWI-014: the player lab (manual checks) stands in for usher; full
  // builds with the dart-define only.
  // Debug and profile builds only, loopback only (`twitchLabOrigin`).
  final lab = twitchLabOrigin(AppConfig.twitchLab);
  if (AppFlavor.twitchNative && lab != null) {
    try {
      await RustLibrary.ensureInitialized();
      twitchNativeSource = LabTwitchNativeSource(lab);
      twitchNative = true;
    } catch (e) {
      debugPrint('Twitch player lab unavailable: $e');
    }
  }
  // TWI-009: "X is live" — the core's watcher for the signed-in account
  // (EventSub + polling), or its made-up Twitch in demo mode.
  late final AppServices services;
  final preferences = FilePreferences('${dir.path}/preferences.json');
  LiveEventSource? Function(TwitchService)? liveSources;
  if (AppConfig.demo) {
    try {
      await RustLibrary.ensureInitialized();
      liveSources = (_) => CoreLiveSource.demo();
    } catch (e) {
      debugPrint('Demo live notifications unavailable: $e');
    }
  } else {
    liveSources = (twitch) => switch (twitch.backend) {
      // TWI-019: AutoRecord follows the monitor's `stream.online` in the
      // core (full builds with the archive queue open).
      final CoreTwitchBackend b => b.liveSource(
        onMonitor: twitchArchive?.followLive,
      ),
      _ => null,
    };
  }
  services = AppServices(
    library: library,
    secureStore: PlatformSecureStore(),
    v0SourceIds: v0SourceIds,
    preferences: preferences,
    sourceManager: sourceManager,
    jellyfin: jellyfin,
    plex: plex,
    subsonic: subsonic,
    discovery: discovery,
    sync: sync,
    buildInfo: buildInfo,
    imageCache: imageCache,
    twitchBackend: twitchBackend,
    twitchNativeAllowed: twitchNative,
    twitchNativeKilled: twitchNativeKilled,
    twitchDownloads: twitchDownloads,
    twitchArchive: twitchArchive,
    offline: offline,
    secretStorage: secretStorage,
    // PKG-018: full builds only; the daily check starts in the background.
    updates: AppConfig.demo
        ? null
        : await openUpdates(
            dataDir: dir.path,
            cacheDir: (await getApplicationCacheDirectory()).path,
            preferences: preferences,
            onRustCore: buildInfo.coreVersion != null,
            // PKG-024: a TV never restarts by itself.
            tv: formFactor == FormFactor.tv,
          ),
    // UI-025: the demo's personal libraries (PIN 2468).
    personal: AppConfig.demo ? FakePersonalMedia.demo() : null,
    // META-016: the demo's review inbox.
    reviewInbox: AppConfig.demo ? FakeReviewInbox.demo() : null,
    liveSources: liveSources,
    liveNotifier: PluginLiveNotifier(
      l10n: () => lookupAppLocalizations(
        services.locale.value ?? services.systemLocale(),
      ),
    ),
  );
  // Phones poll at the battery interval; desktops and TVs are on mains.
  services.liveNotifications.setPower(
    formFactor == FormFactor.phone ? LivePower.battery : LivePower.plugged,
  );
  await services.init();
  unawaited(
    collectAndroidExitReasons(Diagnostics.instance, services.preferences),
  );
  _watchLiveNotifications(services);
  // LIB-024: sources are checked when the app comes back and when the
  // network returns; no manual rescan.
  if (!AppConfig.demo) LibraryAutoCheck.attach(services.checkLibraryNow);
  // SYNC-014: folder sync looks often in front, rarely in the background.
  if (!AppConfig.demo) watchSyncPace(services.sync);
  // NET-019 (ADR-0097): downloads go on in the background on Android (a
  // dataSync foreground service, WorkManager kicks); not for demo data.
  if (Platform.isAndroid && !AppConfig.demo && offline != null) {
    unawaited(
      BackgroundDownloads(offline, preferences: services.preferences).start(),
    );
  }
  // TWI-014: the native player; `decideTwitchRoute` still picks it only in
  // a full build whose config allows it.
  if (twitchNativeSource != null) {
    registerTwitchNativePlayer(services.twitch, twitchNativeSource);
  }
  if (AppConfig.demo) {
    await seedDemoTwitchFavorites(services.twitch);
  } else {
    // TWI-005, TWI-029: the official embed where a web view exists (the
    // demo's channels are made up: it keeps "Open on Twitch"). Windows and
    // Linux check their runtime first; startup does not wait for it.
    unawaited(registerTwitchEmbed(services.twitch));
  }
  // Music tags and network-share items of an imported v0 library come with
  // a scan.
  if (rescan) unawaited(services.scan());
  // QA-006: cold-start timing, only with KINOLUNE_STARTUP_PROBE set.
  armStartupProbe();
  runApp(
    KinoluneApp(
      services: services,
      formFactor: formFactor,
      libmpvMissing: libmpvMissing(),
    ),
  );
}

/// TWI-009: polls when the app comes back; restarts the watcher after a
/// sign-in change; schedules the Android background check while
/// notifications are on and someone is signed in.
void _watchLiveNotifications(AppServices services) {
  final live = services.liveNotifications;
  AppLifecycleListener(onResume: live.resumed);
  var account = services.twitch.account?.userId;
  bool? scheduled;
  void sync() {
    final now = services.twitch.account?.userId;
    if (now != account) {
      account = now;
      unawaited(live.restart());
    }
    final on = !AppConfig.demo && live.enabled && now != null;
    if (on != scheduled) {
      scheduled = on;
      unawaited(scheduleBackgroundLiveChecks(on: on));
    }
  }

  services.twitch.addListener(sync);
  live.addListener(sync);
  sync();
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

/// Opens the core's secret store: the platform store on Android, else the
/// OS keychain with the encrypted-file fallback (CORE-007). The core bounds
/// the keychain with deadlines, so a locked or silent keyring costs a few
/// seconds at most and never hangs the start.
Future<SecretStorageInfo> _openCoreSecrets(String dataDir) async {
  try {
    final info =
        await openPlatformSecrets(dataDir: dataDir) ??
        await core.secretsOpenDefault(dataDir: dataDir);
    debugPrint('Core secrets: ${info.backend}');
    if (info.fallbackReason case final reason?) {
      debugPrint('Core secrets in the encrypted file: $reason');
    }
    return SecretStorageInfo(
      backend: info.backend,
      fallbackKind: info.fallbackKind == null
          ? null
          : SecretFallbackKind.fromName(info.fallbackKind),
      fallbackReason: info.fallbackReason,
      fallbackFile: info.fallbackFile,
    );
  } catch (e) {
    // Sources with passwords fail with a `secrets` error until the next start.
    debugPrint('Core secret store unavailable: $e');
    return const SecretStorageInfo.unknown();
  }
}

/// TWI-017/TWI-019: the download and recording queue of the core into
/// `<data>/downloads/twitch` and `<data>/recordings/twitch` (storage
/// targets: quotas, the free-space reserve and retention apply; the user
/// may choose other folders in Settings).
Future<CoreTwitchDownloadsSource?> _twitchDownloads(
  CoreTwitchBackend backend,
  RustLibrary library,
  String dataDir,
) async {
  try {
    final config = core.CoreRemoteConfig.open(
      cacheDir: '$dataDir/remote-config',
      appVersion: (await runningVersion()).text,
      platform: Platform.operatingSystem,
    );
    return await CoreTwitchDownloadsSource.open(
      twitch: backend.coreTwitch,
      library: library.core,
      config: config,
      downloadsDir: '$dataDir/downloads/twitch',
      recordingsDir: '$dataDir/recordings/twitch',
    );
  } catch (e) {
    debugPrint('Twitch downloads unavailable: $e');
    return null;
  }
}

/// TWI-004/TWI-014: the native playback path when the signed remote
/// config allows it (`twitch.gql_playback`, `player = native`, a GQL client
/// id and query hash, no kill-switch); else null. Reads the cached or
/// built-in copy; no network. TWI-030: also whether a kill-switch (by
/// [appVersion] — the real build version, not [kAppVersion] — and
/// platform) sends this app back to the embed.
(TwitchNativeSource?, bool) _twitchNativeSource(
  String dataDir,
  String appVersion,
) {
  try {
    final config = core.CoreRemoteConfig.open(
      cacheDir: '$dataDir/remote-config',
      appVersion: appVersion,
      platform: Platform.operatingSystem,
    );
    final killed =
        config.isKilled(feature: 'twitch.gql_playback') ||
        config.isKilled(feature: 'twitch.native_player');
    return (CoreTwitchNativeSource.open(config), killed);
  } catch (e) {
    debugPrint('Remote config unavailable: $e');
    return (null, false);
  }
}
