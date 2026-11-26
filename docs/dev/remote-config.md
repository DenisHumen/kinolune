# Удалённый конфиг: ключ minisign и подпись `config/*.json`

> **Статус (2026-11-15):** ключ конфига создан, его публичная часть — `PRODUCTION_PUBLIC_KEY` в `pa-remote-config`, секретная (с паролем) хранится у владельца офлайн. `config/remote-config.json` (serial 2) подписан. Ключ релизов (без пароля) лежит в секрете GitHub `MINISIGN_RELEASE_KEY`: CI подписывает `SHA256SUMS` каждого релиза (`SHA256SUMS.minisig`), публичная часть — `updates.release_public_key` в конфиге.
>
> **Опубликовано (TWI-030, 2026-11-26):** `config/remote-config.json` `serial` 3 — нативный плеер Twitch в full-сборке — подписан владельцем. Открытый ключ конфига — [`config/minisign.pub`](../../config/minisign.pub) (id `5F3D74272765CF77`, он же `PRODUCTION_PUBLIC_KEY`); ключ релизов — другой (`updates.release_public_key`).

> Задача CORE-014. Связано: [ADR-0004](../adr/0004-no-project-servers-local-sync.md) (своих серверов нет), [ADR-0039](../adr/0039-remote-config-minisign.md) (формат и правила доверия), [secrets.md](secrets.md), исследование 06 §2.6–2.7, крейт [`pa-remote-config`](../../core/crates/pa-remote-config).

Приложение вскоре после каждого запуска и дальше раз в несколько часов скачивает из этого репозитория `config/remote-config.json` и `config/remote-config.json.minisig`, проверяет подпись встроенным публичным ключом и только после этого читает флаги. Подписывает файл **только владелец, офлайн**. Секретный ключ никогда не попадает в git, CI, облако или на сервер сборки.

## Как это работает в приложении

