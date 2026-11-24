<!-- SPDX-License-Identifier: MPL-2.0 -->

# AGENTS.md — `server/` (медиасервер)

Общие правила — в корневом [AGENTS.md](../AGENTS.md).

- **Что здесь:** медиасервер пользователя на axum и tokio (SRV) и веб-админка `server/web/`. Сервер работает у пользователя (на ПК или в NAS-ОС), облака проекта нет (ADR-0004). Использует те же крейты ядра, что и клиенты (`pa-library` и др.).
- **Workspace:** крейт `pa-server` (бинарь `alpha-media`, как юнит NAS-ОС по ADR-0010) входит в общий Cargo workspace (`"server"` в `members` корневого `Cargo.toml`).
- **Устройство:** `pa-runtime` даёт tokio-runtime, каталоги и настройки; `pa-library` — БД медиатеки `library.sqlite3` (SQLite WAL, та же схема и миграции, что в клиенте). Сервер: конфиг (`src/config.rs`), axum-роутер (`src/http.rs`), структурные логи `tracing` (text или JSON, `src/logging.rs`), graceful shutdown по Ctrl-C/SIGTERM с checkpoint WAL. Хаб синхронизации (`src/sync/`, SYNC-008): WebSocket `/api/v1/sync` по протоколу `docs/api/sync-protocol.md` (§8 — детали хаба), журнал в `sync.sqlite3` рядом с медиатекой, «сейчас играет» — только в памяти.
- **Конфиг:** TOML-файл (`--config` или `ALPHA_MEDIA_CONFIG`), поверх — `ALPHA_MEDIA_LISTEN`, `ALPHA_MEDIA_DATA_DIR`, `ALPHA_MEDIA_LOG`, `ALPHA_MEDIA_LOG_FORMAT`, `ALPHA_MEDIA_TLS`, `ALPHA_MEDIA_MDNS` (`alpha-media --help`); секции `[sync]` — `playback_ttl_secs`, `playback_max_entries`, `max_connections`, `require_pairing` (по умолчанию `true`: только сопряжённые устройства, SYNC-005; семья устройств — `src/pairing.rs`, ключи в зашифрованном файле секретов и roster `devices.json` в каталоге данных; QR приглашения несёт адрес и `fp` сертификата); `[tls]` — `enabled`, `cert`, `key`; `[discovery]` — `enabled`, `name`; `[auth]` — `trust_loopback` (по умолчанию `false`). Порт по умолчанию — **8487** (ADR-0118: свободен в IANA, не пересекается с Jellyfin 8096/8920, Plex 32400 и т. п.). По умолчанию слушает только `127.0.0.1:8487`: наружу — явной настройкой `listen`.
- **Аккаунты и токены** (`src/auth/`, SRV-003, ADR-NNNN):
  - `auth.sqlite3` — пользователи (`admin`/`member`, Argon2id-хэши пароля и PIN), сессии, хэши refresh-токенов.
  - Первый администратор — `POST /api/v1/auth/setup` с самой машины или с кодом из `<data_dir>/setup-code.txt`. До этого loopback администрирует без входа, после — только токен администратора, вошедшего паролем.
  - Access — PASETO v4.public (`src/auth/paseto.rs`, ключ `server.auth.signing-key` в зашифрованном файле секретов), 15 мин; каждый запрос сверяется с сессией и ростером устройств. Refresh — одноразовый, повторное предъявление отзывает сессию.
  - Устройство: challenge → подпись ключом семьи → сессия устройства → профили и вход по PIN.
  - Блокировка перебора — `src/auth/limit.rs`.
  - Слой `authenticate` в `src/http.rs` отвергает неверный `Bearer` на всех `/api/`, кроме `/refresh` и `/logout`.
- **TLS** (`src/tls.rs`, SRV-004, ADR-0118) — только rustls с провайдером ring (OpenSSL запрещён в `deny.toml`). По умолчанию HTTPS: самоподписанный ECDSA P-256 сертификат создаётся при первом старте в `<data_dir>/tls/server.crt` и `server.key` (ключ 0600) или берётся из `[tls] cert/key`. Клиенты не проверяют CA, а пиннят отпечаток `sha256:<hex>` DER-сертификата (TOFU, `pa_discovery::PinnedVerifier`). HTTP без TLS — только `[tls] enabled = false` / `ALPHA_MEDIA_TLS=off`.
- **mDNS** (`src/announce.rs`): анонс `_projectalpha._tcp` через `pa_discovery::Advertiser`, TXT `txtvers`, `id` (= id хаба синхронизации), `v` (мажор API), `name`, `fp`. На loopback-адресе не анонсируется; ошибка mDNS не роняет сервер.
- **Контракт HTTP API** (SRV-002, ADR-0118) — `docs/api/openapi.json`, OpenAPI 3.1, генерируется из `#[utoipa::path]` обработчиков (`src/api.rs`, `src/http.rs`: маршруты регистрируются через `utoipa_axum::routes!`, руками файл не правится). Новый эндпоинт: обработчик с `#[utoipa::path]` → `.routes(routes!(…))` в `api_router()` → сценарий в `server/tests/contract/live.rs` → `just server openapi`. Версия API: `api::API_MAJOR` (TXT `v`, `/api/v1`) и `API_MINOR` (растёт при добавлениях); внутри major — только добавления, проверяется по замороженным контрактам `docs/api/compat/` (`just server openapi-freeze` при серверном релизе). Ошибки — JSON `{"error": "…"}` (`api::ApiError`).

## Сборка и тесты (из корня репозитория)

```bash
cargo check -p pa-server
cargo clippy -p pa-server --all-targets -- -D warnings
cargo nextest run -p pa-server
cargo run -p pa-server -- --help
cargo test -p pa-server --test contract   # спецификация = код, контракт-тесты (N-1)
just server openapi      # перегенерировать docs/api/openapi.json
just server run          # состояние в target/alpha-media
curl -sk https://localhost:8487/health
```

## Правила области

- В каждом `.rs` первой строкой `// SPDX-License-Identifier: MPL-2.0`.
- Секреты сервера (пароли пользователей, токены) — только в его конфиге и хранилище у пользователя, никогда в репозитории и логах.
- Никаких обращений к серверам проекта и телеметрии без opt-in; запросы к TMDB и другим провайдерам идут напрямую.
