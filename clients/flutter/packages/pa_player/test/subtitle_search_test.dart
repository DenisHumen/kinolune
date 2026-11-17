// SPDX-License-Identifier: MPL-2.0
//
// PLAY-008 in the video player: «Найти субтитры» in the subtitle menu
// shows the offers best first with the quota left, one tap downloads a
// subtitle and switches to it (remembered for the series), and failures
// say what to do.
import 'package:flutter/material.dart';
import 'package:flutter_localizations/flutter_localizations.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:pa_player/pa_player.dart';
import 'package:pa_player/testing.dart';

class _FakeSearch implements SubtitleSearch {
  _FakeSearch({this.result, this.download_});

  SubtitleSearchResult? result;
  SubtitleDownloadResult? download_;
  final searched = <List<String>>[];
  final downloaded = <SubtitleOffer>[];

  @override
  Future<SubtitleSearchResult> search(List<String> languages) async {
    searched.add(languages);
    return result ?? const SubtitleSearchResult();
  }

  @override
  Future<SubtitleDownloadResult> download(SubtitleOffer offer) async {
    downloaded.add(offer);
    return download_ ?? const SubtitleDownloadResult();
  }
}

const _offers = [
  SubtitleOffer(
    id: '7002',
    release: 'Lunar.Harbor.2019.1080p.WEB-DL',
    language: 'rus',
    downloads: 30,
    hashMatch: true,
  ),
  SubtitleOffer(
    id: '7003',
    release: 'Lunar.Harbor.2019.720p.HDTV',
    language: 'rus',
    downloads: 1234,
  ),
  SubtitleOffer(
    id: '7001',
    release: 'Lunar.Harbor.2019.BluRay',
    language: 'eng',
    downloads: 5000,
    hearingImpaired: true,
  ),
];

const _downloaded = ExternalSubtitle(
  uri: '/media/Lunar.Harbor.2019.1080p.WEB-DL.ru.ass',
  title: 'Lunar.Harbor.2019.1080p.WEB-DL.ru.ass',
  language: 'rus',
);

class _Harness {
  _Harness(this.search);

  final _FakeSearch search;
  final engine = FakePlayerEngine();
  final memories = <(String, TrackMemory)>[];

  Widget app(Locale locale) => MaterialApp(
    locale: locale,
    localizationsDelegates: const [
      PaPlayerStrings.delegate,
      ...GlobalMaterialLocalizations.delegates,
    ],
    supportedLocales: const [Locale('en'), Locale('ru')],
    home: PlayerScreen(
      request: PlayRequest(
        uri: '/media/Lunar.Harbor.2019.1080p.WEB-DL.mkv',
        title: 'Lunar Harbor',
        subtitles: const [],
        seriesKey: 'm1',
        subtitleSearch: search,
      ),
      onProgress: (_, _) {},
      engineFactory: ({bool video = true}) => engine,
      subtitleLanguages: const ['ru'],
      captionStyleLoader: () async => null,
      onSubtitleMemory: (k, m) => memories.add((k, m)),
    ),
  );

  PlayerScreenState state(WidgetTester t) =>
      t.state<PlayerScreenState>(find.byType(PlayerScreen));
}

Future<_Harness> _open(
  WidgetTester t,
  _FakeSearch search, {
  Locale locale = const Locale('ru'),
}) async {
  final h = _Harness(search);
  await t.pumpWidget(h.app(locale));
  await t.pumpAndSettle();
  h.state(t).osd.openPanel(OsdPanel.subtitles);
  await t.pumpAndSettle();
  return h;
}

