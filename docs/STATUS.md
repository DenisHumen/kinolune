# Текущее состояние работ

> Файл для передачи работы между компьютерами и сессиями. Обновляется мейнтейнером после каждой волны/слияния.
> Источник истины по задачам — [ROADMAP.md](ROADMAP.md) (галочки, `just roadmap next`) и открытые PR. Решения — [adr/](adr/).

Обновлено: 2026-11-15, ~02:50 UTC

## Приоритет

**Релиз 0.1 — Kinolune** (ADR-0011, контракт [dev/release-0.1-contract.md](dev/release-0.1-contract.md)): Linux + Android, локальные папки (SMB на Linux — бета), распознавание, медиатека, плеер (media_kit, ADR-0012), «Продолжить просмотр», музыка. Сборка в CI → GitHub Releases по тегу `app-v*`. NAS-дистрибутив — последним.

## Релизы

- **[Kinolune 0.1.0-alpha.1](https://github.com/DenisHumen/project_alpha/releases/tag/app-v0.1.0-alpha.1)** (2026-11-13, пре-релиз): Linux AppImage/tar.gz, Android APK (arm64, armv7, x86_64, universal; debug-подпись). Linux требует системный libmpv.
- **[Kinolune 0.1.0-alpha.2](https://github.com/DenisHumen/project_alpha/releases/tag/app-v0.1.0-alpha.2)** (2026-11-14): наш LGPL libmpv внутри AppImage и APK.
- **[Kinolune 0.1.0-alpha.3](https://github.com/DenisHumen/project_alpha/releases/tag/app-v0.1.0-alpha.3)** (2026-11-14): медиатека на Rust-ядре по умолчанию (импорт базы v0 с резервной копией), метаданные TMDB/AniList/Shikimori в ядре, сессия воспроизведения (resume, Up Next), структура папок (диски, части, extras).
- **[Kinolune 0.1.0-alpha.4](https://github.com/DenisHumen/project_alpha/releases/tag/app-v0.1.0-alpha.4)** (2026-11-14): SMB/WebDAV через ядро (libsmb2 в AppImage и APK), главная, сетка медиатеки с фильтрами, поиск, сопряжение устройств и LAN-синхронизация прогресса, кэш картинок, исправлены отступы Android и окно плеера на Linux.
- **[Kinolune 0.1.0-alpha.5](https://github.com/DenisHumen/project_alpha/releases/tag/app-v0.1.0-alpha.5)** (2026-11-14): страницы фильма и сериала, музыка, OSD плеера с превью перемотки, внешние субтитры, версии и диски, передача просмотра между устройствами, онбординг, настройки, раздел «Стримы» (чат, официальный embed), исправление зависания связки ключей.

## Готово и в main

- Инфраструктура: монорепо, `just`, `just roadmap next|lint`, bootstrap/Dev Container, REUSE, gitleaks, cargo-deny/audit, Dependabot, DCO + линт заголовков PR, метки.
- Ядро (Rust): `pa-model`, `pa-ffi` (UniFFI), `pa-frb` + Dart-пакет `pa_core` (flutter_rust_bridge, Native Assets), `pa-runtime`, `pa-vfs`, `pa-library` (схема SQLite v1), `pa-recognition` (парсер имён, META-005), `pa-anime-mappings`, `pa-eval` + формат корпуса.
- Клиент 0.1: `pa_library` (Dart: сканер, парсер v0, TMDB, SQLite, прогресс), `pa_player` (media_kit: видео-OSD, музыка), `pa_design` (токены, иконки Lucide, шрифт Inter), бренд Kinolune (иконки, баннер TV, заглушки).
- Сборки: LGPL FFmpeg для 11 таргетов (CI), CI выпуска приложения (AppImage/tar.gz, APK), CI образа NAS-ОС с подписью, определение дисков установщика.
- Приложение Kinolune (`clients/flutter/app`, R01-APP) — все экраны 0.1, скриншоты в docs/screenshots/0.1.
- LIB-016: refinery удалён, rusqlite 0.42 (закрыт CVE встроенного SQLite).
- Документы: стратегия, 16 ADR, архитектура, тест-стратегия, генератор тестовых медиа, юридические таблицы, политика конфиденциальности, скриншоты прогресса.

## В работе (агенты, ветки на GitHub)

Фоновые загрузки на Android (NET-019 follow-up). Известная проблема: в main падает golden-тест `twitch_golden_test` «twitch home on tablet» во флейворе `full` — починить следующим.

Процесс: агенты вливают main только в начале; мейнтейнер сливает main при приёме PR скриптом (ARB, FRB-привязки и en_XA перегенерируются на сервере сборки, конфликты миграций/Cargo.toml/ADR — автоматически).

Слито 14–15 ноября (пятая волна): автозапись Twitch независимо от профиля (ADR-0071), META-021 (метаданные музыки MusicBrainz/CAA, ADR-0070), QA-006 (замеры производительности + исправление O(N²) поиска постеров), TWI-019 (записи и загрузки Twitch в приложении, ADR-0069), музыка после смахивания на Android (ADR-0068), NET-019 (офлайн-загрузки, ADR-0067), PLAY-AND-008 (MediaSession, фон, PiP, ADR-0065), TWI-018 (запись эфиров, ADR-0066), TWI-017 (загрузчик VOD, ADR-0064), проверка на реальной записи Twitch (3 бага исправлены), MUS-003 (очередь музыки, ADR-0063), CORE-012 (pa-storage, ADR-0062), PLAY-015 (импорт меток пропуска, ADR-0061), LIB-021 (Blu-ray .mpls, ADR-0060), TWI-024 (фильтры и ввод чата), защита профилей в ядре, UI-015 (профили, ADR-0059), PLAY-011 (пропуск заставок, ADR-0058), TWI-009 (уведомления, ADR-0057), PLAY-006 (ASS/PGS, ADR-0056), LIB-012 (профили, ADR-0055). Видение дизайна v2 — docs/design/vision-v2.md (DSN-017…021, после 100 % функциональности).

Слито 14 ноября (четвёртая волна): TWI-014 (нативный плеер Twitch, ADR-0054), UI-006 (файловый браузер), keychain-фикс (ADR-0053), SYNC-006-фиксы (хаб на новом/том же адресе), TWI-006 (live с низкой задержкой, ADR-0052), TWI-005 (официальный embed, ADR-0051), UI-013 (OSD плеера, ADR-0050), UI-004 (онбординг), TWI-013 (раздел «Стримы», ADR-0049), TWI-012 (чат), PLAY-009 (trickplay, ADR-0048), UI-016 (настройки и «О программе»), TWI-008 (главы VOD, ADR-0047), TWI-011 (эмоуты), META-029 (порядок AniDB), UI-010 (страница сериала, ADR-0046).

Слито 14 ноября (третья волна): LIB-020 (причина «не играет» для ISO, склейка частей и VOB через mpv EDL, ADR-0045), SYNC-003 (передача просмотра между устройствами, ADR-0044), UI-009 (страница фильма), MUS-004 (музыка: главная, медиатека, альбомы, артисты, плейлисты), LIB-017 (диски BDMV/VIDEO_TS/ISO, ADR-0043), PLAY-007 (внешние субтитры и стиль, ADR-0042), LIB-010 (версии, коллекции, избранное, плейлисты), NET-020 (обнаружение серверов в LAN).

Слито 14 ноября (вторая волна): CORE-013 (флейворы store/full, ADR-0041), UI-022-фикс (отступы Android, окно плеера Linux), LIB-019 (libsmb2 в сборках, проверка на Linux и эмуляторе), TWI-004 (воспроизведение Twitch по флагам конфига), UI-019 (сопряжение устройств по QR/коду и LAN-синхронизация прогресса в приложении, ADR-0040), TWI-010 (чат Twitch `pa-twitch-chat`), CORE-014 (удалённый конфиг с подписью minisign, ADR-0039; ключ — за владельцем), UI-011 (поиск), UI-008 (сетка медиатеки), UI-007 (главная), LIB-018 (скан и воспроизведение SMB/WebDAV через `pa-vfs` + loopback-прокси, ADR-0038), CORE-015 (Keystore/Keychain, ADR-0037), LIB-013 (кэш картинок, ADR-0036), LIB-009 (фильтры/сортировки/умные полки), UI-005 (экран «Источники»), LIB-005 (watcher), META-013 (группы + NFO в распознавании; замер на реальном TMDB — META-028).

Первая волна 14 ноября: LIB-007, UI-024 (приложение на Rust-ядре), META-027, PLAY-005, NET-011, LIB-006 (ADR-0034), PLAY-APL-001 (ADR-0035), NET-002/003/004, SYNC-006, CORE-007, TV-001, PLAY-002, PLAY-013, CORE-003.

Сделано за 13–14 ноября (кроме перечисленного выше): pa-vfs, pa-runtime, pa-library (схема, прогресс, «Продолжить», Up Next), pa-scanner, pa-sync (модель, протокол), хаб синхронизации на сервере, pa-secrets, pa-discovery (mDNS), pa-playback (контракт плеера), pa-metadata (TMDB, AniList, Shikimori), парсер 0.2 + индекс тайтлов, корпус v0 (2035 групп), музыка (схема + теги), Twitch (Helix + вход), LGPL libmpv на 7 платформ, xcframework (Apple) и AAR (Android), локализация ru/en, исправления UI по скриншотам.

## Политика CI (важно)

CI на PR не запускается для черновиков; для готовых PR — при открытии / пометке «ready». Проверки перед слиянием — на сервере сборки (fmt, clippy, тесты, cargo-deny, reuse, Flutter). Тяжёлые сборки (FFmpeg, libmpv, образ ОС) — вручную (`gh workflow run <file> --ref <branch>`) или по тегу. Проверки — локально или на сервере сборки перед пометкой PR готовым.

## Как продолжить с другого ПК

1. `git clone https://github.com/DenisHumen/project_alpha && cd project_alpha && just bootstrap check`
2. Прочитать `AGENTS.md`/`CLAUDE.md`, этот файл, `docs/adr/0011`, `docs/dev/release-0.1-contract.md`.
3. `gh pr list` — доделать черновики выше; `just roadmap next` — следующие задачи.
4. Одна задача — одна ветка `feat/<ID>-slug`, `git commit -s`, PR с `[<ID>]`.

## Нужно от владельца

- Адреса для пожертвований (GitHub Sponsors, Open Collective, Liberapay, Ko-fi, Boosty — какие используются) — константа `kDonationLinks` в `clients/flutter/app/lib/branding.dart`; пока пусты, раздел «Поддержать проект» скрыт.
- Хэши GQL для глав Twitch (`VideoPlayer_ChapterSelectButtonVideo`) — в подписанный удалённый конфиг (ADR-0047).

- Ключ minisign для удалённого конфига (`docs/dev/remote-config.md`) — без него приложение работает только на встроенной копии конфига, Twitch GQL выключен.
- Twitch client_id (чат и вход не проверены вживую).

- Ключ TMDB (постеры/описания) — пока приложение работает без него по распознанным названиям.
- Ключ подписи Android (иначе APK подписаны debug-ключом) и cosign-ключ для образа ОС — по готовности к релизу.
