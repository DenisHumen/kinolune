// SPDX-License-Identifier: MPL-2.0
import 'package:flutter/material.dart';

import '../l10n/pa_player_strings.dart';
import '../play_request.dart';
import '../ui/focus_button.dart';
import '../ui/tokens.dart';

/// Subtitles from the internet (PLAY-008): «Найти субтитры» in the
/// player's subtitle menu. The app implements [SubtitleSearch] (OpenSubtitles
/// through the core) and passes it in [PlayRequest.subtitleSearch]; the
/// player shows the offers, best first, and one tap downloads a subtitle
/// and switches to it.

/// One subtitle the search offers.
@immutable
class SubtitleOffer {
  const SubtitleOffer({
    required this.id,
    required this.release,
    this.language,
    this.downloads = 0,
    this.hashMatch = false,
    this.hearingImpaired = false,
    this.forced = false,
    this.machineTranslated = false,
    this.data,
  });

  /// Identifies the offer in the list.
  final String id;

  /// Release name.
  final String release;

  /// ISO 639-2/B code (`rus`), or the provider's code; null: unknown.
  final String? language;

  /// Downloads so far.
  final int downloads;

  /// Made for this very file: in sync with it.
  final bool hashMatch;
  final bool hearingImpaired;

  /// Only the foreign-language lines.
  final bool forced;

  /// Machine or AI translation.
  final bool machineTranslated;

  /// What the app needs to download it.
  final Object? data;
}

/// The download quota as last seen.
@immutable
class SubtitleQuota {
  const SubtitleQuota({
    this.remaining,
    this.allowed,
    this.resetsAt,
    this.loggedIn = false,
  });

  /// Downloads left; null: unknown (no account, nothing downloaded yet).
  final int? remaining;

  /// Downloads a day, when known.
  final int? allowed;

  /// When the quota is renewed, when known.
  final DateTime? resetsAt;

  /// The account's quota, not the network address's.
  final bool loggedIn;

  bool get exhausted => remaining != null && remaining! <= 0;
}

/// Why a search or a download gave nothing.
enum SubtitleSearchFailure {
  /// No API key: neither in the build nor the user's own.
  noKey,

  /// The API refused the key.
  keyRejected,

  /// Wrong login or password.
  login,

  /// The download quota is used up.
  quota,

  /// The service is overloaded.
  busy,

  /// No connection.
  network,

  /// Anything else.
  other,
}

/// What a search found.
@immutable
class SubtitleSearchResult {
  const SubtitleSearchResult({
    this.offers = const [],
    this.quota,
    this.failure,
  });

  /// Best first: in sync with the file, the profile's languages, human
  /// translations, downloads.
  final List<SubtitleOffer> offers;
  final SubtitleQuota? quota;
  final SubtitleSearchFailure? failure;
}

/// What a download gave.
@immutable
class SubtitleDownloadResult {
  const SubtitleDownloadResult({
    this.subtitle,
    this.nextToVideo = false,
    this.quota,
    this.failure,
  });

  /// The subtitle to load; null on failure.
  final ExternalSubtitle? subtitle;

  /// Saved next to the video (else in the app's cache).
  final bool nextToVideo;
  final SubtitleQuota? quota;
  final SubtitleSearchFailure? failure;
}

/// Searches and downloads subtitles for the file that plays.
abstract interface class SubtitleSearch {
  /// Offers for the file; [languages]: the profile's subtitle languages
  /// (ISO 639), best first.
  Future<SubtitleSearchResult> search(List<String> languages);

  /// Downloads [offer] (one download of the quota) and keeps it.
  Future<SubtitleDownloadResult> download(SubtitleOffer offer);
}

/// Texts of the subtitle search (ru and en, like [PaPlayerStrings]).
@immutable
class SubtitleSearchStrings {
  const SubtitleSearchStrings._(this.ru);

  final bool ru;

  static SubtitleSearchStrings of(BuildContext context) =>
      SubtitleSearchStrings._(PaPlayerStrings.of(context).languageCode == 'ru');

