<!-- SPDX-License-Identifier: MPL-2.0 -->

# ADR-NNNN: Стилизованные ASS, PGS и VobSub рисует наш libmpv; свой оверлей — только для простого текста

- **Статус:** Предложен
- **Дата:** 2026-11-14
- **Решают:** владелец проекта (мейнтейнер); участники обсуждения — по PR
- **Связано:** PLAY-006, PLAY-007, PLAY-002, PLAY-AND-006, PLAY-APL-010, PLAY-DSK-003; `docs/research/02` §2.6, §3.1; [ADR-0003](0003-native-player-surfaces-engines.md), [ADR-0012](0012-release-0-1-player-media-kit.md), [ADR-0042](0042-subtitle-style-and-sidecars.md)

## Контекст и проблема

PLAY-006: субтитры ASS/SSA должны показываться со своими стилями, положением, поворотом, затуханиями и караоке, с шрифтами из вложений MKV и с запасными шрифтами для кириллицы и CJK; растровые PGS (Blu-ray) и VobSub (DVD) — как картинки; над HDR-видео — с ограниченной яркостью (≈200–300 нит, `docs/research/02` §2.6). Готовность — golden-тесты кадров для ASS (стили, караоке), PGS и VobSub.

Что есть сейчас. Плеер релиза 0.1 — media_kit поверх нашего LGPL libmpv (ADR-0012, PLAY-002). Наш libmpv уже собран с libass 0.17.5 (ISC), FreeType (FTL), HarfBuzz (MIT) и FriBidi (LGPL-2.1+), а FFmpeg — с декодерами `pgssub` и `dvdsub`. Но media_kit по умолчанию выключает рендер субтитров в mpv (`sub-visibility=no`, `sub-ass=no`), и PLAY-007 (ADR-0042) рисует текст `sub-text` своим Flutter-оверлеем: ASS упрощается до текста, PGS и VobSub не видны совсем. Кроме того, libass в наших сборках для Linux и Android собран без системного поставщика шрифтов (`-Dfontconfig=disabled`): ему доступны только шрифты из файла, поэтому ASS без вложенных шрифтов и CJK показались бы «квадратиками».

## Факторы решения

- Полный рендер ASS (стили, `\pos`, `\frz`, `\fad`, `\k`), PGS и VobSub на Linux и Android уже в релизе 0.1, без нового нативного кода в каждом клиенте.
- Только LGPL/пермиссивные компоненты; лицензионные гейты `tools/ffmpeg` и `tools/mpv` проходят.
- Стиль PLAY-007 (системный стиль, размер, шрифт, фон, положение) продолжает работать для текстовых субтитров.
- Проверяемость без GPU и дисплея: golden-кадры в CI.
- Не закрывать путь нативным плеерам (PLAY-AND-006, PLAY-APL-010, PLAY-DSK-003), которым нужен RGBA-оверлей без mpv.

## Рассмотренные варианты

1. **libmpv рисует субтитры сам** (`sub-visibility=yes`, `sub-ass=yes`) в нашей LGPL-сборке — для ASS и растровых дорожек; свой оверлей PLAY-007 — только для простого текста.
2. **Свой рендерер в Rust-ядре** (libass + декодеры FFmpeg → RGBA-оверлей через FRB), который Flutter рисует поверх текстуры media_kit.
3. Оставить как есть до нативных плееров (ASS как текст, PGS/VobSub не видны).

## Решение

Выбран вариант **1**. Всё нужное уже лежит в libmpv, который приложение и так загружает: libass с шейпингом HarfBuzz/FriBidi и вложенными шрифтами MKV, `pgssub`/`dvdsub` из FFmpeg, синхронизация с кадром и масштабирование под окно — это то, что mpv делает много лет. Вариант 2 потребовал бы передавать каждый кадр субтитров через FFI, синхронизировать его с текстурой media_kit по времени и повторять размещение ASS относительно кадра (anamorphic, crop, `ScaledBorderAndShadow`) — ради того же результата, только медленнее и с риском рассинхронизации. Рендерер в ядре понадобится нативным движкам без mpv (Media3, AVFoundation) — это отдельные задачи PLAY-AND-006 и PLAY-APL-010, и golden-кадры отсюда станут для них эталоном.

