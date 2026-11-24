// SPDX-License-Identifier: MPL-2.0
//
// PLAY-012 (ADR-NNNN): one cast session at a time for the whole app — the
// devices found, connecting, choosing what the receiver gets (cast_plan),
// the LAN relay (cast_relay) and the progress sync (cast_progress). The
// cast buttons and the mini-controller listen to it.

import 'dart:async';
import 'dart:io';
import 'dart:ui' show Rect;

import 'package:flutter/foundation.dart';

import '../flavor.dart';
import '../integration/library.dart';
import 'cast_plan.dart';
import 'cast_platform.dart';
import 'cast_progress.dart';
import 'cast_relay.dart';

/// What to cast: one library item and where its progress goes.
class CastRequest {
  const CastRequest({
    required this.itemId,
    required this.title,
    required this.playbackUri,
    required this.fileName,
    required this.onProgress,
    this.subtitle,
    this.artworkUrl,
    this.tech,
    this.durationMs = 0,
    this.startMs = 0,
    this.serverItem = false,
    this.serverStream,
    this.onEnded,
  });

  final String itemId;
  final String title;
  final String? subtitle;
  final String? artworkUrl;

  /// What the app's player would open ([LibraryItem.playbackUri]).
  final String playbackUri;

  /// The file's display path or name (its extension tells the container
  /// when the file has not been probed).
  final String fileName;

  /// The probe of the file; null if not probed.
  final TechInfo? tech;
  final int durationMs;
  final int startMs;

  /// The item is on Jellyfin, Emby or Plex: [serverStream] can transcode.
  final bool serverItem;

  /// The server's HLS URL from [startMs]; null if the server cannot.
  final Future<String?> Function(int startMs)? serverStream;

  /// The progress path of the app's player (library, handoff, server).
  final void Function(int positionMs, int durationMs) onProgress;

  /// The cast session of this item ended.
  final VoidCallback? onEnded;
}

/// How [CastController.cast] went.
enum CastOutcome {
  /// The receiver plays the item.
  started,

  /// The device did not connect, or the receiver did not load the item.
  failed,

  /// The AirPlay picker was closed without a device.
  cancelled,

  /// No Wi-Fi address to serve the file from.
  noNetwork,

  /// The receiver cannot play the file; see [CastResult.unsupported].
  unsupported,
}

class CastResult {
  const CastResult(this.outcome, [this.unsupported]);
  final CastOutcome outcome;
  final CastUnsupported? unsupported;
}

/// A cast session running now.
class CastSession {
  CastSession._(this.device, this.request);

  final CastDevice device;
  final CastRequest request;

  /// The receiver's last status.
  CastStatus? get status => _status;
  CastStatus? _status;
  CastRelay? _relay;
  CastProgressSync? _sync;

  Duration get position =>
      _status?.position ?? Duration(milliseconds: request.startMs);
  Duration get duration {
    final d = _status?.duration ?? Duration.zero;
    return d > Duration.zero ? d : Duration(milliseconds: request.durationMs);
  }

  bool get playing =>
      _status?.state == CastPlayerState.playing ||
      _status?.state == CastPlayerState.buffering;
}

class CastController extends ChangeNotifier {
  CastController({
    CastPlatform? platform,
    List<CastTech>? techs,
    Future<InternetAddress?> Function(InternetAddress? receiver)? lanAddress,
    this.pollInterval = const Duration(seconds: 1),
  }) : platform = platform ?? MethodChannelCastPlatform(),
       techs =
           techs ??
           castTechsFor(
             defaultTargetPlatform,
             googleCastBuild: AppFlavor.googleCast,
           ),
       _lanAddress = lanAddress ?? _systemLanAddress {
    _subs = [
      this.platform.devices.listen(_onDevices),
      this.platform.sessionEnded.listen((_) => _session?._sync?.stop()),
    ];
  }

  static CastController? _instance;

  /// The app's controller, made on first use (no service wiring: the
  /// native side is per process like the controller).
  static CastController get instance => _instance ??= CastController();

  @visibleForTesting
  static set instance(CastController? controller) => _instance = controller;

  final CastPlatform platform;

  /// What this build offers on this platform; empty: no cast UI at all.
  final List<CastTech> techs;
  /// The address the relay listens on, facing the receiver.
  final Future<InternetAddress?> Function(InternetAddress? receiver)
  _lanAddress;

  static Future<InternetAddress?> _systemLanAddress(
    InternetAddress? receiver,
  ) async => pickLanAddress(
    await NetworkInterface.list(type: InternetAddressType.IPv4),
    receiver: receiver,
  );
  final Duration pollInterval;
  late final List<StreamSubscription<Object?>> _subs;

  bool get offered => techs.isNotEmpty;

  /// Devices found now (of [techs]).
  List<CastDevice> get devices => _devices;
  List<CastDevice> _devices = const [];

  /// A cast button has something to offer: devices, or a session to stop.
  bool get hasDevices => _devices.isNotEmpty || _session != null;

  CastSession? get session => _session;
  CastSession? _session;

  /// A device is being connected.
  CastDevice? get connecting => _connecting;
  CastDevice? _connecting;

  int _watchers = 0;
  bool? _available;

  void _onDevices(List<CastDevice> list) {
    final mine = [
      for (final d in list)
        if (techs.contains(d.tech)) d,
    ];
    if (listEquals(mine, _devices)) return;
    _devices = mine;
    notifyListeners();
  }

  /// A cast button is on screen: discovery runs while any is.
  void watch() {
    if (!offered) return;
    if (_watchers++ > 0) return;
    unawaited(_startDiscovery());
  }