  String get findSubtitles => ru ? 'Найти субтитры' : 'Find subtitles';
  String get searching => ru ? 'Ищем субтитры…' : 'Searching for subtitles…';
  String get nothingFound => ru
      ? 'Субтитров для этого файла не нашлось'
      : 'No subtitles found for this file';
  String get retry => ru ? 'Повторить' : 'Try again';
  String get downloading => ru ? 'Скачиваем…' : 'Downloading…';
  String get matchesFile => ru ? 'Точно для этого файла' : 'Made for this file';
  String get sdh => ru ? 'для слабослышащих' : 'SDH';
  String get forced => ru ? 'только надписи' : 'forced';
  String get machine => ru ? 'машинный перевод' : 'machine translation';
  String get poweredBy => 'OpenSubtitles.com';

  String downloads(int n) {
    final s = _thousands(n);
    if (!ru) return n == 1 ? '1 download' : '$s downloads';
    final mod10 = n % 10, mod100 = n % 100;
    final word = mod10 == 1 && mod100 != 11
        ? 'загрузка'
        : (mod10 >= 2 && mod10 <= 4 && (mod100 < 12 || mod100 > 14))
        ? 'загрузки'
        : 'загрузок';
    return '$s $word';
  }

  String _thousands(int n) {
    final digits = n.abs().toString();
    final sep = ru ? ' ' : ',';
    final out = StringBuffer();
    for (var i = 0; i < digits.length; i++) {
      if (i > 0 && (digits.length - i) % 3 == 0) out.write(sep);
      out.write(digits[i]);
    }
    return out.toString();
  }

  String _time(DateTime t) {
    final l = t.toLocal();
    String two(int v) => v.toString().padLeft(2, '0');
    return '${two(l.hour)}:${two(l.minute)}';
  }

  /// "Осталось скачиваний: 17 из 20"; null when nothing is known.
  String? quota(SubtitleQuota? q) {
    if (q == null || q.remaining == null) return null;
    if (q.exhausted) {
      final at = q.resetsAt;
      if (at == null) {
        return ru
            ? 'Лимит скачиваний на сегодня исчерпан'
            : 'Download limit reached for today';
      }
      return ru
          ? 'Лимит скачиваний исчерпан до ${_time(at)}'
          : 'Download limit reached until ${_time(at)}';
    }
    final left = q.remaining!;
    final of = q.allowed;
    if (ru) {
      return of == null
          ? 'Осталось скачиваний: $left'
          : 'Осталось скачиваний: $left из $of';
    }
    return of == null
        ? 'Downloads left: $left'
        : 'Downloads left: $left of $of';
  }

  String failure(SubtitleSearchFailure f, SubtitleQuota? q) => switch (f) {
    SubtitleSearchFailure.noKey =>
      ru
          ? 'В этой сборке нет ключа OpenSubtitles. Добавьте свой ключ в Настройках → «Стиль субтитров».'
          : 'This build has no OpenSubtitles key. Add your own key in Settings → Subtitle style.',
    SubtitleSearchFailure.keyRejected =>
      ru
          ? 'OpenSubtitles не принял ключ API. Проверьте его в Настройках → «Стиль субтитров».'
          : 'OpenSubtitles refused the API key. Check it in Settings → Subtitle style.',
    SubtitleSearchFailure.login =>
      ru
          ? 'Не удалось войти в OpenSubtitles: проверьте логин и пароль в Настройках → «Стиль субтитров».'
          : 'Could not sign in to OpenSubtitles: check the login and password in Settings → Subtitle style.',
    SubtitleSearchFailure.quota =>
      quota(q) ??
          (ru
              ? 'Лимит скачиваний на сегодня исчерпан'
              : 'Download limit reached for today'),
    SubtitleSearchFailure.busy =>
      ru
          ? 'OpenSubtitles перегружен. Попробуйте чуть позже.'
          : 'OpenSubtitles is busy. Try again a bit later.',
    SubtitleSearchFailure.network =>
      ru ? 'Нет связи с OpenSubtitles' : 'Cannot reach OpenSubtitles',
    SubtitleSearchFailure.other =>
      ru
          ? 'OpenSubtitles ответил ошибкой'
          : 'OpenSubtitles answered with an error',
  };

