<!-- SPDX-License-Identifier: MPL-2.0 -->

# pa-player-android — движок Media3 для контракта `PlayerEngine`

PLAY-AND-002. Android-библиотека (Kotlin, AndroidX Media3 1.11) с `Media3Engine` — реализацией контракта `PlayerEngine` ядра (ADR-0017) через UniFFI-трейт из `pa-ffi` (`core/crates/pa-ffi/src/playback.rs`). Встраивание в UI — вариант B из ADR-0008: Flutter-плагин [`pa_player_android`](../../flutter/packages/pa_player_android/README.md) с отдельной `PlayerActivity`.

## Как устроено

- **Команды** (`open`, `play`, `pause`, `seek`, `setRate`, `setVolume`, `setRepeat`, `jump`, `setAudioTrack`, `setSubtitleTrack`, `addExternalSubtitle`, `stats`, `closeEngine`) можно звать из любого потока. Аргументы и состояние проверяются сразу (отказ — `EngineException`, ничего не меняется), работа идёт на главном потоке, которому принадлежит `ExoPlayer`.
- **Состояние — свёртка событий** редьюсером ядра `engineStateApply`: `state()` всегда равен свёртке отправленных `EngineEvent`. События уходят с главного потока, кроме `Closed` — его `closeEngine()` отправляет до возврата.
- `close` в UniFFI-трейте называется `closeEngine`: сгенерированные Kotlin-объекты — `AutoCloseable`, их `close()` освобождает handle.
- **Позиция** — тик раз в 200 мс во время воспроизведения, плюс после seek, паузы и переходов.
- **Дорожки**: только поддерживаемые устройством; id — номер дорожки контейнера (MKV), префикс источника Media3 (`0:`, `1:`) отбрасывается, поэтому id не меняются после добавления внешних субтитров. Внешние субтитры — `ext-N`; Media3 не умеет добавлять дорожку в подготовленный элемент, поэтому элемент перезагружается с той же позиции (без `Loading`). Языки Media3 нормализует в BCP 47 (`rus` → `ru`).
- **Конец плейлиста**: `Ended`, плеер ставится на паузу; `play()` начинает элемент сначала.
- **Ошибки** Media3 → `EngineError`: IO (`2xxx`) → `Source`, разбор/декодер/аудио (`3xxx–5xxx`) → `Decoder`, неподдерживаемый формат и DRM → `Unsupported`.
- **Громкость** выше 100 % (boost, PLAY-004) пока не поддерживается: значение ограничивается 100 и об этом сообщается событием.
- **Видео**: `setVideoSurfaceView(SurfaceView)` (своя поверхность SurfaceFlinger — HDR и AFR не ломаются) или `exoPlayer` для Media3 `PlayerView`.
- **Сетевые файлы** (PLAY-AND-003, ADR-0127): `defaultPlayer` читает через `CoreDataSource`. URL loopback-прокси ядра (`http://127.0.0.1:<порт>/<токен>/…`, тот же `streamUri`, что получает любой плеер) сначала уходит в ядро (`streamOpen` через UniFFI): если прокси работает в этой же копии ядра, файл читается позиционно прямо из бэкенда `pa-vfs` (SMB, WebDAV, SFTP/FTP, DLNA…; NFS — когда появится бэкенд), блоками по 512 КиБ, без сокета. Иначе (`NotLocal`, нет библиотеки ядра) тот же URL читается по HTTP через прокси; для этого манифест библиотеки разрешает cleartext только к `127.0.0.1` и `localhost`. Прочие URI (`file://`, `content://`) — как в Media3. `CoreLibrary.preferAppCore()` до первого вызова ядра привязывает UniFFI к `libpa_frb.so` приложения, чтобы Dart и Media3 работали с одной копией ядра (плагин зовёт его сам).
- **Субтитры** (PLAY-AND-006, ADR-0135): `VideoLayout` — `SurfaceView` и `SubtitleOverlay` в рамке по пропорциям видео; `bind(engine)` подключает всё (или `setSubtitleOverlay`). PGS, VobSub, SRT, WebVTT, TTML — cues Media3 в `SubtitleView` (системный стиль субтитров). ASS рисует libass из нашего `libmpv.so` через JNA: `AssRenderer` (текстовый рендерер перед `TextRenderer`) кладёт сырые события Matroska в libass, `AssOverlayView` рендерит картинку для каждого кадра видео (`VideoFrameMetadataListener`) и поднимает её к моменту показа кадра — караоке совпадает с картинкой. Шрифты — системные (Roboto, Noto CJK/Arabic/Hebrew/Thai) и вложения MKV (`MkvAttachments`); внешний `.ass` читается тем же `DataSource`. Без `libmpv.so` SSA рисует Media3 простым текстом.
- Пока не входит: AFR и HDR-режимы (PLAY-AND-005), FFmpeg-аудио и passthrough (PLAY-AND-004), MediaSession/PiP (PLAY-AND-008), туннелирование.

