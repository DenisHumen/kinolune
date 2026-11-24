// SPDX-License-Identifier: MPL-2.0

// Subtitles from the internet (PLAY-008): the player's «Найти субтитры»
// on top of the library's search (OpenSubtitles and Subdl, PLAY-016), and
// the account and own keys kept in the device's secure storage.

import 'package:pa_library/pa_library.dart'
    show
        ItemKind,
        Library,
        LibraryItem,
        LibraryOnlineSubtitles,
        OnlineSubtitleError,
        OnlineSubtitleOffer,
        OnlineSubtitleQuota,
        OpenSubtitlesAccount;
import 'package:pa_player/pa_player.dart'
    show
        ExternalSubtitle,
        SubtitleDownloadResult,
        SubtitleOffer,
        SubtitleQuota,
        SubtitleSearch,
        SubtitleSearchFailure,
        SubtitleSearchResult;

import '../services/secure_store.dart';

/// Secure storage keys of the OpenSubtitles account.
abstract final class OpenSubtitlesKeys {
  static const username = 'opensubtitles_username';
  static const password = 'opensubtitles_password';
  static const apiKey = 'opensubtitles_api_key';

  /// The user's own Subdl key (PLAY-016).
  static const subdlApiKey = 'subdl_api_key';
}

String? _clean(String? v) => v == null || v.trim().isEmpty ? null : v;

/// The account and own key from [store]; empty fields are null.
Future<OpenSubtitlesAccount> readOpenSubtitlesAccount(
  SecureStore store,
) async => OpenSubtitlesAccount(
  username: _clean(await store.read(OpenSubtitlesKeys.username))?.trim(),
  password: _clean(await store.read(OpenSubtitlesKeys.password)),
  apiKey: _clean(await store.read(OpenSubtitlesKeys.apiKey))?.trim(),
  subdlApiKey: _clean(await store.read(OpenSubtitlesKeys.subdlApiKey))?.trim(),
);

/// Saves [account] in [store]; empty fields are removed.
Future<void> writeOpenSubtitlesAccount(
  SecureStore store,
  OpenSubtitlesAccount account,
) async {
  await store.write(
    OpenSubtitlesKeys.username,
    _clean(account.username)?.trim(),
  );
  await store.write(OpenSubtitlesKeys.password, _clean(account.password));
  await store.write(OpenSubtitlesKeys.apiKey, _clean(account.apiKey)?.trim());
  await store.write(
    OpenSubtitlesKeys.subdlApiKey,
    _clean(account.subdlApiKey)?.trim(),
  );
}

/// Display name of an offer's provider.
String providerName(String provider) => switch (provider) {
  OnlineSubtitleOffer.providerSubdl => 'SubDL',
  _ => 'OpenSubtitles',
};

/// The «Найти субтитры» search for [item], or null when the library cannot
/// search (the v0 library) or the item is not a video of the library.
SubtitleSearch? subtitleSearchFor(
  Library library,
  SecureStore store,
  LibraryItem item,
) {
  if (library is! LibraryOnlineSubtitles) return null;
  if (item.kind != ItemKind.movie && item.kind != ItemKind.episode) {
    return null;
  }
  return LibrarySubtitleSearch(
    library as LibraryOnlineSubtitles,
    item,
    () => readOpenSubtitlesAccount(store),
  );
}

SubtitleQuota? _quota(OnlineSubtitleQuota? q) => q == null
    ? null
    : SubtitleQuota(
        remaining: q.remaining,
        allowed: q.allowed,
        resetsAt: q.resetsAt,
        loggedIn: q.loggedIn,
      );

SubtitleSearchFailure? _failure(OnlineSubtitleError? e) => switch (e) {
  null => null,
  OnlineSubtitleError.noKey => SubtitleSearchFailure.noKey,
  OnlineSubtitleError.keyRejected => SubtitleSearchFailure.keyRejected,
  OnlineSubtitleError.login => SubtitleSearchFailure.login,
  OnlineSubtitleError.quota => SubtitleSearchFailure.quota,
  OnlineSubtitleError.busy => SubtitleSearchFailure.busy,
  OnlineSubtitleError.network => SubtitleSearchFailure.network,
  OnlineSubtitleError.other => SubtitleSearchFailure.other,
};

/// [SubtitleSearch] of the player over [LibraryOnlineSubtitles].
class LibrarySubtitleSearch implements SubtitleSearch {
  LibrarySubtitleSearch(this.library, this.item, this.account);

  final LibraryOnlineSubtitles library;
  final LibraryItem item;

  /// Read at every call: a change in Settings counts at once.
  final Future<OpenSubtitlesAccount> Function() account;

  @override
  Future<SubtitleSearchResult> search(List<String> languages) async {
    final r = await library.searchOnlineSubtitles(
      item,
      languages,
      await account(),
    );
    return SubtitleSearchResult(
      offers: [
        for (final o in r.offers)
          SubtitleOffer(
            id: o.provider == OnlineSubtitleOffer.providerOpenSubtitles
                ? '${o.fileId}'
                : o.key,
            release: o.release,
            provider: providerName(o.provider),
            language: o.language,
            downloads: o.downloads,
            hashMatch: o.hashMatch,
            hearingImpaired: o.hearingImpaired,
            forced: o.forced,
            machineTranslated: o.machineTranslated,
            data: o,
          ),
      ],
      quota: _quota(r.quota),
      failure: _failure(r.error),
    );
  }

  @override
  Future<SubtitleDownloadResult> download(SubtitleOffer offer) async {
    final o = offer.data;
    if (o is! OnlineSubtitleOffer) {
      return const SubtitleDownloadResult(failure: SubtitleSearchFailure.other);
    }
    final r = await library.downloadOnlineSubtitle(item, o, await account());
    final s = r.subtitle;
    return SubtitleDownloadResult(
      subtitle: s == null
          ? null
          : ExternalSubtitle(
              uri: s.uri,
              title: s.name,
              language: s.language,
              forced: s.forced,
              hearingImpaired: s.hearingImpaired,
            ),
      nextToVideo: r.nextToVideo,
      quota: _quota(r.quota),
      failure: _failure(r.error),
    );
  }
}
