// SPDX-License-Identifier: MPL-2.0
//
// App releases on GitHub (PKG-018, ADR-NNNN): the public releases API of the
// repository, read straight from the device. The request carries no
// identifiers: no account, no token, no device or install id, no app
// version — only the generic User-Agent GitHub requires.

import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:typed_data' show BytesBuilder;

import 'app_version.dart';

/// Releases of the app are tagged `app-v<semver>` (release-app.yml).
const kAppTagPrefix = 'app-v';

/// GitHub API of the repository's releases.
final kGitHubReleasesApi = Uri.parse(
  'https://api.github.com/repos/DenisHumen/project_alpha/releases', // l10n-ignore
);

/// User-Agent of update requests: the app name only (GitHub rejects
/// requests without one).
const kUpdateUserAgent = 'Kinolune'; // l10n-ignore

/// Which releases the user wants.
enum UpdateChannel {
  /// Releases only.
  stable,

  /// Pre-releases too (alpha, beta, rc).
  beta,
}

/// A downloadable file of a release.
class ReleaseAsset {
  const ReleaseAsset({required this.name, required this.url, this.size});

  final String name;
  final Uri url;

  /// Bytes, as GitHub reports them; null if unknown.
  final int? size;
}

/// One app release.
class AppRelease {
  const AppRelease({
    required this.tag,
    required this.version,
    required this.prerelease,
    required this.notes,
    required this.page,
    required this.assets,
    this.title,
    this.publishedAt,
  });

  final String tag;
  final AppVersion version;
  final bool prerelease;

  /// The release notes (Markdown), as written on GitHub.
  final String notes;

  /// The release page on GitHub.
  final Uri page;
  final List<ReleaseAsset> assets;
  final String? title;
  final DateTime? publishedAt;

  ReleaseAsset? asset(String name) {
    for (final a in assets) {
      if (a.name == name) return a;
    }
    return null;
  }

  /// The release says its APKs carry the throw-away CI debug key
  /// (release-app.yml without a release keystore, PKG-009).
  bool get androidDebugSigned =>
      (title ?? '').contains('debug-signed') ||
      notes.contains('Android signing: **debug**'); // l10n-ignore

  /// The changelog part of [notes]: everything before the generated
  /// "Downloads" table and licence notes, without the debug-signing
  /// warning block (the app explains that itself).
  String get changelog {
    var text = notes.replaceAll('\r\n', '\n');
    final cut = text.indexOf('\n## Downloads');
    if (cut >= 0) text = text.substring(0, cut);
    if (text.startsWith('## Downloads')) text = '';
    final lines = text
        .split('\n')
        .where((l) => !l.startsWith('> '))
        .where((l) => l.trim() != '>');
    return lines.join('\n').trim();
  }

  /// Parses one entry of the GitHub releases API; null for drafts, tags
  /// that are not app releases and broken entries.
  static AppRelease? fromGitHub(Map<String, dynamic> json) {
    if (json['draft'] == true) return null;
    final tag = json['tag_name'];
    if (tag is! String || !tag.startsWith(kAppTagPrefix)) return null;
    final version = AppVersion.tryParse(tag.substring(kAppTagPrefix.length));
    final page = Uri.tryParse('${json['html_url'] ?? ''}');
    if (version == null || page == null) return null;
    final assets = <ReleaseAsset>[];
    for (final a in (json['assets'] as List?) ?? const []) {
      if (a is! Map) continue;
      final name = a['name'];
      final url = Uri.tryParse('${a['browser_download_url'] ?? ''}');
      if (name is! String || url == null || !url.hasScheme) continue;
      assets.add(
        ReleaseAsset(name: name, url: url, size: (a['size'] as num?)?.toInt()),
      );
    }
    return AppRelease(
      tag: tag,
      version: version,
      prerelease: json['prerelease'] == true || version.isPrerelease,
      notes: (json['body'] as String?) ?? '',
      page: page,
      assets: assets,
      title: json['name'] as String?,
      publishedAt: DateTime.tryParse('${json['published_at'] ?? ''}'),
    );
  }
}

/// The newest release above [current] on [channel], or null.
AppRelease? newestUpdate(
  Iterable<AppRelease> releases,
  AppVersion current,
  UpdateChannel channel,
) {
  AppRelease? best;
  for (final r in releases) {
    if (channel == UpdateChannel.stable && r.prerelease) continue;
    if (!(r.version > current)) continue;
    if (best == null || r.version > best.version) best = r;
  }
  return best;
}

/// Why a check or a download failed.
class UpdateNetworkException implements Exception {
  const UpdateNetworkException(this.message, {this.rateLimited = false});

  final String message;

  /// GitHub's limit for anonymous requests (60 an hour per address).
  final bool rateLimited;

  @override
  String toString() => 'UpdateNetworkException: $message';
}

/// Where releases come from.
abstract class ReleaseSource {
  Future<List<AppRelease>> releases();
}

