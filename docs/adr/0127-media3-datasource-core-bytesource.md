<!-- SPDX-License-Identifier: MPL-2.0 -->

# ADR-0127: Media3 `DataSource` поверх `ByteSource` ядра — тот же URL прокси, чтение в процессе, HTTP как запасной путь

- **Статус:** Предложен
- **Дата:** 2026-11-18
- **Решают:** владелец проекта (поручил решать по правилу «максимально просто для клиента»), мейнтейнер
- **Связано:** PLAY-AND-003, PLAY-AND-002, NET-001, NET-007, LIB-018; [ADR-0001](0001-rust-core-uniffi-frb.md), [ADR-0008](0008-android-player-embedding.md), [ADR-0013](0013-pa-vfs-blocking-positional-reads.md), [ADR-0017](0017-player-engine-contract.md), [ADR-0038](0038-network-playback-loopback-proxy.md)

## Контекст и проблема

ROADMAP PLAY-AND-003: «DataSource через UniFFI для SMB, NFS и WebDAV; запасной путь — pa-proxy». Done when: 4K-remux с Samba воспроизводится без рывков на эмуляторе и на реальном устройстве.

Как сетевые файлы играют на Android сейчас (ADR-0038): приложение получает от ядра (FRB, `libpa_frb.so`) базовый URL loopback-прокси `pa-proxy` и кладёт в элемент медиатеки `streamUri` = `http://127.0.0.1:<порт>/<токен>/<id источника>/<путь>`; media_kit (libmpv) читает его по HTTP Range. Движок Media3 (`Media3Engine`, PLAY-AND-002) принимает любой URI и читал бы тот же URL через `DefaultHttpDataSource`. Пароли источников живут только в ядре (`pa-secrets`), Kotlin и Dart их не видят.

Что мешает «просто HTTP»:

- лишний переход через сокет и HTTP-разбор на каждый диапазон, отдельный поток прокси на соединение;
- Android с targetSdk 28+ запрещает cleartext HTTP: `DefaultHttpDataSource` к `127.0.0.1` без network security config падает;
- ROADMAP и ADR-0001 обещают адаптер `ByteSource` → Media3 `DataSource` наравне с AVIO и mpv `stream_cb`.

Нужно решить: как Kotlin получает `ByteSource` нужного файла, не узнавая пароля и не дублируя менеджер источников, и что делать, когда прямой путь недоступен.

## Факторы решения

- Для клиента (приложения и плагина `pa_player_android`) ничего не меняется: тот же `streamUri`, никакого нового API «открыть источник по id».
- Пароль — по-прежнему только в ядре.
- Один код открытия файлов источника: тот же `Opener`, что у прокси (`SourceOpener` в `pa-catalog`, кэш открытых файлов, переподключение).
- Любой протокол `pa-vfs` — SMB, WebDAV, NFS (когда появится бэкенд), SFTP/FTP, DLNA — без кода под каждый протокол в Kotlin.
- Если прямой путь невозможен — воспроизведение всё равно работает (через прокси).
- Без новых зависимостей; без настроек для пользователя.

## Рассмотренные варианты

1. **`stream_open(url)` по URL прокси.** `pa-proxy` ведёт реестр своих работающих прокси в процессе; `open_local(url)` по порту и токену находит прокси и вызывает его `Opener` напрямую. `pa-ffi` экспортирует `stream_open(url) -> CoreByteSource` (`size`, `read_at`) и ошибку `NotLocal`. Kotlin `CoreDataSource` сначала спрашивает ядро, при `NotLocal` читает тот же URL по HTTP.
2. **UniFFI-API «открыть файл источника»** (`open_source_file(source_id, path)`) с менеджером источников и хранилищем секретов внутри `pa-ffi`. Второй экземпляр менеджера и хранилища рядом с FRB (ADR-0038 явно требует один объект хранилища на процесс), новый API для клиента, плагину нужны id источника и путь вместо URL.
3. **`DataSource` по адресу `smb://…` с паролем из Kotlin.** Пароль выходит из ядра — противоречит ADR-0038 и NET-011.
4. **Только HTTP через прокси** (`DefaultHttpDataSource`). Ничего не надо в ядре, но остаются сокет, cleartext-исключение как единственный путь и невыполненная часть задачи.

