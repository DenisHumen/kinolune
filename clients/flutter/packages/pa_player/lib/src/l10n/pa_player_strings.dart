// SPDX-License-Identifier: MPL-2.0
import 'package:flutter/foundation.dart';
import 'package:flutter/widgets.dart';

import '../engine/playback_error.dart';
import '../engine/player_engine.dart';

/// User-facing strings of the player (OSD, side panels, mini player, Now
/// Playing). The package carries its own English and Russian texts so it does
/// not depend on the app's gen-l10n.
///
/// The app passes its locale by adding [PaPlayerStrings.delegate] to
/// `MaterialApp.localizationsDelegates`. Without the delegate, [of] still
/// picks the texts from the ambient [Localizations] locale, and falls back to
/// English for unsupported locales.
@immutable
class PaPlayerStrings {
  const PaPlayerStrings._({
    required this.languageCode,
    required this.back,
    required this.back10,
    required this.forward10,
    required this.play,
    required this.pause,
    required this.audio,
    required this.subtitles,
    required this.subtitleFile,
    required this.speed,
    required this.playbackSpeed,
    required this.info,
    required this.fullscreen,
    required this.pictureInPicture,
    required this.close,
    required this.noAudioTracks,
    required this.off,
    required this.externalFile,
    required this.loadFromFile,
    required this.normalSpeed,
    required this.infoTitle,
    required this.infoPosition,
    required this.cannotOpenFolder,
    required this.noSubtitleFiles,
    required this.next,
    required this.previous,
    required this.nowPlaying,
    required this.nothingPlaying,
    required this.shuffleOn,
    required this.shuffleOff,
    required this.repeatOff,
    required this.repeatAll,
    required this.repeatOne,
    required this.upNext,
    required this.endOfQueue,
    required this.queueTab,
    required this.historyTab,
    required this.playingNext,
    required this.clearQueue,
    required this.saveAsPlaylist,
    required this.removeFromQueue,
    required this.dragToReorder,
    required this.historyEmpty,
    required this.musicPaused,
    required this.playNow,
    required this.cancel,
    required this.subtitleDelay,
    required this.subtitleDelayNone,
    required this.subtitlesEarlier,
    required this.subtitlesLater,
    required this.resetDelay,
    required this.forcedTrack,
    required this.sdhTrack,
    required this.commentaryTrack,
    required this.audioDescriptionTrack,
    required this.audioDelay,
    required this.soundEarlier,
    required this.soundLater,
    required this.audioDelayNone,
    required this.stereoDownmix,
    required this.lfeToStereo,
    required this.normalizeDownmix,
    required this.volumeBoost,
    required this.volumeBoostLess,
    required this.volumeBoostMore,
    required this.channelsMono,
    required this.channelsStereo,
    required this.zoom,
    required this.zoomFit,
    required this.zoomFill,
    required this.zoomStretch,
    required this.resetPinch,
    required this.chapters,
    required this.previousChapter,
    required this.nextChapter,
    required this.skipIntro,
    required this.skipRecap,
    required this.skipCredits,
    required this.statistics,
    required this.showStatistics,
    required this.exportStats,
    required this.transcodeHint,
    required this.brightness,
    required this.volume,
    required this.holdSpeed,
    required this.toggleControls,
    required this.retry,
    required this.errorCode,
    required this.errorDetails,
    required this.errorOpenFailed,
    required this.errorOpenFailedHint,
    required this.errorUnsupported,
    required this.errorUnsupportedHint,
    required this.errorNothingToPlay,
    required this.errorNothingToPlayHint,
    required String Function(int seconds) skipBackBy,
    required String Function(int seconds) skipForwardBy,
    required String Function(int seconds) seconds,
    required String Function(int n) chapterNumber,
    required String Function(String seconds) delayLater,
    required String Function(String seconds) delayEarlier,
    required String Function(String seconds) soundDelayLater,
    required String Function(String seconds) soundDelayEarlier,
    required String Function(String id) track,
    required String Function(String remaining, String time) remainingEndsAt,
    required String Function(int seconds) upNextIn,
    required String Function(String source) upNextFrom,
    required Map<String, String> engineInfoKeys,
    // MUS-005. Optional so a locale without them falls back to English.
    this.audioOutput = 'Audio output',
    this.openNowPlaying = 'Open Now Playing',
    this.showQueue = 'Show queue',
    this.miniPlayer = 'Mini player',
  }) : _track = track,
       _remainingEndsAt = remainingEndsAt,
       _upNextIn = upNextIn,
       _upNextFrom = upNextFrom,
       _delayLater = delayLater,
       _delayEarlier = delayEarlier,
       _soundDelayLater = soundDelayLater,
       _soundDelayEarlier = soundDelayEarlier,
       _engineInfoKeys = engineInfoKeys,
       _skipBackBy = skipBackBy,
       _skipForwardBy = skipForwardBy,
       _seconds = seconds,
       _chapterNumber = chapterNumber;

