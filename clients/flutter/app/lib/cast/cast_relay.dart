// SPDX-License-Identifier: MPL-2.0
//
// PLAY-012 (ADR-0153): the one-item HTTP server a Chromecast or an Apple TV
// fetches a file from. The loopback proxy (LIB-018, ADR-0038) listens on
// 127.0.0.1 only and serves every file of every source to whoever has its
// per-process token, so it is never shown to the LAN. While casting, this
// relay listens on the Wi-Fi address, serves exactly one file under a fresh
// token, and stops with the cast session.

import 'dart:async';
import 'dart:io';
import 'dart:math';

import 'package:flutter/foundation.dart';

/// The bytes of the one file a [CastRelay] serves.
abstract class CastBody {
  /// Size in bytes.
  Future<int> length();

  /// Bytes [start]..[end] (inclusive).
  Future<Stream<List<int>>> read(int start, int end);

  Future<void> close() async {}

  /// A body for what the player would open: a local path or `file://` URI,
  /// or an `http://127.0.0.1` URL of the loopback proxy. Null for anything
  /// else (a remote URL is handed to the receiver as it is).
  static CastBody? forUri(String uri) {
    final parsed = Uri.tryParse(uri);
    if (parsed == null || !parsed.hasScheme || parsed.scheme.length == 1) {
      // A path (`/media/x.mp4`, `C:\x.mp4`).
      return FileCastBody(File(uri));
    }
    if (parsed.scheme == 'file') return FileCastBody(File.fromUri(parsed));
    if (parsed.scheme == 'http' &&
        (parsed.host == '127.0.0.1' || parsed.host == 'localhost')) {
      return ProxyCastBody(parsed);
    }
    return null;
  }
}

/// A local file.
class FileCastBody extends CastBody {
  FileCastBody(this.file);
  final File file;

  @override
  Future<int> length() => file.length();

  @override
  Future<Stream<List<int>>> read(int start, int end) async =>
      file.openRead(start, end + 1);
}

/// A file of a network source, read through the loopback proxy with a
/// `Range` request (the proxy reads the source at that offset).
class ProxyCastBody extends CastBody {
  ProxyCastBody(this.url, {HttpClient? client})
    : _client = client ?? HttpClient();
  final Uri url;
  final HttpClient _client;
  int? _length;

  @override
  Future<int> length() async {
    if (_length case final n?) return n;
    final req = await _client.headUrl(url);
    final res = await req.close();
    await res.drain<void>();
    if (res.statusCode != HttpStatus.ok || res.contentLength < 0) {
      throw HttpException('proxy HEAD ${res.statusCode}');
    }
    return _length = res.contentLength;
  }

  @override
  Future<Stream<List<int>>> read(int start, int end) async {
    final req = await _client.getUrl(url);
    req.headers.set(HttpHeaders.rangeHeader, 'bytes=$start-$end');
    final res = await req.close();
    if (res.statusCode != HttpStatus.partialContent &&
        res.statusCode != HttpStatus.ok) {
      await res.drain<void>();
      throw HttpException('proxy GET ${res.statusCode}');
    }
    return res;
  }

  @override
  Future<void> close() async => _client.close(force: true);
}

/// A byte range of an HTTP `Range` header within [length] bytes:
/// `bytes=a-b`, `bytes=a-`, `bytes=-n`. Null without a header (the whole
/// file); a range outside the file throws [RangeError].
(int, int)? parseRange(String? header, int length) {
  if (header == null || header.trim().isEmpty) return null;
  final m = RegExp(r'^\s*bytes=(\d*)-(\d*)\s*$').firstMatch(header);
  if (m == null) throw RangeError('bad range $header');
  final a = m.group(1)!, b = m.group(2)!;
  if (a.isEmpty && b.isEmpty) throw RangeError('bad range $header');
  int start, end;
  if (a.isEmpty) {
    final n = int.parse(b);
    if (n == 0) throw RangeError('empty suffix');
    start = max(0, length - n);
    end = length - 1;
  } else {
    start = int.parse(a);
    end = b.isEmpty ? length - 1 : min(int.parse(b), length - 1);
  }
  if (start >= length || end < start) throw RangeError('outside the file');
  return (start, end);
}

/// A random URL token: 128 bits, hex.
String newRelayToken([Random? random]) {
  final r = random ?? Random.secure();
  return List.generate(
    16,
    (_) => r.nextInt(256).toRadixString(16).padLeft(2, '0'),
  ).join();
}

