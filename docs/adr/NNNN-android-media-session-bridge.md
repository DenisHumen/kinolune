<!-- SPDX-License-Identifier: MPL-2.0 -->

# ADR-NNNN: Системная медиасессия Android — Media3 `MediaSessionService` с мостом `SimpleBasePlayer` к libmpv; свой audio focus; PiP из `PlayerScreen`

- **Статус:** Предложен
- **Дата:** 2026-11-15
- **Решают:** владелец проекта (мейнтейнер); участники обсуждения — по PR
- **Связано:** `PLAY-AND-008`, `PLAY-AND-002`, `MUS-003`, `MUS-007`, `MUS-011`, `TWI-014`; `docs/research/02-client-stack-playback.md` §5.3, `docs/research/06-twitch-integration.md` §6.4; [ADR-0012](0012-release-0-1-player-media-kit.md), [ADR-0054](0054-twitch-native-player.md), [ADR-0063](0063-music-queue-per-profile-prefs.md)

## Контекст и проблема

Приложение играет через media_kit / libmpv (ADR-0012), а не через Media3 ExoPlayer из PLAY-AND-002. У libmpv на Android нет системной интеграции: музыка не держит процесс в фоне, нет уведомления, экрана блокировки, кнопок гарнитуры, audio focus и PiP. Нужно:

1. настоящий `MediaSessionService` (foreground service `mediaPlayback`) с уведомлением, обложкой и кнопками;
2. Bluetooth- и проводные кнопки, экран блокировки, системные медиаконтролы;
3. audio focus: приглушение, пауза на звонке, пауза при отключении наушников;
4. `enterPictureInPictureMode` для видео и Twitch live, автоматический PiP при уходе из приложения, переживающий поворот.

Исследование 02 §5.3 уже предлагает для своего движка обёртку Media3 `SimpleBasePlayer` (MUS-011).

## Факторы решения

- Лицензия: только MIT / BSD / Apache / MPL, без GPL.
- Движок остаётся libmpv (gapless, ReplayGain, правка плейлиста на месте — MUS-002, MUS-003); второй движок ради уведомления не нужен.
- Одна медиасессия на приложение: видео поверх музыки забирает уведомление и возвращает его при закрытии.
- Задел на Android Auto, Wear OS и `pa-audio` (MUS-011, MUS-013): современный стек Media3, а не `MediaSessionCompat`.
- Проверяемость: логика audio focus тестируется без устройства.

## Рассмотренные варианты

1. **Media3 `MediaSessionService` + свой `SimpleBasePlayer`, который отражает состояние Dart-плеера и отдаёт команды обратно по каналу** (Apache-2.0).
2. Пакет `audio_service` (MIT): Dart API поверх `MediaBrowserServiceCompat` / `MediaSessionCompat` (androidx.media).
3. Свой `MediaSessionCompat` и `NotificationCompat.MediaStyle` без Media3.
4. Перевести воспроизведение на Media3 ExoPlayer (PLAY-AND-002) ради готовой интеграции.

## Решение

Выбран **вариант 1**. Media3 сам ведёт уведомление (`DefaultMediaNotificationProvider`, обложка через `DataSourceBitmapLoader`), foreground service, кнопки (`KEYCODE_MEDIA_*`, двойное нажатие headset hook), экран блокировки и системные контролы; движок остаётся прежним. `audio_service` старше по API (compat-слой, свой изолят для фона), даёт меньше контроля над очередью и плохо стыкуется с Media3 для Auto (02 §5.3). Вариант 3 повторяет то, что Media3 уже делает. Вариант 4 ломает решения ADR-0012 и MUS-002/003.

Конкретика:

