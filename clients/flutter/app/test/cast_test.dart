// SPDX-License-Identifier: MPL-2.0
// PLAY-012: Google Cast and AirPlay — which casting a build offers, what a
// receiver gets (the file through the LAN relay, the server's HLS, or a
// reason), the relay itself and the progress path while casting.
//
// Plain `test()`s only: the widget binding fakes HttpClient, and the relay
// tests talk to real sockets on 127.0.0.1. Widgets: cast_ui_test.dart.

import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:ui' show Rect;

import 'package:flutter/foundation.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/cast/cast_controller.dart';
import 'package:kinolune/cast/cast_plan.dart';
import 'package:kinolune/cast/cast_platform.dart';
import 'package:kinolune/cast/cast_progress.dart';
import 'package:kinolune/cast/cast_relay.dart';
import 'package:kinolune/integration/library.dart';

TechInfo _tech({
  String container = 'mov,mp4,m4a,3gp,3g2,mj2',
  String video = 'h264',
  int height = 1080,
  int bitDepth = 8,
  List<HdrFormat> hdr = const [],
  List<String> audio = const ['aac'],
}) => TechInfo(
  container: container,
  videoCodec: video,
  width: height * 16 ~/ 9,
  height: height,
  bitDepth: bitDepth,
  hdr: hdr,
  audioFormats: audio,
);

CastPlan _plan(
  String file, {
  TechInfo? tech,
  ReceiverProfile receiver = ReceiverProfile.chromecast,
  bool server = false,
  String? uri,
}) => planCast(
  playbackUri: uri ?? file,
  fileName: file,
  tech: tech,
  receiver: receiver,
  serverItem: server,
);

/// An address of an interface with its subnet.
class _Addr implements InterfaceAddress {
  _Addr(String address, this.prefixLength) : _a = InternetAddress(address);
  final InternetAddress _a;

  @override
  final int prefixLength;
  @override
  InternetAddress? get broadcast => null;
  @override
  String get address => _a.address;
  @override
  Uint8List get rawAddress => _a.rawAddress;
  @override
  InternetAddressType get type => _a.type;
  @override
  bool get isLoopback => _a.isLoopback;
  @override
  bool get isLinkLocal => _a.isLinkLocal;

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

class _Iface implements NetworkInterface {
  _Iface(this.name, List<String> addresses, {int prefix = 24})
    : addresses = [for (final a in addresses) _Addr(a, prefix)];

  @override
  final String name;
  @override
  final List<InterfaceAddress> addresses;
  @override
  int get index => 0;
}

/// The native side, scripted.
class _FakePlatform implements CastPlatform {
  final devicesCtl = StreamController<List<CastDevice>>.broadcast();
  final endedCtl = StreamController<void>.broadcast();
  final calls = <String>[];
  CastConnection? connection = const CastConnection(
    address: '127.0.0.1',
    model: 'Chromecast',
  );
  bool loadOk = true;
  bool airPlayOk = true;
  CastLoad? loaded;
  Rect? anchor;
  final statuses = <CastStatus?>[];
  CastStatus? lastStatus;

  @override
  Future<bool> available() async => true;
  @override
  Stream<List<CastDevice>> get devices => devicesCtl.stream;
  @override
  Stream<void> get sessionEnded => endedCtl.stream;
  @override
  Future<void> startDiscovery() async => calls.add('startDiscovery');
  @override
  Future<void> stopDiscovery() async => calls.add('stopDiscovery');
  @override
  Future<CastConnection?> connect(CastDevice device) async {
    calls.add('connect ${device.id}');
    return connection;
  }

  @override
  Future<bool> load(CastLoad load) async {
    calls.add('load');
    loaded = load;
    return loadOk;
  }

  @override
  Future<bool> airPlay(CastLoad load, Rect anchor) async {
    calls.add('airPlay');
    loaded = load;
    this.anchor = anchor;
    return airPlayOk;
  }

  @override
  Future<CastStatus?> status() async {
    if (statuses.isNotEmpty) lastStatus = statuses.removeAt(0);
    return lastStatus;
  }

