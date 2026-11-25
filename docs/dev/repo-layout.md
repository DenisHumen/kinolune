<!-- SPDX-License-Identifier: MPL-2.0 -->

# Аккуратная структура репозитория (REPO-014)

Запрос владельца 2026-11-19: в корне репозитория слишком много файлов и папок, на GitHub это выглядит неряшливо. Нужно разложить всё аккуратно, ничего не сломав. **Выполнено 2026-11-25** в «тихое окно» (открыты были только PR Dependabot). Решение — [ADR-0168](../adr/0168-tidy-repository-root.md).

## Корень сейчас

- **Видимые папки (10):** `clients`, `config`, `core`, `design`, `distro`, `docs`, `packaging`, `server`, `tools`, `LICENSES`.
- **Видимые файлы (8):** `AGENTS.md`, `Cargo.lock`, `Cargo.toml`, `LICENSE`, `README.md`, `README.en.md`, `REUSE.toml`, `rust-toolchain.toml`.
- **Скрытые:** `.cargo/`, `.claude/`, `.config/`, `.devcontainer/`, `.github/`, `.editorconfig`, `.env.example`, `.gitattributes`, `.gitignore`, `.justfile`, `.pre-commit-config.yaml`.

До переноса было 18 папок и 23 файла.

## Что куда переехало

| Было | Стало | Что поправлено |
|---|---|---|
| `CODE_OF_CONDUCT.md`, `CONTRIBUTING.md`, `SECURITY.md` | `.github/` | GitHub находит их там же, вкладки на главной сохраняются; ссылки в README, docs, `hygiene.yml` (lychee), REUSE |
| `CLAUDE.md` | `.claude/CLAUDE.md` | Claude Code читает оба места; ссылки в AGENTS.md и CONTRIBUTING; `.gitignore` не пускает в git локальные `.claude/worktrees/` и `.claude/settings.local.json` |
| `.gitleaks.toml`, `.gitleaksignore` | `.github/gitleaks/gitleaks.toml`, `.github/gitleaks/.gitleaksignore` | `--config` и `--gitleaks-ignore-path` в `hygiene.yml` и `.pre-commit-config.yaml` |
| `deny.toml` | `.cargo/deny.toml` | **отличие от плана** (`.config/deny.toml`): cargo-deny 0.20 сам ищет `deny.toml`, `.deny.toml` и `.cargo/deny.toml` рядом с `Cargo.toml` workspace, но не `.config/`. В `.cargo/` `cargo deny check` работает без `--config` у людей, агентов и в CI; в `deps.yml` canary копирует `.cargo/` |
| `mise.toml` | `.config/mise.toml` | mise читает его там сам; `mise trust` в `bootstrap.sh`/`.ps1` и Dev Container, `COPY` в `.devcontainer/Dockerfile` |
| `.xcode-version` | `.config/xcode-version` | `tools/bootstrap/bootstrap.sh` (единственный читатель) |
| `justfile` | `.justfile` | just находит скрытое имя сам; команды не меняются |
| `site/` (только `.gitkeep`) | удалён | сайт появится в SITE-001: `docs/site/` или отдельный репозиторий (DEC-001) |
| `spikes/android-player/` | удалён | спайк закончен (PLAY-AND-001); в ADR-0008 ссылка на историю git; Dependabot (pub, gradle) больше его не смотрит |
| `spikes/desktop-player/` | `tools/desktop-player-spike/` | **отличие от плана** (удалить): спайк ещё используется — Docker-образ для `pa_player/tool/headless-check.sh`, рецепт скриншотов прогресса (`docs/screenshots/progress/README.md`), задача PLAY-DSK-009. Скрипты спайка работают от своего каталога, перенос их не ломает |
| `data/anime-mappings/` | `tools/anime-mappings/` | `release-app.yml` (Linux и Android), `tools/release/precheck.sh`, ссылки в ядре, приложении, ADR и `THIRD-PARTY-NOTICES.md` |
| `tests/justfile` | `tools/qa/justfile` | модуль just по-прежнему называется `tests`: `just tests::media` и др. без изменений |
| `tests/media/` (README, манифест, `.gitignore`) | `tools/qa/media/` рядом со скриптами | пути в `generate.py`, `verify.py`, `fetch.py`, `library.py`, `manifest.py`, тестах `pa-probe` и `pa-trickplay`, `test-media.yml`; правила LFS в `.gitattributes` сужены до `synthetic/`, `external/`, `library/`, чтобы не задеть скрипты |

## Что осталось в корне и почему

| Файл / папка | Почему |
|---|---|
| `README.md`, `README.en.md`, `LICENSE` | GitHub показывает их на главной; лицензию ищет в корне |
| `LICENSES/`, `REUSE.toml` | спецификация REUSE требует корень |
| `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `.cargo/` | корень Cargo workspace (core, server, tools); rustup ищет toolchain вверх по дереву |
| `AGENTS.md` | AI-агенты (Codex, Cursor, Copilot и др.) читают его из корня |
| `config/` | **адрес вшит в выпущенные сборки**: `pa-remote-config` качает `https://raw.githubusercontent.com/DenisHumen/kinolune/main/config/...`. Перенос сломает удалённый конфиг у всех установленных версий |
| `.pre-commit-config.yaml` | pre-commit ищет только в корне |
| `.env.example` | так принято, его ищут люди |
| `.github/`, `.gitignore`, `.gitattributes`, `.editorconfig` | GitHub, git и редакторы читают из корня |

Перенос Cargo workspace в `core/` (чтобы убрать ещё `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `.cargo/`) **не делаем**: придётся менять все пути `target/`, codegen FRB, CI, кэши и скрипты сборки, а выигрыш маленький.

## Главная страница GitHub

- README: логотип, одна строка «что это», бейджи (лицензия, CI, последний релиз, платформы), скриншоты (компьютер, телефон, телевизор), блоки «Скачать», «Возможности», «Участвовать», «Пожертвовать», ссылка на английскую версию.
- **«About» и social preview — только владелец** (настройки репозитория): описание, сайт, темы (topics) и картинка превью ссылок (`design/brand/export/logo/lockup-dark.png` или скриншот главного экрана).

## После слияния

1. Вкладки GitHub (Code of conduct, Contributing, Security policy) на главной и в «Insights → Community standards».
2. `https://raw.githubusercontent.com/DenisHumen/kinolune/main/config/...` отдаёт конфиг (каталог не менялся).
3. Закрыть PR Dependabot по удалённому `spikes/android-player` (gradle).
4. Ветки, открытые до переноса, сливают `main`; на сервере сборки в старых клонах сгенерированные медиа лежат в `tests/media/` — перегенерировать (`just tests::media`) или задать `PA_TEST_MEDIA_DIR`.
5. Первый полный CI на `main` и пробный релиз (`release-app.yml`, `version=0.0.0-dryrun`) — в вечерний прогон.