- **Dart (`pa_player/lib/src/system/`).** `SystemMediaSession` — единственная сессия приложения со стеком «захватов» (`claim`): показывается и получает команды последний захват, `release` возвращает предыдущий или убирает уведомление (`clear`). `MusicPlayerController` захватывает её, когда что-то заиграло (восстановленная очередь не появляется, пока не нажат Play), и отпускает на `stop`; `PlayerScreen` (фильмы, сериалы, Twitch VOD и live) захватывает на время экрана. Состояние (`MediaSessionInfo`: тип music / video / live, название, исполнитель или канал, альбом, обложка, длительность, позиция, playing, buffering, ended, скорость, есть ли next / previous, можно ли перематывать) отправляется по `app.kinolune/media_session` только при изменении; позиция — при расхождении больше 1,5 с с экстраполяцией системы. Live всегда без перемотки.
- **Kotlin (`pa_player/android`, `app.kinolune.player.media`).** `BridgePlayer : SimpleBasePlayer` строит `State` из снимка Dart: плейлист-окно `[previous] current [next]` с плейсхолдерами соседей, чтобы Media3 предлагал «назад» / «вперёд» ровно тогда, когда они есть в очереди; позиция экстраполируется по `elapsedRealtime`; live — `isDynamic` и `LiveConfiguration`. `handleSetPlayWhenReady` / `handleSeek` / `handleStop` отправляют в Dart `play`, `pause`, `seek`, `next`, `previous`, `stop` (Media3 сам решает «в начало трека или предыдущий» по порогу 3 с, как `MusicPlayerController.previous`). `PlaybackService : MediaSessionService` объявлен в манифесте плагина (`foregroundServiceType="mediaPlayback"`, `exported`), стартует при первом `update` через `MediaController`, останавливается на `clear`. `MediaBridge` — синглтон процесса; сессией владеет движок Flutter, который последним прислал `update`, поэтому фоновый движок workmanager (TWI-009) её не трогает.
- **Audio focus** — свой (`AudioFocus` + чистая `FocusPolicy`, JUnit): libmpv фокус не запрашивает, а `SimpleBasePlayer`, в отличие от ExoPlayer, его не ведёт. Запрос `AUDIOFOCUS_GAIN` с `USAGE_MEDIA` при старте; отказ → пауза; `LOSS` → пауза; `LOSS_TRANSIENT` (звонок, ассистент) → пауза и продолжение по `GAIN`, если пользователь сам ничего не нажал; `LOSS_TRANSIENT_CAN_DUCK` → команда `duck` (музыка — громкость 20, видео — 20 % от текущей), `GAIN` → `unduck`. Запрос с `setWillPauseWhenDucked(true)`: система не приглушает сама поверх нашего. Пока играет — приёмник `ACTION_AUDIO_BECOMING_NOISY` (отключили наушники, пропала Bluetooth-гарнитура) ставит паузу; debug-сборки принимают и `app.kinolune.debug.BECOMING_NOISY` для проверки из `adb`.
- **PiP** — `PipController` и канал `app.kinolune/pip`. Активность объявляет `supportsPictureInPicture` и `resizeableActivity`; `configChanges` уже включает ориентацию и размеры, поэтому поворот не пересоздаёт активность, движок Flutter и поверхность видео. Пока видео играет, `PlayerScreen` включает автоматический вход: `setAutoEnterEnabled` на Android 12+, `onUserLeaveHint` на 8–11; на паузе и при закрытии плеера — выключает. Кнопка PiP в OSD — где PiP поддержан и разрешён (AppOps). В окне PiP рисуются только картинка и субтитры; кнопки окна (play / pause / next) система берёт из нашей медиасессии. Закрытие окна (активность не вернулась на передний план) ставит видео на паузу. `MainActivity.onPictureInPictureModeChanged` передаёт смену режима плагину: `FlutterActivity` не даёт плагинам такого хука.
- **Не входит:** воспроизведение без активности (смахнули из «Недавних» — Dart-движок уходит вместе с `FlutterActivity`, поэтому сервис ставит паузу и останавливается); возобновление после убийства процесса (`MediaButtonReceiver`, `onPlaybackResumption`); browse-дерево для Android Auto (MUS-013); фоновое «только звук» для Twitch (кнопка качества `audio_only` есть, отдельного режима нет).

### Последствия

- Хорошо: один и тот же мост обслуживает музыку, фильмы и Twitch; при переходе на `pa-audio` (MUS-011) меняется только источник состояния.
- Хорошо: зависимость — только `androidx.media3:media3-session`, `-common`, `-datasource` 1.11.1 (Apache-2.0), та же версия, что в `clients/android/player`.
- Плохо: состояние в системе отстаёт от движка на один кадр канала; позиция экстраполируется, поэтому скачки меньше 1,5 с не видны в уведомлении.
- Плохо: смахивание приложения останавливает музыку (в отличие от Spotify). Нужен фоновый движок (отдельная задача после MUS-011).
- Требует работы: CarPlay / iOS (`MPNowPlayingInfoCenter`, PLAY-APL), MPRIS / SMTC на десктопе (PLAY-DSK-006, MUS-007).

### Как проверяем соблюдение

- `flutter test` в `pa_player`: `test/media_session_test.dart` (стек захватов, троттлинг позиции, команды, музыка, видео, PiP, live).
- JUnit плагина: `FocusPolicyTest`, `SessionSnapshotTest` (`./gradlew :pa_player:testDebugUnitTest` из `clients/flutter/app/android`).
- Ручная проверка на эмуляторе: `adb shell input keyevent KEYCODE_MEDIA_*`, `cmd media_session dispatch …`, `adb emu gsm call` (пауза на звонке), поворот в PiP; скриншоты в `docs/screenshots/progress/play-and-008/`.
