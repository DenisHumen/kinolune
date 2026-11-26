<!-- SPDX-License-Identifier: MPL-2.0 -->
# distro/updated — `alpha-updated`, агент обновлений NAS-ОС (Go)

Демон проверяет, скачивает и ставит новые образы ОС через штатный `bootc`, умеет откатываться и перезагружает машину только в окне обслуживания, когда никто ничего не смотрит. Архитектура — [исследование 05 §4.1–4.2](../../docs/research/05-nas-linux-distro.md), решения — [ADR-0164](../../docs/adr/0164-alpha-updated-agent.md), фаза 0 — [ADR-0010](../../docs/adr/0010-nas-os-phase0.md).

Стандартный `bootc-fetch-apply-updates.timer` в образе **замаскирован** (вместе с его `.service`): он сам ставит обновление и перезагружает машину посреди фильма. Вместо него работает `alpha-updated`.

## Что делает

| Операция | Команда bootc | Перезагрузка |
|---|---|---|
| проверка | `bootc upgrade --check` (только манифест), затем `bootc status --format=json` → `cachedUpdate` | нет |
| загрузка | `bootc upgrade --download-only`: образ скачан и подготовлен, но **заблокирован** — случайная перезагрузка его не применит | нет |
| применение | `bootc upgrade --from-downloaded` (разблокировать скачанное) или `bootc upgrade`, если не скачано; затем `systemctl reboot` | да |
| смена канала | `bootc switch <репозиторий>:<канал>`; новый деплоймент применится при следующей перезагрузке (в окне или по кнопке) | нет |
| откат | `bootc rollback`; с `reboot: true` — сразу `systemctl reboot` | по запросу |
| статус | `bootc status --format=json` | — |

Одновременно идёт **одна** операция; вторая получает `409 busy`. Пока операция идёт, статус берётся из кэша (bootc может держать блокировку).

### Состояния обновления (`update.state`)

`up-to-date` → `available` (нашли при проверке) → `downloaded` (скачано, заблокировано) → `staged` (разблокировано или новый канал: применится при перезагрузке). Ещё есть `rollback-queued` (откат применится при перезагрузке) и `unknown` (не удалось прочитать `bootc status`, текст ошибки — в `bootc_error`).

Старые версии bootc не пишут поле `downloadOnly`; тогда агент помнит digest скачанного в `state.json`.

## Настройки

Хранятся в `/var/lib/alpha/updated/settings.json` (запись атомарная: временный файл, fsync, rename; права 0600).

| Поле | Значения | По умолчанию |
|---|---|---|
| `channel` | `stable`, `beta` | канал, с которого установлено устройство (тег образа), иначе `stable` |
| `auto` | `off` — только проверять и показывать «есть обновление»; `download` — ещё и скачивать в фоне; `install` — скачать и поставить в окне обслуживания. Можно передать `true`/`false` (= `install`/`off`) | `install` |
| `window` | `"HH:MM-HH:MM"` по местному времени устройства, может переходить через полночь (`"23:30-01:00"`), конец не включается | `"03:00-05:00"` |

В `state.json` рядом — время последней проверки, запрос «поставить в окно», последняя ошибка и запись о последнем применении (`pending` → `booted` или `not-booted`, если после перезагрузки загрузилась старая версия: откат greenboot или выбор в меню загрузки).

## Планировщик

Шаг раз в минуту (первый — через 2 минуты после старта):

1. Проверка раз в 6 часов (`-check-interval`); после ошибки — повтор не раньше чем через 30 минут.
2. Если есть обновление и `auto` ≠ `off` (или пользователь попросил «поставить в окно») — скачать (`--download-only`). Ошибка загрузки повторяется не чаще раза в 30 минут.
3. Если скачано и (`auto` = `install` или есть запрос «в окно») и сейчас окно обслуживания — спросить медиасервер. Если он занят или не ответил — отложить до следующего шага (в журнал пишется не чаще раза в 15 минут). Иначе — разблокировать, **ещё раз** спросить медиасервер прямо перед перезагрузкой и перезагрузиться.