/// The GitHub releases API ([kGitHubReleasesApi]).
class GitHubReleaseSource implements ReleaseSource {
  GitHubReleaseSource({Uri? api, UpdateHttp? http})
    : api = api ?? kGitHubReleasesApi,
      http = http ?? UpdateHttp();

  final Uri api;
  final UpdateHttp http;

  @override
  Future<List<AppRelease>> releases() async {
    final body = await http.getBytes(
      api.replace(queryParameters: {'per_page': '30'}),
      accept: 'application/vnd.github+json', // l10n-ignore
      maxBytes: 4 << 20,
    );
    final Object? json;
    try {
      json = jsonDecode(utf8.decode(body));
    } on FormatException catch (e) {
      throw UpdateNetworkException('bad releases JSON: $e');
    }
    if (json is! List) throw const UpdateNetworkException('not a list');
    return [
      for (final e in json)
        if (e is Map<String, dynamic>) ?AppRelease.fromGitHub(e),
    ];
  }
}

/// Plain HTTPS GETs of update checks and downloads (dart:io, redirects
/// followed: GitHub serves assets from its CDN).
class UpdateHttp {
  UpdateHttp({this.timeout = const Duration(seconds: 30)});

  final Duration timeout;

  HttpClient _client() => HttpClient()
    ..userAgent = kUpdateUserAgent
    ..connectionTimeout = timeout
    ..idleTimeout = const Duration(seconds: 5);

  Future<HttpClientResponse> _get(
    HttpClient client,
    Uri url, {
    String? accept,
  }) async {
    if (url.scheme != 'https' && url.scheme != 'http') {
      throw UpdateNetworkException('unsupported URL scheme: ${url.scheme}');
    }
    try {
      final req = await client.getUrl(url).timeout(timeout);
      if (accept != null) req.headers.set(HttpHeaders.acceptHeader, accept);
      final res = await req.close().timeout(timeout);
      if (res.statusCode != HttpStatus.ok) {
        await res.drain<void>();
        final limited =
            (res.statusCode == 403 || res.statusCode == 429) &&
            res.headers.value('x-ratelimit-remaining') == '0';
        throw UpdateNetworkException(
          'HTTP ${res.statusCode} for ${url.path}',
          rateLimited: limited,
        );
      }
      return res;
    } on SocketException catch (e) {
      throw UpdateNetworkException('network: ${e.message}');
    } on HttpException catch (e) {
      throw UpdateNetworkException('http: ${e.message}');
    } on TimeoutException {
      throw const UpdateNetworkException('timed out');
    } on HandshakeException catch (e) {
      throw UpdateNetworkException('tls: ${e.message}');
    }
  }

  /// The body of [url], at most [maxBytes].
  Future<List<int>> getBytes(
    Uri url, {
    String? accept,
    int maxBytes = 1 << 20,
  }) async {
    final client = _client();
    try {
      final res = await _get(client, url, accept: accept);
      final out = BytesBuilder(copy: false);
      await for (final chunk in res.timeout(timeout)) {
        out.add(chunk);
        if (out.length > maxBytes) {
          throw UpdateNetworkException('${url.path} is too large');
        }
      }
      return out.takeBytes();
    } on TimeoutException {
      throw const UpdateNetworkException('timed out');
    } finally {
      client.close(force: true);
    }
  }

  /// Saves [url] to [to] (through `<to>.part`), reporting progress as
  /// (received, total or null). [cancelled] stops the download between
  /// chunks.
  Future<void> download(
    Uri url,
    File to, {
    void Function(int received, int? total)? onProgress,
    bool Function()? cancelled,
  }) async {
    final client = _client();
    final part = File('${to.path}.part');
    IOSink? sink;
    try {
      final res = await _get(client, url);
      final total = res.contentLength >= 0 ? res.contentLength : null;
      await part.parent.create(recursive: true);
      sink = part.openWrite();
      var received = 0;
      await for (final chunk in res.timeout(timeout)) {
        if (cancelled?.call() ?? false) {
          throw const UpdateCancelled();
        }
        sink.add(chunk);
        received += chunk.length;
        onProgress?.call(received, total);
      }
      await sink.flush();
      await sink.close();
      sink = null;
      if (total != null && received != total) {
        throw UpdateNetworkException('short download: $received of $total');
      }
      await part.rename(to.path);
    } on TimeoutException {
      throw const UpdateNetworkException('timed out');
    } on FileSystemException catch (e) {
      throw UpdateNetworkException('disk: ${e.message}');
    } finally {
      await sink?.close();
      if (part.existsSync()) {
        try {
          part.deleteSync();
        } on FileSystemException {
          // Removed on the next start (UpdateController cleans the folder).
        }
      }
      client.close(force: true);
    }
  }
}

/// The user cancelled a download.
class UpdateCancelled implements Exception {
  const UpdateCancelled();
}
