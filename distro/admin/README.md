<!-- SPDX-License-Identifier: MPL-2.0 -->
# distro/admin — `alpha-admin`, бэкенд админки NAS-ОС (Go)

Один REST/JSON API для веб-админки (DIST-014) и вкладки «Сервер» в клиентском приложении: вход пользователями ОС, состояние и действия над системой через её штатные D-Bus API (systemd, NetworkManager, firewalld, UDisks2, logind), прокси к агенту обновлений `alpha-updated`. Исследование — [05 §5.1](../../docs/research/05-nas-linux-distro.md), решения — [ADR-NNNN](../../docs/adr/NNNN-alpha-admin-backend.md), фаза 0 — [ADR-0010](../../docs/adr/0010-nas-os-phase0.md).

API описан в [`internal/server/openapi.yaml`](internal/server/openapi.yaml); работающий сервис отдаёт его по `GET /api/v1/openapi.yaml`.

## Как устроено

```
браузер / приложение ──HTTPS :443 (только LAN)──► alpha-admin (пользователь alpha-admin, без root)
                                                   │  ├─ D-Bus (system bus) ─► systemd, NetworkManager, firewalld, UDisks2, logind
                                                   │  │     изменения разрешает polkit-правило 50-alpha-admin.rules
                                                   │  ├─ journalctl (группа systemd-journal) ─► журнал наших сервисов
                                                   │  ├─ unix /run/alpha-updated/api.sock (группа alpha-admin) ─► alpha-updated
                                                   │  ├─ unix /run/alpha-admin/auth.sock (0600, только alpha-admin)
                                                   │  │         └─► alpha-admin-auth@.service (root, PAM, один процесс на проверку)
                                                   │  └─ unix /run/alpha-admin/priv.sock (0600, только alpha-admin)
                                                   │            └─► alpha-admin-priv@.service (root: пользователи, Samba, SSH)
                     HTTP :80 ─► 301 на HTTPS, /health для проверок
```

- **Демон без root.** `alpha-admin.service` работает от системного пользователя `alpha-admin` с жёсткой песочницей (`ProtectSystem=strict`, `NoNewPrivileges`, фильтр системных вызовов, только `CAP_NET_BIND_SERVICE`). Всё, что меняет систему, — вызовы D-Bus, которые polkit разрешает **только этому пользователю и только нужные действия** (перезапуск наших сервисов, переключение подключений и Wi-Fi, открытие наших портов, монтирование, перезагрузка и выключение). Кто может войти, решает сам демон: только администраторы.
- **Пароли проверяет отдельный помощник.** `alpha-admin-auth` запускается systemd на каждое подключение к `/run/alpha-admin/auth.sock` (`Accept=yes`), проверяет один пароль через PAM (сервис `/etc/pam.d/alpha-admin`: `password-auth`, плюс проверки учётной записи — заблокированные и просроченные не войдут) и завершается. Сокет принадлежит `alpha-admin` с правами 0600, помощник дополнительно сверяет `SO_PEERCRED`. Помощник не говорит, почему вход не удался: неизвестный пользователь, неверный пароль, заблокированная и системная учётная запись выглядят одинаково.
- **Пользователи, Samba и SSH — через второй помощник** (DIST-018, ADR-NNNN). У них нет D-Bus API с polkit, поэтому `alpha-admin-priv` (root, тоже по подключению к `/run/alpha-admin/priv.sock`, 0600, `SO_PEERCRED`) выполняет только фиксированные операции из `internal/privhelper`: список пользователей; создать, сменить пароль, сделать администратором, удалить; синхронизировать пароль Samba; прочитать и записать настройки общих папок; включить и выключить SSH. Команд, путей и текста конфига он не принимает: `smb.conf.d/kinolune.conf` рендерит сам из булевых настроек (`internal/samba`), команды запускает без shell, пароли передаёт только через stdin. Изменения пользователей требуют **пароль действующего администратора** в запросе (`admin_password`, как `sudo`): помощник сам проверяет его через PAM и членство в `wheel`/`alpha-admin`; неверный пароль считается в блокировке входа. Нельзя удалить себя, снять права с себя и с последнего администратора.
- **Только администраторы** (решение мейнтейнера): войти могут члены групп `wheel` или `alpha-admin` (05 §5.2: админ = wheel). Остальные пользователи с верным паролем получают `403 not_admin` «Нет прав администратора» — без сессии и без счёта в блокировке. `root` и системные учётные записи (UID < 1000) не входят никогда. Отдельной роли «член семьи» пока нет.

