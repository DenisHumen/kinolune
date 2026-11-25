<!-- SPDX-License-Identifier: MPL-2.0 -->

# DIST-006 — полноэкранный установщик Kinolune OS (TUI)

Сервер сборки проекта, 2026-11-26: ISO `0.0.1-tui.1` (`distro/scripts/build-iso.sh`, 1292 MiB), QEMU + OVMF с KVM в контейнере. Снимки — экран монитора (tty1, консоль Linux, шрифт `LatArCyrHeb-16`) через `screendump` QEMU; клавиши — `sendkey`.

| Файл | Что |
|---|---|
| [tty1-01-language-uk.png](tty1-01-language-uk.png) | сразу после загрузки ISO: выбор языка, Українська первой |
| [tty1-02-quit-uk.png](tty1-02-quit-uk.png) | Ctrl+C: «Вийти з інсталятора?» |
| [tty1-03-disk-uk.png](tty1-03-disk-uk.png) | диск: виртуальный NVMe предложен; DVD с ISO и loop — «Не пропонуються» с причиной; S/N без символа «…» |
| [tty1-04-layout-uk.png](tty1-04-layout-uk.png) | «Залишити решту диска під дані», система 20 ГиБ |
| [tty1-06-user-uk.png](tty1-06-user-uk.png) | администратор: пароль звёздочками, дважды |
| [tty1-07-network-static-uk.png](tty1-07-network-static-uk.png) | статический адрес (в VM нет сети — порт «будь-який дротовий») |
| [tty1-09-summary-wrong-name-uk.png](tty1-09-summary-wrong-name-uk.png) | сводка, предупреждение о стирании; введено `sda` вместо `nvme0n1` — «Ім'я не збігається» |
| [tty1-11-done-uk.png](tty1-11-done-uk.png) | настоящая установка завершена (26 с на этом сервере): что дальше, адрес панели |
| [tty1-12-save-uk.png](tty1-12-save-uk.png) | «Зберегти файл відповідей»: флешки нет — только `/root` live-системы; пароль — хешем |
| [tty1-13-saved-uk.png](tty1-13-saved-uk.png) | «Збережено: /root/kinolune-answers.json» |
| [iso-boot-install-tui.txt](iso-boot-install-tui.txt) | `INSTALL=tui distro/tests/iso-boot.sh`: TUI на последовательной консоли, текстовое меню, пробный запуск через все экраны, настоящая установка через TUI, проверка сохранённого файла (хеш yescrypt, без открытого пароля, проходит `--dry-run`), загрузка установленного диска до `kinolune-iso login:` |

Эталонные экраны 80×24 (uk/en/ru) для тестов — в [`distro/installer/internal/tui/testdata/golden/`](../../../../distro/installer/internal/tui/testdata/golden/).
