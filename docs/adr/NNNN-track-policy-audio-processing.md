<!-- SPDX-License-Identifier: MPL-2.0 -->

# ADR-NNNN: Политика дорожек и обработка звука — по профилю в Preferences, одна память на сериал, фильтры mpv/FFmpeg с ограничителем

- **Статус:** Предложен
- **Дата:** 2026-11-15
- **Решают:** владелец проекта (мейнтейнер); участники обсуждения — по PR
- **Связано:** PLAY-004, PLAY-007, PLAY-006, LIB-012, UI-013, PLAY-DSK-007; [ADR-0042](0042-subtitle-style-and-sidecars.md), [ADR-0055](0055-profiles-parental-filters.md), [ADR-0056](0056-subtitles-rendered-by-libmpv.md), [ADR-0063](0063-music-queue-per-profile-prefs.md)

## Контекст и проблема

PLAY-004 (документ 01 §1.4 «Audio», «Language»): предпочтительные языки звука и субтитров по профилю — Auto, Original (по `original_language` TMDB), свой список; forced и SDH; запоминание выбора на сериал; даунмикс с нормализацией, LFE в стерео, Volume Boost до 300 %, задержка звука с понятным знаком.

Что уже есть: PLAY-007 (ADR-0042) выбирает субтитры (`pickSubtitle`) и хранит выбор и задержку субтитров на сериал (`SubtitleMemory`, ключ `subtitle_series_<id>` в Preferences); ADR-0042 оставил «Auto/Original» и перенос в профиль на PLAY-004. Плеер — media_kit поверх нашей LGPL-сборки libmpv (mpv 0.42, FFmpeg 9.0). Профили (LIB-012, ADR-0055) хранят в ядре только имя, аватар и ограничения; настройки музыки по профилю уже лежат в Preferences с ключом по `syncId` (ADR-0063). `works.original_lang` пишет прогон TMDB, но в Dart он не попадал.

## Факторы решения

- Таблица выбора дорожек проверяется на mock-движке, без libmpv.
- Одна память на сериал для звука и субтитров, старые записи PLAY-007 читаются.
- Только LGPL-фильтры, без форка media_kit и mpv.
- Volume Boost не должен клиппировать; даунмикс не должен терять LFE, если пользователь этого не хочет.
- Минимум изменений в ядре и FRB (горячие файлы).

## Рассмотренные варианты

Где хранить настройки профиля:

1. **Preferences с ключом по `syncId`** (как ADR-0063): `track_policy[_<syncId>]`, `audio_processing[_<syncId>]`.
2. Новые поля в `profiles` ядра — миграция, FRB, синхронизация; пользы сейчас нет.

Обработка звука:

1. **Опции mpv + фильтры FFmpeg:** даунмикс — встроенный фильтр mpv `format=channels=stereo` (libswresample, `audio-normalize-downmix`, `audio-swresample-o=lfe_mix_level`), усиление и ограничитель — `lavfi=[volume,alimiter]` после даунмикса.
2. Свой граф `aresample=ochl=stereo:…` в lavfi — замер показал, что внутри цепочки mpv он всё равно нормализует и иначе обращается с LFE (−24 дБ вместо −6); управлять им нельзя.
3. `volume`/`volume-max` mpv — усиление после пользовательских фильтров, ограничитель поставить некуда, на 300 % синус 0,5 даёт пик 1,5.

## Решение

**Политика** (`TrackPolicy`, pa_player): языки звука и субтитров (`LanguagePreference`: `auto`, `original`, `custom` + список ISO 639-2/B), когда включать субтитры (`SubtitleMode`: `auto` — правило PLAY-007, `always`, `forcedOnly`, `off`) и SDH (`SdhPreference`: `auto` — первыми при включённых системных субтитрах, `prefer`, `avoid`). Языки звука: Auto — язык интерфейса, затем оригинал (оригинал лучше дубляжа на третьем языке); Original — оригинал, затем язык интерфейса; свой список — список. Субтитры: Original показывает субтитры на языке оригинала, но «понятными» остаются языки пользователя, поэтому для японской речи с Original-субтитрами включаются полные японские.

**Выбор звука** (`pickAudio`): запомненный выбор сериала (язык, затем комментарий/тифлокомментарий и название дорожки — так различаются «Дубляж» и «MVO» одного языка) → первый язык политики, среди его дорожек — с флагом default, иначе первая → нет совпадений — default, иначе первая. Комментарии и тифлокомментарии сами не выбираются (флаги из `track-list` mpv — `visual-impaired`, и из названия). **Субтитры** (`pickSubtitle`): к правилу PLAY-007 добавлены режимы, SDH-предпочтение, исключение комментариев и forced сначала на языке звука (forced переводят реплики на язык дорожки). Звук выбирается сразу при загрузке файла, субтитры — после него, по языку выбранного звука.

