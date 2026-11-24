<!-- SPDX-License-Identifier: MPL-2.0 -->

# ADR-0153: Google Cast через официальный SDK и Default Media Receiver, AirPlay через AVPlayer; файл — через одноразовый LAN-ретранслятор

- **Статус:** Предложен
- **Дата:** 2026-11-24
- **Решают:** владелец проекта (мейнтейнер); участники обсуждения — по PR
- **Связано:** `PLAY-012`; `PLAY-005`, `SYNC-003`, `SYNC-014` (прогресс), `LIB-018` и [ADR-0038](0038-network-playback-loopback-proxy.md) (loopback-прокси), `NET-013`/`NET-014` (транскод Plex/Jellyfin/Emby), [ADR-0007](0007-store-full-build-flavors.md) и [ADR-0041](0041-build-flavor-mechanism.md) (флейворы), [ADR-0035](0035-apple-engines-foreign-player-engine-mpvkit.md) (Apple-движки), `PKG-010` (F-Droid)

## Контекст и проблема

Пользователь хочет смотреть фильм из медиатеки на телевизоре: Chromecast/Google TV с телефона Android и Apple TV по AirPlay с Mac (и iPhone, когда появится iOS-сборка). Ограничения:

- **Cast SDK проприетарный** (условия Google APIs): в F-Droid его быть не может (ADR-0007: «внутри `full` отдельной фичей сборки, без третьего флейвора»).
- **media_kit/libmpv не умеет AirPlay видео** — только системный AVPlayer отдаёт поток на Apple TV.
- Ресивер сам забирает видео по HTTP. Файлы сетевых источников играют через loopback-прокси `pa-proxy` (ADR-0038): он слушает только `127.0.0.1` и отдаёт **любой** файл любого источника тому, у кого есть токен процесса. Показывать его в LAN нельзя.
- Chromecast (Default Media Receiver) играет не всё: MP4/WebM, H.264 до 1080p, VP8, AAC/MP3/Opus/Vorbis/FLAC; Ultra и Google TV — ещё HEVC/VP9 до 4K и HDR10/HLG. MKV, DTS, TrueHD, AC-3 без HDMI-ресивера — нет. AVPlayer: MP4/MOV, H.264/HEVC, AAC/AC-3/E-AC-3/ALAC.
- «Продолжить» и синхронизация (SYNC) должны работать так же, как при просмотре на этом устройстве.

## Факторы решения

- Просто для пользователя: одна кнопка, без своего приложения-ресивера и без аккаунтов.
- Приватность: в LAN не уходит ничего, кроме одного файла на время трансляции.
- Лицензии: без GPL; проприетарное — только там, где канал это разрешает.
- Минимум нового нативного кода, который нельзя проверить без устройства.

## Рассмотренные варианты

Отправитель Google Cast:

1. **Официальный Cast sender SDK через свой тонкий platform channel** (`app.kinolune/cast`), ресивер — Default Media Receiver (`CC1AD845`).
2. Flutter-плагин поверх SDK (`flutter_chrome_cast` и подобные).
3. Своя реализация открытого протокола CASTV2 (TLS + protobuf + mDNS) на Dart или Rust — работает и в F-Droid, и на десктопе.
4. Свой ресивер (Custom Web Receiver).

AirPlay:

1. **Отдать поток AVPlayer и направить его через `AVRoutePickerView`** (macOS: свойство `player` пикера; iOS — системный маршрут).
2. Зеркалирование экрана — не наша функция, и картинка хуже.

Что получает ресивер:

1. **Файл как есть через одноразовый LAN-ретранслятор** (только этот файл, новый токен, только на время сессии), если ресивер его играет; иначе **HLS сервера** (Jellyfin/Emby/Plex — уже есть в NET-013/014); иначе понятное сообщение и «Всё равно попробовать».
2. Привязать `pa-proxy` к `0.0.0.0`.
3. Транскодировать на телефоне (FFmpeg) — батарея, нагрев, LGPL-сборка без x264.

## Решение

Выбраны варианты **1 / 1 / 1**.

**Где что есть.** `castTechsFor` (`lib/cast/cast_plan.dart`): Android — Google Cast, если сборка с SDK; iOS — AirPlay (и Google Cast с SDK, когда появится iOS-раннер); macOS — AirPlay; ТВ (это ресивер), Linux и Windows — ничего, кнопка не показывается. Флаг сборки — `--dart-define=KINOLUNE_GOOGLE_CAST=false` (по умолчанию `true` во **всех** сборках обоих флейворов, включая GitHub Releases); его ставит только рецепт F-Droid (PKG-010). Gradle читает тот же dart-define: без него в APK нет ни `play-services-cast-framework`, ни `mediarouter`, ни манифеста с `OptionsProvider` (`src/cast` ↔ заглушка `src/nocast`). Это не capability `pa-flavor`: ядро о касте ничего не знает.

**Android.** `CastBridge` (`android/app/src/cast`) без `MediaRouteButton` и AppCompat: `MediaRouter` ищет устройства с категорией Default Media Receiver, Dart показывает свой список, `connect` выбирает маршрут — сессию начинает сам Cast framework. Без Google Play services `available` = false, кнопки нет. `CastOptionsProvider` — Default Media Receiver, своего ресивера и аккаунта разработчика Cast нет. Уведомление и экран блокировки — штатные Cast framework.

**macOS (AirPlay).** `AirPlayBridge` в `MainFlutterWindow.swift`: `AVRouteDetector` даёт одну запись «AirPlay», пока система видит AirPlay-устройства; при выборе создаётся `AVPlayer` с URL потока на нужной позиции, над кнопкой открывается `AVRoutePickerView` этого плеера; сессия начинается, когда `isExternalPlaybackActive` = true, и кончается, если маршрут вернулся на Mac. iOS получит тот же код, когда появится iOS-раннер.

