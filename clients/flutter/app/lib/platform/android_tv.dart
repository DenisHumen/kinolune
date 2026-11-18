// SPDX-License-Identifier: MPL-2.0
//
// Android TV integration (PLAY-AND-009, ADR-NNNN), channel `app.kinolune/tv`
// (TvBridge on the Android side):
// * the Continue row of Home (library items and Twitch VODs) and Up Next go
//   to the system Watch Next row of the launcher (Android TV "Play Next",
//   Google TV "Continue watching"), following every change of the rows;
// * links from the launcher come back: a Watch Next card resumes its item
//   (`kinolune://play/<id>`), `kinolune://item/<id>` opens its page, voice
//   search opens Search with the query. The same channel brings every
//   `kinolune://` link on phones too (UI-022, app_links.dart).
// The row lives outside the app's locks: with App Lock or a PIN on the
// active profile it is left empty.

import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:ui' as ui;

import 'package:flutter/services.dart';
import 'package:material_ui/material_ui.dart';
import 'package:path_provider/path_provider.dart';

import '../integration/library.dart';
import '../screens/home_feed.dart';
import '../services/app_services.dart';
import '../widgets/artwork.dart';
import 'app_links.dart';

const _tvChannel = MethodChannel('app.kinolune/tv');
const _twitchVodPrefix = 'twitch:vod:'; // l10n-ignore

/// A card of the system Watch Next row.
@immutable
class WatchNextCard {
  const WatchNextCard({
    required this.key,
    required this.kind,
    required this.next,
    required this.title,
    required this.link,
    required this.engagement,
    required this.artSeed,
    this.episodeTitle,
    this.season,
    this.episode,
    this.positionMs = 0,
    this.durationMs = 0,
    this.artUrl,
  });

  /// The library item or Twitch VOD id.
  final String key;

  /// `movie`, `episode` or `clip` (a Twitch VOD).
  final String kind;

  /// Up Next rather than Continue.
  final bool next;
  final String title;
  final String? episodeTitle;
  final int? season;
  final int? episode;
  final int positionMs;
  final int durationMs;
  final DateTime engagement;
  final String link;

  /// The art as the app has it (http(s), a file, demo art or none) and the
  /// seed of the drawn placeholder.
  final String? artUrl;
  final String artSeed;

  Map<String, Object?> toMap({String? art}) => {
    'key': key,
    'kind': kind,
    'next': next ? 'next' : 'continue',
    'title': title,
    'episodeTitle': episodeTitle,
    'season': season,
    'episode': episode,
    'positionMs': positionMs,
    'durationMs': durationMs,
    'engagementMs': engagement.millisecondsSinceEpoch,
    'art': art,
    'link': link,
  };
}

/// The Watch Next cards of the Continue row and Up Next: everything the
/// Continue row shows, then the Up Next episodes not already in it.
List<WatchNextCard> watchNextCards(
  List<ContinueEntry> continueRow,
  List<UpNextEntry> upNext,
) {
  final out = <WatchNextCard>[];
  final keys = <String>{};
  for (final e in continueRow) {
    final p = e.progress;
    final item = e.item;
    if (e.kind == ContinueKind.twitchVod) {
      final id = e.id.startsWith(_twitchVodPrefix)
          ? e.id.substring(_twitchVodPrefix.length)
          : e.id;
      out.add(
        WatchNextCard(
          key: e.id,
          kind: 'clip',
          next: false,
          title: e.title,
          episodeTitle: e.subtitle,
          positionMs: p.positionMs,
          durationMs: p.durationMs,
          engagement: p.updatedAt,
          link: AppLink.twitchVod(id, p.positionMs),
          artUrl: e.imageUrl,
          artSeed: e.id,
        ),
      );
    } else if (item != null) {
      final episode = e.kind == ContinueKind.episode;
      out.add(
        WatchNextCard(
          key: item.id,
          kind: episode ? 'episode' : 'movie',
          next: false,
          title: e.title,
          episodeTitle: episode ? item.title : null,
          season: episode ? item.season : null,
          episode: episode ? item.episode : null,
          positionMs: p.positionMs,
          durationMs: p.durationMs,
          engagement: p.updatedAt,
          link: AppLink.play(item.id),
          artUrl: e.imageUrl ?? item.posterUrl,
          artSeed: e.id,
        ),
      );
    } else {
      continue;
    }
    keys.add(out.last.key);
  }
  for (final u in upNext) {
    final ep = u.episode;
    if (!keys.add(ep.id)) continue;
    out.add(
      WatchNextCard(
        key: ep.id,
        kind: 'episode',
        next: true,
        title: u.show.title,
        episodeTitle: ep.title,
        season: ep.season,
        episode: ep.episode,
        engagement: u.after,
        link: AppLink.play(ep.id),
        artUrl: ep.backdropUrl ?? u.show.backdropUrl ?? u.show.posterUrl,
        artSeed: ep.id,
      ),
    );
  }
  return out;
}