  /// `en` or `ru`.
  final String languageCode;

  /// MUS-005: Now Playing's button that opens the system output picker.
  final String audioOutput;

  /// MUS-005: the mini player opens Now Playing (screen readers, TV).
  final String openNowPlaying;

  /// MUS-005: Now Playing's queue button on a phone.
  final String showQueue;

  /// MUS-005: the mini player as a whole (screen readers).
  final String miniPlayer;

  final String back;
  final String back10;
  final String forward10;
  final String play;
  final String pause;
  final String audio;
  final String subtitles;
  final String subtitleFile;

  /// Tooltip prefix of the speed button: "Speed (1.5×)".
  final String speed;
  final String playbackSpeed;
  final String info;
  final String fullscreen;

  /// The OSD button that puts the video in a PiP window (PLAY-AND-008).
  final String pictureInPicture;
  final String close;
  final String noAudioTracks;
  final String off;
  final String externalFile;
  final String loadFromFile;
  final String normalSpeed;
  final String infoTitle;
  final String infoPosition;
  final String cannotOpenFolder;
  final String noSubtitleFiles;
  final String next;
  final String previous;
  final String nowPlaying;
  final String nothingPlaying;
  final String shuffleOn;
  final String shuffleOff;
  final String repeatOff;
  final String repeatAll;
  final String repeatOne;
  final String upNext;
  final String endOfQueue;

  /// Now Playing tabs (MUS-003).
  final String queueTab;
  final String historyTab;

  /// Section of the tracks queued with Play Next / Add to Queue.
  final String playingNext;
  final String clearQueue;
  final String saveAsPlaylist;
  final String removeFromQueue;
  final String dragToReorder;
  final String historyEmpty;

  /// A video paused the music: the "resume?" offer.
  final String musicPaused;

  /// Up Next card: play the next item without waiting.
  final String playNow;

  /// Up Next card: stop the countdown.
  final String cancel;

  /// Subtitles panel: heading of the delay control (PLAY-007).
  final String subtitleDelay;

  /// Delay value when subtitles are not shifted.
  final String subtitleDelayNone;

  /// Tooltip of the button that shows subtitles 0.1 s earlier.
  final String subtitlesEarlier;

  /// Tooltip of the button that shows subtitles 0.1 s later.
  final String subtitlesLater;
  final String resetDelay;

  /// Tag of a forced subtitle track.
  final String forcedTrack;

  /// Tag of an SDH subtitle track.
  final String sdhTrack;

  /// Tag of a commentary track (PLAY-004).
  final String commentaryTrack;

  /// Tag of an audio description track (for the blind).
  final String audioDescriptionTrack;

  /// Audio panel: the audio delay row and its buttons.
  final String audioDelay;
  final String soundEarlier;
  final String soundLater;
  final String audioDelayNone;

  /// Audio panel quick toggles (PLAY-004).
  final String stereoDownmix;
  final String lfeToStereo;
  final String normalizeDownmix;
  final String volumeBoost;
  final String volumeBoostLess;
  final String volumeBoostMore;

  /// Channel layouts of an audio track: 1 and 2 channels; more are "5.1",
  /// "7.1" ([channelsLabel]).
  final String channelsMono;
  final String channelsStereo;

  /// Zoom panel title and button tooltip.
  final String zoom;

  /// Zoom: whole picture with black bars.
  final String zoomFit;

  /// Zoom: fill the screen, crop the overflow.
  final String zoomFill;

  /// Zoom: fill the screen, distort.
  final String zoomStretch;

  /// Resets the pinch zoom.
  final String resetPinch;

