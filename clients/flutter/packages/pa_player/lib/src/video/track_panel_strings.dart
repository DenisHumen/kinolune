// SPDX-License-Identifier: MPL-2.0
import 'package:flutter/widgets.dart';

import '../engine/player_engine.dart';
import '../l10n/pa_player_strings.dart';
import '../subtitles/subtitle_style.dart';

/// Texts of the track panels (UI-014): the Audio · Subtitles · Video tabs,
/// the video tab and the quick subtitle style. Russian and English like
/// [PaPlayerStrings], whose locale they follow.
@immutable
class TrackPanelStrings {
  const TrackPanelStrings._(this.ru);

  final bool ru;

  static TrackPanelStrings of(BuildContext context) =>
      TrackPanelStrings._(PaPlayerStrings.of(context).languageCode == 'ru');

  static const en = TrackPanelStrings._(false);
  static const russian = TrackPanelStrings._(true);

  String get tabAudio => ru ? 'Аудио' : 'Audio';
  String get tabSubtitles => ru ? 'Субтитры' : 'Subtitles';
  String get tabVideo => ru ? 'Видео' : 'Video';

  /// Semantics label of the tab row.
  String get tracks => ru ? 'Дорожки' : 'Tracks';

  String get videoTrack => ru ? 'Видеодорожка' : 'Video track';
  String get noVideoInfo =>
      ru ? 'Сведений о видео пока нет' : 'No video details yet';

  String get subtitleStyle => ru ? 'Вид субтитров' : 'Subtitle style';
  String get textSize => ru ? 'Размер текста' : 'Text size';
  String get smaller => ru ? 'Меньше' : 'Smaller';
  String get larger => ru ? 'Больше' : 'Larger';
  String get systemSize => ru ? 'Системный' : 'System';
  String get height => ru ? 'Высота над краем' : 'Height above the edge';
  String get lower => ru ? 'Ниже' : 'Lower';
  String get higher => ru ? 'Выше' : 'Higher';

  String background(SubtitleBackground b) {
    final value = switch (b) {
      SubtitleBackground.none => ru ? 'без фона' : 'none',
      SubtitleBackground.translucent => ru ? 'полупрозрачный' : 'translucent',
      SubtitleBackground.opaque => ru ? 'сплошной' : 'solid',
    };
    return ru ? 'Фон: $value' : 'Background: $value';
  }

  /// "120 %".
  String percent(num value) => '${value.round()} %';

  /// "1920×1080 · h264 · 23.976 fps · English": what tells video streams
  /// apart.
  String videoLabel(MediaTrack t) {
    final w = t.width, h = t.height, fps = t.fps;
    var rate = fps?.toStringAsFixed(3).replaceFirst(RegExp(r'\.?0+$'), '');
    if (rate != null) rate = ru ? '$rate кадр/с' : '$rate fps';
    final parts = [
      if (w != null && h != null) '$w×$h',
      ?t.codec,
      ?rate,
      if (t.title != null && t.title!.isNotEmpty) t.title!,
      if (t.language != null && t.language!.isNotEmpty) t.language!,
    ];
    return parts.isEmpty
        ? (ru ? 'Дорожка ${t.id}' : 'Track ${t.id}')
        : parts.join(' · ');
  }
}
