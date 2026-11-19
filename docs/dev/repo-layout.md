<!-- SPDX-License-Identifier: MPL-2.0 -->

# Аккуратная структура репозитория (план REPO-014)

Запрос владельца 2026-11-19: в корне репозитория слишком много файлов и папок, на GitHub это выглядит неряшливо. Нужно разложить всё аккуратно и красиво, ничего не сломав. Делаем **после 100 % основного плана**, перед публичным 1.0. Сейчас только исследование и план, в ROADMAP это задача REPO-014.

## Что сейчас в корне (2026-11-19)

На главной странице GitHub 17 папок и 22 файла:

- **Папки:** `.cargo`, `.config`, `.devcontainer`, `.github`, `LICENSES`, `clients`, `config`, `core`, `data`, `design`, `distro`, `docs`, `packaging`, `server`, `site`, `spikes`, `tests`, `tools`.
- **Файлы:** `.editorconfig`, `.env.example`, `.gitattributes`, `.gitignore`, `.gitleaks.toml`, `.gitleaksignore`, `.pre-commit-config.yaml`, `.xcode-version`, `AGENTS.md`, `CLAUDE.md`, `CODE_OF_CONDUCT.md`, `CONTRIBUTING.md`, `Cargo.lock`, `Cargo.toml`, `LICENSE`, `README.en.md`, `README.md`, `REUSE.toml`, `SECURITY.md`, `deny.toml`, `justfile`, `mise.toml`, `rust-toolchain.toml`.

Почти пустые папки:

| Папка | Что внутри | Куда |
|---|---|---|
| `site/` | только `.gitkeep` | удалить; сайт появится в SITE-001 в `docs/site/` или отдельном репозитории |
| `spikes/` | два законченных спайка (64 файла, ADR-0008 и ADR их спайков) | удалить из дерева; история остаётся в git, в ADR ссылка на коммит |
| `data/` | `anime-mappings/` (скрипт сборки и `upstream.env`) | `tools/anime-mappings/` |
| `tests/` | `justfile` и `media/` (манифест тестовых медиа) | `tools/qa/media/` |
| `config/` | удалённый конфиг и его подпись | **остаётся** (см. «Что нельзя трогать») |

## Что обязано лежать в корне

| Файл / папка | Почему |
|---|---|
| `README.md`, `README.en.md`, `LICENSE` | GitHub показывает их на главной; лицензию ищет в корне |
| `LICENSES/`, `REUSE.toml` | спецификация REUSE требует корень |
| `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `.cargo/` | корень Cargo workspace (core, server, tools); rustup ищет toolchain вверх по дереву |
| `.github/` | GitHub |
| `.gitignore`, `.gitattributes`, `.editorconfig` | git и редакторы читают из корня |
| `AGENTS.md` | AI-агенты читают его из корня |
| `config/` | **адрес вшит в выпущенные сборки**: `pa-remote-config` качает `https://raw.githubusercontent.com/DenisHumen/kinolune/main/config/...`. Перенос сломает удалённый конфиг у всех установленных версий. Можно перенести, только сохранив копию по старому пути на годы вперёд, поэтому не трогаем |

## Что можно убрать из корня без потерь

| Сейчас | Куда | Что поправить |
|---|---|---|
| `CODE_OF_CONDUCT.md`, `CONTRIBUTING.md`, `SECURITY.md` | `.github/` | GitHub находит их там же (вкладки на главной сохраняются); ссылки в README, docs, шаблонах issue |
| `CLAUDE.md` | `.claude/CLAUDE.md` | Claude Code читает оба места; ссылки в AGENTS.md |
| `.gitleaks.toml`, `.gitleaksignore` | `.github/gitleaks/` | `--config` и `--gitleaks-ignore-path` в `hygiene.yml` и pre-commit |
| `deny.toml` | `.config/deny.toml` | `cargo deny --config .config/deny.toml` в CI и `justfile` |
| `mise.toml` | `.config/mise.toml` | mise читает его там сам |
| `justfile` | `.justfile` | just находит скрытое имя сам; команды не меняются |
| `.xcode-version` | `.config/xcode-version` | путь в Apple-workflows |
| `.pre-commit-config.yaml` | остаётся | pre-commit ищет только в корне |
| `.env.example` | остаётся | так принято, и его ищут люди |
| `site/`, `spikes/`, `data/`, `tests/` | см. таблицу выше | `REUSE.toml`, `dependabot.yml`, ссылки в ADR и ROADMAP §0.4, `just` |

Итог: **9 папок** (`clients`, `config`, `core`, `design`, `distro`, `docs`, `packaging`, `server`, `tools`) плюс `.github`, `LICENSES` и скрытые `.cargo`, `.config`, `.devcontainer`, и **около 10 файлов**: README×2, LICENSE, AGENTS.md, Cargo.toml/lock, rust-toolchain.toml, REUSE.toml и скрытые служебные.

Отдельный вариант — перенести Cargo workspace в `core/`, чтобы из корня ушли ещё `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml` и `.cargo/`. Тогда `server/` и `tools/crates/` придётся подключать через `../`, а поменять нужно все пути `target/`, codegen FRB, CI, кэши и скрипты сборки. Это рискованно и даёт мало, поэтому **не делаем**.

## Красота главной страницы

- **README:**
  - баннер или логотип и одна строка «что это»;
  - бейджи: лицензия, CI, последний релиз, платформы;
  - скриншот главного экрана (телевизор, компьютер, телефон);
  - блоки «Скачать», «Возможности», «Участвовать», «Пожертвовать»;
  - ссылка на английскую версию.
- **«About» репозитория:** описание, сайт, темы (topics) и картинка для превью ссылок (social preview).
- **`.github/`:** шаблоны issue, PR и Discussions, `FUNDING.yml` — проверить, что всё на месте и на нужных языках.

## Порядок выполнения (когда дойдёт очередь)

1. Одна ветка `chore/REPO-014-tidy-root`, переносы через `git mv`, чтобы сохранить историю файлов.
2. Поправить все ссылки и пути: `grep -rn` по старым путям в `.github/`, `justfile`, `REUSE.toml`, `docs/`, `AGENTS.md`, скриптах `tools/` и `packaging/`, `pubspec`/Gradle/Xcode-проектах.
3. Проверки:
   - полный CI на PR (Rust, Flutter, Android, Apple, Hygiene с gitleaks, reuse, Dependencies, Roadmap, Netem);
   - `release-app.yml` в пробном режиме (`version=0.0.0-dryrun`);
   - `distro.yml` без публикации;
   - проверка ссылок `links` и `lychee` в docs;
   - `just --list` и `just bootstrap` в чистом клоне;
   - `reuse lint`, `roadmap lint`.
4. Во время переноса не должно быть открытых PR агентов: делать в «тихое окно», потом всем веткам слить main.
5. После слияния: проверить, что вкладки GitHub (Code of conduct, Contributing, Security) и `raw.githubusercontent.com/.../main/config/...` работают.

## Риски

- Удалённый конфиг по старому адресу (`config/`) — поэтому не трогаем.
- Ссылки на файлы в старых ADR, issue и внешних статьях ведут на `blob/main/...`. В ADR пути правим, внешние ссылки на перенесённые `CONTRIBUTING.md`/`SECURITY.md` GitHub не перенаправляет. Это допустимо, вкладки остаются.
- Агенты и их скрипты на сервере сборки, где жёстко прописаны пути (`tests/servers`, `tools/qa/netem`). Сверить по `grep`.
