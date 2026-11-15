// SPDX-License-Identifier: MPL-2.0

import 'dart:async';
import 'dart:convert';

import 'package:material_ui/material_ui.dart';
import 'package:pa_player/pa_player.dart'
    show AudioProfile, ReplayGainMode, kSkipStepsSeconds;

import '../build_info.dart';
import '../features/music/music_queue_store.dart';
import '../features/music/smart_playlists.dart';
import '../features/twitch/chat/chat_settings.dart';
import '../flavor.dart';
import '../integration/library.dart';
import '../integration/player.dart';
import '../l10n/l10n.dart';
import '../library/library_queries.dart';
import '../offline/offline_downloads.dart';
import '../profiles/profile_controller.dart';
import '../sources/discovery.dart';
import '../sources/library_source_manager.dart';
import '../sources/source_manager.dart';
import '../sync/device_sync.dart';
import '../sync/handoff.dart';
import '../twitch/live_notifications.dart';
import '../twitch/twitch_backend.dart';
import '../twitch/twitch_downloads.dart';
import '../twitch/twitch_service.dart';
import 'image_cache_control.dart';
import 'local_profile.dart';
import 'preferences.dart';
import 'secret_storage.dart';
import 'secure_store.dart';

/// App-wide services, provided to the tree by [Services].
class AppServices extends ChangeNotifier {
  AppServices({
    required this.library,
    required this.secureStore,
    Preferences? preferences,
    MusicPlayerController? music,
    this.playerEngineFactory,
    Locale Function()? systemLocale,
    SourceManager? sourceManager,
    this.discovery = const NoSourceDiscovery(),
    this.sync = const NoDeviceSync(),
    this.onRemoteLibraryChange,
    LibraryQueries? queries,
    DateTime Function()? clock,
    this.buildInfo = const BuildInfo(),
    this.imageCache,
    TwitchBackend? twitchBackend,
    bool twitchNativeAllowed = false,
    this.twitchDownloads,
    this.offline,
    this.secretStorage = const SecretStorageInfo.unknown(),
    bool? twitchFullBuild,
    this.profileStore,
    LiveEventSource? Function(TwitchService twitch)? liveSources,
    LiveNotifier liveNotifier = const NoLiveNotifier(),
  }) : preferences = preferences ?? MemoryPreferences(),
       queries = queries ?? LibraryQueries.forLibrary(library),
       clock = clock ?? DateTime.now,
       sourceManager = sourceManager ?? LibrarySourceManager(library),
       systemLocale = systemLocale ?? _platformLocale,
       music =
           music ?? MusicPlayerController(engineFactory: playerEngineFactory) {
    twitch = TwitchService(
      backend: twitchBackend ?? const LocalTwitchBackend(),
      preferences: this.preferences,
      clock: this.clock,
      fullBuild: twitchFullBuild ?? AppFlavor.twitchNative,
      nativeAllowedByConfig: twitchNativeAllowed,
    );
    liveNotifications = LiveNotificationController(
      twitch: twitch,
      preferences: this.preferences,
      notifier: liveNotifier,
      sourceFor: () => _liveAllowed ? liveSources?.call(twitch) : null,
      texts: (a) => liveTexts(
        lookupAppLocalizations(locale.value ?? this.systemLocale()),
        a,
      ),
    );
  }

  /// "X is live" notifications of favourites (TWI-009).
  late final LiveNotificationController liveNotifications;

  /// Twitch VOD downloads (TWI-017): full builds while the signed remote
  /// config allows them and someone is signed in; else null.
  final TwitchDownloads? twitchDownloads;

  /// Offline copies of network items (NET-019) on the Rust core; null
  /// without it.
  final OfflineController? offline;

  /// "Wi-Fi only" of offline downloads, kept in [preferences].
  Future<void> setOfflineWifiOnly(bool on) async {
    await preferences.write(Preferences.offlineWifiOnly, on ? null : 'false');
    await offline?.setWifiOnly(on);
  }

  Future<void> _initOffline() async {
    final off = offline;
    if (off == null) return;
    try {
      await off.backend.setWifiOnly(
        await preferences.read(Preferences.offlineWifiOnly) != 'false',
      );
      await off.start();
      // Watched copies with "delete watched" on.
      unawaited(off.cleanup());
    } catch (e) {
      debugPrint('Offline downloads not started: $e');
    }
  }

  /// The active profile may get "X is live" notifications: not a kid
  /// profile and without parental filters (UI-015 hides Twitch for them).
  bool _liveAllowed = true;