  /// Chapters panel title and button tooltip.
  final String chapters;

  final String previousChapter;

  final String nextChapter;

  final String skipIntro;

  final String skipRecap;

  final String skipCredits;

  /// Stats overlay title.
  final String statistics;

  /// Info panel switch of the stats overlay.
  final String showStatistics;

  /// Info panel row: the stats into a diagnostic report (PLAY-010).
  final String exportStats;

  /// Stats overlay: a media server does not keep up (PLAY-010).
  final String transcodeHint;

  final String brightness;

  final String volume;

  /// Badge while the long press plays at 2×.
  final String holdSpeed;

  /// Screen reader label of the video surface: a tap shows or hides the
  /// controls (UI-021).
  final String toggleControls;

  /// Playback error screen (BUG-003, UI-044): retry button, the label of
  /// the stable code under the text, and a title and hint per code.
  final String retry;
  final String errorCode;

  /// The fold with the code and the engine's message (UI-044).
  final String errorDetails;
  final String errorOpenFailed;
  final String errorOpenFailedHint;
  final String errorUnsupported;
  final String errorUnsupportedHint;
  final String errorNothingToPlay;
  final String errorNothingToPlayHint;

  /// Human title of a playback failure.
  String playbackErrorTitle(PlaybackErrorCode code) => switch (code) {
    PlaybackErrorCode.openFailed => errorOpenFailed,
    PlaybackErrorCode.unsupportedFormat => errorUnsupported,
    PlaybackErrorCode.nothingToPlay => errorNothingToPlay,
  };

  /// What the person can do about a playback failure.
  String playbackErrorHint(PlaybackErrorCode code) => switch (code) {
    PlaybackErrorCode.openFailed => errorOpenFailedHint,
    PlaybackErrorCode.unsupportedFormat => errorUnsupportedHint,
    PlaybackErrorCode.nothingToPlay => errorNothingToPlayHint,
  };
  final String Function(int seconds) _skipBackBy;
  final String Function(int seconds) _skipForwardBy;
  final String Function(int seconds) _seconds;
  final String Function(int n) _chapterNumber;

  /// Tooltip of the skip-back button: "Back 10 s".
  String skipBackBy(int seconds) => _skipBackBy(seconds);

  /// Tooltip of the skip-forward button: "Forward 10 s".
  String skipForwardBy(int seconds) => _skipForwardBy(seconds);

  /// Signed seconds of a double-tap seek: "+20 s", "−10 s".
  String seconds(int seconds) => _seconds(seconds);

  /// Name of an untitled chapter: "Chapter 3".
  String chapterNumber(int n) => _chapterNumber(n);
  final String Function(String seconds) _delayLater;
  final String Function(String seconds) _delayEarlier;

  final String Function(String id) _track;
  final String Function(String remaining, String time) _remainingEndsAt;
  final String Function(int seconds) _upNextIn;
  final Map<String, String> _engineInfoKeys;

  /// "-1:02:03 · ends at 21:40".
  String remainingEndsAt(String remaining, String time) =>
      _remainingEndsAt(remaining, time);

  /// Up Next countdown: "Starts in 7 s".
  String upNextIn(int seconds) => _upNextIn(seconds);

  /// Queue section of the source: "Up next from: Abbey Road".
  String upNextFrom(String source) => _upNextFrom(source);
  final String Function(String source) _upNextFrom;

  /// Subtitle delay with an unambiguous sign: "Subtitles 0.3 s later",
  /// "Subtitles 0.3 s earlier", or [subtitleDelayNone].
  String delayLabel(Duration delay) {
    final ms = delay.inMilliseconds;
    if (ms == 0) return subtitleDelayNone;
    var secs = (ms.abs() / 1000).toStringAsFixed(ms.abs() % 100 == 0 ? 1 : 2);
    if (languageCode == 'ru') secs = secs.replaceAll('.', ',');
    return ms > 0 ? _delayLater(secs) : _delayEarlier(secs);
  }

  /// Audio delay with an unambiguous sign: "Sound 0.3 s later", "Sound
  /// 0.3 s earlier", or [audioDelayNone] (PLAY-004).
  String audioDelayLabel(Duration delay) {
    final ms = delay.inMilliseconds;
    if (ms == 0) return audioDelayNone;
    var secs = (ms.abs() / 1000).toStringAsFixed(ms.abs() % 100 == 0 ? 1 : 2);
    if (languageCode == 'ru') secs = secs.replaceAll('.', ',');
    return ms > 0 ? _soundDelayLater(secs) : _soundDelayEarlier(secs);
  }

