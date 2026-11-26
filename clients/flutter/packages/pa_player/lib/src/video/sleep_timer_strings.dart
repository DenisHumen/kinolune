// SPDX-License-Identifier: MPL-2.0
import 'package:flutter/widgets.dart';

import '../l10n/pa_player_strings.dart';
import 'sleep_timer.dart';

/// Texts of the sleep timer and «Вы ещё смотрите?» (PLAY-022). English,
/// Russian and Ukrainian like [PaPlayerStrings], whose locale they follow;
/// the Twitch embed of the app uses them too.
@immutable
class SleepTimerStrings {
  const SleepTimerStrings._(this.languageCode);

  /// `en`, `ru` or `uk`.
  final String languageCode;

  static SleepTimerStrings of(BuildContext context) =>
      SleepTimerStrings._(PaPlayerStrings.of(context).languageCode);

  static const en = SleepTimerStrings._('en');
  static const russian = SleepTimerStrings._('ru');
  static const ukrainian = SleepTimerStrings._('uk');

  String _t(String en, String ru, String uk) => switch (languageCode) {
    'ru' => ru,
    'uk' => uk,
    _ => en,
  };

  bool get _ru => languageCode == 'ru';
  bool get _uk => languageCode == 'uk';

  String get sleepTimer => _t('Sleep timer', 'Таймер сна', 'Таймер сну');
  String get off => _t('Off', 'Выкл.', 'Вимк.');
  String get endOfItem => _t(
    'At the end of this episode or movie',
    'В конце этой серии или фильма',
    'Наприкінці цієї серії або фільму',
  );

  /// "15 min".
  String minutes(int n) => _t('$n min', '$n мин', '$n хв');

  String option(SleepTimerOption o) => switch (o) {
    SleepTimerOption.off => off,
    SleepTimerOption.endOfItem => endOfItem,
    _ => minutes(o.duration!.inMinutes),
  };

  /// "Pauses in 14:32".
  String pausesIn(String time) =>
      _t('Pauses in $time', 'Пауза через $time', 'Пауза через $time');

  /// "Pauses in 30 s" (the fade before the pause).
  String pausesInSeconds(int s) =>
      _t('Pauses in $s s', 'Пауза через $s с', 'Пауза через $s с');

  String get extend => _t('Extend', 'Продлить', 'Подовжити');

  /// Tooltip of «Продлить»: how much it adds.
  String extendBy(int minutes) =>
      _t('Add $minutes min', 'Добавить $minutes мин', 'Додати $minutes хв');

  String get stillWatching =>
      _t('Are you still watching?', 'Вы ещё смотрите?', 'Ви ще дивитеся?');
  String get keepWatching =>
      _t('Keep watching', 'Продолжить просмотр', 'Продовжити перегляд');
  String get pauseNow => _t('Pause', 'Пауза', 'Пауза');

  /// The section of the panel.
  String get askSection => _t(
    '«Are you still watching?»',
    '«Вы ещё смотрите?»',
    '«Ви ще дивитеся?»',
  );

  /// "Ask after 3 episodes in a row".
  String get askAfter => _t(
    'Ask after episodes in a row',
    'Спрашивать после серий подряд',
    'Питати після серій поспіль',
  );

  String get fewer => _t('Fewer', 'Меньше', 'Менше');
  String get more => _t('More', 'Больше', 'Більше');

  /// "3 episodes" / "Never".
  String episodes(int n) {
    if (n == 0) return _t('Never', 'Никогда', 'Ніколи');
    if (_ru) return '$n ${_slavic(n, 'серия', 'серии', 'серий')}';
    if (_uk) return '$n ${_slavic(n, 'серія', 'серії', 'серій')}';
    return n == 1 ? '1 episode' : '$n episodes';
  }

  /// "3 episodes played in a row without a press · pauses in 60 s".
  String askDetail(String episodes, int seconds) => _t(
    '$episodes in a row without a press · pauses in $seconds s',
    '$episodes подряд без нажатий · пауза через $seconds с',
    '$episodes поспіль без натискань · пауза через $seconds с',
  );

  static String _slavic(int n, String one, String few, String many) {
    final m10 = n % 10, m100 = n % 100;
    if (m10 == 1 && m100 != 11) return one;
    if (m10 >= 2 && m10 <= 4 && (m100 < 12 || m100 > 14)) return few;
    return many;
  }
}