  /// Re-reads [_liveAllowed] for the active profile and restarts the
  /// watcher (a profile switch, LIB-012).
  Future<void> refreshLiveProfile() async {
    final core = profiles;
    if (core != null) {
      try {
        final active = await core.activeProfile();
        // UI-015: no alerts where the Streams section is hidden (by
        // default every restricted profile; a parent may change it).
        _liveAllowed = profileControl.active?.id == active.id
            ? !profileControl.hidesTwitch(active)
            : !active.isKid && !active.isRestricted;
      } catch (e) {
        debugPrint('Active profile not read: $e');
      }
    }
    await liveNotifications.restart();
  }

  /// The "Streams" section (Twitch, TWI-013): favourites without a login,
  /// Helix data when signed in, the Continue hook of Home.
  late final TwitchService twitch;

  final Library library;

  /// Version, flavor and capabilities for About (UI-016).
  final BuildInfo buildInfo;

  /// Where the core keeps passwords and tokens (CORE-007), for Settings →
  /// Privacy and the one-time fallback notice.
  final SecretStorageInfo secretStorage;

  /// The poster and backdrop cache (LIB-013) for Settings → Storage; null
  /// without the core.
  final ImageCacheControl? imageCache;

  /// Queries of the library grid (UI-008): LIB-009 on the Rust core, in
  /// memory without filters on the v0 fallback.
  final LibraryQueries queries;

  /// Current time ("new" badges); tests pin it.
  final DateTime Function() clock;

  final SecureStore secureStore;
  final Preferences preferences;
  final MusicPlayerController music;

  /// Settings and status of sources (UI-005): the Rust source manager, or
  /// the reduced v0 one ([LibrarySourceManager]).
  final SourceManager sourceManager;

  /// LAN discovery for the add flow: `CoreSourceDiscovery` on the Rust
  /// core (NET-020), off on the v0 fallback.
  final SourceDiscovery discovery;

  /// Pairing and LAN sync with the user's other devices (UI-019).
  final DeviceSync sync;

  /// Refreshes the library views when another device changed progress;
  /// default: [RustLibrary.notifyExternalChange].
  final void Function()? onRemoteLibraryChange;
  StreamSubscription<SyncEvent>? _syncSub;

  /// The playback session between devices (SYNC-003): heartbeats,
  /// takeover, "Jump to …?" and the Home banner.
  late final HandoffController handoff = HandoffController(sync);

  /// What other devices play now (`pb.update` / `pb.takeover`) as raw
  /// events; [handoff] keeps the state.
  Stream<SyncEvent> get playbackElsewhere => sync.events.where(
    (e) => e.kind == SyncEventKind.playback || e.kind == SyncEventKind.takeover,
  );

  /// UI language chosen in Settings; null = follow the system locale.
  final ValueNotifier<Locale?> locale = ValueNotifier(null);

  /// The system UI language, for [locale] == null (tests override it).
  final Locale Function() systemLocale;

  /// The first preferred system language the app has strings for (as the
  /// framework resolves it), else English.
  static Locale _platformLocale() {
    final supported = {
      for (final l in AppLocalizations.supportedLocales) l.languageCode,
    };
    for (final l in WidgetsBinding.instance.platformDispatcher.locales) {
      if (supported.contains(l.languageCode)) return l;
    }
    return const Locale('en');
  }

  /// Subtitle style (PLAY-007): the system caption style by default.
  final ValueNotifier<SubtitleSettings> subtitleSettings = ValueNotifier(
    const SubtitleSettings(),
  );

  /// Languages the user reads subtitles in, best first: the UI language.
  /// The player puts the system caption language before them.
  List<String> get subtitleLanguages => [
    (locale.value ?? systemLocale()).languageCode,
  ];

  /// Whether the Twitch section is in the navigation. Off by default until
  /// the Twitch feature ships (after release 0.1); toggled in Settings.
  final ValueNotifier<bool> showTwitch = ValueNotifier(false);

  /// Twitch chat (TWI-012): animated emotes stand still. Off by default;
  /// the system's reduce-motion setting stops them too.
  final ValueNotifier<bool> twitchChatPauseAnimations = ValueNotifier(false);

  /// Twitch chat filters and highlights of the active profile (TWI-024);
  /// a chat loads them when it opens.
  late final ChatSettingsStore twitchChatSettings = ChatSettingsStore(
    preferences,
    () async => (await profiles?.activeProfile())?.syncId,
  );