**Память на сериал** — одна: `SubtitleMemory` стал `TrackMemory` (typedef сохранён) с полями `audio` (`AudioChoice`) и `audioDelay`. Ключи JSON PLAY-007 (`choice`, `delayMs`) не менялись. Память по профилю: у профиля по умолчанию ключ PLAY-007 `subtitle_series_<id>`, у остальных `track_series_<syncId>_<id>`. Автоматический выбор не запоминается — только выбор в панели OSD.

**Обработка звука** (`AudioProcessing`, по профилю; `AudioProcessingEngine` — необязательный интерфейс движка, как `StatsEngine`):

| Настройка | mpv |
|---|---|
| Сведение в стерео: Авто / Всегда стерео | `af=format=channels=stereo` (до ограничителя) |
| Нормализовать сведение | `audio-normalize-downmix=yes` |
| LFE в стерео | `audio-swresample-o=lfe_mix_level=0.707` |
| Volume Boost 100–300 % | `af=…,lavfi=[volume=G,alimiter=limit=0.891:level=0]` |
| Задержка звука (на сериал) | `audio-delay`, плюс = звук позже |

Ограничитель (−1 dBFS, без авто-уровня `level=0`) включается при усилении и при сведении в стерео без нормализации. Замер на нашей libmpv (тест `audio_processing_libmpv_test.dart`, `ao=pcm`, float): тихий синус 0,1 → 200 % даёт +6,0 дБ, 300 % — +9,5 дБ; громкий синус 0,5 на 300 % — пик 0,891; 5.1 с 0,5 во всех каналах кроме LFE — без нормализации пик был бы 1,21, ограничитель держит 0,891, с нормализацией — 0,5; LFE-синус 0,5 в стерео — тишина, с «LFE в стерео» — 0,25 в каждом канале.

**Original-язык** из ядра: `Catalog::original_language(ItemRef)` (фильм, сериал или сериал серии) и `CoreLibrary::original_language` в FRB; в Dart — `LibraryOriginalLanguage`, приложение кладёт его в `PlayRequest.originalLanguage`.

**UI:** Настройки → Воспроизведение → «Языки и звук» (языки, список языков по порядку, режим субтитров, SDH; сведение, нормализация, LFE, усиление); в панели «Звук» OSD — дорожки с каналами и пометками, задержка звука с кнопками «Звук раньше/позже» и подписью «Звук позже на 0,15 с», быстрые переключатели сведения, LFE, нормализации и усиления (они меняют настройку профиля).

### Последствия

- Хорошо: 28 случаев таблицы выбора проверяются на mock-движке; обработка звука измерена на настоящей libmpv; ни миграций, ни правок схемы ядра.
- Хорошо: старые записи PLAY-007 читаются без переноса; профили получают свои языки и память на сериал.
- Плохо: настройки профиля и память на сериал не синхронизируются между устройствами (как и в ADR-0042/0063) — до синхронизации настроек профиля.
- Нейтрально: в режиме «Авто» сведение делает mpv по устройству вывода, ограничитель стоит до него; без нормализации такое сведение может клиппировать, как и раньше. «Всегда стерео» этого лишено.
- Нейтрально: на Apple (`AlphaPlayerKit`) и в нативных движках Android те же поля нужно перевести в свои опции (PLAY-APL, PLAY-AND); в desktop-плеере — PLAY-DSK-007.

## Ссылки

- `clients/flutter/packages/pa_player/lib/src/tracks/track_policy.dart`, `.../engine/audio_processing.dart`, `.../subtitles/subtitle_policy.dart`, `.../engine/media_kit_engine.dart`, `.../video/osd_panels.dart`
- `clients/flutter/app/lib/screens/track_settings_screen.dart`, `.../services/app_services.dart`, `.../screens/playback.dart`
- `core/crates/pa-catalog/src/details.rs` (`original_language`), `core/crates/pa-frb/src/api/library.rs`
- Тесты: `pa_player/test/track_selection_test.dart` (таблица), `track_policy_test.dart`, `audio_panel_test.dart`, `audio_processing_libmpv_test.dart`; `app/test/track_settings_test.dart`; `pa-catalog/tests/details.rs`