  final String Function(String) _soundDelayLater;
  final String Function(String) _soundDelayEarlier;

  /// "150 %" (Volume Boost).
  String percent(int value) => languageCode == 'ru' ? '$value %' : '$value%';

  /// "Mono", "Stereo", "5.1", "7.1"; null for an unknown count.
  String? channelsLabel(int? channels) {
    if (channels == null || channels <= 0) return null;
    return switch (channels) {
      1 => channelsMono,
      2 => channelsStereo,
      6 => '5.1',
      8 => '7.1',
      _ => '$channels ch',
    };
  }

  /// Tooltip of the speed button.
  String speedWithRate(String rate) => '$speed ($rate)';

  /// [MediaTrack.label] with the fixed words translated.
  String trackLabel(MediaTrack t) {
    if (t.isOff) return off;
    final parts = <String>[
      if (t.title != null && t.title!.isNotEmpty) t.title!,
      if (t.language != null && t.language!.isNotEmpty) t.language!,
    ];
    final head = parts.isEmpty ? _track(t.id) : parts.join(' · ');
    return t.codec == null ? head : '$head (${t.codec})';
  }

  /// Translates a key of [EngineState.info] ("Resolution", "Video codec"…);
  /// unknown keys are returned as is.
  String engineInfoKey(String key) => _engineInfoKeys[key] ?? key;

  static const en = PaPlayerStrings._(
    languageCode: 'en',
    back: 'Back',
    back10: 'Back 10 s',
    forward10: 'Forward 10 s',
    play: 'Play',
    pause: 'Pause',
    audio: 'Audio',
    subtitles: 'Subtitles',
    subtitleFile: 'Subtitle file',
    speed: 'Speed',
    playbackSpeed: 'Playback speed',
    info: 'Info',
    fullscreen: 'Fullscreen (F)',
    pictureInPicture: 'Picture in picture',
    close: 'Close',
    noAudioTracks: 'No audio tracks',
    off: 'Off',
    externalFile: 'External file',
    loadFromFile: 'Load from file…',
    normalSpeed: 'Normal',
    infoTitle: 'Title',
    infoPosition: 'Position',
    cannotOpenFolder: 'Cannot open folder',
    noSubtitleFiles: 'No subtitle files here',
    next: 'Next',
    previous: 'Previous',
    nowPlaying: 'Now Playing',
    nothingPlaying: 'Nothing playing',
    shuffleOn: 'Shuffle on',
    shuffleOff: 'Shuffle off',
    repeatOff: 'Repeat off',
    repeatAll: 'Repeat all',
    repeatOne: 'Repeat one',
    upNext: 'Up next',
    endOfQueue: 'End of queue',
    queueTab: 'Queue',
    historyTab: 'History',
    playingNext: 'Playing next',
    clearQueue: 'Clear',
    saveAsPlaylist: 'Save as playlist',
    removeFromQueue: 'Remove from queue',
    dragToReorder: 'Drag to reorder',
    historyEmpty: 'Nothing played yet',
    musicPaused: 'Music paused',
    playNow: 'Play now',
    cancel: 'Cancel',
    subtitleDelay: 'Subtitle timing',
    subtitleDelayNone: 'In sync with the video',
    subtitlesEarlier: 'Show subtitles earlier',
    subtitlesLater: 'Show subtitles later',
    resetDelay: 'Reset',
    forcedTrack: 'Forced',
    sdhTrack: 'SDH',
    commentaryTrack: 'Commentary',
    audioDescriptionTrack: 'Audio description',
    audioDelay: 'Audio delay',
    soundEarlier: 'Play sound earlier',
    soundLater: 'Play sound later',
    audioDelayNone: 'In sync',
    stereoDownmix: 'Stereo downmix',
    lfeToStereo: 'Subwoofer (LFE) in stereo',
    normalizeDownmix: 'Normalize downmix',
    volumeBoost: 'Volume Boost',
    volumeBoostLess: 'Less boost',
    volumeBoostMore: 'More boost',
    channelsMono: 'Mono',
    channelsStereo: 'Stereo',
    zoom: 'Zoom',
    zoomFit: 'Fit',
    zoomFill: 'Fill',
    zoomStretch: 'Stretch',
    resetPinch: 'Reset zoom',
    chapters: 'Chapters',
    previousChapter: 'Previous chapter',
    nextChapter: 'Next chapter',
    skipIntro: 'Skip Intro',
    skipRecap: 'Skip Recap',
    skipCredits: 'Skip Credits',
    statistics: 'Statistics',
    showStatistics: 'Show statistics',
    exportStats: 'Save playback report',
    transcodeHint:
        'The network is slower than the stream: choose a lower quality on the server',
    brightness: 'Brightness',
    volume: 'Volume',
    holdSpeed: '2× while held',
    toggleControls: 'Video. Show or hide controls',
    retry: 'Try again',
    errorCode: 'Error code',
    errorDetails: 'Details',
    errorOpenFailed: "Couldn't open the file",
    errorOpenFailedHint:
        'Check that the file is still there and that the network folder '
        'or server is reachable.',
    errorUnsupported: "This file can't be played",
    errorUnsupportedHint:
        "The file is damaged or isn't a video. Try another copy.",
    errorNothingToPlay: 'Neither picture nor sound can be played',
    errorNothingToPlayHint:
        "This device doesn't support the format of the file's tracks.",
    skipBackBy: _enSkipBack,
    skipForwardBy: _enSkipForward,
    seconds: _enSeconds,
    chapterNumber: _enChapter,
    delayLater: _enDelayLater,
    delayEarlier: _enDelayEarlier,
    soundDelayLater: _enSoundLater,
    soundDelayEarlier: _enSoundEarlier,
    track: _enTrack,
    remainingEndsAt: _enEndsAt,
    upNextIn: _enUpNextIn,
    upNextFrom: _enUpNextFrom,
    engineInfoKeys: {},
  );