  /// The music queue of the active profile, kept across restarts
  /// (MUS-003).
  late final MusicQueueStore musicQueue = MusicQueueStore(
    player: music,
    preferences: preferences,
    profileId: _activeSyncId,
    libraryTracks: () => libraryTrackRequests(library),
  );

  /// Smart playlists of the active profile (MUS-003): rules in
  /// Preferences, evaluated over the music tracks.
  late final SmartPlaylistStore smartPlaylists = SmartPlaylistStore(
    preferences,
    _activeSyncId,
  );

  Future<String?> _activeSyncId() async =>
      (await profiles?.activeProfile())?.syncId;

  /// Hide Spoilers (UI-010): the Show page blurs overviews and stills of
  /// episodes not started yet. Off by default; toggled in Settings.
  final ValueNotifier<bool> hideSpoilers = ValueNotifier(false);

  /// Continuous playback (UI-016): Up Next offers the next episode at the
  /// end of one. On by default.
  final ValueNotifier<bool> continuousPlayback = ValueNotifier(true);

  /// Skip interval of the player (UI-013), seconds; one of
  /// [kSkipStepsSeconds]. 10 by default.
  final ValueNotifier<int> skipSeconds = ValueNotifier(10);

  /// Skip intro and credits (PLAY-011): a button, automatic after 3 s, at
  /// once, or off. A button by default.
  final ValueNotifier<SkipMode> skipMode = ValueNotifier(SkipMode.button);

  /// Seek previews (PLAY-009): videos without previews of their own get
  /// them generated while they play. On by default.
  final ValueNotifier<bool> trickplayGenerate = ValueNotifier(true);

  /// Episode order of shows without one set on their page (UI-016).
  final ValueNotifier<EpisodeOrder> defaultEpisodeOrder = ValueNotifier(
    EpisodeOrder.tmdb,
  );

  /// Metadata language chosen in Settings (`en-US`, `ru-RU`); null = the
  /// UI language.
  final ValueNotifier<String?> metadataLanguage = ValueNotifier(null);

  /// Overrides the media_kit engine (tests use pa_player's FakePlayerEngine).
  final PlayerEngineFactory? playerEngineFactory;

  /// Name and avatar of the active profile (UI-004); null until it is
  /// named. On the Rust core it is the core's active profile (LIB-012).
  final ValueNotifier<LocalProfile?> profile = ValueNotifier(null);

  /// The core's profiles (LIB-012); null for a library without them (the
  /// v0 and demo libraries). Tests and the demo may pass their own.
  LibraryProfiles? get profiles =>
      profileStore ??
      (library is LibraryProfiles ? library as LibraryProfiles : null);

  /// Profiles other than the library's own (the demo, tests).
  final LibraryProfiles? profileStore;

  /// Profiles and parental control in the app (UI-015): the picker, PINs,
  /// App Lock, Search Lock, sections hidden for restricted profiles.
  late final ProfileController profileControl = ProfileController(
    store: profiles,
    preferences: preferences,
    hasMusic: () async => (await library.tracks().first).isNotEmpty,
  );

  /// The Streams section is in the navigation: on in Settings and not
  /// hidden for the active profile (UI-015).
  bool get twitchVisible => twitchShown.value;

  /// [twitchVisible] as a listenable.
  final ValueNotifier<bool> twitchShown = ValueNotifier(false);

  void _updateTwitchShown() =>
      twitchShown.value = showTwitch.value && !profileControl.twitchHidden;

  /// How the user watches, as chosen in the first-run setup; null = not
  /// asked yet.
  final ValueNotifier<AppMode?> appMode = ValueNotifier(null);

  bool _onboardingPending = false;

  /// Whether the app starts in the first-run setup (UI-004): it was never
  /// finished or skipped and there is nothing to show yet (no sources). An
  /// install that already has sources goes straight to the library.
  bool get onboardingPending => _onboardingPending;

  bool _secretNoticePending = false;

  /// Whether to show the one-time notice that passwords are in the
  /// encrypted file because the OS keychain does not work (CORE-007).
  bool get secretNoticePending => _secretNoticePending;

  /// The notice was read; it is not shown again. Settings → Privacy keeps
  /// saying where secrets are.
  Future<void> dismissSecretNotice() async {
    _secretNoticePending = false;
    notifyListeners();
    await preferences.write(Preferences.secretFallbackNoticeShown, 'true');
  }

  List<MediaSource> _sources = const [];
  List<MediaSource> get sources => _sources;

  List<ManagedSource> _managed = const [];

