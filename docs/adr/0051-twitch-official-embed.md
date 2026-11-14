<!-- SPDX-License-Identifier: MPL-2.0 -->

# ADR-0051: Официальный embed Twitch как запасной плеер — WebView со страницей-строкой и `parent` домена проекта, синхронизация позиции VOD через JS API, браузер на Linux

- **Статус:** Предложен
- **Дата:** 2026-11-14
- **Решают:** владелец проекта (мейнтейнер); участники обсуждения — по PR
- **Связано:** `TWI-005`, `TWI-004`, `TWI-013`, `TWI-014`, `SYNC-003`, `docs/research/06-twitch-integration.md` §2.2, [ADR-0011](0011-maintainer-decisions-release-0-1.md) §5 (DEC-006), [ADR-0039](0039-remote-config-minisign.md), [ADR-0041](0041-build-flavor-mechanism.md), [ADR-0044](0044-playback-handoff.md), [ADR-0049](0049-twitch-section-ui.md)

## Контекст и проблема

Store-сборки смотрят Twitch только через официальный встраиваемый плеер (ADR-0011 §5). Full-сборки переходят на него же, когда подписанный конфиг говорит `player = embed` или выключает GQL (ADR-0039). ADR-0049 оставил для этого хук `TwitchService.embedPlayer`. Нужно решить:

1. чем показывать embed на каждой платформе и что делать там, где WebView нет;
2. как выполнить требование Twitch о параметре `parent` (домен страницы, которая встраивает плеер, с HTTPS) в приложении без своего сервера;
3. как позиция VOD в embed попадает в прогресс Kinolune: «Продолжить», синхронизацию между устройствами, передачу просмотра (SYNC-003);
4. как «Открыть на Twitch» открывает ссылку напрямую (решение владельца по ADR-0049: добавить `url_launcher`).

## Решение

1. **WebView — `webview_flutter`** (BSD-3-Clause, flutter/packages) с `webview_flutter_android` и `webview_flutter_wkwebview` для автозапуска без жеста и inline-воспроизведения. Реализации есть для Android (включая Android TV), iOS и macOS. Для Linux и Windows реализации нет. Совместимые по лицензии варианты (`desktop_webview_window`, `webview_cef`, `webview_windows`) открывают отдельное окно, тянут CEF или поддерживают только одну ОС, и ни один не даёт JS-канал и общий виджет так же, как Android. Поэтому на Linux и Windows хук не регистрируется: раздел показывает «Открыть на Twitch», и кнопка «Открыть» передаёт ссылку браузеру. На tvOS WebView нет вовсе (06 §2.2).
2. **Страница — строка, `parent` — домен проекта.** Приложение загружает маленькую страницу (`twitchEmbedHtml`) через `loadHtmlString` с базовым URL `https://<parent>/kinolune/twitch-embed`. Источник документа в WebView — этот HTTPS-домен, поэтому embed видит своего родителя, а с домена ничего не скачивается. По умолчанию `parent` — `denishumen.github.io`, GitHub Pages публичного репозитория (HTTPS, домен проекта, не чужой). Сборка может задать другой домен через `--dart-define=KINOLUNE_TWITCH_EMBED_PARENT`. На странице работает интерактивный embed Twitch (`player.twitch.tv/js/embed/v1.js`, `new Twitch.Player`): live — `channel`, VOD — `video=v<id>` и `time=1h2m3s` для продолжения. Client id embed не нужен, вход в Twitch не нужен. Значения передаются как JSON с экранированным `<`.
3. **Правила Twitch.** Плеер занимает весь WebView, панель приложения стоит над ним и ничего не перекрывает. На телефоне экран embed открывается в альбомной ориентации, чтобы плеер был не меньше 400×300. Ссылки, уводящие страницу с базового URL (логотип Twitch, имя канала), открываются в приложении Twitch или в браузере, а не внутри WebView. Реклама, sub-gate и Turbo работают как у Twitch: embed не меняется.
4. **Синхронизация позиции — JS API.** Страница пересылает события `READY`, `PLAY`, `PAUSE`, `SEEK`, `ENDED`, `ONLINE`, `OFFLINE` в JS-канал `KinoluneEmbed`. Во время воспроизведения VOD она раз в 5 с шлёт `getCurrentTime()`/`getDuration()`. `TwitchEmbedSession` сохраняет позицию на `time`, `pause`, `seek` и при закрытии экрана, а на `ENDED` отмечает VOD просмотренным. Нулевую позицию до применения `time` она не пишет. Прогресс идёт в `Library.saveProgress` с id `twitch:vod:<id>`; это и есть `MediaKey` VOD. Ядро принимает такой id: `CoreLibrary.save_progress` → `Catalog::save_key_progress` → `pa_library::progress` через реплику синхронизации, как прогресс фильмов. Поэтому VOD появляется в «Продолжить» раздела и главной (`continue_vods`) и синхронизируется между устройствами. `HandoffController` получает `begin`, `report` и `end` с тем же id. `CoreSync` понимает `twitch:vod:<id>` в heartbeat, takeover и `resume_hint` и возвращает его в событиях и «Смотрят на других устройствах». Live не пишет прогресс.
5. **Регистрация.** `main.dart` вызывает `registerTwitchEmbed(services.twitch)`: где WebView есть, `embedPlayer = openTwitchEmbed`. Демо-режим оставляет «Открыть на Twitch», потому что его каналы выдуманы. Выбор между native, embed и «Открыть на Twitch» по-прежнему делает `decideTwitchRoute` (ADR-0049). Флаг `player = embed` или отсутствие нативного плеера ведут в embed.
6. **«Открыть на Twitch».** `url_launcher` (BSD-3-Clause): на телефоне и десктопе в диалоге появляется кнопка «Открыть», которая отдаёт ссылку приложению Twitch или браузеру (`LaunchMode.externalApplication`). На TV кнопки нет, остаются QR-код и копирование. Из embed VOD открывается на Twitch с текущей позицией (`?t=`). На Android добавлен `<queries>` для https-ссылок.