## Решение

Выбран **вариант 1**.

### Ядро

- `pa-proxy`: `Proxy::start` регистрирует прокси в статическом реестре процесса (порт + слабая ссылка на общее состояние с токеном и `Opener`), `Drop` снимает. `pa_proxy::open_local(url)`:
  - `None` — URL не принадлежит живому прокси этого процесса (не `http`, не `127.0.0.1`/`localhost`, чужой порт, неверный токен, прокси остановлен);
  - `Some(Err)` — URL наш, но путь пустой или с битым `%`-экранированием (`InvalidPath`), либо ошибка `Opener` — те же виды, что прокси превращает в `4xx/5xx`;
  - `Some(Ok(ByteSource))` — тот же объект, который прокси отдал бы по HTTP.
  Запрос и фрагмент URL игнорируются, как у сервера. Токен не попадает в ошибки.
- `pa-ffi` (модуль `streams`): `stream_open(url) -> CoreByteSource`; `CoreByteSource.size()`, `read_at(offset, max_len) -> bytes` — короткое чтение не ошибка, пустой ответ — конец файла, не больше 4 МиБ за вызов. `StreamError`: `NotLocal`, `NotFound`, `Denied`, `Unavailable`, `TimedOut`, `Failed`.
- Кода для протоколов нет: чтение идёт через `Opener` прокси, то есть через `pa-vfs` нужного источника — SMB (libsmb2 с read-ahead и параллельной подкачкой, NET-003), WebDAV, SFTP/FTP, DLNA и т. д. **NFS** заработает так же, когда появится бэкенд `pa-vfs` для NFS (сейчас его в ядре нет).
- Фича `testing` у `pa-ffi` (не для сборок приложения): `stream_test_serve_dir`, `stream_test_serve_smb`, `stream_test_stop_all` — тестовые прокси внутри библиотеки для instrumented-тестов. `PA_TESTING=1 tools/bindings/build-aar.sh` собирает ядро с ней.

### Android

- `CoreDataSource` (`clients/android/player`): для `http://127.0.0.1:<порт>/…` и `http://localhost:<порт>/…` вызывает `streamOpen`; получилось — читает файл блоками по 512 КиБ (один JNA-вызов на блок, отдача мелкими чтениями экстрактора из буфера), события передачи Media3 — как у сетевого источника (`BandwidthMeter` видит скорость). `NotLocal`, отсутствие библиотеки ядра (`LinkageError`) или `direct = false` → тот же `DataSpec` уходит в upstream (`DefaultDataSource`: HTTP к прокси, `file://`, `content://`). Остальные ошибки ядра → `DataSourceException` с кодами Media3 `2xxx` (нет файла, нет доступа, сеть, таймаут), которые `Media3Mapping` уже переводит в `EngineError.Source`.
- `Media3Engine.defaultPlayer` собирает ExoPlayer с `DefaultMediaSourceFactory(CoreDataSource.Factory(context))`, поэтому `PlayerActivity` плагина получает прямой путь без изменений. `Media3Engine.player(context, dataSources)` — для тестов и своих фабрик.
- Запасной путь работает на Android 9+: библиотека движка и плагин объявляют `networkSecurityConfig` с cleartext только для `127.0.0.1` и `localhost` (остальное — по умолчанию) и `INTERNET`. Если у приложения появится свой network security config, в него нужно перенести эти два домена (иначе слияние манифестов сообщит о конфликте).
- Буфер Media3 — `DefaultLoadControl` по умолчанию (до 50 с, цель для видео около 128 МБ): для 4K-ремукса около 90 Мбит/с это примерно 11 с запаса, на замерах ниже хватило. Адаптивный буфер (ADR-0120) для Media3 — задача PLAY-AND-010.

### Одна копия ядра в приложении

