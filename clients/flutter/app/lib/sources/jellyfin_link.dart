// SPDX-License-Identifier: MPL-2.0
//
// Jellyfin and Emby servers (NET-014, NET-015) for the app: signing in
// (password, or Quick Connect on Jellyfin) and what only a media server has — how an item plays on this
// device, playback reports, favorites, "watched" and Continue Watching at
// the server. A Jellyfin or Emby source is otherwise a source like the others: the
// core scans its libraries into the library and streams its files through
// the proxy.

import 'package:material_ui/material_ui.dart';

import '../integration/jellyfin.dart';
import 'core_source_manager.dart';
import 'source_manager.dart';

/// A Jellyfin server before signing in.
class JellyfinServerInfo {
  const JellyfinServerInfo({
    required this.name,
    required this.version,
    this.localAddress,
    this.quickConnect = false,
  });

  final String name;
  final String version;

  /// The LAN address the server reports: offered as a second address.
  final String? localAddress;
  final bool quickConnect;
}

/// A Quick Connect sign-in waiting for approval.
class QuickConnectTicket {
  const QuickConnectTicket({
    required this.handle,
    required this.code,
    required this.serverName,
  });

  final BigInt handle;

  /// Six digits to enter in a signed-in Jellyfin app.
  final String code;
  final String serverName;
}

/// How an item plays: [url] is null for DirectPlay (the item's own stream
/// URI through the proxy), an HLS URL for DirectStream and Transcode.
class JellyfinPlay {
  const JellyfinPlay({required this.method, this.url, this.reasons = const []});

  /// `direct_play`, `direct_stream` or `transcode`.
  final String method;

  /// Contains the server token: do not log or show it.
  final String? url;
  final List<String> reasons;
}

/// The Jellyfin and Emby side of the app. Item calls do nothing for items
/// of other sources, so callers need not know where an item comes from.
/// [server] is [SourceType.jellyfin] or [SourceType.emby]; Quick Connect is
/// Jellyfin's only.
abstract class JellyfinLink {
  Future<JellyfinServerInfo> probe(
    String address, {
    SourceType server = SourceType.jellyfin,
  });

  /// Signs in and adds the server as a source; the password is not kept.
  Future<ManagedSource> signIn({
    SourceType server = SourceType.jellyfin,
    required List<String> addresses,
    required String username,
    required String password,
    required String deviceName,
    String? name,
  });

  Future<QuickConnectTicket> quickConnectStart({
    required List<String> addresses,
    required String deviceName,
  });

  /// Whether the code of [handle] has been approved.
  Future<bool> quickConnectPoll(BigInt handle);
  Future<ManagedSource> quickConnectFinish(BigInt handle, {String? name});
  void quickConnectCancel(BigInt handle);

  /// Null if [itemId] is not on a Jellyfin or Emby server.
  Future<JellyfinPlay?> playback(
    String itemId, {
    int startMs = 0,
    bool forceTranscode = false,
  });
  Future<void> report(String itemId, int positionMs, {bool paused = false});
  Future<void> stopped(String itemId, int positionMs);
  Future<void> setFavorite(String itemId, bool favorite);
  Future<void> setWatched(String itemId, bool watched);
  Future<void> removeFromResume(String itemId);

  /// Brings the server's resume positions, watched marks and favorites of
  /// source [sourceId] into the library; returns how many items changed.
  Future<int> importUserData(String sourceId);
}

/// [JellyfinLink] on the Rust core (`CoreJellyfin`).
class CoreJellyfinLink implements JellyfinLink {
  CoreJellyfinLink(this._core);

  final CoreJellyfin _core;

  /// Opens the link over the library [library] with the secret store whose
  /// fallback file is in [dataDir].
  static Future<CoreJellyfinLink> open({
    required CoreLibrary library,
    required String dataDir,
  }) async => CoreJellyfinLink(
    await CoreJellyfin.open(library_: library, dataDir: dataDir),
  );

  static Future<T> _guard<T>(Future<T> Function() call) async {
    try {
      return await call();
    } on SourcesError catch (e) {
      throw SourceException.fromCore(e.code, e.message);
    }
  }

