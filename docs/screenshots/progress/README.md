<!-- SPDX-License-Identifier: MPL-2.0 -->
# Kinolune: прогресс в картинках (2026-11-13)

Снимки сделаны с того, что уже есть в репозитории, без правок кода. Видео — синтетический тестовый ролик
(ffmpeg `smptehdbars` + `testsrc2` 1080p30 H.264 с титром «Kinolune»), стороннего контента нет.

| Файл | Что на снимке |
|---|---|
| [`01-desktop-texture-osd.png`](01-desktop-texture-osd.png) | Десктоп-спайк PLAY-DSK-001 (Flutter Linux + libmpv), режим `texture`: видео в текстуре Flutter, поверх — красная плашка Flutter OSD (headless sway в Docker, VAAPI). |
| [`10-androidtv-launcher.png`](10-androidtv-launcher.png) | Android-спайк PLAY-AND-001 на эмуляторе Android TV 14: главный экран с вариантами A–D, AFR, tunneling. |
| [`11-androidtv-variant-b-flutter-osd.png`](11-androidtv-variant-b-flutter-osd.png) | Вариант B (рекомендованный): нативный Media3-плеер на `SurfaceView` + Flutter OSD во втором движке — панель статистики и кнопки поверх видео (эмулятор, программный декодер, кадр в конце ролика). |
| [`20-arch-context.png`](20-arch-context.png) | Архитектура §2: контекст — что где работает (из `docs/dev/architecture.md`). |
| [`21-arch-components.png`](21-arch-components.png) | Архитектура §3: компоненты — клиенты, нативные плееры, привязки ядра, сервер, NAS-ОС. |
| [`22-arch-dataflow.png`](22-arch-dataflow.png) | Архитектура §6: поток данных «скан → распознавание → медиатека → воспроизведение». |
| [`30-recognition-eval-report.png`](30-recognition-eval-report.png) | Отчёт `pa-eval` (бейзлайн `naive`) на 54 примерах `tools/recognition-eval/examples`: точность разбора имён по полям; идентификация 0 %, т. к. кассеты ответов провайдеров ещё нет. |
| [`40-nas-os-login.png`](40-nas-os-login.png) | NAS-ОС (`distro/`, bootc): образ собран `build-image.sh`, диск — `make-disk.sh`, загружен в QEMU/KVM; VGA-консоль с приглашением входа. |
| [`41-nas-os-bootc-status.png`](41-nas-os-bootc-status.png) | Та же VM, вход на консоли: `bootc status` (загружен наш образ), сервисы `alpha-*` (заглушки), Samba, wsdd, Avahi, firewalld активны, `/health` отвечает, SELinux `Enforcing`, зона `home`. |

## Как получено

- **Десктоп**: `CLIPS='kinolune*' MODES=texture BACKENDS=wayland spikes/desktop-player/tools/run-in-docker.sh`, кадр `grim` обрезан до окна 1280×720.
- **Android TV**: `flutter build apk --debug`, AVD `android-tv` API 34 (2 ядра, 2 GB), `adb exec-out screencap -p`.
  Замечания: с `-gpu swiftshader_indirect` главный Flutter-экран снимается чёрным (известный артефакт, см. README спайка),
  поэтому он снят с `-gpu guest`; в этом режиме вариант B на эмуляторе получил ANR при старте второго Flutter-движка
  (известная проблема блокировки главного потока, см. ADR-0008), поэтому снимок B — из прогона со swiftshader.
  Первый кадр в B на эмуляторе — ~48 с (программный декодер, ARM-трансляция), это не характеристика железа.
- **Архитектура**: mermaid-cli 11, тема `dark`.
- **Распознавание**: `cargo run -p pa-eval -- run --recognizer naive --json report.json --md report.md tools/recognition-eval/examples/examples.jsonl`,
  затем `compare` с бейзлайном `abstain` — «регресса нет». Markdown-отчёт отрисован в headless Chrome (разделы «по категориям» и пустая калибровка опущены).
- **NAS-ОС**: `build-image.sh` → локальный реестр → `make-disk.sh` (~5 мин), QEMU 2 vCPU / 3 GiB, `-vga std`;
  снимки через `screendump` монитора QEMU, команды набраны на консоли через `sendkey` (одноразовый пароль root только в этой VM, VM удалена).
