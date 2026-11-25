# Документация API (DOC-004)

Контракты, которые клиентам, серверу и инструментам сборки нужно читать без исходников. Своих серверов у проекта нет ([ADR-0004](../adr/0004-no-project-servers-local-sync.md)): всё ниже — либо API медиасервера, который пользователь запускает у себя, либо статические файлы в этом репозитории и на GitHub.

| Файл | Что описывает | Источник истины |
|---|---|---|
| [`openapi.json`](openapi.json) | HTTP API медиасервера Kinolune (OpenAPI 3.1, `/api/v1`) | код сервера, генерируется ([ADR-0118](../adr/0118-openapi-contract.md)) |
| [`index.html`](index.html) | просмотр `openapi.json` в браузере (Redoc CE) | этот файл |
| [`compat/`](compat/) | замороженные контракты прошлых версий (N-1) | `just server openapi-freeze` |
| [`sync-protocol.md`](sync-protocol.md) | протокол синхронизации устройств: версии, кадры, совместимость | SYNC-002 |
| [`remote-config.schema.json`](remote-config.schema.json) | подписанный удалённый конфиг `config/remote-config.json` (флаги Twitch, kill-switch'и, ключ релизов) | `core/crates/pa-remote-config/src/model.rs` |
| [`releases.schema.json`](releases.schema.json) | поля ответа GitHub Releases API, которые читает встроенное обновление приложения | `clients/flutter/app/lib/features/updates/releases.dart` |
| [`channels.schema.json`](channels.schema.json) | **черновик** манифеста канала обновлений Kinolune OS | ещё не реализован (SITE-004) |

## HTTP API сервера

`openapi.json` генерируется из `#[utoipa::path]`-обработчиков `server/` — **руками не править**. Новый эндпоинт и перегенерация — по [`server/AGENTS.md`](../../server/AGENTS.md) (`just server openapi`); workflow [`openapi.yml`](../../.github/workflows/openapi.yml) и `rust.yml` проверяют, что файл совпадает с кодом, сервер отвечает по спецификации и замороженные контракты `compat/` не нарушены.

Коротко о текущей версии (`info.version` 1.1):

- **Аутентификация** (SRV-003): `POST /api/v1/auth/setup` (первый администратор), `login` (имя и пароль), `challenge` + `device` (вход привязанного устройства его ключом), `pin` и `GET profiles` (профили на привязанном устройстве), `refresh`, `logout`, `GET me`. Защищённые эндпоинты требуют `Authorization: Bearer <токен>` — PASETO v4.public, живёт 15 минут, продлевается через `/api/v1/auth/refresh`.
- **Пользователи и устройства** (с токеном): `/api/v1/users`, `/api/v1/devices`, приглашение к привязке — `POST /api/v1/pair/invite`.
- **WebSocket:** привязка `GET /api/v1/pair`, синхронизация `GET /api/v1/sync` (протокол — [`sync-protocol.md`](sync-protocol.md)).
- **Активность сервера** (SRV-019): `GET /api/v1/system/activity` — занят ли сервер (просмотр, запись, сканирование), чтобы агент обновлений ОС отложил перезагрузку. Только с самой машины сервера (loopback), без токена.
- **Проверка живости:** `/health`, `/healthz` — без токена.
- Ошибки — JSON `{"error": "…"}`. Внутри major-версии API только добавления; клиенты и серверы, чьи major отличаются не больше чем на один, работают вместе (ADR-0118).

Точный список полей и кодов ответа — только в `openapi.json`; этот список — оглавление, при расхождении прав `openapi.json`.

### Просмотр

`index.html` — статическая страница: грузит Redoc CE 2.5.4 с jsDelivr (точная версия, проверка Subresource Integrity) и рендерит лежащий рядом `openapi.json`. При открытии страницы браузер обращается к `cdn.jsdelivr.net`. Локально:

```bash
cd docs/api && python3 -m http.server 8000   # затем http://localhost:8000/
```

Открыть файл напрямую (`file://`) не получится: браузер не даст странице прочитать `openapi.json`. Без JavaScript страница показывает ссылку на сам JSON.

## Удалённый конфиг

Формат, правила доверия и подпись — [docs/dev/remote-config.md](../dev/remote-config.md) и [ADR-0039](../adr/0039-remote-config-minisign.md). Схема проверяет только структуру: подпись `remote-config.json.minisig` (minisign) и запрет отката `serial` проверяет приложение. Клиенты игнорируют неизвестные поля, поэтому схема их тоже допускает; неизвестное значение `twitch.player` означает `embed`.

Проверить файл схемой:

```bash
python3 -c 'import json,jsonschema; jsonschema.validate(json.load(open("config/remote-config.json")), json.load(open("docs/api/remote-config.schema.json")))'
```

## Обновления приложения

Отдельного `releases.json` пока нет (PKG-001 открыт). full-сборка приложения (PKG-018, PKG-022, [ADR-0088](../adr/0088-in-app-updates.md)) читает публичный GitHub Releases API: `GET https://api.github.com/repos/DenisHumen/kinolune/releases?per_page=30`, без токена и идентификаторов устройства. Из списка берутся только релизы не-черновики с тегом `app-v<semver>`; канал «стабильные» пропускает пре-релизы. Устанавливается только релиз с `SHA256SUMS` и, если в конфиге есть `updates.release_public_key`, с верной подписью `SHA256SUMS.minisig`. store-сборки обновляет магазин. В CI ответ API подменяет [`release-json.py`](../../packaging/app/update-test/release-json.py).

## Каналы Kinolune OS (черновик)

`channels.schema.json` повторяет манифест из [исследования 05 §4.1](../research/05-nas-linux-distro.md) и [architecture.md](../dev/architecture.md): `version`, `digest`, `rollout`, `min_from`, `notes_url`, `requires_reboot`, `halt`. Он ещё не реализован и не публикуется; формат закрепит SITE-004 (подписанный статический файл на GitHub Pages, не сервер проекта).

## Публикация

Папка самодостаточна: ссылки внутри `index.html` относительные, её можно выложить на любой статический хостинг. Публикация на сайте — задача SITE-003 (сайт ещё не настроен, SITE-001).