## Последствия

- Store-сборки на Android смотрят live и VOD внутри приложения; позиция VOD видна в «Продолжить» и на других устройствах.
- На Linux и Windows embed нет: только браузер. Если появится совместимый WebView с JS-каналом, хватит одной реализации `TwitchEmbedSurface`.
- Если Twitch начнёт отвергать страницы, загруженные строкой (проверка `parent` по `ancestorOrigins` или Referer), запасной путь такой: опубликовать ту же страницу на GitHub Pages и загружать её по URL. Код страницы и JS-канал при этом не меняются.
- Тесты подменяют WebView фейковой поверхностью (`TwitchEmbedSurfaceFactory`), которая отправляет сообщения страницы, и в сеть не ходят. Rust-тесты `pa-catalog` проверяют сохранение по ключу, «Продолжить» и операцию синхронизации.

## Рассмотренные варианты

- **Страница-обёртка на GitHub Pages, загружаемая по URL** (так в ROADMAP). Отложено: нужен деплой Pages и сетевой запрос при каждом открытии, а `parent` тот же. Это путь на случай, если вариант со строкой перестанет работать.
- **`parent=localhost` или `file://`.** Отклонено: документация Twitch требует HTTPS-домен, и такие значения не описаны.
- **Неинтерактивный iframe без JS API.** Отклонено: позицию VOD не получить, а «done when» требует синхронизацию.
- **Отдельное окно WebView на Linux (`desktop_webview_window`).** Отклонено для 0.x: окно вне приложения, нет общего JS-канала на всех платформах, и для такого окна нужен webkit2gtk.

## Решения владельца

- Домен `parent` по умолчанию: GitHub Pages репозитория (`denishumen.github.io`) или свой домен проекта, если он появится. Меняется одной константой или `--dart-define`.
- Включать ли GitHub Pages со страницей-обёрткой заранее, как запасной путь из «Последствий».