  /// [sources] with settings and status, from [sourceManager].
  List<ManagedSource> get managedSources => _managed;

  /// Why [managedSources] could not be read, if it failed.
  Object? get managedSourcesError => _managedError;
  Object? _managedError;

  /// End time and error of each source's last scan in this session (the
  /// reduced manager has no stored status).
  final Map<String, ({DateTime at, String? error})> sessionScans = {};

  /// Sources a scan is running for (all of them for a full scan).
  final Set<String> scanningSources = {};
  bool _loaded = false;
  bool get loaded => _loaded;

  ScanProgress? _scan;
  bool _scanRunning = false;
  ScanProgress? get scanProgress => _scan;
  bool get scanning => _scanRunning;

  Future<void> init() async {
    locale.value = parseLocaleTag(await preferences.read(Preferences.locale));
    metadataLanguage.value = _metadataLanguage(
      await preferences.read(Preferences.metadataLanguage),
    );
    await _syncMetadataLanguage();
    showTwitch.value = await preferences.read(Preferences.showTwitch) == 'true';
    await twitch.init();
    twitchChatPauseAnimations.value =
        await preferences.read(Preferences.twitchChatPauseAnimations) == 'true';
    hideSpoilers.value =
        await preferences.read(Preferences.hideSpoilers) == 'true';
    handoff.pauseOnTakeover.value =
        await preferences.read(Preferences.takeoverPause) != 'false';
    continuousPlayback.value =
        await preferences.read(Preferences.continuousPlayback) != 'false';
    final skip = int.tryParse(
      await preferences.read(Preferences.skipSeconds) ?? '',
    );
    skipSeconds.value = kSkipStepsSeconds.contains(skip) ? skip! : 10;
    final mode = await preferences.read(Preferences.skipMode);
    skipMode.value = SkipMode.values.firstWhere(
      (m) => m.name == mode,
      orElse: () => SkipMode.button,
    );
    _applyTrickplay(
      await preferences.read(Preferences.trickplayGenerate) != 'false',
    );
    final order = await preferences.read(Preferences.defaultEpisodeOrder);
    defaultEpisodeOrder.value = EpisodeOrder.values.firstWhere(
      (o) => o.name == order,
      orElse: () => EpisodeOrder.tmdb,
    );
    await music.setAudioProfile(
      music.audioProfile.copyWith(
        gapless: await preferences.read(Preferences.musicGapless) != 'false',
        replayGain: _replayGain(
          await preferences.read(Preferences.musicReplayGain),
        ),
      ),
    );
    final limit = int.tryParse(
      await preferences.read(Preferences.imageCacheMaxBytes) ?? '',
    );
    final cache = imageCache;
    if (limit != null && limit > 0 && cache != null) {
      try {
        await cache.setMaxBytes(limit);
      } catch (e) {
        debugPrint('Image cache limit not applied: $e');
      }
    }
    subtitleSettings.value = SubtitleSettings.fromJson(
      _json(await preferences.read(Preferences.subtitleSettings)),
    );
    profile.value = await _loadProfile(
      LocalProfile.fromJson(
        _json(await preferences.read(Preferences.localProfile)),
      ),
    );
    appMode.value = AppMode.fromName(
      await preferences.read(Preferences.appMode),
    );
    try {
      await profileControl.init();
      if (profileControl.active != null) {
        _liveAllowed = !profileControl.twitchHidden;
      }
    } catch (e) {
      debugPrint('Profiles not loaded: $e');
    }
    profileControl
      ..onSwitched = _onProfileSwitched
      ..addListener(notifyListeners)
      ..addListener(_updateTwitchShown);
    showTwitch.addListener(_updateTwitchShown);
    _updateTwitchShown();
    // TWI-009: after the profile is known (no notifications for kids).
    await liveNotifications.init();
    // MUS-003: the queue of the profile, paused where it stopped.
    unawaited(musicQueue.load());
    unawaited(smartPlaylists.load());
    _sources = await library.sources();
    _onboardingPending =
        await preferences.read(Preferences.onboardingDone) != 'true' &&
        _sources.isEmpty;
    _secretNoticePending =
        secretStorage.isFallback &&
        await preferences.read(Preferences.secretFallbackNoticeShown) != 'true';
    await _loadManaged();
    await _initOffline();
    await migrateLegacySmbPasswords();
    final key = await secureStore.read(SecureStore.tmdbKey);
    if (key != null && key.isNotEmpty) await library.setTmdbApiKey(key);
    _loaded = true;
    notifyListeners();
    unawaited(startWatching());
    unawaited(startSync());
  }