Прямой путь работает, когда UniFFI-привязки и прокси живут в одной копии ядра. В приложении Kotlin-привязки по умолчанию грузят `libpa_ffi.so`, а прокси запускает FRB в `libpa_frb.so`; это две копии Rust-кода со своими статическими данными, и `stream_open` из `libpa_ffi.so` вернёт `NotLocal` — воспроизведение пойдёт через прокси. `libpa_frb.so` зависит от `pa-ffi` и экспортирует все его UniFFI-символы (проверено `nm -D`: 159 символов `uniffi_pa_ffi_*`, включая `ffi_pa_ffi_uniffi_contract_version`), поэтому при подключении плагина в приложение (переход экрана плеера на Media3, UI-013/PLAY-AND) достаточно до первого вызова UniFFI поставить `System.setProperty("uniffi.component.pa_ffi.libraryOverride", "pa_frb")` — тогда одна копия ядра обслуживает и Dart, и Media3, а `libpa_ffi.so` в APK не нужен. Сейчас плагин в приложение не подключён (README плагина), поэтому переключатель не включён и не проверен — см. «Не проверено».

### Замеры (эмулятор)

Эмулятор Android 14 x86_64 (образ `default`, 4 ядра, swiftshader), Samba в Docker на той же машине (контейнер alpine, скрипт `pa-vfs-smb/tests/servers/start.sh`), эмулятор ходит на хост через `10.0.2.2`. Файл — синтетический «4K-ремукс»: 3840x2160 H.264 High 5.1, 23.976 к/с, 80 Мбит/с CBR (filler NAL), AAC stereo + PCM 8 каналов 48 кГц; в сумме 92.5 Мбит/с, 120 с, 1.39 ГБ. Генерировался на машине сборки, в репозиторий не попадает. Тест — `SambaStreamingTest` (instrumented).

RESULTS_TABLE

Декодирование 4K H.264 на эмуляторе программное, поэтому число выброшенных кадров говорит о скорости CPU эмулятора, а не о чтении; критерий «без рывков» на эмуляторе — ноль остановок на подкачку (`STATE_BUFFERING` во время воспроизведения) и позиция, не отстающая от часов. Плавность картинки 4K проверяется только на реальном устройстве с аппаратным декодером.

### Последствия

- Хорошо: клиенту ничего не нужно знать — тот же URL; прямой путь, когда он возможен, и прокси, когда нет.
- Хорошо: все протоколы `pa-vfs` сразу, включая будущий NFS; пароль не покидает ядро; один `Opener` и один кэш открытых файлов для прокси и Media3.
- Хорошо: запасной путь теперь не упирается в запрет cleartext HTTP.
- Плохо: прямой путь в приложении требует одной копии ядра (`libraryOverride` на `pa_frb`) — это ещё надо включить и проверить при подключении плагина.
- Плохо: данные копируются через JNA (`Vec<u8>` → `ByteArray`) блоками по 512 КиБ; на замерах это не узкое место.
- Нейтрально: прокси в процессе по-прежнему слушает порт — для других плееров (media_kit, AVPlayer) и как запасной путь.

## Не проверено

- Реальные устройства (телефон, Shield, Chromecast GTV): 4K-ремукс с Samba по Wi-Fi и Ethernet — строки PLAY-83…PLAY-86 каталога проверок.
- WebDAV и прочие протоколы через прямой путь на эмуляторе (код общий, тест — только Samba и локальная папка).
- `libraryOverride` на `pa_frb` внутри приложения Kinolune (плагин ещё не подключён к приложению).

## Ссылки

- `core/crates/pa-proxy/src/local.rs`, `core/crates/pa-ffi/src/streams.rs`
- `clients/android/player/src/main/kotlin/dev/projectalpha/player/CoreDataSource.kt`, `Media3Engine.kt`
- Тесты: `pa-proxy/tests/proxy.rs` (`open_local_skips_the_socket`), `pa-ffi` `streams::tests`, instrumented `CoreDataSourceTest` и `SambaStreamingTest` (`clients/android/player/src/androidTest`)
