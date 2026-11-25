<!-- SPDX-License-Identifier: MPL-2.0 -->

# DIST-007 — Live-ISO Kinolune OS

Сборка и загрузка на сервере сборки проекта, 2026-11-25: `distro/scripts/build-iso.sh` (docker, контейнер-помощник Fedora 45), QEMU 10 + OVMF с KVM в контейнере.

| Файл | Что |
|---|---|
| [iso-info.txt](iso-info.txt) | размер ISO (1 355 087 872 байт = 1292 MiB при лимите 1.9 GiB), digest payload, параметры squashfs (zstd, блок 128 KiB, дубликаты удалены) |
| [grub-menu.png](grub-menu.png) | меню GRUB ISO (UEFI): «Install Kinolune OS», «… (basic graphics)», «… (copy to RAM …)» |
| [installer-menu-ru.png](installer-menu-ru.png) | tty1 после выбора языка «Русский»: меню `alpha-live-installer` |
| [installer-disks-ru.png](installer-disks-ru.png) | пункт 1: виртуальный NVMe предложен, DVD с ISO и loop squashfs исключены |
| [boot-serial.txt](boot-serial.txt) | последовательная консоль: `distro/tests/iso-boot.sh` с `INSTALL=1` — меню, диски, `os-release`, payload, `--dry-run`, офлайн-установка на NVMe |
| [installed-serial.txt](installed-serial.txt) | загрузка установленного с ISO диска до `kinolune-iso login:` |

Числа: payload (образ ОС) 2.16 GB в распакованном виде, live-образ 2.21 GB; благодаря общей базе и удалению дубликатов squashfs весит 1128 MiB, ISO — 1292 MiB.

Известная мелочь: консольный шрифт `LatArCyrHeb-16` не содержит «…», в списке дисков вместо него точка (`S/N .rget`). Исправит TUI (DIST-006).