  static const ru = PaPlayerStrings._(
    languageCode: 'ru',
    back: 'Назад',
    back10: 'Назад на 10 с',
    forward10: 'Вперёд на 10 с',
    play: 'Воспроизвести',
    pause: 'Пауза',
    audio: 'Звук',
    subtitles: 'Субтитры',
    subtitleFile: 'Файл субтитров',
    speed: 'Скорость',
    playbackSpeed: 'Скорость воспроизведения',
    info: 'Информация',
    fullscreen: 'Полный экран (F)',
    pictureInPicture: 'Картинка в картинке',
    close: 'Закрыть',
    noAudioTracks: 'Нет звуковых дорожек',
    off: 'Выкл.',
    externalFile: 'Внешний файл',
    loadFromFile: 'Загрузить из файла…',
    normalSpeed: 'Обычная',
    infoTitle: 'Название',
    infoPosition: 'Позиция',
    cannotOpenFolder: 'Не удалось открыть папку',
    noSubtitleFiles: 'Здесь нет файлов субтитров',
    next: 'Следующий',
    previous: 'Предыдущий',
    nowPlaying: 'Сейчас играет',
    nothingPlaying: 'Ничего не играет',
    shuffleOn: 'Перемешивание включено',
    shuffleOff: 'Перемешивание выключено',
    repeatOff: 'Повтор выключен',
    repeatAll: 'Повтор всех',
    repeatOne: 'Повтор одного',
    upNext: 'Далее',
    endOfQueue: 'Конец очереди',
    queueTab: 'Очередь',
    historyTab: 'История',
    playingNext: 'Далее в очереди',
    clearQueue: 'Очистить',
    saveAsPlaylist: 'Сохранить как плейлист',
    removeFromQueue: 'Убрать из очереди',
    dragToReorder: 'Перетащите, чтобы изменить порядок',
    historyEmpty: 'Пока ничего не играло',
    musicPaused: 'Музыка на паузе',
    playNow: 'Смотреть сейчас',
    cancel: 'Отмена',
    subtitleDelay: 'Синхронизация субтитров',
    subtitleDelayNone: 'Совпадают с видео',
    subtitlesEarlier: 'Показывать субтитры раньше',
    subtitlesLater: 'Показывать субтитры позже',
    resetDelay: 'Сбросить',
    forcedTrack: 'Форсированные',
    sdhTrack: 'Для слабослышащих',
    commentaryTrack: 'Комментарий',
    audioDescriptionTrack: 'Тифлокомментарий',
    audioDelay: 'Задержка звука',
    soundEarlier: 'Звук раньше',
    soundLater: 'Звук позже',
    audioDelayNone: 'Без сдвига',
    stereoDownmix: 'Свести в стерео',
    lfeToStereo: 'Сабвуфер (LFE) в стерео',
    normalizeDownmix: 'Нормализовать сведение',
    volumeBoost: 'Усиление громкости',
    volumeBoostLess: 'Меньше усиления',
    volumeBoostMore: 'Больше усиления',
    channelsMono: 'Моно',
    channelsStereo: 'Стерео',
    zoom: 'Масштаб',
    zoomFit: 'Вписать',
    zoomFill: 'Заполнить',
    zoomStretch: 'Растянуть',
    resetPinch: 'Сбросить увеличение',
    chapters: 'Главы',
    previousChapter: 'Предыдущая глава',
    nextChapter: 'Следующая глава',
    skipIntro: 'Пропустить заставку',
    skipRecap: 'Пропустить пересказ',
    skipCredits: 'Пропустить титры',
    statistics: 'Статистика',
    showStatistics: 'Показывать статистику',
    exportStats: 'Сохранить отчёт о воспроизведении',
    transcodeHint: 'Сеть медленнее потока: выберите на сервере качество пониже',
    brightness: 'Яркость',
    volume: 'Громкость',
    holdSpeed: '2× пока удерживаете',
    toggleControls: 'Видео. Показать или скрыть управление',
    retry: 'Повторить',
    errorCode: 'Код ошибки',
    errorDetails: 'Подробнее',
    errorOpenFailed: 'Не удалось открыть файл',
    errorOpenFailedHint:
        'Проверьте, что файл на месте, а сетевая папка или сервер '
        'доступны.',
    errorUnsupported: 'Этот файл не воспроизводится',
    errorUnsupportedHint:
        'Файл повреждён или это не видео. Попробуйте другую копию.',
    errorNothingToPlay: 'Не удаётся воспроизвести ни изображение, ни звук',
    errorNothingToPlayHint:
        'Это устройство не поддерживает формат дорожек файла.',
    skipBackBy: _ruSkipBack,
    skipForwardBy: _ruSkipForward,
    seconds: _ruSeconds,
    chapterNumber: _ruChapter,
    delayLater: _ruDelayLater,
    delayEarlier: _ruDelayEarlier,
    soundDelayLater: _ruSoundLater,
    soundDelayEarlier: _ruSoundEarlier,
    track: _ruTrack,
    remainingEndsAt: _ruEndsAt,
    upNextIn: _ruUpNextIn,
    upNextFrom: _ruUpNextFrom,
    engineInfoKeys: {
      'Live latency': 'Задержка эфира',
      'Prefetch segments': 'Prefetch-сегменты',
      'Catch-up': 'Догон эфира',
      'Ad break': 'Реклама',
      'File': 'Файл',
      'Resolution': 'Разрешение',
      'Video codec': 'Видеокодек',
      'Frame rate': 'Частота кадров',
      'Pixel format': 'Формат пикселей',
      'HW format': 'Аппаратный формат',
      'Primaries': 'Основные цвета',
      'Transfer': 'Передаточная функция',
      'Audio codec': 'Аудиокодек',
      'Sample rate': 'Частота дискретизации',
      'Channels': 'Каналы',
      'Engine': 'Движок',
      'Dropped frames': 'Пропущено кадров',
      'Video bitrate': 'Битрейт видео',
      'Audio bitrate': 'Битрейт звука',
      'Cache': 'Буфер',
      'Network speed': 'Скорость сети',
      'Hardware decoding': 'Аппаратное декодирование',
      'Rendered fps': 'Кадров в секунду',
      'Display fps': 'Частота экрана',
      'A/V sync': 'Синхронизация A/V',
      // PLAY-010: rows of the stats model and their words.
      'Source': 'Источник',
      'Container': 'Контейнер',
      'Video': 'Видео',
      'Decoder': 'Декодер',
      'Output': 'Вывод',
      'AFR': 'Частота экрана под видео',
      'Decoder drops': 'Пропущено декодером',
      'Audio': 'Звук',
      'Audio path': 'Путь звука',
      'Bitrate': 'Битрейт',
      'Buffer': 'Буфер',
      'Stalls': 'Остановки на загрузку',
      'Hardware': 'Аппаратный',
      'Software': 'Программный',
      'Local file': 'Файл на устройстве',
      'Network share': 'Сетевая папка',
      'Media server': 'Медиасервер',
      'Live stream': 'Прямой эфир',
      'Passthrough': 'Без декодирования (passthrough)',
      'Decoded to PCM': 'Декодируется в PCM',
      'Off': 'Выкл.',
      'Switched': 'Переключена',
      'Matched': 'Уже совпадает',
      'Unsupported': 'Нет подходящего режима',
      // QA-007: the indicator «сеть против битрейта».
      'Network vs bitrate': 'Сеть против битрейта',
      'Enough': 'хватает',
      'Tight': 'впритык',
      'Too slow': 'не хватает',
    },
    audioOutput: 'Вывод звука',
    openNowPlaying: 'Открыть «Сейчас играет»',
    showQueue: 'Показать очередь',
    miniPlayer: 'Мини-плеер',
  );

