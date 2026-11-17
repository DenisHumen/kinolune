// SPDX-License-Identifier: MPL-2.0

// Demo mode of «Найти субтитры» (PLAY-008): made-up offers and quota, no
// network. A login named `wrong` fails, like a wrong password would.

import 'package:pa_library/pa_library.dart'
    show
        ItemKind,
        LibraryItem,
        LibraryOnlineSubtitles,
        OnlineSubtitleDownload,
        OnlineSubtitleError,
        OnlineSubtitleOffer,
        OnlineSubtitleQuota,
        OnlineSubtitleSearch,
        OpenSubtitlesAccount,
        OpenSubtitlesStatus,
        SidecarSubtitle;

mixin FakeOnlineSubtitles implements LibraryOnlineSubtitles {
  /// Downloads left of [_allowed] for an account.
  int _left = 18;
  static const _allowed = 20;

  /// Offers per search; tests may replace them.
  List<OnlineSubtitleOffer>? demoSubtitleOffers;

  @override
  bool get openSubtitlesHasProjectKey => true;

  OnlineSubtitleQuota? _quota(OpenSubtitlesAccount a) => a.hasLogin
      ? OnlineSubtitleQuota(
          remaining: _left,
          allowed: _allowed,
          resetsAt: DateTime.utc(2026, 11, 18, 3),
          loggedIn: true,
        )
      : null;

  static String _base(LibraryItem item) =>
      item.title.replaceAll(RegExp(r'[^\p{L}\p{N}]+', unicode: true), '.');

  @override
  Future<OnlineSubtitleSearch> searchOnlineSubtitles(
    LibraryItem item,
    List<String> languages,
    OpenSubtitlesAccount account,
  ) async {
    if (account.username == 'wrong') {
      return const OnlineSubtitleSearch(error: OnlineSubtitleError.login);
    }
    if (item.kind != ItemKind.movie && item.kind != ItemKind.episode) {
      return const OnlineSubtitleSearch();
    }
    final b = _base(item);
    final offers =
        demoSubtitleOffers ??
        [
          OnlineSubtitleOffer(
            fileId: 1,
            subtitleId: '1',
            release: '$b.2160p.WEB-DL',
            language: 'rus',
            downloads: 1284,
            hashMatch: true,
          ),
          OnlineSubtitleOffer(
            fileId: 2,
            subtitleId: '2',
            release: '$b.1080p.BluRay',
            language: 'rus',
            downloads: 5310,
          ),
          OnlineSubtitleOffer(
            fileId: 3,
            subtitleId: '3',
            release: '$b.1080p.BluRay',
            language: 'rus',
            downloads: 402,
            forced: true,
          ),
          OnlineSubtitleOffer(
            fileId: 4,
            subtitleId: '4',
            release: '$b.2160p.WEB-DL',
            language: 'eng',
            downloads: 21877,
            hearingImpaired: true,
          ),
          OnlineSubtitleOffer(
            fileId: 5,
            subtitleId: '5',
            release: '$b.720p.HDTV',
            language: 'ukr',
            downloads: 96,
            machineTranslated: true,
          ),
        ];
    return OnlineSubtitleSearch(offers: offers, quota: _quota(account));
  }

  @override
  Future<OnlineSubtitleDownload> downloadOnlineSubtitle(
    LibraryItem item,
    OnlineSubtitleOffer offer,
    OpenSubtitlesAccount account,
  ) async {
    if (account.hasLogin && _left <= 0) {
      return OnlineSubtitleDownload(
        quota: _quota(account),
        error: OnlineSubtitleError.quota,
      );
    }
    if (account.hasLogin) _left--;
    final lang = switch (offer.language) {
      'rus' => 'ru',
      'eng' => 'en',
      'ukr' => 'uk',
      _ => 'und',
    };
    final name = '${_base(item)}.$lang.ass';
    return OnlineSubtitleDownload(
      subtitle: SidecarSubtitle(
        uri: '/demo/subtitles/$name',
        name: name,
        language: offer.language,
        forced: offer.forced,
        hearingImpaired: offer.hearingImpaired,
      ),
      nextToVideo: true,
      quota: _quota(account),
    );
  }

  @override
  Future<OpenSubtitlesStatus> openSubtitlesStatus(
    OpenSubtitlesAccount account,
  ) async {
    if (account.username == 'wrong') {
      return const OpenSubtitlesStatus(error: OnlineSubtitleError.login);
    }
    return OpenSubtitlesStatus(quota: _quota(account), level: 'Sub leecher');
  }
}
