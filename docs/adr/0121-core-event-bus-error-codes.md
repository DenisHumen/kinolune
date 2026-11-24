<!-- SPDX-License-Identifier: MPL-2.0 -->

# ADR-0121: Шина событий ядра — один поток `core_events`; у каждой публичной ошибки стабильный код, текст делает клиент

- **Статус:** Предложен
- **Дата:** 2026-11-18
- **Решают:** владелец проекта (поручил решать по правилу «максимально просто для клиента»), мейнтейнер
- **Связано:** CORE-009, CORE-006, CORE-002, CORE-011, UI-005, UI-007; `docs/research/04` §1.5; [ADR-0001](0001-rust-core-uniffi-frb.md), [ADR-0015](0015-frb-native-assets-committed-bindings.md), [ADR-0014](0014-core-runtime-settings-rusqlite.md), [ADR-0028](0028-pa-jobs-persistent-queue.md)

## Контекст и проблема

ROADMAP CORE-009 требует стрим событий ядра в UI (library, scan, jobs, sync, playback) через FRB Stream и UniFFI callback и стабильные коды ошибок, по которым текст формирует клиент через l10n. «Done when»: Dart-тест получает события скана; у каждой публичной ошибки есть код.

К моменту задачи ядро уже большое (40 доменов в `pa-frb`) и события в нём есть, но разрозненно: у скана, метаданных, вотчера, синхронизации, офлайн-копий свой `StreamSink` на вызов; чтобы узнать «медиатека изменилась», экран должен был сам держать нужный поток. Ошибки — 20 типов: у части есть поле `code` (`SourcesError`, `CoreSyncError`, `CoreOfflineError`…), у части только `message` (`LibraryError`, `StorageError`, `SkipError`, `TrickplayError`, `DiagnosticsError`, `ImageCacheError`, `RemoteConfigError`), у двух — enum `kind`. ADR-0014 прямо оставил «CORE-009 заменит упрощённый `CoreError` стабильными кодами».

## Факторы решения

- Клиенту — один способ подписаться на всё и одно правило для ошибок, без знания внутренностей ядра.
- Без большой переделки: существующие потоки и поля остаются, Dart-код приложения не ломается.
- Генератор FRB в проекте не использует `freezed`: enum с данными в Dart-API нет, только записи и простые enum.
- Без новых крейтов и зависимостей.

## Рассмотренные варианты

1. **Один общий поток `core_events` поверх существующих**, плоская запись `CoreEvent { domain, <payload>? }`; у всех ошибок — поле `code`.
2. **Поток на домен** (`library_events`, `job_events`…) — клиенту пять подписок вместо одной, а у скана и синхронизации свои потоки уже есть.
3. **Общий enum ошибок ядра `CoreErrorCode`** вместо строк — один тип на все домены требует переделки всех 20 ошибок и ломает уже существующие строковые коды (`usher_403`, `store_build`), на которые опирается приложение.

## Решение

Выбран вариант **1**.

### Шина

- `pa_runtime::EventBus<E>` — маленький publish/subscribe: `const`-конструктор (живёт в `static`), доставка синхронно в потоке публикатора, без буфера и без потерь; подписчик возвращает `false` — он удаляется; колбэки вызываются вне блокировки (можно публиковать и подписываться изнутри). Когда никто не слушает, публикация — одна блокировка пустого списка.
- **Dart (FRB):** `core_events()` → `Stream<CoreEvent>`; можно открыть сколько угодно потоков, поток живёт, пока Dart его не отменит. `CoreEvent` — плоская запись: `domain` (`libraryChanged`, `scanProgress`, `jobChanged`, `syncChanged`, `playbackSaved`) говорит, какое из одноимённых полей заполнено (имена без `library`/`sync`: в Dart это служебные слова, генератор дописал бы `_`):
  - `scan_progress: CoreScanUpdate` — тот же тип, что у `CoreLibrary.scan`; публикуют `scan` и `probe_deferred`;
  - `library_changed: CoreLibraryEvent { change, source_id? }` — `change`: `scan` (после каждого скана и докачки заголовков), `watcher` (вотчер что-то добавил, изменил или убрал), `metadata` (пришли метаданные), `other_device` (изменения с другого устройства);
  - `job_changed: CoreJobEvent { job_id, change, kind, state?, progress?, bytes_done, bytes_total? }` — все события общей очереди `pa-jobs` (загрузки Twitch, офлайн-копии…);
  - `sync_changed: CoreSyncEventKind` — только вид события сервиса синхронизации; данные (позиции других профилей, текст удалённого ввода) остаются в `CoreSync.events`, `input` в шину не попадает вовсе;
  - `playback_saved: CorePlaybackEvent { item_id, position_ms, duration_ms, watched }` — после каждого `save_progress`: ряд «Продолжить» обновляется без перезапроса по таймеру.

  Дополнение (BUG-007, 2026-11-19): `coreEvents()` в pa_core написан руками поверх трёх синхронных вызовов ядра — `core_events_next_id`, `core_events_listen(id, sink)` и `core_events_close(id)`. Подписка в ядре появляется на `listen` до его возврата (события следующих вызовов не теряются), а `cancel()` закрывает поток в ядре сразу. Обычный поток FRB узнаёт об отмене только на следующем событии: на тихой шине `cancel()` не завершался, открытый порт держал изолят, и `dart test` висел до лимита CI.

  Дополнение (BUG-008, 2026-11-24): та же схема — на всех долгих потоках, которые заканчивает отмена из Dart: `CoreSync.events`, `CoreDiscovery.watch`, `CoreOffline.watch`. Общий реестр потоков ядра (`pa-frb/src/streams.rs`) выдаёт id (`core_stream_next_id`) и закрывает поток по id (`core_stream_close`, заменил `core_events_next_id`/`core_events_close`); вызовы подписки синхронные: `core_events_listen(id, sink)`, `CoreSync.events_listen(id, sink)`, `CoreDiscovery.watch_listen(id, sink)`, `CoreOffline.watch_listen(id, sink)`. В Dart их оборачивает `closableStream` из pa_core, а `events()`/`watch()` остались прежними (расширения). Закрытие сразу отбирает `StreamSink` у потока ядра (Dart получает конец потока), а поток ядра, ждущий события, замечает закрытие не позже чем через 250 мс и отпускает подписку. Потоки с собственной остановкой (`start_watcher`/`stop_watcher`, чат и live Twitch с `stop`/`close`) не менялись: приложение сначала останавливает их, и поток заканчивается сам.