  void unwatch() {
    if (!offered || _watchers == 0) return;
    if (--_watchers > 0) return;
    unawaited(platform.stopDiscovery().catchError((Object e) {
      debugPrint('Cast discovery not stopped: $e');
    }));
  }

  Future<void> _startDiscovery() async {
    try {
      _available ??= await platform.available();
      if (_available! && _watchers > 0) await platform.startDiscovery();
    } catch (e) {
      debugPrint('Cast discovery not started: $e');
    }
  }

  /// Plays [request] on [device]. [anchor] places the AirPlay picker.
  /// With [force], a file the receiver is not expected to play is sent
  /// anyway («Всё равно попробовать»).
  Future<CastResult> cast(
    CastRequest request,
    CastDevice device, {
    Rect? anchor,
    bool force = false,
  }) async {
    if (_session != null) await stop();
    _connecting = device;
    notifyListeners();
    try {
      return await _cast(request, device, anchor, force);
    } catch (e) {
      debugPrint('Cast of ${request.itemId} failed: $e');
      return const CastResult(CastOutcome.failed);
    } finally {
      _connecting = null;
      notifyListeners();
    }
  }

  Future<CastResult> _cast(
    CastRequest request,
    CastDevice device,
    Rect? anchor,
    bool force,
  ) async {
    final airPlay = device.tech == CastTech.airPlay;
    CastConnection? connection;
    if (!airPlay) {
      connection = await platform.connect(device);
      if (connection == null) return const CastResult(CastOutcome.failed);
    }
    Future<CastResult> giveUp(CastResult r) async {
      if (!airPlay) await platform.stop();
      return r;
    }

    final receiver = airPlay
        ? ReceiverProfile.airPlay
        : ReceiverProfile.forCastModel(connection?.model ?? device.model);
    var plan = planCast(
      playbackUri: request.playbackUri,
      fileName: request.fileName,
      tech: request.tech,
      receiver: receiver,
      serverItem: request.serverItem && request.serverStream != null,
    );
    if (plan case CastUnsupported u) {
      if (!force) return giveUp(CastResult(CastOutcome.unsupported, u));
      // A Chromecast told «MKV» refuses at once; told MP4 it tries the
      // file (newer models play many MKVs).
      plan = CastDirect(
        !airPlay && u.mime == 'video/x-matroska' ? 'video/mp4' : u.mime,
      );
    }

    String url;
    String mime;
    CastRelay? relay;
    switch (plan) {
      case CastServerStream():
        final stream = await request.serverStream!(request.startMs);
        if (stream == null || !reachableFromLan(stream)) {
          return giveUp(const CastResult(CastOutcome.failed));
        }
        (url, mime) = (stream, hlsMime);
      case CastDirect(mime: final m):
        mime = m;
        final body = CastBody.forUri(request.playbackUri);
        if (body == null) {
          // Already a URL of another host (a WebDAV or server link).
          if (!reachableFromLan(request.playbackUri)) {
            return giveUp(const CastResult(CastOutcome.failed));
          }
          url = request.playbackUri;
        } else {
          final address = await _lanAddress(
            InternetAddress.tryParse(connection?.address ?? ''),
          );
          if (address == null) {
            await body.close();
            return giveUp(const CastResult(CastOutcome.noNetwork));
          }
          relay = await CastRelay.start(
            address: address,
            body: body,
            mime: mime,
            name: _baseName(request.fileName),
          );
          url = relay.url;
        }
      case CastUnsupported():
        throw StateError('handled above');
    }

    final load = CastLoad(
      url: url,
      contentType: mime,
      title: request.title,
      subtitle: request.subtitle,
      imageUrl: request.artworkUrl,
      positionMs: request.startMs,
      durationMs: request.durationMs,
    );
    final ok = airPlay
        ? await platform.airPlay(load, anchor ?? Rect.zero)
        : await platform.load(load);
    if (!ok) {
      await relay?.close();
      return giveUp(
        CastResult(airPlay ? CastOutcome.cancelled : CastOutcome.failed),
      );
    }

    final session = CastSession._(device, request).._relay = relay;
    session._sync = CastProgressSync(
      poll: platform.status,
      onProgress: request.onProgress,
      knownDurationMs: request.durationMs,
      interval: pollInterval,
      onStatus: (s) {
        if (s == session._status) return;
        session._status = s;
        if (identical(_session, session)) notifyListeners();
      },
      onEnded: () => _ended(session),
    )..start();
    _session = session;
    return const CastResult(CastOutcome.started);
  }

  void _ended(CastSession session) {
    unawaited(session._relay?.close());
    session.request.onEnded?.call();
    if (identical(_session, session)) {
      _session = null;
      notifyListeners();
    }
  }

  Future<void> togglePause() async {
    final s = _session;
    if (s == null) return;
    await (s.playing ? platform.pause() : platform.play());
    await s._sync?.tick();
  }

  Future<void> seek(Duration position) async {
    final s = _session;
    if (s == null) return;
    final d = s.duration;
    await platform.seek(
      position < Duration.zero
          ? Duration.zero
          : (d > Duration.zero && position > d ? d : position),
    );
    await s._sync?.tick();
  }

  /// Stops casting: the last position is already saved (every poll).
  Future<void> stop() async {
    final s = _session;
    if (s == null) return;
    await s._sync?.tick();
    try {
      await platform.stop();
    } catch (e) {
      debugPrint('Cast stop failed: $e');
    }
    s._sync?.stop();
  }

  @override
  void dispose() {
    for (final s in _subs) {
      unawaited(s.cancel());
    }
    super.dispose();
  }
}

String _baseName(String path) {
  final i = path.replaceAll('\\', '/').lastIndexOf('/');
  final name = i < 0 ? path : path.substring(i + 1);
  return name.isEmpty ? 'media' : name;
}