  static String _enDelayLater(String s) => 'Subtitles $s s later';
  static String _enDelayEarlier(String s) => 'Subtitles $s s earlier';
  static String _ruDelayLater(String s) => 'Субтитры позже на $s с';
  static String _ruDelayEarlier(String s) => 'Субтитры раньше на $s с';
  static String _enSoundLater(String s) => 'Sound $s s later';
  static String _enSoundEarlier(String s) => 'Sound $s s earlier';
  static String _ruSoundLater(String s) => 'Звук позже на $s с';
  static String _ruSoundEarlier(String s) => 'Звук раньше на $s с';
  static String _enTrack(String id) => 'Track $id';
  static String _ruTrack(String id) => 'Дорожка $id';
  static String _enEndsAt(String r, String t) => '-$r · ends at $t';
  static String _ruEndsAt(String r, String t) => '-$r · закончится в $t';
  static String _enUpNextIn(int s) => 'Starts in $s s';
  static String _enSkipBack(int s) => 'Back $s s';
  static String _enSkipForward(int s) => 'Forward $s s';
  static String _ruSkipBack(int s) => 'Назад на $s с';
  static String _ruSkipForward(int s) => 'Вперёд на $s с';
  static String _enSeconds(int s) => '${s < 0 ? '−' : '+'}${s.abs()} s';
  static String _ruSeconds(int s) => '${s < 0 ? '−' : '+'}${s.abs()} с';
  static String _enChapter(int n) => 'Chapter $n';
  static String _ruChapter(int n) => 'Глава $n';
  static String _ruUpNextIn(int s) => 'Начнётся через $s с';
  static String _enUpNextFrom(String s) => 'Up next from: $s';
  static String _ruUpNextFrom(String s) => 'Далее из: $s';

