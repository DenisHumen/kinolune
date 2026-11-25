<!-- SPDX-License-Identifier: MPL-2.0 -->

# ADR-0168: Корень репозитория — только то, что обязано там лежать

- **Статус:** Принят (запрос владельца 2026-11-19, выполнен 2026-11-25 в «тихое окно»)
- **Дата:** 2026-11-25
- **Решают:** владелец проекта, мейнтейнер
- **Связано:** REPO-014, [план и ограничения](../dev/repo-layout.md), [ADR-0006](0006-public-monorepo-layout-ownership.md) (раскладка и владение, эта запись дополняет её таблицу каталогов), ROADMAP §0.4–0.5

## Контекст и проблема

На главной странице GitHub было 18 папок и 23 файла, из них половина — служебные конфиги и почти пустые каталоги (`site/` с одним `.gitkeep`, законченные `spikes/`, `data/` и `tests/` с одним подкаталогом). Владелец попросил разложить всё аккуратно, ничего не сломав: выпущенные сборки, релизный и дистрибутивный CI, страницу прогресса, Dependabot, REUSE и рабочие процессы агентов.

## Решение

Каждый файл переносим только туда, где его инструмент находит конфиг сам или где путь меняется в одном-двух местах. Если перенос что-то ломает у пользователей или в выпущенных сборках — не переносим.

| Было | Стало | Почему работает |
|---|---|---|
| `CODE_OF_CONDUCT.md`, `CONTRIBUTING.md`, `SECURITY.md` | `.github/` | GitHub ищет их в корне, `.github/` и `docs/`; вкладки сохраняются |
| `CLAUDE.md` | `.claude/CLAUDE.md` | Claude Code читает оба места |
| `.gitleaks.toml`, `.gitleaksignore` | `.github/gitleaks/gitleaks.toml`, `.github/gitleaks/.gitleaksignore` | `--config` и `--gitleaks-ignore-path` в `hygiene.yml` и `.pre-commit-config.yaml` |
| `deny.toml` | `.cargo/deny.toml` | cargo-deny сам ищет `.cargo/deny.toml` рядом с `Cargo.toml` workspace, `cargo deny check` работает без флагов; рядом уже лежит `.cargo/audit.toml` |
| `mise.toml` | `.config/mise.toml` | mise сам читает `.config/mise.toml` |
| `.xcode-version` | `.config/xcode-version` | читает только `tools/bootstrap/bootstrap.sh` |
| `justfile` | `.justfile` | just сам находит скрытое имя; команды не меняются |
| `site/` (только `.gitkeep`) | удалён | сайт появится в SITE-001 (`docs/site/` или отдельный репозиторий) |
| `spikes/android-player/` | удалён | спайк закончен (PLAY-AND-001); история в git |
| `spikes/desktop-player/` | `tools/desktop-player-spike/` | ещё нужен: Docker-образ для `pa_player/tool/headless-check.sh`, скриншоты прогресса, задача PLAY-DSK-009 |
| `data/anime-mappings/` | `tools/anime-mappings/` | путь в `release-app.yml` и `precheck.sh` обновлён |
| `tests/justfile`, `tests/media/` | `tools/qa/justfile`, `tools/qa/media/` (рядом со скриптами) | модуль just по-прежнему называется `tests`: `just tests::media` не меняется |

**Остаются в корне:** `README.md`, `README.en.md`, `LICENSE`, `LICENSES/`, `REUSE.toml` (требование REUSE и GitHub), `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `.cargo/` (корень workspace, rustup), `AGENTS.md` (агенты ищут в корне), `config/` (адрес `raw.githubusercontent.com/.../main/config/...` вшит в выпущенные сборки), `.pre-commit-config.yaml` (pre-commit ищет только в корне), `.env.example`, `.editorconfig`, `.gitattributes`, `.gitignore`.

Перенос Cargo workspace в `core/` отклонён, как и в плане: много путей (`target/`, FRB codegen, CI, кэши), мало пользы.

## Последствия

- Хорошо: в корне 9 видимых папок (`clients`, `config`, `core`, `design`, `distro`, `docs`, `packaging`, `server`, `tools`) плюс `LICENSES`, и 8 видимых файлов; служебное — в скрытых `.github/`, `.config/`, `.cargo/`, `.claude/`.
- Плохо: внешние ссылки `blob/main/CONTRIBUTING.md` и `blob/main/SECURITY.md` GitHub не перенаправляет (вкладки репозитория работают). Сгенерированные тестовые медиа теперь живут в `tools/qa/media/{synthetic,external,library}`: старые клоны с `tests/media/` нужно перегенерировать (`just tests::media`) или указать `PA_TEST_MEDIA_DIR`.
- Открытые ветки и PR, созданные до переноса, сливают `main` и правят пути у себя.
