// SPDX-License-Identifier: MPL-2.0

// Subtitles from the internet (PLAY-008): OpenSubtitles through the core,
// and Subdl as a second source (PLAY-016): both are asked at once and their
// offers ranked together; each offer names its provider.
// The core asks by the file's OSHash and the ids of its work, ranks the
// offers (in sync with the file, profile languages, human translation,
// downloads) and keeps a download byte for byte: next to the video on a
// writable source, else in the app's cache, attached to the file.

import 'models.dart';
import 'subtitles.dart';

/// The user's OpenSubtitles settings and own Subdl key (from the app's
/// secure storage).
class OpenSubtitlesAccount {
  const OpenSubtitlesAccount({
    this.username,
    this.password,
    this.apiKey,
    this.subdlApiKey,
  });

  /// No account and no own key: the key built into the app, the quota of
  /// the network address.
  static const none = OpenSubtitlesAccount();

  final String? username;
  final String? password;

  /// The user's own API key; null: the key built into the app.
  final String? apiKey;

  /// The user's own Subdl API key (PLAY-016); null: the key built into the
  /// app, if any.
  final String? subdlApiKey;

  bool get hasLogin =>
      (username?.trim().isNotEmpty ?? false) && (password?.isNotEmpty ?? false);

  @override
  bool operator ==(Object other) =>
      other is OpenSubtitlesAccount &&
      other.username == username &&
      other.password == password &&
      other.apiKey == apiKey &&
      other.subdlApiKey == subdlApiKey;

  @override
  int get hashCode => Object.hash(username, password, apiKey, subdlApiKey);

  @override
  String toString() => 'OpenSubtitlesAccount(${username ?? '-'})';
}

/// The download quota as last seen.
class OnlineSubtitleQuota {
  const OnlineSubtitleQuota({
    this.remaining,
    this.allowed,
    this.resetsAt,
    this.loggedIn = false,
  });

  /// Downloads left; null until known.
  final int? remaining;

  /// Downloads a day, when known.
  final int? allowed;

  /// When the quota is renewed, when known.
  final DateTime? resetsAt;

  /// The account's quota, not the network address's.
  final bool loggedIn;
}

/// One subtitle the search offers, best first.
class OnlineSubtitleOffer {
  const OnlineSubtitleOffer({
    required this.fileId,
    required this.subtitleId,
    required this.release,
    this.provider = providerOpenSubtitles,
    this.downloadUrl,
    this.language,
    this.providerLanguage,
    this.downloads = 0,
    this.hashMatch = false,
    this.hearingImpaired = false,
    this.forced = false,
    this.machineTranslated = false,
    this.trusted = false,
    this.fps,
    this.uploader,
  });

  /// [provider] of OpenSubtitles offers.
  static const providerOpenSubtitles = 'opensubtitles';

  /// [provider] of Subdl offers.
  static const providerSubdl = 'subdl';

  /// Where the offer comes from: [providerOpenSubtitles] or [providerSubdl].
  final String provider;

  /// OpenSubtitles' file id (0 for Subdl).
  final int fileId;
  final String subtitleId;

  /// Subdl: the archive's path on its download host.
  final String? downloadUrl;
  final String release;

  /// Tells offers of every provider apart.
  String get key => '$provider:$fileId:${downloadUrl ?? subtitleId}';

  /// ISO 639-2/B (`rus`), or the provider's code when unknown.
  final String? language;

  /// The provider's code (`ru`, `pt-br`).
  final String? providerLanguage;
  final int downloads;

  /// Made for this very file: in sync with it.
  final bool hashMatch;
  final bool hearingImpaired;
  final bool forced;
  final bool machineTranslated;
  final bool trusted;
  final double? fps;
  final String? uploader;
}

/// Why a search or a download gave nothing (the core's codes).
enum OnlineSubtitleError {
  noKey,
  keyRejected,
  login,
  quota,
  busy,
  network,
  other;

  static OnlineSubtitleError? parse(String? code) => switch (code) {
    null => null,
    'no_key' => noKey,
    'key_rejected' => keyRejected,
    'login' => login,
    'quota' => quota,
    'busy' => busy,
    'network' => network,
    _ => other,
  };
}

/// What a search found.
class OnlineSubtitleSearch {
  const OnlineSubtitleSearch({this.offers = const [], this.quota, this.error});

  final List<OnlineSubtitleOffer> offers;
  final OnlineSubtitleQuota? quota;
  final OnlineSubtitleError? error;
}

/// What a download gave.
class OnlineSubtitleDownload {
  const OnlineSubtitleDownload({
    this.subtitle,
    this.nextToVideo = false,
    this.quota,
    this.error,
  });

  /// The subtitle for the player; null on error.
  final SidecarSubtitle? subtitle;

  /// Written next to the video (else into the app's cache).
  final bool nextToVideo;
  final OnlineSubtitleQuota? quota;
  final OnlineSubtitleError? error;
}

/// The account check of the settings screen.
class OpenSubtitlesStatus {
  const OpenSubtitlesStatus({this.quota, this.level, this.error});

  final OnlineSubtitleQuota? quota;

  /// Rank name (`Sub leecher`), when logged in.
  final String? level;
  final OnlineSubtitleError? error;
}

/// A library that finds subtitles for its videos on the internet.
abstract interface class LibraryOnlineSubtitles {
  /// The build has the project's OpenSubtitles key.
  bool get openSubtitlesHasProjectKey;

  /// The build has the project's Subdl key (else Settings offer a field
  /// for the user's own one).
  bool get subdlHasProjectKey;

  /// Offers of OpenSubtitles and Subdl for the file [item] plays, ranked
  /// together. [languages]: the profile's subtitle languages, best first.
  /// Spends no download quota. An error only when no provider answered.
  Future<OnlineSubtitleSearch> searchOnlineSubtitles(
    LibraryItem item,
    List<String> languages,
    OpenSubtitlesAccount account,
  );

  /// Downloads [offer] from its provider for the file [item] plays (one
  /// download of the OpenSubtitles quota for its offers) and keeps it; a
  /// Subdl archive is unpacked.
  Future<OnlineSubtitleDownload> downloadOnlineSubtitle(
    LibraryItem item,
    OnlineSubtitleOffer offer,
    OpenSubtitlesAccount account,
  );

  /// Logs in with [account] and reads its quota.
  Future<OpenSubtitlesStatus> openSubtitlesStatus(OpenSubtitlesAccount account);
}