  /// Listens to the sync service and starts it if devices are paired.
  Future<void> startSync() async {
    _syncSub ??= sync.events.listen(_onSyncEvent);
    handoff.start();
    try {
      await sync.start();
    } catch (e) {
      // Pairing still works from the Devices screen; sync retries there.
      debugPrint('Sync not started: $e');
    }
  }

  void _onSyncEvent(SyncEvent e) {
    switch (e.kind) {
      case SyncEventKind.libraryChanged:
        final refresh = onRemoteLibraryChange;
        if (refresh != null) {
          refresh();
        } else if (library is RustLibrary) {
          (library as RustLibrary).notifyExternalChange();
        }
      case SyncEventKind.devicesChanged ||
          SyncEventKind.paired ||
          SyncEventKind.inviteClosed:
        notifyListeners();
      case SyncEventKind.pairingFailed ||
          SyncEventKind.playback ||
          SyncEventKind.takeover:
        break;
    }
  }

  /// Follows the auto-scan sources (LIB-005): new and removed files show up
  /// in the library on their own, network shares are rescanned on a timer.
  /// With [scanOnStart] every source is scanned once now (changes made
  /// while the app was closed). Restarted when the sources change.
  Future<void> startWatching({bool scanOnStart = true}) async {
    try {
      await library.startWatching(scanOnStart: scanOnStart);
    } catch (e) {
      // The library still works; manual scans stay available.
      debugPrint('Library watcher not started: $e');
    }
  }

  Future<void> refreshSources() async {
    _sources = await library.sources();
    await _loadManaged();
    notifyListeners();
  }

  Future<void> _loadManaged() async {
    try {
      _managed = await sourceManager.list();
      _managedError = null;
    } catch (e) {
      _managedError = e;
    }
  }

  ManagedSource? managedSource(String id) =>
      _managed.where((s) => s.id == id).firstOrNull;

  /// Adds a source through [sourceManager]. Its password stays in the
  /// core's secret store only (LIB-018): the core reads the share itself.
  Future<ManagedSource> createSource(SourceDraft draft) async {
    final added = await sourceManager.create(draft);
    await refreshSources();
    // The add dialog scans the new source itself.
    unawaited(startWatching(scanOnStart: false));
    return added;
  }

  /// Saves edited settings; [SourceDraft.password] null keeps the password,
  /// empty removes it. The watcher follows the new auto-scan setting.
  Future<ManagedSource> updateSource(String id, SourceDraft draft) async {
    final saved = await sourceManager.update(id, draft);
    await refreshSources();
    unawaited(startWatching(scanOnStart: false));
    return saved;
  }

  /// Moves SMB passwords that app versions before LIB-018 kept in
  /// [secureStore] (`smb_password_<id>`) into the core's secret store, once,
  /// then deletes the old copy. A source that already has a password in
  /// the core keeps it; the old copy is only deleted. On the reduced
  /// manager (no core secret store) nothing is moved or deleted.
  Future<void> migrateLegacySmbPasswords() async {
    if (sourceManager.reduced || !sourceManager.canEdit) return;
    for (final m in _managed) {
      if (m.type != SourceType.smb) continue;
      final key = SecureStore.smbPassword(m.id);
      final String? old;
      try {
        old = await secureStore.read(key);
      } catch (e) {
        debugPrint('Old SMB password of source ${m.id} not readable: $e');
        continue;
      }
      if (old == null) continue;
      try {
        if (!m.hasPassword && old.isNotEmpty) {
          await sourceManager.update(m.id, draftOf(m, password: old));
        }
        await secureStore.write(key, null);
      } catch (e) {
        // Kept for the next start.
        debugPrint('SMB password of source ${m.id} not migrated: $e');
      }
    }
    await _loadManaged();
  }

  /// Removes a source: its settings and password, then its library items.
  Future<void> deleteSource(String id) async {
    await sourceManager.delete(id);
    await removeSource(id);
  }

  /// Checks a source; the new status is in [managedSources].
  Future<SourceStatusInfo> checkSource(String id) async {
    try {
      return await sourceManager.check(id);
    } finally {
      await refreshSources();
    }
  }

  Future<SpeedResult> speedTest(String id, {int maxSeconds = 10}) async {
    try {
      return await sourceManager.speedTest(id, maxSeconds: maxSeconds);
    } finally {
      await refreshSources();
    }
  }