## Веб-интерфейс

`/` отдаёт веб-админку (DIST-014, [ADR-NNNN](../../docs/adr/NNNN-admin-web-ui.md)): статические HTML, CSS и ES-модули из [`internal/webui/static`](internal/webui/static), встроенные в бинарник (`go:embed`), без сборки и CDN, под строгий CSP `default-src 'self'`. Страницы: вход, обзор, обновления (проверить, скачать, установить сейчас или в окне, откатить, канал и авто-режим), сеть и доступ из LAN, пользователи, общие папки и SSH (если API их перечисляет, DIST-018), диски, службы и питание, «Расширенный режим (Cockpit)». Данные — только из этого же API. Цвета — семантические токены `tokens.css` (копия `design/tokens/generated/web/tokens.css`, палитра `bordeaux-graphite`); строки — `static/i18n/{ru,en,uk}.json`. После `just design tokens` скопируйте `tokens.css` заново — иначе упадёт `TestTokensUpToDate`.

## Сессии и защита

| Что | Как |
|---|---|
| Cookie | `__Host-alpha_admin`: `Secure`, `HttpOnly`, `SameSite=Strict`, `Path=/`, без `Domain`; в памяти хранится только SHA-256 токена |
| Время жизни | 30 минут без действий, не больше 12 часов; до 10 сессий на пользователя. Перезапуск демона (и обновление ОС) завершает все сессии |
| CSRF | на каждый `POST/PUT/DELETE` — заголовок `X-CSRF-Token` (выдаётся при входе и в `GET /api/v1/session`); заголовок `Origin`/`Referer`, если есть, должен совпадать с хостом. Вход тоже проверяет `Origin` и принимает только `application/json` |
| Перебор паролей | 10 попыток в минуту с адреса; 5 неверных паролей к одной учётной записи с одного адреса — блокировка пары на 5 минут (дальше 10, 20… до часа); 20 неверных с адреса по любым учётным записям — адрес на 15 минут. Ответ `429 locked` с `Retry-After`. Блокируется пара «учётная запись + адрес», а не вся учётная запись: чужое устройство в сети не может запереть владельца |
| Только LAN | соединения с публичных адресов закрываются до TLS-рукопожатия. «LAN» — те же диапазоны, что у медиасервера (`pa-vfs-http` Zone): loopback, 10/8, 172.16/12, 192.168/16, 100.64/10 (Tailscale), `fc00::/7`, `fec0::/10`, плюс link-local (ноутбук, подключённый к NAS кабелем напрямую). То же продублировано в юните (`IPAddressAllow=`/`IPAddressDeny=any`) |
| TLS | самоподписанный ECDSA P-256 на 10 лет, создаётся при первом запуске в `/var/lib/alpha/admin/tls-{cert,key}.pem` (ключ 0600) и переживает перезагрузки и обновления ОС; имена `<hostname>`, `<hostname>.local`, `localhost`, 127.0.0.1, ::1. Отпечаток SHA-256 — в `GET /api/v1/info` для пиннинга в приложении (как у медиасервера, SRV-004). Чтобы выпустить новый, удалить оба файла и перезапустить сервис |
| Перезагрузка и выключение | в два шага: первый `POST /api/v1/power/reboot` отвечает `409 confirmation_required` с одноразовым токеном (60 с, привязан к сессии и действию) и списком «что сейчас мешает» (ингибиторы logind: идёт фильм, обновление); повтор с `{"confirm": "<токен>"}` выполняет |
| Журнал действий | каждый вход, отказ, блокировка и изменяющее действие пишется в журнал с пользователем и адресом: `journalctl -u alpha-admin -o cat \| jq` |

## API кратко

