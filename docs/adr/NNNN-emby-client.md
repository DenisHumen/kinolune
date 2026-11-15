<!-- SPDX-License-Identifier: MPL-2.0 -->

# ADR-NNNN: Клиент Emby — тот же `pa-jellyfin` с флагом вида сервера, тесты против Emby Server только локально

- **Статус:** Предложен
- **Дата:** 2026-11-15
- **Решают:** владелец проекта (мейнтейнер); участники обсуждения — по PR
- **Связано:** NET-015, NET-014, NET-011; `docs/research/01` §1.5; [ADR-0075](0075-jellyfin-client.md), [ADR-0004](0004-no-project-servers-local-sync.md), [ADR-0048](0048-trickplay-previews.md), [ADR-0061](0061-import-skip-markers.md)

## Контекст и проблема

Паритет с Infuse требует и сервер Emby как источник (research 01 §1.5): вход, библиотеки, «Продолжить просмотр», избранное, отчёты, DirectPlay/DirectStream/Transcode и превью сервера. Клиент Jellyfin уже есть (ADR-0075): `pa-jellyfin`, дерево `JellyfinFs`, вид источника `jellyfin`, мост `CoreJellyfin`, форма в приложении. Jellyfin — форк Emby 3.5, их API во многом общий. Нужно решить, как переиспользовать клиент, и можно ли гонять тесты против Emby Server в CI.

Различия, проверенные на Emby Server 4.10.2.0 (Docker `emby/embyserver:4.10.2.0`) и по его собственной схеме OpenAPI (`/emby/openapi.json`):

| Что | Jellyfin 10.10 | Emby 4.10 |
|---|---|---|
| Заголовок входа | `Authorization: MediaBrowser …` | `X-Emby-Authorization: MediaBrowser …` (документирован; `Authorization` тоже принимается), токен ещё и в `X-Emby-Token` |
| Пользователь по токену | `/Users/Me` | нет (`500`), только `/Users/{id}` |
| Пути пользователя | `/UserViews`, `/UserItems/Resume`, `/Items?userId=`, `/UserFavoriteItems/{id}`, `/UserPlayedItems/{id}` | `/Users/{id}/Views`, `/Users/{id}/Items/Resume`, `/Users/{id}/Items`, `/Users/{id}/FavoriteItems/{id}`, `/Users/{id}/PlayedItems/{id}` |
| Убрать из «Продолжить» | флага нет, сбрасывают позицию | `POST /Users/{id}/Items/{id}/HideFromResume?Hide=true`, позиция остаётся |
| Вход кодом | Quick Connect (локально) | нет; коды и PIN — только через Emby Connect (облако Emby) |
| Метки заставок | Media Segments `/MediaSegments/{id}` | маркеры глав `Chapters[].MarkerType` (`IntroStart`/`IntroEnd`/`CreditsStart`) |
| Превью перемотки | trickplay-листы `/Videos/{id}/Trickplay/…` | BIF `/Videos/{id}/index.bif?Width=` (пустой BIF, если превью нет) |
| Ид версии | равен ид элемента | `mediasource_<id>` |
| `TranscodingUrl` | без ключа | уже с `api_key`; при выключенном копировании видео причины остаются «контейнерными», но в URL `allowVideoStreamCopy=false` |
| `/System/Info/Public` | `ProductName: "Jellyfin Server"` | без `ProductName` |

Отчёты (`/Sessions/Playing…`), `PlaybackInfo` с профилем устройства, `stream?static=true` с диапазонами, картинки, `/Users/AuthenticateByName`, `/Sessions/Logout`, `/Videos/ActiveEncodings` — одинаковы.

## Факторы решения

- Общий код больше разницы: дерево файлов, профиль устройства и разбор `PlaybackInfo`, отчёты, DTO, адреса с переключением — одни и те же.
- Слой источников синхронный (ADR-0013), новых зависимостей и лицензий не нужно.
- Всё локально (ADR-0004): облачный вход Emby Connect — сторонний сервис со своим аккаунтом.
- Emby Server — проприетарная программа: условия Emby дают «личную, некоммерческую, отзывную» лицензию, запрещают распространение и использование как сервиса.

## Рассмотренные варианты

1. **Отдельный крейт `pa-emby`** с копией клиента: две копии ~3 000 строк, исправления дважды.
2. **Общий крейт-ядро + два тонких крейта** (`pa-mediabrowser` + `pa-jellyfin` + `pa-emby`): чисто, но переименование и перенос всего NET-014 ради ~150 строк различий, а все потребители (`pa-sources`, мост, тесты) меняются.
3. **Флаг вида сервера в `pa-jellyfin`** (`ServerKind::{Jellyfin, Emby}`) с ветвлением в нескольких методах клиента и в построении дерева.

Для тестов: **(а)** Emby в CI GitHub; **(б)** только локально, тем же скриптом, что Jellyfin.

## Решение

Вариант **3** и тесты **(б)**.