  /// Whether a full scan includes [s]: not for browse-only sources nor for
  /// kinds the scan cannot read yet (WebDAV).
  bool _scannable(MediaSource s, {required bool explicit}) {
    final m = managedSource(s.id);
    if (m == null) return true;
    if (!m.type.indexable) return false;
    return explicit || m.scanIntoLibrary;
  }

  Future<MediaSource> addSource(MediaSource s) async {
    final added = await library.addSource(s);
    await refreshSources();
    // The add dialog scans the new source itself.
    unawaited(startWatching(scanOnStart: false));
    return added;
  }

  Future<void> removeSource(String id) async {
    await library.removeSource(id);
    await refreshSources();
    unawaited(startWatching(scanOnStart: false));
  }

  /// Scans one source or all of them; progress is published via [scanProgress].
  Future<void> scan({String? sourceId}) async {
    if (_scanRunning) return;
    final targets = sourceId == null
        ? _sources.where((s) => _scannable(s, explicit: false)).toList()
        : _sources
              .where((s) => s.id == sourceId && _scannable(s, explicit: true))
              .toList();
    if (sourceId != null && targets.isEmpty) return;
    _scanRunning = true;
    scanningSources.addAll(targets.map((s) => s.id));
    _scan = const ScanProgress(phase: ScanPhase.connecting);
    notifyListeners();
    final errors = <String>[];
    var found = 0;
    try {
      for (final s in targets) {
        String? sourceError;
        await for (final p in library.scan(sourceId: s.id)) {
          _scan = p;
          sourceError ??= p.error;
          if (p.isDone) {
            found += p.filesFound;
            if (p.error != null) errors.add('${s.name}: ${p.error}');
          }
          notifyListeners();
        }
        sessionScans[s.id] = (at: DateTime.now(), error: sourceError);
        scanningSources.remove(s.id);
      }
    } catch (e) {
      errors.add('$e');
    }
    _scanRunning = false;
    scanningSources.clear();
    await _loadManaged();
    _scan = ScanProgress(
      phase: ScanPhase.done,
      filesFound: found,
      processed: found,
      total: found,
      error: errors.isEmpty ? null : errors.join('\n'),
    );
    notifyListeners();
  }

  Future<String?> tmdbKey() => secureStore.read(SecureStore.tmdbKey);

  Future<void> setTmdbKey(String? key) async {
    final k = (key == null || key.trim().isEmpty) ? null : key.trim();
    await secureStore.write(SecureStore.tmdbKey, k);
    await library.setTmdbApiKey(k);
    notifyListeners();
  }

  /// Metadata languages offered in Settings (UI-016).
  static const metadataLanguages = ['en-US', 'ru-RU'];

  static String? _metadataLanguage(String? tag) =>
      metadataLanguages.contains(tag) ? tag : null;

  /// The language provider metadata is fetched in: the one chosen in
  /// Settings, else the UI language (UI-024).
  String get effectiveMetadataLanguage =>
      metadataLanguage.value ??
      metadataLanguageFor(locale.value ?? systemLocale());

  /// Stored metadata in another language is fetched again on the next scan.
  Future<void> _syncMetadataLanguage() =>
      library.setMetadataLanguage(effectiveMetadataLanguage);

  /// Sets the metadata language; null follows the UI language.
  Future<void> setMetadataLanguage(String? tag) async {
    metadataLanguage.value = _metadataLanguage(tag);
    await _syncMetadataLanguage();
    await preferences.write(
      Preferences.metadataLanguage,
      metadataLanguage.value,
    );
  }

  Future<void> setLocale(Locale? value) async {
    locale.value = value;
    await _syncMetadataLanguage();
    await preferences.write(
      Preferences.locale,
      value == null ? null : localeTag(value),
    );
  }

  Future<void> setSubtitleSettings(SubtitleSettings value) async {
    subtitleSettings.value = value;
    await preferences.write(
      Preferences.subtitleSettings,
      jsonEncode(value.toJson()),
    );
  }

  /// Subtitle track and delay remembered for a series (the show id).
  Future<SubtitleMemory?> subtitleMemory(String series) async {
    final raw = await preferences.read(Preferences.subtitleSeries(series));
    return raw == null ? null : SubtitleMemory.fromJson(_json(raw));
  }

  Future<void> saveSubtitleMemory(String series, SubtitleMemory memory) =>
      preferences.write(
        Preferences.subtitleSeries(series),
        jsonEncode(memory.toJson()),
      );