| Метод и путь | Вход | Что |
|---|---|---|
| `GET /healthz`, `GET /api/v1/info`, `GET /api/v1/openapi.yaml` | — | живость, версия ОС и отпечаток TLS, описание API |
| `POST/GET/DELETE /api/v1/session` | — / да | вход, текущая сессия (и CSRF-токен), выход |
| `GET /api/v1/system` | да | имя, версия ОС, ядро, uptime, нагрузка, память, данные установки (`/var/lib/alpha/install.json`, DIST-005) |
| `GET /api/v1/services[/{unit}]` | да | состояние наших сервисов (systemd) |
| `POST /api/v1/services/{unit}/restart` | да | перезапуск (медиасервер, админка, обновления, SMB, wsdd, Avahi, smartd, chrony) |
| `GET /api/v1/services/{unit}/journal?lines=N` | да | последние строки журнала сервиса |
| `GET /api/v1/network`, `GET /api/v1/network/connections` | да | состояние NetworkManager, интерфейсы и адреса; сохранённые подключения (без секретов) |
| `POST /api/v1/network/connections/{uuid}/up\|down`, `PUT /api/v1/network/wireless` | да | включить/выключить подключение, радио Wi-Fi |
| `GET /api/v1/firewall`, `PUT /api/v1/firewall/openings/{name}` | да | зоны firewalld; открыть или закрыть наше: `media` (8487/tcp), `smb` (445/tcp; старый сервис `samba` считается открытым и убирается при закрытии), `discovery` (mDNS, WS-Discovery), `cockpit`, `ssh` — сразу и после перезагрузки. http/https админки из админки не закрываются |
| `GET /api/v1/users` | да | люди (UID ≥ 1000): администратор, пользователь NAS, есть ли пароль Samba |
| `POST /api/v1/users`, `PUT /api/v1/users/{name}/password\|admin`, `DELETE /api/v1/users/{name}` | да + `admin_password` | создать пользователя NAS (группа `alpha-users`, пароль ОС и Samba, папки в `Users` и `TimeMachine`), сменить пароль, дать или снять права администратора, удалить (папки сохраняются как `deleted-<имя>-<время>` для `root:wheel`) |
| `GET\|PUT /api/v1/sharing` | да | общие папки Samba: `media_guest_read` (чтение `Media` без пароля, по умолчанию выключено), `time_machine`; конфиг проверяется `testparm` и перечитывается |
| `GET\|PUT /api/v1/ssh` | да | SSH (по умолчанию выключен, только ключи): включить или выключить `sshd` вместе с открытием `ssh` в firewall |
| `GET /api/v1/disks`, `POST /api/v1/disks/{dev}/mount\|unmount` | да | диски, разделы, сводка SMART (ATA и NVMe: статус, температура, часы, плохие сектора); монтирование файловых систем с данными. Файловые системы самой ОС (из `install.json` и смонтированные в `/`, `/sysroot`, `/boot`, `/var`…) не трогаются; отмонтировать можно только то, что под `/var/srv`, `/srv`, `/run/media`, `/media` |
| `GET /api/v1/power`, `POST /api/v1/power/{reboot\|poweroff}` | да | ингибиторы; перезагрузка и выключение с подтверждением |
| `/api/v1/system/update…` | да | прокси к `alpha-updated` (DIST-009): `GET ""` → `/v1/update`, `GET /status`, `POST /check`, `POST /download`, `POST\|DELETE /apply`, `POST /rollback`, `GET\|PUT /settings`. Пробрасывается только эта таблица, тела и коды ответов — как у агента; если сокет не отвечает — `502 update_agent_unavailable` |

Успешный вход любого пользователя NAS (и администратора, и того, кому отвечено «Нет прав администратора») заодно ставит тот же пароль для общих папок: так пароль, сменённый в консоли через `passwd`, доходит до Samba.

Ошибки: `{"error": "<код>", "message": "<текст>"}`, коды перечислены в OpenAPI.

```bash
H=https://kinolune.local
curl -sk -c jar -H 'Content-Type: application/json' -d '{"username":"anna","password":"…"}' $H/api/v1/session
CSRF=$(curl -sk -b jar $H/api/v1/session | jq -r .csrf_token)
curl -sk -b jar $H/api/v1/services | jq '.services[] | {name, active_state}'
curl -sk -b jar -H "X-CSRF-Token: $CSRF" -X POST $H/api/v1/services/smb.service/restart
curl -sk -b jar -H "X-CSRF-Token: $CSRF" -X POST $H/api/v1/power/reboot        # → confirm_token
```