## Сборка и тесты

Ядро берётся из сборки CORE-004 (как в `pa-core-android`): сгенерированные Kotlin-привязки и `libpa_ffi.so` из `paGenDir` (по умолчанию `target/android`).

```bash
tools/bindings/build-aar.sh --debug x86_64   # ядро для эмулятора (или `just core aar` — все ABI)
just android test                            # JUnit: маппинг Media3 → контракт
ANDROID_SERIAL=emulator-5554 just android test-device   # instrumented-тесты на эмуляторе
```

`test-device` сначала генерирует тестовые медиа (`tools/make-fixtures.sh`: H.264 и HEVC MKV 12 с, 320x240, две AAC-дорожки `eng`/`rus`, SRT-субтитры, внешний SRT; для PLAY-AND-006 — `karaoke.mkv` с номером кадра в картинке и ASS-караоке «квадрат на кадр», тот же `karaoke.ass` отдельно и `pgs.mkv` с дорожкой PGS). Нужны ffmpeg с libx264 и libx265 (`FFMPEG` переопределяет команду) и python3; файлы не коммитятся, энкодеры используются только на машине сборки. В тестовый APK кладутся `libmpv.so` и библиотеки FFmpeg из `PA_TEST_JNI_DIR` (по умолчанию jniLibs `media_kit_libs_android_video`, их ставит `clients/flutter/app/tool/native_libs.sh`): из них берётся libass.

Instrumented-тесты:

- `Media3ContractTest` — весь контрактный набор ядра (`pa_playback::contract`, 16 случаев) через `playerEngineContractRun`: Rust гоняет те же случаи, что проходит mock, против `Media3Engine` с видео на настоящем `SurfaceView`.
- `Media3PlaybackTest` — «done when» PLAY-AND-002: H.264 и HEVC MKV воспроизводятся (кадры декодируются и выводятся), seek, переключение аудио и субтитров.
- `CoreDataSourceTest` (PLAY-AND-003) — чтение через ядро совпадает с файлом (диапазоны, конец файла, «нет файла»), откат на HTTP, когда URL не обслуживает эта копия ядра (тот же прокси за другим портом), воспроизведение обоими путями. Нужно ядро с фичей `testing`: `PA_TESTING=1 tools/bindings/build-aar.sh --debug x86_64`.
- `SambaStreamingTest` (PLAY-AND-003) — скорость чтения и воспроизведение высокобитрейтного файла с Samba напрямую и через прокси; запускается только с аргументами `smbServer`, `smbShare`, `smbUser`, `smbPassword`, `smbFile` (иначе пропускается), результаты — в logcat с тегом `PLAYAND003`. Для теста в APK нужен `libsmb2.so`: `-PpaTestJniDir=<каталог>/<abi>/libsmb2.so`.
- `AppCoreTest` (PLAY-AND-003) — привязка к `libpa_frb.so` (`CoreLibrary.preferAppCore`): запускается отдельно с `appCore=1` и `libpa_frb.so` (собранным с `pa-ffi/testing`) в `paTestJniDir`.

- `SubtitleSyncTest` (PLAY-AND-006, «done when») — скриншоты дисплея во время воспроизведения `karaoke.mkv`: номер кадра видео против позиции караоке, расхождение не больше кадра (на эмуляторе — 0 почти во всех скриншотах). `AssSubtitlesTest` — libass через JNA, слоги по кадрам, шрифты из вложений MKV. `SubtitleTracksTest` — картинки PGS в своё время, внешний `.ass` через libass.

Проверено на эмуляторе Android 14 (x86_64, образ `default`, программные декодеры `c2.android.*`/`c2.goldfish.*`); на реальных устройствах — нет.
