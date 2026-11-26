# distro/ — NAS-ОС project_alpha (прототип, фаза 0)

Неизменяемая (image-based) ОС для NAS и медиасервера на базе bootc. Вся ОС описана одним `Containerfile` и поставляется как OCI-образ; обновления атомарные, с откатом (`bootc upgrade` / `bootc rollback`). Решения фазы 0 — в [ADR-0010](../docs/adr/0010-nas-os-phase0.md), исследование — [05](../docs/research/05-nas-linux-distro.md).

> Основано на Fedora Linux. Имя ОС ещё не выбрано (DEC-004); рабочее имя образа — `alpha-os`.

## Структура

| Путь | Что |
|---|---|
| `Containerfile` | образ ОС: `FROM ghcr.io/denishumen/fedora-bootc:45@sha256:…` (пин по digest из нашего зеркала, см. ниже), пакеты, сервисы, firewall, `bootc container lint` |
| `packages.txt` | пакеты поверх базы (Samba, wsdd, Avahi, firewalld, smartmontools, mdadm, btrfs-progs, greenboot, Cockpit) |
| `rootfs/` | файлы, копируемые в образ: юниты наших сервисов, preset, `tmpfiles.d`, `smb.conf` и сгенерированный по умолчанию `smb.conf.d/kinolune.conf`, `sshd_config.d`, `file_contexts.local` |
| `scripts/build-image.sh` | сборка образа (`podman`, если есть, иначе `docker`) |
| `scripts/make-disk.sh` | загрузочный raw-диск через `bootc install to-disk --via-loopback` в привилегированном контейнере |
| `scripts/ci-build.sh`, `ci-sbom.sh`, `sign-image.sh` | сборка в CI (build → rechunk → lint), SBOM (syft), подпись cosign |
| `keys/` | публичный ключ cosign `alpha-os.pub` (кладёт владелец, см. ниже) |
| `tests/signature-policy.sh` | DIST-002: устройство отклоняет неподписанный образ |
| `tests/branding.sh` | DIST-003: брендинг Kinolune OS в собранном образе (os-release, нет бренда Fedora, логотип в `fastfetch`) |
| `branding/` | генератор логотипа для `fastfetch` из знака Kinolune и синхронизация ассетов бренда в `rootfs/` |
| `scripts/run-vm.sh` | запуск диска в QEMU/KVM (2 vCPU, 3 GiB, SSH на `127.0.0.1:22522`, `VM_UEFI=1` для OVMF) |
| `admin/` | `alpha-admin` (Go): бэкенд админки — вход пользователями ОС, REST поверх D-Bus, прокси к `alpha-updated`, самоподписанный TLS (DIST-013), см. [admin/README.md](admin/README.md) |
| `installer/` | `alpha-installer` (Go): пока только обнаружение дисков (DIST-004), см. [installer/README.md](installer/README.md) |
| `updated/` | `alpha-updated` (Go): агент обновлений — check, download-only, apply, rollback через bootc, API на unix-сокете, окно обслуживания (DIST-009), см. [updated/README.md](updated/README.md) |
| `installer/` | `alpha-installer` (Go): обнаружение дисков (DIST-004), установка по файлу ответов (DIST-005), полноэкранный установщик `alpha-installer tui` на Bubble Tea (DIST-006), см. [installer/README.md](installer/README.md) |
| `installer/Containerfile`, `installer/live/` | DIST-007: live-окружение установщика для ISO (TUI DIST-006 на tty1 и последовательной консоли, текстовое меню — запасной путь, `iso.yaml` для image-builder, dracut `dmsquash-live`) |
| `scripts/build-iso.sh` | DIST-007: ISO установщика — образ ОС + live-образ → `image-builder … generic-iso` с офлайн-payload, проверка размера < 1.9 GiB |
| `tests/iso-boot.sh` | DIST-007: загрузка ISO в QEMU/OVMF до установщика (TUI, затем текстовое меню), проверки на последовательной консоли, пробная установка через все экраны TUI |
| `tests/build-iso-helper.sh` | DIST-007: `build-iso.sh` на фейковом движке контейнеров — helper, оставленный `KEEP=1`, переиспользуется только с теми же монтированиями, с другим `OUT_DIR` пересоздаётся (секунды, без контейнеров) |
| `tests/vm-switch-rollback.sh` | сценарий фазы 0: switch → rollback → upgrade → rollback → digest-пин и возврат на тег |
| `tests/vm-install.sh` | DIST-005: установка `alpha-installer` по файлу ответов на виртуальный NVMe (UEFI), загрузка установленной системы, проверки по SSH, `bootc upgrade --check` против реестра |
| `tests/wifi-hwsim.sh`, `tests/wifi-hwsim-vm.sh` | DIST-032: Wi-Fi приложения (клиент NetworkManager, `tool/nm_probe.dart`) в VM Fedora Cloud с `mac80211_hwsim` — точки WPA2 и открытая, пользователь группы `kinolune` без root |
| `.work/` | локальные артефакты (диски, ключи SSH, логи); в git не попадает |

