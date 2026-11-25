<!-- SPDX-License-Identifier: MPL-2.0 -->
# ADR-NNNN. ISO установщика Kinolune OS: `image-builder generic-iso` из live-образа на той же базе, payload офлайн, текстовое меню на tty1

- Статус: **предложен** (DIST-007)
- Дата: 2026-11-25
- Задача: DIST-007 · исследование: [05](../research/05-nas-linux-distro.md) §2.3, §6.2–6.3 · связано: [ADR-0010](0010-nas-os-phase0.md), [ADR-0166](0166-installer-disk-layout.md), DIST-003, DIST-005, DIST-006, DIST-008

## Контекст

Установщику (DIST-005) нужен загрузочный носитель: live-ISO, который стартует на любом UEFI-ПК, сам показывает установщик на tty1 и ставит ОС **без сети** — образ ОС (payload) лежит на ISO в `containers-storage`. Ограничения: ISO меньше 1.9 GiB (GitHub Releases принимает файлы < 2 GiB, 05 §6.3); сборка — в CI без своих серверов; бренд Kinolune OS, без бренда Fedora (DIST-003); TUI (DIST-006) ещё не готов. `bootc-image-builder` архивирован, `anaconda-iso` в `image-builder` нет, Anaconda нам не нужна (свой установщик, ADR-0166).

## Решение

1. **`image-builder build generic-iso`** (osbuild, пакет Fedora) из нашего bootc-контейнера `distro/installer/Containerfile` с `--bootc-installer-payload-ref <образ ОС>`. Контейнер следует контракту `generic-iso`: одно ядро, `initramfs.img` с `dmsquash-live`, shim и GRUB EFI в `/boot/efi/EFI/<vendor>`, `/usr/lib/image-builder/bootc/iso.yaml` (метка, аргументы ядра, меню GRUB), инструменты ISO (`xorriso`, `squashfs-tools`, `isomd5sum`) внутри образа.
2. **Live-образ — от той же закреплённой базы**, что и ОС (`fedora-bootc:45@sha256:…`), а не от отдельной minimal-базы. Причина — размер: `mksquashfs` хранит одинаковые файлы один раз, поэтому базовые слои payload в `/var/lib/containers/storage` почти ничего не добавляют к live-системе. Отдельная minimal-база дала бы два разных набора файлов, и ISO вырос бы. Live-образ дополнительно худеет: без документации, man-страниц и чужих локалей, без прошивок GPU/NPU/аудио/WWAN (у установленной ОС свои прошивки, их это не касается), initramfs `--no-hostonly` со сжатием zstd и без микрокода.
3. **Payload кладётся под тем именем, которое отслеживает установленная система** (`ghcr.io/denishumen/alpha-os:stable` по умолчанию): файл ответов по умолчанию берёт `image.source = containers-storage:<image.target>`, так что `bootc install` ставит офлайн, а система затем обновляется из реестра (ловушка 05 §2.4 закрыта).
4. **Запуск установщика:** `alpha-live-installer@tty1.service` вместо `getty@tty1`, второй экземпляр — на `ttyS0`, только если ядру передан `console=ttyS0` (меню ISO передаёт `console=ttyS0,115200 console=tty0`: монитор остаётся главной консолью, а headless-машины и QEMU-тест видят установщик по COM-порту). Пока нет TUI, это текстовое меню (en/uk/ru, выбор языка первым экраном): показать диски, найти `kinolune-answers.json`/`answers.json` на флешке, написать файл ответов из примера в `nano`, проверить план (`--dry-run`), установить (`alpha-installer` сам спросит имя диска), консоль, перезагрузка. Две консоли не запустят установку одновременно (`flock`). Root без пароля — только на локальной консоли live-системы, `sshd` выключен. DIST-006 заменит меню на TUI в том же юните.
5. **Меню GRUB и метка ISO:** «Install Kinolune OS», «… (basic graphics)» с `nomodeset`, «… (copy to RAM …)» с `rd.live.ram=1`; метка тома `Kinolune-OS-Installer`; `os-release` live-системы — `Kinolune OS <версия> Installer` (скрипт брендинга DIST-003, `VARIANT=Installer`).
6. **Сборка в одном привилегированном контейнере-помощнике Fedora 45** (podman + image-builder), `distro/scripts/build-iso.sh`: хосту нужен только docker или rootful podman, одинаково на сервере сборки и в CI. Хранилище, кэш osbuild и `/var/tmp` помощника — каталоги хоста (overlayfs внутри overlay-корня контейнера не работает).
7. **CI:** `.github/workflows/distro-iso.yml` — вручную, на `main` и на готовых (не draft) PR с изменениями `distro/**`: сборка ISO, проверка размера (`build-iso.sh` падает при ≥ 1.9 GiB), загрузка в QEMU/OVMF и проверки по последовательной консоли (`distro/tests/iso-boot.sh`: меню GRUB, приглашение установщика, список дисков без самого ISO, `os-release` Kinolune, payload в `containers-storage`, `--dry-run` на виртуальном NVMe). Сам ISO — артефакт только для `main` и ручных запусков (7 дней), для PR — журналы.

## Последствия

- `+` Ноль своего кода сборки ISO: GRUB, shim (Secure Boot подписи Fedora), squashfs, xorriso — из osbuild; мы описываем только содержимое live-образа.
- `+` Офлайн-установка и правильная цель обновлений с первого запуска.
- `+` Один скрипт сборки для сервера и CI.
- `−` Размер ISO зависит от размера образа ОС: каждый новый пакет в `distro/packages.txt` идёт в ISO почти целиком. CI ловит превышение лимита; при росте — сжатие xz и отдельный вариант ISO (NVIDIA, 05 §6.3 п.2).
- `−` Payload — один канал (`stable`); файл ответов с `image.channel: beta` должен задать `image.source` явно (или ставить `stable` и переключаться после установки).
- `−` Меню — временное решение до TUI (DIST-006); Secure Boot + swtpm и полная установка с ISO в CI — DIST-008.
