<!-- SPDX-License-Identifier: MPL-2.0 -->
<!-- progress-digest: dd63a217b563d3830dd9e782e30127e5 -->

# Kinolune — прогресс

Обновлено: **2026-11-28 00:37 UTC** · main [`9e4f6f6`](https://github.com/DenisHumen/kinolune/commit/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a) · страница собирается сама из `docs/ROADMAP.md` и открытых PR ([как](https://github.com/DenisHumen/kinolune/blob/main/tools/roadmap/README.md#страница-прогресса)).

- **Kinolune** `▓▓▓▓▓▓▓▓▓▓░░░░░░░░░░ 51% (291/568)` · весь план без багов
- **Система** `▓▓▓▓▓▓▓▓▓▓▓░░░░░░░░░ 57% (291/510)` · по объёму 54%
- **Эксперименты** `░░░░░░░░░░░░░░░░░░░░ 0% (0/9)` · M11, M12
- **Подписка** `░░░░░░░░░░░░░░░░░░░░ 0% (0/23)` · M13
- **Социальное** `░░░░░░░░░░░░░░░░░░░░ 0% (0/7)` · M15
- **Экосистема** `░░░░░░░░░░░░░░░░░░░░ 0% (0/13)` · M16
- **Диски** `░░░░░░░░░░░░░░░░░░░░ 0% (0/6)` · M14
- **Баги** `▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓░░░ 83% (30/36)` · открыто: 6
- **Проверки** `░░░░░░░░░░░░░░░░░░░░ 1% (58/7965)` · ячейки каталога на устройствах; только автотесты: 2754
- **Прогноз** ≈ 26 дн. (≈ 24.12.2026) при скорости 11.5 задач/день за 4 дн.

| ⚪ не начато | 🔵 в работе | 🟡 сделано, не проверено | ✅ готово |
|---|---|---|---|
| **283** | **0** | **281** | **40** |

<details><summary><b>Дерево прогресса</b> (как <code>roadmap.py progress</code>)</summary>

```text
Kinolune                         ▓▓▓▓▓▓▓▓░░░░░░░░ 51% (291/568) · весь план
├─ Система                       ▓▓▓▓▓▓▓▓▓░░░░░░░ 57% (291/510) · по объёму 54%
│  ├─ Прогноз                     ≈ 26 дн. (≈ 24.12.2026) при скорости 11.5 задач/день за 4 дн.
│  ├─ По размеру                  S — для малого лимита, L — после сброса
│  │  ├─ Маленькие (S)           ▓▓▓▓▓▓▓▓▓▓░░░░░░ 62% (79/127) · готовы к работе: 14
│  │  ├─ Средние (M)             ▓▓▓▓▓▓▓▓▓░░░░░░░ 58% (181/314) · готовы к работе: 46
│  │  └─ Большие (L)             ▓▓▓▓▓▓▓░░░░░░░░░ 45% (31/69) · готовы к работе: 10
│  ├─ По темам
│  │  ├─ Ядро и источники        ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓░ 96% (64/67) · объём 97% · S13/14 M44/46 L7/7
│  │  │  ├─ CORE                 ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓░ 94% (15/16) · S5/6 M10/10
│  │  │  ├─ NET                  ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓░ 92% (22/24) · S3/3 M15/17 L4/4
│  │  │  └─ LIB                  ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓ 100% (27/27) · S5/5 M19/19 L3/3
│  │  ├─ Распознавание           ▓▓▓▓▓▓▓▓▓▓▓▓░░░░ 75% (27/36) · объём 72% · S7/8 M15/21 L5/7
│  │  │  └─ META                 ▓▓▓▓▓▓▓▓▓▓▓▓░░░░ 75% (27/36) · S7/8 M15/21 L5/7
│  │  ├─ Плеер                   ▓▓▓▓▓▓▓░░░░░░░░░ 45% (23/51) · объём 45% · S1/4 M17/35 L5/12
│  │  │  ├─ PLAY                 ▓▓▓▓▓▓▓▓▓▓▓▓▓░░░ 84% (16/19) · S1/1 M12/13 L3/5
│  │  │  ├─ PLAY-AND             ▓▓▓▓▓▓▓▓▓▓░░░░░░ 60% (6/10) · M5/9 L1/1
│  │  │  ├─ PLAY-DSK             ░░░░░░░░░░░░░░░░ 0% (0/9) · S0/1 M0/6 L0/2
│  │  │  └─ PLAY-APL             ▓░░░░░░░░░░░░░░░ 8% (1/13) · S0/2 M0/7 L1/4
│  │  ├─ Интерфейс и ТВ          ▓▓▓▓▓▓▓▓▓░░░░░░░ 57% (54/94) · объём 49% · S16/19 M33/60 L5/15
│  │  │  ├─ UI                   ▓▓▓▓▓▓▓▓▓▓▓▓▓░░░ 82% (47/57) · S12/13 M31/40 L4/4
│  │  │  ├─ TV                   ▓░░░░░░░░░░░░░░░ 7% (1/14) · S1/2 M0/8 L0/4
│  │  │  └─ DSN                  ▓▓▓▓░░░░░░░░░░░░ 26% (6/23) · S3/4 M2/12 L1/7
│  │  ├─ Синхронизация и сервер  ▓▓▓▓▓▓▓▓░░░░░░░░ 51% (20/39) · объём 39% · S4/6 M15/25 L1/8
│  │  │  ├─ SYNC                 ▓▓▓▓▓▓▓▓▓▓▓▓░░░░ 75% (15/20) · S1/1 M13/16 L1/3
│  │  │  └─ SRV                  ▓▓▓▓░░░░░░░░░░░░ 26% (5/19) · S3/5 M2/9 L0/5
│  │  ├─ Музыка                  ▓▓▓▓▓▓▓▓░░░░░░░░ 53% (10/19) · объём 53% · S0/1 M8/14 L2/4
│  │  │  └─ MUS                  ▓▓▓▓▓▓▓▓░░░░░░░░ 53% (10/19) · S0/1 M8/14 L2/4
│  │  ├─ Twitch                  ▓▓▓▓▓▓▓▓▓▓▓▓░░░░ 76% (32/42) · объём 76% · S7/9 M22/29 L3/4
│  │  │  └─ TWI                  ▓▓▓▓▓▓▓▓▓▓▓▓░░░░ 76% (32/42) · S7/9 M22/29 L3/4
│  │  ├─ Дистрибутив             ▓▓▓▓▓░░░░░░░░░░░ 33% (11/33) · объём 37% · S1/2 M7/25 L3/6
│  │  │  └─ DIST                 ▓▓▓▓▓░░░░░░░░░░░ 33% (11/33) · S1/2 M7/25 L3/6
│  │  ├─ Сборка и релизы         ▓▓▓▓▓▓▓░░░░░░░░░ 44% (24/54) · объём 33% · S16/26 M8/26 L0/2
│  │  │  ├─ PKG                  ▓▓▓▓▓░░░░░░░░░░░ 31% (8/26) · S2/7 M6/17 L0/2
│  │  │  ├─ CI                   ▓▓▓▓▓▓░░░░░░░░░░ 38% (5/13) · S5/7 M0/6
│  │  │  └─ REPO                 ▓▓▓▓▓▓▓▓▓▓▓▓░░░░ 73% (11/15) · S9/12 M2/3
│  │  ├─ Качество и тесты        ▓▓▓▓▓▓▓░░░░░░░░░ 42% (8/19) · объём 43% · S1/3 M7/16
│  │  │  └─ QA                   ▓▓▓▓▓▓▓░░░░░░░░░ 42% (8/19) · S1/3 M7/16
│  │  ├─ Документы и право       ▓▓▓▓▓▓░░░░░░░░░░ 35% (17/48) · объём 25% · S12/28 M5/16 L0/4
│  │  │  ├─ DOC                  ▓▓▓▓░░░░░░░░░░░░ 27% (3/11) · S0/3 M3/7 L0/1
│  │  │  ├─ LEGAL                ▓▓▓▓▓▓░░░░░░░░░░ 40% (4/10) · S3/5 M1/5
│  │  │  ├─ SITE                 ▓▓░░░░░░░░░░░░░░ 14% (1/7) · S0/4 M1/2 L0/1
│  │  │  ├─ COMM                 ▓▓▓░░░░░░░░░░░░░ 20% (2/10) · S2/6 M0/2 L0/2
│  │  │  └─ DEC                  ▓▓▓▓▓▓▓▓▓▓▓░░░░░ 70% (7/10) · S7/10
│  │  └─ Заготовки подписки      ▓▓░░░░░░░░░░░░░░ 12% (1/8) · объём 10% · S1/7 M0/1
│  │     └─ SUB                  ▓▓░░░░░░░░░░░░░░ 12% (1/8) · S1/7 M0/1
│  └─ Ждут владельца             16 · DEC-007, DEC-011, DEC-012, REPO-005, REPO-011, DIST-025 …
├─ Эксперименты                  ░░░░░░░░░░░░░░░░ 0% (0/9) · M11, M12
├─ Подписка                      ░░░░░░░░░░░░░░░░ 0% (0/23) · M13
├─ Социальное                    ░░░░░░░░░░░░░░░░ 0% (0/7) · M15, после подписки
├─ Экосистема                    ░░░░░░░░░░░░░░░░ 0% (0/13) · M16, по команде владельца
├─ Диски                         ░░░░░░░░░░░░░░░░ 0% (0/6) · M14, последним
├─ Баги                          ▓▓▓▓▓▓▓▓▓▓▓▓▓░░░ 83% (30/36) · исправлено; открыто: 6 (P0 0 · P1 2 · P2 2)
│  ├─ BUG-002                     [P1] Подвисание в первые 10 секунд первого просмотра по SMB
│  ├─ BUG-027                     [P2] Windows: клик по выбранному разделу не закрыл страницу фильма (BU
│  ├─ BUG-029                     [P1] Linux: встроенный плеер Twitch после рекламной паузы падает с «Th
│  ├─ BUG-032                     [P2] Установщик: «извлеките флешку и перезагрузите» ломает live-систем
│  ├─ BUG-033                     [P3] Установщик: первый экран выбора языка — латиницей вместо кириллиц
│  └─ BUG-034                     [P3] ОС: в интерфейсах видны старые имена проекта и подсказка Cockpit
└─ Проверки                      ░░░░░░░░░░░░░░░░ 1% (58/7965) · на устройствах; только автотесты: 2754 (35%)
   ├─ Android                    ░░░░░░░░░░░░░░░░ 0% (0/1169)
   ├─ Android TV                 ░░░░░░░░░░░░░░░░ 0% (0/1153)
   ├─ iOS                        ░░░░░░░░░░░░░░░░ 0% (0/1032)
   ├─ iPadOS                     ░░░░░░░░░░░░░░░░ 0% (0/1038)
   ├─ macOS                      ░░░░░░░░░░░░░░░░ 0% (0/1185)
   ├─ Windows                    ▓░░░░░░░░░░░░░░░ 5% (56/1168)
   ├─ Linux                      ░░░░░░░░░░░░░░░░ 0% (2/1208)
   └─ tvOS                       ░░░░░░░░░░░░░░░░ 0% (0/12)
```

</details>

## По темам

<details><summary><b>Ядро и источники</b> <code>▓▓▓▓▓▓▓▓▓▓ 96% (64/67)</code> · ⚪ 3 · 🔵 0 · 🟡 64 · ✅ 0</summary>

_CORE, NET, LIB_

- 🟡 [**CORE-001**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L326) `pa-ffi`: каркас UniFFI · S · M0 · нет строк в каталоге проверок
- 🟡 [**CORE-002**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L330) flutter_rust_bridge: пакет `pa_core` · M · M0 · нет строк в каталоге проверок
- 🟡 [**CORE-003**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L334) Apple: xcframework и SwiftPM `PACore` · M · M0 · нет строк в каталоге проверок
- 🟡 [**CORE-004**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L338) Android: AAR `pa-core-android` · S · M0 · нет строк в каталоге проверок
- 🟡 [**CORE-005**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L342) `pa-model`: доменные типы · M · M0 · нет строк в каталоге проверок
- 🟡 [**CORE-006**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L346) `pa-runtime`: жизненный цикл и конфигурация · M · M0 · нет строк в каталоге проверок
- 🟡 [**CORE-007**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L350) `pa-secrets`: хранилище секретов · M · M1 · нет строк в каталоге проверок
- 🟡 [**CORE-008**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L354) `pa-log`, диагностический отчёт, crash-отчёты по opt-in · M · M1 · нет строк в каталоге проверок
- 🟡 [**CORE-009**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L358) Шина событий и модель ошибок · S · M0 · нет строк в каталоге проверок
- ⚪ [**CORE-010**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L362) Smoke-приложения M0 на 7 платформах · S · M0
- 🟡 [**CORE-011**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L366) `pa-jobs`: персистентная очередь задач · M · M1 · нет строк в каталоге проверок
- 🟡 [**CORE-012**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L370) `pa-storage`: места хранения, квоты, retention · M · M4 · нет строк в каталоге проверок
- 🟡 [**CORE-013**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L374) Флейворы сборки `store` и `full` · M · M1 · нет строк в каталоге проверок
- 🟡 [**CORE-014**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L378) Клиент удалённого конфига без серверов · S · M2 · нет строк в каталоге проверок
- 🟡 [**CORE-015**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L382) Платформенные хранилища секретов: Android Keystore и Apple Keychain · M · M1 · нет строк в каталоге проверок
- 🟡 [**CORE-016**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L386) Диагностика: логи сканирования и проверка pa-log на Windows · S · M1 · нет строк в каталоге проверок
- 🟡 [**NET-001**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L393) `pa-vfs`: трейт Vfs и локальная ФС · S · M1 · нет строк в каталоге проверок
- 🟡 [**NET-002**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L397) libsmb2: сборка и `smb2-sys` · M · M1 · нет строк в каталоге проверок
- 🟡 [**NET-003**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L401) SMB2/3 бэкенд · L · M1 · нет строк в каталоге проверок
- 🟡 [**NET-004**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L405) WebDAV бэкенд · M · M1 · нет строк в каталоге проверок
- ⚪ [**NET-005**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L409) NFS v3/v4 бэкенд (libnfs) [P1] · M · M1
- 🟡 [**NET-006**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L413) SFTP и FTP/FTPS бэкенды [P1] · M · M1 · нет строк в каталоге проверок
- 🟡 [**NET-007**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L417) `pa-proxy`: loopback HTTP Range-сервер · S · M1 · нет строк в каталоге проверок
- 🟡 [**NET-008**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L421) Адаптеры ByteSource для FFmpeg и mpv · M · M1 · нет строк в каталоге проверок
- 🟡 [**NET-009**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L425) mDNS/DNS-SD обнаружение · M · M1 · нет строк в каталоге проверок
- 🟡 [**NET-010**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L429) WS-Discovery, NetBIOS, Plex GDM, Jellyfin UDP · M · M4 · нет строк в каталоге проверок
- 🟡 [**NET-011**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L433) Менеджер источников · M · M1 · нет строк в каталоге проверок
- 🟡 [**NET-012**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L437) Plex: вход, серверы, библиотеки · M · M4 · на устройствах ✅ 0/7
- 🟡 [**NET-013**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L441) Plex: воспроизведение, прогресс, транскод по запросу · M · M4 · на устройствах ✅ 0/28
- 🟡 [**NET-014**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L445) Jellyfin: клиент · L · M4 · нет строк в каталоге проверок
- 🟡 [**NET-015**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L449) Emby: клиент · M · M4 · нет строк в каталоге проверок
- 🟡 [**NET-016**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L453) Subsonic/OpenSubsonic клиент [P1] · M · M6 · нет строк в каталоге проверок
- 🟡 [**NET-017**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L457) Облачные хранилища [P1] · L · M4 · на устройствах ✅ 0/70
- 🟡 [**NET-018**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L461) Прочие источники [P2]: UPnP/DLNA, `.strm`, M3U/M3U8 · M · M10 · нет строк в каталоге проверок
- 🟡 [**NET-019**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L465) Офлайн-загрузки из источника на устройство [P1] · M · M4 · нет строк в каталоге проверок
- 🟡 [**NET-020**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L470) Обвязка обнаружения в LAN на Android и Apple · S · M1 · нет строк в каталоге проверок
- 🟡 [**NET-021**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L474) SMB: скорость на каналах с потерями · M · M2 · на устройствах ✅ 0/20
- ⚪ [**NET-022**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L478) Фоновые офлайн-загрузки на iOS [P1] · M · M5
- 🟡 [**NET-023**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L483) Запись в источники: загрузка, папки, перенос, удаление · L · M3 · нет строк в каталоге проверок
- 🟡 [**NET-024**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L487) Постоянная проверка доступности серверов и источников · M · M10 · на устройствах ✅ 0/74
- 🟡 [**LIB-001**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L494) Схема SQLite v1 и миграции · M · M1 · нет строк в каталоге проверок
- 🟡 [**LIB-002**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L498) Политика миграций expand→contract · S · M1 · нет строк в каталоге проверок
- 🟡 [**LIB-003**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L502) `pa-scanner`: поверхностный инкрементальный скан · M · M1 · нет строк в каталоге проверок
- 🟡 [**LIB-004**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L506) Отпечатки файлов: OSHash и ed2k · S · M1 · нет строк в каталоге проверок
- 🟡 [**LIB-005**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L510) Watcher изменений · M · M1 · нет строк в каталоге проверок
- 🟡 [**LIB-006**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L514) `pa-probe`: анализ контейнера · M · M1 · нет строк в каталоге проверок
- 🟡 [**LIB-007**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L518) Структурный анализ папок · M · M1 · нет строк в каталоге проверок
- 🟡 [**LIB-008**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L522) Поиск FTS5 · M · M1 · нет строк в каталоге проверок
- 🟡 [**LIB-009**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L526) Запросы библиотеки: фильтры, сортировки, виды · M · M1 · нет строк в каталоге проверок
- 🟡 [**LIB-010**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L530) Версии, коллекции, избранное, плейлисты · M · M1 · нет строк в каталоге проверок
- 🟡 [**LIB-011**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L534) Прогресс, история, «Продолжить», Up Next · M · M1 · нет строк в каталоге проверок
- 🟡 [**LIB-012**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L538) Профили и фильтры родительского контроля (данные) · M · M1 · нет строк в каталоге проверок
- 🟡 [**LIB-013**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L542) Кэш изображений · S · M1 · нет строк в каталоге проверок
- 🟡 [**LIB-014**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L546) Экспорт и импорт знаний: NFO и ID в имени [P1] · M · M2 · на устройствах ✅ 0/35
- 🟡 [**LIB-015**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L550) Импорт медиатеки pa_library v0 (релиз 0.1) · M · M1 · нет строк в каталоге проверок
- 🟡 [**LIB-016**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L555) Убрать уязвимый SQLite: refinery → rusqlite_migration · S · M1 · нет строк в каталоге проверок
- 🟡 [**LIB-017**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L559) Образы дисков в скане медиатеки · S · M2 · нет строк в каталоге проверок
- 🟡 [**LIB-018**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L563) Скан и воспроизведение сетевых источников через `pa-vfs` · L · M1 · нет строк в каталоге проверок
- 🟡 [**LIB-019**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L567) Сетевые источники в собранном приложении: libsmb2 в сборках и проверка на устройствах · M · M1 · нет строк в каталоге проверок
- 🟡 [**LIB-020**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L571) Диски и составные фильмы в приложении: причина «не играет» и склейка частей · M · M2 · нет строк в каталоге проверок
- 🟡 [**LIB-021**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L575) Blu-ray по плейлисту `.mpls` и субтитры частей · M · M2 · нет строк в каталоге проверок
- 🟡 [**LIB-022**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L579) Пользовательские категории, теги и оценки элементов · M · M3 · нет строк в каталоге проверок
- 🟡 [**LIB-023**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L583) Быстрое распознавание большой сетевой медиатеки без зависаний · L · M2 · нет строк в каталоге проверок
- 🟡 [**LIB-024**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L587) Автоматическое распознавание без кнопки «Сканировать» · M · M2 · нет строк в каталоге проверок
- 🟡 [**LIB-025**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L592) Тип содержимого определяется сам: фильмы, сериалы, музыка, аудиокниги, свои видео · M · M3 · нет строк в каталоге проверок
- 🟡 [**LIB-027**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L601) Закинуть файл прямо через приложение · M · M10 · нет строк в каталоге проверок
- 🟡 [**LIB-028**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L605) Закинуть файл — и он сразу виден на всех устройствах: серая карточка с живым прогрессом · L · M10 · на устройствах ✅ 0/7

</details>

<details><summary><b>Распознавание</b> <code>▓▓▓▓▓▓▓▓░░ 75% (27/36)</code> · ⚪ 9 · 🔵 0 · 🟡 27 · ✅ 0</summary>

_META_

- 🟡 [**META-001**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L613) Таксономия, гайд по разметке, формат корпуса · S · M0 · нет строк в каталоге проверок
- 🟡 [**META-002**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L617) Корпус v0: 2 000 групп · L · M2 · нет строк в каталоге проверок
- 🟡 [**META-003**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L621) Eval-раннер `pa-eval` · M · M2 · нет строк в каталоге проверок
- ⚪ [**META-004**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L625) Бейзлайны · M · M2
- 🟡 [**META-005**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L629) Парсер имён: токенизатор и N-best гипотезы · L · M1 · нет строк в каталоге проверок
- 🟡 [**META-006**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L633) Правила для аниме и русских релизов · M · M2 · нет строк в каталоге проверок
- 🟡 [**META-007**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L637) Нормализация и локальный индекс названий · L · M2 · нет строк в каталоге проверок
- 🟡 [**META-008**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L641) Встроенные маппинги аниме · M · M2 · нет строк в каталоге проверок
- 🟡 [**META-009**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L645) Трейт Provider и клиент TMDB · M · M1 · нет строк в каталоге проверок
- 🟡 [**META-010**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L649) Клиенты AniList и Shikimori · M · M2 · нет строк в каталоге проверок
- ⚪ [**META-011**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L653) AniDB: HTTP и UDP с аккаунтом пользователя · M · M2
- 🟡 [**META-012**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L657) Дополнительные провайдеры [P1] · M · M2 · нет строк в каталоге проверок
- 🟡 [**META-013**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L661) Базовый матчер для M1: название + год → TMDB · S · M1 · нет строк в каталоге проверок
- 🟡 [**META-014**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L665) Скорер v0 и совместный вывод по группе · L · M2 · нет строк в каталоге проверок
- 🟡 [**META-015**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L669) Модуль нумерации эпизодов · L · M2 · нет строк в каталоге проверок
- 🟡 [**META-016**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L673) Пороги, состояния матча, объяснения, API инбокса · M · M2 · нет строк в каталоге проверок
- 🟡 [**META-017**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L677) FolderRule и шаблоны релизов из исправлений · M · M2 · нет строк в каталоге проверок
- ⚪ [**META-018**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L681) Обученный и откалиброванный скорер · L · M2
- ⚪ [**META-019**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L685) Корпус v1: 10 000 групп и инструмент разметки · L · M2
- ⚪ [**META-020**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L689) LLM-fallback по opt-in · M · M2
- 🟡 [**META-021**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L694) Музыкальные метаданные · M · M6 · нет строк в каталоге проверок
- ⚪ [**META-022**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L698) Открытый датасет «хэш → ID» в git [P2] · M · M10
- ⚪ [**META-023**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L702) Парсер: замер на тест-кейсах guessit и anitomy в eval-репо и доводка до ≥ 85 % · M · M2
- 🟡 [**META-024**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L706) Адаптер парсера для `pa-eval`: метрики разбора имён · S · M2 · нет строк в каталоге проверок
- ⚪ [**META-025**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L710) Сверка draft-записей корпуса v0 · M · M2
- 🟡 [**META-026**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L714) Скорер кандидатов v0 по локальному индексу: работа, запись франшизы и эпизод · M · M2 · нет строк в каталоге проверок
- 🟡 [**META-027**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L718) Метаданные провайдеров в медиатеке на Rust-ядре · M · M2 · нет строк в каталоге проверок
- ⚪ [**META-028**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L722) Замер распознавания на реальных ответах TMDB · S · M2
- 🟡 [**META-029**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L726) Порядок AniDB у реальных аниме: маппинги Anime-Lists в `episode_mappings` · M · M2 · нет строк в каталоге проверок
- 🟡 [**META-030**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L730) База аниме в приложении с ежедневным обновлением · M · M2 · нет строк в каталоге проверок
- 🟡 [**META-031**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L734) Распознавание аниме по кадрам через trace.moe · M · M2 · нет строк в каталоге проверок
- 🟡 [**META-032**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L738) Распознавание реальных имён: нумерованные папки, мусор сайтов, сиквелы-сезоны, русские названия и постеры · M · M2 · нет строк в каталоге проверок
- 🟡 [**META-033**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L742) Английские и ромадзи-названия при метаданных на русском · S · M2 · нет строк в каталоге проверок
- 🟡 [**META-034**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L747) Проверка распознавания прямо в карточке, без настроек · S · M2 · нет строк в каталоге проверок
- 🟡 [**META-035**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L751) Нумерация, тип и «Забыть правило папки» в «Исправить распознавание» · S · M2 · на устройствах ✅ 0/70
- 🟡 [**META-036**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L755) Причина «На проверку» показывает чистое совпадение названия и поправки отдельно · S · M10 · нет строк в каталоге проверок

</details>

<details><summary><b>Плеер</b> <code>▓▓▓▓▓░░░░░ 45% (23/51)</code> · ⚪ 28 · 🔵 0 · 🟡 23 · ✅ 0</summary>

_PLAY, PLAY-AND, PLAY-DSK, PLAY-APL_

- 🟡 [**PLAY-001**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L762) LGPL-сборка FFmpeg для всех таргетов · L · M0 · нет строк в каталоге проверок
- 🟡 [**PLAY-002**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L766) LGPL-сборка libmpv · M · M1 · нет строк в каталоге проверок
- 🟡 [**PLAY-003**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L770) Контракт `PlayerEngine` и выбор движка · M · M1 · нет строк в каталоге проверок
- 🟡 [**PLAY-004**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L774) Политика дорожек и аудиообработки · M · M1 · нет строк в каталоге проверок
- 🟡 [**PLAY-005**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L778) Сессия воспроизведения · M · M1 · нет строк в каталоге проверок
- 🟡 [**PLAY-006**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L782) Рендер субтитров в ядре: libass, PGS, VobSub · L · M1 · нет строк в каталоге проверок
- 🟡 [**PLAY-007**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L786) Внешние субтитры и стиль · M · M1 · нет строк в каталоге проверок
- 🟡 [**PLAY-008**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L790) Поиск и загрузка субтитров · M · M1 · нет строк в каталоге проверок
- 🟡 [**PLAY-009**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L794) Trickplay-превью · M · M1 · нет строк в каталоге проверок
- 🟡 [**PLAY-010**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L798) Модель stats overlay и адаптивный буфер · M · M1 · нет строк в каталоге проверок
- 🟡 [**PLAY-011**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L802) Пропуск интро и титров [P1] · L · M4 · нет строк в каталоге проверок
- 🟡 [**PLAY-012**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L806) Google Cast и AirPlay [P1] · M · M5 · на устройствах ✅ 0/18
- 🟡 [**PLAY-013**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L810) Наш libmpv в Android-сборке приложения вместо jar media_kit · S · M1 · нет строк в каталоге проверок
- ⚪ [**PLAY-014**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L814) libdovi в сборке libmpv · M · M2
- 🟡 [**PLAY-015**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L818) Импорт маркеров интро и титров Plex и Jellyfin [P2] · M · M4 · нет строк в каталоге проверок
- 🟡 [**PLAY-016**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L822) Второй провайдер субтитров: Subdl или Podnapisi [P1] · M · M4 · на устройствах ✅ 0/63
- ⚪ [**PLAY-017**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L826) Blu-ray и образы дисков: всё играет само · L · M10
- ⚪ [**PLAY-018**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L830) Меню Blu-ray (HDMV) с переключателем «Показывать / Сразу фильм» · L · M10
- 🟡 [**PLAY-022**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L846) Таймер сна и «Вы ещё смотрите?» · M · M10 · на устройствах ✅ 2/82
- 🟡 [**PLAY-AND-001**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L854) Спайк: встраивание плеера во Flutter без потери HDR · M · M0 · нет строк в каталоге проверок
- 🟡 [**PLAY-AND-002**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L858) Плагин `pa_player_android` на Media3 · L · M1 · нет строк в каталоге проверок
- ⚪ [**PLAY-AND-003**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L862) Media3 DataSource поверх ByteSource · M · M1
- ⚪ [**PLAY-AND-004**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L867) Аудио: FFmpeg-расширение и passthrough · M · M1
- 🟡 [**PLAY-AND-005**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L872) HDR, Dolby Vision и AFR · M · M1 · на устройствах ✅ 0/8
- 🟡 [**PLAY-AND-006**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L876) Оверлей субтитров libass и PGS · M · M1 · на устройствах ✅ 0/10
- ⚪ [**PLAY-AND-007**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L880) Запасной движок libmpv на Android · M · M1
- 🟡 [**PLAY-AND-008**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L884) MediaSession, фон и PiP · M · M1 · на устройствах ✅ 0/6
- 🟡 [**PLAY-AND-009**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L888) Интеграция с Android TV [P1] · M · M4 · нет строк в каталоге проверок
- ⚪ [**PLAY-AND-010**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L892) Stats overlay и матрица устройств · M · M1
- ⚪ [**PLAY-DSK-001**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L899) Спайк: libmpv во Flutter-десктопе · M · M0
- ⚪ [**PLAY-DSK-002**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L903) Плагин `pa_player_desktop` · L · M1
- ⚪ [**PLAY-DSK-003**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L907) HDR на десктопе · L · M1
- ⚪ [**PLAY-DSK-004**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L911) HW-декод и passthrough · M · M1
- ⚪ [**PLAY-DSK-005**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L915) Окна, полный экран и ввод · M · M1
- ⚪ [**PLAY-DSK-006**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L919) Системные медиаконтролы · S · M1
- ⚪ [**PLAY-DSK-007**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L923) Субтитры, зум, деинтерлейсинг и stats в mpv · M · M1
- ⚪ [**PLAY-DSK-008**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L927) AFR на десктопе [P2] · M · M10
- ⚪ [**PLAY-DSK-009**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L931) Замеры спайка libmpv на Windows, macOS и реальном Linux-композиторе · M · M1
- 🟡 [**PLAY-APL-001**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L938) `AlphaPlayerKit` и MPVKit (этап 1) · L · M5 · нет строк в каталоге проверок
- ⚪ [**PLAY-APL-002**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L942) Путь AVPlayer · M · M5
- ⚪ [**PLAY-APL-003**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L946) Мост ByteSource для Apple-движков · S · M5
- ⚪ [**PLAY-APL-004**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L950) Flutter-плагин `pa_player_apple` (iOS, iPadOS, macOS) · M · M5
- ⚪ [**PLAY-APL-005**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L954) Свой движок, шаг 1: FFmpeg → VideoToolbox → AVSampleBufferDisplayLayer (SDR) · L · M5
- ⚪ [**PLAY-APL-006**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L958) Свой движок, шаг 2: HDR10, HLG, DV P5 и P8.1 · L · M5
- ⚪ [**PLAY-APL-007**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L962) Свой движок, шаг 3: DV P7 → P8.1 на лету · M · M5
- ⚪ [**PLAY-APL-008**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L966) Аудио в своём движке · M · M5
- ⚪ [**PLAY-APL-009**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L970) AFR на tvOS и macOS · S · M5
- ⚪ [**PLAY-APL-010**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L974) Субтитры в своём движке · M · M5
- ⚪ [**PLAY-APL-011**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L978) Логика Auto и stats overlay на Apple · M · M5
- ⚪ [**PLAY-APL-012**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L982) Матрица устройств Apple · M · M5
- ⚪ [**PLAY-APL-013**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L986) Своя LGPL-2.1 сборка FFmpeg и libmpv для iOS и tvOS вместо MPVKit · L · M5

</details>

<details><summary><b>Интерфейс и ТВ</b> <code>▓▓▓▓▓▓░░░░ 57% (54/94)</code> · ⚪ 40 · 🔵 0 · 🟡 54 · ✅ 0</summary>

_UI, TV, DSN_

- 🟡 [**UI-001**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L993) Каркас Flutter-приложения · M · M0 · нет строк в каталоге проверок
- 🟡 [**UI-002**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L997) Адаптивная навигация · M · M1 · нет строк в каталоге проверок
- ⚪ [**UI-003**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1001) Тема из токенов и шрифты · M · M1
- 🟡 [**UI-004**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1005) Онбординг · M · M1 · нет строк в каталоге проверок
- 🟡 [**UI-005**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1009) Экран «Источники» и формы · M · M1 · нет строк в каталоге проверок
- 🟡 [**UI-006**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1013) Файловый браузер источника · S · M1 · нет строк в каталоге проверок
- 🟡 [**UI-007**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1017) Главная · M · M1 · нет строк в каталоге проверок
- 🟡 [**UI-008**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1021) Сетка библиотеки и контекстные меню · M · M1 · нет строк в каталоге проверок
- 🟡 [**UI-009**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1025) Detail фильма и страницы людей · M · M1 · нет строк в каталоге проверок
- 🟡 [**UI-010**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1029) Detail сериала, сезоны и эпизоды · M · M1 · нет строк в каталоге проверок
- 🟡 [**UI-011**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1033) Поиск · S · M1 · на устройствах ✅ 3/132
- 🟡 [**UI-012**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1037) Инбокс «Нужна проверка» и «Исправить матч» · M · M2 · на устройствах ✅ 1/63
- 🟡 [**UI-013**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1041) OSD плеера · L · M1 · на устройствах ✅ 0/1
- 🟡 [**UI-014**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1045) Панели дорожек и субтитров · M · M1 · нет строк в каталоге проверок
- 🟡 [**UI-015**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1049) Профили и родительский контроль · M · M1 · нет строк в каталоге проверок
- 🟡 [**UI-016**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1053) Настройки и «О программе» · M · M1 · нет строк в каталоге проверок
- 🟡 [**UI-017**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1057) Экран «Состояние» (диагностика) · S · M1 · на устройствах ✅ 2/56
- 🟡 [**UI-018**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1061) Экран загрузок [P1] · M · M4 · на устройствах ✅ 0/278
- 🟡 [**UI-019**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1065) Pairing по QR и коду · M · M3 · нет строк в каталоге проверок
- 🟡 [**UI-020**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1069) Локализация ru и en · M · M1 · нет строк в каталоге проверок
- 🟡 [**UI-023**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1073) Исправить баги со скриншотов дизайна 2026-11-14 · M · M1 · нет строк в каталоге проверок
- 🟡 [**UI-024**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1077) Приложение на Rust-ядре: медиатека, сканер, поиск, прогресс через `pa_core` · L · M2 · нет строк в каталоге проверок
- 🟡 [**UI-021**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1081) Доступность · M · M1 · нет строк в каталоге проверок
- ⚪ [**UI-022**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1085) Особенности платформ · M · M1
- 🟡 [**UI-025**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1089) Личные медиа: своя медиатека без распознавания · L · M3 · нет строк в каталоге проверок
- 🟡 [**UI-026**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1093) Внутренние баннеры уведомлений · M · M3 · нет строк в каталоге проверок
- 🟡 [**UI-027**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1097) Ввод текста на ТВ с телефона · M · M3 · на устройствах ✅ 0/18
- 🟡 [**UI-028**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1102) Простое добавление сетевой папки SMB · M · M3 · нет строк в каталоге проверок
- 🟡 [**UI-029**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1106) SMB на Windows и «Назад» в мастере добавления источника · S · M3 · нет строк в каталоге проверок
- 🟡 [**UI-030**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1110) SMB: вход, когда сервер не отдаёт список общих папок · S · M3 · нет строк в каталоге проверок
- 🟡 [**UI-031**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1114) SMB: сервер не показывает общие папки — ввести имя самому · S · M3 · нет строк в каталоге проверок
- 🟡 [**UI-032**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1118) Личные медиа: превью-кадры вместо плиток · M · M3 · нет строк в каталоге проверок
- 🟡 [**UI-033**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1122) Личные медиа: продолжение просмотра внутри библиотеки · S · M3 · нет строк в каталоге проверок
- 🟡 [**UI-034**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1126) Личные медиа: без выбора типа, «Перенести в Личное» у папки, отметка в списке источников · S · M3 · нет строк в каталоге проверок
- 🟡 [**UI-035**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1131) Файлы как в Nextcloud: загрузка с телефона, перенос, папки · L · M3 · нет строк в каталоге проверок
- ⚪ [**UI-036**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1136) Трейлеры: видео-баннер в карточке, на весь экран по «вверх» на пульте · M · M3
- ⚪ [**UI-054**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1141) Онлайн-трейлер внутри приложения на Linux, Windows и Kinolune OS · S · M10
- 🟡 [**UI-037**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1145) Просмотренность на кадрах серий и постерах: резкая и размытая части · M · M3 · на устройствах ✅ 0/7
- ⚪ [**UI-038**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1150) iOS и iPadOS: раннер Flutter-приложения, Files и ссылки · M · M5
- ⚪ [**UI-039**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1154) Загрузка файлов через браузер (web-upload) [P1] · M · M5
- ⚪ [**UI-040**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1158) Виджеты «Продолжить просмотр» [P2] · M · M5
- 🟡 [**UI-041**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1162) Настройки: обычные и расширенные · M · M10 · на устройствах ✅ 5/741
- 🟡 [**UI-042**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1166) Первый запуск в три шага · M · M10 · на устройствах ✅ 7/154
- 🟡 [**UI-043**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1170) Тихое распознавание · S · M10 · на устройствах ✅ 0/7
- 🟡 [**UI-044**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1174) Понятные ошибки и «Всё ли в порядке» · M · M10 · на устройствах ✅ 0/42
- 🟡 [**UI-045**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1178) Общий и голосовой поиск · M · M10 · на устройствах ✅ 3/132
- 🟡 [**UI-046**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1182) Новое устройство по QR — главный способ · M · M10 · на устройствах ✅ 0/36
- 🟡 [**UI-047**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1186) Режим «просто смотреть» · S · M10 · на устройствах ✅ 0/28
- 🟡 [**UI-049**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1194) Отметить просмотренные серии · S · M3 · нет строк в каталоге проверок
- 🟡 [**UI-052**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1198) Украинский язык интерфейса · M · M3 · на устройствах ✅ 0/7
- 🟡 [**UI-053**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1202) Индикатор доступности и предупреждение о синхронизации · M · M10 · на устройствах ✅ 0/74
- ⚪ [**UI-050**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1207) Пульт в «островке» iPhone (Dynamic Island и Live Activity) · M · M10
- ⚪ [**UI-051**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1219) Пульт в «островке» MacBook (вырез экрана и строка меню) · M · M10
- 🟡 [**UI-055**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1223) Серии сезона одной строкой: стрелки на компьютере, свайп на телефоне, пульт на ТВ · M · M10 · на устройствах ✅ 0/7
- 🟡 [**UI-056**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1233) Общая длительность и «осталось» у сериала, сезона и фильма · S · M10 · нет строк в каталоге проверок
- 🟡 [**UI-057**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1249) Онгоинги: когда выйдет следующая серия · M · M10 · на устройствах ✅ 0/21
- ⚪ [**UI-058**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1253) Первый запуск: «У меня ещё нет сервера» — как поставить серверную часть · M · M10
- 🟡 [**TV-001**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1260) Каркас tvOS-приложения на SwiftUI · S · M0 · нет строк в каталоге проверок
- ⚪ [**TV-002**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1264) Фокус-движок для Android TV во Flutter · L · M1
- ⚪ [**TV-003**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1268) TV-плотность компонентов · M · M1
- ⚪ [**TV-004**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1272) Специфика Android TV и производительность · M · M1
- ⚪ [**TV-005**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1276) Режим Big Picture на десктопе · M · M4
- ⚪ [**TV-006**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1280) tvOS: хранилище и навигационный каркас · M · M5
- ⚪ [**TV-007**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1284) tvOS: Главная, сетка и Detail · L · M5
- ⚪ [**TV-008**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1288) tvOS: источники и pairing с iPhone · M · M5
- ⚪ [**TV-009**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1292) tvOS: OSD плеера поверх AlphaPlayerKit · L · M5
- ⚪ [**TV-010**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1296) tvOS: Top Shelf и иконки · M · M5
- ⚪ [**TV-011**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1300) tvOS: профили, родительский контроль, настройки, «Состояние» · M · M5
- ⚪ [**TV-012**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1304) tvOS: раздел Twitch · L · M7
- ⚪ [**TV-013**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1308) tvOS: раздел «Музыка» · M · M6
- ⚪ [**TV-014**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1312) tvOS: баннеры уведомлений в приложении · S · M5
- 🟡 [**DSN-001**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1906) Инструмент и структура `design/` · S · M0 · нет строк в каталоге проверок
- 🟡 [**DSN-002**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1910) Токены W3C DTCG · M · M0 · нет строк в каталоге проверок
- 🟡 [**DSN-003**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1914) Пайплайн Style Dictionary и контраст-линт · S · M0 · нет строк в каталоге проверок
- 🟡 [**DSN-004**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1918) Набор иконок и конвейер · M · M1 · нет строк в каталоге проверок
- 🟡 [**DSN-005**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1922) Шрифты · S · M1 · нет строк в каталоге проверок
- 🟡 [**DSN-006**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1926) Бренд, иконки приложения, плейсхолдеры · L · M5 · нет строк в каталоге проверок
- ⚪ [**DSN-007**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1931) Иконки источников и сервисов · S · M4
- ⚪ [**DSN-008**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1935) Макеты: TV как референс · L · M1
- ⚪ [**DSN-009**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1939) Макеты: музыка и Twitch · M · M6
- ⚪ [**DSN-010**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1943) Спецификация компонентов · M · M1
- ⚪ [**DSN-011**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1947) [M9] Полировка фокуса и параллакса · M · M9
- ⚪ [**DSN-012**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1951) [M9] Материалы Glass и Liquid Glass, динамический акцент · L · M9
- ⚪ [**DSN-013**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1955) [M9] Движение (motion) · M · M9
- ⚪ [**DSN-014**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1959) Скриншоты для сторов · M · M10
- ⚪ [**DSN-015**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1963) [M9] Аудит визуальной консистентности · M · M9
- ⚪ [**DSN-016**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1967) Бренд: Apple-слои, MSIX, AppStream и иллюстрации пустых состояний · M · M5
- ⚪ [**DSN-017**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1971) Дизайн v2: концепция «одной умной главной» (уровень Netflix/Crunchyroll) · L · M10
- ⚪ [**DSN-018**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1975) Локальные рекомендации и изучение интересов · L · M10
- ⚪ [**DSN-019**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1979) Трейлеры: автопоиск, превью на карточке и внутри, выбор языка · M · M10
- ⚪ [**DSN-020**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1983) Реализация главной v2 во Flutter · L · M10
- ⚪ [**DSN-021**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1987) Полировка v2: анимации, переходы, пустые состояния, доступность · M · M10
- ⚪ [**DSN-022**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1991) Kinolune OS в духе экосистемы Apple: направление и макеты (делать последним) · M · M10
- ⚪ [**DSN-023**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1995) Kinolune OS в духе Apple: тема в образе · L · M10

</details>

<details><summary><b>Синхронизация и сервер</b> <code>▓▓▓▓▓░░░░░ 51% (20/39)</code> · ⚪ 19 · 🔵 0 · 🟡 20 · ✅ 0</summary>

_SYNC, SRV_

- 🟡 [**SYNC-001**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1319) `pa-sync`: модель, HLC, LWW и outbox · M · M3 · нет строк в каталоге проверок
- 🟡 [**SYNC-002**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1323) Протокол синхронизации · M · M3 · нет строк в каталоге проверок
- 🟡 [**SYNC-003**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1327) Эфемерная сессия воспроизведения · M · M3 · нет строк в каталоге проверок
- 🟡 [**SYNC-004**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1331) Сопоставление медиа между устройствами · M · M3 · нет строк в каталоге проверок
- 🟡 [**SYNC-005**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1335) Pairing устройств («семья устройств») · M · M3 · нет строк в каталоге проверок
- 🟡 [**SYNC-006**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1339) P2P-транспорт в LAN и выбор хаба · L · M3 · нет строк в каталоге проверок
- 🟡 [**SYNC-007**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1343) Синхронизация через хранилище · M · M3 · нет строк в каталоге проверок
- 🟡 [**SYNC-008**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1348) Хаб синхронизации на сервере · M · M4 · нет строк в каталоге проверок
- 🟡 [**SYNC-009**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1352) Профили в синхронизации · S · M3 · нет строк в каталоге проверок
- 🟡 [**SYNC-010**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1356) Синхронизация списка источников [P1] · M · M3 · нет строк в каталоге проверок
- 🟡 [**SYNC-011**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1360) Trakt [P1] · M · M3 · на устройствах ✅ 0/84
- ⚪ [**SYNC-012**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1364) Тесты сходимости и задержки handoff · M · M3
- 🟡 [**SYNC-013**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1368) Синхронизация категорий, тегов и оценок · M · M3 · нет строк в каталоге проверок
- 🟡 [**SYNC-014**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1372) Прогресс каждую секунду и мгновенная синхронизация · M · M10 · на устройствах ✅ 0/21
- 🟡 [**SYNC-015**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1376) Семья без настройки · M · M10 · на устройствах ✅ 0/35
- ⚪ [**SYNC-016**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1381) Кластерная синхронизация серверов (эпик) · L · M4
- 🟡 [**SYNC-017**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1385) Журнал каждого узла и векторы версий · M · M4 · нет строк в каталоге проверок
- ⚪ [**SYNC-018**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1389) Устойчивость к разным часам на узлах · M · M4
- ⚪ [**SYNC-019**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1393) Состав кластера, новый узел и сборка мусора · L · M4
- ⚪ [**SYNC-020**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1397) Хаос-тесты кластера · M · M4
- 🟡 [**SRV-001**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1404) Каркас сервера · M · M4 · нет строк в каталоге проверок
- 🟡 [**SRV-002**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1408) OpenAPI и контрактные тесты · S · M4 · нет строк в каталоге проверок
- 🟡 [**SRV-003**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1412) Аутентификация и pairing клиентов · M · M4 · нет строк в каталоге проверок
- 🟡 [**SRV-004**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1416) TLS и mDNS-анонс · S · M4 · нет строк в каталоге проверок
- ⚪ [**SRV-005**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1420) Библиотеки на сервере · M · M4
- ⚪ [**SRV-006**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1424) Media API и клиент своего сервера в ядре · L · M4
- ⚪ [**SRV-007**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1428) Direct play · S · M4
- ⚪ [**SRV-008**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1432) Транскод-менеджер · L · M4
- ⚪ [**SRV-009**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1436) HW-ускорение и тон-маппинг · L · M4
- ⚪ [**SRV-010**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1440) Remux для AVPlayer и транскод аудио · M · M4
- ⚪ [**SRV-011**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1444) API задач и события WebSocket · M · M4
- ⚪ [**SRV-012**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1448) «Оптимизировать для загрузки» [P1] · S · M4
- ⚪ [**SRV-013**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1452) Удалённый доступ через iroh · L · M4
- ⚪ [**SRV-014**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1457) Веб-админка сервера · L · M4
- ⚪ [**SRV-015**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1461) Хранилище и retention на сервере · M · M4
- ⚪ [**SRV-016**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1465) [full] Twitch-задачи на сервере · M · M7
- ⚪ [**SRV-017**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1470) Нагрузочные тесты и безопасность сервера · M · M4
- ⚪ [**SRV-018**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1474) Сборки сервера · M · M4
- 🟡 [**SRV-019**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1478) Сервер сообщает «идёт просмотр» (для обновлений ОС) · S · M8 · нет строк в каталоге проверок

</details>

<details><summary><b>Музыка</b> <code>▓▓▓▓▓░░░░░ 53% (10/19)</code> · ⚪ 9 · 🔵 0 · 🟡 10 · ✅ 0</summary>

_MUS_

- 🟡 [**MUS-001**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1485) Музыкальная библиотека: схема и скан тегов · M · M6 · нет строк в каталоге проверок
- 🟡 [**MUS-002**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1489) Воспроизведение v1 через libmpv в аудиорежиме · M · M6 · нет строк в каталоге проверок
- 🟡 [**MUS-003**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1493) Очередь и плейлисты · M · M6 · нет строк в каталоге проверок
- 🟡 [**MUS-004**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1497) UI музыкальной библиотеки · L · M6 · нет строк в каталоге проверок
- 🟡 [**MUS-005**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1501) Now Playing и мини-плеер · M · M6 · нет строк в каталоге проверок
- 🟡 [**MUS-006**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1505) Тексты песен · M · M6 · на устройствах ✅ 0/49
- ⚪ [**MUS-007**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1509) Системная интеграция аудио · M · M6
- ⚪ [**MUS-008**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1513) Свой аудиодвижок `pa-audio` · L · M6
- ⚪ [**MUS-009**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1517) Bit-perfect вывод · L · M6
- ⚪ [**MUS-010**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1521) DSD · M · M6
- ⚪ [**MUS-011**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1525) Обёртки `pa-audio` для систем · M · M6
- 🟡 [**MUS-012**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1529) Музыка из Plex, Jellyfin и Subsonic, офлайн-альбомы [P1] · M · M6 · на устройствах ✅ 0/7
- ⚪ [**MUS-013**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1534) Android Auto [P2] · M · M10
- ⚪ [**MUS-014**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1538) CarPlay Audio [P2] · M · M10
- 🟡 [**MUS-015**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1542) Продолжение музыки на другом устройстве, скробблинг, эквалайзер [P1/P2] · M · M6 · на устройствах ✅ 0/63
- ⚪ [**MUS-016**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1546) Аудиорежим MPVKit на Apple (`AudioKit`) · S · M6
- ⚪ [**MUS-017**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1550) Распознавание музыки по звуку (AcoustID/Chromaprint) · M · M6
- 🟡 [**MUS-018**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1554) Аудиокниги: медиатека и плеер · L · M6 · нет строк в каталоге проверок
- 🟡 [**MUS-019**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1558) Аудиокниги: распознавание и метаданные · M · M6 · нет строк в каталоге проверок

</details>

<details><summary><b>Twitch</b> <code>▓▓▓▓▓▓▓▓░░ 76% (32/42)</code> · ⚪ 10 · 🔵 0 · 🟡 32 · ✅ 0</summary>

_TWI_

- 🟡 [**TWI-001**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1569) `pa-twitch`: клиент Helix · M · M7 · нет строк в каталоге проверок
- 🟡 [**TWI-002**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1573) Вход через Device Code Flow · M · M7 · нет строк в каталоге проверок
- ⚪ [**TWI-003**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1577) Прототип режима «без логина» · M · M7
- 🟡 [**TWI-004**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1581) Воспроизведение: GQL → usher и удалённый конфиг · M · M7 · нет строк в каталоге проверок
- 🟡 [**TWI-005**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1585) Запасной режим: официальный embed · M · M7 · нет строк в каталоге проверок
- 🟡 [**TWI-006**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1589) Live: низкая задержка и реклама «как есть» · M · M7 · нет строк в каталоге проверок
- 🟡 [**TWI-007**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1593) Модель данных Twitch в библиотеке · M · M7 · нет строк в каталоге проверок
- 🟡 [**TWI-008**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1597) Главы и категории VOD · M · M7 · нет строк в каталоге проверок
- 🟡 [**TWI-009**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1601) Уведомления «X в эфире» без облака · M · M7 · нет строк в каталоге проверок
- 🟡 [**TWI-010**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1605) Live-чат: абстракция ChatSource · M · M7 · нет строк в каталоге проверок
- 🟡 [**TWI-011**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1609) Эмоуты и бейджи · M · M7 · нет строк в каталоге проверок
- 🟡 [**TWI-012**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1613) Отрисовка чата · M · M7 · нет строк в каталоге проверок
- 🟡 [**TWI-013**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1617) UI раздела Twitch (Flutter) и брендинг · L · M7 · нет строк в каталоге проверок
- 🟡 [**TWI-014**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1621) Плеер Twitch · M · M7 · нет строк в каталоге проверок
- ⚪ [**TWI-015**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1625) Ключи синхронизации и выбор источника · M · M7
- 🟡 [**TWI-016**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1630) [full] Чат-replay VOD онлайн · M · M7 · нет строк в каталоге проверок
- 🟡 [**TWI-017**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1634) [full] Загрузчик VOD · L · M7 · на устройствах ✅ 0/5
- 🟡 [**TWI-018**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1638) [full] Запись live · L · M7 · нет строк в каталоге проверок
- 🟡 [**TWI-019**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1643) [full] Хранилище, имена файлов, импорт, правила записи · M · M7 · нет строк в каталоге проверок
- ⚪ [**TWI-020**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1647) Канареечный тест Twitch в nightly CI · S · M7
- ⚪ [**TWI-021**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1651) Twitch на Android TV · M · M7
- 🟡 [**TWI-022**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1656) Мультипросмотр 2–4 стримов [P2] · M · M10 · нет строк в каталоге проверок
- 🟡 [**TWI-023**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1660) Сторонние эмоуты 7TV, BetterTTV, FFZ в чате · M · M10 · нет строк в каталоге проверок
- 🟡 [**TWI-024**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1664) Фильтры и ввод в чате · M · M7 · нет строк в каталоге проверок
- 🟡 [**TWI-025**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1668) [full] Live DVR по окну плейлиста и «Смотреть с начала эфира» · M · M7 · на устройствах ✅ 0/5
- 🟡 [**TWI-026**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1672) [full] Архив стримов как сериал: стример → игра → части · M · M7 · нет строк в каталоге проверок
- 🟡 [**TWI-027**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1676) Быстрый переход по стриму: «Пропустить болтовню» · S · M7 · нет строк в каталоге проверок
- ⚪ [**TWI-028**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1680) [full] Сохранение «всё вместе»: видео, чат, игра, метаданные · M · M7
- 🟡 [**TWI-029**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1684) Официальный плеер Twitch на Windows и Linux · M · M10 · на устройствах ✅ 0/1103
- 🟡 [**TWI-030**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1688) [full] Включить нативный плеер Twitch через подписанный конфиг · S · M10 · нет строк в каталоге проверок
- ⚪ [**TWI-031**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1692) Эпик: Twitch — полноценный клиент (запрос владельца 2026-11-25) · L · M10
- 🟡 [**TWI-032**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1696) Аватар и имя аккаунта Twitch в приложении · S · M10 · нет строк в каталоге проверок
- ⚪ [**TWI-033**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1700) Фоллов и отписка из приложения · S · M10
- 🟡 [**TWI-034**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1704) Обзор Twitch: категории, эфиры в категории, поиск стримеров · M · M10 · на устройствах ✅ 0/78
- 🟡 [**TWI-035**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1708) [full] Кнопка «Записать» в эфире и готовая запись по окончании · S · M10 · на устройствах ✅ 0/30
- ⚪ [**TWI-036**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1712) Страница канала целиком: эфир, записи, клипы, расписание, о канале · M · M10
- ⚪ [**TWI-037**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1716) «Ссылки канала»: панели под стримом (баннеры и ссылки) · M · M10
- ⚪ [**TWI-038**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1720) Чат на телефоне, пока эфир идёт на ТВ или компьютере · M · M10
- 🟡 [**TWI-039**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1724) Пузырь «Отправлено» с игривой анимацией · S · M10 · нет строк в каталоге проверок
- 🟡 [**TWI-040**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1734) Клип из приложения · S · M10 · нет строк в каталоге проверок
- 🟡 [**TWI-041**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1738) Чат рядом со встроенным плеером Twitch · M · M10 · на устройствах ✅ 0/5
- 🟡 [**TWI-042**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1742) Главы записей Twitch из GQL в плеере · S · M10 · нет строк в каталоге проверок

</details>

<details><summary><b>Дистрибутив</b> <code>▓▓▓░░░░░░░ 33% (11/33)</code> · ⚪ 22 · 🔵 0 · 🟡 11 · ✅ 0</summary>

_DIST_

- 🟡 [**DIST-001**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1751) Прототип образа · M · M8 · нет строк в каталоге проверок
- 🟡 [**DIST-002**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1755) Сборка образа в CI и политика подписи · M · M8 · нет строк в каталоге проверок
- 🟡 [**DIST-003**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1759) Брендинг ОС · S · M8 · нет строк в каталоге проверок
- 🟡 [**DIST-004**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1763) Установщик: обнаружение дисков (Go) · M · M8 · нет строк в каталоге проверок
- 🟡 [**DIST-005**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1767) Установщик: разметка, `bootc install`, конфигурация, файл ответов · L · M8 · нет строк в каталоге проверок
- ⚪ [**DIST-006**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1771) Установщик: TUI (bubbletea) · M · M8
- 🟡 [**DIST-007**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1775) Live-ISO · M · M8 · нет строк в каталоге проверок
- ⚪ [**DIST-008**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1779) QEMU-тесты установки в CI · M · M8
- 🟡 [**DIST-009**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1783) `alpha-updated`: агент обновлений · M · M8 · нет строк в каталоге проверок
- 🟡 [**DIST-010**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1787) Снапшоты `/var` и откат состояния · M · M8 · на устройствах ✅ 0/5
- ⚪ [**DIST-011**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1792) greenboot и тесты обновления и отката · M · M8
- ⚪ [**DIST-012**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1796) Каналы и поэтапная раскатка · M · M8
- 🟡 [**DIST-013**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1800) `alpha-admin`: бэкенд (Go) · L · M8 · нет строк в каталоге проверок
- 🟡 [**DIST-014**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1804) `alpha-admin`: веб-интерфейс · L · M8 · на устройствах ✅ 0/36
- ⚪ [**DIST-015**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1808) Сервис хранилища и мастер пулов · L · M8
- ⚪ [**DIST-016**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1812) Обслуживание и уведомления · M · M8
- ⚪ [**DIST-017**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1816) Мастер замены диска · M · M8
- ⚪ [**DIST-018**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1820) Samba, пользователи и firewall · M · M8
- ⚪ [**DIST-019**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1824) Медиасервер в образе и VAAPI · M · M8
- ⚪ [**DIST-020**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1828) Вариант образа с NVIDIA [фаза 2] · L · M8
- ⚪ [**DIST-021**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1832) Веб-установщик для NAS без монитора [фаза 2] · M · M10
- ⚪ [**DIST-022**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1836) LUKS2 + TPM2 [фаза 2] · M · M10
- ⚪ [**DIST-023**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1840) Режим «Медиа-архив»: mergerfs + SnapRAID [фаза 2] · M · M10
- ⚪ [**DIST-024**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1844) Хостинг ISO · S · M8
- ⚪ [**DIST-025**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1848) (owner) Раннер с реальным железом · M · M8 · владелец
- ⚪ [**DIST-026**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1852) Переход на новую major-версию Fedora (45 → 46) · M · M10
- ⚪ [**DIST-027**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1856) Бэкапы и снапшоты пулов [P2] · L · M10
- 🟡 [**DIST-032**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1879) Wi-Fi и сеть прямо с главной · M · M8 · на устройствах ✅ 0/11
- ⚪ [**DIST-033**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1883) Сервер Kinolune: настройка с телефона в пару нажатий · M · M10
- ⚪ [**DIST-034**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1887) Установщик: подробный прогресс каждого шага (как в pacman) · M · M8
- ⚪ [**DIST-035**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1891) Обновления Kinolune OS из клиентского приложения · M · M8
- ⚪ [**DIST-036**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1895) Обновления ОС с экрана без пароля: уведомление и меню «Обновить» · M · M10
- ⚪ [**DIST-037**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1899) Загрузка и установка обновления: логотип и аккуратная полоса вместо отладочного текста · M · M10

</details>

<details><summary><b>Сборка и релизы</b> <code>▓▓▓▓░░░░░░ 44% (24/54)</code> · ⚪ 30 · 🔵 0 · 🟡 8 · ✅ 16</summary>

_PKG, CI, REPO_

- ✅ [**REPO-001**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L207) Каркас монорепо · S · M0 · проверка на устройстве не нужна
- ✅ [**REPO-002**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L211) LICENSE, SPDX-заголовки и REUSE · S · M0 · проверка на устройстве не нужна
- ✅ [**REPO-003**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L215) CONTRIBUTING, CODE_OF_CONDUCT, SECURITY · S · M0 · проверка на устройстве не нужна
- ✅ [**REPO-004**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L219) DCO-проверка и линт заголовка PR · S · M0 · проверка на устройстве не нужна
- ⚪ [**REPO-005**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L223) (owner) CODEOWNERS, rulesets и окружения · S · M0 · владелец
- ✅ [**REPO-006**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L227) Шаблоны issue и PR, метки как код · S · M0 · проверка на устройстве не нужна
- ✅ [**REPO-007**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L231) AGENTS.md и CLAUDE.md для агентов · S · M0 · проверка на устройстве не нужна
- ⚪ [**REPO-008**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L235) Закрепить toolchain, `just bootstrap`, Dev Container · M · M0
- ✅ [**REPO-009**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L239) Политика секретов · S · M0 · проверка на устройстве не нужна
- ⚪ [**REPO-010**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L243) release-please (manifest mode) · S · M0
- ⚪ [**REPO-011**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L248) (owner) Подготовка и переключение репо в публичный режим · S · M0 · владелец
- ✅ [**REPO-012**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L252) Инструменты ROADMAP: `just roadmap lint` и `just roadmap next` · S · M0 · проверка на устройстве не нужна
- ✅ [**REPO-013**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L256) Dependabot или Renovate · S · M0 · проверка на устройстве не нужна
- ✅ [**REPO-014**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L260) Аккуратная структура репозитория · M · M10 · проверка на устройстве не нужна
- ✅ [**REPO-015**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L264) Живая страница прогресса в ветке `progress` · M · M0 · проверка на устройстве не нужна
- ✅ [**CI-001**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L271) `rust.yml`: fmt, clippy, nextest · S · M0 · проверка на устройстве не нужна
- ✅ [**CI-002**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L275) Безопасный CI для PR из форков · S · M0 · проверка на устройстве не нужна
- ✅ [**CI-003**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L279) `cargo deny` и `cargo audit` · S · M0 · проверка на устройстве не нужна
- ⚪ [**CI-004**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L283) Матрица Rust-таргетов и sccache · M · M0
- ⚪ [**CI-005**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L287) `flutter.yml` · M · M0
- ⚪ [**CI-006**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L291) `android.yml`: нативные модули и эмулятор · M · M1
- ⚪ [**CI-007**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L295) `apple.yml` на macOS-runner · M · M0
- ✅ [**CI-008**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L299) Кэш и раздача артефактов FFmpeg и libmpv · S · M1 · проверка на устройстве не нужна
- ⚪ [**CI-009**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L303) Golden-скриншоты Flutter по плотностям · S · M1
- ⚪ [**CI-010**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L307) Eval распознавания на каждый PR · S · M2
- ⚪ [**CI-011**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L311) Nightly и релизный пайплайн · M · M1
- ⚪ [**CI-012**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L315) Сетевые интеграционные тесты · M · M1
- ✅ [**CI-013**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L319) Gradle в CI без падений на HTTP 429 от Maven Central · S · M0 · проверка на устройстве не нужна
- ⚪ [**PKG-001**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2002) Раскладка артефактов, подписи и `releases.json` · S · M1
- ⚪ [**PKG-002**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2006) Linux: AppImage, deb, rpm · M · M1
- ⚪ [**PKG-003**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2011) Flathub · M · M10
- ⚪ [**PKG-004**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2015) AUR · S · M10
- ⚪ [**PKG-005**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2019) Windows: установщик, автообновление, подпись · M · M1
- ⚪ [**PKG-006**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2023) winget · S · M10
- ⚪ [**PKG-007**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2027) [store] Microsoft Store · M · M10
- ⚪ [**PKG-008**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2031) macOS: DMG, нотаризация, обновления, Homebrew · M · M5
- ⚪ [**PKG-009**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2035) Android: APK в GitHub Releases · S · M1
- ⚪ [**PKG-010**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2039) F-Droid · L · M10
- ⚪ [**PKG-011**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2043) (owner) [store] Google Play · L · M10 · владелец
- ⚪ [**PKG-012**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2047) (owner) Apple Developer Program и конвейер TestFlight · M · M5 · владелец
- ⚪ [**PKG-013**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2051) Пакет для ревью App Store · M · M10
- ⚪ [**PKG-014**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2055) Матрица флейворов store и full по каналам · S · M1
- ⚪ [**PKG-015**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2059) [store] Чаевые через IAP или удаление экрана донатов · M · M10
- ⚪ [**PKG-016**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2063) Пакет соответствия LGPL · M · M1
- ⚪ [**PKG-017**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2067) Автообновление десктопа: каналы и UX · M · M5
- 🟡 [**PKG-018**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2071) Обновление из приложения (full-флейвор) · M · M1 · нет строк в каталоге проверок
- 🟡 [**PKG-019**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2075) macOS: неподписанная сборка приложения в GitHub Releases · M · M1 · нет строк в каталоге проверок
- 🟡 [**PKG-020**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2079) Windows: портативная сборка приложения в GitHub Releases · M · M1 · нет строк в каталоге проверок
- 🟡 [**PKG-021**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2083) macOS: падение при старте видео (CoreAudio, alpha.11) и проверка воспроизведения в CI · S · M1 · нет строк в каталоге проверок
- 🟡 [**PKG-022**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2088) Установщик Windows и автообновление на Windows, macOS и Linux · M · M1 · нет строк в каталоге проверок
- 🟡 [**PKG-023**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2092) Репозитории APT и DNF: установка из терминала и обновление через `apt upgrade` / `dnf upgrade` · M · M8 · нет строк в каталоге проверок
- 🟡 [**PKG-024**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2097) Тихие обновления · S · M10 · на устройствах ✅ 0/16
- 🟡 [**PKG-025**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2101) Предрелизная проверка сборки на сервере сборки · M · M10 · нет строк в каталоге проверок
- ⚪ [**PKG-026**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2105) Сборка для Linux на ARM (aarch64) · M · M10

</details>

<details><summary><b>Качество и тесты</b> <code>▓▓▓▓░░░░░░ 42% (8/19)</code> · ⚪ 11 · 🔵 0 · 🟡 1 · ✅ 7</summary>

_QA_

- ✅ [**QA-001**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2112) Тест-стратегия · S · M0 · проверка на устройстве не нужна
- ✅ [**QA-002**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2116) Тестовые медиа-сэмплы · M · M0 · проверка на устройстве не нужна
- ⚪ [**QA-003**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2120) Матрица воспроизведения (smoke) · M · M1
- ⚪ [**QA-004**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2124) E2E-тесты Flutter · M · M1
- ⚪ [**QA-005**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2128) Фаззинг · M · M3
- ✅ [**QA-006**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2132) Бюджеты производительности UI · M · M1 · проверка на устройстве не нужна
- 🟡 [**QA-007**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2136) Тесты на плохой сети · M · M1 · на устройствах ✅ 0/7
- ⚪ [**QA-008**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2140) Релизный чек-лист · S · M1
- ⚪ [**QA-009**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2144) Аудит доступности и локализации · M · M1
- ⚪ [**QA-010**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2148) Модель угроз и ревью безопасности · M · M4
- ⚪ [**QA-011**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2152) (owner) Лаборатория устройств и бета-программа · M · M1 · владелец
- ⚪ [**QA-012**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2156) Отчёты об ошибках от пользователей по кнопке (после редизайна) · M · M10
- ⚪ [**QA-013**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2160) (owner) Дежурный агент по почте: исправление ошибок из отчётов · S · M10 · владелец
- ✅ [**QA-014**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2165) Каталог проверок: все кнопки, меню и функции на всех платформах · M · M3 · проверка на устройстве не нужна
- ✅ [**QA-015**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2169) Исправления по каталогу проверок · M · M3 · проверка на устройстве не нужна
- ✅ [**QA-016**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2173) Живые e2e-тесты: настоящее приложение, сеть и SMB · M · M10 · проверка на устройстве не нужна
- ✅ [**QA-017**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2178) Живые тесты на Windows: настоящее установленное приложение · M · M10 · проверка на устройстве не нужна
- ⚪ [**QA-018**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2183) Живые тесты на macOS: настоящее приложение, захват экрана и клики · M · M10
- ⚪ [**QA-019**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2188) Живые тесты на симуляторе iPhone и iPad · M · M10

</details>

<details><summary><b>Документы и право</b> <code>▓▓▓▓░░░░░░ 35% (17/48)</code> · ⚪ 31 · 🔵 0 · 🟡 1 · ✅ 16</summary>

_DOC, LEGAL, SITE, COMM, DEC_

- ✅ [**DEC-001**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L134) (owner) Сделать `DenisHumen/kinolune` публичным; решить судьбу `project_alpha-releases` · S · M0 · проверка на устройстве не нужна
- ✅ [**DEC-002**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L138) (owner) Лицензия кода: MPL-2.0 (рекомендация) или GPL-3.0 · S · M0 · проверка на устройстве не нужна
- ✅ [**DEC-003**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L142) (owner) Донаты в store-сборках: без ссылок или «чаевые» через IAP · S · M5 · проверка на устройстве не нужна
- ✅ [**DEC-004**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L146) (owner) Название и бренд продукта (и имя NAS-ОС) · S · M1 · проверка на устройстве не нужна
- ✅ [**DEC-005**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L150) (owner) Юридическая позиция по кодекам (H.264/HEVC/AAC, RPM Fusion, своя сборка FFmpeg на сервере) · S · M4 · проверка на устройстве не нужна
- ✅ [**DEC-006**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L154) (owner) Twitch без логина: публичные данные через GQL или только вход через Device Code Flow · S · M7 · проверка на устройстве не нужна
- ⚪ [**DEC-007**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L158) (owner) Аккаунты и юрисдикция: Apple Developer, Google Play, SignPath, Microsoft Store · S · M1 · владелец
- ✅ [**DEC-008**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L162) (owner) Ключ TMDB в открытом коде: встроенный ключ проекта по умолчанию или обязательный свой ключ · S · M2 · проверка на устройстве не нужна
- ⚪ [**DEC-011**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L196) (owner) Референс дизайна сайта и VPS для сайта-визитки · S · M10 · владелец
- ⚪ [**DEC-012**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L200) (owner) Железо для Kinolune OS: управление и своё устройство · S · M10 · владелец
- ✅ [**DOC-001**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2195) Базовый набор ADR · M · M0 · проверка на устройстве не нужна
- ⚪ [**DOC-002**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2199) Настройка среды разработчика · S · M0
- ✅ [**DOC-003**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2203) Обзор архитектуры · M · M1 · проверка на устройстве не нужна
- ⚪ [**DOC-004**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2207) Документация API · S · M4
- ✅ [**DOC-005**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2212) Пользовательская документация: приложение · M · M1 · проверка на устройстве не нужна
- ⚪ [**DOC-006**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2216) Пользовательская документация: сервер · M · M4
- ⚪ [**DOC-007**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2220) Пользовательская документация: NAS-ОС · M · M8
- ⚪ [**DOC-008**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2224) Пользовательская документация: Twitch и музыка · S · M7
- ⚪ [**DOC-009**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2228) Runbooks · M · M7
- ⚪ [**DOC-010**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2232) Отчёт о качестве распознавания · M · M10
- ⚪ [**DOC-011**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2236) Пошаговые руководства «для чайника»: как поставить и настроить сервер · L · M10
- ⚪ [**LEGAL-001**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2243) (owner) Письмо в TMDB · S · M0 · владелец
- ✅ [**LEGAL-002**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2247) Таблица условий провайдеров · M · M1 · проверка на устройстве не нужна
- ✅ [**LEGAL-003**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2251) Лицензии данных маппингов аниме · S · M1 · проверка на устройстве не нужна
- ⚪ [**LEGAL-004**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2255) (owner) Патенты кодеков, Dolby и DTS · M · M4 · владелец
- ✅ [**LEGAL-005**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2259) Проверка названия и товарных знаков · S · M0 · проверка на устройстве не нужна
- ⚪ [**LEGAL-006**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2263) (owner) Twitch: ToS, Developer Agreement, регистрация приложения · M · M7 · владелец
- ✅ [**LEGAL-007**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2267) Политика конфиденциальности · S · M1 · проверка на устройстве не нужна
- ⚪ [**LEGAL-008**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2271) Политика донатов в сторах · S · M10
- ⚪ [**LEGAL-009**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2276) Аудит лицензий зависимостей и ассетов · M · M1
- ⚪ [**LEGAL-010**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2280) Юридические вопросы дистрибутива · M · M8
- 🟡 [**SITE-001**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2292) Каркас сайта · M · M1 · на устройствах ✅ 0/32
- ⚪ [**SITE-002**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2297) Страница «Скачать» · M · M10
- ⚪ [**SITE-003**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2301) Документация и API на сайте · S · M4
- ⚪ [**SITE-004**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2305) Хостинг подписанных манифестов · S · M1
- ⚪ [**SITE-005**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2309) Донаты и публичный roadmap · S · M1
- ⚪ [**SITE-006**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2313) (owner) Домен и поддомен загрузок · S · M8 · владелец
- ⚪ [**SITE-007**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2317) Сайт-визитка на VPS: скачать всё одной кнопкой, QR в сторы, ISO, документация · L · M10
- ⚪ [**COMM-001**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2324) (owner) FUNDING.yml и площадки донатов · S · M0 · владелец
- ✅ [**COMM-002**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2328) GitHub Discussions и запросы функций · S · M0 · проверка на устройстве не нужна
- ✅ [**COMM-003**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2332) README на русском и английском · S · M0 · проверка на устройстве не нужна
- ⚪ [**COMM-004**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2336) Good first issues из ROADMAP · S · M1
- ⚪ [**COMM-005**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2340) (owner) Каналы общения и анонсы · S · M1 · владелец
- ⚪ [**COMM-006**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2344) Переводы силами сообщества · S · M10
- ⚪ [**COMM-007**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2348) Промо-ролики высочайшего качества · L · M10
- ⚪ [**COMM-008**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2352) Запуск и продвижение: аудитория до подписки · M · M10
- ⚪ [**COMM-009**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2356) Короткие смешные ролики для TikTok через Blender · M · M10
- ⚪ [**COMM-010**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2360) Видео-руководства на YouTube: 3D-ролики в Blender и запись установки дистрибутива с озвучкой на трёх языках · L · M10

</details>

<details><summary><b>Заготовки подписки</b> <code>▓░░░░░░░░░ 12% (1/8)</code> · ⚪ 7 · 🔵 0 · 🟡 0 · ✅ 1</summary>

_SUB_

- ✅ [**SUB-001**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2392) Стратегия и ADR готовности к подписке · S · M10 · проверка на устройстве не нужна
- ⚪ [**SUB-002**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2396) `Entitlements`: одна точка решения доступности · S · M10
- ⚪ [**SUB-003**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2400) `ApiKeySource`: ключи провайдеров в одном месте · S · M10
- ⚪ [**SUB-004**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2404) `SyncBackend`: общий интерфейс доставки журнала · M · M10
- ⚪ [**SUB-005**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2408) `Notifier`: уведомления через один интерфейс · S · M10
- ⚪ [**SUB-006**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2412) `RemoteAccess`: транспорт вне дома · S · M10
- ⚪ [**SUB-007**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2416) `AiProvider`: модель для ИИ-функций · S · M10
- ⚪ [**SUB-008**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2420) Канал обновлений «бета» · S · M10

</details>

<details><summary><b>Эксперименты</b> <code>░░░░░░░░░░ 0% (0/9)</code> · ⚪ 9 · 🔵 0 · 🟡 0 · ✅ 0</summary>

_M11, M12_

- ⚪ [**PLAY-019**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L834) Меню Blu-ray BD-J (Java) на компьютере и сервере [может быть] · L · M12
- ⚪ [**DIST-028**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1861) ТВ-оболочка ОС: исследование и ADR [может быть] · S · M12
- ⚪ [**DIST-029**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1865) Kinolune-сессия: вход сразу в полноэкранный Kinolune [может быть] · L · M12
- ⚪ [**DIST-030**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1869) Переключение «Рабочий стол ↔ Kinolune» и доработанный KDE Plasma [может быть] · M · M12
- ⚪ [**DIST-031**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1874) Системные настройки внутри оболочки [может быть] · L · M12
- ⚪ [**EXT-001**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2371) watchOS и visionOS: исследование и ADR [P2] · S · M11
- ⚪ [**EXT-002**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2375) Apple Watch: пульт воспроизведения [P2] · L · M11
- ⚪ [**EXT-003**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2379) Apple Watch: уведомления и «сейчас играет» [P2] · M · M11
- ⚪ [**EXT-004**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2383) visionOS: экспериментальный клиент [P2] · L · M11

</details>

<details><summary><b>Подписка</b> <code>░░░░░░░░░░ 0% (0/23)</code> · ⚪ 23 · 🔵 0 · 🟡 0 · ✅ 0</summary>

_M13_

- ⚪ [**SUB-010**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2424) Аккаунт и оплата Kinolune+ · L · M13
- ⚪ [**SUB-011**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2428) Наш ретранслятор для доступа вне дома · M · M13
- ⚪ [**SUB-012**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2432) Облачная синхронизация и бэкап (E2E) · M · M13
- ⚪ [**SUB-013**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2436) Push-уведомления · M · M13
- ⚪ [**SUB-014**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2440) Прокси метаданных и субтитров с квотами · M · M13
- ⚪ [**SUB-015**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2444) ИИ-функции Kinolune+ · L · M13
- ⚪ [**SUB-016**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2448) Магазины: модель продажи · M · M13
- ⚪ [**SUB-017**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2452) Юридическое: условия, конфиденциальность, налоги · M · M13
- ⚪ [**SUB-018**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2456) Сайт и стратегия v3 · S · M13
- ⚪ [**SUB-019**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2465) ADR: аккаунты, достижения, бэкенд и админка · M · M13
- ⚪ [**SUB-020**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2475) Ядро достижений `pa-achievements` · L · M13
- ⚪ [**SUB-021**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2480) Достижения, которые интересно получать: типы условий и первые наборы · M · M13
- ⚪ [**SUB-022**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2485) Редкость достижений · M · M13
- ⚪ [**SUB-023**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2495) Достижения в приложении · M · M13
- ⚪ [**SUB-024**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2500) Бэкенд на Rust · L · M13
- ⚪ [**SUB-025**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2505) Кластер Kubernetes на трёх VPS · L · M13
- ⚪ [**SUB-026**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2510) Веб-админка · L · M13
- ⚪ [**SUB-027**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2515) Баны с причиной и отменой подписки · M · M13
- ⚪ [**SUB-028**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2520) Подарочные коды на 1, 2 и 3 месяца · M · M13
- ⚪ [**SUB-029**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2525) Доступ агентов к бэкенду и админке · M · M13
- ⚪ [**SUB-030**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2530) Статистика и обратная связь в админке · M · M13
- ⚪ [**SUB-031**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2534) Пульт в «островке» через наше облако · M · M13
- ⚪ [**SUB-032**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2538) Облачные аккаунты: принципы с первого дня · M · M13

</details>

<details><summary><b>Социальное</b> <code>░░░░░░░░░░ 0% (0/7)</code> · ⚪ 7 · 🔵 0 · 🟡 0 · ✅ 0</summary>

_M15_

- ⚪ [**SOC-001**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2717) ADR социальных функций · M · M15
- ⚪ [**SOC-002**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2722) Публичный профиль: никнейм, аватар, видимость · M · M15
- ⚪ [**SOC-003**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2727) Друзья · M · M15
- ⚪ [**SOC-004**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2732) Что смотрят друзья и их достижения · M · M15
- ⚪ [**SOC-005**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2737) Дашборды на аккаунте · M · M15
- ⚪ [**SOC-006**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2742) Передача файлов между друзьями со сквозным шифрованием · L · M15
- ⚪ [**SOC-007**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2747) Модерация и безопасность социальных функций · M · M15

</details>

<details><summary><b>Экосистема</b> <code>░░░░░░░░░░ 0% (0/13)</code> · ⚪ 13 · 🔵 0 · 🟡 0 · ✅ 0</summary>

_M16_

- ⚪ [**ECO-001**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2758) ADR и протокол «приложение ↔ сервер» · M · M16
- ⚪ [**ECO-002**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2763) Подключение приложений к серверу: одноразовый код, QR, логин · M · M16
- ⚪ [**ECO-003**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2768) Приём файлов на сервере · M · M16
- ⚪ [**ECO-004**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2773) «Присланное» в Kinolune · M · M16
- ⚪ [**ECO-005**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2778) События и синхронизация состояния присланного · S · M16
- ⚪ [**ECO-006**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2783) Тестовый поставщик и общий e2e · S · M16
- ⚪ [**ECO-007**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2788) Universal Video Downloader: отправка на сервер Kinolune (в репозитории UVD) · M · M16
- ⚪ [**ECO-008**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2793) Kinolune desktop как поставщик · S · M16
- ⚪ [**ECO-009**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2798) Экосистема вне дома через облако · M · M16
- ⚪ [**ECO-010**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2802) UVD ↔ запущенный Kinolune: сопряжение через профиль и общие папки · L · M16
- ⚪ [**ECO-011**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2806) Живая карточка скачивания из UVD в Kinolune · M · M16
- ⚪ [**ECO-012**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2810) Отслеживание новых серий из UVD на карточке сериала Kinolune · M · M16
- ⚪ [**ECO-013**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2814) UVD ↔ Kinolune через облако на всех устройствах · M · M16

</details>

<details><summary><b>Диски</b> <code>░░░░░░░░░░ 0% (0/6)</code> · ⚪ 6 · 🔵 0 · 🟡 0 · ✅ 0</summary>

_M14_

- ⚪ [**LIB-026**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L596) Копирование дисков: вопрос открыт (blocked: LEGAL-011) · L · M14
- ⚪ [**PLAY-020**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L838) Kinolune как Blu-ray/DVD-плеер: смотреть диск прямо с привода · M · M14
- ⚪ [**PLAY-021**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L842) (owner) Защищённые Blu-ray: лицензия AACS для воспроизведения · M · M14 · владелец
- ⚪ [**UI-048**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1190) Баннер «Найден диск» на всех устройствах · S · M14
- ⚪ [**MUS-020**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L1562) Аудио-CD в FLAC одним нажатием (blocked: LEGAL-011) · M · M14
- ⚪ [**LEGAL-011**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2284) Копирование дисков и позиционирование продукта в ЕС и других странах · S · M14

</details>

<details><summary><b>Баги</b> <code>▓▓▓▓▓▓▓▓░░ 83% (30/36)</code> · ⚪ 6 · 🔵 0 · 🟡 30 · ✅ 0</summary>

_BUG_

- 🟡 [**BUG-001**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2547) [P0] Память растёт при каждом открытии и закрытии плеера · M · M10 · нет строк в каталоге проверок
- ⚪ [**BUG-002**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2551) [P1] Подвисание в первые 10 секунд первого просмотра по SMB · S · M10
- 🟡 [**BUG-003**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2555) [P1] Состояние ошибки плеера «залипает» после безвредного сообщения mpv · S · M10 · на устройствах ✅ 0/7
- 🟡 [**BUG-004**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2559) [P1] Папка, найденная в сети, сохраняется по IP, а не по имени · S · M10 · нет строк в каталоге проверок
- 🟡 [**BUG-005**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2563) [P2] Файлы из «свалки» показываются как фильм «clip» · S · M10 · нет строк в каталоге проверок
- 🟡 [**BUG-006**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2567) [P2] Белая вертикальная линия на карточке «Продолжить» · S · M10 · на устройствах ✅ 0/7
- 🟡 [**BUG-007**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2571) [P1] Тест шины событий pa_core зависает в CI · S · M10 · нет строк в каталоге проверок
- 🟡 [**BUG-008**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2575) [P2] Долгие потоки ядра (синхронизация, поиск в сети) отписываются только со следующим событием · S · M10 · нет строк в каталоге проверок
- 🟡 [**BUG-009**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2579) [P0] Медиа с верной подсказкой не распознаётся само — пользователя заставляют подтверждать вручную · M · M10 · на устройствах ✅ 0/7
- 🟡 [**BUG-010**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2583) [P1] База аниме не обновляется: сжатый выпуск читается как текст · S · M10 · нет строк в каталоге проверок
- 🟡 [**BUG-011**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2587) [P1] Фильм с единственным верным кандидатом «Название: подзаголовок» не распознаётся сам · S · M10 · нет строк в каталоге проверок
- 🟡 [**BUG-012**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2591) [P2] Строка «… стоит проверить» на главной и счётчик проверки в настройках не обновляются после распознавания · S · M10 · нет строк в каталоге проверок
- 🟡 [**BUG-013**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2595) [P2] «Источники» показывают сырую английскую ошибку хранилища ключей · S · M10 · на устройствах ✅ 0/7
- 🟡 [**BUG-014**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2599) [P2] Серии нераспознанного сериала подписаны по-английски «Episode N» · S · M10 · нет строк в каталоге проверок
- 🟡 [**BUG-015**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2603) [P2] При перемотке в плеере падает построение семантики (`Null check operator` в `_updateSemanticsNodeGeometry`) · S · M10 · нет строк в каталоге проверок
- 🟡 [**BUG-016**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2608) [P2] Плашка о хранилище ключей остаётся поверх видео в плеере · S · M10 · нет строк в каталоге проверок
- 🟡 [**BUG-017**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2612) [P2] `tools/qa/e2e/run.sh` не собирает приложение в свежем клоне: нет шрифтов Inter · S · M10 · нет строк в каталоге проверок
- 🟡 [**BUG-018**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2617) [P0] Папка SMB сканируется, но медиатека пуста: одна паника на имени файла рушит весь скан · S · M10 · нет строк в каталоге проверок
- 🟡 [**BUG-019**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2622) [P0] На странице сериала не добраться до серий: длинный сезон уходит за край экрана · S · M10 · нет строк в каталоге проверок
- 🟡 [**BUG-020**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2627) [P1] Пробел или «Воспроизвести» в конце серии запускает ту же серию заново вместо следующей · S · M10 · нет строк в каталоге проверок
- 🟡 [**BUG-021**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2632) [P2] Фильм из нескольких частей показывает длительность только первой части · S · M10 · нет строк в каталоге проверок
- 🟡 [**BUG-022**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2637) [P2] Windows: пути со смешанными разделителями · S · M10 · нет строк в каталоге проверок
- 🟡 [**BUG-023**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2642) [P2] Фильм с точным оригинальным названием уходит на проверку: имя файла сравнивается с русским переводом · S · M10 · нет строк в каталоге проверок
- 🟡 [**BUG-024**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2647) [P2] Диагностический отчёт на Windows: версия без номера сборки, пустой config.txt, не сказано, куда сохранён · S · M10 · нет строк в каталоге проверок
- 🟡 [**BUG-025**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2652) [P2] Нажатие на уже выбранный раздел в левой панели не возвращает к списку раздела · S · M10 · на устройствах ✅ 0/7
- ⚪ [**BUG-027**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2657) [P2] Windows: клик по выбранному разделу не закрыл страницу фильма (BUG-025 не воспроизведена) · S · M10
- 🟡 [**BUG-026**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2662) [P0] Twitch: прямой эфир и запись открываются серым экраном, помогает только перезапуск · M · M10 · на устройствах ✅ 0/14
- 🟡 [**BUG-028**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2667) [P0] Linux: прямой эфир Twitch открывается чёрным экраном, видео не идёт · M · M10 · нет строк в каталоге проверок
- ⚪ [**BUG-029**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2672) [P1] Linux: встроенный плеер Twitch после рекламной паузы падает с «There was a network error… (Error #2000)» · M · M10
- 🟡 [**BUG-030**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2677) [P1] Приложение никогда не скачивает удалённую настройку: опубликованная serial 3 (нативный плеер Twitch) не применяется · S · M10 · нет строк в каталоге проверок
- 🟡 [**BUG-031**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2682) [P1] Установленная ОС не может обновиться: следит за `ghcr.io/denishumen/alpha-os:stable`, такого тега нет · M · M8 · нет строк в каталоге проверок
- ⚪ [**BUG-032**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2688) [P2] Установщик: «извлеките флешку и перезагрузите» ломает live-систему · S · M8
- ⚪ [**BUG-033**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2693) [P3] Установщик: первый экран выбора языка — латиницей вместо кириллицы · S · M8
- ⚪ [**BUG-034**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2698) [P3] ОС: в интерфейсах видны старые имена проекта и подсказка Cockpit · S · M8
- 🟡 [**BUG-035**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2703) [P0] Нативный плеер Twitch: эфир не играет (fMP4-сегменты), чат зависает на подключении · M · M10 · нет строк в каталоге проверок
- 🟡 [**BUG-036**](https://github.com/DenisHumen/kinolune/blob/9e4f6f6823f28b119cc96a0d4a4ed206f0dd4c5a/docs/ROADMAP.md#L2708) [P0] Чат Twitch со входом бесконечно висит на «Подключение к чату…» · S · M10 · нет строк в каталоге проверок

</details>

---

**Статусы.** ⚪ не начато — `[ ]` и нет открытого PR; 🔵 в работе — `[ ]` и открыт PR с `[ID]` в заголовке или ветка `тип/ID-…`; 🟡 сделано, не проверено — `[x]`, но проверки задачи в `docs/qa/test-catalog.md` не все ✅ (или у задачи приложения нет строк в каталоге); ✅ готово — `[x]` и проверки на устройствах ✅ (или проверять на устройстве нечего: решения, репозиторий, CI, документы).
