<!-- SPDX-License-Identifier: MPL-2.0 -->

# ADR-0163: Звук на Android — passthrough по HDMI и FFmpeg-расширение Media3

- **Статус:** Принят
- **Дата:** 2026-11-25
- **Решают:** мейнтейнер (владелец делегировал, PLAY-AND-004)
- **Связано:** PLAY-AND-004, PLAY-001 (LGPL FFmpeg), PLAY-002 (LGPL libmpv), [ADR-0008](0008-android-player-embedding.md), [ADR-0012](0012-release-0-1-player-media-kit.md) (плеер 0.1 на media_kit), [ADR-0073](0073-track-policy-audio-processing.md) (аудиообработка), [ADR-0119](0119-prebuilt-ffmpeg-libmpv.md) (готовые сборки), [ADR-0147](0147-android-hdr-dolby-vision-afr.md) (HDR и AFR)

## Контекст и проблема

Владелец домашнего кинотеатра ждёт, что ресивер покажет «Dolby Atmos» на TrueHD-дорожке Blu-ray-ремукса, как в Kodi, Plex или Infuse. Для этого звук должен уйти по HDMI **без декодирования** (passthrough, IEC 61937). Если ресивера нет (телефон, телевизор без ARC), тот же файл должен звучать — декодированным в PCM. Есть ресиверы и саундбары, которые принимают не всё (без DTS, без Atmos), и ресиверы, которые с битстримом ведут себя плохо — нужна настройка «всегда декодировать».

На Android у нас два плеера (ADR-0147): **media_kit (libmpv)** — плеер приложения 0.x, и **Media3** (`Media3Engine`, `pa_player_android`) — нативный плеер, в приложение ещё не подключён. Media3 сам умеет passthrough по `AudioCapabilities`, но декодеров TrueHD и DTS в Android почти нигде нет, а официальное FFmpeg-расширение Media3 не публикуется в Maven: его собирают из исходников, и upstream-скрипт линкует FFmpeg **статически** с произвольной конфигурацией.

## Решение

### Одна логика для обоих плееров

`clients/android/audio` (общий Kotlin, как `clients/android/display`):

- `AudioOutputProbe.capabilities` читает, что принимает текущий выход: API 33+ — `AudioManager.getDirectProfilesForAttributes`; API 29–32 — `AudioTrack.isDirectPlaybackSupported` по каждому формату; всегда — `AudioDeviceInfo.getEncodings` выходов HDMI/ARC/eARC/S/PDIF и sticky `ACTION_HDMI_AUDIO_PLUG` (Android TV 7–9). Это тот же источник, что у Media3 `AudioCapabilities`: EDID телевизора или ресивера с учётом системной настройки «Объёмный звук».
- `AudioPassthrough.decide(codec, caps, mode)` — чистая функция с юнит-тестами:

| Дорожка | Выход принимает | Итог |
|---|---|---|
| любая | «Всегда декодировать» | PCM |
| AAC, FLAC, Opus… | — | PCM (нечего передавать) |
| AC-3 / E-AC-3 / TrueHD / DTS / DTS-HD | свой формат | passthrough |
| E-AC-3 + Atmos (JOC) | только E-AC-3 | passthrough как E-AC-3 (тот же битстрим; ресивер без Atmos играет основу 5.1/7.1) |
| DTS-HD MA/HRA | только DTS | passthrough ядра DTS (5.1 лучше, чем PCM на саундбаре) |
| DTS Express | без DTS-HD | PCM (ядра нет) |
| TrueHD | без TrueHD | PCM (в MKV у TrueHD нет AC-3 ядра) |

- `AudioPassthrough.mpvSpdif` переводит возможности выхода в `audio-spdif` mpv (`ac3,eac3,dts,dts-hd,truehd` или часть): mpv выбирает по дорожке теми же правилами (без `dts-hd` DTS-HD идёт ядром).

### Плеер 0.x (media_kit)

Канал `app.kinolune/audio_passthrough` (`AudioOutputChannel` в `pa_player`) отдаёт `audio-spdif` для выхода и настройки. `AudioPassthrough` (Dart) спрашивает его **до открытия плеера** (иначе первая секунда звука пересоздаётся) и передаёт движку через `AudioPassthroughEngine.setAudioPassthrough` (только видео: музыка и книги всегда декодируются — без пересинхронизации ресивера на каждом треке, с громкостью и gapless). AO `audiotrack` mpv отправляет битстрим в `AudioTrack` как IEC 61937. Декодирование того, что не прошло, — наш LGPL FFmpeg внутри libmpv (TrueHD, DTS-HD MA, E-AC-3 → многоканальный PCM; на динамиках телефона Android сводит в стерео).

### Плеер Media3

- `PolicyAudioSink` (обёртка `DefaultAudioSink`) при «Всегда декодировать» отказывает сжатым форматам — оба рендерера декодируют. В «Авто» решает сам Media3 по `AudioCapabilities` (с теми же откатами: E-AC-3 JOC → E-AC-3, DTS-HD → DTS).
- `EXTENSION_RENDERER_MODE_ON`: `FfmpegAudioRenderer` после платформенных — сначала passthrough и аппаратные декодеры, FFmpeg для остального.
- Настройка приходит в запросе `pa_player_android` (`audio_output`: `auto`/`decode`).

### FFmpeg-расширение из нашего LGPL FFmpeg

`tools/media3-ffmpeg` (CI: `libmpv.yml`, шаги Android):