/// Card art the launcher can load: http(s) URLs as they are; local files,
/// the demo library's drawn art and missing art become PNG files in
/// `cache/tv_art`, which the Android side serves to the launcher.
class TvArtFiles {
  TvArtFiles(this.dir);

  final Directory dir;
  static const _width = 640, _height = 360;

  static Future<TvArtFiles> open() async => TvArtFiles(
    Directory('${(await getApplicationCacheDirectory()).path}/tv_art'),
  );

  Future<String?> resolve(WatchNextCard card) async {
    final url = card.artUrl;
    try {
      if (url != null &&
          (url.startsWith('https://') || url.startsWith('http://'))) {
        return url;
      }
      await dir.create(recursive: true);
      if (url != null && url.startsWith(kDemoArtScheme)) {
        return await _png(
          'g${_fnv(card.artSeed)}',
          () => renderArt(card.artSeed),
        );
      }
      final local = url == null
          ? null
          : (url.startsWith('file:') ? Uri.parse(url).toFilePath() : url);
      if (local != null && local.startsWith('/')) {
        final ext = local.split('.').last.toLowerCase();
        if (const {'png', 'jpg', 'jpeg', 'webp'}.contains(ext)) {
          final f = File('${dir.path}/f${_fnv(local)}.$ext');
          final src = File(local);
          if (await src.exists() &&
              (!await f.exists() ||
                  (await src.lastModified()).isAfter(await f.lastModified()))) {
            await src.copy(f.path);
          }
          if (await f.exists()) return f.path;
        }
      }
      return await _png('placeholder', () async {
        final b = await rootBundle.load('assets/placeholders/backdrop.png');
        return b.buffer.asUint8List(b.offsetInBytes, b.lengthInBytes);
      });
    } catch (e) {
      debugPrint('TV art for ${card.key} unavailable: $e');
      return null;
    }
  }

  Future<String> _png(String name, Future<Uint8List> Function() bytes) async {
    final f = File('${dir.path}/$name.png');
    if (!await f.exists()) {
      final tmp = File('${f.path}.tmp');
      await tmp.writeAsBytes(await bytes(), flush: true);
      await tmp.rename(f.path);
    }
    return f.path;
  }

  /// The drawn art of the demo library (as on Home) in the card's size.
  static Future<Uint8List> renderArt(String seed) async {
    final rec = ui.PictureRecorder();
    paintGeneratedArt(
      Canvas(rec),
      const Size(_width + .0, _height + .0),
      seed,
      ArtStyle.backdrop,
    );
    final img = await rec.endRecording().toImage(_width, _height);
    try {
      final data = await img.toByteData(format: ui.ImageByteFormat.png);
      return data!.buffer.asUint8List();
    } finally {
      img.dispose();
    }
  }

  static String _fnv(String s) {
    var h = 0x811c9dc5;
    for (final c in utf8.encode(s)) {
      h ^= c;
      h = (h * 0x01000193) & 0xffffffff;
    }
    return h.toRadixString(16).padLeft(8, '0');
  }
}