  @override
  Future<JellyfinServerInfo> probe(
    String address, {
    SourceType server = SourceType.jellyfin,
  }) => _guard(() async {
    final s = await _core.probe(address: address, server: server.coreKind);
    return JellyfinServerInfo(
      name: s.name,
      version: s.version,
      localAddress: s.localAddress,
      quickConnect: s.quickConnect,
    );
  });

  @override
  Future<ManagedSource> signIn({
    SourceType server = SourceType.jellyfin,
    required List<String> addresses,
    required String username,
    required String password,
    required String deviceName,
    String? name,
  }) => _guard(
    () async => CoreSourceManager.sourceOf(
      await _core.signIn(
        server: server.coreKind,
        addresses: addresses,
        username: username,
        password: password,
        deviceName: deviceName,
        name: name,
        libraries: const [],
      ),
    ),
  );

  @override
  Future<QuickConnectTicket> quickConnectStart({
    required List<String> addresses,
    required String deviceName,
  }) => _guard(() async {
    final t = await _core.quickConnectStart(
      addresses: addresses,
      deviceName: deviceName,
    );
    return QuickConnectTicket(
      handle: t.handle,
      code: t.code,
      serverName: t.serverName,
    );
  });

  @override
  Future<bool> quickConnectPoll(BigInt handle) =>
      _guard(() => _core.quickConnectPoll(handle: handle));

  @override
  Future<ManagedSource> quickConnectFinish(BigInt handle, {String? name}) =>
      _guard(
        () async => CoreSourceManager.sourceOf(
          await _core.quickConnectFinish(
            handle: handle,
            name: name,
            libraries: const [],
          ),
        ),
      );

  @override
  void quickConnectCancel(BigInt handle) =>
      _core.quickConnectCancel(handle: handle);

  @override
  Future<JellyfinPlay?> playback(
    String itemId, {
    int startMs = 0,
    bool forceTranscode = false,
  }) => _guard(() async {
    final p = await _core.playback(
      itemId: itemId,
      startMs: startMs,
      forceTranscode: forceTranscode,
    );
    if (p == null) return null;
    return JellyfinPlay(method: p.method, url: p.url, reasons: p.reasons);
  });

  @override
  Future<void> report(String itemId, int positionMs, {bool paused = false}) =>
      _guard(
        () => _core.report(
          itemId: itemId,
          positionMs: positionMs,
          paused: paused,
        ),
      );

  @override
  Future<void> stopped(String itemId, int positionMs) =>
      _guard(() => _core.stopped(itemId: itemId, positionMs: positionMs));

  @override
  Future<void> setFavorite(String itemId, bool favorite) =>
      _guard(() => _core.setFavorite(itemId: itemId, favorite: favorite));

  @override
  Future<void> setWatched(String itemId, bool watched) =>
      _guard(() => _core.setWatched(itemId: itemId, watched: watched));

  @override
  Future<void> removeFromResume(String itemId) =>
      _guard(() => _core.removeFromResume(itemId: itemId));

  @override
  Future<int> importUserData(String sourceId) => _guard(() async {
    final id = int.tryParse(sourceId);
    if (id == null) return 0;
    final r = await _core.importUserData(sourceId: id);
    return r.progress + r.watched + r.favorites;
  });
}

/// Reports one player session to the server (PLAY-005 progress saves are
/// the heartbeats): the first report starts it, the next ones update it,
/// [close] or a report for another item (Up Next) stops it. Errors are
/// logged, never shown: the server is told again on the next report.
class JellyfinReporter {
  JellyfinReporter(this._link);

  final JellyfinLink? _link;
  String? _item;
  int _position = 0;

  void report(String itemId, int positionMs) {
    final link = _link;
    if (link == null) return;
    final previous = _item;
    if (previous != null && previous != itemId) {
      _quietly(link.stopped(previous, _position));
    }
    _item = itemId;
    _position = positionMs;
    _quietly(link.report(itemId, positionMs));
  }

  /// The player closed.
  void close() {
    final link = _link;
    final item = _item;
    if (link == null || item == null) return;
    _item = null;
    _quietly(link.stopped(item, _position));
  }

  static void _quietly(Future<void> call) => call.catchError((Object e) {
    debugPrint('Jellyfin report failed: $e');
  });
}
