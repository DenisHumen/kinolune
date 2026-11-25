<!-- SPDX-License-Identifier: MPL-2.0 -->
# distro/installer — `alpha-installer` (Go)

Установщик NAS-ОС ([исследование 05 §2.3–2.4](../../docs/research/05-nas-linux-distro.md), [ADR-0010](../../docs/adr/0010-nas-os-phase0.md)). В нём **обнаружение дисков** (DIST-004) и **установка по файлу ответов** (DIST-005, решения — [ADR-0166](../../docs/adr/0166-installer-disk-layout.md)). TUI — в DIST-006, live-ISO — в DIST-007.

## Обнаружение дисков (`internal/disks`)

Пакет работает **только на чтение**. Он запускает `lsblk -J -b -o …`, `udevadm info --query=property` и, если попросить, `smartctl -H -j`, а также читает `/sys/block/*/device/model` и `/proc/cmdline`. На блочные устройства он ничего не пишет.

Что пакет делает:

- **Человеческое имя.** Для NVMe модель берётся из sysfs, для остальных — `MODEL` из lsblk, затем `ID_MODEL_ENC` из udev. Пробелы нормализуются, бренд приводится к написанию «как на коробке»: `Samsung SSD 990 PRO 1TB` → `Samsung 990 PRO 1TB`, `WDC WD40EFRX-68N32N0` → `WD WD40EFRX-68N32N0`, `ST4000VN008` → `Seagate ST4000VN008`, `WD_BLACK SN850X` → `WD Black SN850X`. У USB, SAS и SCSI к модели добавляется `VENDOR`, если он не общий (`ATA`, `Generic`). Если модели нет совсем, имя строится как «вендор + тип + размер», например `JMicron USB drive 500 GB` или `Virtual disk 69 GB`.
- **Размер в десятичных единицах**, как на коробке: 1 TB = 10¹² байт (`1000204886016` → `1.0 TB`).
- **Шина и тип носителя**: NVMe, SATA SSD/HDD, SAS HDD, USB, eMMC, SD/MMC, virtio. Для QEMU и VirtualBox пишется «virtual».
- **Исключённые устройства** возвращаются с причиной, чтобы интерфейс мог объяснить, почему диска нет в списке:
  - `virtual-device` — loop, zram, ram, nbd;
  - `optical` — `sr*`;
  - `boot-media` — собственная загрузочная флешка установщика или диск, с которого работает текущая система. Признаки: в поддереве смонтирован `/run/initramfs/live`, `/`, `/sysroot` или `/boot`, либо совпадает метка, UUID или путь из `root=live:…` в `/proc/cmdline`; это работает и при `rd.live.ram=1`;
  - `no-media` — пустой картридер;
  - `read-only`;
  - `too-small` — меньше ≈20.4 GB (ESP 1 GiB + `/boot` 2 GiB + корень 16 GiB + GPT, DIST-005);
  - `hardware-area` — `mmcblkXbootN` и `rpmb`.
- **Признаки существующих данных**: таблица разделов (GPT или DOS) и число разделов; файловые системы с метками; участники mdraid вместе с массивом и ФС на нём; LVM, LUKS, ZFS (с именем пула), swap; ESP («вероятно, установлена ОС»); Microsoft reserved («Windows»); смонтированные разделы; провал SMART.

Результат возвращается как `[]disks.Disk`. JSON-форма этой структуры — контракт для TUI и будущего веб-установщика. Тексты находок в `Finding.String()` даны на английском и служат запасным вариантом; локализует их TUI по полю `Kind`.

## Установка (`internal/install`, `internal/answers`)

```bash
alpha-installer install --answers answers.json --dry-run   # проверить файл и напечатать план, диски не трогаются
alpha-installer install --answers answers.json             # в терминале спросит имя диска для подтверждения
alpha-installer install --answers answers.json --yes       # без вопросов (автоматическая установка, CI)
alpha-installer install --answers answers.json --dry-run --from DIR   # план на записанном снимке дисков
```

Запускается от root на live-системе. Что происходит:

1. **Выбор диска.** Путь из файла ответов (`/dev/disk/by-id/…` надёжнее, чем `/dev/sdX`) разрешается и сверяется с пробой дисков. Установка **отказывается**, если это загрузочный носитель установщика или диск текущей системы, loop/zram, оптический или read-only привод, раздел вместо диска, диск меньше ≈20.4 GB или на нём что-то смонтировано. Остальные диски не трогаются: команды записи получают только выбранный диск и его разделы.
2. **Разметка** (GPT, только UEFI) через `wipefs` + `sfdisk`:

   | # | Метка | Размер | ФС | Точка монтирования |
   |---|---|---|---|---|
   | 1 | `EFI-SYSTEM` | 1 GiB | vfat | `/boot/efi` |
   | 2 | `boot` | 2 GiB | ext4 | `/boot` |
   | 3 | `root` | остаток или `root_size_gib` | btrfs | `/`, `/var` = сабволюм `@var` |
   | 4 | `kinolune-data` (если `data_partition`) | остаток | btrfs | `/var/srv/data/<data_pool>` |

3. **`bootc install to-filesystem`** из `containers-storage` (офлайн-образ ISO) с `--target-imgref` на реестр, `--enforce-container-sigpolicy` и `--skip-finalize`. По умолчанию bootc запускается в привилегированном контейнере самого образа (`podman run --pull=never …`), `runner: "host"` вызывает `bootc` живой системы с `--source-imgref`.
4. **Настройка:** сабволюм `@var` с содержимым `/var` и строка в `/etc/fstab`; `/etc/hostname`; пользователь в группе `wheel` с паролем yescrypt и домашним каталогом `/var/home/<имя>`; NetworkManager-подключение `kinolune-lan`; `/var/lib/alpha/install.json`; метки SELinux (`setfiles`).

