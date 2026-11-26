<!-- SPDX-License-Identifier: MPL-2.0 -->

# AGENTS.md — `distro/` (NAS-ОС)

Общие правила — в корневом [AGENTS.md](../AGENTS.md). Подробности — [distro/README.md](README.md), решения — ADR-0010. По ADR-0011 NAS-ОС идёт после релиза 0.1 клиентов.

- **Что здесь:** image-based ОС на bootc: `Containerfile`, `packages.txt`, `rootfs/`, скрипты в `scripts/`, VM-тест в `tests/`. Владелец — workstream DIST.
- **Нужно:** docker (пользователь в группе `docker`) или rootful podman; для VM-тестов — `qemu-system-x86_64`, OVMF (для `vm-install.sh`), `/dev/kvm`, `ssh`, `python3`, ~6–10 GB диска. sudo на хосте не нужен.

## Сборка и тесты (из корня репозитория)

```bash
distro/scripts/build-image.sh localhost/alpha-os:dev   # сборка образа + bootc container lint
distro/tests/branding.sh localhost/alpha-os:dev        # брендинг Kinolune OS в образе, fastfetch (DIST-003)
distro/tests/vm-switch-rollback.sh                     # ~10 мин: switch → rollback → upgrade → digest-пин
(cd distro/admin && go vet ./... && go test -race ./...) # alpha-admin: нужен dbus-daemon (иначе D-Bus-тесты пропускаются)
distro/admin/tests/pam-container.sh                    # alpha-admin с настоящим PAM в контейнере Fedora
distro/admin/tests/samba-container.sh                  # DIST-018: alpha-admin-priv + smbd + smbclient в контейнере Fedora
distro/admin/tests/vm-admin-e2e.sh                     # ~20 мин: alpha-admin в QEMU против настоящих systemd/NM/firewalld/UDisks2/logind, Samba по SMB снаружи VM (нужен smbclient)
distro/tests/vm-install.sh                             # ~25 мин: установщик по файлу ответов (OVMF, NVMe), затем снапшот и откат /var (DIST-010)
distro/scripts/build-iso.sh                            # ~30 мин: ISO установщика (DIST-007), размер < 1.9 GiB
distro/tests/iso-boot.sh distro/.work/iso/kinolune-os-*.iso  # ~7 мин: ISO в QEMU/OVMF: TUI, текстовое меню, пробная установка через TUI
distro/tests/wifi-hwsim-vm.sh FEDORA_CLOUD.qcow2 nm_probe  # ~10 мин: Wi-Fi приложения в VM с mac80211_hwsim (DIST-032)
shellcheck distro/scripts/*.sh distro/tests/*.sh       # если установлен
just distro::installer-test                            # Go: установщик и TUI (DIST-004…006; эталоны экранов: go test ./internal/tui -update)
just distro::installer-tui-demo                        # TUI на снимке дисков, пробный запуск (без root)
just distro::updated-test                              # Go: агент обновлений alpha-updated на fake bootc (DIST-009)
```

Локальные артефакты (диски, ключи SSH, логи) — в `distro/.work/`, он в `.gitignore`.

## Правила области

- В каждом скрипте и конфиге SPDX-заголовок (`# SPDX-License-Identifier: MPL-2.0`).
- Базовый образ — по digest; пакеты RPM Fusion и NVIDIA — только по задачам LEGAL-010 и DIST.
- Без бренда и логотипов Fedora: имя Kinolune OS, формулировка «based on Fedora Linux»; что сознательно остаётся «fedora» — в README, раздел «Брендинг» (DIST-003). После правки `design/brand` — `distro/branding/sync-assets.sh`.
- Ключи подписи образа (cosign) — только в CI-окружении, см. [docs/dev/secrets.md](../docs/dev/secrets.md).