Явные действия пользователя («Обновить сейчас», «Откатить и перезагрузить») вето медиасервера **не** проверяют: это решение человека, предупреждать о просмотре должен интерфейс.

## Вето медиасервера

Интерфейс в коде — `agent.Vetoer` (`Busy(ctx) (busy, reason, err)`), реализация — `agent.HTTPVeto`, несколько источников объединяет `agent.MultiVeto`.

Медиасервер отдаёт этот адрес с SRV-019. Он отвечает только самой машине (loopback), на запросы с других адресов возвращает 403. Сессии на паузе тоже считаются занятостью.

```http
GET https://127.0.0.1:8487/api/v1/system/activity
200 OK
{"busy": false, "sessions": 0, "reason": ""}
```

Настройки:

- `ALPHA_MEDIA_ACTIVITY_URL` (`-media-activity-url`) — адрес, по умолчанию `https://127.0.0.1:8487/api/v1/system/activity`; пустое значение выключает вето;
- `ALPHA_MEDIA_TLS_CERT` (`-media-cert`) — сертификат сервера, по умолчанию `/var/lib/alpha-media/tls/server.crt`. Это каталог данных сервера (`StateDirectory=alpha-media`), где он при первом старте создаёт самоподписанный сертификат (SRV-004). Если в конфиге сервера задан свой `[tls] cert`, этот путь нужно поменять.

**Как проверяется сервер.** Сервер работает по HTTPS с самоподписанным сертификатом. Поэтому системные CA не используются, а сертификат **пиннится**. При каждом запросе агент читает файл сертификата (PEM или DER) и считает SHA-256 DER-сертификата. Сервер с другим сертификатом не принимается. Это тот же отпечаток `sha256:…`, что пиннят клиенты (TOFU). `InsecureSkipVerify` включён только вместе с этой проверкой: она заменяет проверку цепочки и имени.

Как читается ответ:

| Ситуация | Решение |
|---|---|
| `busy: true` или `sessions > 0` | занят — перезагружать нельзя; `reason` попадает в журнал и в админку |
| нет файла сертификата | сервер не установлен или ни разу не запускался. Агент пробует plain HTTP на том же адресе (сервер с `[tls] enabled = false`); если и там нет ответа — «не занят» |
| соединение отклонено (сервер остановлен) или `404` (сервер без этого адреса) | «не занят» — без сервера ничего не играет |
| сервер отвечает не TLS (на ClientHello пришёл HTTP 400 — TLS выключили, а старый сертификат остался) | повтор по plain HTTP на том же адресе |
| **отпечаток не совпал** | ошибка, перезагрузка откладывается. В журнал пишется `media server certificate pin mismatch` с ожидаемым отпечатком |
| другая ошибка TLS, тайм-аут (5 с), другой код ответа, неверный JSON | «не знаю» — перезагрузка откладывается |

Plain HTTP — только запасной путь для сервера без TLS: его определяют по отсутствию сертификата или по HTTP-ответу на TLS-рукопожатие. URL с `http://` в настройке выключает TLS и пиннинг совсем.

## API (unix-сокет)

Сокет `/run/alpha-updated/api.sock`, его создаёт `alpha-updated.socket`: владелец `root:alpha-admin`, права `0660`. Подключаться могут только root и члены группы `alpha-admin` (её получает `alpha-admin.service` через `SupplementaryGroups=`, группа задана в `/usr/lib/sysusers.d/alpha.conf`). Наружу API не открыт: веб-админка (DIST-013) проксирует его как `/api/system/update…` (05 §4.1). Каждый изменяющий запрос пишется в журнал с uid, gid и pid клиента (`SO_PEERCRED`).