**Зеркало базового образа.** quay.io периодически удаляет старые digest-ы `fedora-bootc`, и сборка по закреплённому digest-у падает с «manifest unknown». Поэтому `Containerfile` и `installer/Containerfile` берут базу из зеркала `ghcr.io/denishumen/fedora-bootc` (публичный пакет). Workflow [`base-image-mirror.yml`](../.github/workflows/base-image-mirror.yml) копирует туда закреплённые digest-ы и текущий `:45` с теми же digest-ами (`skopeo copy --all --preserve-digests`) раз в неделю, при смене пина на main и вручную. Сменить базу: взять digest текущего `quay.io/fedora/fedora-bootc:45`, запустить `base-image-mirror` (или он уже скопировал его по расписанию), вписать digest в оба `Containerfile`.

## Что в образе

- **`alpha-updated` — настоящий агент обновлений** (Go, DIST-009): бинарник `/usr/libexec/alpha/alpha-updated` собирается в стадии `go-build` `Containerfile`, API — только на unix-сокете `/run/alpha-updated/api.sock` (`root:alpha-admin 0660`, `alpha-updated.socket`), состояние — `/var/lib/alpha/updated`. Подробно — [updated/README.md](updated/README.md).
- **`alpha-media` пока заглушка** (`/usr/libexec/alpha/alpha-stub`, Python): `alpha-media` (:8487). Отвечает `GET /health` и `/healthz` JSON-ом с версией образа. Юниты с `DynamicUser=yes` и `ProtectSystem=strict`.
- **`alpha-admin`** (DIST-013) — настоящий бэкенд админки: HTTPS на :443 с самоподписанным сертификатом, :80 перенаправляет и отвечает `/health`; только LAN; работает от пользователя `alpha-admin`, права на D-Bus — через polkit-правило, пароли проверяет PAM-помощник `alpha-admin-auth`. Подробно — [admin/README.md](admin/README.md).
- **Samba** (DIST-018, ADR-NNNN): SMB3 только на 445 (NetBIOS выключен), `vfs_fruit` для macOS, `server string = Kinolune OS (%h)`. Статический `/etc/samba/smb.conf` подключает `/etc/samba/smb.conf.d/kinolune.conf`, который `alpha-samba-config.service` рендерит при каждой загрузке из настроек админки (`/var/lib/alpha/samba/settings.json`). Шары: `Media` → `/var/srv/media` (пользователи NAS читают и пишут), `Users` → `/var/srv/users/<имя>` (каждый видит только свою папку), `TimeMachine` → `/var/srv/timemachine/<имя>` (резервные копии Mac). **Анонимного доступа по умолчанию нет**; чтение `Media` без пароля (для телевизоров) включается в админке, записи для гостя нет никогда. Пользователи NAS — люди в группах `alpha-users`, `wheel`, `alpha-admin`; пароль Samba = пароль пользователя (ставится при создании и смене пароля в админке, при входе в админку, из установщика). Типы SELinux шар — `samba_share_t` (`file_contexts.local`). **wsdd** (Windows), **Avahi** + nss-mdns (`<hostname>.local`, Samba сама анонсирует `_smb._tcp` и `_adisk._tcp`).
- **firewalld**: зона по умолчанию `home`; открыты http, https, 445/tcp, 8487/tcp, mdns, ws-discovery и стандартный dhcpv6-client. `ssh`, `samba-client` и NetBIOS 137–139 закрыты (DIST-018).
- **SSH выключен по умолчанию** и принимает только ключи (`/etc/ssh/sshd_config.d/40-kinolune.conf`). Только на свежей установке с ключами (метка установщика `/var/lib/alpha/ssh-keys-at-install` или `bootc install --root-ssh-authorized-keys` без установщика) `alpha-ssh-firstboot.service` один раз включает `sshd` и открывает порт; на обновлённой системе состояние SSH не меняется. Дальше — `PUT /api/v1/ssh` в админке.
- **NetworkManager** с Wi-Fi (`NetworkManager-wifi`, `wpa_supplicant`): группа `kinolune` (`sysusers.d`) по polkit-правилу `50-kinolune-network.rules` подключает сети, сканирует, включает и выключает Wi-Fi, сохраняет и забывает профили без root — для панели «Сеть» на главной Kinolune (DIST-032, ADR-0167).
- **smartd**, **chronyd**, **greenboot** (проверки появятся в DIST-011), **Cockpit** установлен, `cockpit.socket` выключен.
- `bootc-fetch-apply-updates.timer` и его `.service` **замаскированы**: обновлениями управляет `alpha-updated` (DIST-009) — скачивает в фоне, ставит в окне обслуживания и не перезагружает NAS во время просмотра.
- Версия образа — `/usr/share/alpha/version` (`ALPHA_OS_VERSION`, build-arg).