- **`Client::with_kind(ServerKind::Emby)`**: заголовок `X-Emby-Authorization` + `X-Emby-Token`; пути `/Users/{id}/…`; `me()` по ид пользователя входа (`Client::set_user_id` для сохранённого входа); `remove_from_resume` — `HideFromResume`; Quick Connect — `Error::Unsupported` без запроса (`quick_connect_enabled` = `false`); поля элементов — `Chapters` и `ProductionYear` вместо `Trickplay`; `media_segments_of` строит сегменты из маркеров глав (`segments_from_chapters`), `bif()` отдаёт BIF сервера, пустой BIF — `NotFound`. Классификация ответа `PlaybackInfo` считает `allowVideoStreamCopy=false` транскодом (полезно и для Jellyfin). Публичный API для Jellyfin не меняется.
- **Дерево `JellyfinFs`** то же; для Emby другие сайдкары: `<stem>.bif` (читается `pa-trickplay`, ADR-0048; если превью на сервере нет, открытие даёт `NotFound`, и превью делаются как для любого файла) и `<stem>.jellyfin.json` в формате Media Segments из маркеров глав — только у элементов с маркерами (его уже импортирует PLAY-015, ADR-0061; провайдер в базе — `jellyfin`, формат тот же).
- **`pa-sources`**: новый вид источника `emby` (`SourceKind::Emby`, `is_media_server()`, `server_kind()`), настройки — те же `JellyfinOptions` (поле `jellyfin` в `options_json`), все `jellyfin_*` действия работают для обоих видов. `JellyfinAccount::sign_in_to(kind, …)`; вход отказывает, если сервер другого вида (по `ProductName`).
- **Мост `CoreJellyfin`**: `probe(address, server)` и `sign_in(server, …)` с `server` = `jellyfin` | `emby`; действия по элементам (воспроизведение, отчёты, избранное, «просмотрено», импорт состояния) — для обоих.
- **Приложение**: Emby в окне «Добавить источник» больше не «скоро»; форма Jellyfin с `server: emby` — только пароль (без переключателя Quick Connect) и подсказка, что учётные записи Emby Connect не поддерживаются (пользователю нужен локальный пароль сервера).
- **Emby Connect не делаем**: это вход через облако Emby (`connect.emby.media`) с отдельным аккаунтом и условиями; пользователь Emby Connect может войти локальным паролем, который задаётся на сервере.
- **Тесты против Emby — только локально.** `core/crates/pa-jellyfin/tests/emby/run.sh` (образ `emby/embyserver:4.10.2.0`, медиатека генерируется ffmpeg из образа, контейнер и файлы удаляются после) — не в CI. Условия Emby (emby.media/terms) дают личную некоммерческую лицензию и запрещают распространение и использование «как сервиса»; прогон в общем CI проекта под эту лицензию не подходит однозначно. Тесты Jellyfin тоже запускаются скриптом, так что порядок одинаковый.

Не входит: Emby Connect и вход по PIN; превью и метки, которые делает сервер с Emby Premiere (на тестовом сервере без Premiere задачи «Video preview thumbnail extraction» и «Detect Episode Intros» ничего не делают — разбор BIF и маркеров проверен юнит-тестами и локальным HTTP-сервером, против настоящего Emby — только ответ «превью нет»); то же, что не входит в ADR-0075.

### Последствия

- Хорошо: Emby добавлен ~600 строками (клиент, вид источника, мост, форма) вместо второго клиента; исправления клиента сразу для обоих серверов.
- Хорошо: сервер Emby виден во всех существующих видах, как Jellyfin; 8 интеграционных тестов против настоящего Emby Server 4.10.2 со сгенерированной медиатекой.
- Плохо: внутри `pa-jellyfin` ветвления по `ServerKind`; если Emby и Jellyfin разойдутся сильнее, придётся выделять общий крейт (вариант 2) — граница уже видна по методам с ветвлением.
- Плохо: имя крейта и типов (`JellyfinFs`, `JellyfinOptions`, `jellyfin_*`) не говорит про Emby; оставлено ради неизменного API, документация упоминает оба сервера.
- Нейтрально: тесты Emby не защищают от регрессий автоматически; их надо запускать вручную при изменениях `pa-jellyfin`.

## Ссылки

- `core/crates/pa-jellyfin/src/http.rs` (`ServerKind`, заголовки), `api.rs` (пути, `HideFromResume`, BIF, маркеры), `fs.rs` (сайдкары Emby); `core/crates/pa-sources/src/jellyfin.rs`; `core/crates/pa-frb/src/api/jellyfin.rs`; `clients/flutter/app/lib/screens/jellyfin_source_form.dart`
- Тесты: `core/crates/pa-jellyfin/tests/emby.rs`, `core/crates/pa-catalog/tests/emby.rs`, запуск `core/crates/pa-jellyfin/tests/emby/run.sh`; `clients/flutter/app/test/emby_source_test.dart`
- Emby REST API: схема OpenAPI сервера (`/emby/openapi.json`) и dev.emby.media; условия использования — emby.media/terms.html. Код Emby и его клиентов не использован.
