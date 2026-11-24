// SPDX-License-Identifier: MPL-2.0
//
// UI-052: the player's own strings in Ukrainian: every string is translated,
// the locale is picked by language code, numbers use a decimal comma, and
// the player screen follows the locale.
import 'package:flutter/material.dart';
import 'package:flutter_localizations/flutter_localizations.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:pa_player/pa_player.dart';
import 'package:pa_player/testing.dart';

void main() {
  test('every string is translated into Ukrainian (UI-052)', () {
    const en = PaPlayerStrings.en;
    const uk = PaPlayerStrings.uk;
    final pairs = <(String, String)>[
      (en.queueTab, uk.queueTab),
      (en.historyTab, uk.historyTab),
      (en.playingNext, uk.playingNext),
      (en.clearQueue, uk.clearQueue),
      (en.saveAsPlaylist, uk.saveAsPlaylist),
      (en.removeFromQueue, uk.removeFromQueue),
      (en.dragToReorder, uk.dragToReorder),
      (en.historyEmpty, uk.historyEmpty),
      (en.musicPaused, uk.musicPaused),
      (en.upNextFrom('X'), uk.upNextFrom('X')),
      (en.back, uk.back),
      (en.back10, uk.back10),
      (en.forward10, uk.forward10),
      (en.play, uk.play),
      (en.pause, uk.pause),
      (en.audio, uk.audio),
      (en.subtitles, uk.subtitles),
      (en.subtitleFile, uk.subtitleFile),
      (en.speed, uk.speed),
      (en.playbackSpeed, uk.playbackSpeed),
      (en.info, uk.info),
      (en.fullscreen, uk.fullscreen),
      (en.close, uk.close),
      (en.noAudioTracks, uk.noAudioTracks),
      (en.off, uk.off),
      (en.externalFile, uk.externalFile),
      (en.loadFromFile, uk.loadFromFile),
      (en.normalSpeed, uk.normalSpeed),
      (en.infoTitle, uk.infoTitle),
      (en.infoPosition, uk.infoPosition),
      (en.cannotOpenFolder, uk.cannotOpenFolder),
      (en.noSubtitleFiles, uk.noSubtitleFiles),
      (en.next, uk.next),
      (en.previous, uk.previous),
      (en.nowPlaying, uk.nowPlaying),
      (en.nothingPlaying, uk.nothingPlaying),
      (en.shuffleOn, uk.shuffleOn),
      (en.shuffleOff, uk.shuffleOff),
      (en.repeatOff, uk.repeatOff),
      (en.repeatAll, uk.repeatAll),
      (en.repeatOne, uk.repeatOne),
      (en.upNext, uk.upNext),
      (en.endOfQueue, uk.endOfQueue),
      (en.playNow, uk.playNow),
      (en.cancel, uk.cancel),
      (en.upNextIn(7), uk.upNextIn(7)),
      (en.subtitleDelay, uk.subtitleDelay),
      (en.subtitleDelayNone, uk.subtitleDelayNone),
      (en.subtitlesEarlier, uk.subtitlesEarlier),
      (en.subtitlesLater, uk.subtitlesLater),
      (en.resetDelay, uk.resetDelay),
      (en.forcedTrack, uk.forcedTrack),
      (en.sdhTrack, uk.sdhTrack),
      (
        en.delayLabel(const Duration(milliseconds: 100)),
        uk.delayLabel(const Duration(milliseconds: 100)),
      ),
      (
        en.delayLabel(const Duration(milliseconds: -100)),
        uk.delayLabel(const Duration(milliseconds: -100)),
      ),
      (
        en.remainingEndsAt('1:00', '21:40'),
        uk.remainingEndsAt('1:00', '21:40'),
      ),
      (
        en.trackLabel(const MediaTrack(id: '2')),
        uk.trackLabel(const MediaTrack(id: '2')),
      ),
      (en.engineInfoKey('Resolution'), uk.engineInfoKey('Resolution')),
      // UI-013
      (en.zoom, uk.zoom),
      (en.zoomFit, uk.zoomFit),
      (en.zoomFill, uk.zoomFill),
      (en.zoomStretch, uk.zoomStretch),
      (en.resetPinch, uk.resetPinch),
      (en.chapters, uk.chapters),
      (en.previousChapter, uk.previousChapter),
      (en.nextChapter, uk.nextChapter),
      (en.skipIntro, uk.skipIntro),
      (en.skipRecap, uk.skipRecap),
      (en.skipCredits, uk.skipCredits),
      (en.statistics, uk.statistics),
      (en.showStatistics, uk.showStatistics),
      (en.exportStats, uk.exportStats),
      (en.transcodeHint, uk.transcodeHint),
      (en.brightness, uk.brightness),
      (en.volume, uk.volume),
      (en.holdSpeed, uk.holdSpeed),
      (en.retry, uk.retry),
      (en.errorCode, uk.errorCode),
      (en.errorDetails, uk.errorDetails),
      for (final c in PlaybackErrorCode.values) ...[
        (en.playbackErrorTitle(c), uk.playbackErrorTitle(c)),
        (en.playbackErrorHint(c), uk.playbackErrorHint(c)),
      ],
      (en.skipBackBy(15), uk.skipBackBy(15)),
      (en.skipForwardBy(15), uk.skipForwardBy(15)),
      (en.seconds(-20), uk.seconds(-20)),
      (en.chapterNumber(3), uk.chapterNumber(3)),
      for (final k in const [
        'Dropped frames',
        'Video bitrate',
        'Audio bitrate',
        'Cache',
        'Network speed',
        'Hardware decoding',
        'Rendered fps',
        'Display fps',
        'A/V sync',
        // PLAY-010: rows of the stats model and their words.
        'Source',
        'Container',
        'Video',
        'Decoder',
        'Output',
        'AFR',
        'Decoder drops',
        'Audio',
        'Audio path',
        'Bitrate',
        'Buffer',
        'Stalls',
        'Hardware',
        'Software',
        'Local file',
        'Network share',
        'Media server',
        'Live stream',
        'Passthrough',
        'Decoded to PCM',
        'Off',
        'Switched',
        'Matched',
        'Unsupported',
        // QA-007: the indicator «мережа проти бітрейту».
        'Network vs bitrate',
        'Enough',
        'Tight',
        'Too slow',
      ])
        (en.engineInfoKey(k), uk.engineInfoKey(k)),
    ];
    for (final (e, r) in pairs) {
      expect(e, isNotEmpty);
      expect(r, isNotEmpty);
      expect(r, isNot(e), reason: '"$e" is not translated');
    }
    expect(uk.trackLabel(MediaTrack.off), 'Вимк.');
    expect(uk.engineInfoKey('Unknown key'), 'Unknown key');
  });

  test('forLocale picks Ukrainian by language code', () {
    expect(
      PaPlayerStrings.forLocale(const Locale('uk', 'UA')).languageCode,
      'uk',
    );
    expect(PaPlayerStrings.forLocale(const Locale('uk')), PaPlayerStrings.uk);
    expect(PaPlayerStrings.supportedLocales, contains(const Locale('uk')));
  });

  test('Ukrainian numbers use a decimal comma', () {
    const uk = PaPlayerStrings.uk;
    expect(
      uk.delayLabel(const Duration(milliseconds: 300)),
      'Субтитри пізніше на 0,3 с',
    );
    expect(
      uk.audioDelayLabel(const Duration(milliseconds: -250)),
      'Звук раніше на 0,25 с',
    );
    expect(uk.percent(150), '150 %');
  });

  testWidgets('PlayerScreen in Ukrainian', (t) async {
    final engine = FakePlayerEngine();
    await t.pumpWidget(
      MaterialApp(
        locale: const Locale('uk'),
        supportedLocales: const [Locale('en'), Locale('uk')],
        localizationsDelegates: const [
          PaPlayerStrings.delegate,
          ...GlobalMaterialLocalizations.delegates,
        ],
        home: PlayerScreen(
          request: const PlayRequest(uri: '/media/movie.mkv', title: 'Фільм'),
          onProgress: (_, _) {},
          engineFactory: ({bool video = true}) => engine,
        ),
      ),
    );
    await t.pump(const Duration(milliseconds: 300));
    expect(find.byTooltip('Пауза'), findsOneWidget);
    expect(find.byTooltip('Назад на 10 с'), findsOneWidget);
    expect(find.byTooltip('Субтитри'), findsOneWidget);
    expect(find.textContaining('закінчиться о'), findsOneWidget);
    await t.pumpWidget(const SizedBox());
  });
}