## Брендинг Kinolune OS (DIST-003)

Имя ОС — **Kinolune OS** (DEC-004, ADR-0011). По правилам товарного знака Fedora изменённую сборку нельзя называть Fedora и показывать её логотипы, поэтому везде, где пользователь видит имя системы, — Kinolune OS, а Fedora упоминается только фактической ссылкой «based on Fedora Linux» (motd; в `/usr/lib/os-release` комментариев нет: image-builder их не принимает, ADR «live-ISO» п. 9). Всё делает `/usr/libexec/alpha/alpha-branding` при сборке образа:

| Где | Что |
|---|---|
| `/usr/lib/os-release` (`/etc/os-release`) | `NAME="Kinolune OS"`, `ID=kinolune-os`, `ID_LIKE=fedora`, `VERSION`/`IMAGE_VERSION` = версия образа, `PRETTY_NAME`, `VARIANT_ID=nas`, `LOGO=kinolune-logo-icon`, `ANSI_COLOR` (lune.400), `DEFAULT_HOSTNAME=kinolune`, ссылки на наш GitHub |
| `/etc/system-release` | → `/usr/lib/kinolune-release` («Kinolune OS release …») |
| `/etc/issue`, `/etc/issue.net` | `\S` (agetty подставляет `PRETTY_NAME`), версия Kinolune OS |
| `/usr/lib/motd.d/10-kinolune-os` | «Kinolune OS … — based on Fedora Linux 45» после входа по SSH |
| загрузчик | заголовки BLS ostree берутся из `PRETTY_NAME` («Kinolune OS … (ostree:0)»); в статическом `grub.cfg` bootupd бренда Fedora нет |
| иконки | `kinolune-logo-icon` в `hicolor` (копии `design/brand/export`, `branding/sync-assets.sh`) |
| `fastfetch` | `/etc/xdg/fastfetch/config.jsonc` для всех пользователей: логотип-знак Kinolune (`/usr/share/kinolune-os/fastfetch-logo.txt`, цвета бренда) и «OS: Kinolune OS …» |

Что остаётся «fedora» сознательно — это технические идентификаторы, по которым инструменты узнают семейство и релиз:

- `ID_LIKE=fedora` и `VERSION_ID=45` (номер релиза базы): их читают dnf, bootc, image-builder и скрипты; `rpm -E %fedora` и `releasever` dnf остаются 45 — тест это проверяет;
- `/etc/fedora-release` и `/etc/redhat-release` → `fedora-release`: файлы, по имени которых семейство определяют python `distro`, факты ansible и сторонние установщики;
- имена пакетов, репозитории `/etc/yum.repos.d`, ключи, каталог `EFI/fedora` (там подписанный shim).