/// The IPv4 address the relay listens on: the one that shares the longest
/// prefix with [receiver] (the cast device's address, when known), else a
/// private address of a Wi-Fi or Ethernet interface. Loopback, link-local
/// and interfaces of VPNs, mobile data and containers come last or not at
/// all. Null without a usable address (no Wi-Fi).
InternetAddress? pickLanAddress(
  List<NetworkInterface> interfaces, {
  InternetAddress? receiver,
}) {
  final candidates = <(NetworkInterface, InterfaceAddress)>[
    for (final i in interfaces)
      for (final a in i.addresses)
        if (a.type == InternetAddressType.IPv4 &&
            !a.isLoopback &&
            !a.isLinkLocal &&
            _isPrivate(a))
          (i, a),
  ];
  if (candidates.isEmpty) return null;
  int score((NetworkInterface, InterfaceAddress) c) {
    final (iface, addr) = c;
    var s = 0;
    if (receiver != null && receiver.type == InternetAddressType.IPv4) {
      final common = _commonPrefixBits(addr.rawAddress, receiver.rawAddress);
      // The receiver is on this interface's subnet: the one.
      if (addr.prefixLength > 0 && common >= addr.prefixLength) s += 10000;
      s += common * 10;
    }
    final name = iface.name.toLowerCase();
    const avoid = ['tun', 'tap', 'ppp', 'rmnet', 'ccmni', 'docker', 'veth'];
    const prefer = ['wlan', 'wifi', 'en', 'eth', 'ap'];
    if (avoid.any(name.startsWith)) s -= 1000;
    if (prefer.any(name.startsWith)) s += 5;
    return s;
  }

  candidates.sort((x, y) => score(y).compareTo(score(x)));
  return candidates.first.$2;
}

bool _isPrivate(InternetAddress a) {
  final b = a.rawAddress;
  return b[0] == 10 ||
      (b[0] == 172 && b[1] >= 16 && b[1] < 32) ||
      (b[0] == 192 && b[1] == 168) ||
      // Carrier-grade NAT ranges show up on some hotspots.
      (b[0] == 100 && b[1] >= 64 && b[1] < 128);
}

int _commonPrefixBits(List<int> a, List<int> b) {
  var bits = 0;
  for (var i = 0; i < min(a.length, b.length); i++) {
    final x = a[i] ^ b[i];
    if (x == 0) {
      bits += 8;
      continue;
    }
    var mask = 0x80;
    while (mask > 0 && x & mask == 0) {
      bits++;
      mask >>= 1;
    }
    break;
  }
  return bits;
}

/// A running relay for one file; [close] it when the cast session ends.
class CastRelay {
  CastRelay._(this._server, this._body, this._token, this._name, this.mime)
    : url =
          'http://${_server.address.address}:${_server.port}/$_token/$_name';

  /// Listens on [address] (a port the system picks) and serves [body] at
  /// `/<token>/<name>`; every other path gets `404`, other methods `405`.
  static Future<CastRelay> start({
    required InternetAddress address,
    required CastBody body,
    required String mime,
    String name = 'media',
    Random? random,
  }) async {
    final server = await HttpServer.bind(address, 0);
    final relay = CastRelay._(
      server,
      body,
      newRelayToken(random),
      Uri.encodeComponent(name),
      mime,
    );
    server.listen(
      relay._handle,
      onError: (Object e) {
        debugPrint('Cast relay: $e');
      },
    );
    return relay;
  }

  final HttpServer _server;
  final CastBody _body;
  final String _token;
  final String _name;

  /// Content type of the file.
  final String mime;

  /// The file's URL for the receiver. Contains the token: not logged.
  /// Kept after [close]: the closed server no longer knows its address.
  final String url;

  /// Requests being served.
  int get active => _active;
  int _active = 0;
  bool _closed = false;

  Future<void> _handle(HttpRequest request) async {
    final res = request.response;
    _active++;
    try {
      // CORS: the Default Media Receiver is a web page; media elements do
      // not need it, the receiver's own fetches (HEAD probes) do.
      res.headers
        ..set('Access-Control-Allow-Origin', '*')
        ..set(HttpHeaders.acceptRangesHeader, 'bytes');
      if (request.uri.path != '/$_token/$_name') {
        res.statusCode = HttpStatus.notFound;
        return;
      }
      final head = request.method == 'HEAD';
      if (!head && request.method != 'GET') {
        res.statusCode = HttpStatus.methodNotAllowed;
        return;
      }
      final length = await _body.length();
      (int, int)? range;
      try {
        range = parseRange(
          request.headers.value(HttpHeaders.rangeHeader),
          length,
        );
      } on RangeError {
        res
          ..statusCode = HttpStatus.requestedRangeNotSatisfiable
          ..headers.set(HttpHeaders.contentRangeHeader, 'bytes */$length');
        return;
      }
      final (start, end) = range ?? (0, length - 1);
      res.headers.contentType = ContentType.parse(mime);
      res.contentLength = length == 0 ? 0 : end - start + 1;
      if (range != null) {
        res.statusCode = HttpStatus.partialContent;
        res.headers.set(
          HttpHeaders.contentRangeHeader,
          'bytes $start-$end/$length',
        );
      }
      if (head || length == 0) return;
      await res.addStream(await _body.read(start, end));
    } catch (e) {
      // The receiver closed the connection (a seek), or the source failed.
      debugPrint('Cast relay request failed: $e');
      try {
        res.statusCode = HttpStatus.badGateway;
      } catch (_) {
        // Headers already sent.
      }
    } finally {
      _active--;
      try {
        await res.close();
      } catch (_) {
        // Connection gone.
      }
    }
  }

  /// Stops listening and drops open connections.
  Future<void> close() async {
    if (_closed) return;
    _closed = true;
    await _server.close(force: true);
    await _body.close();
  }
}