### Кто рисует какую дорожку

`subtitleRendererFor(track, options)` в `pa_player`:

| Дорожка (`codec` mpv) | Рисует | Стиль |
|---|---|---|
| `hdmv_pgs_subtitle`, `dvd_subtitle`, `dvb_subtitle`, `xsub` | libmpv | картинка как есть |
| `ass`, `ssa` | libmpv | свои стили ASS; `sub-ass-override` по умолчанию mpv (`scale`) |
| `ass`, `ssa` при «Заменять стили ASS» | оверлей PLAY-007 | стиль пользователя, текст без разметки |
| `subrip`, `webvtt`, `mov_text` и прочий текст | оверлей PLAY-007 | стиль пользователя / системный |

`MediaKitEngine` реализует новый необязательный интерфейс `SubtitleRenderingEngine` (как `StatsEngine`): на каждую смену `sid` (наблюдение за свойством mpv — media_kit не видит выбор mpv и `sub-add … select`) он читает `current-tracks/sub/codec` и ставит `sub-visibility` и `sub-ass`; решение публикуется в `EngineState.subtitleRenderer`, и `PlayerScreen` тогда не рисует оверлей (иначе текст ASS был бы виден дважды). `sub-text` mpv обновляет и при `sub-visibility=no`, поэтому путь PLAY-007 не меняется.

### Настройки

В `SubtitleSettings` (экран «Стиль субтитров») два новых поля, старые сохранённые настройки читаются с умолчаниями:

- `overrideAssStyle` («Заменять стили ASS», по умолчанию выкл.) — ASS идёт в оверлей в стиле пользователя;
- `hdrPeakNits` («Яркость поверх HDR-видео», 203 / 250 / 300 нит, по умолчанию 203 — эталонный белый BT.2408) → `sub-hdr-peak` и `image-subs-hdr-peak` mpv. Эти опции действуют там, где mpv композитит в HDR (`vo=gpu-next`, собственное окно PLAY-DSK-003); текстура media_kit — SDR, в ней субтитры и так на белом SDR. Значение хранится сейчас, чтобы HDR-выход взял его без миграции.

### Шрифты

- **Linux:** libass собирается с системным **fontconfig** (`-Dfontconfig=enabled` в `tools/ffmpeg/lib/targets.sh`), который подключается динамически, как libva и libpulse: `libfontconfig.so.1` есть в любом настольном дистрибутиве (от него же зависит GTK). Лицензия fontconfig — MIT-подобная (HPND), совместима с LGPL-сборкой; в `BUILDINFO.txt` он указан как системная библиотека. Так libass находит шрифты по имени из ASS и подбирает запасной шрифт для каждого символа (CJK, арабский и т. д.). Сборочные образы получили `libfontconfig-dev`.
- **Linux, изоляция символов:** libmpv линкуется с `-Wl,--exclude-libs,ALL`. До этого libmpv экспортировал ~880 символов статически вшитых libass/FreeType/HarfBuzz/FriBidi (`FT_*`, `hb_*`…), которые в одном процессе с GTK, pango и fontconfig могли подменить системные копии. Теперь наружу видны только `mpv_*`.
- **Android:** fontconfig нет, а собирать его под NDK (с конфигом, кэшем и первым сканированием `/system/fonts`) — отдельная работа. Вместо этого плагин `pa_player` (метод `subtitleFonts` канала `app.kinolune/captioning`) создаёт `no_backup/subfonts` с **символическими ссылками** на системные Roboto, Noto Sans CJK, Noto Naskh Arabic, Noto Sans Hebrew и Noto Sans Thai (что есть на устройстве), и движок ставит `sub-fonts-dir`. Запасное семейство `sub-font` выбирается по языку дорожки: по умолчанию Noto Sans CJK JP (в нём есть латиница, кириллица, греческий и CJK), SC/TC/KR — для китайского и корейского, Naskh/Hebrew/Thai — для своих письменностей, Roboto — если Noto CJK на устройстве нет. libass берёт это семейство и для отсутствующих шрифтов, и для каждого символа, которого нет в запрошенном шрифте. Цена: libass читает файлы шрифтов из каталога в память (Noto Sans CJK ≈ 20 МБ) при включении ASS- или растровой дорожки.
- Шрифты из вложений MKV работают везде (`embeddedfonts=yes` по умолчанию в mpv).

