<!-- SPDX-License-Identifier: MPL-2.0 -->
<!-- progress-digest: e1b0f08f77364871435bea99a0c96128 -->

# Kinolune — прогресс

Обновлено: **2026-11-21 09:02 UTC** · main [`be429ca`](https://github.com/DenisHumen/kinolune/commit/be429ca372637d7f6b6cb5ee8104fedba0311cf6) · страница собирается сама из `docs/ROADMAP.md` и открытых PR ([как](https://github.com/DenisHumen/kinolune/blob/main/tools/roadmap/README.md#страница-прогресса)).

- **Kinolune** `▓▓▓▓▓▓▓▓▓▓░░░░░░░░░░ 49% (238/486)` · весь план без багов
- **Система** `▓▓▓▓▓▓▓▓▓▓░░░░░░░░░░ 52% (238/462)` · по объёму 49%
- **Эксперименты** `░░░░░░░░░░░░░░░░░░░░ 0% (0/9)` · M11, M12
- **Подписка** `░░░░░░░░░░░░░░░░░░░░ 0% (0/9)` · M13
- **Диски** `░░░░░░░░░░░░░░░░░░░░ 0% (0/6)` · M14
- **Баги** `▓▓▓▓▓▓▓▓░░░░░░░░░░░░ 39% (7/18)` · открыто: 11
- **Проверки** `░░░░░░░░░░░░░░░░░░░░ 0% (0/6532)` · ячейки каталога на устройствах; только автотесты: 2228
- **Прогноз** ≈ 32 дн. (≈ 23.12.2026) при скорости 10.8 задач/день за 4 дн.

| ⚪ не начато | 🔵 в работе | 🟡 сделано, не проверено | ✅ готово |
|---|---|---|---|
| **259** | **0** | **209** | **36** |

<details><summary><b>Дерево прогресса</b> (как <code>roadmap.py progress</code>)</summary>

```text
Kinolune                         ▓▓▓▓▓▓▓▓░░░░░░░░ 49% (238/486) · весь план
├─ Система                       ▓▓▓▓▓▓▓▓░░░░░░░░ 52% (238/462) · по объёму 49%
│  ├─ Прогноз                     ≈ 32 дн. (≈ 23.12.2026) при скорости 10.8 задач/день за 4 дн.
│  ├─ По размеру                  S — для малого лимита, L — после сброса
│  │  ├─ Маленькие (S)           ▓▓▓▓▓▓▓▓▓░░░░░░░ 58% (66/113) · готовы к работе: 12
│  │  ├─ Средние (M)             ▓▓▓▓▓▓▓▓░░░░░░░░ 51% (146/288) · готовы к работе: 56
│  │  └─ Большие (L)             ▓▓▓▓▓▓▓░░░░░░░░░ 43% (26/61) · готовы к работе: 11
│  ├─ По темам
│  │  ├─ Ядро и источники        ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓░ 92% (60/65) · объём 91% · S13/14 M42/45 L5/6
│  │  │  ├─ CORE                 ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓░ 94% (15/16) · S5/6 M10/10
│  │  │  ├─ NET                  ▓▓▓▓▓▓▓▓▓▓▓▓▓▓░░ 87% (20/23) · S3/3 M14/16 L3/4
│  │  │  └─ LIB                  ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓░ 96% (25/26) · S5/5 M18/19 L2/2
│  │  ├─ Распознавание           ▓▓▓▓▓▓▓▓▓▓▓▓░░░░ 74% (26/35) · объём 72% · S6/7 M15/21 L5/7
│  │  │  └─ META                 ▓▓▓▓▓▓▓▓▓▓▓▓░░░░ 74% (26/35) · S6/7 M15/21 L5/7
│  │  ├─ Плеер                   ▓▓▓▓▓▓░░░░░░░░░░ 38% (19/50) · объём 40% · S1/4 M13/34 L5/12
│  │  │  ├─ PLAY                 ▓▓▓▓▓▓▓▓▓▓▓▓░░░░ 72% (13/18) · S1/1 M9/12 L3/5
│  │  │  ├─ PLAY-AND             ▓▓▓▓▓▓▓▓░░░░░░░░ 50% (5/10) · M4/9 L1/1
│  │  │  ├─ PLAY-DSK             ░░░░░░░░░░░░░░░░ 0% (0/9) · S0/1 M0/6 L0/2
│  │  │  └─ PLAY-APL             ▓░░░░░░░░░░░░░░░ 8% (1/13) · S0/2 M0/7 L1/4
│  │  ├─ Интерфейс и ТВ          ▓▓▓▓▓▓▓▓▓░░░░░░░ 56% (46/82) · объём 48% · S14/16 M27/52 L5/14
│  │  │  ├─ UI                   ▓▓▓▓▓▓▓▓▓▓▓▓▓░░░ 83% (39/47) · S10/10 M25/33 L4/4
│  │  │  ├─ TV                   ▓░░░░░░░░░░░░░░░ 7% (1/14) · S1/2 M0/8 L0/4
│  │  │  └─ DSN                  ▓▓▓▓▓░░░░░░░░░░░ 29% (6/21) · S3/4 M2/11 L1/6
│  │  ├─ Синхронизация и сервер  ▓▓▓▓▓▓▓░░░░░░░░░ 45% (15/33) · объём 37% · S3/5 M11/22 L1/6
│  │  │  ├─ SYNC                 ▓▓▓▓▓▓▓▓▓▓▓▓▓░░░ 80% (12/15) · S1/1 M10/13 L1/1
│  │  │  └─ SRV                  ▓▓▓░░░░░░░░░░░░░ 17% (3/18) · S2/4 M1/9 L0/5
│  │  ├─ Музыка                  ▓▓▓▓▓▓░░░░░░░░░░ 37% (7/19) · объём 41% · S0/1 M5/14 L2/4
│  │  │  └─ MUS                  ▓▓▓▓▓▓░░░░░░░░░░ 37% (7/19) · S0/1 M5/14 L2/4
│  │  ├─ Twitch                  ▓▓▓▓▓▓▓▓▓▓░░░░░░ 60% (18/30) · объём 68% · S1/3 M14/24 L3/3
│  │  │  └─ TWI                  ▓▓▓▓▓▓▓▓▓▓░░░░░░ 60% (18/30) · S1/3 M14/24 L3/3
│  │  ├─ Дистрибутив             ▓▓░░░░░░░░░░░░░░ 10% (3/29) · объём 8% · S0/2 M3/21 L0/6
│  │  │  └─ DIST                 ▓▓░░░░░░░░░░░░░░ 10% (3/29) · S0/2 M3/21 L0/6
│  │  ├─ Сборка и релизы         ▓▓▓▓▓▓░░░░░░░░░░ 40% (21/52) · объём 28% · S15/25 M6/25 L0/2
│  │  │  ├─ PKG                  ▓▓▓▓░░░░░░░░░░░░ 28% (7/25) · S2/7 M5/16 L0/2
│  │  │  ├─ CI                   ▓▓▓▓▓░░░░░░░░░░░ 33% (4/12) · S4/6 M0/6
│  │  │  └─ REPO                 ▓▓▓▓▓▓▓▓▓▓▓░░░░░ 67% (10/15) · S9/12 M1/3
│  │  ├─ Качество и тесты        ▓▓▓▓▓▓▓░░░░░░░░░ 44% (7/16) · объём 45% · S1/3 M6/13
│  │  │  └─ QA                   ▓▓▓▓▓▓▓░░░░░░░░░ 44% (7/16) · S1/3 M6/13
│  │  ├─ Документы и право       ▓▓▓▓▓▓░░░░░░░░░░ 35% (15/43) · объём 28% · S11/26 M4/16 L0/1
│  │  │  ├─ DOC                  ▓▓▓▓▓░░░░░░░░░░░ 30% (3/10) · S0/3 M3/7
│  │  │  ├─ LEGAL                ▓▓▓▓▓▓░░░░░░░░░░ 40% (4/10) · S3/5 M1/5
│  │  │  ├─ SITE                 ░░░░░░░░░░░░░░░░ 0% (0/6) · S0/4 M0/2
│  │  │  ├─ COMM                 ▓▓░░░░░░░░░░░░░░ 11% (1/9) · S1/6 M0/2 L0/1
│  │  │  └─ DEC                  ▓▓▓▓▓▓▓▓▓▓▓▓▓▓░░ 88% (7/8) · S7/8
│  │  └─ Заготовки подписки      ▓▓░░░░░░░░░░░░░░ 12% (1/8) · объём 10% · S1/7 M0/1
│  │     └─ SUB                  ▓▓░░░░░░░░░░░░░░ 12% (1/8) · S1/7 M0/1
│  └─ Ждут владельца             14 · DEC-007, REPO-005, REPO-011, DIST-025, PKG-011, PKG-012 …
├─ Эксперименты                  ░░░░░░░░░░░░░░░░ 0% (0/9) · M11, M12
├─ Подписка                      ░░░░░░░░░░░░░░░░ 0% (0/9) · M13
├─ Диски                         ░░░░░░░░░░░░░░░░ 0% (0/6) · M14, последним
├─ Баги                          ▓▓▓▓▓▓░░░░░░░░░░ 39% (7/18) · исправлено; открыто: 11 (P0 1 · P1 3 · P2 7)
│  ├─ BUG-001                     [P0] Память растёт при каждом открытии и закрытии плеера
│  ├─ BUG-002                     [P1] Подвисание в первые 10 секунд первого просмотра по SMB
│  ├─ BUG-008                     [P2] Долгие потоки ядра (синхронизация, поиск в сети) отписываются тол
│  ├─ BUG-010                     [P1] База аниме не обновляется: сжатый выпуск читается как текст
│  ├─ BUG-011                     [P1] Фильм с единственным верным кандидатом «Название: подзаголовок» н
│  ├─ BUG-012                     [P2] Строка «… стоит проверить» на главной и счётчик проверки в настро
│  ├─ BUG-013                     [P2] «Источники» показывают сырую английскую ошибку хранилища ключей
│  ├─ BUG-014                     [P2] Серии нераспознанного сериала подписаны по-английски «Episode N»
│  ├─ BUG-015                     [P2] При перемотке в плеере падает построение семантики (`Null check o
│  ├─ BUG-016                     [P2] Плашка о хранилище ключей остаётся поверх видео в плеере
│  └─ BUG-017                     [P2] `tools/qa/e2e/run.sh` не собирает приложение в свежем клоне: нет
└─ Проверки                      ░░░░░░░░░░░░░░░░ 0% (0/6532) · на устройствах; только автотесты: 2228 (34%)
   ├─ Android                    ░░░░░░░░░░░░░░░░ 0% (0/968)
   ├─ Android TV                 ░░░░░░░░░░░░░░░░ 0% (0/955)
   ├─ iOS                        ░░░░░░░░░░░░░░░░ 0% (0/867)
   ├─ iPadOS                     ░░░░░░░░░░░░░░░░ 0% (0/868)
   ├─ macOS                      ░░░░░░░░░░░░░░░░ 0% (0/964)
   ├─ Windows                    ░░░░░░░░░░░░░░░░ 0% (0/945)
   ├─ Linux                      ░░░░░░░░░░░░░░░░ 0% (0/953)
   └─ tvOS                       ░░░░░░░░░░░░░░░░ 0% (0/12)
```

</details>

## По темам

<details><summary><b>Ядро и источники</b> <code>▓▓▓▓▓▓▓▓▓░ 92% (60/65)</code> · ⚪ 5 · 🔵 0 · 🟡 60 · ✅ 0</summary>

_CORE, NET, LIB_

- 🟡 [**CORE-001**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L309) `pa-ffi`: каркас UniFFI · S · M0 · нет строк в каталоге проверок
- 🟡 [**CORE-002**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L313) flutter_rust_bridge: пакет `pa_core` · M · M0 · нет строк в каталоге проверок
- 🟡 [**CORE-003**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L317) Apple: xcframework и SwiftPM `PACore` · M · M0 · нет строк в каталоге проверок
- 🟡 [**CORE-004**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L321) Android: AAR `pa-core-android` · S · M0 · нет строк в каталоге проверок
- 🟡 [**CORE-005**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L325) `pa-model`: доменные типы · M · M0 · нет строк в каталоге проверок
- 🟡 [**CORE-006**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L329) `pa-runtime`: жизненный цикл и конфигурация · M · M0 · нет строк в каталоге проверок
- 🟡 [**CORE-007**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L333) `pa-secrets`: хранилище секретов · M · M1 · нет строк в каталоге проверок
- 🟡 [**CORE-008**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L337) `pa-log`, диагностический отчёт, crash-отчёты по opt-in · M · M1 · нет строк в каталоге проверок
- 🟡 [**CORE-009**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L341) Шина событий и модель ошибок · S · M0 · нет строк в каталоге проверок
- ⚪ [**CORE-010**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L345) Smoke-приложения M0 на 7 платформах · S · M0
- 🟡 [**CORE-011**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L349) `pa-jobs`: персистентная очередь задач · M · M1 · нет строк в каталоге проверок
- 🟡 [**CORE-012**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L353) `pa-storage`: места хранения, квоты, retention · M · M4 · нет строк в каталоге проверок
- 🟡 [**CORE-013**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L357) Флейворы сборки `store` и `full` · M · M1 · нет строк в каталоге проверок
- 🟡 [**CORE-014**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L361) Клиент удалённого конфига без серверов · S · M2 · нет строк в каталоге проверок
- 🟡 [**CORE-015**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L365) Платформенные хранилища секретов: Android Keystore и Apple Keychain · M · M1 · нет строк в каталоге проверок
- 🟡 [**CORE-016**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L369) Диагностика: логи сканирования и проверка pa-log на Windows · S · M1 · нет строк в каталоге проверок
- 🟡 [**NET-001**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L376) `pa-vfs`: трейт Vfs и локальная ФС · S · M1 · нет строк в каталоге проверок
- 🟡 [**NET-002**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L380) libsmb2: сборка и `smb2-sys` · M · M1 · нет строк в каталоге проверок
- 🟡 [**NET-003**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L384) SMB2/3 бэкенд · L · M1 · нет строк в каталоге проверок
- 🟡 [**NET-004**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L388) WebDAV бэкенд · M · M1 · нет строк в каталоге проверок
- ⚪ [**NET-005**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L392) NFS v3/v4 бэкенд (libnfs) [P1] · M · M1
- 🟡 [**NET-006**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L396) SFTP и FTP/FTPS бэкенды [P1] · M · M1 · нет строк в каталоге проверок
- 🟡 [**NET-007**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L400) `pa-proxy`: loopback HTTP Range-сервер · S · M1 · нет строк в каталоге проверок
- 🟡 [**NET-008**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L404) Адаптеры ByteSource для FFmpeg и mpv · M · M1 · нет строк в каталоге проверок
- 🟡 [**NET-009**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L408) mDNS/DNS-SD обнаружение · M · M1 · нет строк в каталоге проверок
- 🟡 [**NET-010**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L412) WS-Discovery, NetBIOS, Plex GDM, Jellyfin UDP · M · M4 · нет строк в каталоге проверок
- 🟡 [**NET-011**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L416) Менеджер источников · M · M1 · нет строк в каталоге проверок
- 🟡 [**NET-012**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L420) Plex: вход, серверы, библиотеки · M · M4 · на устройствах ✅ 0/7
- 🟡 [**NET-013**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L424) Plex: воспроизведение, прогресс, транскод по запросу · M · M4 · на устройствах ✅ 0/28
- 🟡 [**NET-014**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L428) Jellyfin: клиент · L · M4 · нет строк в каталоге проверок
- 🟡 [**NET-015**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L432) Emby: клиент · M · M4 · нет строк в каталоге проверок
- 🟡 [**NET-016**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L436) Subsonic/OpenSubsonic клиент [P1] · M · M6 · нет строк в каталоге проверок
- ⚪ [**NET-017**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L440) Облачные хранилища [P1] · L · M4
- 🟡 [**NET-018**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L444) Прочие источники [P2]: UPnP/DLNA, `.strm`, M3U/M3U8 · M · M10 · нет строк в каталоге проверок
- 🟡 [**NET-019**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L448) Офлайн-загрузки из источника на устройство [P1] · M · M4 · нет строк в каталоге проверок
- 🟡 [**NET-020**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L453) Обвязка обнаружения в LAN на Android и Apple · S · M1 · нет строк в каталоге проверок
- 🟡 [**NET-021**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L457) SMB: скорость на каналах с потерями · M · M2 · на устройствах ✅ 0/20
- ⚪ [**NET-022**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L461) Фоновые офлайн-загрузки на iOS [P1] · M · M5
- 🟡 [**NET-023**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L466) Запись в источники: загрузка, папки, перенос, удаление · L · M3 · нет строк в каталоге проверок
- 🟡 [**LIB-001**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L473) Схема SQLite v1 и миграции · M · M1 · нет строк в каталоге проверок
- 🟡 [**LIB-002**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L477) Политика миграций expand→contract · S · M1 · нет строк в каталоге проверок
- 🟡 [**LIB-003**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L481) `pa-scanner`: поверхностный инкрементальный скан · M · M1 · нет строк в каталоге проверок
- 🟡 [**LIB-004**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L485) Отпечатки файлов: OSHash и ed2k · S · M1 · нет строк в каталоге проверок
- 🟡 [**LIB-005**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L489) Watcher изменений · M · M1 · нет строк в каталоге проверок
- 🟡 [**LIB-006**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L493) `pa-probe`: анализ контейнера · M · M1 · нет строк в каталоге проверок
- 🟡 [**LIB-007**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L497) Структурный анализ папок · M · M1 · нет строк в каталоге проверок
- 🟡 [**LIB-008**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L501) Поиск FTS5 · M · M1 · нет строк в каталоге проверок
- 🟡 [**LIB-009**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L505) Запросы библиотеки: фильтры, сортировки, виды · M · M1 · нет строк в каталоге проверок
- 🟡 [**LIB-010**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L509) Версии, коллекции, избранное, плейлисты · M · M1 · нет строк в каталоге проверок
- 🟡 [**LIB-011**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L513) Прогресс, история, «Продолжить», Up Next · M · M1 · нет строк в каталоге проверок
- 🟡 [**LIB-012**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L517) Профили и фильтры родительского контроля (данные) · M · M1 · нет строк в каталоге проверок
- 🟡 [**LIB-013**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L521) Кэш изображений · S · M1 · нет строк в каталоге проверок
- 🟡 [**LIB-014**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L525) Экспорт и импорт знаний: NFO и ID в имени [P1] · M · M2 · на устройствах ✅ 0/35
- 🟡 [**LIB-015**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L529) Импорт медиатеки pa_library v0 (релиз 0.1) · M · M1 · нет строк в каталоге проверок
- 🟡 [**LIB-016**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L534) Убрать уязвимый SQLite: refinery → rusqlite_migration · S · M1 · нет строк в каталоге проверок
- 🟡 [**LIB-017**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L538) Образы дисков в скане медиатеки · S · M2 · нет строк в каталоге проверок
- 🟡 [**LIB-018**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L542) Скан и воспроизведение сетевых источников через `pa-vfs` · L · M1 · нет строк в каталоге проверок
- 🟡 [**LIB-019**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L546) Сетевые источники в собранном приложении: libsmb2 в сборках и проверка на устройствах · M · M1 · нет строк в каталоге проверок
- 🟡 [**LIB-020**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L550) Диски и составные фильмы в приложении: причина «не играет» и склейка частей · M · M2 · нет строк в каталоге проверок
- 🟡 [**LIB-021**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L554) Blu-ray по плейлисту `.mpls` и субтитры частей · M · M2 · нет строк в каталоге проверок
- 🟡 [**LIB-022**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L558) Пользовательские категории, теги и оценки элементов · M · M3 · нет строк в каталоге проверок
- 🟡 [**LIB-023**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L562) Быстрое распознавание большой сетевой медиатеки без зависаний · L · M2 · нет строк в каталоге проверок
- 🟡 [**LIB-024**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L566) Автоматическое распознавание без кнопки «Сканировать» · M · M2 · нет строк в каталоге проверок
- 🟡 [**LIB-025**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L571) Тип содержимого определяется сам: фильмы, сериалы, музыка, аудиокниги, свои видео · M · M3 · нет строк в каталоге проверок
- ⚪ [**LIB-027**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L580) Закинуть файл прямо через приложение · M · M10

</details>

<details><summary><b>Распознавание</b> <code>▓▓▓▓▓▓▓░░░ 74% (26/35)</code> · ⚪ 9 · 🔵 0 · 🟡 26 · ✅ 0</summary>

_META_

- 🟡 [**META-001**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L587) Таксономия, гайд по разметке, формат корпуса · S · M0 · нет строк в каталоге проверок
- 🟡 [**META-002**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L591) Корпус v0: 2 000 групп · L · M2 · нет строк в каталоге проверок
- 🟡 [**META-003**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L595) Eval-раннер `pa-eval` · M · M2 · нет строк в каталоге проверок
- ⚪ [**META-004**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L599) Бейзлайны · M · M2
- 🟡 [**META-005**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L603) Парсер имён: токенизатор и N-best гипотезы · L · M1 · нет строк в каталоге проверок
- 🟡 [**META-006**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L607) Правила для аниме и русских релизов · M · M2 · нет строк в каталоге проверок
- 🟡 [**META-007**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L611) Нормализация и локальный индекс названий · L · M2 · нет строк в каталоге проверок
- 🟡 [**META-008**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L615) Встроенные маппинги аниме · M · M2 · нет строк в каталоге проверок
- 🟡 [**META-009**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L619) Трейт Provider и клиент TMDB · M · M1 · нет строк в каталоге проверок
- 🟡 [**META-010**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L623) Клиенты AniList и Shikimori · M · M2 · нет строк в каталоге проверок
- ⚪ [**META-011**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L627) AniDB: HTTP и UDP с аккаунтом пользователя · M · M2
- 🟡 [**META-012**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L631) Дополнительные провайдеры [P1] · M · M2 · нет строк в каталоге проверок
- 🟡 [**META-013**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L635) Базовый матчер для M1: название + год → TMDB · S · M1 · нет строк в каталоге проверок
- 🟡 [**META-014**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L639) Скорер v0 и совместный вывод по группе · L · M2 · нет строк в каталоге проверок
- 🟡 [**META-015**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L643) Модуль нумерации эпизодов · L · M2 · нет строк в каталоге проверок
- 🟡 [**META-016**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L647) Пороги, состояния матча, объяснения, API инбокса · M · M2 · нет строк в каталоге проверок
- 🟡 [**META-017**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L651) FolderRule и шаблоны релизов из исправлений · M · M2 · нет строк в каталоге проверок
- ⚪ [**META-018**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L655) Обученный и откалиброванный скорер · L · M2
- ⚪ [**META-019**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L659) Корпус v1: 10 000 групп и инструмент разметки · L · M2
- ⚪ [**META-020**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L663) LLM-fallback по opt-in · M · M2
- 🟡 [**META-021**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L668) Музыкальные метаданные · M · M6 · нет строк в каталоге проверок
- ⚪ [**META-022**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L672) Открытый датасет «хэш → ID» в git [P2] · M · M10
- ⚪ [**META-023**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L676) Парсер: замер на тест-кейсах guessit и anitomy в eval-репо и доводка до ≥ 85 % · M · M2
- 🟡 [**META-024**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L680) Адаптер парсера для `pa-eval`: метрики разбора имён · S · M2 · нет строк в каталоге проверок
- ⚪ [**META-025**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L684) Сверка draft-записей корпуса v0 · M · M2
- 🟡 [**META-026**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L688) Скорер кандидатов v0 по локальному индексу: работа, запись франшизы и эпизод · M · M2 · нет строк в каталоге проверок
- 🟡 [**META-027**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L692) Метаданные провайдеров в медиатеке на Rust-ядре · M · M2 · нет строк в каталоге проверок
- ⚪ [**META-028**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L696) Замер распознавания на реальных ответах TMDB · S · M2
- 🟡 [**META-029**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L700) Порядок AniDB у реальных аниме: маппинги Anime-Lists в `episode_mappings` · M · M2 · нет строк в каталоге проверок
- 🟡 [**META-030**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L704) База аниме в приложении с ежедневным обновлением · M · M2 · нет строк в каталоге проверок
- 🟡 [**META-031**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L708) Распознавание аниме по кадрам через trace.moe · M · M2 · нет строк в каталоге проверок
- 🟡 [**META-032**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L712) Распознавание реальных имён: нумерованные папки, мусор сайтов, сиквелы-сезоны, русские названия и постеры · M · M2 · нет строк в каталоге проверок
- 🟡 [**META-033**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L716) Английские и ромадзи-названия при метаданных на русском · S · M2 · нет строк в каталоге проверок
- 🟡 [**META-034**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L721) Проверка распознавания прямо в карточке, без настроек · S · M2 · нет строк в каталоге проверок
- 🟡 [**META-035**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L725) Нумерация, тип и «Забыть правило папки» в «Исправить распознавание» · S · M2 · на устройствах ✅ 0/70

</details>

<details><summary><b>Плеер</b> <code>▓▓▓▓░░░░░░ 38% (19/50)</code> · ⚪ 31 · 🔵 0 · 🟡 19 · ✅ 0</summary>

_PLAY, PLAY-AND, PLAY-DSK, PLAY-APL_

- 🟡 [**PLAY-001**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L732) LGPL-сборка FFmpeg для всех таргетов · L · M0 · нет строк в каталоге проверок
- 🟡 [**PLAY-002**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L736) LGPL-сборка libmpv · M · M1 · нет строк в каталоге проверок
- 🟡 [**PLAY-003**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L740) Контракт `PlayerEngine` и выбор движка · M · M1 · нет строк в каталоге проверок
- 🟡 [**PLAY-004**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L744) Политика дорожек и аудиообработки · M · M1 · нет строк в каталоге проверок
- 🟡 [**PLAY-005**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L748) Сессия воспроизведения · M · M1 · нет строк в каталоге проверок
- 🟡 [**PLAY-006**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L752) Рендер субтитров в ядре: libass, PGS, VobSub · L · M1 · нет строк в каталоге проверок
- 🟡 [**PLAY-007**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L756) Внешние субтитры и стиль · M · M1 · нет строк в каталоге проверок
- 🟡 [**PLAY-008**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L760) Поиск и загрузка субтитров · M · M1 · нет строк в каталоге проверок
- 🟡 [**PLAY-009**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L764) Trickplay-превью · M · M1 · нет строк в каталоге проверок
- 🟡 [**PLAY-010**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L768) Модель stats overlay и адаптивный буфер · M · M1 · нет строк в каталоге проверок
- 🟡 [**PLAY-011**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L772) Пропуск интро и титров [P1] · L · M4 · нет строк в каталоге проверок
- ⚪ [**PLAY-012**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L776) Google Cast и AirPlay [P1] · M · M5
- 🟡 [**PLAY-013**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L780) Наш libmpv в Android-сборке приложения вместо jar media_kit · S · M1 · нет строк в каталоге проверок
- ⚪ [**PLAY-014**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L784) libdovi в сборке libmpv · M · M2
- 🟡 [**PLAY-015**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L788) Импорт маркеров интро и титров Plex и Jellyfin [P2] · M · M4 · нет строк в каталоге проверок
- ⚪ [**PLAY-016**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L792) Второй провайдер субтитров: Subdl или Podnapisi [P1] · M · M4
- ⚪ [**PLAY-017**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L796) Blu-ray и образы дисков: всё играет само · L · M10
- ⚪ [**PLAY-018**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L800) Меню Blu-ray (HDMV) с переключателем «Показывать / Сразу фильм» · L · M10
- 🟡 [**PLAY-AND-001**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L819) Спайк: встраивание плеера во Flutter без потери HDR · M · M0 · нет строк в каталоге проверок
- 🟡 [**PLAY-AND-002**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L823) Плагин `pa_player_android` на Media3 · L · M1 · нет строк в каталоге проверок
- ⚪ [**PLAY-AND-003**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L827) Media3 DataSource поверх ByteSource · M · M1
- ⚪ [**PLAY-AND-004**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L832) Аудио: FFmpeg-расширение и passthrough · M · M1
- ⚪ [**PLAY-AND-005**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L836) HDR, Dolby Vision и AFR · M · M1
- 🟡 [**PLAY-AND-006**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L840) Оверлей субтитров libass и PGS · M · M1 · на устройствах ✅ 0/10
- ⚪ [**PLAY-AND-007**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L844) Запасной движок libmpv на Android · M · M1
- 🟡 [**PLAY-AND-008**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L848) MediaSession, фон и PiP · M · M1 · на устройствах ✅ 0/6
- 🟡 [**PLAY-AND-009**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L852) Интеграция с Android TV [P1] · M · M4 · нет строк в каталоге проверок
- ⚪ [**PLAY-AND-010**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L856) Stats overlay и матрица устройств · M · M1
- ⚪ [**PLAY-DSK-001**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L863) Спайк: libmpv во Flutter-десктопе · M · M0
- ⚪ [**PLAY-DSK-002**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L867) Плагин `pa_player_desktop` · L · M1
- ⚪ [**PLAY-DSK-003**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L871) HDR на десктопе · L · M1
- ⚪ [**PLAY-DSK-004**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L875) HW-декод и passthrough · M · M1
- ⚪ [**PLAY-DSK-005**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L879) Окна, полный экран и ввод · M · M1
- ⚪ [**PLAY-DSK-006**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L883) Системные медиаконтролы · S · M1
- ⚪ [**PLAY-DSK-007**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L887) Субтитры, зум, деинтерлейсинг и stats в mpv · M · M1
- ⚪ [**PLAY-DSK-008**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L891) AFR на десктопе [P2] · M · M10
- ⚪ [**PLAY-DSK-009**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L895) Замеры спайка libmpv на Windows, macOS и реальном Linux-композиторе · M · M1
- 🟡 [**PLAY-APL-001**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L902) `AlphaPlayerKit` и MPVKit (этап 1) · L · M5 · нет строк в каталоге проверок
- ⚪ [**PLAY-APL-002**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L906) Путь AVPlayer · M · M5
- ⚪ [**PLAY-APL-003**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L910) Мост ByteSource для Apple-движков · S · M5
- ⚪ [**PLAY-APL-004**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L914) Flutter-плагин `pa_player_apple` (iOS, iPadOS, macOS) · M · M5
- ⚪ [**PLAY-APL-005**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L918) Свой движок, шаг 1: FFmpeg → VideoToolbox → AVSampleBufferDisplayLayer (SDR) · L · M5
- ⚪ [**PLAY-APL-006**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L922) Свой движок, шаг 2: HDR10, HLG, DV P5 и P8.1 · L · M5
- ⚪ [**PLAY-APL-007**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L926) Свой движок, шаг 3: DV P7 → P8.1 на лету · M · M5
- ⚪ [**PLAY-APL-008**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L930) Аудио в своём движке · M · M5
- ⚪ [**PLAY-APL-009**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L934) AFR на tvOS и macOS · S · M5
- ⚪ [**PLAY-APL-010**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L938) Субтитры в своём движке · M · M5
- ⚪ [**PLAY-APL-011**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L942) Логика Auto и stats overlay на Apple · M · M5
- ⚪ [**PLAY-APL-012**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L946) Матрица устройств Apple · M · M5
- ⚪ [**PLAY-APL-013**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L950) Своя LGPL-2.1 сборка FFmpeg и libmpv для iOS и tvOS вместо MPVKit · L · M5

</details>

<details><summary><b>Интерфейс и ТВ</b> <code>▓▓▓▓▓▓░░░░ 56% (46/82)</code> · ⚪ 36 · 🔵 0 · 🟡 46 · ✅ 0</summary>

_UI, TV, DSN_

- 🟡 [**UI-001**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L957) Каркас Flutter-приложения · M · M0 · нет строк в каталоге проверок
- 🟡 [**UI-002**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L961) Адаптивная навигация · M · M1 · нет строк в каталоге проверок
- ⚪ [**UI-003**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L965) Тема из токенов и шрифты · M · M1
- 🟡 [**UI-004**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L969) Онбординг · M · M1 · нет строк в каталоге проверок
- 🟡 [**UI-005**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L973) Экран «Источники» и формы · M · M1 · нет строк в каталоге проверок
- 🟡 [**UI-006**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L977) Файловый браузер источника · S · M1 · нет строк в каталоге проверок
- 🟡 [**UI-007**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L981) Главная · M · M1 · нет строк в каталоге проверок
- 🟡 [**UI-008**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L985) Сетка библиотеки и контекстные меню · M · M1 · нет строк в каталоге проверок
- 🟡 [**UI-009**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L989) Detail фильма и страницы людей · M · M1 · нет строк в каталоге проверок
- 🟡 [**UI-010**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L993) Detail сериала, сезоны и эпизоды · M · M1 · нет строк в каталоге проверок
- 🟡 [**UI-011**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L997) Поиск · S · M1 · нет строк в каталоге проверок
- 🟡 [**UI-012**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1001) Инбокс «Нужна проверка» и «Исправить матч» · M · M2 · на устройствах ✅ 0/63
- 🟡 [**UI-013**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1005) OSD плеера · L · M1 · нет строк в каталоге проверок
- 🟡 [**UI-014**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1009) Панели дорожек и субтитров · M · M1 · нет строк в каталоге проверок
- 🟡 [**UI-015**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1013) Профили и родительский контроль · M · M1 · нет строк в каталоге проверок
- 🟡 [**UI-016**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1017) Настройки и «О программе» · M · M1 · нет строк в каталоге проверок
- 🟡 [**UI-017**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1021) Экран «Состояние» (диагностика) · S · M1 · на устройствах ✅ 0/56
- 🟡 [**UI-018**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1025) Экран загрузок [P1] · M · M4 · на устройствах ✅ 0/199
- 🟡 [**UI-019**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1029) Pairing по QR и коду · M · M3 · нет строк в каталоге проверок
- 🟡 [**UI-020**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1033) Локализация ru и en · M · M1 · нет строк в каталоге проверок
- 🟡 [**UI-023**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1037) Исправить баги со скриншотов дизайна 2026-11-14 · M · M1 · нет строк в каталоге проверок
- 🟡 [**UI-024**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1041) Приложение на Rust-ядре: медиатека, сканер, поиск, прогресс через `pa_core` · L · M2 · нет строк в каталоге проверок
- 🟡 [**UI-021**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1045) Доступность · M · M1 · нет строк в каталоге проверок
- ⚪ [**UI-022**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1049) Особенности платформ · M · M1
- 🟡 [**UI-025**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1053) Личные медиа: своя медиатека без распознавания · L · M3 · нет строк в каталоге проверок
- 🟡 [**UI-026**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1057) Внутренние баннеры уведомлений · M · M3 · нет строк в каталоге проверок
- 🟡 [**UI-027**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1061) Ввод текста на ТВ с телефона · M · M3 · на устройствах ✅ 0/15
- 🟡 [**UI-028**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1066) Простое добавление сетевой папки SMB · M · M3 · нет строк в каталоге проверок
- 🟡 [**UI-029**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1070) SMB на Windows и «Назад» в мастере добавления источника · S · M3 · нет строк в каталоге проверок
- 🟡 [**UI-030**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1074) SMB: вход, когда сервер не отдаёт список общих папок · S · M3 · нет строк в каталоге проверок
- 🟡 [**UI-031**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1078) SMB: сервер не показывает общие папки — ввести имя самому · S · M3 · нет строк в каталоге проверок
- 🟡 [**UI-032**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1082) Личные медиа: превью-кадры вместо плиток · M · M3 · нет строк в каталоге проверок
- 🟡 [**UI-033**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1086) Личные медиа: продолжение просмотра внутри библиотеки · S · M3 · нет строк в каталоге проверок
- 🟡 [**UI-034**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1090) Личные медиа: без выбора типа, «Перенести в Личное» у папки, отметка в списке источников · S · M3 · нет строк в каталоге проверок
- 🟡 [**UI-035**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1095) Файлы как в Nextcloud: загрузка с телефона, перенос, папки · L · M3 · нет строк в каталоге проверок
- ⚪ [**UI-036**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1100) Трейлеры: видео-баннер в карточке, на весь экран по «вверх» на пульте · M · M3
- 🟡 [**UI-037**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1104) Просмотренность на кадрах серий и постерах: резкая и размытая части · M · M3 · нет строк в каталоге проверок
- ⚪ [**UI-038**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1109) iOS и iPadOS: раннер Flutter-приложения, Files и ссылки · M · M5
- ⚪ [**UI-039**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1113) Загрузка файлов через браузер (web-upload) [P1] · M · M5
- ⚪ [**UI-040**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1117) Виджеты «Продолжить просмотр» [P2] · M · M5
- 🟡 [**UI-041**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1121) Настройки: обычные и расширенные · M · M10 · на устройствах ✅ 0/715
- 🟡 [**UI-042**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1125) Первый запуск в три шага · M · M10 · на устройствах ✅ 0/154
- 🟡 [**UI-043**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1129) Тихое распознавание · S · M10 · на устройствах ✅ 0/7
- 🟡 [**UI-044**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1133) Понятные ошибки и «Всё ли в порядке» · M · M10 · на устройствах ✅ 0/42
- ⚪ [**UI-045**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1137) Общий и голосовой поиск · M · M10
- ⚪ [**UI-046**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1141) Новое устройство по QR — главный способ · M · M10
- 🟡 [**UI-047**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1145) Режим «просто смотреть» · S · M10 · на устройствах ✅ 0/28
- 🟡 [**TV-001**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1156) Каркас tvOS-приложения на SwiftUI · S · M0 · нет строк в каталоге проверок
- ⚪ [**TV-002**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1160) Фокус-движок для Android TV во Flutter · L · M1
- ⚪ [**TV-003**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1164) TV-плотность компонентов · M · M1
- ⚪ [**TV-004**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1168) Специфика Android TV и производительность · M · M1
- ⚪ [**TV-005**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1172) Режим Big Picture на десктопе · M · M4
- ⚪ [**TV-006**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1176) tvOS: хранилище и навигационный каркас · M · M5
- ⚪ [**TV-007**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1180) tvOS: Главная, сетка и Detail · L · M5
- ⚪ [**TV-008**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1184) tvOS: источники и pairing с iPhone · M · M5
- ⚪ [**TV-009**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1188) tvOS: OSD плеера поверх AlphaPlayerKit · L · M5
- ⚪ [**TV-010**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1192) tvOS: Top Shelf и иконки · M · M5
- ⚪ [**TV-011**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1196) tvOS: профили, родительский контроль, настройки, «Состояние» · M · M5
- ⚪ [**TV-012**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1200) tvOS: раздел Twitch · L · M7
- ⚪ [**TV-013**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1204) tvOS: раздел «Музыка» · M · M6
- ⚪ [**TV-014**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1208) tvOS: баннеры уведомлений в приложении · S · M5
- 🟡 [**DSN-001**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1701) Инструмент и структура `design/` · S · M0 · нет строк в каталоге проверок
- 🟡 [**DSN-002**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1705) Токены W3C DTCG · M · M0 · нет строк в каталоге проверок
- 🟡 [**DSN-003**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1709) Пайплайн Style Dictionary и контраст-линт · S · M0 · нет строк в каталоге проверок
- 🟡 [**DSN-004**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1713) Набор иконок и конвейер · M · M1 · нет строк в каталоге проверок
- 🟡 [**DSN-005**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1717) Шрифты · S · M1 · нет строк в каталоге проверок
- 🟡 [**DSN-006**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1721) Бренд, иконки приложения, плейсхолдеры · L · M5 · нет строк в каталоге проверок
- ⚪ [**DSN-007**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1726) Иконки источников и сервисов · S · M4
- ⚪ [**DSN-008**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1730) Макеты: TV как референс · L · M1
- ⚪ [**DSN-009**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1734) Макеты: музыка и Twitch · M · M6
- ⚪ [**DSN-010**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1738) Спецификация компонентов · M · M1
- ⚪ [**DSN-011**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1742) [M9] Полировка фокуса и параллакса · M · M9
- ⚪ [**DSN-012**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1746) [M9] Материалы Glass и Liquid Glass, динамический акцент · L · M9
- ⚪ [**DSN-013**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1750) [M9] Движение (motion) · M · M9
- ⚪ [**DSN-014**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1754) Скриншоты для сторов · M · M10
- ⚪ [**DSN-015**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1758) [M9] Аудит визуальной консистентности · M · M9
- ⚪ [**DSN-016**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1762) Бренд: Apple-слои, MSIX, AppStream и иллюстрации пустых состояний · M · M5
- ⚪ [**DSN-017**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1766) Дизайн v2: концепция «одной умной главной» (уровень Netflix/Crunchyroll) · L · M10
- ⚪ [**DSN-018**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1770) Локальные рекомендации и изучение интересов · L · M10
- ⚪ [**DSN-019**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1774) Трейлеры: автопоиск, превью на карточке и внутри, выбор языка · M · M10
- ⚪ [**DSN-020**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1778) Реализация главной v2 во Flutter · L · M10
- ⚪ [**DSN-021**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1782) Полировка v2: анимации, переходы, пустые состояния, доступность · M · M10

</details>

<details><summary><b>Синхронизация и сервер</b> <code>▓▓▓▓▓░░░░░ 45% (15/33)</code> · ⚪ 18 · 🔵 0 · 🟡 15 · ✅ 0</summary>

_SYNC, SRV_

- 🟡 [**SYNC-001**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1215) `pa-sync`: модель, HLC, LWW и outbox · M · M3 · нет строк в каталоге проверок
- 🟡 [**SYNC-002**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1219) Протокол синхронизации · M · M3 · нет строк в каталоге проверок
- 🟡 [**SYNC-003**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1223) Эфемерная сессия воспроизведения · M · M3 · нет строк в каталоге проверок
- 🟡 [**SYNC-004**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1227) Сопоставление медиа между устройствами · M · M3 · нет строк в каталоге проверок
- 🟡 [**SYNC-005**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1231) Pairing устройств («семья устройств») · M · M3 · нет строк в каталоге проверок
- 🟡 [**SYNC-006**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1235) P2P-транспорт в LAN и выбор хаба · L · M3 · нет строк в каталоге проверок
- 🟡 [**SYNC-007**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1239) Синхронизация через хранилище · M · M3 · нет строк в каталоге проверок
- 🟡 [**SYNC-008**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1244) Хаб синхронизации на сервере · M · M4 · нет строк в каталоге проверок
- 🟡 [**SYNC-009**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1248) Профили в синхронизации · S · M3 · нет строк в каталоге проверок
- 🟡 [**SYNC-010**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1252) Синхронизация списка источников [P1] · M · M3 · нет строк в каталоге проверок
- ⚪ [**SYNC-011**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1256) Trakt [P1] · M · M3
- ⚪ [**SYNC-012**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1260) Тесты сходимости и задержки handoff · M · M3
- 🟡 [**SYNC-013**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1264) Синхронизация категорий, тегов и оценок · M · M3 · нет строк в каталоге проверок
- 🟡 [**SYNC-014**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1268) Прогресс каждую секунду и мгновенная синхронизация · M · M10 · на устройствах ✅ 0/21
- ⚪ [**SYNC-015**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1272) Семья без настройки · M · M10
- 🟡 [**SRV-001**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1280) Каркас сервера · M · M4 · нет строк в каталоге проверок
- 🟡 [**SRV-002**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1284) OpenAPI и контрактные тесты · S · M4 · нет строк в каталоге проверок
- ⚪ [**SRV-003**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1288) Аутентификация и pairing клиентов · M · M4
- 🟡 [**SRV-004**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1292) TLS и mDNS-анонс · S · M4 · нет строк в каталоге проверок
- ⚪ [**SRV-005**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1296) Библиотеки на сервере · M · M4
- ⚪ [**SRV-006**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1300) Media API и клиент своего сервера в ядре · L · M4
- ⚪ [**SRV-007**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1304) Direct play · S · M4
- ⚪ [**SRV-008**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1308) Транскод-менеджер · L · M4
- ⚪ [**SRV-009**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1312) HW-ускорение и тон-маппинг · L · M4
- ⚪ [**SRV-010**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1316) Remux для AVPlayer и транскод аудио · M · M4
- ⚪ [**SRV-011**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1320) API задач и события WebSocket · M · M4
- ⚪ [**SRV-012**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1324) «Оптимизировать для загрузки» [P1] · S · M4
- ⚪ [**SRV-013**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1328) Удалённый доступ через iroh · L · M4
- ⚪ [**SRV-014**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1333) Веб-админка сервера · L · M4
- ⚪ [**SRV-015**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1337) Хранилище и retention на сервере · M · M4
- ⚪ [**SRV-016**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1341) [full] Twitch-задачи на сервере · M · M7
- ⚪ [**SRV-017**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1345) Нагрузочные тесты и безопасность сервера · M · M4
- ⚪ [**SRV-018**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1349) Сборки сервера · M · M4

</details>

<details><summary><b>Музыка</b> <code>▓▓▓▓░░░░░░ 37% (7/19)</code> · ⚪ 12 · 🔵 0 · 🟡 7 · ✅ 0</summary>

_MUS_

- 🟡 [**MUS-001**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1356) Музыкальная библиотека: схема и скан тегов · M · M6 · нет строк в каталоге проверок
- 🟡 [**MUS-002**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1360) Воспроизведение v1 через libmpv в аудиорежиме · M · M6 · нет строк в каталоге проверок
- 🟡 [**MUS-003**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1364) Очередь и плейлисты · M · M6 · нет строк в каталоге проверок
- 🟡 [**MUS-004**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1368) UI музыкальной библиотеки · L · M6 · нет строк в каталоге проверок
- ⚪ [**MUS-005**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1372) Now Playing и мини-плеер · M · M6
- ⚪ [**MUS-006**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1376) Тексты песен · M · M6
- ⚪ [**MUS-007**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1380) Системная интеграция аудио · M · M6
- ⚪ [**MUS-008**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1384) Свой аудиодвижок `pa-audio` · L · M6
- ⚪ [**MUS-009**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1388) Bit-perfect вывод · L · M6
- ⚪ [**MUS-010**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1392) DSD · M · M6
- ⚪ [**MUS-011**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1396) Обёртки `pa-audio` для систем · M · M6
- 🟡 [**MUS-012**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1400) Музыка из Plex, Jellyfin и Subsonic, офлайн-альбомы [P1] · M · M6 · на устройствах ✅ 0/7
- ⚪ [**MUS-013**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1405) Android Auto [P2] · M · M10
- ⚪ [**MUS-014**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1409) CarPlay Audio [P2] · M · M10
- ⚪ [**MUS-015**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1413) Продолжение музыки на другом устройстве, скробблинг, эквалайзер [P1/P2] · M · M6
- ⚪ [**MUS-016**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1417) Аудиорежим MPVKit на Apple (`AudioKit`) · S · M6
- ⚪ [**MUS-017**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1421) Распознавание музыки по звуку (AcoustID/Chromaprint) · M · M6
- 🟡 [**MUS-018**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1425) Аудиокниги: медиатека и плеер · L · M6 · нет строк в каталоге проверок
- 🟡 [**MUS-019**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1429) Аудиокниги: распознавание и метаданные · M · M6 · нет строк в каталоге проверок

</details>

<details><summary><b>Twitch</b> <code>▓▓▓▓▓▓░░░░ 60% (18/30)</code> · ⚪ 12 · 🔵 0 · 🟡 18 · ✅ 0</summary>

_TWI_

- 🟡 [**TWI-001**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1440) `pa-twitch`: клиент Helix · M · M7 · нет строк в каталоге проверок
- 🟡 [**TWI-002**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1444) Вход через Device Code Flow · M · M7 · нет строк в каталоге проверок
- ⚪ [**TWI-003**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1448) Прототип режима «без логина» · M · M7
- 🟡 [**TWI-004**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1452) Воспроизведение: GQL → usher и удалённый конфиг · M · M7 · нет строк в каталоге проверок
- 🟡 [**TWI-005**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1456) Запасной режим: официальный embed · M · M7 · нет строк в каталоге проверок
- 🟡 [**TWI-006**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1460) Live: низкая задержка и реклама «как есть» · M · M7 · нет строк в каталоге проверок
- 🟡 [**TWI-007**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1464) Модель данных Twitch в библиотеке · M · M7 · нет строк в каталоге проверок
- 🟡 [**TWI-008**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1468) Главы и категории VOD · M · M7 · нет строк в каталоге проверок
- 🟡 [**TWI-009**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1472) Уведомления «X в эфире» без облака · M · M7 · нет строк в каталоге проверок
- 🟡 [**TWI-010**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1476) Live-чат: абстракция ChatSource · M · M7 · нет строк в каталоге проверок
- 🟡 [**TWI-011**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1480) Эмоуты и бейджи · M · M7 · нет строк в каталоге проверок
- 🟡 [**TWI-012**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1484) Отрисовка чата · M · M7 · нет строк в каталоге проверок
- 🟡 [**TWI-013**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1488) UI раздела Twitch (Flutter) и брендинг · L · M7 · нет строк в каталоге проверок
- 🟡 [**TWI-014**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1492) Плеер Twitch · M · M7 · нет строк в каталоге проверок
- ⚪ [**TWI-015**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1496) Ключи синхронизации и выбор источника · M · M7
- ⚪ [**TWI-016**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1500) [full] Чат-replay VOD онлайн · M · M7
- 🟡 [**TWI-017**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1504) [full] Загрузчик VOD · L · M7 · нет строк в каталоге проверок
- 🟡 [**TWI-018**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1508) [full] Запись live · L · M7 · нет строк в каталоге проверок
- 🟡 [**TWI-019**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1513) [full] Хранилище, имена файлов, импорт, правила записи · M · M7 · нет строк в каталоге проверок
- ⚪ [**TWI-020**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1517) Канареечный тест Twitch в nightly CI · S · M7
- ⚪ [**TWI-021**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1521) Twitch на Android TV · M · M7
- ⚪ [**TWI-022**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1525) Мультипросмотр 2–4 стримов [P2] · M · M10
- ⚪ [**TWI-023**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1529) Сторонние эмоуты 7TV, BetterTTV, FFZ в чате · M · M10
- 🟡 [**TWI-024**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1533) Фильтры и ввод в чате · M · M7 · нет строк в каталоге проверок
- ⚪ [**TWI-025**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1537) [full] Live DVR по окну плейлиста и «Смотреть с начала эфира» · M · M7
- ⚪ [**TWI-026**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1541) [full] Архив стримов как сериал: стример → игра → части · M · M7
- 🟡 [**TWI-027**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1545) Быстрый переход по стриму: «Пропустить болтовню» · S · M7 · нет строк в каталоге проверок
- ⚪ [**TWI-028**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1549) [full] Сохранение «всё вместе»: видео, чат, игра, метаданные · M · M7
- ⚪ [**TWI-029**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1553) Официальный плеер Twitch на Windows и Linux · M · M10
- ⚪ [**TWI-030**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1557) [full] Включить нативный плеер Twitch через подписанный конфиг · S · M10

</details>

<details><summary><b>Дистрибутив</b> <code>▓░░░░░░░░░ 10% (3/29)</code> · ⚪ 26 · 🔵 0 · 🟡 3 · ✅ 0</summary>

_DIST_

- 🟡 [**DIST-001**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1564) Прототип образа · M · M8 · нет строк в каталоге проверок
- 🟡 [**DIST-002**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1568) Сборка образа в CI и политика подписи · M · M8 · нет строк в каталоге проверок
- ⚪ [**DIST-003**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1572) Брендинг ОС · S · M8
- 🟡 [**DIST-004**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1576) Установщик: обнаружение дисков (Go) · M · M8 · нет строк в каталоге проверок
- ⚪ [**DIST-005**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1580) Установщик: разметка, `bootc install`, конфигурация, файл ответов · L · M8
- ⚪ [**DIST-006**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1584) Установщик: TUI (bubbletea) · M · M8
- ⚪ [**DIST-007**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1588) Live-ISO · M · M8
- ⚪ [**DIST-008**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1592) QEMU-тесты установки в CI · M · M8
- ⚪ [**DIST-009**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1596) `alpha-updated`: агент обновлений · M · M8
- ⚪ [**DIST-010**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1600) Снапшоты `/var` и откат состояния · M · M8
- ⚪ [**DIST-011**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1604) greenboot и тесты обновления и отката · M · M8
- ⚪ [**DIST-012**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1608) Каналы и поэтапная раскатка · M · M8
- ⚪ [**DIST-013**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1612) `alpha-admin`: бэкенд (Go) · L · M8
- ⚪ [**DIST-014**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1616) `alpha-admin`: веб-интерфейс · L · M8
- ⚪ [**DIST-015**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1620) Сервис хранилища и мастер пулов · L · M8
- ⚪ [**DIST-016**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1624) Обслуживание и уведомления · M · M8
- ⚪ [**DIST-017**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1628) Мастер замены диска · M · M8
- ⚪ [**DIST-018**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1632) Samba, пользователи и firewall · M · M8
- ⚪ [**DIST-019**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1636) Медиасервер в образе и VAAPI · M · M8
- ⚪ [**DIST-020**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1640) Вариант образа с NVIDIA [фаза 2] · L · M8
- ⚪ [**DIST-021**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1644) Веб-установщик для NAS без монитора [фаза 2] · M · M10
- ⚪ [**DIST-022**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1648) LUKS2 + TPM2 [фаза 2] · M · M10
- ⚪ [**DIST-023**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1652) Режим «Медиа-архив»: mergerfs + SnapRAID [фаза 2] · M · M10
- ⚪ [**DIST-024**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1656) Хостинг ISO · S · M8
- ⚪ [**DIST-025**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1660) (owner) Раннер с реальным железом · M · M8 · владелец
- ⚪ [**DIST-026**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1664) Переход на новую major-версию Fedora (45 → 46) · M · M10
- ⚪ [**DIST-027**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1668) Бэкапы и снапшоты пулов [P2] · L · M10
- ⚪ [**DIST-032**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1690) Wi-Fi и сеть прямо с главной · M · M8
- ⚪ [**DIST-033**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1694) Сервер Kinolune: настройка с телефона в пару нажатий · M · M10

</details>

<details><summary><b>Сборка и релизы</b> <code>▓▓▓▓░░░░░░ 40% (21/52)</code> · ⚪ 31 · 🔵 0 · 🟡 7 · ✅ 14</summary>

_PKG, CI, REPO_

- ✅ [**REPO-001**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L195) Каркас монорепо · S · M0 · проверка на устройстве не нужна
- ✅ [**REPO-002**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L199) LICENSE, SPDX-заголовки и REUSE · S · M0 · проверка на устройстве не нужна
- ✅ [**REPO-003**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L203) CONTRIBUTING, CODE_OF_CONDUCT, SECURITY · S · M0 · проверка на устройстве не нужна
- ✅ [**REPO-004**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L207) DCO-проверка и линт заголовка PR · S · M0 · проверка на устройстве не нужна
- ⚪ [**REPO-005**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L211) (owner) CODEOWNERS, rulesets и окружения · S · M0 · владелец
- ✅ [**REPO-006**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L215) Шаблоны issue и PR, метки как код · S · M0 · проверка на устройстве не нужна
- ✅ [**REPO-007**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L219) AGENTS.md и CLAUDE.md для агентов · S · M0 · проверка на устройстве не нужна
- ⚪ [**REPO-008**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L223) Закрепить toolchain, `just bootstrap`, Dev Container · M · M0
- ✅ [**REPO-009**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L227) Политика секретов · S · M0 · проверка на устройстве не нужна
- ⚪ [**REPO-010**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L231) release-please (manifest mode) · S · M0
- ⚪ [**REPO-011**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L235) (owner) Подготовка и переключение репо в публичный режим · S · M0 · владелец
- ✅ [**REPO-012**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L239) Инструменты ROADMAP: `just roadmap lint` и `just roadmap next` · S · M0 · проверка на устройстве не нужна
- ✅ [**REPO-013**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L243) Dependabot или Renovate · S · M0 · проверка на устройстве не нужна
- ⚪ [**REPO-014**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L247) Аккуратная структура репозитория (делать последним в основном плане) · M · M10
- ✅ [**REPO-015**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L251) Живая страница прогресса в ветке `progress` · M · M0 · проверка на устройстве не нужна
- ✅ [**CI-001**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L258) `rust.yml`: fmt, clippy, nextest · S · M0 · проверка на устройстве не нужна
- ✅ [**CI-002**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L262) Безопасный CI для PR из форков · S · M0 · проверка на устройстве не нужна
- ✅ [**CI-003**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L266) `cargo deny` и `cargo audit` · S · M0 · проверка на устройстве не нужна
- ⚪ [**CI-004**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L270) Матрица Rust-таргетов и sccache · M · M0
- ⚪ [**CI-005**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L274) `flutter.yml` · M · M0
- ⚪ [**CI-006**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L278) `android.yml`: нативные модули и эмулятор · M · M1
- ⚪ [**CI-007**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L282) `apple.yml` на macOS-runner · M · M0
- ✅ [**CI-008**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L286) Кэш и раздача артефактов FFmpeg и libmpv · S · M1 · проверка на устройстве не нужна
- ⚪ [**CI-009**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L290) Golden-скриншоты Flutter по плотностям · S · M1
- ⚪ [**CI-010**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L294) Eval распознавания на каждый PR · S · M2
- ⚪ [**CI-011**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L298) Nightly и релизный пайплайн · M · M1
- ⚪ [**CI-012**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L302) Сетевые интеграционные тесты · M · M1
- ⚪ [**PKG-001**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1789) Раскладка артефактов, подписи и `releases.json` · S · M1
- ⚪ [**PKG-002**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1793) Linux: AppImage, deb, rpm · M · M1
- ⚪ [**PKG-003**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1798) Flathub · M · M10
- ⚪ [**PKG-004**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1802) AUR · S · M10
- ⚪ [**PKG-005**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1806) Windows: установщик, автообновление, подпись · M · M1
- ⚪ [**PKG-006**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1810) winget · S · M10
- ⚪ [**PKG-007**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1814) [store] Microsoft Store · M · M10
- ⚪ [**PKG-008**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1818) macOS: DMG, нотаризация, обновления, Homebrew · M · M5
- ⚪ [**PKG-009**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1822) Android: APK в GitHub Releases · S · M1
- ⚪ [**PKG-010**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1826) F-Droid · L · M10
- ⚪ [**PKG-011**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1830) (owner) [store] Google Play · L · M10 · владелец
- ⚪ [**PKG-012**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1834) (owner) Apple Developer Program и конвейер TestFlight · M · M5 · владелец
- ⚪ [**PKG-013**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1838) Пакет для ревью App Store · M · M10
- ⚪ [**PKG-014**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1842) Матрица флейворов store и full по каналам · S · M1
- ⚪ [**PKG-015**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1846) [store] Чаевые через IAP или удаление экрана донатов · M · M10
- ⚪ [**PKG-016**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1850) Пакет соответствия LGPL · M · M1
- ⚪ [**PKG-017**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1854) Автообновление десктопа: каналы и UX · M · M5
- 🟡 [**PKG-018**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1858) Обновление из приложения (full-флейвор) · M · M1 · нет строк в каталоге проверок
- 🟡 [**PKG-019**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1862) macOS: неподписанная сборка приложения в GitHub Releases · M · M1 · нет строк в каталоге проверок
- 🟡 [**PKG-020**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1866) Windows: портативная сборка приложения в GitHub Releases · M · M1 · нет строк в каталоге проверок
- 🟡 [**PKG-021**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1870) macOS: падение при старте видео (CoreAudio, alpha.11) и проверка воспроизведения в CI · S · M1 · нет строк в каталоге проверок
- 🟡 [**PKG-022**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1875) Установщик Windows и автообновление на Windows, macOS и Linux · M · M1 · нет строк в каталоге проверок
- 🟡 [**PKG-023**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1879) Репозитории APT и DNF: установка из терминала и обновление через `apt upgrade` / `dnf upgrade` · M · M8 · нет строк в каталоге проверок
- 🟡 [**PKG-024**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1884) Тихие обновления · S · M10 · на устройствах ✅ 0/16
- ⚪ [**PKG-025**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1888) Предрелизная проверка сборки на сервере сборки · M · M10

</details>

<details><summary><b>Качество и тесты</b> <code>▓▓▓▓░░░░░░ 44% (7/16)</code> · ⚪ 9 · 🔵 0 · 🟡 1 · ✅ 6</summary>

_QA_

- ✅ [**QA-001**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1895) Тест-стратегия · S · M0 · проверка на устройстве не нужна
- ✅ [**QA-002**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1899) Тестовые медиа-сэмплы · M · M0 · проверка на устройстве не нужна
- ⚪ [**QA-003**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1903) Матрица воспроизведения (smoke) · M · M1
- ⚪ [**QA-004**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1907) E2E-тесты Flutter · M · M1
- ⚪ [**QA-005**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1911) Фаззинг · M · M3
- ✅ [**QA-006**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1915) Бюджеты производительности UI · M · M1 · проверка на устройстве не нужна
- 🟡 [**QA-007**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1919) Тесты на плохой сети · M · M1 · на устройствах ✅ 0/7
- ⚪ [**QA-008**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1923) Релизный чек-лист · S · M1
- ⚪ [**QA-009**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1927) Аудит доступности и локализации · M · M1
- ⚪ [**QA-010**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1931) Модель угроз и ревью безопасности · M · M4
- ⚪ [**QA-011**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1935) (owner) Лаборатория устройств и бета-программа · M · M1 · владелец
- ⚪ [**QA-012**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1939) Отчёты об ошибках от пользователей по кнопке (после редизайна) · M · M10
- ⚪ [**QA-013**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1943) (owner) Дежурный агент по почте: исправление ошибок из отчётов · S · M10 · владелец
- ✅ [**QA-014**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1948) Каталог проверок: все кнопки, меню и функции на всех платформах · M · M3 · проверка на устройстве не нужна
- ✅ [**QA-015**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1952) Исправления по каталогу проверок · M · M3 · проверка на устройстве не нужна
- ✅ [**QA-016**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1956) Живые e2e-тесты: настоящее приложение, сеть и SMB · M · M10 · проверка на устройстве не нужна

</details>

<details><summary><b>Документы и право</b> <code>▓▓▓░░░░░░░ 35% (15/43)</code> · ⚪ 28 · 🔵 0 · 🟡 0 · ✅ 15</summary>

_DOC, LEGAL, SITE, COMM, DEC_

- ✅ [**DEC-001**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L132) (owner) Сделать `DenisHumen/kinolune` публичным; решить судьбу `project_alpha-releases` · S · M0 · проверка на устройстве не нужна
- ✅ [**DEC-002**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L136) (owner) Лицензия кода: MPL-2.0 (рекомендация) или GPL-3.0 · S · M0 · проверка на устройстве не нужна
- ✅ [**DEC-003**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L140) (owner) Донаты в store-сборках: без ссылок или «чаевые» через IAP · S · M5 · проверка на устройстве не нужна
- ✅ [**DEC-004**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L144) (owner) Название и бренд продукта (и имя NAS-ОС) · S · M1 · проверка на устройстве не нужна
- ✅ [**DEC-005**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L148) (owner) Юридическая позиция по кодекам (H.264/HEVC/AAC, RPM Fusion, своя сборка FFmpeg на сервере) · S · M4 · проверка на устройстве не нужна
- ✅ [**DEC-006**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L152) (owner) Twitch без логина: публичные данные через GQL или только вход через Device Code Flow · S · M7 · проверка на устройстве не нужна
- ⚪ [**DEC-007**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L156) (owner) Аккаунты и юрисдикция: Apple Developer, Google Play, SignPath, Microsoft Store · S · M1 · владелец
- ✅ [**DEC-008**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L160) (owner) Ключ TMDB в открытом коде: встроенный ключ проекта по умолчанию или обязательный свой ключ · S · M2 · проверка на устройстве не нужна
- ✅ [**DOC-001**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1963) Базовый набор ADR · M · M0 · проверка на устройстве не нужна
- ⚪ [**DOC-002**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1967) Настройка среды разработчика · S · M0
- ✅ [**DOC-003**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1971) Обзор архитектуры · M · M1 · проверка на устройстве не нужна
- ⚪ [**DOC-004**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1975) Документация API · S · M4
- ✅ [**DOC-005**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1979) Пользовательская документация: приложение · M · M1 · проверка на устройстве не нужна
- ⚪ [**DOC-006**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1983) Пользовательская документация: сервер · M · M4
- ⚪ [**DOC-007**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1987) Пользовательская документация: NAS-ОС · M · M8
- ⚪ [**DOC-008**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1991) Пользовательская документация: Twitch и музыка · S · M7
- ⚪ [**DOC-009**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1995) Runbooks · M · M7
- ⚪ [**DOC-010**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1999) Отчёт о качестве распознавания · M · M10
- ⚪ [**LEGAL-001**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L2006) (owner) Письмо в TMDB · S · M0 · владелец
- ✅ [**LEGAL-002**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L2010) Таблица условий провайдеров · M · M1 · проверка на устройстве не нужна
- ✅ [**LEGAL-003**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L2014) Лицензии данных маппингов аниме · S · M1 · проверка на устройстве не нужна
- ⚪ [**LEGAL-004**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L2018) (owner) Патенты кодеков, Dolby и DTS · M · M4 · владелец
- ✅ [**LEGAL-005**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L2022) Проверка названия и товарных знаков · S · M0 · проверка на устройстве не нужна
- ⚪ [**LEGAL-006**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L2026) (owner) Twitch: ToS, Developer Agreement, регистрация приложения · M · M7 · владелец
- ✅ [**LEGAL-007**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L2030) Политика конфиденциальности · S · M1 · проверка на устройстве не нужна
- ⚪ [**LEGAL-008**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L2034) Политика донатов в сторах · S · M10
- ⚪ [**LEGAL-009**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L2038) Аудит лицензий зависимостей и ассетов · M · M1
- ⚪ [**LEGAL-010**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L2042) Юридические вопросы дистрибутива · M · M8
- ⚪ [**SITE-001**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L2053) Каркас сайта · M · M1
- ⚪ [**SITE-002**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L2057) Страница «Скачать» · M · M10
- ⚪ [**SITE-003**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L2061) Документация и API на сайте · S · M4
- ⚪ [**SITE-004**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L2065) Хостинг подписанных манифестов · S · M1
- ⚪ [**SITE-005**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L2069) Донаты и публичный roadmap · S · M1
- ⚪ [**SITE-006**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L2073) (owner) Домен и поддомен загрузок · S · M8 · владелец
- ⚪ [**COMM-001**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L2080) (owner) FUNDING.yml и площадки донатов · S · M0 · владелец
- ⚪ [**COMM-002**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L2084) GitHub Discussions и запросы функций · S · M0
- ✅ [**COMM-003**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L2088) README на русском и английском · S · M0 · проверка на устройстве не нужна
- ⚪ [**COMM-004**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L2092) Good first issues из ROADMAP · S · M1
- ⚪ [**COMM-005**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L2096) (owner) Каналы общения и анонсы · S · M1 · владелец
- ⚪ [**COMM-006**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L2100) Переводы силами сообщества · S · M10
- ⚪ [**COMM-007**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L2104) Промо-ролики высочайшего качества · L · M10
- ⚪ [**COMM-008**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L2108) Запуск и продвижение: аудитория до подписки · M · M10
- ⚪ [**COMM-009**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L2112) Короткие смешные ролики для TikTok через Blender · M · M10

</details>

<details><summary><b>Заготовки подписки</b> <code>▓░░░░░░░░░ 12% (1/8)</code> · ⚪ 7 · 🔵 0 · 🟡 0 · ✅ 1</summary>

_SUB_

- ✅ [**SUB-001**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L2144) Стратегия и ADR готовности к подписке · S · M10 · проверка на устройстве не нужна
- ⚪ [**SUB-002**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L2148) `Entitlements`: одна точка решения доступности · S · M10
- ⚪ [**SUB-003**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L2152) `ApiKeySource`: ключи провайдеров в одном месте · S · M10
- ⚪ [**SUB-004**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L2156) `SyncBackend`: общий интерфейс доставки журнала · M · M10
- ⚪ [**SUB-005**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L2160) `Notifier`: уведомления через один интерфейс · S · M10
- ⚪ [**SUB-006**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L2164) `RemoteAccess`: транспорт вне дома · S · M10
- ⚪ [**SUB-007**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L2168) `AiProvider`: модель для ИИ-функций · S · M10
- ⚪ [**SUB-008**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L2172) Канал обновлений «бета» · S · M10

</details>

<details><summary><b>Эксперименты</b> <code>░░░░░░░░░░ 0% (0/9)</code> · ⚪ 9 · 🔵 0 · 🟡 0 · ✅ 0</summary>

_M11, M12_

- ⚪ [**PLAY-019**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L804) Меню Blu-ray BD-J (Java) на компьютере и сервере [может быть] · L · M12
- ⚪ [**DIST-028**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1673) ТВ-оболочка ОС: исследование и ADR [может быть] · S · M12
- ⚪ [**DIST-029**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1677) Kinolune-сессия: вход сразу в полноэкранный Kinolune [может быть] · L · M12
- ⚪ [**DIST-030**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1681) Переключение «Рабочий стол ↔ Kinolune» и доработанный KDE Plasma [может быть] · M · M12
- ⚪ [**DIST-031**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1685) Системные настройки внутри оболочки [может быть] · L · M12
- ⚪ [**EXT-001**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L2123) watchOS и visionOS: исследование и ADR [P2] · S · M11
- ⚪ [**EXT-002**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L2127) Apple Watch: пульт воспроизведения [P2] · L · M11
- ⚪ [**EXT-003**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L2131) Apple Watch: уведомления и «сейчас играет» [P2] · M · M11
- ⚪ [**EXT-004**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L2135) visionOS: экспериментальный клиент [P2] · L · M11

</details>

<details><summary><b>Подписка</b> <code>░░░░░░░░░░ 0% (0/9)</code> · ⚪ 9 · 🔵 0 · 🟡 0 · ✅ 0</summary>

_M13_

- ⚪ [**SUB-010**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L2176) Аккаунт и оплата Kinolune+ · L · M13
- ⚪ [**SUB-011**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L2180) Наш ретранслятор для доступа вне дома · M · M13
- ⚪ [**SUB-012**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L2184) Облачная синхронизация и бэкап (E2E) · M · M13
- ⚪ [**SUB-013**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L2188) Push-уведомления · M · M13
- ⚪ [**SUB-014**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L2192) Прокси метаданных и субтитров с квотами · M · M13
- ⚪ [**SUB-015**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L2196) ИИ-функции Kinolune+ · L · M13
- ⚪ [**SUB-016**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L2200) Магазины: модель продажи · M · M13
- ⚪ [**SUB-017**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L2204) Юридическое: условия, конфиденциальность, налоги · M · M13
- ⚪ [**SUB-018**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L2208) Сайт и стратегия v3 · S · M13

</details>

<details><summary><b>Диски</b> <code>░░░░░░░░░░ 0% (0/6)</code> · ⚪ 6 · 🔵 0 · 🟡 0 · ✅ 0</summary>

_M14_

- ⚪ [**LIB-026**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L575) Копирование дисков: вопрос открыт (blocked: LEGAL-011) · L · M14
- ⚪ [**PLAY-020**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L808) Kinolune как Blu-ray/DVD-плеер: смотреть диск прямо с привода · M · M14
- ⚪ [**PLAY-021**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L812) (owner) Защищённые Blu-ray: лицензия AACS для воспроизведения · M · M14 · владелец
- ⚪ [**UI-048**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1149) Баннер «Найден диск» на всех устройствах · S · M14
- ⚪ [**MUS-020**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L1433) Аудио-CD в FLAC одним нажатием (blocked: LEGAL-011) · M · M14
- ⚪ [**LEGAL-011**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L2046) Копирование дисков и позиционирование продукта в ЕС и других странах · S · M14

</details>

<details><summary><b>Баги</b> <code>▓▓▓▓░░░░░░ 39% (7/18)</code> · ⚪ 11 · 🔵 0 · 🟡 7 · ✅ 0</summary>

_BUG_

- ⚪ [**BUG-001**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L2217) [P0] Память растёт при каждом открытии и закрытии плеера · M · M10
- ⚪ [**BUG-002**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L2221) [P1] Подвисание в первые 10 секунд первого просмотра по SMB · S · M10
- 🟡 [**BUG-003**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L2225) [P1] Состояние ошибки плеера «залипает» после безвредного сообщения mpv · S · M10 · на устройствах ✅ 0/7
- 🟡 [**BUG-004**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L2229) [P1] Папка, найденная в сети, сохраняется по IP, а не по имени · S · M10 · нет строк в каталоге проверок
- 🟡 [**BUG-005**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L2233) [P2] Файлы из «свалки» показываются как фильм «clip» · S · M10 · нет строк в каталоге проверок
- 🟡 [**BUG-006**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L2237) [P2] Белая вертикальная линия на карточке «Продолжить» · S · M10 · на устройствах ✅ 0/7
- 🟡 [**BUG-007**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L2241) [P1] Тест шины событий pa_core зависает в CI · S · M10 · нет строк в каталоге проверок
- ⚪ [**BUG-008**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L2245) [P2] Долгие потоки ядра (синхронизация, поиск в сети) отписываются только со следующим событием · S · M10
- 🟡 [**BUG-009**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L2249) [P0] Медиа с верной подсказкой не распознаётся само — пользователя заставляют подтверждать вручную · M · M10 · на устройствах ✅ 0/7
- ⚪ [**BUG-010**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L2253) [P1] База аниме не обновляется: сжатый выпуск читается как текст · S · M10
- ⚪ [**BUG-011**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L2257) [P1] Фильм с единственным верным кандидатом «Название: подзаголовок» не распознаётся сам · S · M10
- ⚪ [**BUG-012**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L2261) [P2] Строка «… стоит проверить» на главной и счётчик проверки в настройках не обновляются после распознавания · S · M10
- ⚪ [**BUG-013**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L2265) [P2] «Источники» показывают сырую английскую ошибку хранилища ключей · S · M10
- ⚪ [**BUG-014**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L2269) [P2] Серии нераспознанного сериала подписаны по-английски «Episode N» · S · M10
- ⚪ [**BUG-015**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L2273) [P2] При перемотке в плеере падает построение семантики (`Null check operator` в `_updateSemanticsNodeGeometry`) · S · M10
- ⚪ [**BUG-016**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L2277) [P2] Плашка о хранилище ключей остаётся поверх видео в плеере · S · M10
- ⚪ [**BUG-017**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L2281) [P2] `tools/qa/e2e/run.sh` не собирает приложение в свежем клоне: нет шрифтов Inter · S · M10
- 🟡 [**BUG-018**](https://github.com/DenisHumen/kinolune/blob/be429ca372637d7f6b6cb5ee8104fedba0311cf6/docs/ROADMAP.md#L2286) [P0] Папка SMB сканируется, но медиатека пуста: одна паника на имени файла рушит весь скан · S · M10 · нет строк в каталоге проверок

</details>

---

**Статусы.** ⚪ не начато — `[ ]` и нет открытого PR; 🔵 в работе — `[ ]` и открыт PR с `[ID]` в заголовке или ветка `тип/ID-…`; 🟡 сделано, не проверено — `[x]`, но проверки задачи в `docs/qa/test-catalog.md` не все ✅ (или у задачи приложения нет строк в каталоге); ✅ готово — `[x]` и проверки на устройствах ✅ (или проверять на устройстве нечего: решения, репозиторий, CI, документы).