### Файл ответов (схема версии 1)

JSON, неизвестные поля — ошибка, все ошибки показываются сразу. Пример — [examples/answers.json](examples/answers.json).

| Поле | Обязательно | По умолчанию | Что это |
|---|---|---|---|
| `version` | да | — | всегда `1` |
| `disk.path` | да | — | весь диск: `/dev/disk/by-id/…` или `/dev/nvme0n1` |
| `disk.data_partition` | | `false` | оставить остаток диска под данные |
| `disk.root_size_gib` | | `64` (только с `data_partition`) | размер корня, не меньше 16 |
| `disk.data_pool` | | `media` (только с `data_partition`) | имя пула: `/var/srv/data/<имя>` |
| `hostname` | | `kinolune` | имя хоста (a-z, 0-9, `-`) |
| `user.name` | да | — | администратор (a-z, 0-9, `-`, `_`; не `root`) |
| `user.full_name` | | — | полное имя |
| `user.password_hash` | одно из двух | — | yescrypt-хеш `$y$…` (`mkpasswd -m yescrypt`), рекомендуется для файлов на диске |
| `user.password` | одно из двух | — | открытый пароль; хешируется yescrypt при установке |
| `user.ssh_authorized_keys` | | — | открытые ключи SSH пользователя |
| `network.mode` | | `dhcp` | `dhcp` или `static` |
| `network.interface` | | любой проводной | например `enp1s0` |
| `network.address`, `gateway`, `dns` | для `static` | — | `192.168.1.20/24`, `192.168.1.1`, `["192.168.1.1"]` |
| `image.channel` | | `stable` | `stable` или `beta` |
| `image.target` | | `ghcr.io/denishumen/alpha-os:<channel>` | что отслеживает `bootc upgrade`; только реестр, не локальный путь |
| `image.source` | | `containers-storage:<target>` | офлайн-образ в хранилище live-системы |
| `image.enforce_signature_policy` | | `true` | `false` — только для неподписанных тестовых образов |
| `image.runner` | | `podman` | `podman` или `host` |
| `root_ssh_authorized_keys` | | — | ключи для root (CI, опытные пользователи) |
| `kernel_args` | | — | дополнительные аргументы ядра |
| `language` | | `ru` | язык первого запуска (`ru`, `uk`, `en`…), пишется в `install.json` |

### `/var/lib/alpha/install.json`

Читают админка и `alpha-updated` при первом запуске; секретов нет. Поля: `schema` (1), `install_id`, `installed_at`, `installer_version`, `hostname`, `language`, `admin_user`, `update_channel`, `image` (`source`, `target`, `signature_policy_enforced`), `disk` (`model`, `size`, `path`), `filesystems` (метка → UUID), `data_pool` (`name`, `mountpoint`, `uuid` или `null`), `network`, `first_boot`.

## Команда дисков

```bash
cd distro/installer
go run ./cmd/alpha-installer disks             # кандидаты для установки
go run ./cmd/alpha-installer disks --all       # вместе с исключёнными и причинами
go run ./cmd/alpha-installer disks --json --smart
go run ./cmd/alpha-installer disks --record DIR   # снимок железа для баг-репорта (S/N и WWN скрыты)
go run ./cmd/alpha-installer disks --from DIR     # воспроизвести снимок
```

## Тесты

```bash
go vet ./... && go test ./...          # go test -run TestGolden -update — обновить эталоны дисков
```

В `internal/disks/testdata/` лежат 10 конфигураций железа. Каждая состоит из `lsblk.json`, свойств udev, файлов sysfs, `/proc/cmdline`, ответов `smartctl` и эталона `expected.txt`:

| Конфигурация | Что проверяет |
|---|---|
| `01-live-usb-nas` | live-USB, NVMe, SATA SSD, 2×HDD, loop и zram |
| `02-mdadm-reinstall` | mdadm RAID1 с btrfs, участник RAID на целом диске, прошлая установка на NVMe, флешка не смонтирована (`rd.live.ram`), SMART FAILED |
| `03-vm-virtio` | virtio без модели, загрузка с виртуального CD |
| `04-qemu-ci` | QEMU NVMe и 3 SATA HDD, как в DIST-008 |
| `05-windows-desktop` | OEM-Samsung с Windows (ESP, MSR), NTFS на Crucial |
| `06-n100-emmc` | eMMC с boot-областями, SD-карта, WD_BLACK |
| `07-usb-mix` | внешний HDD, пустой картридер, маленькая флешка, мост без модели, загрузка по UUID |
| `08-legacy-lsblk-sas` | старый формат lsblk (строки, `mountpoint`, без `path`), SAS, ZFS, LVM, LUKS, Micron с `_` |
| `09-dual-nvme-previous-install` | модель из sysfs при пустом lsblk, прошлая установка с разделом `media`, `root=live:/dev/sda1` |
| `10-workstation-host` | реальный снимок рабочей станции (S/N скрыты): диск с `/` исключается |

Установка тестируется на фейковом runner (`internal/install/install_test.go`): `sfdisk`, `mkfs`, `mount`, `btrfs`, `bootc`/`podman`, `chpasswd`, `setfiles` не запускаются, а их результат эмулируется во временном каталоге. Тесты проверяют порядок команд, что ни одна команда не называет чужой диск, содержимое `/etc` и `@var`, `install.json` и отказы (загрузочный диск, смонтированный диск, раздел, нет `--yes`).

Сквозной тест в QEMU (UEFI/OVMF, виртуальный NVMe) — [`distro/tests/vm-install.sh`](../tests/vm-install.sh): установка по файлу ответов, загрузка установленного диска, проверки по SSH и `bootc upgrade --check` против реестра.
