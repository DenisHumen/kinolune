<!-- SPDX-License-Identifier: MPL-2.0 -->

# Документация API

Эта папка содержит контракты, которые клиентам и инструментам сборки нужно
читать без доступа к исходникам:

- [`index.html`](index.html) отображает спецификацию сервера
  [`openapi.json`](openapi.json) через Redoc CE. HTML и спецификация — статические
  файлы; Redoc CE загружается с CDN. Для локального просмотра запустите из этой
  папки `python3 -m http.server 8000` и откройте `http://localhost:8000/`.
- [`sync-protocol.md`](sync-protocol.md) описывает версии, кадры и совместимость
  протокола синхронизации устройств.
- [`remote-config.schema.json`](remote-config.schema.json) описывает структуру
  подписанного `config/remote-config.json`. Файл и `.minisig` должны проверяться
  вместе; JSON Schema не проверяет подпись или защиту от отката `serial`.
- [`releases.schema.json`](releases.schema.json) описывает используемую приложением
  часть ответа GitHub Releases API. В приложении источник —
  `GET /repos/DenisHumen/kinolune/releases`; CI-фикстура создаётся
  `packaging/app/update-test/release-json.py`.
- [`channels.schema.json`](channels.schema.json) фиксирует черновой формат
  манифеста обновлений Kinolune OS из `docs/research/05 §4.1`. Контракт ещё не
  реализован и не публикуется; перед использованием его должен закрепить SITE-004.

## Обновление спецификации сервера

`openapi.json` генерируется серверным контрактом. Не редактируйте его вручную:
измените API в `server/`, запустите генерацию из инструкции в
`server/AGENTS.md`, затем проверьте diff. Workflow
[`openapi.yml`](../../.github/workflows/openapi.yml) запускает контрактные тесты
для изменений `docs/api/**`.

## Публикация

Папку можно разместить целиком на статическом хостинге: относительные ссылки в
`index.html` и README сохраняют структуру. Сайт и автоматическая публикация
документации ещё не настроены (SITE-001, SITE-003); до их готовности эта папка
остаётся источником для будущей публикации.