Чего нет в базе и поэтому не нужно менять: `fedora-logos` (тест падает, если пакет появится, — тогда `dnf swap fedora-logos generic-logos`), plymouth (headless NAS, заставки нет; своя тема — вместе с графической сессией, DIST-028), `/etc/default/grub` и `grub2-mkconfig` (bootc использует статический конфиг bootupd + BLS), `lsb_release`.

Логотип для терминала: `python3 distro/branding/gen-fastfetch-logo.py` рисует знак (луна и «плей») полублоками по геометрии `design/brand/src/mark.svg`; после правки бренда — `distro/branding/sync-assets.sh` (CI проверяет `--check`). Снимок — [docs/screenshots/progress/dist-003](../docs/screenshots/progress/dist-003/).

## Сборка

```bash
distro/scripts/build-image.sh localhost/alpha-os:dev            # или:
docker build -f distro/Containerfile -t localhost/alpha-os:dev distro
```

`bootc container lint` выполняется в конце сборки. Остаётся одно предупреждение из базового образа (`/var/lib/authselect/checksum`), на работу не влияет.

## Проверка на VM

Требования: docker (пользователь в группе `docker`) **или** rootful podman, `qemu-system-x86_64`, доступ к `/dev/kvm`, `ssh`, `python3`, ~6 GB на диске. Sudo на хосте не нужен.

```bash
distro/tests/vm-switch-rollback.sh          # ~10 минут; KEEP=1 — оставить VM и реестр
distro/tests/vm-install.sh                  # ~20 минут: установка по файлу ответов в QEMU (OVMF, NVMe)
```

Сценарий поднимает локальный реестр `registry:2` на `127.0.0.1:5000` (VM видит хост как `10.0.2.2`), собирает v1 и v2, ставит **стоковую** базу на raw-диск, загружает её в QEMU и по SSH проверяет:

1. `bootc switch 10.0.2.2:5000/alpha-os:dev` → загружен v1, сервисы живы, `/health` отвечает, `alpha-updated` отвечает на своём сокете (`660 root alpha-admin`), smbd слушает 445, зона firewalld `home`, таймер замаскирован; дальше агент на время сценария останавливается, чтобы не трогать bootc параллельно;
2. `bootc rollback` → снова стоковая база;
3. `bootc rollback` → снова v1;
4. в реестр под тем же тегом публикуется v2, `bootc upgrade --check` и `bootc upgrade` → загружен v2;
5. `bootc rollback` → v1;
6. `bootc switch` на v2 **по digest**, затем обратно на тег `:dev`; `bootc upgrade --check` снова отслеживает тег (нужно для поэтапной раскатки, DIST-012).

### Результат (2026-11-13)

Сценарий пройден целиком (`ALL STEPS PASSED`) на хосте Ubuntu с docker 29 и QEMU 10.2 (KVM, SeaBIOS, 2 vCPU, 3 GiB): базовый образ bootc 1.17.1, SELinux в VM — `Enforcing`. Все 6 шагов, включая переход на digest и возврат на тег. `smartd` в VM неактивен — у virtio-диска нет SMART, на железе это нормальная работа; в проверку сервисов он не входит.

### Почему именно так

- **Диск** пишется `bootc install to-disk --generic-image --via-loopback` из контейнера базового образа с `--privileged --pid=host -v /dev:/dev`. Нужны loop-устройства, `mkfs` и монтирование, поэтому **rootless podman не подходит**; docker из группы `docker` — подходит.
- **Источник** для установки — `docker://localhost:5000/…`, а не `docker-archive:`: `docker save` (containerd image store) отдаёт слои, которые ostree не принимает (`invalid gzip header`).
- **Реестр без TLS** разрешается файлом в `/etc/containers/registries.conf.d/` — и в контейнере-установщике, и в VM. Это только для тестов.
- **image-builder / bootc-image-builder** пока не используем: им нужен образ в containers-storage хоста (то есть podman), а на машине разработчика может быть только docker. Для ISO (DIST-007) — `image-builder … generic-iso` в CI на runner с podman.

## CI и подпись образов (DIST-002)

Workflow `.github/workflows/distro.yml`:

- **PR** (в том числе из форков) — только сборка, токен read-only, без секретов: `podman build` → `bootc-base-imagectl rechunk` (стабильные слои, обновление качает только изменённое) → `bootc container lint` → SBOM (syft, SPDX JSON, артефакт workflow) → `tests/signature-policy.sh`.
- **push в `main`** — то же, плюс push `ghcr.io/denishumen/alpha-os:<ver>` (`<ver>` = `45.YYYYMMDD.<run>`), подпись cosign по digest, SBOM как attestation, затем `:nightly` переносится на тот же digest. **Тег `os-vX.Y.Z`** — публикует `:X.Y.Z`.
- **promote** (ручной запуск, окружение `release`): проверяет подпись версии публичным ключом и переносит `:beta` или `:stable` на её digest.

Подпись — cosign v2 **с ключом** (не keyless, 05 §4.2 п.4): подпись лежит рядом с образом как `<repo>:sha256-<digest>.sig`, этот формат проверяет containers/image, а значит и bootc.

### Политика на устройстве

`/usr/libexec/alpha/alpha-signature-policy` ставит при сборке:

- `/etc/containers/policy.json`: `default: reject`; для `ghcr.io/denishumen/alpha-os` — `sigstoreSigned` с ключом `/usr/share/alpha/keys/alpha-os.pub` и `matchRepository` (подходит и для тегов-каналов, и для digest); локальное `containers-storage` разрешено;
- `/etc/containers/registries.d/alpha-os.yaml`: `use-sigstore-attachments: true`.

Режим задаёт build-arg `ALPHA_SIGNATURE_POLICY`: `auto` (по умолчанию: включить, если в `distro/keys/` есть `alpha-os.pub`), `enforce` (без ключа сборка падает), `off` (стоковая политика; так собирает образы VM-тест с локальным реестром). Режим записан в `/usr/share/alpha/version`.

Пока ключ не закоммичен, CI публикует образ **без** политики и без подписи (с предупреждением) — так нельзя «закирпичить» устройства политикой раньше, чем появятся подписи. Если ключ закоммичен, а секрета нет, публикация падает.

### Тест «неподписанный образ отклоняется»

`distro/tests/signature-policy.sh IMAGE` (в CI на каждом PR; локально нужен `cosign` v2): генерирует одноразовую пару ключей, кладёт в локальный реестр крошечные образы и **внутри собранного образа ОС** (его skopeo и containers/image — тот же стек, которым тянет обновления bootc) ставит политику с тестовым ключом. Ожидается: подписанный — принят; неподписанный, подписанный чужим ключом и образ из другого репозитория — отклонены. Если образ собран с `enforce`, дополнительно проверяется сама поставляемая политика.

### Шаги владельца (один раз)

Агенты ключи не создают и секреты не заводят.

1. Локально, на доверенной машине: `cosign generate-key-pair` (cosign v2), надёжный пароль. Появятся `cosign.key` и `cosign.pub`. `cosign.key` в репозиторий **не** класть; хранить офлайн-копию.
2. Settings → Secrets and variables → Actions: секрет `COSIGN_PRIVATE_KEY` = содержимое `cosign.key`, `COSIGN_PASSWORD` = пароль.
3. Settings → Environments: создать `release` с обязательным ревьюером (владелец) — для promote.
4. PR: `cosign.pub` → `distro/keys/alpha-os.pub`. С этого коммита образы собираются с `enforce`, а CI сверяет подпись этим ключом.
5. После первой публикации: пакет `alpha-os` в GHCR сделать **public** (иначе устройства не скачают образ) и связать с репозиторием.
6. Позже — перенос ключа в KMS (`cosign --key gcpkms://…`/`awskms://…`) и ротация: новый образ сначала поставляет оба ключа.

## Ограничения прототипа

- Нет брендинга (DIST-003), `alpha-admin` и медиасервер — заглушки (DIST-013), нет health-check-ов greenboot (DIST-011). Вето перезагрузки — адрес медиасервера `/api/v1/system/activity` (SRV-019) по HTTPS с пиннингом сертификата.
- Нет настоящих сервисов (DIST-009, DIST-013), нет health-check-ов greenboot (DIST-011).
- Пакеты не залочены по версиям (lockfile — отдельной задачей).
- Офлайн-обновления с USB (`oci-archive`) политикой пока отклоняются: проверку подписи для них решим вместе с офлайн-обновлением.
- Проверено только в QEMU/KVM; железо (Intel N100, AMD APU, `vainfo`, QSV/VAAPI) — отдельно.
