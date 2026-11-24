// SPDX-License-Identifier: MPL-2.0
//
// PLAY-012 (ADR-NNNN): the native half of casting on the channel
// `app.kinolune/cast`. Android: the Google Cast sender SDK with the Default
// Media Receiver (android/app/src/cast, a no-op in F-Droid builds). macOS:
// AVRouteDetector, AVRoutePickerView and an AVPlayer for AirPlay
// (macos/Runner/MainFlutterWindow.swift). Elsewhere the channel is missing
// and casting is simply not offered.

import 'dart:async';

import 'package:flutter/services.dart';

import 'cast_plan.dart';
import 'cast_progress.dart';

/// A device to cast to. For AirPlay there is one entry, «AirPlay», while
/// the system sees any AirPlay route; the system picker chooses the device.
class CastDevice {
  const CastDevice({
    required this.id,
    required this.name,
    required this.tech,
    this.model,
  });

  factory CastDevice.fromMap(Map<Object?, Object?> m) => CastDevice(
    id: '${m['id']}',
    name: '${m['name'] ?? ''}',
    tech: m['tech'] == 'airPlay' ? CastTech.airPlay : CastTech.googleCast,
    model: m['model'] as String?,
  );

  final String id;
  final String name;
  final CastTech tech;

  /// Model name (`Chromecast Ultra`), for the receiver profile.
  final String? model;

  @override
  bool operator ==(Object other) =>
      other is CastDevice && other.id == id && other.tech == tech;

  @override
  int get hashCode => Object.hash(id, tech);
}

/// A connected Google Cast device.
class CastConnection {
  const CastConnection({this.address, this.model});

  /// The device's IPv4 address: the relay listens on the interface facing
  /// it.
  final String? address;
  final String? model;
}

/// What a receiver loads.
class CastLoad {
  const CastLoad({
    required this.url,
    required this.contentType,
    required this.title,
    this.subtitle,
    this.imageUrl,
    this.positionMs = 0,
    this.durationMs = 0,
  });

  /// Contains a token: not logged.
  final String url;
  final String contentType;
  final String title;
  final String? subtitle;

  /// Artwork; only an `http(s)` URL the receiver can fetch is sent.
  final String? imageUrl;
  final int positionMs;
  final int durationMs;

  Map<String, Object?> toMap() => {
    'url': url,
    'contentType': contentType,
    'title': title,
    'subtitle': subtitle,
    'imageUrl': imageUrl != null && reachableFromLan(imageUrl!)
        ? imageUrl
        : null,
    'positionMs': positionMs,
    'durationMs': durationMs,
  };
}

/// The native side. [MethodChannelCastPlatform] in the app, a fake in
/// tests.
abstract class CastPlatform {
  /// Whether this device can cast at all (Google Play services present,
  /// AVKit). False where the channel is missing.
  Future<bool> available();

  /// Devices found while discovery runs; the platform pushes changes.
  Stream<List<CastDevice>> get devices;

  /// The session ended on the platform side (stopped from the
  /// notification or on the TV, the device left the network).
  Stream<void> get sessionEnded;

  Future<void> startDiscovery();
  Future<void> stopDiscovery();

  /// Google Cast: selects [device] and waits for the session; null if it
  /// did not start.
  Future<CastConnection?> connect(CastDevice device);

  /// Google Cast: loads [load] on the connected receiver.
  Future<bool> load(CastLoad load);

  /// AirPlay: prepares an AVPlayer with [load] and opens the system route
  /// picker at [anchor] (logical pixels in the window). True once the
  /// player plays on an AirPlay device; false when the picker was closed.
  Future<bool> airPlay(CastLoad load, Rect anchor);

  Future<CastStatus?> status();
  Future<void> play();
  Future<void> pause();
  Future<void> seek(Duration position);

  /// Stops the receiver and ends the session.
  Future<void> stop();
}

/// [CastPlatform] over `app.kinolune/cast`.
class MethodChannelCastPlatform implements CastPlatform {
  MethodChannelCastPlatform({MethodChannel? channel})
    : _channel = channel ?? const MethodChannel('app.kinolune/cast') {
    _channel.setMethodCallHandler(_onCall);
  }

  final MethodChannel _channel;
  final _devices = StreamController<List<CastDevice>>.broadcast();
  final _ended = StreamController<void>.broadcast();

  Future<Object?> _onCall(MethodCall call) async {
    switch (call.method) {
      case 'devices':
        final list = call.arguments as List<Object?>? ?? const [];
        _devices.add([
          for (final d in list)
            if (d is Map) CastDevice.fromMap(d),
        ]);
      case 'sessionEnded':
        _ended.add(null);
    }
    return null;
  }

  Future<T?> _call<T>(String method, [Object? args]) async {
    try {
      return await _channel.invokeMethod<T>(method, args);
    } on MissingPluginException {
      return null;
    }
  }

  @override
  Future<bool> available() async => await _call<bool>('available') ?? false;

  @override
  Stream<List<CastDevice>> get devices => _devices.stream;

  @override
  Stream<void> get sessionEnded => _ended.stream;

  @override
  Future<void> startDiscovery() => _call<void>('startDiscovery');

  @override
  Future<void> stopDiscovery() => _call<void>('stopDiscovery');

  @override
  Future<CastConnection?> connect(CastDevice device) async {
    final r = await _call<Map<Object?, Object?>>('connect', {'id': device.id});
    if (r == null || r['ok'] != true) return null;
    return CastConnection(
      address: r['address'] as String?,
      model: r['model'] as String?,
    );
  }

  @override
  Future<bool> load(CastLoad load) async =>
      await _call<bool>('load', load.toMap()) ?? false;

  @override
  Future<bool> airPlay(CastLoad load, Rect anchor) async =>
      await _call<bool>('airPlay', {
        ...load.toMap(),
        'x': anchor.left,
        'y': anchor.top,
        'width': anchor.width,
        'height': anchor.height,
      }) ??
      false;

  @override
  Future<CastStatus?> status() async {
    final m = await _call<Map<Object?, Object?>>('status');
    return m == null ? null : CastStatus.fromMap(m);
  }

  @override
  Future<void> play() => _call<void>('play');

  @override
  Future<void> pause() => _call<void>('pause');

  @override
  Future<void> seek(Duration position) =>
      _call<void>('seek', {'positionMs': position.inMilliseconds});

  @override
  Future<void> stop() => _call<void>('stop');
}