## Файлы в образе

| Путь | Что |
|---|---|
| `/usr/libexec/alpha/alpha-admin` | демон (статический, `CGO_ENABLED=0`) |
| `/usr/libexec/alpha/alpha-admin-auth` | помощник проверки пароля (cgo, libpam), собирается с `-tags pam` |
| `/usr/lib/systemd/system/alpha-admin.service` | демон, песочница, `IPAddressAllow` только LAN |
| `/usr/lib/systemd/system/alpha-admin-auth.socket`, `alpha-admin-auth@.service` | сокет помощника (`Accept=yes`, 0600 `alpha-admin`) и его экземпляр |
| `/usr/libexec/alpha/alpha-admin-priv` | помощник пользователей, Samba и SSH (cgo, libpam, `-tags pam`); `render-samba` — рендер конфига при загрузке |
| `/usr/lib/systemd/system/alpha-admin-priv.socket`, `alpha-admin-priv@.service` | его сокет (`Accept=yes`, 0600 `alpha-admin`) и экземпляр |
| `/usr/lib/systemd/system/alpha-samba-config.service` | до `smb.service`: рендер `smb.conf.d/kinolune.conf`, импорт пароля Samba из установщика, папки пользователей |
| `/usr/lib/systemd/system/alpha-ssh-firstboot.service`, `/usr/libexec/alpha/alpha-ssh-firstboot` | один раз: SSH включается только на свежей установке с ключами; на обновлённой системе состояние SSH не меняется |
| `/var/lib/alpha/samba/settings.json` | настройки общих папок (без секретов) |
| `/usr/lib/sysusers.d/alpha-admin.conf` | системный пользователь и группа `alpha-admin` |
| `/usr/share/polkit-1/rules.d/50-alpha-admin.rules` | что пользователю `alpha-admin` можно делать через D-Bus |
| `/etc/pam.d/alpha-admin` | PAM-сервис проверки пароля |
| `/var/lib/alpha/admin/` | сертификат и ключ TLS (`StateDirectory=`, 0700) |

В образ добавлены пакеты `udisks2` и `polkit`. Бинарники собираются в стадии `admin-build` `distro/Containerfile` компилятором Go из той же закреплённой базы Fedora; модули Go (`godbus/dbus` — BSD-2-Clause, `msteinert/pam` — BSD-2-Clause, `golang.org/x/sys` — BSD-3-Clause) скачиваются через module proxy и сверяются с `go.sum`.

Флаги демона: `-listen` (`:443`), `-http-listen` (`:80`, пусто — выключить), `-state-dir`, `-auth-socket`, `-updated-socket`, `-install-info`, `-version-file`, `-admin-groups` (`wheel,alpha-admin`), `-allow-public`, `-journalctl`, `-log-level`.

**Открыть админку не только для LAN** (не рекомендуется; лучше Tailscale — его адреса 100.64/10 и так считаются локальными): drop-in `systemctl edit alpha-admin` с

```ini
[Service]
IPAddressAllow=
IPAddressDeny=
ExecStart=
ExecStart=/usr/libexec/alpha/alpha-admin -allow-public
```

## Разработка и тесты

```bash
cd distro/admin
gofmt -l . ; go vet ./... && go test -race ./...    # или: just distro::admin-test
go vet -tags pam ./...                              # нужен pam-devel и cgo
```