- **PLAY-007 (субтитры)**, `play-007/`: debug-APK x86_64 с нашим libmpv на эмуляторе Android 14 (AVD без Google APIs).
  Фильм — `testsrc2` из ffmpeg с sidecar-файлами `*.en.srt` и `*.ru.forced.srt`; локальная папка `/sdcard/Movies` и WebDAV
  (nginx в Docker на сервере сборки, эмулятор ходит через `10.0.2.2`; файлы играют через loopback-прокси).
  Системный стиль задан через `adb shell settings put secure accessibility_captioning_*` (жёлтый моноширинный текст на синем,
  масштаб 1,5); `android-standard-style.png` — субтитры при выключенных системных субтитрах.
- **PLAY-006 (рендер субтитров)**, `play-006/`: тестовое приложение `pa_player/example`, наш libmpv из рецепта
  ветки (`tools/mpv`, Linux — с fontconfig). Видео — `testsrc2` 720p, 60 с; ASS с вложенным DejaVu Sans: реплика,
  вывеска с `\pos` и `\frz`, караоке `\kf`, строка CJK (во вложенном шрифте её нет — запасной шрифт) и стиль с
  невложенным «Arial»; PGS — наш `.sup`, вмуксированный FFmpeg; VobSub — та же дорожка через `dvdsub`.
  `desktop-*` — Linux release-сборка с нашим libmpv в одноразовом контейнере Ubuntu 24.04 под Xvfb 1280×720 (Mesa,
  программный рендер), снимок — XGetImage; `desktop-ass-override.png` — с `PA_ASS_OVERRIDE=1` (ASS простым текстом
  в стиле PLAY-007). `android-*` — debug-APK x86_64 с нашим libmpv на эмуляторе Android 14 в альбомной ориентации;
  запасной шрифт — системный Noto Sans CJK через `sub-fonts-dir`.
- **UI-013 (OSD плеера)**, `ui-013/`: тестовое приложение `pa_player/example` поверх настоящего видео (`testsrc2` 720p,
  3 мин, четыре главы в контейнере MKV). `desktop-*` — Linux-сборка в одноразовом контейнере Ubuntu 24.04 под Xvfb
  (системный libmpv контейнера, только для проверки), `PA_SKIP=intro:42-100 PA_SKIP_STEP=30`; ввод — `xdotool`.
  `android-*` — debug-APK x86_64 с нашим libmpv на эмуляторе Android 14 (720×1440); жесты — `adb shell input swipe/tap`,
  клавиши S, Z и D-pad — `input keyevent`. Pinch на эмуляторе не снят (нужен мультитач), он покрыт widget-тестом.