| Метод и путь | Тело | Ответ |
|---|---|---|
| `GET /healthz` | — | `200 {"service":"alpha-updated","status":"ok"}` |
| `GET /v1/status` | — | полный статус: `booted`, `staged`, `rollback`, `tracking`, `update`, `settings`, `last_apply`, `next_window`, `bootc_error` |
| `GET /v1/update` | — | `update`: `state`, `current_version`, `available`, `pending_apply`, `last_check`, `operation`, `last_error` |
| `POST /v1/update/check` | — | `202 {"operation":…}` |
| `POST /v1/update/download` | — | `202` |
| `POST /v1/update/apply` | `{"when":"now"}` (по умолчанию) или `{"when":"maintenance"}` | `202 {"operation":…}` или `202 {"pending_apply":true}` |
| `DELETE /v1/update/apply` | — | отменить «в окно»: `200 {"pending_apply":false}` |
| `POST /v1/rollback` | `{"reboot": true}` | `202` |
| `GET /v1/settings` | — | настройки |
| `PUT /v1/settings` | любое подмножество `{"channel","auto","window"}` | `200 {"settings":…, "operation": …}` (операция `switch`, если сменился канал) |

Ошибки: `400 bad_request` (неверный JSON, лишние поля, неверные значения), `409 busy` (идёт другая операция), `409 conflict` (нечего ставить, некуда откатываться). Результат фоновой операции — в `update.operation` (пока идёт) и `update.last_error`.

```bash
S=/run/alpha-updated/api.sock
curl -s --unix-socket $S http://localhost/v1/status | jq
curl -s --unix-socket $S -X POST http://localhost/v1/update/check
curl -s --unix-socket $S -X PUT -d '{"auto":"download","window":"02:00-04:00"}' http://localhost/v1/settings
curl -s --unix-socket $S -X POST -d '{"when":"maintenance"}' http://localhost/v1/update/apply
```

## Снапшоты `/var` (DIST-010)

bootc не откатывает `/var`, поэтому перед каждой перезагрузкой в обновление агент делает read-only снапшот сабволюма `@var`. Если обновление не удержалось, `/var` возвращается к этому снапшоту ещё до монтирования. Решение — [ADR-0174](../../docs/adr/0174-var-snapshots.md).

- **Перед перезагрузкой** (после вето): `btrfs subvolume snapshot -r @var @var-snapshots/<UTC>` и маркер `@var-snapshots/pending.json` (checksum старого и нового деплоймента). Снапшот не удался — перезагрузки нет, ошибка видна в `update.last_error`. Хранятся 3 последних снапшота.
- **При каждой загрузке** `alpha-var-restore.service` (до `var.mount`) запускает `alpha-updated var-restore`:
  - загрузилась новая версия → в маркере отмечается `to_booted`;
  - снова старая, а новая уже работала (greenboot откатил) → `@var` уходит в `@var-snapshots/<UTC>-replaced` (read-only, данные не теряются), новый `@var` создаётся из снапшота, итог пишется в `last-restore.json`;
  - новая так и не загрузилась → маркер удаляется.
- **Подтверждение:** когда новая версия достигла `boot-complete.target` (прошли проверки greenboot), агент удаляет маркер. После этого ручной откат ОС `/var` не трогает: базу открывает старая версия (expand→contract, LIB-002).
- Верхний уровень btrfs монтируется во `/run/alpha-varsnap` только на время операции. Устройство берётся из `/sysroot`, сабволюм — из строки `/var` в `/etc/fstab`. Без разметки `@var` (dev-диск) обновление идёт без снапшота, с предупреждением в журнале.
- В статусе: `var_snapshots.{snapshots,pending,last_restore}` и `last_apply.var_snapshot`.

Посмотреть вручную:

```bash
dev=$(findmnt -no SOURCE /sysroot | sed 's/\[.*//'); mkdir -p /run/t && mount -o subvolid=5 "$dev" /run/t
ls /run/t/@var-snapshots; cat /run/t/@var-snapshots/*.json; umount /run/t
journalctl -b -u alpha-var-restore
```

## Журнал

Структурированный JSON (`log/slog`) в stderr → journald: `journalctl -u alpha-updated -o cat | jq`. Поля: `msg`, `op`, `auto`, `duration`, `digest`, `version`, `err`, для API — `method`, `path`, `uid`, `gid`, `pid`. Уровень — `-log-level`.

## Файлы и юниты