- **Kotlin/Swift (UniFFI):** `core_events_subscribe(listener: CoreEventListener) -> CoreEventSubscription`; отписка — `cancel()` или просто освобождение объекта. `CoreEvent` — enum UniFFI; сейчас в нём то, что экспортирует `pa-ffi`: `Lifecycle` (каждая смена состояния, даже без `CoreCallbacks`) и `SettingsChanged { keys }`. Медиатека, скан, задачи, синхронизация и воспроизведение в UniFFI ещё не экспортированы (tvOS работает через сервер, ADR-0002); когда домен экспортируется, он добавляет вариант в этот enum и публикует в ту же шину.
- Шина — по одной на фасад и процесс; события всех открытых медиатек идут в один поток, общее поле `library_path` (путь базы из `CoreLibrary.open`) различает их.
- Существующие потоки вызовов (`scan`, `fetch_metadata`, `start_watcher`, `CoreSync.events`, `CoreOffline.watch`) не меняются; шина повторяет их события для всех, кто слушает (бейдж, уведомление, другой экран).

### Ошибки

- **Каждая ошибка, которую публичная функция FRB возвращает в Dart, — запись с полем `code: String`.** Код — `snake_case` ASCII (`[a-z][a-z0-9_]*`, до 40 байт; `pa_runtime::is_valid_code`). `message` — английская подробность для логов, пользователю не показывается.
- **Общий словарь** (`pa_runtime::error_codes`): `not_found`, `invalid`, `denied`, `network`, `unreachable`, `io`, `database`, `secrets`, `no_space`, `busy`, `disabled`, `not_initialized`, `sync`, `provider`, `internal`. В этих ситуациях домены используют именно эти слова, поэтому одна строка l10n служит всем доменам. Доменные коды (`usher_403`, `store_build`, `foreign_database`, `rollback`…) остаются — они точнее.
- **Стабильность:** выпущенный код не меняет смысла и не исчезает; новые коды добавляются. Клиент обязан иметь общий текст для незнакомого кода.
- **Текст делает клиент:** `switch (e.code)` → строка из ARB, по умолчанию — общий текст «Что-то пошло не так». Так уже сделано для источников, офлайн-копий и синхронизации; для остальных доменов приложение переходит на коды по мере правки экранов (UI-005 и др.), ядро для этого больше ничего не меняет.
- Добавлено поле `code` в `LibraryError` (по `pa_catalog::Error::code()`), `StorageError`, `SkipError`, `TrickplayError`, `DiagnosticsError`, `ImageCacheError`, `RemoteConfigError`; у `UpdateVerifyError` и `TwitchApiError` — `code` дублирует `kind` строкой (enum остаётся). Старые поля (`offline`, `disabled`, `kind`, префикс `код: ` в сообщениях файлового браузера) сохранены для совместимости.
- **Ошибки внутри потоков:** у `CoreScanUpdate` появилось `error_code` (`not_found`, `unreachable`, `io`; при провале всего скана — код `LibraryError`). Строки `error` в событиях вотчера и метаданных пока без кода — это целевая модель для следующих правок этих доменов.
- **UniFFI:** ошибки — enum `uniffi::Error`; вариант и есть код (Kotlin sealed class, Swift enum). `pa_runtime::ErrorCode` — трейт для Rust-ошибок ядра (реализован для `RuntimeError`, `SettingsError`), им пользуются фасады при сопоставлении.
- **Проверка:** `pa-frb/tests/error_codes.rs` читает `src/api/*.rs` и падает, если у `pub struct …Error` нет `pub code: String` или код-литерал не проходит `is_valid_code`. Новая ошибка без кода не пройдёт CI.

### Последствия

- Хорошо: UI подписывается один раз и обновляет экраны по событиям; один обработчик ошибок по коду на всё приложение; шина без зависимостей и без потерь событий.
- Плохо (цена): доставка синхронная — медленный подписчик в Rust тормозит публикатора (FRB-`StreamSink` и UniFFI-колбэк «быстрые» по контракту); частые события скана идут всем открытым потокам — фильтрация на стороне Dart (`where`). Коды — строки, опечатку в клиенте компилятор не поймает (тест ядра ловит форму, но не смысл).
- Требует работы: экраны приложения постепенно переходят с `message` на `code` и с опроса — на `core_events`; при экспорте доменов в UniFFI — варианты `CoreEvent` для них; коды для строк ошибок вотчера и метаданных.
