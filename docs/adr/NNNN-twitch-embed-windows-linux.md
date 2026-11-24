<!-- SPDX-License-Identifier: MPL-2.0 -->

# ADR-NNNN: Официальный плеер Twitch на Windows и Linux — WebView2 через `webview_windows` и системный WebKitGTK поверх окна Flutter

- **Статус:** Предложен
- **Дата:** 2026-11-24
- **Решают:** владелец проекта (мейнтейнер); участники обсуждения — по PR
- **Связано:** `TWI-029`, `TWI-030`, `TWI-005`, `TWI-004`, `TWI-014`, [ADR-0051](0051-twitch-official-embed.md), [ADR-0049](0049-twitch-section-ui.md), [ADR-0039](0039-remote-config-minisign.md), [ADR-0041](0041-build-flavor-mechanism.md), отчёт владельца 2026-11-20 (alpha.17, Windows)

## Контекст и проблема

ADR-0051 подключил официальный embed Twitch через `webview_flutter`, а у него есть реализации только для Android, iOS и macOS. На Windows и Linux хук `embedPlayer` не регистрировался. Нативный плеер (TWI-004/TWI-014) выключен подписанным конфигом до TWI-030. Поэтому на компьютере владельца с alpha.17 вместо плеера открывалось окно «Откройте на Twitch», и смотреть Twitch внутри Kinolune было нельзя.

Нужно выбрать движок WebView для Windows и Linux. Условия: лицензия совместима с MPL-2.0 и без GPL (LGPL допустим только при динамической связи), embed работает с `parent` на нашем HTTPS-домене (GitHub Pages), есть JS-канал для позиции VOD (как в ADR-0051), есть полный экран и клавиатура. Приложение должно запускаться там, где WebView нет.

## Факторы решения

- Видео Twitch — H.264 (+ AAC) через MSE: движок обязан его декодировать.
- Нет GPL; размер сборки и AppImage не должен заметно вырасти.
- Страница-обёртка должна иметь HTTPS-источник `https://denishumen.github.io` без сетевого запроса к этому домену (ADR-0051 §2).
- Один JS-протокол страницы (`twitchEmbedHtml`) и одна `TwitchEmbedSession` на всех платформах.
- Нативный плеер остаётся первым выбором, когда конфиг его разрешает (`decideTwitchRoute` не меняется).

## Рассмотренные варианты

1. **Windows: `webview_windows` (WebView2). Linux: системный WebKitGTK, который сам runner кладёт поверх FlView.**
2. `flutter_inappwebview` (Apache-2.0): Windows есть, Linux в стабильной версии (6.1.5) нет — Linux не решает.
3. `webview_cef` / CEF (BSD + Chromium): ~150–200 МБ на платформу, стандартные сборки CEF без проприетарных кодеков H.264 — Twitch не играет.
4. `desktop_webview_window` (Apache-2.0, WebKitGTK на Linux): отдельное окно вне приложения, грузит только URL (без `loadHtml` с базовым URL), полный экран и пульт — в чужом окне. Отклонено ещё в ADR-0051.
5. Только браузер (как было). Не выполняет задачу.

## Решение

Выбран вариант **1**.

### Windows — Microsoft Edge WebView2 через `webview_windows` 0.4.0

- Лицензия плагина BSD-3-Clause (Niklas Schulze). WebView2 SDK (NuGet `Microsoft.Web.WebView2`, BSD-3-Clause, Microsoft) скачивается CMake-сценарием плагина при сборке. Сам рантайм WebView2 системный: он есть в Windows 11 и в Windows 10 с обновлениями Edge, в сборку не входит.
- Плагин рисует WebView2 в текстуру Flutter (Windows.Graphics.Capture), поэтому плеер — обычный виджет, и над ним работают диалоги.
- **`parent`:** `loadStringContent` даёт источник `about:blank`, а Twitch его не примет. Поэтому страница пишется во временную папку, и эта папка отображается на хост `parent` через `SetVirtualHostNameToFolderMapping`. Страница загружается как `https://denishumen.github.io/kinolune/twitch-embed.html`: WebView2 отдаёт её сам, в сеть к GitHub запрос не уходит, а источник у страницы — наш HTTPS-домен. При закрытии экрана папка удаляется.
- JS-канал: страница пишет в `KinoluneEmbed.postMessage`, а если его нет — в `chrome.webview.postMessage`. Протокол событий тот же, что в ADR-0051.
- Полный экран: кнопка плеера Twitch → `ContainsFullScreenElementChanged` → окно на весь экран (`setWindowFullscreen`, тот же код media_kit, что у плеера Kinolune), шапка экрана скрывается.
- Всплывающие окна (логотип Twitch) запрещены, для этого есть кнопка «Открыть на Twitch» в шапке. Если главный фрейм уходит со страницы, ссылка открывается в браузере, а страница загружается заново.
- Нет рантайма (`getWebViewVersion() == null`) → хук не регистрируется, а диалог «Открыть на Twitch» просит установить «Microsoft Edge WebView2 Runtime».

### Linux — системный WebKitGTK, слой GTK поверх Flutter