- `internal/system/fakebus` поднимает **приватную шину** (`dbus-daemon` с собственным конфигом во временном каталоге) и на ней — фейковые systemd, logind, NetworkManager, firewalld и UDisks2 с теми же именами, путями, интерфейсами и сигнатурами, что у настоящих. Клиенты `internal/system` и весь HTTP-слой тестируются через настоящий D-Bus, включая отказ polkit (`AccessDenied`). Без `dbus-daemon` эти тесты пропускаются; `ALPHA_ADMIN_REQUIRE_DBUS=1` (CI) делает пропуск ошибкой.
- `internal/server` — вход только администраторов («Нет прав администратора» для остальных), cookie, CSRF и `Origin`, блокировка и `Retry-After`, все маршруты, подтверждение перезагрузки, прокси к фейковому `alpha-updated` на unix-сокете (включая недоступный сокет), покрытие OpenAPI.
- `internal/auth` — сессии (простой, максимум, лимит на пользователя), блокировки и лимит попыток на подменённых часах.
- `internal/authhelper`, `cmd/alpha-admin-auth` — протокол помощника, отказ в чужом `SO_PEERCRED`, тайм-аут; `TestRealPAM` — **настоящий PAM** с тестовыми учётными записями.
- `internal/netpolicy`, `internal/tlscert` — классификация адресов, отбрасывание публичных клиентов, сертификат переживает перезапуск.
- `internal/privhelper` — операции помощника на подменённых командах и PAM: аргументы команд (пароли только в stdin), повторная проверка пароля администратора, защита последнего администратора и себя, удаление с сохранением папок, откат настроек, если `testparm` отказал, импорт пароля из установщика, протокол по настоящему unix-сокету. `internal/samba` — рендер конфига; golden-тест сверяет файл образа `rootfs/etc/samba/smb.conf.d/kinolune.conf` с `Render` (`go test ./internal/samba -update`).
- [`tests/samba-container.sh`](tests/samba-container.sh) — настоящие `alpha-admin-priv` (с PAM), `useradd`/`smbpasswd`/`pdbedit` и `smbd` в контейнере Fedora, проверки `smbclient` ([`tests/samba_check.py`](tests/samba_check.py)): импорт пароля установщика, анонимный доступ запрещён, папки и права, создание и удаление пользователей, синхронизация пароля, чтение `Media` для гостя.
- [`tests/pam-container.sh`](tests/pam-container.sh) — всё сразу в одноразовом контейнере Fedora: учётные записи (wheel, обычная, заблокированная, системная), наш PAM-файл, `TestRealPAM`, сборка как в образе, `go test -race`.
- [`tests/vm-admin-e2e.sh`](tests/vm-admin-e2e.sh) — сквозной тест в QEMU: образ из этой ветки, установка на диск, второй пустой диск; по SSH проверяются юниты и права, затем [`tests/vm_api_check.py`](tests/vm_api_check.py) ходит в настоящий API: вход PAM администратора, отказ «Нет прав администратора» обычному пользователю, cookie, CSRF, блокировка, перезапуск smb, журнал, Wi-Fi, firewalld (runtime и permanent), монтирование и отмонтирование btrfs на втором диске, отказ трогать системную ФС, прокси к фейковому `alpha-updated` на его сокете, LAN-фильтр (адрес 192.168/16 обслуживается, 203.0.113.0/24 — нет), перезагрузка с подтверждением, тот же сертификат после перезагрузки. DIST-018 ([`tests/vm_samba_check.py`](tests/vm_samba_check.py) и `smbclient` снаружи VM через проброс порта QEMU и firewall VM): SSH включён при первой загрузке, потому что ключи переданы, только ключи; firewall без NetBIOS и SSH-пароля; конфиг отрисован при загрузке, метки SELinux; пользователь, созданный через API, пишет в свою папку и `Media`, чужую не читает; анонимный доступ запрещён, затем чтение `Media` для гостя; закрытие открытия `smb` закрывает SMB снаружи; SSH выключается и включается через API; настройки и закрытый порт переживают перезагрузку. `QEMU_EXEC="docker exec <контейнер>"` запускает QEMU во вспомогательном контейнере.
- [`tests/vm-ui-e2e.sh`](tests/vm-ui-e2e.sh) — веб-интерфейс в QEMU: две версии образа в локальном реестре, установлена первая; Chromium (Playwright, [`tests/ui_update_rollback.py`](tests/ui_update_rollback.py)) входит, проверяет обновление, ставит его, ждёт перезагрузку, затем откатывает; потом скрипт по журналу в VM сверяет, какая версия работала в каждой из трёх загрузок. Снимки экрана — `distro/.work/ui-e2e/`. Playwright берётся из контейнера `mcr.microsoft.com/playwright/python` (или `PLAYWRIGHT_RUN=python3`).