  static Object? _json(String? raw) {
    if (raw == null) return null;
    try {
      return jsonDecode(raw);
    } on FormatException {
      return null;
    }
  }

  /// Whether a takeover by another device pauses playback here (SYNC-003).
  Future<void> setPauseOnTakeover(bool value) async {
    handoff.pauseOnTakeover.value = value;
    await preferences.write(Preferences.takeoverPause, value ? null : 'false');
  }

  Future<void> setShowTwitch(bool value) async {
    showTwitch.value = value;
    await preferences.write(Preferences.showTwitch, value ? 'true' : null);
  }

  Future<void> setTwitchChatPauseAnimations(bool value) async {
    twitchChatPauseAnimations.value = value;
    await preferences.write(
      Preferences.twitchChatPauseAnimations,
      value ? 'true' : null,
    );
  }

  Future<void> setHideSpoilers(bool value) async {
    hideSpoilers.value = value;
    await preferences.write(Preferences.hideSpoilers, value ? 'true' : null);
  }

  Future<void> setContinuousPlayback(bool value) async {
    continuousPlayback.value = value;
    await preferences.write(
      Preferences.continuousPlayback,
      value ? null : 'false',
    );
  }

  Future<void> setSkipSeconds(int value) async {
    skipSeconds.value = value;
    await preferences.write(
      Preferences.skipSeconds,
      value == 10 ? null : '$value',
    );
  }

  Future<void> setSkipMode(SkipMode value) async {
    skipMode.value = value;
    await preferences.write(
      Preferences.skipMode,
      value == SkipMode.button ? null : value.name,
    );
  }

  Future<void> setTrickplayGenerate(bool value) async {
    _applyTrickplay(value);
    await preferences.write(
      Preferences.trickplayGenerate,
      value ? null : 'false',
    );
  }

  void _applyTrickplay(bool value) {
    trickplayGenerate.value = value;
    final lib = library;
    if (lib is LibraryTrickplay) {
      (lib as LibraryTrickplay).trickplayGenerate = value;
    }
  }

  Future<void> setDefaultEpisodeOrder(EpisodeOrder value) async {
    defaultEpisodeOrder.value = value;
    await preferences.write(
      Preferences.defaultEpisodeOrder,
      value == EpisodeOrder.tmdb ? null : value.name,
    );
  }

  /// The order to open [showId] in: null when the user picked one on the
  /// show's page (the library keeps it) or the default is TMDB.
  Future<EpisodeOrder?> initialEpisodeOrder(String showId) async {
    final order = defaultEpisodeOrder.value;
    if (order == EpisodeOrder.tmdb) return null;
    final set = await preferences.read(Preferences.showEpisodeOrderSet(showId));
    return set == 'true' ? null : order;
  }

  /// Notes that the user picked the episode order of [showId] on its page.
  Future<void> episodeOrderPicked(String showId) =>
      preferences.write(Preferences.showEpisodeOrderSet(showId), 'true');

  static ReplayGainMode _replayGain(String? name) => ReplayGainMode.values
      .firstWhere((m) => m.name == name, orElse: () => ReplayGainMode.auto);

  /// Music: gapless playback and ReplayGain (MUS-002), kept in Preferences.
  Future<void> setMusicAudio({
    bool? gapless,
    ReplayGainMode? replayGain,
  }) async {
    final AudioProfile p = music.audioProfile.copyWith(
      gapless: gapless,
      replayGain: replayGain,
    );
    await music.setAudioProfile(p);
    notifyListeners();
    await preferences.write(
      Preferences.musicGapless,
      p.gapless ? null : 'false',
    );
    await preferences.write(
      Preferences.musicReplayGain,
      p.replayGain == ReplayGainMode.auto ? null : p.replayGain.name,
    );
  }

  /// Changes the image cache limit and keeps it for the next start.
  Future<void> setImageCacheLimit(int maxBytes) async {
    final cache = imageCache;
    if (cache == null) return;
    await cache.setMaxBytes(maxBytes);
    await preferences.write(Preferences.imageCacheMaxBytes, '$maxBytes');
    notifyListeners();
  }

  /// Deletes the cached posters and backdrops on disk and the decoded
  /// images in memory.
  Future<void> clearImageCache() async {
    await imageCache?.clear();
    PaintingBinding.instance.imageCache
      ..clear()
      ..clearLiveImages();
    notifyListeners();
  }

  /// Ends the first-run setup (finished or skipped); it is not shown on
  /// start again. Settings can open it again.
  Future<void> completeOnboarding() async {
    await preferences.write(Preferences.onboardingDone, 'true');
    if (!_onboardingPending) return;
    _onboardingPending = false;
    notifyListeners();
  }