- **TWI-014 (нативный плеер Twitch)**, `twi-014/`: сборка `full` в демо-режиме с лабораторией вместо Twitch
  (`--dart-define=KINOLUNE_TWITCH_LAB=http://127.0.0.1:18714`). Лаборатория — пример `pa-twitch` `player_lab`:
  live — фейковый low-latency origin TWI-006 (`FakeLiveOrigin`, prefetch-сегменты, рекламный перерыв
  `twitch-stitched-ad` на сегментах 40–47), VOD — непрерывная запись 4 ч 05 мин (`testsrc2` 320×180, сегменты по 6 с),
  чтобы главы и позиция «Продолжить» выдуманного канала попали на шкалу. Чат — выдуманный (`CoreTwitchChat.demo`).
  `desktop-*` — Linux debug-сборка в одноразовом контейнере Ubuntu 24.04 под Xvfb 1600×900 (системный libmpv контейнера,
  без звуковой карты), ввод — `xdotool`. `android-*` — debug-APK x86_64 с нашим libmpv на эмуляторе Android 14
  (720×1440), лаборатория на хосте через `adb reverse`; касания — `adb shell input swipe x y x y 60`
  (`input tap` на этом эмуляторе в плеере не срабатывал). Живой Twitch не проверялся: нет подписанного конфига владельца
  с GQL client id и хэшем `PlaybackAccessToken`.

  Ручной чек-лист (✔ — проверено, ✘ — найдена проблема, — — не проверено):

  | Проверка | Linux (десктоп) | Android (телефон) |
  |---|---|---|
  | Live через прокси ядра: «В ЭФИРЕ», задержка ≈ 2 с в «Низкой задержке» | ✔ `desktop-1` | ✔ `android-5` (0,3 с) |
  | Отставание после паузы: догон 1,10×, кнопка «К эфиру» | ✔ `desktop-2` (кнопка была посередине — исправлено) | — |
  | «К эфиру» возвращает задержку ≈ 2 с | ✔ `desktop-3` | — |
  | «Стабильно»: задержка ≈ 8 с, новый движок | ✔ `desktop-6` | — |
  | Плашка «Реклама · 0:07» и при скрытом OSD; ← во время рекламы не перематывает | ✔ `desktop-9`, `desktop-10` | — |
  | Задержка в панели статистики (`Live latency 7.7 s / 8.0 s`) | ✔ `desktop-7` | — |
  | Качество: варианты usher, «Только звук» | ✔ `desktop-4`, `desktop-5` (в контейнере без звука аудиопоток стоит) | ✔ `android-3`, `android-4` (звук идёт) |
  | Чат: сбоку на десктопе, «театр» поверх видео, уходит при OSD | ✔ `desktop-1`, `desktop-7`, `desktop-8` | — |
  | Чат на телефоне в портрете — снизу | — | ✔ `android-5`; ✘ OSD не помещался в полосу 16:9 — исправлено (полоса выше, до 340 dp), не переснято |
  | VOD: продолжение с позиции «Продолжить», главы на шкале, текущая глава | ✔ (до лаборатории с длинным VOD — позиция за концом) | ✔ `android-1` |
  | VOD: скорости 0,5–3× | — | ✔ `android-2` |
  | Смена качества VOD сохраняет позицию | — | ✔ (`android-3` → `android-4`); скорость сбрасывалась — исправлено, не переснято |
  | Назад из плеера | ✔ | ✔ (один раз приложение закрылось целиком, затем libmpv упал в SIGSEGV при выгрузке; повторить не удалось) |
  | Телефон в ландшафте (чат поверх), планшет, Android TV | — | — (покрыто widget-тестами раскладки) |
  | Живой Twitch (live и VOD) с подписанным конфигом | — | — |
- **PLAY-AND-008 (медиасессия, фон, PiP)**, `play-and-008/`: debug-APK x86_64 (`full`) с нашим libmpv на эмуляторе
  Android 14 (AVD без Google APIs, 720×1440). Медиа — синтетические: альбом из трёх MP3 (синус, обложка `mandelbrot`)
  и фильм `testsrc2` 720p, всё сгенерировано ffmpeg; Twitch — демо-режим с лабораторией `player_lab` (как TWI-014).
  `android-1` — медиа-уведомление с обложкой и кнопками; `android-2` — те же контролы на экране блокировки;
  `android-3` — OSD фильма с кнопкой PiP (справа внизу); `android-4` — окно PiP после ухода на главный экран
  (автоматический вход) с кнопками медиасессии; `android-5` — PiP фильма после поворота в альбомную ориентацию;
  `android-6` — Twitch live в PiP после поворота (чат в окне PiP скрыт).

  Проверено на эмуляторе: `input keyevent KEYCODE_MEDIA_PLAY/PAUSE/PLAY_PAUSE/NEXT/PREVIOUS` и `HEADSETHOOK` (приложение
  на переднем плане и в фоне), `cmd media_session dispatch play|pause|next|previous|play-pause`, кнопки уведомления и
  экрана блокировки; музыка играет с выключенным экраном (сервис `mediaPlayback` на переднем плане, позиция идёт);
  `adb emu gsm call` — пауза на звонке и продолжение после него; SMS-уведомление — приглушение и возврат громкости;
  «отключение наушников» — debug-трансляция `app.kinolune.debug.BECOMING_NOISY` (системную `AUDIO_BECOMING_NOISY`
  оболочка послать не может); PiP: автоматический вход по Home, кнопка в OSD, play/pause из окна PiP, поворот туда и
  обратно (тот же процесс, воспроизведение не прерывается), разворот обратно в плеер, закрытие окна ставит паузу.
  Не проверено: настоящая Bluetooth-гарнитура и AVRCP, Android 8–11 (`onUserLeaveHint`), Android TV.
