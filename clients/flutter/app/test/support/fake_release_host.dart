// SPDX-License-Identifier: MPL-2.0
//
// PKG-018: a local stand-in for the GitHub releases API and its assets, on
// loopback.

import 'dart:convert';
import 'dart:io';

/// One release the host lists.
class FakeRelease {
  FakeRelease(
    this.version, {
    this.prerelease,
    this.assets = const {},
    this.notes = '',
    this.title,
    this.draft = false,
  });

  final String version;
  final bool? prerelease;

  /// Asset name → bytes.
  final Map<String, List<int>> assets;
  final String notes;
  final String? title;
  final bool draft;
}

/// Serves `GET /releases` like `api.github.com/repos/…/releases` and
/// `GET /download/<tag>/<name>` for the assets.
class FakeReleaseHost {
  FakeReleaseHost._(this._server);

  final HttpServer _server;
  final List<FakeRelease> releases = [];

  /// Replaces what `/download/…/<name>` serves (a tampered asset) without
  /// touching the listed size.
  final Map<String, List<int>> overrides = {};

  /// Answers every request with this status (e.g. 403 + rate limit).
  int? failWith;
  int requests = 0;

  /// Request headers seen, for privacy checks.
  final List<HttpHeaders> seenHeaders = [];
  final List<Uri> seenUris = [];

  static Future<FakeReleaseHost> start() async {
    final server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
    final host = FakeReleaseHost._(server);
    server.listen(host._handle);
    return host;
  }

  Uri get api => Uri.parse('http://127.0.0.1:${_server.port}/releases');

  Future<void> close() => _server.close(force: true);

  Future<void> _handle(HttpRequest req) async {
    requests++;
    seenHeaders.add(req.headers);
    seenUris.add(req.uri);
    final res = req.response;
    if (failWith case final status?) {
      res.statusCode = status;
      res.headers.set('x-ratelimit-remaining', '0');
      await res.close();
      return;
    }
    final segments = req.uri.pathSegments;
    if (segments.length == 1 && segments.first == 'releases') {
      res.headers.contentType = ContentType.json;
      res.write(jsonEncode([for (final r in releases) _json(r)]));
      await res.close();
      return;
    }
    if (segments.length == 3 && segments.first == 'download') {
      final tag = segments[1], name = segments[2];
      for (final r in releases) {
        if ('app-v${r.version}' != tag) continue;
        final bytes = overrides[name] ?? r.assets[name];
        if (bytes == null) break;
        res.contentLength = bytes.length;
        res.add(bytes);
        await res.close();
        return;
      }
    }
    res.statusCode = HttpStatus.notFound;
    await res.close();
  }

  Map<String, Object?> _json(FakeRelease r) {
    final tag = 'app-v${r.version}';
    return {
      'tag_name': tag,
      'name': r.title ?? 'Kinolune ${r.version}',
      'draft': r.draft,
      'prerelease': r.prerelease ?? r.version.contains('-'),
      'body': r.notes,
      'html_url': 'https://github.com/DenisHumen/project_alpha/releases/tag/$tag',
      'published_at': '2026-11-14T12:00:00Z',
      'assets': [
        for (final e in r.assets.entries)
          {
            'name': e.key,
            'size': e.value.length,
            'browser_download_url':
                'http://127.0.0.1:${_server.port}/download/$tag/${e.key}',
          },
      ],
    };
  }
}
