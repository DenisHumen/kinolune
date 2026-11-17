// SPDX-License-Identifier: MPL-2.0

// The trailer of a movie or show page (UI-036, ADR-0100). The core picks
// it (pa-trailers): the user's own trailer file, the media server's
// trailer, then — with internet trailers on — a TMDB video for the
// official embedded player of YouTube or Vimeo. Videos are never
// downloaded or extracted.

import 'item_details.dart';

/// Where a trailer comes from.
enum TrailerSource {
  /// A file next to the film (`-trailer`, `Trailers/`).
  file,

  /// The user's Jellyfin or Emby server.
  server,

  /// YouTube, through the `youtube-nocookie.com` IFrame player.
  youtube,

  /// Vimeo, through the `player.vimeo.com` embed.
  vimeo;

  /// An internet video (played by the site's own player).
  bool get online => this == youtube || this == vimeo;
}

/// The trailer a page plays.
class Trailer {
  const Trailer({
    required this.source,
    this.extra,
    this.url,
    this.videoKey,
    this.name = '',
    this.language,
    this.videoType = 'trailer',
  });

  final TrailerSource source;

  /// [TrailerSource.file]: the extra.
  final MediaExtra? extra;

  /// [TrailerSource.server]: the stream URL (holds a token, never log
  /// it); online: the video's page at its site (opens the site's app
  /// where no embedded player exists).
  final String? url;

  /// Online: the validated video id for the embedded player.
  final String? videoKey;
  final String name;

  /// ISO 639-1, if known.
  final String? language;

  /// `trailer`, `teaser` or `clip`.
  final String videoType;

  /// What the app's own player opens: the file or the server stream;
  /// null for online videos.
  String? get playbackUri => switch (source) {
    TrailerSource.file => extra?.playbackUri,
    TrailerSource.server => url,
    _ => null,
  };
}

/// A library that finds trailers of its movies and shows.
abstract interface class LibraryTrailers {
  /// The trailer of [itemId], or null. [uiLanguage] (`ru`, `en-US`) is
  /// the first language a video is wanted in; [internet] off means no
  /// TMDB, YouTube or Vimeo; [serverUrl] is the media server's trailer,
  /// if the app found one.
  Future<Trailer?> trailer(
    String itemId, {
    required String uiLanguage,
    required bool internet,
    String? serverUrl,
  });
}