- Flutter на Linux не поддерживает platform views. Поэтому окно runner'а теперь `GtkOverlay`: основной ребёнок — `FlView`, а `WebKitWebView` лежит слоем поверх, в прямоугольнике, который сообщает Dart (`linux/runner/twitch_webview.cc`, канал `app.kinolune/twitch_webview`). Виджет-заглушка сообщает свой прямоугольник после каждого кадра, поэтому вид следует за переходами маршрутов, сменой размера окна и полным экраном. Над нативным видом Flutter ничего нарисовать не может, поэтому вид прячется, пока открыт диалог или сверху лежит другой маршрут. По правилам Twitch плеер и так ничем не перекрывается.
- **WebKitGTK открывается через `dlopen`** (`libwebkit2gtk-4.1.so.0`, затем `libwebkit2gtk-4.0.so.37`; `webkitgtk-6.0` — это GTK 4, в одном процессе с GTK 3 работать не может). Бинарник с ним не связан, и в AppImage и tar.gz WebKit не входит, поэтому размер сборки не растёт. Без WebKitGTK приложение работает как прежде, а диалог просит установить `libwebkit2gtk-4.1`. Лицензия WebKitGTK — LGPL-2.1 (WebCore) и BSD-2-Clause (JavaScriptCore). Это системная библиотека, загружаемая динамически, что допускает и LGPL, и наши правила (без GPL в клиентах).
- Страница загружается `webkit_web_view_load_html(html, base_uri)` с базовым URL `https://denishumen.github.io/kinolune/twitch-embed`, как на macOS. JS-канал — `webkit.messageHandlers.KinoluneEmbed`; пользовательский скрипт определяет `window.KinoluneEmbed.postMessage`.
- Декодирование видео идёт через GStreamer того же WebKit: H.264 даёт системный `gst-libav` (LGPL) или `openh264`. В популярных дистрибутивах (Ubuntu, Fedora с RPM Fusion или openh264) он ставится вместе с браузером; если его нет, Twitch покажет свою ошибку воспроизведения.
- Полный экран: кнопка плеера Twitch → WebKit сам разворачивает окно (`enter-fullscreen` не перехватывается) и сообщает Dart, а шапка скрывается. `Escape` внутри веб-вида выводит из полного экрана (WebKit), а без него закрывает экран (событие `back`).
- Ссылки в новом окне (`NEW_WINDOW_ACTION`) уходят в браузер. Если главный фрейм ушёл со страницы, ссылка тоже открывается в браузере, а страница загружается заново. Если упал процесс WebKit, показывается «Плеер Twitch не загрузился» с повтором.
- `KINOLUNE_TWITCH_WEBKIT=0` выключает движок (проверка запасного пути).

### Общее для компьютеров

- `TwitchEmbedEngine` (`webviewFlutter`, `webView2`, `webKitGtk`) выбирается по платформе (`twitchEmbedEngineFor`). `registerTwitchEmbed` проверяет наличие рантайма при запуске, не задерживая старт, и либо регистрирует `embedPlayer`, либо ставит `TwitchService.embedMissing`. `decideTwitchRoute` не меняется: нативный плеер (full + разрешение конфига) → embed → «Открыть на Twitch». VOD только для подписчиков по-прежнему всегда уходят на Twitch.
- Клавиатура и пульт на экране плеера: пробел/K/Play-Pause — пауза, ←/→ (и перемотка с пульта) — ±10 с в VOD, F и кнопка «Полный экран (F)» в шапке — полный экран окна без шапки, Escape — выход из полного экрана, затем с экрана. Команды идут в страницу через `window.kinolune.*`.
- Вход в Twitch не нужен, реклама и sub-gate — как у Twitch. Позиция VOD, «Продолжить» и синхронизация — как в ADR-0051.

### Последствия

- Store- и full-сборки на Windows и Linux смотрят live и VOD внутри Kinolune при выключенном нативном плеере. Когда TWI-030 включит нативный плеер, он станет первым выбором, а embed — запасным путём и kill-switch'ем.
- Добавлена одна зависимость pub (`webview_windows`, BSD-3-Clause). На Linux новых зависимостей нет: код runner'а под MPL-2.0, а WebKitGTK системный.
- Windows-сборка при первом CMake-конфиге скачивает `nuget.exe` и пакет WebView2 SDK (так устроен плагин). Если CI упрётся в сеть, пакет можно положить в кэш.
- `webview_windows` не обновлялся с 2024 года. Если он перестанет собираться новым Flutter, замена — свой небольшой плагин на том же WebView2 SDK, а Dart-интерфейс `TwitchEmbedSurface` останется прежним.
- Слой GTK над Flutter — компромисс: всё, что Flutter рисует поверх плеера, скрыто под ним. Поэтому вид прячется под диалогами. Когда во Flutter для Linux появятся platform views, слой можно заменить, не трогая Dart-протокол.
- Тесты подменяют web view фейком (`TwitchEmbedSurfaceFactory`), а канал runner'а — моком: проверяются выбор движка по платформе, регистрация и запасной путь без рантайма, приоритет нативного плеера, клавиши и полный экран, прямоугольник и скрытие нативного вида. Живой Twitch проверяется вручную (скриншоты в PR).

## Решения владельца

- Подтвердить WebView2 (Windows) и системный WebKitGTK через `dlopen` (Linux) вместо встроенного движка.
- Нужна ли в будущих пакетах (Flatpak, .deb, .rpm) явная зависимость `libwebkit2gtk-4.1-0` и `gstreamer1.0-libav` (сейчас — «рекомендуется», приложение работает и без них).