  /// Locales with their own texts.
  static const supportedLocales = [Locale('en'), Locale('ru')];

  /// Texts for [locale] (by language code); English for anything else.
  static PaPlayerStrings forLocale(Locale? locale) =>
      locale?.languageCode == 'ru' ? ru : en;

  /// Add to `MaterialApp.localizationsDelegates` to follow the app's locale.
  static const LocalizationsDelegate<PaPlayerStrings> delegate =
      _PaPlayerStringsDelegate();

  /// Texts for [context]: from [delegate] if registered, else by the ambient
  /// locale, else English.
  static PaPlayerStrings of(BuildContext context) =>
      Localizations.of<PaPlayerStrings>(context, PaPlayerStrings) ??
      forLocale(Localizations.maybeLocaleOf(context));
}

class _PaPlayerStringsDelegate extends LocalizationsDelegate<PaPlayerStrings> {
  const _PaPlayerStringsDelegate();

  // Every locale is "supported": unsupported ones get English, so adding the
  // delegate never breaks an app locale (e.g. a pseudo-locale).
  @override
  bool isSupported(Locale locale) => true;

  @override
  Future<PaPlayerStrings> load(Locale locale) =>
      SynchronousFuture(PaPlayerStrings.forLocale(locale));

  @override
  bool shouldReload(_PaPlayerStringsDelegate old) => false;
}