/// Follows the active profile's Continue and Up Next rows and publishes them
/// to the Watch Next row; hands launcher links to [onLink]. One per shell
/// (a new profile gets a new shell, and so a fresh row).
class AndroidTvLauncher {
  AndroidTvLauncher({
    required this.services,
    required this.onLink,
    this.channel = _tvChannel,
    Future<TvArtFiles> Function()? art,
    this.debounce = const Duration(milliseconds: 1500),
  }) : _art = art ?? TvArtFiles.open;

  final AppServices services;
  final Future<void> Function(AppLink link) onLink;
  final Duration debounce;
  final MethodChannel channel;
  final Future<TvArtFiles> Function() _art;

  final _subs = <StreamSubscription<Object?>>[];
  List<ContinueEntry> _continue = const [];
  List<UpNextEntry> _upNext = const [];
  Timer? _timer;
  String? _lastPayload;
  String? _lastLink;
  DateTime _lastLinkAt = DateTime.fromMillisecondsSinceEpoch(0);
  TvArtFiles? _files;
  bool _disposed = false;
  bool _publishing = false;
  bool _again = false;

  Future<void> start() async {
    channel.setMethodCallHandler(_onCall);
    try {
      final link = await channel.invokeMethod<String>('takeLink');
      if (link != null) _deliver(link);
      final supported =
          await channel.invokeMethod<bool>('watchNextSupported') ?? false;
      if (!supported || _disposed) return;
    } on MissingPluginException {
      return;
    } on PlatformException catch (e) {
      debugPrint('Android TV integration unavailable: $e');
      return;
    }
    final feed = HomeFeed(services.library);
    _subs
      ..add(
        feed
            .continueRow(
              extra: [
                services.twitch.continueEntries(enabled: services.twitchShown),
              ],
            )
            .listen((r) {
              _continue = r;
              _schedule();
            }, onError: (Object e) => debugPrint('Watch Next: $e')),
      )
      ..add(
        feed.upNext().listen((r) {
          _upNext = r;
          _schedule();
        }, onError: (Object e) => debugPrint('Watch Next: $e')),
      );
    services.profileControl.addListener(_schedule);
  }

  void dispose() {
    _disposed = true;
    _timer?.cancel();
    for (final s in _subs) {
      unawaited(s.cancel());
    }
    services.profileControl.removeListener(_schedule);
    channel.setMethodCallHandler(null);
  }

  Future<Object?> _onCall(MethodCall call) async {
    if (call.method == 'link' && call.arguments is String) {
      _deliver(call.arguments as String);
      return true;
    }
    throw MissingPluginException();
  }

  void _deliver(String link) {
    // The same link can come twice: from `takeLink` and as `link`.
    final now = DateTime.now();
    if (link == _lastLink && now.difference(_lastLinkAt).inSeconds < 3) return;
    _lastLink = link;
    _lastLinkAt = now;
    final l = AppLink.parse(link);
    if (l != null) unawaited(onLink(l));
  }

  /// The row stays empty while the app is behind a lock: the launcher shows
  /// it to anyone.
  bool get _hidden {
    final p = services.profileControl;
    return p.appLock || p.locked || (p.active?.hasPin ?? false);
  }

  void _schedule() {
    if (_disposed) return;
    _timer?.cancel();
    _timer = Timer(debounce, () => unawaited(_publishSerially()));
  }

  /// One publish at a time; a change during one publishes again after it.
  Future<void> _publishSerially() async {
    if (_publishing) {
      _again = true;
      return;
    }
    _publishing = true;
    try {
      do {
        _again = false;
        await _publish();
      } while (_again && !_disposed);
    } finally {
      _publishing = false;
    }
  }

  Future<void> _publish() async {
    if (_disposed) return;
    final cards = _hidden
        ? const <WatchNextCard>[]
        : watchNextCards(_continue, _upNext);
    final files = _files ??= await _art();
    final payload = <Map<String, Object?>>[
      for (final c in cards) c.toMap(art: await files.resolve(c)),
    ];
    final json = jsonEncode(payload);
    if (json == _lastPayload || _disposed) return;
    _lastPayload = json;
    try {
      await channel.invokeMethod<void>('publishWatchNext', payload);
    } on PlatformException catch (e) {
      debugPrint('Watch Next publish failed: $e');
    } on MissingPluginException {
      // not on Android
    }
  }
}