void main() {
  testWidgets('the subtitle menu offers the search; one tap downloads and '
      'switches', (t) async {
    final search = _FakeSearch(
      result: const SubtitleSearchResult(
        offers: _offers,
        quota: SubtitleQuota(remaining: 18, allowed: 20, loggedIn: true),
      ),
      download_: const SubtitleDownloadResult(
        subtitle: _downloaded,
        nextToVideo: true,
        quota: SubtitleQuota(remaining: 17, allowed: 20, loggedIn: true),
      ),
    );
    final h = await _open(t, search);
    await t.tap(find.text('Найти субтитры'));
    await t.pumpAndSettle();

    // The profile's languages go to the search.
    expect(search.searched, [
      ['rus'],
    ]);
    expect(find.text('Осталось скачиваний: 18 из 20'), findsOneWidget);
    // Best first, as the core ranked them; the file's own match is marked.
    final rows = find.byWidgetPredicate(
      (w) =>
          w.key is ValueKey<String> &&
          (w.key! as ValueKey<String>).value.startsWith('subsearch-offer-'),
    );
    expect(t.widgetList(rows).map((w) => (w.key! as ValueKey<String>).value), [
      'subsearch-offer-7002',
      'subsearch-offer-7003',
      'subsearch-offer-7001',
    ]);
    expect(find.text('Русский · Точно для этого файла'), findsOneWidget);
    expect(
      find.text('Lunar.Harbor.2019.720p.HDTV · 1 234 загрузки'),
      findsOneWidget,
    );
    expect(
      find.text('Lunar.Harbor.2019.BluRay · 5 000 загрузок'),
      findsOneWidget,
    );
    expect(find.text('Английский · для слабослышащих'), findsOneWidget);

    await t.tap(find.byKey(const ValueKey('subsearch-offer-7002')));
    await t.pumpAndSettle();
    expect(search.downloaded.single.id, '7002');
    expect(
      h.engine.calls,
      contains('addsub:Lunar.Harbor.2019.1080p.WEB-DL.ru.ass:select'),
    );
    expect(
      h.engine.state.value.subtitleTrack!.title,
      'Lunar.Harbor.2019.1080p.WEB-DL.ru.ass',
    );
    // The panel closed; the choice is kept for the series.
    expect(find.byKey(const ValueKey('panel-subtitleSearch')), findsNothing);
    expect(h.memories.last.$1, 'm1');
    expect(h.memories.last.$2.choice?.language, 'rus');
  });

  testWidgets('no key: the search says where to add one', (t) async {
    final search = _FakeSearch(
      result: const SubtitleSearchResult(failure: SubtitleSearchFailure.noKey),
    );
    await _open(t, search, locale: const Locale('en'));
    await t.tap(find.text('Find subtitles'));
    await t.pumpAndSettle();
    expect(
      find.textContaining('Add your own key in Settings → Subtitle style'),
      findsOneWidget,
    );
    await t.tap(find.text('Try again'));
    await t.pumpAndSettle();
    expect(search.searched, hasLength(2));
  });

  testWidgets('quota used up: the download says until when', (t) async {
    final search = _FakeSearch(
      result: const SubtitleSearchResult(offers: _offers),
      download_: SubtitleDownloadResult(
        failure: SubtitleSearchFailure.quota,
        quota: SubtitleQuota(
          remaining: 0,
          allowed: 20,
          resetsAt: DateTime(2026, 11, 18, 3, 5),
        ),
      ),
    );
    final h = await _open(t, search);
    await t.tap(find.text('Найти субтитры'));
    await t.pumpAndSettle();
    await t.tap(find.byKey(const ValueKey('subsearch-offer-7003')));
    await t.pumpAndSettle();
    expect(find.text('Лимит скачиваний исчерпан до 03:05'), findsWidgets);
    expect(h.engine.addedSubtitles, isEmpty);
    // The panel stays open: another offer is no better today.
    expect(find.byKey(const ValueKey('panel-subtitleSearch')), findsOneWidget);
  });

  testWidgets('without a search the menu has no such row', (t) async {
    final h = _Harness(_FakeSearch());
    await t.pumpWidget(
      MaterialApp(
        home: PlayerScreen(
          request: const PlayRequest(uri: '/m/a.mkv', title: 'A'),
          onProgress: (_, _) {},
          engineFactory: ({bool video = true}) => h.engine,
          captionStyleLoader: () async => null,
        ),
      ),
    );
    await t.pumpAndSettle();
    h.state(t).osd.openPanel(OsdPanel.subtitles);
    await t.pumpAndSettle();
    expect(find.byKey(const ValueKey('sub-search')), findsNothing);
    expect(find.byKey(const ValueKey('sub-file')), findsOneWidget);
  });
}