### Проверка: golden-кадры

`tools/mpv/tests/subtitles/`:

- `media.py` делает тестовые файлы любым `ffmpeg`: ASS из QA-002 (`texts.ass()`: стили, `\pos` + `\frz`, `\fad`, караоке `\k`, строка CJK) с вложенным DejaVu Sans; дорожку PGS (наш `.sup` из `tools/qa/media/pgs.py`, вмуксированный FFmpeg) и ту же дорожку как VobSub (`dvdsub`). Картинка — ровный цвет в FFV1 без потерь, чтобы кадры зависели только от libmpv.
- `golden.py` (только stdlib, `ctypes`) открывает их в libmpv через **программный render API** (`MPV_RENDER_API_TYPE_SW`, без GPU и дисплея), ставит паузу на заданных временах и сравнивает кадр 640×360 с PNG в `testdata/golden/` (допуск — не больше 0,1 % пикселей с отличием больше 40 уровней и среднее отличие меньше 0,5). Восемь кадров: ASS (затухание на середине, реплика + повёрнутая вывеска, караоке на третьем слоге), PGS (событие 1, пауза без субтитров, событие 2), VobSub (события 1 и 3). Пустой кадр там, где должны быть субтитры, — отдельная ошибка.
- В CI (`libmpv.yml`, Linux x86_64) шаг идёт после рантайм-проверки libmpv; при расхождении сохраняются фактические кадры и карты отличий.

Что **не** входит: рендерер в ядре для нативных движков (PLAY-AND-006, PLAY-APL-010), HDR-выход media_kit, поиск субтитров (PLAY-008), DVB-teletext.

### Последствия

- Хорошо: ASS со всеми эффектами, PGS и VobSub видны в 0.1 на Linux и Android; нового нативного кода — одна функция Kotlin.
- Хорошо: golden-кадры ловят регрессии libass/FFmpeg/mpv при обновлении версий в рецепте.
- Хорошо: libmpv на Linux больше не подменяет символы FreeType/HarfBuzz системных библиотек.
- Плохо: стилизованные субтитры рисуются в текстуре видео: настройки PLAY-007 (размер, фон, положение) к ним не применяются — для этого есть «Заменять стили ASS».
- Плохо: на Android libass держит в памяти файлы шрифтов из каталога (≈20–25 МБ с Noto CJK), пока включена ASS- или растровая дорожка.
- Нейтрально: Linux-сборка libmpv теперь зависит от системного `libfontconfig.so.1` (как GTK-приложение в целом).

## Ссылки

- `tools/ffmpeg/lib/targets.sh`, `tools/mpv/lib/mpv.sh`, `tools/{ffmpeg,mpv}/docker/Dockerfile`
- `tools/mpv/tests/subtitles/{media.py,golden.py,testdata/golden/}`, `.github/workflows/libmpv.yml`
- `clients/flutter/packages/pa_player/lib/src/subtitles/subtitle_rendering.dart`, `.../engine/media_kit_engine.dart`, `.../android/src/main/kotlin/app/kinolune/player/PaPlayerPlugin.kt`
- Тесты: `pa_player/test/subtitle_rendering_test.dart`, `app/test/subtitles_test.dart`
