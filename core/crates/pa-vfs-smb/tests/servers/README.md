# Контракт-тесты SMB против Samba

Общий контракт-тест-сьют `pa_vfs::contract` (ADR-0013) против Samba в Docker
(`alpine:3.22`, Samba 4.21), три контейнера:

| Сервис | Порт | Что проверяется |
|---|---|---|
| `samba` | `127.0.0.1:18445` | шара `media`: SMB 3.1.1 (по умолчанию), принудительно SMB 2.0.2, чтение напрямую и через read-ahead; шара `sealed` с `smb encrypt = required` (шифрование SMB3) |
| `samba-signed` | `127.0.0.1:18446` | `server signing = mandatory`, клиент требует подпись |
| `samba-netem` | `127.0.0.1:18447` | профиль «30 мс, 1 % потерь» из NET-003 (в `bench.sh` — `lossy`): `tc netem delay 30ms loss 1%` на исходящем трафике сервера, поток 192 МиБ |

Запуск (нужны Docker с плагином compose и cargo):

```bash
core/crates/pa-vfs-smb/tests/servers/run.sh              # всё
core/crates/pa-vfs-smb/tests/servers/run.sh smb3         # фильтр cargo test
```

Скрипт берёт libsmb2 из `$PA_SMB2_LIB` или `tools/libsmb2/out/linux-x86_64` (собирает её в
Docker, если нет), кладёт фикстуру в `/media/контракт` рядом с посторонним файлом и
случайный `stream/stream.bin` (его SHA-256 передаётся тесту), поднимает контейнеры, ждёт,
пока `smbd` начнёт слушать, и запускает
`cargo test -p pa-vfs-smb --features server-tests --test contract_servers`. Логин и пароль —
одноразовые для этих контейнеров; анонимный вход и гости запрещены.

Кроме контракта: отказ анонимному входу и неверному паролю (`AuthenticationFailed`, пароль не
попадает в текст ошибки), несуществующая шара (`NotFound`), шифрованная шара без шифрования
(отказ), папка внутри шары как корень источника, согласованный диалект, потоковое чтение с
проверкой SHA-256 (в LAN и на netem-профиле).

Тест `stream_wifi_netem_80_mbit` проверяет цель NET-003/NET-021 — 80 Мбит/с на профиле
«30 мс, 1 % потерь» с настройками по умолчанию (число параллельных блоков подбирается само,
ADR-0033). Подбор параметров вручную: `PA_SMB_PREFETCH=<блоков>` (фиксированное число, без
автоподбора) и `PA_SMB_CHUNK=<байт>`.

## Замер скорости по профилям сети (NET-021)

`bench.sh` поднимает один одноразовый контейнер Samba без проброса портов (клиент ходит на его
адрес в docker-сети), меняет netem на `eth0` внутри контейнера и для каждого профиля несколько раз
запускает `stream_bench` (192 МиБ подряд чтениями по 256 КиБ, release-сборка, проверка SHA-256).
Интерфейсы хоста не трогаются; контейнер и данные удаляются при выходе.

```bash
core/crates/pa-vfs-smb/tests/servers/bench.sh                  # lan gige wifi lossy, по 3 раза
PA_SMB_BENCH_MIN_MBIT_LOSSY=80 core/crates/pa-vfs-smb/tests/servers/bench.sh lossy
PA_SMB_PREFETCH=4 core/crates/pa-vfs-smb/tests/servers/bench.sh  # как до NET-021
```

| Профиль | netem | Что моделирует |
|---|---|---|
| `lan` | — | мост docker: упор в процессор клиента |
| `gige` | `delay 500us rate 1gbit` | проводная сеть 1 Гбит/с |
| `wifi` | `delay 30ms 5ms distribution normal loss 0.1% rate 300mbit` | Wi-Fi, где канальный уровень (повторы 802.11) исправляет почти все потери, до TCP доходит 0,1 % |
| `lossy` | `delay 30ms loss 1%` | профиль NET-003: каждая потеря доходит до TCP |

Замер на настоящем Wi-Fi — пример `smb_speed` (строки SRC-121…124 каталога проверок).

Без фичи `server-tests` эти тесты не компилируются; в обычном `cargo test` идут юнит-тесты и
`tests/without_server.rs` (с `PA_SMB2_LIB` — ещё проверка недоступного сервера). Подключение к
CI — задача CI-012.