**Что отдаём** (`planCast`): профиль ресивера — по модели Cast-устройства (Ultra, Google TV, Android TV, SHIELD, BRAVIA — 4K/HEVC/HDR10/HLG; остальные — базовый Chromecast) или AirPlay. Решают контейнер (из `TechInfo` пробы, без пробы — по расширению: MP4/WebM считаем играбельными), видеокодек, высота и битность, HDR (DV 8.1 с базой HDR10 — как HDR10), **первая** аудиодорожка. Подходит → `CastDirect`; нет, но это элемент медиасервера → `CastServerStream` (HLS из `JellyfinLink.playback(forceTranscode: true)`, адрес сервера должен быть доступен из LAN, не `127.*`); иначе `CastUnsupported` с причиной и «Всё равно попробовать» (MKV на Chromecast уходит как `video/mp4`: новые модели часто его играют). Фильм из нескольких частей (EDL, LIB-020) — только сервер или «только здесь».

**LAN-ретранслятор** (`lib/cast/cast_relay.dart`, Dart `HttpServer`): слушает **один** IPv4-адрес Wi-Fi/Ethernet (тот, что ближе всего к адресу Chromecast; VPN, мобильные данные и docker — в конце), путь `/<128 бит токена>/<имя файла>`, только `GET`/`HEAD`, `Range`, всё остальное — `404`/`405`. Локальный файл читается напрямую, файл сетевого источника — `Range`-запросом в loopback-прокси. Закрывается вместе с сессией. `pa-proxy` по-прежнему только на `127.0.0.1`. Сделан на Dart, а не в `pa-proxy`, потому что нужен и для локальных файлов, и для прокси-URL, не меняет FRB-API и тестируется обычным `flutter test`; перенести в ядро можно позже без изменения протокола.

**Прогресс** (`lib/cast/cast_progress.dart`): раз в секунду статус ресивера (`playerState`, `approximateStreamPosition`, `idleReason`; у AVPlayer — `timeControlStatus`, `currentTime`) превращается в то же сохранение, что у плеера приложения (`playback.dart`: библиотека, heartbeat передачи просмотра SYNC-003, отчёт Jellyfin/Plex). До первого кадра позиция 0 не пишется (не теряем точку «Продолжить»); «досмотрено» (`FINISHED`) пишет конец — эпизод становится просмотренным; остановка на ТВ и ошибка оставляют последнее сохранение. Сессию передачи просмотра трансляция подхватывает, когда плеер на телефоне закрылся.

**UI.** Кнопка трансляции в OSD плеера (не на ТВ) — только если найдено устройство; на странице фильма — «На телевизор». Из плеера: пауза, трансляция с текущей позиции, плеер закрывается, внизу оболочки — мини-пульт (название, «Играет на «…»», пауза, стоп; по нажатию — перемотка ±10 с и ползунок). Тексты ru/en.

**Не входит:** iOS (нет iOS-раннера Flutter — PLAY-APL-004/M5), страница сериала (кнопка в плеере эпизода уже есть; на странице — после PR UI-049, чтобы не конфликтовать), Up Next на ТВ, субтитры на ресивере (WebVTT-дорожки Cast), десктопный отправитель Cast (только вариант 3), трансляция, пока процесс приложения на Android выгружен (движок Flutter живёт с активностью, ADR-0068: если свернуть и смахнуть приложение, ТВ продолжит играть, но прогресс перестанет сохраняться).

### Последствия

- Хорошо: одна кнопка, без ресивера и аккаунтов; в LAN — только один файл и только на время трансляции; «Продолжить» и SYNC работают как с локальным плеером; Jellyfin/Emby/Plex закрывают «не тот формат» без вопросов.
- Плохо: проприетарный SDK в GitHub- и store-APK (+~1,5 МБ и Google Play services); F-Droid без Cast. Нативный код (Kotlin, Swift) в этом PR не собран и не проверен на устройстве — только Dart-тесты. Определение 4K-ресивера по названию модели — эвристика (ошибка → «Всё равно попробовать»). Программное открытие `AVRoutePickerView` на macOS нажатием его внутренней `NSButton`; если Apple это поменяет, пикер остаётся над кнопкой и открывается вторым щелчком.
- Требует работы: рецепт F-Droid (PKG-010) — `--dart-define=KINOLUNE_GOOGLE_CAST=false` и удаление строк `play-services-cast-framework` (сканер F-Droid ищет их в Gradle-файлах); iOS-раннер; проверка на реальных Chromecast/Google TV и Apple TV (владелец).

### Как проверяем соблюдение

- `app/test/cast_test.dart`: выбор технологии по платформе и флагу сборки, выбор URL (файл / HLS сервера / причина), ретранслятор на настоящих сокетах (токен, `Range`, `404`/`405`/`416`, закрытие), выбор адреса интерфейса, перевод статуса ресивера в прогресс, контроллер с фейковой платформой. `app/test/cast_ui_test.dart`: кнопка, список устройств, мини-пульт, окно «не воспроизведёт».
- Проверка F-Droid-артефакта на отсутствие `com.google.android.gms.cast` — вместе с PKG-010.

## Ссылки

- [Supported Media for Google Cast](https://developers.google.com/cast/docs/media)
- [Android Sender: custom device selection](https://developers.google.com/cast/docs/android_sender/integrate)
- [AVRoutePickerView](https://developer.apple.com/documentation/avkit/avroutepickerview), [AVRouteDetector](https://developer.apple.com/documentation/avfoundation/avroutedetector)
- `clients/flutter/app/lib/cast/`, `clients/flutter/app/android/app/src/{cast,nocast}/`, `clients/flutter/app/macos/Runner/MainFlutterWindow.swift`