  /// The active profile's name and avatar. A profile the UI-004 setup kept
  /// in Preferences ([stored]) names the default core profile once, if it
  /// has no name yet, and leaves Preferences.
  Future<LocalProfile?> _loadProfile(LocalProfile? stored) async {
    final core = profiles;
    if (core == null) return stored;
    try {
      var active = await core.activeProfile();
      if (stored != null) {
        final first = (await core.profiles())
            .where((p) => p.syncId == 'default')
            .firstOrNull;
        if (first != null && first.name.trim().isEmpty) {
          final named = await core.updateProfile(
            first.id,
            first.settings.copyWith(
              name: stored.name,
              avatar: stored.avatarRef,
            ),
          );
          if (named.id == active.id) active = named;
        }
        await preferences.write(Preferences.localProfile, null);
      }
      _liveAllowed = !active.isKid && !active.isRestricted;
      return LocalProfile.fromCore(active) ?? stored;
    } catch (e) {
      debugPrint('Profiles not read from the library: $e');
      return stored;
    }
  }

  /// Saves the name and avatar of the active profile; null clears them.
  Future<void> setProfile(LocalProfile? value) async {
    profile.value = value;
    final core = profiles;
    if (core != null) {
      try {
        final active = await core.activeProfile();
        await core.updateProfile(
          active.id,
          active.settings.copyWith(
            name: value?.name ?? '',
            avatar: value?.avatarRef,
          ),
        );
        return;
      } catch (e) {
        debugPrint('Profile not saved in the library: $e');
      }
    }
    await preferences.write(
      Preferences.localProfile,
      value == null ? null : jsonEncode(value.toJson()),
    );
  }

  /// Another profile is active (or the active one changed): its name and
  /// avatar, and the sources it sees (a folder white list hides others).
  Future<void> _onProfileSwitched(LibraryProfile active) async {
    profile.value = LocalProfile.fromCore(active);
    try {
      await refreshSources();
    } catch (e) {
      debugPrint('Sources not reloaded for the profile: $e');
    }
    // MUS-003: the music queue and smart playlists are per profile.
    try {
      await musicQueue.switchProfile();
      await smartPlaylists.load();
    } catch (e) {
      debugPrint('Music queue not switched for the profile: $e');
    }
    // TWI-024: the chat filters of the new profile.
    try {
      await twitchChatSettings.load();
    } catch (e) {
      debugPrint('Chat settings not reloaded for the profile: $e');
    }
    // TWI-009: a kid profile stops "X is live" alerts at once.
    try {
      await refreshLiveProfile();
    } catch (e) {
      debugPrint('Live notifications not refreshed for the profile: $e');
    }
  }

  Future<void> setAppMode(AppMode value) async {
    appMode.value = value;
    await preferences.write(Preferences.appMode, value.name);
  }

  /// Forgets the recent search queries (UI-011).
  Future<void> clearSearchHistory() =>
      preferences.write(profileControl.searchHistoryKey, null);

  @override
  void dispose() {
    unawaited(_syncSub?.cancel());
    handoff.dispose();
    unawaited(library.stopWatching());
    locale.dispose();
    showTwitch.dispose();
    liveNotifications.dispose();
    twitch.dispose();
    twitchChatPauseAnimations.dispose();
    twitchChatSettings.dispose();
    musicQueue.dispose();
    smartPlaylists.dispose();
    hideSpoilers.dispose();
    continuousPlayback.dispose();
    skipSeconds.dispose();
    skipMode.dispose();
    trickplayGenerate.dispose();
    defaultEpisodeOrder.dispose();
    metadataLanguage.dispose();
    subtitleSettings.dispose();
    profile.dispose();
    profileControl
      ..removeListener(notifyListeners)
      ..removeListener(_updateTwitchShown)
      ..dispose();
    twitchShown.dispose();
    appMode.dispose();
    music.dispose();
    super.dispose();
  }
}

class Services extends InheritedNotifier<AppServices> {
  const Services({
    super.key,
    required AppServices services,
    required super.child,
  }) : super(notifier: services);

  static AppServices of(BuildContext context) =>
      context.dependOnInheritedWidgetOfExactType<Services>()!.notifier!;

  /// Read without subscribing to rebuilds.
  static AppServices read(BuildContext context) =>
      context.getInheritedWidgetOfExactType<Services>()!.notifier!;
}