- Исходники Media3 **1.11.1** (та же версия, что зависимости приложения: JNI регистрируется по этим классам) — по одному файлу с закреплённым коммитом и SHA-256 каждого; полный архив androidx/media (~480 МБ) не нужен. Только аудиочасть: `ExperimentalFfmpegVideoRenderer` не берём — видео декодирует MediaCodec.
- `ffmpeg_jni.cc` компилируется NDK r28c (тот же, что у FFmpeg и libmpv) в `libffmpegJNI.so` и **динамически** линкуется с `libavcodec.so`, `libswresample.so`, `libavutil.so` — теми, что APK уже везёт для libmpv. Вторая копия FFmpeg в APK не появляется, а пользователь может заменить LGPL-библиотеки (LGPL §6), как и для libmpv.
- Воспроизводимость: без build id, пути исходников заменены (`-ffile-prefix-map`), libc++ статически (нет `libc++_shared.so`), страницы 16 КБ; одинаковые входы дают одинаковые байты (проверено двумя сборками в разные каталоги), SHA-256 в `BUILDINFO.txt`.
- Лицензионный гейт `check-license.sh` (как GPL-канарейка libmpv): FFmpeg проходит `tools/ffmpeg/check-license.sh --artifact`; `libffmpegJNI.so` нуждается в трёх библиотеках FFmpeg и не определяет ни одного символа FFmpeg и не содержит строк лицензии FFmpeg (нет статической копии); прочие зависимости — только системные NDK; в `libavcodec.so` есть декодеры TrueHD, MLP, DCA (DTS), E-AC-3 и AC-3, а в строке `configure` нет `--disable-decoder*`; Java — только аудиоклассы. В CI канарейки: FFmpeg с GPL-строкой и FFmpeg без TrueHD-декодера должны проваливать гейт.
- **Декодеры в рецепте FFmpeg не перечисляются.** Конфигурация `tools/ffmpeg` отключает только энкодеры и устройства, все декодеры (в том числе truehd, mlp, dca, eac3, ac3 — все LGPL) включены по умолчанию. Явный `--enable-decoder=…` ничего бы не изменил в библиотеке, но сменил бы хеш рецепта и пересобрал все готовые сборки FFmpeg и libmpv (ADR-0119). Вместо этого гейт расширения проверяет наличие нужных декодеров и падает, если их когда-нибудь отключат.
- Результат — каталог (`java/`, `jniLibs/<abi>/libffmpegJNI.so`, `proguard-rules.txt`, лицензии), его берут `clients/android/player` и `pa_player_android` через `-PpaFfmpegExtDir`. Без него сборка работает: TrueHD/DTS тогда только passthrough или платформенный декодер.

### Настройка

«Звук» в разделе «Воспроизведение»: «Авто (passthrough)» (по умолчанию) и «Всегда декодировать». Только на Android (там плеер спрашивает HDMI); на десктопах passthrough не подключён — строка скрыта, чтобы не обещать. Хранение — `playback_audio_output` (`decode`; умолчание не пишется).

### Честная индикация

Статистика показывает то, что **реально** ушло в `AudioTrack`, а не намерение: media_kit — `audio-out-params/format` (`spdif-truehd` и т. п. → «Без декодирования (passthrough)»), Media3 — `AudioTrackConfig.encoding` из `onAudioTrackInitialized` (`PlaybackStats.audioPassthrough`). Ресивер, который отказался от формата, даст «Декодируется в PCM».

## Рассмотренные варианты

- **Статическая сборка расширения upstream-скриптом (`build_ffmpeg.sh`).** Вторая копия FFmpeg в APK (несколько МБ на ABI), своя конфигурация вне гейта PLAY-001, замена LGPL-библиотеки пользователем невозможна без перелинковки. Отклонено.
- **Готовые AAR расширения от третьих лиц (Jellyfin, `nextlib`).** Сборка не наша: конфигурация FFmpeg (иногда с GPL-частями) не проверяется нашим гейтом, версия Media3 может разойтись. Отклонено.
- **Passthrough всегда, без настройки.** Часть ресиверов (и HDMI-переходники) молчит или щёлкает на битстриме; громкость и аудиообработка (ADR-0073) к битстриму не применяются. Нужна «Всегда декодировать».
- **Транскод в AC-3 для ресиверов без TrueHD/DTS (как Kodi «AC3 transcode»).** У нас есть энкодер `ac3`, но это отдельная фича (5.1 из 7.1, задержка); не в объёме задачи.

## Последствия

- Хорошо: Dolby/DTS доходят до ресивера уже в плеере 0.x; одна логика решений с тестами для двух плееров; FFmpeg-расширение без второй копии FFmpeg и под тем же LGPL-гейтом.
- Плохо: на устройствах не проверено — поведение `audiotrack` mpv с HBR-форматами (TrueHD, DTS-HD MA) на конкретных приставках, Atmos на ресивере, переключение дорожек на ходу. При passthrough не работают громкость приложения, сведение в стерео и усиление (ADR-0073) — это свойство битстрима. Media3 в приложение ещё не подключён: расширение пока собирается в CI, но в APK не входит.
- Нейтрально: смена настройки применяется со следующего видео (mpv и Media3 пересоздают аудиовыход при открытии).

## Как проверяем соблюдение

- Юнит-тесты `clients/android/audio/src/test` (`AudioPassthroughTest`, в `just android test`), `pa_player/test/audio_passthrough_test.dart`, `app/test/audio_output_settings_test.dart`, `pa_player_android/test`.
- CI `libmpv.yml`: сборка расширения на каждом Android ABI и гейт `tools/media3-ffmpeg/check-license.sh`, канарейки GPL и «нет декодера».
- Ручной чек-лист — [docs/qa/android-audio-passthrough-checklist.md](../qa/android-audio-passthrough-checklist.md), строки PLAY-170…174 и SET-160 каталога.