  /// Name of a language code; the code itself if unknown.
  String language(String? code) {
    if (code == null || code.isEmpty) {
      return ru ? 'Язык неизвестен' : 'Unknown language';
    }
    final names = _names[code.toLowerCase()];
    if (names == null) return code.toUpperCase();
    return ru ? names.$1 : names.$2;
  }

  static const Map<String, (String, String)> _names = {
    'rus': ('Русский', 'Russian'),
    'eng': ('Английский', 'English'),
    'ukr': ('Украинский', 'Ukrainian'),
    'bel': ('Белорусский', 'Belarusian'),
    'kaz': ('Казахский', 'Kazakh'),
    'jpn': ('Японский', 'Japanese'),
    'ger': ('Немецкий', 'German'),
    'fre': ('Французский', 'French'),
    'spa': ('Испанский', 'Spanish'),
    'ita': ('Итальянский', 'Italian'),
    'por': ('Португальский', 'Portuguese'),
    'chi': ('Китайский', 'Chinese'),
    'kor': ('Корейский', 'Korean'),
    'pol': ('Польский', 'Polish'),
    'cze': ('Чешский', 'Czech'),
    'dut': ('Нидерландский', 'Dutch'),
    'swe': ('Шведский', 'Swedish'),
    'nor': ('Норвежский', 'Norwegian'),
    'dan': ('Датский', 'Danish'),
    'fin': ('Финский', 'Finnish'),
    'tur': ('Турецкий', 'Turkish'),
    'ara': ('Арабский', 'Arabic'),
    'heb': ('Иврит', 'Hebrew'),
    'hin': ('Хинди', 'Hindi'),
    'gre': ('Греческий', 'Greek'),
    'hun': ('Венгерский', 'Hungarian'),
    'rum': ('Румынский', 'Romanian'),
    'bul': ('Болгарский', 'Bulgarian'),
    'srp': ('Сербский', 'Serbian'),
    'hrv': ('Хорватский', 'Croatian'),
    'tha': ('Тайский', 'Thai'),
    'vie': ('Вьетнамский', 'Vietnamese'),
    'ind': ('Индонезийский', 'Indonesian'),
  };
}

/// The «Найти субтитры» panel: searches on open, lists the offers best
/// first with the quota above them; a tap downloads the offer and hands it
/// to [onDownloaded] (the player loads and selects it).
class SubtitleSearchView extends StatefulWidget {
  const SubtitleSearchView({
    super.key,
    required this.search,
    required this.languages,
    required this.onDownloaded,
  });

  final SubtitleSearch search;

  /// The profile's subtitle languages, best first.
  final List<String> languages;

  /// A subtitle was downloaded; the panel is done.
  final Future<void> Function(ExternalSubtitle subtitle) onDownloaded;

  @override
  State<SubtitleSearchView> createState() => _SubtitleSearchViewState();
}

class _SubtitleSearchViewState extends State<SubtitleSearchView> {
  SubtitleSearchResult? _result;
  SubtitleQuota? _quota;
  SubtitleSearchFailure? _downloadFailure;
  String? _downloading;
  bool _searching = false;

  @override
  void initState() {
    super.initState();
    _run();
  }

  Future<void> _run() async {
    setState(() {
      _searching = true;
      _downloadFailure = null;
    });
    SubtitleSearchResult r;
    try {
      r = await widget.search.search(widget.languages);
    } catch (_) {
      r = const SubtitleSearchResult(failure: SubtitleSearchFailure.other);
    }
    if (!mounted) return;
    setState(() {
      _searching = false;
      _result = r;
      _quota = r.quota ?? _quota;
    });
  }

  Future<void> _download(SubtitleOffer offer) async {
    if (_downloading != null) return;
    setState(() {
      _downloading = offer.id;
      _downloadFailure = null;
    });
    SubtitleDownloadResult r;
    try {
      r = await widget.search.download(offer);
    } catch (_) {
      r = const SubtitleDownloadResult(failure: SubtitleSearchFailure.other);
    }
    if (!mounted) return;
    setState(() {
      _downloading = null;
      _quota = r.quota ?? _quota;
      _downloadFailure = r.subtitle == null
          ? (r.failure ?? SubtitleSearchFailure.other)
          : null;
    });
    final sub = r.subtitle;
    if (sub != null) await widget.onDownloaded(sub);
  }

