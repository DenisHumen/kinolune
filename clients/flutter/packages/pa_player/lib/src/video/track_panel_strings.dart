// SPDX-License-Identifier: MPL-2.0
import 'package:flutter/widgets.dart';

import '../engine/player_engine.dart';
import '../l10n/pa_player_strings.dart';
import '../subtitles/subtitle_style.dart';

/// Texts of the track panels (UI-014): the Audio · Subtitles · Video tabs,
/// the video tab and the quick subtitle style. English, Russian and
/// Ukrainian (UI-052) like [PaPlayerStrings], whose locale they follow.
@immutable
class TrackPanelStrings {
  const TrackPanelStrings._(this.languageCode);

  /// `en`, `ru` or `uk`.
  final String languageCode;

  static TrackPanelStrings of(BuildContext context) =>
      TrackPanelStrings._(PaPlayerStrings.of(context).languageCode);

  static const en = TrackPanelStrings._('en');
  static const russian = TrackPanelStrings._('ru');
  static const ukrainian = TrackPanelStrings._('uk');

  String _t(String en, String ru, String uk) => switch (languageCode) {
    'ru' => ru,
    'uk' => uk,
    _ => en,
  };

  String get tabAudio => _t('Audio', 'Аудио', 'Аудіо');
  String get tabSubtitles => _t('Subtitles', 'Субтитры', 'Субтитри');
  String get tabVideo => _t('Video', 'Видео', 'Відео');

  /// Semantics label of the tab row.
  String get tracks => _t('Tracks', 'Дорожки', 'Доріжки');

  String get videoTrack => _t('Video track', 'Видеодорожка', 'Відеодоріжка');
  String get noVideoInfo => _t(
    'No video details yet',
    'Сведений о видео пока нет',
    'Відомостей про відео поки немає',
  );

  String get subtitleStyle =>
      _t('Subtitle style', 'Вид субтитров', 'Вигляд субтитрів');
  String get textSize => _t('Text size', 'Размер текста', 'Розмір тексту');
  String get smaller => _t('Smaller', 'Меньше', 'Менше');
  String get larger => _t('Larger', 'Больше', 'Більше');
  String get systemSize => _t('System', 'Системный', 'Системний');
  String get height =>
      _t('Height above the edge', 'Высота над краем', 'Висота над краєм');
  String get lower => _t('Lower', 'Ниже', 'Нижче');
  String get higher => _t('Higher', 'Выше', 'Вище');

  String background(SubtitleBackground b) {
    final value = switch (b) {
      SubtitleBackground.none => _t('none', 'без фона', 'без фону'),
      SubtitleBackground.translucent => _t(
        'translucent',
        'полупрозрачный',
        'напівпрозорий',
      ),
      SubtitleBackground.opaque => _t('solid', 'сплошной', 'суцільний'),
    };
    return _t('Background: $value', 'Фон: $value', 'Фон: $value');
  }

  /// "120 %".
  String percent(num value) => '${value.round()} %';

  /// "1920×1080 · h264 · 23.976 fps · English": what tells video streams
  /// apart.
  String videoLabel(MediaTrack t) {
    final w = t.width, h = t.height, fps = t.fps;
    var rate = fps?.toStringAsFixed(3).replaceFirst(RegExp(r'\.?0+$'), '');
    if (rate != null) rate = _t('$rate fps', '$rate кадр/с', '$rate кадр/с');
    final parts = [
      if (w != null && h != null) '$w×$h',
      ?t.codec,
      ?rate,
      if (t.title != null && t.title!.isNotEmpty) t.title!,
      if (t.language != null && t.language!.isNotEmpty) t.language!,
    ];
    return parts.isEmpty
        ? _t('Track ${t.id}', 'Дорожка ${t.id}', 'Доріжка ${t.id}')
        : parts.join(' · ');
  }
}