| Путь | Что |
|---|---|
| `/usr/libexec/alpha/alpha-updated` | статический бинарник (собирается в стадии `go-build` образа из этой папки) |
| `/usr/lib/systemd/system/alpha-updated.{service,socket}` | демон и его сокет; оба включены preset-ом |
| `/usr/lib/sysusers.d/alpha.conf` | группа `alpha-admin` |
| `/var/lib/alpha/updated/` | `settings.json`, `state.json` (`StateDirectory=`, 0700) |
| `/usr/lib/systemd/system/alpha-var-restore.service` | ранний откат `/var` к снапшоту (`alpha-updated var-restore`), включён preset-ом |
| `@var-snapshots/` (верхний уровень корневого btrfs) | снапшоты `@var`, `pending.json`, `last-restore.json` |

Флаги: `-socket`, `-socket-group` (если запускать без systemd), `-state-dir`, `-bootc`, `-version-file`, `-media-activity-url`, `-media-cert`, `-reboot-command`, `-check-interval`, `-tick`, `-start-delay`, `-log-level`, `-var-snapshots` (по умолчанию `true`), `-var-keep` (по умолчанию 3). Подкоманда `var-restore` — ранний шаг загрузки (см. выше).

## Разработка и тесты

```bash
cd distro/updated
gofmt -l . ; go vet ./... && go test -race ./...     # или: just distro::updated-test
```

Только стандартная библиотека Go, внешних модулей нет. Тесты не трогают настоящий bootc:

- `internal/fakebootc` — bootc в памяти: реестр, booted/staged/rollback, `--download-only` с блокировкой, перезагрузка, откат greenboot, ошибки по команде; `bootc status` отдаётся в настоящем JSON-формате;
- `internal/bootc` — разбор `bootc status --format=json` (пример в `testdata/`), аргументы команд, запуск настоящего процесса на shell-скрипте вместо bootc;
- `internal/agent` — проверка, загрузка, применение, откат, ошибки, «одна операция», смена канала, сохранение настроек и состояния, окно обслуживания, вето во время просмотра;
- `internal/api` — все маршруты через настоящий unix-сокет, коды ошибок, журнал с uid клиента;
- `internal/varsnap` — снапшот, маркер, откат при загрузке, хранение трёх, отказы (btrfs эмулируется каталогами), разбор fstab, mountinfo и `ostree=`;
- `internal/settings` — окна (в том числе через полночь), проверка значений, атомарная запись.

## Ручная проверка на VM

Пока не выполнена (см. PR DIST-009). Чек-лист — образ из этой ветки, VM из `distro/tests/vm-switch-rollback.sh` (`KEEP=1`), локальный реестр с v1 и v2:

1. `systemctl status alpha-updated.socket alpha-updated` — оба активны; `stat -c '%a %U %G' /run/alpha-updated/api.sock` → `660 root alpha-admin`; `systemctl is-enabled bootc-fetch-apply-updates.timer` → `masked`.
2. От обычного пользователя `curl --unix-socket …/healthz` — `Permission denied`; от root — `ok`.
3. Опубликовать v2, `POST /v1/update/check` → `available`; `POST /v1/update/download` → `downloaded`, `bootc status` показывает download-only. `systemctl reboot` — загрузилась **v1** (скачанное не применилось).
4. `POST /v1/update/apply {"when":"now"}` — перезагрузка в v2, `last_apply.result` = `booted`.
5. `POST /v1/rollback {"reboot":true}` — снова v1.
6. Окно: `PUT /v1/settings {"window":"<сейчас>-<через 10 мин>"}`, перезапустить сервис с `-start-delay=10s`; при заглушке медиасервера (404) — перезагрузка в окне. С медиасервером SRV-019 во время воспроизведения (или на паузе) перезагрузки нет, в журнале «update postponed». Подменить `/var/lib/alpha-media/tls/server.crt` чужим сертификатом — в журнале `pin mismatch`, перезагрузки нет.
7. `PUT /v1/settings {"channel":"beta"}` при опубликованном теге `:beta` — `bootc status` показывает staged с `:beta`.
8. `journalctl -u alpha-updated -o cat` — JSON-строки с `op`, `uid`.
