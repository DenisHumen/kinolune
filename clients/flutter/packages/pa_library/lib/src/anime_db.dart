// SPDX-License-Identifier: MPL-2.0
//
// The anime database on the device (META-030): anime-offline-database
// (ODbL) shipped with the app and updated once a day from its GitHub
// releases. The metadata run recognises anime by it without a request.

/// Where the database in use comes from.
enum AnimeDbOrigin {
  /// Shipped with the app.
  bundled,

  /// Downloaded from the upstream release by this device.
  downloaded,
}

/// The anime database the device uses.
class AnimeDbStatus {
  const AnimeDbStatus({
    required this.origin,
    required this.tag,
    required this.lastUpdate,
    this.entries,
    this.checkedAt,
  });

  final AnimeDbOrigin origin;

  /// Upstream release tag, `2026-46`.
  final String tag;

  /// Upstream release date, `2026-11-15`.
  final String lastUpdate;

  /// Number of titles, when known.
  final int? entries;

  /// The last answered check for a newer release.
  final DateTime? checkedAt;
}

/// Result of [LibraryAnimeDb.refreshAnimeDb].
enum AnimeDbRefresh {
  /// Checked less than a day ago; nothing was asked.
  skipped,

  /// The device already has the latest release.
  upToDate,

  /// A newer release was verified and installed.
  updated,
}

/// A library whose metadata run uses the anime database (META-030).
abstract interface class LibraryAnimeDb {
  /// The database file shipped with the app (copied out of the assets), or
  /// null in builds without one.
  String? get animeDbBundledPath;
  set animeDbBundledPath(String? path);

  /// Daily update of the anime databases from GitHub (anime-offline-database
  /// releases and the Anime-Lists list); a user option, on by default. Off:
  /// only what the device already has is used.
  bool get animeDataUpdates;
  set animeDataUpdates(bool value);

  /// The database in use, or null if there is none.
  Future<AnimeDbStatus?> animeDbStatus();

  /// Asks GitHub for a newer release (no more than once a day unless
  /// [force]); throws if the check or the download failed, in which case
  /// the database in use is kept.
  Future<AnimeDbRefresh> refreshAnimeDb({bool force = false});
}