| Шаг | Что происходит |
|---|---|
| Старт | Берётся встроенная копия (`core/crates/pa-remote-config/builtin/remote-config.json`), поверх — последняя валидная копия с диска, если её подпись снова проходит проверку и `serial` не меньше встроенного. Сеть не трогается. |
| Первая проверка | Через ~10 с после старта, в фоне, не задерживая первый кадр (BUG-030; full-сборка, `lib/twitch/remote_config_refresh.dart`). Первая проверка каждого запуска обязательна, даже если кэш проверялся минуту назад: она стоит один условный `GET`. В журнале — строки `Remote config: serial N (cache)` при старте и `Remote config: updated, serial N (remote)` / `unchanged …` после проверки. |
| Обновление | Дальше приложение спрашивает ядро каждые 30 мин и при возврате на передний план, а ядро ходит в сеть раз в `refresh.interval_secs` (15 мин … 7 дней, по умолчанию 6 ч): `GET` с `If-None-Match`. `304` — ничего не меняется. `200` — скачивается `.minisig`, проверяется подпись, потом JSON разбирается. |
| Применение | Новая `serial` действует сразу, без перезапуска: маршрут Twitch (нативный плеер или embed, kill-switch'и) пересчитывается, и следующий открытый плеер идёт по нему. Уже открытый плеер доигрывает как был. Остальные пользователи настройки (загрузки VOD, обновления приложения) читают её со следующего запуска. |
| Ошибка | Нет сети, неверная подпись, старый `serial`, новая `schema` — текущие флаги остаются, кэш на диске не трогается, повтор через `refresh.retry_secs`. |
| Зеркала | Сначала `raw.githubusercontent.com/…/main/config/`, затем GitHub Pages `…github.io/kinolune/config/` (если включены). |
| Ключ-заглушка | Пока `PRODUCTION_PUBLIC_KEY` — заглушка, приложение работает **только на встроенной копии** (и в debug, и в release), кэш игнорирует, в сеть не ходит. |

Подпись должна быть «prehashed» (`ED`, по умолчанию в minisign ≥ 0.6), а её доверенный комментарий — содержать `file:<имя файла>` (minisign пишет его сам). Это не даёт выдать один подписанный файл за другой.

## Формат документа

```json
{
  "schema": 1,
  "serial": 2,
  "issued_at": "2026-12-01T00:00:00Z",
  "refresh": { "interval_secs": 21600, "retry_secs": 1800 },
  "flags": {
    "twitch.gql_playback": true,
    "twitch.chapters_gql": true,
    "twitch.chat_replay": false,
    "twitch.archive": false
  },
  "kill_switches": [
    { "feature": "twitch.gql_playback", "max_app_version": "0.3.1", "platforms": ["ios"], "reason": "integrity token" },
    { "feature": "twitch.*", "min_app_version": "0.4.0-alpha.1", "max_app_version": "0.4.0-alpha.3" }
  ],
  "twitch": {
    "gql_client_id": "<публичный client-id>",
    "persisted_queries": { "VideoPlayer_ChapterSelectButtonVideo": "<sha256>" },
    "player": "native"
  }
}
```

- `serial` — увеличивать при **каждой** публикации. Документ с меньшим `serial`, чем уже есть у приложения, отвергается: старый подписанный файл не может выключить kill-switch обратно.
- `flags` — нет флага — значит выключен. `kill_switches` выключают функцию по шаблону имени (`twitch.*`, `*`), диапазону версий приложения (включительно, semver) и платформам; если версию приложения не удаётся разобрать, ограниченный по версиям kill-switch срабатывает.
- `twitch.player`: при выключенном или убитом `twitch.gql_playback` приложение всегда берёт `embed`. Неизвестное значение тоже означает `embed`.
- Kill-switch `twitch.native_player` (TWI-030) переключает **только плеер** на официальный embed (TWI-029), а главы из GQL оставляет. Флага с этим именем нет: плеер включает `player: "native"`. Его понимают сборки начиная с TWI-030. Kill-switch `twitch.gql_playback` (или `twitch.*`) выключает весь GQL — и плеер, и главы — и работает во **всех** выпущенных сборках. Версию для kill-switch'ей приложение берёт из сборки (`KINOLUNE_VERSION`, например `0.1.0-alpha.20`). Сборки до TWI-030 сообщали `0.1.0`: чтобы задеть только их, нужен диапазон `"min_app_version": "0.1.0", "max_app_version": "0.1.0"`.
- Нативное воспроизведение (TWI-004, `pa_twitch::playback`) требует `twitch.gql_playback: true`, `gql_client_id`, `player: "native"` и хэш операции `PlaybackAccessToken` в `persisted_queries`; без любого из них ответ — embed, запросов к Twitch нет. Работает только в full-сборке (возможность `twitch-native` крейта `pa-flavor`, `cfg(pa_twitch_native)`, CORE-013 — см. [flavors.md](flavors.md)); store-сборка всегда берёт embed (ADR-0011 §5). Ошибки GQL/usher имеют коды `gql_hash_invalid`, `integrity_required`, `usher_403` — по ним видно, что пора обновить хэш или выключить флаг.
- Главы VOD из GQL (TWI-008) требуют `twitch.chapters_gql` и хэш `VideoPlayer_ChapterSelectButtonVideo`. Хэш `VideoMetadata` необязателен: с ним VOD без `moments` (одна категория на весь эфир) получает одну главу с категорией VOD — ответ запроса глав категории не содержит (проверено на реальном VOD, [twitch-live-checks.md](twitch-live-checks.md)).
- Чат-replay VOD онлайн (TWI-016, `pa_twitch::chat_replay`) требует `twitch.chat_replay` (вместе с `twitch.gql_playback` и `gql_client_id`) и хэш `VideoCommentsByOffsetOrCursor`; только full-сборка. Без него у VOD нет панели чата; ошибки GQL (`gql_hash_invalid`, `integrity_required`, `not_found`) показываются как «Чат записи недоступен», видео играет дальше. Тот же флаг разрешает сохранять чат рядом со скачанным VOD (`.chat.json.gz`, TWI-017). Kill-switch `twitch.chat_replay` по версии и платформе выключает только чат.
- `updates.release_public_key` (PKG-018, [ADR-0088](../adr/0088-in-app-updates.md)) — строка публичного ключа minisign (`RW…`), которым владелец подписывает `SHA256SUMS` релизов приложения (`SHA256SUMS.minisig` в релизе, подпись `minisign -S -m SHA256SUMS`). Нет поля — приложение сверяет только контрольные суммы; есть — релиз без верной подписи не ставится. Ключ, который не разбирается, делает документ невалидным. Это **другой** ключ, не ключ конфига. Kill-switch `updates.install` (или `updates.*`) запрещает установку из приложения — остаётся ссылка на страницу релиза.
- Неизвестные поля игнорируются (поле `_comment` в примере — это допускает). Ломающее изменение формата — только новая `schema` вместе с релизом приложения.
- Встроенная копия в крейте — копия последнего подписанного документа на момент релиза. Перед релизом её обновляют, иначе приложение начнёт с более старых флагов (кэш и сеть всё равно победят, если `serial` больше).

## Ключ: создать один раз (владелец, офлайн)

Нужен [minisign](https://jedisct1.github.io/minisign/) (пакет `minisign` в Fedora/Debian/Homebrew, `scoop install minisign`). Лучше — на машине без сети или с live-USB.

```bash
# 1. Пара ключей, секретный — с паролем (не -W).
minisign -G -p kinolune-config.pub -s kinolune-config.key \
  -c "Kinolune remote config"

# 2. Секретный ключ и пароль — в офлайн-хранилище (менеджер паролей + копия
#    на зашифрованном носителе). В репозиторий, CI и облака не кладём.

# 3. Публичный ключ — вторая строка kinolune-config.pub:
sed -n 2p kinolune-config.pub
```

Эту строку (`RW…`, 56 символов) владелец вписывает в `PRODUCTION_PUBLIC_KEY` в [`core/crates/pa-remote-config/src/verify.rs`](../../core/crates/pa-remote-config/src/verify.rs) отдельным PR, и копирует файл в `config/minisign.pub` (для ручной проверки другими людьми). Тест `production_key_constant_is_placeholder_or_valid` проверит, что ключ разбирается; `just core remote-config-release-check` перестанет падать.

## Подписать и опубликовать

```bash
# правим config/remote-config.json, увеличиваем serial, затем:
KINOLUNE_CONFIG_KEY=/путь/к/kinolune-config.key just core sign-config
# = minisign -S -s "$KINOLUNE_CONFIG_KEY" -m <файл> для каждого config/**/*.json,
#   затем just core remote-config-verify (проверка как в приложении)

git add config/ && git commit -s -m "chore(config): <что поменяли>, serial N"
git push   # через PR или напрямую в main — решение владельца
```

Проверить чужие или свои подписи без секретного ключа:

```bash
just core remote-config-verify                      # встроенным ключом приложения
just core remote-config-verify pubkey=config/minisign.pub
minisign -Vm config/remote-config.json -p config/minisign.pub
```

Сырой файл на `raw.githubusercontent.com` обновляется с задержкой CDN (до ~5 мин), причём JSON и `.minisig` — независимо. В это окно приложение может увидеть новую пару «файл + старая подпись» и отвергнуть её; оно просто повторит позже. Ничего страшного, но срочные kill-switch'и лучше публиковать заранее проверенными.

## TWI-030: включить нативный плеер Twitch

Следующая версия (`serial` 3, [ADR-0148](../adr/0148-twitch-native-config-rollout.md)) подготовлена неподписанной в `tools/remote-config/remote-config.next.json`: `twitch.gql_playback` и `twitch.chapters_gql` включены, `player: "native"`, публичный client id сайта twitch.tv и хэши `PlaybackAccessToken`, `VideoPlayer_ChapterSelectButtonVideo` и `VideoMetadata`; TWI-016 добавил хэш `VideoCommentsByOffsetOrCursor` (из TwitchDownloader), флаг `twitch.chat_replay` в ней пока выключен — включает владелец после живой проверки (`real_twitch_chat_replay`). Источники значений — открытые клиенты (streamlink, yt-dlp, TwitchDownloader, twitch-dl); каждый хэш есть минимум в двух из них. `config/remote-config.json` пока не тронут: он подписан, и клиенты скачивают его с `main`.

**Проверка до подписи (debug, без ключа).** Debug-сборка ядра берёт неподписанный документ вместо встроенной копии, если задана переменная окружения. Кэш и сеть при этом не используются. В release-сборке этого кода нет.

```bash
# из clients/flutter/app; переменная читается при запуске (Linux, Windows, macOS)
KINOLUNE_DEBUG_REMOTE_CONFIG="$(git rev-parse --show-toplevel)/tools/remote-config/remote-config.next.json" \
CARGO_KINOLUNE_FLAVOR=full flutter run -d linux --dart-define=KINOLUNE_FLAVOR=full
```

Две оговорки (живая проверка 2026-11-26): `flutter build/run --debug` собирает Rust-ядро в release-профиле, где кода `KINOLUNE_DEBUG_REMOTE_CONFIG` нет — пересоберите `pa-frb` с `CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS=true`; без `--dart-define=KINOLUNE_VERSION=<версия>` локальная сборка сообщает `0.1.0` и попадает под kill-switch для старых сборок (0.1.0–0.1.0), то есть уходит на embed.

В логе: `remote config: DEBUG unsigned copy … (serial 3)`. Live и VOD должны играть в нативном плеере, у VOD должны быть главы.

**Шаги владельца (офлайн, ключ только у владельца):**

```bash
git switch main && git pull
cp tools/remote-config/remote-config.next.json config/remote-config.json
# при желании: поправить issued_at на текущее время (UTC)
KINOLUNE_CONFIG_KEY=/путь/к/kinolune-config.key just core sign-config
#   minisign спросит пароль ключа; затем just core remote-config-verify должен сказать OK
git add config/remote-config.json config/remote-config.json.minisig
git commit -s -m "chore(config): native Twitch player on, serial 3 [TWI-030]"
git push     # или через PR — как удобнее
```

После этого: отметить TWI-030 в ROADMAP. Встроенную копию (`core/crates/pa-remote-config/builtin/`) менять не нужно: опубликованная копия с `serial` 3 придёт при первой проверке — примерно через 10 с после запуска (до BUG-030, alpha.22 и раньше, приложение настройку не скачивало вовсе).

**Если Twitch сломает API** — новая версия (`serial` + 1) с kill-switch'ем, подписать так же:

```json
"kill_switches": [
  { "feature": "twitch.gql_playback", "platforms": ["windows"], "max_app_version": "0.1.0-alpha.25", "reason": "integrity token" }
]
```

Шаблоны: `twitch.gql_playback` — всё на embed (все сборки); `twitch.native_player` — только плеер на embed, главы остаются (сборки начиная с TWI-030); пустой `platforms` — все платформы.

## Если ключ скомпрометирован или потерян

1. Новый ключ (шаги выше), новый `PRODUCTION_PUBLIC_KEY`, релиз приложения.
2. Старые версии приложения продолжат верить старому ключу, пока не обновятся: держать их на безопасных значениях можно только тем же старым ключом. Поэтому секретный ключ хранится в двух офлайн-копиях.
3. Утечка ключа без релиза не лечится — отсюда правило «только офлайн».

## Релизы

`just core remote-config-release-check` падает, пока ключ — заглушка. Подключить его в релизный workflow (`app-v*`) — решение владельца: до появления ключа это заблокирует релизы, а без проверки релиз выйдет «только со встроенной копией» (безопасно, но без удалённых kill-switch'ей).
