<!-- SPDX-License-Identifier: MPL-2.0 -->

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="design/brand/export/logo/lockup-dark.png">
    <img alt="Kinolune" src="design/brand/export/logo/lockup-light.png" width="420">
  </picture>
</p>

<p align="center"><b>Свободный медиаплеер для дома: фильмы, сериалы, аниме, музыка и Twitch на всех ваших устройствах — без облака и аккаунтов.</b></p>

<p align="center">
  <a href="LICENSE"><img alt="Лицензия MPL-2.0" src="https://img.shields.io/badge/license-MPL--2.0-blue.svg"></a>
  <a href="https://github.com/DenisHumen/kinolune/actions/workflows/rust.yml"><img alt="Rust" src="https://github.com/DenisHumen/kinolune/actions/workflows/rust.yml/badge.svg"></a>
  <a href="https://github.com/DenisHumen/kinolune/actions/workflows/flutter.yml"><img alt="Flutter" src="https://github.com/DenisHumen/kinolune/actions/workflows/flutter.yml/badge.svg"></a>
  <a href="https://github.com/DenisHumen/kinolune/actions/workflows/hygiene.yml"><img alt="Hygiene" src="https://github.com/DenisHumen/kinolune/actions/workflows/hygiene.yml/badge.svg"></a>
  <a href="https://github.com/DenisHumen/kinolune/releases"><img alt="Последний релиз" src="https://img.shields.io/github/v/release/DenisHumen/kinolune?include_prereleases&amp;label=release"></a>
  <img alt="Платформы" src="https://img.shields.io/badge/platforms-Linux%20%7C%20Windows%20%7C%20macOS%20%7C%20Android-informational">
</p>

<p align="center"><b>Русский</b> · <a href="README.en.md">English</a></p>

<p align="center">
  <img alt="Главный экран Kinolune на компьютере" src="docs/screenshots/0.1/desktop-home.png" width="640">
</p>
<p align="center">
  <img alt="Kinolune на телефоне" src="docs/screenshots/0.1/phone-home.png" height="300">
  &nbsp;
  <img alt="Kinolune на телевизоре" src="docs/screenshots/0.1/tv-home.png" height="300">
</p>

<sub>На скриншотах — демо-медиатека с собственными обложками-заглушками, без чужого арта.</sub>

## Возможности

- библиотека фильмов, сериалов и аниме с точным распознаванием по именам файлов и папок;
- воспроизведение почти любых форматов, HDR/Dolby Vision, passthrough звука;
- источники: локальные папки, SMB, NFS, WebDAV, облака, Plex/Jellyfin/Emby, собственный медиасервер;
- музыкальный плеер (mp3, flac, alac, opus, dsd…), gapless, тексты песен;
- удобный клиент Twitch: эфиры с чатом, записи, избранные стримеры;
- продолжение просмотра на любом устройстве — без облака, всё локально;
- собственный NAS-дистрибутив Linux с простым установщиком и RAID.

Никаких серверов и аккаунтов проекта: приложение работает у вас, запросы к TMDB и другим сервисам идут прямо с устройства, телеметрии нет. Название — **Kinolune** (ADR-0011); до 2026-11-15 репозиторий назывался `project_alpha`, старые ссылки GitHub перенаправляет.

## Статус

🚧 **Ранние альфа-версии** для Linux, Windows, macOS и Android. Текущее состояние работ — [docs/STATUS.md](docs/STATUS.md), полный план — [docs/ROADMAP.md](docs/ROADMAP.md), живой прогресс по задачам — [ветка `progress`](https://github.com/DenisHumen/kinolune/blob/progress/README.md).

## Скачать

Сборки — в [GitHub Releases](https://github.com/DenisHumen/kinolune/releases): Linux x86_64 и ARM64 (`.deb`, `.rpm`, AppImage), Windows (установщик), macOS (`.dmg`), Android (APK).

### Установка на Linux

Ubuntu 24.04+, Debian 13+, Linux Mint 22+:

```sh
curl -fsSL https://denishumen.github.io/kinolune/apt/kinolune.sources | sudo tee /etc/apt/sources.list.d/kinolune.sources >/dev/null
sudo apt update && sudo apt install kinolune
```

Fedora:

```sh
curl -fsSL https://denishumen.github.io/kinolune/rpm/kinolune.repo | sudo tee /etc/yum.repos.d/kinolune.repo >/dev/null
sudo dnf install kinolune
```

Дальше Kinolune обновляется вместе с системой (`sudo apt upgrade`, `sudo dnf upgrade`). Файлы `.deb` и `.rpm` из релиза при установке добавляют этот же репозиторий. Команды одинаковы для x86_64 и ARM64 (aarch64: Raspberry Pi 5, ARM-ноутбуки) — пакетный менеджер сам берёт сборку своей архитектуры.

## Участвовать

Pull request'ы приветствуются — см. [CONTRIBUTING](.github/CONTRIBUTING.md) и [кодекс поведения](.github/CODE_OF_CONDUCT.md). AI-агентам — [AGENTS.md](AGENTS.md). Вопросы и идеи — в [Discussions](https://github.com/DenisHumen/kinolune/discussions). Уязвимости — только приватно, см. [SECURITY](.github/SECURITY.md).

Где что лежит:

| Каталог | Что внутри |
|---|---|
| [`core/`](core/) | ядро на Rust: источники, распознавание, медиатека, синхронизация |
| [`clients/`](clients/) | Flutter-приложение и нативные плееры (Android, Apple, десктоп) |
| [`server/`](server/) | медиасервер |
| [`distro/`](distro/) | Kinolune OS — NAS-дистрибутив |
| [`packaging/`](packaging/) | пакеты и установщики |
| [`design/`](design/) | бренд, токены, иконки |
| [`config/`](config/) | подписанный удалённый конфиг |
| [`tools/`](tools/) | сборка библиотек, релизы, QA и тестовые медиа, roadmap |
| [`docs/`](docs/) | документация, ADR, планы |

## Пожертвовать

Площадки для пожертвований ещё подключаются; когда они появятся, ссылки будут здесь и на кнопке **Sponsor** репозитория. Уже сейчас очень помогают отчёты об ошибках, примеры неверного распознавания и pull request'ы.

## Документация

- [docs/user/](docs/user/README.md) — руководство пользователя: установка, источники, распознавание, плеер
- [docs/STRATEGY.md](docs/STRATEGY.md) — стратегия и ключевые решения
- [docs/ROADMAP.md](docs/ROADMAP.md) — детальный план и чек-лист задач
- [docs/adr/](docs/adr/) — архитектурные решения (ADR)
- [docs/dev/architecture.md](docs/dev/architecture.md) — архитектура для разработчиков
- [docs/research/](docs/research/) — исследования
- [docs/legal/privacy-policy.md](docs/legal/privacy-policy.md) — политика конфиденциальности

## Лицензия

Код — [MPL-2.0](LICENSE). Проект соответствует спецификации [REUSE](https://reuse.software/): лицензия каждого файла указана в его заголовке или в [REUSE.toml](REUSE.toml), тексты лицензий — в [LICENSES/](LICENSES/).

Kinolune не связан с Infuse/Firecore, Plex, Jellyfin, Emby, Twitch, TMDB и другими упомянутыми сервисами; их названия принадлежат владельцам.