  /// The language and what kind of subtitle it is: the row's title.
  String _title(SubtitleOffer o, SubtitleSearchStrings s) => [
    s.language(o.language),
    if (o.hashMatch) s.matchesFile,
    if (o.forced) s.forced,
    if (o.hearingImpaired) s.sdh,
    if (o.machineTranslated) s.machine,
  ].join(' · ');

  /// Release and downloads: the row's second line.
  String _details(SubtitleOffer o, SubtitleSearchStrings s) => [
    if (o.release.isNotEmpty) o.release,
    s.downloads(o.downloads),
  ].join(' · ');

  @override
  Widget build(BuildContext context) {
    final s = SubtitleSearchStrings.of(context);
    final r = _result;
    final quota = s.quota(_quota);
    final children = <Widget>[
      if (quota != null)
        _Note(
          key: const ValueKey('subsearch-quota'),
          text: quota,
          warn: _quota?.exhausted ?? false,
        ),
      if (_downloadFailure != null)
        _Note(
          key: const ValueKey('subsearch-download-failure'),
          text: s.failure(_downloadFailure!, _quota),
          warn: true,
        ),
    ];
    if (_searching || r == null) {
      children.add(
        Padding(
          key: const ValueKey('subsearch-loading'),
          padding: const EdgeInsets.all(24),
          child: Column(
            children: [
              const SizedBox(
                width: 28,
                height: 28,
                child: CircularProgressIndicator(
                  strokeWidth: 3,
                  color: PaTokens.text,
                ),
              ),
              const SizedBox(height: 12),
              Text(
                s.searching,
                style: const TextStyle(color: PaTokens.textSecondary),
              ),
            ],
          ),
        ),
      );
    } else if (r.failure != null) {
      children
        ..add(
          _Note(
            key: const ValueKey('subsearch-failure'),
            text: s.failure(r.failure!, _quota),
            warn: true,
          ),
        )
        ..add(
          FocusListRow(
            key: const ValueKey('subsearch-retry'),
            title: s.retry,
            leading: const Icon(Icons.refresh_rounded),
            autofocus: true,
            onPressed: _run,
          ),
        );
    } else if (r.offers.isEmpty) {
      children
        ..add(
          _Note(key: const ValueKey('subsearch-empty'), text: s.nothingFound),
        )
        ..add(
          FocusListRow(
            key: const ValueKey('subsearch-retry'),
            title: s.retry,
            leading: const Icon(Icons.refresh_rounded),
            autofocus: true,
            onPressed: _run,
          ),
        );
    } else {
      for (var i = 0; i < r.offers.length; i++) {
        final o = r.offers[i];
        final busy = _downloading == o.id;
        children.add(
          FocusListRow(
            key: ValueKey('subsearch-offer-${o.id}'),
            title: _title(o, s),
            subtitle: busy ? s.downloading : _details(o, s),
            autofocus: i == 0,
            leading: Icon(
              o.hashMatch ? Icons.verified_rounded : Icons.subtitles_rounded,
            ),
            trailing: busy
                ? const SizedBox(
                    width: 20,
                    height: 20,
                    child: CircularProgressIndicator(
                      strokeWidth: 2,
                      color: PaTokens.text,
                    ),
                  )
                : const Icon(Icons.download_rounded),
            onPressed: _downloading == null ? () => _download(o) : null,
          ),
        );
      }
    }
    children.add(
      Padding(
        padding: const EdgeInsets.fromLTRB(14, 12, 14, 4),
        child: Text(
          s.poweredBy,
          style: const TextStyle(color: PaTokens.textSecondary, fontSize: 12),
        ),
      ),
    );
    return ListView(children: children);
  }
}

class _Note extends StatelessWidget {
  const _Note({super.key, required this.text, this.warn = false});
  final String text;
  final bool warn;

  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 8),
    child: Text(
      text,
      style: TextStyle(
        color: warn ? const Color(0xFFFFB4A8) : PaTokens.textSecondary,
        fontSize: 14,
      ),
    ),
  );
}