  @override
  Future<void> play() async => calls.add('play');
  @override
  Future<void> pause() async => calls.add('pause');
  @override
  Future<void> seek(Duration position) async =>
      calls.add('seek ${position.inMilliseconds}');
  @override
  Future<void> stop() async => calls.add('stop');
}

const _chromecast = CastDevice(
  id: 'cc1',
  name: 'Living room',
  tech: CastTech.googleCast,
  model: 'Chromecast',
);
const _airPlay = CastDevice(id: 'airplay', name: '', tech: CastTech.airPlay);

Future<(int, List<int>, HttpHeaders)> _get(
  String url, {
  String method = 'GET',
  String? range,
}) async {
  final client = HttpClient();
  try {
    final req = await client.openUrl(method, Uri.parse(url));
    if (range != null) req.headers.set(HttpHeaders.rangeHeader, range);
    final res = await req.close();
    final body = <int>[];
    await res.forEach(body.addAll);
    return (res.statusCode, body, res.headers);
  } finally {
    client.close(force: true);
  }
}

void main() {
  group('route per platform and build', () {
    test('Android: Google Cast only with the Cast SDK (not F-Droid)', () {
      expect(castTechsFor(TargetPlatform.android, googleCastBuild: true), [
        CastTech.googleCast,
      ]);
      expect(
        castTechsFor(TargetPlatform.android, googleCastBuild: false),
        isEmpty,
      );
    });

    test('Apple: AirPlay always, Google Cast on iOS with the SDK', () {
      expect(castTechsFor(TargetPlatform.macOS, googleCastBuild: true), [
        CastTech.airPlay,
      ]);
      expect(castTechsFor(TargetPlatform.iOS, googleCastBuild: false), [
        CastTech.airPlay,
      ]);
      expect(castTechsFor(TargetPlatform.iOS, googleCastBuild: true), [
        CastTech.airPlay,
        CastTech.googleCast,
      ]);
    });

    test('desktops without a sender and TVs offer nothing', () {
      for (final p in [TargetPlatform.linux, TargetPlatform.windows]) {
        expect(castTechsFor(p, googleCastBuild: true), isEmpty);
      }
      expect(
        castTechsFor(TargetPlatform.android, googleCastBuild: true, tv: true),
        isEmpty,
      );
    });

    test('receiver profile by Cast model name', () {
      expect(
        ReceiverProfile.forCastModel('Chromecast Ultra'),
        same(ReceiverProfile.chromecastUltra),
      );
      expect(
        ReceiverProfile.forCastModel('Chromecast with Google TV'),
        same(ReceiverProfile.chromecastUltra),
      );
      expect(
        ReceiverProfile.forCastModel('Chromecast'),
        same(ReceiverProfile.chromecast),
      );
      expect(
        ReceiverProfile.forCastModel(null),
        same(ReceiverProfile.chromecast),
      );
    });
  });

  group('what the receiver gets', () {
    test('container from the probe, else from the extension', () {
      expect(containerOf('mov,mp4,m4a,3gp,3g2,mj2', '/m/a.mp4'), 'mp4');
      expect(containerOf('mov,mp4,m4a,3gp,3g2,mj2', '/m/a.mov'), 'mov');
      expect(containerOf('matroska,webm', '/m/a.mkv'), 'mkv');
      expect(containerOf('matroska,webm', '/m/a.webm'), 'webm');
      expect(containerOf('avi', '/m/a.avi'), 'avi');
      expect(containerOf(null, 'smb://nas/m/a.M4V'), 'mp4');
      expect(containerOf(null, '/m/noext'), isNull);
      expect(mimeOf('mp4'), 'video/mp4');
      expect(mimeOf('webm'), 'video/webm');
      expect(mimeOf('hls'), hlsMime);
    });

    test('an H.264/AAC MP4 is sent as it is', () {
      final p = _plan('/m/a.mp4', tech: _tech());
      expect(p, isA<CastDirect>());
      expect((p as CastDirect).mime, 'video/mp4');
    });

    test('an MP4 or WebM not probed yet goes by its name', () {
      expect(_plan('/m/a.mp4'), isA<CastDirect>());
      expect((_plan('/m/a.webm') as CastDirect).mime, 'video/webm');
      final u = _plan('/m/a.bin') as CastUnsupported;
      expect(u.problem, CastProblem.unknown);
    });

    test('MKV: the server transcodes, else the reason', () {
      final tech = _tech(container: 'matroska,webm');
      expect(
        _plan('/m/a.mkv', tech: tech, server: true),
        isA<CastServerStream>(),
      );
      final u = _plan('/m/a.mkv', tech: tech) as CastUnsupported;
      expect(u.problem, CastProblem.container);
      expect(u.detail, 'MKV');
    });

    test('HEVC: Chromecast no, Ultra and AirPlay yes', () {
      final hevc = _tech(video: 'hevc', height: 2160, bitDepth: 10);
      final u = _plan('/m/a.mp4', tech: hevc) as CastUnsupported;
      expect(u.problem, CastProblem.videoCodec);
      expect(u.detail, 'HEVC');
      expect(
        _plan(
          '/m/a.mp4',
          tech: hevc,
          receiver: ReceiverProfile.chromecastUltra,
        ),
        isA<CastDirect>(),
      );
      expect(
        _plan('/m/a.mp4', tech: hevc, receiver: ReceiverProfile.airPlay),
        isA<CastDirect>(),
      );
    });

    test('4K or 10-bit H.264 is too much for a Chromecast', () {
      final big =
          _plan('/m/a.mp4', tech: _tech(height: 2160)) as CastUnsupported;
      expect(big.problem, CastProblem.picture);
      expect(big.detail, '2160p');
      final hi10 =
          _plan('/m/a.mp4', tech: _tech(bitDepth: 10)) as CastUnsupported;
      expect(hi10.problem, CastProblem.picture);
    });

    test('HDR: shown when the receiver knows one of the formats', () {
      final dvOnly = _tech(
        video: 'hevc',
        bitDepth: 10,
        hdr: const [HdrFormat.dolbyVision],
      );
      expect(
        (_plan(
          '/m/a.mp4',
          tech: dvOnly,
          receiver: ReceiverProfile.chromecastUltra,
        ) as CastUnsupported).problem,
        CastProblem.hdr,
      );
      final dv81 = _tech(
        video: 'hevc',
        bitDepth: 10,
        hdr: const [HdrFormat.dolbyVision, HdrFormat.hdr10],
      );
      expect(
        _plan(
          '/m/a.mp4',
          tech: dv81,
          receiver: ReceiverProfile.chromecastUltra,
        ),
        isA<CastDirect>(),
      );
      expect(
        _plan('/m/a.mp4', tech: dvOnly, receiver: ReceiverProfile.airPlay),
        isA<CastDirect>(),
      );
    });

    test('the first audio track decides; Atmos marks are skipped', () {
      final dts = _plan(
        '/m/a.mp4',
        tech: _tech(audio: ['dts_hd_ma', 'aac']),
      ) as CastUnsupported;
      expect(dts.problem, CastProblem.audioCodec);
      expect(dts.detail, 'DTS-HD');
      // E-AC-3: AirPlay passes it through, the Default Media Receiver not.
      final eac3 = _tech(audio: ['eac3', 'atmos']);
      expect(_plan('/m/a.mp4', tech: eac3), isA<CastUnsupported>());
      expect(
        _plan('/m/a.mp4', tech: eac3, receiver: ReceiverProfile.airPlay),
        isA<CastDirect>(),
      );
    });

    test('AirPlay: MP4 and MOV only', () {
      final u = _plan(
        '/m/a.webm',
        tech: _tech(container: 'matroska,webm', video: 'vp9'),
        receiver: ReceiverProfile.airPlay,
      ) as CastUnsupported;
      expect(u.problem, CastProblem.container);
      expect(
        (_plan(
          '/m/a.mov',
          tech: _tech(),
          receiver: ReceiverProfile.airPlay,
        ) as CastDirect).mime,
        'video/quicktime',
      );
    });

    test('a movie of several parts: the server, or only here', () {
      expect(
        (_plan(
          '/m/a.mp4',
          uri: 'edl://x;y',
          tech: _tech(),
        ) as CastUnsupported).problem,
        CastProblem.parts,
      );
      expect(
        _plan('/m/a.mp4', uri: 'edl://x;y', server: true),
        isA<CastServerStream>(),
      );
    });

    test('server and artwork URLs must be reachable from the LAN', () {
      expect(reachableFromLan('http://192.168.1.10:8096/x.m3u8'), isTrue);
      expect(reachableFromLan('https://media.example.org/x.m3u8'), isTrue);
      expect(reachableFromLan('http://127.0.0.1:4000/t/x'), isFalse);
      expect(reachableFromLan('http://localhost/x'), isFalse);
      expect(reachableFromLan('http://[::1]:80/x'), isFalse);
      expect(reachableFromLan('http://169.254.1.2/x'), isFalse);
      expect(reachableFromLan('file:///m/a.mp4'), isFalse);
      final load = const CastLoad(
        url: 'http://192.168.1.5:1/t/a.mp4',
        contentType: 'video/mp4',
        title: 'A',
        imageUrl: 'file:///poster.jpg',
      ).toMap();
      expect(load['imageUrl'], isNull);
    });
  });

  group('LAN relay', () {
    test('Range header parsing', () {
      expect(parseRange(null, 100), isNull);
      expect(parseRange('bytes=0-9', 100), (0, 9));
      expect(parseRange('bytes=90-', 100), (90, 99));
      expect(parseRange('bytes=-10', 100), (90, 99));
      expect(parseRange('bytes=50-500', 100), (50, 99));
      expect(() => parseRange('bytes=100-', 100), throwsRangeError);
      expect(() => parseRange('bytes=5-1', 100), throwsRangeError);
      expect(() => parseRange('items=0-1', 100), throwsRangeError);
      expect(() => parseRange('bytes=-', 100), throwsRangeError);
    });

    test('tokens are 128 random bits', () {
      final a = newRelayToken(), b = newRelayToken();
      expect(a, matches(RegExp(r'^[0-9a-f]{32}$')));
      expect(a, isNot(b));
    });

    test('listens on the Wi-Fi address facing the receiver', () {
      final ifs = [
        _Iface('lo', ['127.0.0.1']),
        _Iface('tun0', ['10.8.0.2']),
        _Iface('rmnet0', ['100.70.1.2']),
        _Iface('wlan0', ['192.168.1.23']),
        _Iface('eth0', ['10.0.5.4']),
        _Iface('wlan1', ['172.20.0.7'], prefix: 12),
      ];
      expect(
        pickLanAddress(ifs, receiver: InternetAddress('192.168.1.50'))?.address,
        '192.168.1.23',
      );
      expect(
        pickLanAddress(ifs, receiver: InternetAddress('10.0.5.9'))?.address,
        '10.0.5.4',
      );
      // Same /12 subnet beats a longer common prefix elsewhere.
      expect(
        pickLanAddress(ifs, receiver: InternetAddress('172.31.9.9'))?.address,
        '172.20.0.7',
      );
      // No receiver address (AirPlay): Wi-Fi/Ethernet, never the VPN.
      expect(pickLanAddress(ifs)?.address, isNot('10.8.0.2'));
      expect(
        pickLanAddress([
          _Iface('lo', ['127.0.0.1']),
        ]),
        isNull,
      );
      expect(
        pickLanAddress([
          _Iface('wlan0', ['169.254.3.4', '8.8.8.8']),
        ]),
        isNull,
      );
    });

    test('serves one file under its token with ranges', () async {
      final dir = await Directory.systemTemp.createTemp('cast_relay');
      addTearDown(() => dir.delete(recursive: true));
      final file = File('${dir.path}/movie.mp4');
      final bytes = Uint8List.fromList(List.generate(1000, (i) => i % 251));
      await file.writeAsBytes(bytes);
      final relay = await CastRelay.start(
        address: InternetAddress.loopbackIPv4,
        body: CastBody.forUri(file.path)!,
        mime: 'video/mp4',
        name: 'movie.mp4',
      );
      addTearDown(relay.close);
      expect(relay.url, startsWith('http://127.0.0.1:'));
      expect(relay.url, endsWith('/movie.mp4'));

      final (status, body, headers) = await _get(relay.url);
      expect(status, HttpStatus.ok);
      expect(body, bytes);
      expect(headers.contentType?.mimeType, 'video/mp4');
      expect(headers.value(HttpHeaders.acceptRangesHeader), 'bytes');

      final part = await _get(relay.url, range: 'bytes=100-199');
      expect(part.$1, HttpStatus.partialContent);
      expect(part.$2, bytes.sublist(100, 200));
      expect(
        part.$3.value(HttpHeaders.contentRangeHeader),
        'bytes 100-199/1000',
      );

      final head = await _get(relay.url, method: 'HEAD');
      expect(head.$1, HttpStatus.ok);
      expect(head.$3.contentLength, 1000);
      expect(head.$2, isEmpty);

      expect(
        (await _get(relay.url, range: 'bytes=5000-')).$1,
        HttpStatus.requestedRangeNotSatisfiable,
      );
      expect((await _get(relay.url, method: 'POST')).$1, 405);
      // Another path, another token: nothing.
      final base = Uri.parse(relay.url);
      expect(
        (await _get(base.replace(path: '/movie.mp4').toString())).$1,
        HttpStatus.notFound,
      );
      expect(
        (await _get(
          base.replace(path: '/${newRelayToken()}/movie.mp4').toString(),
        )).$1,
        HttpStatus.notFound,
      );

      await relay.close();
      await expectLater(_get(relay.url), throwsA(isA<SocketException>()));
    });

    test(
      'a file of a network source is read through the loopback proxy',
      () async {
        final content = utf8.encode('0123456789abcdef');
        final ranges = <String?>[];
        // The loopback proxy (pa-proxy) as the relay sees it.
        final proxy = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
        addTearDown(() => proxy.close(force: true));
        proxy.listen((req) async {
          final res = req.response;
          final range = req.headers.value(HttpHeaders.rangeHeader);
          ranges.add(range);
          final r = parseRange(range, content.length);
          final (a, b) = r ?? (0, content.length - 1);
          res.statusCode = r == null ? 200 : 206;
          res.contentLength = b - a + 1;
          if (req.method != 'HEAD') res.add(content.sublist(a, b + 1));
          await res.close();
        });
        final proxyUrl = 'http://127.0.0.1:${proxy.port}/tok/src/movie.mp4';
        final body = CastBody.forUri(proxyUrl);
        expect(body, isA<ProxyCastBody>());
        final relay = await CastRelay.start(
          address: InternetAddress.loopbackIPv4,
          body: body!,
          mime: 'video/mp4',
        );
        addTearDown(relay.close);
        final (status, got, _) = await _get(relay.url, range: 'bytes=4-7');
        expect(status, HttpStatus.partialContent);
        expect(utf8.decode(got), '4567');
        expect(ranges.last, 'bytes=4-7');
        // Remote URLs are not relayed: the receiver fetches them itself.
        expect(CastBody.forUri('http://192.168.1.4/x.mp4'), isNull);
        expect(CastBody.forUri('file:///m/a.mp4'), isA<FileCastBody>());
      },
    );
  });

  group('progress while casting', () {
    CastStatus s(
      CastPlayerState state, [
      int pos = 0,
      int dur = 0,
      CastIdleReason idle = CastIdleReason.none,
    ]) => CastStatus(
      state: state,
      position: Duration(milliseconds: pos),
      duration: Duration(milliseconds: dur),
      idleReason: idle,
    );

    test('position and duration map to a progress save', () {
      expect(
        castProgressOf(
          s(CastPlayerState.playing, 60000, 7200000),
          knownDurationMs: 0,
          started: true,
        ),
        (60000, 7200000),
      );
      // A transcode without a duration: the library's.
      expect(
        castProgressOf(
          s(CastPlayerState.paused, 60000),
          knownDurationMs: 5400000,
          started: true,
        ),
        (60000, 5400000),
      );
      // Not loaded yet: the resume point is kept.
      expect(
        castProgressOf(
          s(CastPlayerState.buffering, 0, 7200000),
          knownDurationMs: 0,
          started: false,
        ),
        isNull,
      );
      expect(
        castProgressOf(
          s(CastPlayerState.loading),
          knownDurationMs: 100,
          started: false,
        ),
        isNull,
      );
      // Finished: the end, so it counts as watched.
      expect(
        castProgressOf(
          s(CastPlayerState.idle, 0, 0, CastIdleReason.finished),
          knownDurationMs: 7200000,
          started: true,
        ),
        (7200000, 7200000),
      );
      // Stopped on the TV or an error: the last save stays.
      expect(
        castProgressOf(
          s(CastPlayerState.idle, 0, 0, CastIdleReason.cancelled),
          knownDurationMs: 7200000,
          started: true,
        ),
        isNull,
      );
    });

    test('status from the platform map', () {
      final st = CastStatus.fromMap(const {
        'state': 'paused',
        'positionMs': 1500,
        'durationMs': 3000.0,
        'idleReason': 'none',
      });
      expect(st, s(CastPlayerState.paused, 1500, 3000));
      expect(
        CastStatus.fromMap(const {'state': 'weird', 'positionMs': -5}),
        s(CastPlayerState.idle),
      );
    });

    test('the sync saves changes and ends once', () async {
      final saved = <(int, int)>[];
      var ended = 0;
      final statuses = <CastStatus?>[
        s(CastPlayerState.idle),
        s(CastPlayerState.buffering, 0, 1000),
        s(CastPlayerState.playing, 100, 1000),
        s(CastPlayerState.playing, 100, 1000),
        null,
        s(CastPlayerState.paused, 400, 1000),
        s(CastPlayerState.idle, 0, 0, CastIdleReason.finished),
        s(CastPlayerState.playing, 500, 1000),
      ];
      final sync = CastProgressSync(
        poll: () async => statuses.removeAt(0),
        onProgress: (p, d) => saved.add((p, d)),
        onEnded: () => ended++,
        knownDurationMs: 1000,
      );
      for (var i = 0; i < 8; i++) {
        await sync.tick();
      }
      expect(saved, [(100, 1000), (400, 1000), (1000, 1000)]);
      expect(ended, 1);
      sync.stop();
      expect(ended, 1);
    });
  });

  group('controller', () {
    late Directory dir;
    late File file;

    setUp(() async {
      dir = await Directory.systemTemp.createTemp('cast_ctl');
      file = File('${dir.path}/movie.mp4');
      await file.writeAsBytes(List.filled(64, 7));
    });
    tearDown(() => dir.delete(recursive: true));

    CastController controller(_FakePlatform p, {List<CastTech>? techs}) {
      final c = CastController(
        platform: p,
        techs: techs ?? const [CastTech.googleCast, CastTech.airPlay],
        lanAddress: (_) async => InternetAddress.loopbackIPv4,
        pollInterval: const Duration(hours: 1),
      );
      addTearDown(c.dispose);
      return c;
    }

    CastRequest request({
      TechInfo? tech,
      bool server = false,
      String? serverUrl,
      void Function(int, int)? onProgress,
      VoidCallback? onEnded,
      String? fileName,
    }) => CastRequest(
      itemId: 'm1',
      title: 'Movie',
      playbackUri: file.path,
      fileName: fileName ?? file.path,
      tech: tech,
      durationMs: 1000,
      startMs: 250,
      serverItem: server,
      serverStream: server ? (_) async => serverUrl : null,
      onProgress: onProgress ?? (_, _) {},
      onEnded: onEnded,
    );

    test('discovery runs while a cast button is on screen', () async {
      final p = _FakePlatform();
      final c = controller(p);
      c.watch();
      c.watch();
      await pumpEventQueue();
      c.unwatch();
      expect(p.calls, ['startDiscovery']);
      c.unwatch();
      await pumpEventQueue();
      expect(p.calls, ['startDiscovery', 'stopDiscovery']);
      expect(c.hasDevices, isFalse);
      p.devicesCtl.add([_chromecast]);
      await pumpEventQueue();
      expect(c.devices, [_chromecast]);
      expect(c.hasDevices, isTrue);
      // Devices of a technology this build does not offer are left out.
      final onlyAirPlay = controller(p, techs: const [CastTech.airPlay]);
      p.devicesCtl.add([_chromecast]);
      await pumpEventQueue();
      expect(onlyAirPlay.devices, isEmpty);
    });

    test('a playable file goes through the relay from the position', () async {
      final p = _FakePlatform();
      final c = controller(p);
      final saved = <(int, int)>[];
      var ended = 0;
      final r = await c.cast(
        request(
          tech: _tech(),
          onProgress: (a, b) => saved.add((a, b)),
          onEnded: () => ended++,
        ),
        _chromecast,
      );
      expect(r.outcome, CastOutcome.started);
      expect(p.calls, ['connect cc1', 'load']);
      final load = p.loaded!;
      expect(load.contentType, 'video/mp4');
      expect(load.positionMs, 250);
      expect(load.url, startsWith('http://127.0.0.1:'));
      final (status, body, _) = await _get(load.url);
      expect(status, 200);
      expect(body, List.filled(64, 7));
      expect(c.session, isNotNull);

      p.statuses.addAll([
        const CastStatus(
          state: CastPlayerState.playing,
          position: Duration(milliseconds: 300),
          duration: Duration(milliseconds: 1000),
        ),
        const CastStatus(
          state: CastPlayerState.idle,
          idleReason: CastIdleReason.finished,
        ),
      ]);
      // Not known to play yet: the toggle plays.
      await c.togglePause();
      expect(p.calls.last, 'play');
      expect(saved, [(300, 1000)]);
      await c.seek(const Duration(seconds: 5));
      expect(p.calls.last, 'seek 1000');
      expect(saved.last, (1000, 1000));
      expect(ended, 1);
      expect(c.session, isNull);
      // The relay stopped with the session.
      await pumpEventQueue();
      await expectLater(_get(load.url), throwsA(isA<SocketException>()));
    });

    test('a file the receiver cannot play: asked, then tried as MP4', () async {
      final p = _FakePlatform();
      final c = controller(p);
      final mkv = request(
        tech: _tech(container: 'matroska,webm'),
        fileName: '/m/movie.mkv',
      );
      final r = await c.cast(mkv, _chromecast);
      expect(r.outcome, CastOutcome.unsupported);
      expect(r.unsupported!.problem, CastProblem.container);
      expect(p.calls, ['connect cc1', 'stop']);
      expect(c.session, isNull);

      final forced = await c.cast(mkv, _chromecast, force: true);
      expect(forced.outcome, CastOutcome.started);
      expect(p.loaded!.contentType, 'video/mp4');
      await c.stop();
      expect(c.session, isNull);
    });

    test(
      'a server item the receiver cannot play gets the server HLS',
      () async {
        final p = _FakePlatform();
        final c = controller(p);
        final r = await c.cast(
          request(
            tech: _tech(video: 'hevc'),
            server: true,
            serverUrl: 'http://192.168.1.10:8096/videos/1/master.m3u8?t=x',
          ),
          _chromecast,
        );
        expect(r.outcome, CastOutcome.started);
        expect(p.loaded!.contentType, hlsMime);
        expect(p.loaded!.url, contains('master.m3u8'));
        await c.stop();

        // A server reached through this device's loopback cannot be cast.
        final p2 = _FakePlatform();
        final c2 = controller(p2);
        final r2 = await c2.cast(
          request(
            tech: _tech(video: 'hevc'),
            server: true,
            serverUrl: 'http://127.0.0.1:8096/master.m3u8',
          ),
          _chromecast,
        );
        expect(r2.outcome, CastOutcome.failed);
        expect(p2.calls.last, 'stop');
      },
    );

    test('no connection, no load, no Wi-Fi', () async {
      final p = _FakePlatform()..connection = null;
      expect(
        (await controller(p).cast(request(), _chromecast)).outcome,
        CastOutcome.failed,
      );
      final p2 = _FakePlatform()..loadOk = false;
      expect(
        (await controller(p2).cast(request(), _chromecast)).outcome,
        CastOutcome.failed,
      );
      final p3 = _FakePlatform();
      final c3 = CastController(
        platform: p3,
        techs: const [CastTech.googleCast],
        lanAddress: (_) async => null,
      );
      addTearDown(c3.dispose);
      expect(
        (await c3.cast(request(), _chromecast)).outcome,
        CastOutcome.noNetwork,
      );
    });

    test('AirPlay: the picker at the button; closing it cancels', () async {
      final p = _FakePlatform();
      final c = controller(p);
      const anchor = Rect.fromLTWH(10, 20, 40, 40);
      final r = await c.cast(request(tech: _tech()), _airPlay, anchor: anchor);
      expect(r.outcome, CastOutcome.started);
      expect(p.calls, ['airPlay']);
      expect(p.anchor, anchor);
      await c.stop();

      final p2 = _FakePlatform()..airPlayOk = false;
      final r2 = await controller(p2)
          .cast(request(tech: _tech()), _airPlay, anchor: anchor);
      expect(r2.outcome, CastOutcome.cancelled);
      // An MKV is refused before the picker opens.
      final r3 = await controller(_FakePlatform()).cast(
        request(
          tech: _tech(container: 'matroska,webm'),
          fileName: '/m/movie.mkv',
        ),
        _airPlay,
      );
      expect(r3.outcome, CastOutcome.unsupported);
    });

    test('the session ends when the device ends it', () async {
      final p = _FakePlatform();
      final c = controller(p);
      var ended = 0;
      await c.cast(request(onEnded: () => ended++), _chromecast);
      expect(c.session, isNotNull);
      p.endedCtl.add(null);
      await pumpEventQueue();
      expect(c.session, isNull);
      expect(ended, 1);
    });
  });
}
