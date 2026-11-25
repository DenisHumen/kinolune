# Выпуск приложения: precheck → dry_run → тег

Порядок выпуска Kinolune (PKG-025, решение владельца после сбоя alpha.17: скрипт без права на запуск нашёлся только через час сборки на GitHub). Тег ставится только когда прошли обе проверки. Сборка и публикация — `.github/workflows/release-app.yml`, шаги владельца (ключи, секреты) — [packaging/app/README.md](../../packaging/app/README.md).

## 0. Release PR: версия и changelog (release-please, REPO-010)

`.github/workflows/release-please.yml` на каждом push в main открывает или обновляет по одному release PR на компоненту: `app` (`clients/flutter/app`), `core`, `server`, `tvos` (`clients/apple/TVApp`), `distro`. Настройки — `release-please-config.json`, текущие версии — `.release-please-manifest.json`.

- Заголовок: `chore(release): release app 0.1.0-alpha.22`. В PR — новая версия в манифесте и раздел в `CHANGELOG.md` компоненты (`clients/flutter/app/CHANGELOG.md` и т. д.).
- Версии — alpha-серия (`versioning: prerelease`, `prerelease: true`, `prerelease-type: alpha`): `feat`, `fix` и даже breaking change на 0.x увеличивают только номер alpha (`0.1.0-alpha.21` → `0.1.0-alpha.22`). Другую версию задаёт строка `Release-As: 0.1.0-beta.1` в теле коммита или правка манифеста.
- В changelog попадают `feat`, `fix`, `perf`, `revert` (по коммитам на main, то есть по заголовкам смёрженных PR); `docs`, `ci`, `chore`, `test`, `refactor`, `build`, `style` скрыты. Коммит идёт в changelog той компоненты, чьи файлы он меняет: изменения ядра — в `core/CHANGELOG.md`, не в `app`.
- **Тег и GitHub Release release-please не создаёт** (`skip-github-release`), поэтому merge release PR ничего не собирает и не публикует: `release-app.yml` запускается только тегом `app-v*`, который ставит владелец (шаг 3).
- PR от `GITHUB_TOKEN` не запускает CI. Чтобы прогнать проверки на release PR — закрыть и снова открыть его.
- Нужна настройка репозитория «Allow GitHub Actions to create and approve pull requests» (Settings → Actions → General), без неё workflow падает на создании PR.

Выпуск по команде владельца:

1. Смёржить release PR нужной компоненты — в main появятся новая версия в манифесте и `CHANGELOG.md`.
2. Взять раздел этой версии из `CHANGELOG.md` за основу `notes.md` (первая строка — заголовок, дальше «Что нового»; для приложения стоит добавить заметное из `core/CHANGELOG.md`).
3. Шаги 1–3 ниже на коммите merge, тег — ровно той версии, что в release PR (`app-v0.1.0-alpha.22`).

Пока смёрженный release PR не помечен как выпущенный (`autorelease: tagged`), release-please не открывает новые release PR. Workflow сам переставляет метку `autorelease: pending` → `autorelease: tagged`, как только на origin появляется тег из заголовка PR. Если тег не будет ставиться, снять метку вручную: `gh pr edit <N> --remove-label "autorelease: pending" --add-label "autorelease: tagged"`.

Выпуск без release PR (тег прямо с main) сбивает манифест: следующий release PR снова предложит ту же версию. Тогда поправить версию в `.release-please-manifest.json` отдельным PR.

## 1. precheck на сервере сборки (10–20 минут)

```sh
git pull                                   # отдельный клон только для precheck
tools/release/precheck.sh 0.1.0-alpha.19 --notes notes.md
```

`notes.md` — текст будущего аннотированного тега: первая строка — заголовок, дальше «Что нового» (тот же файл потом пойдёт в `git tag -a -F`). Без `--notes` берётся сообщение тега `app-v<версия>`, если он уже есть локально.

Что проверяет — то же, что `release-app.yml`, теми же скриптами `packaging/` и `tools/`:

| Этап | Что |
|---|---|
| general | права на запуск у всех скриптов, которые CI вызывает по пути (`tools/release/check_ci_exec.py`); версия semver; тега ещё нет на origin; заметки (лёгкий тег без текста — ошибка); незакоммиченные изменения — предупреждение |
| native | наши LGPL libmpv (готовые сборки из выпуска `prebuilt`, как в CI) и libsmb2 (сборка из `tools/libsmb2`) для Linux x86_64, Android arm64-v8a и armeabi-v7a; проверки лицензий |
| assets | шрифты, база аниме (META-030) |
| linux | `flutter build linux` (full), tar.gz + AppImage, .deb + .rpm, проверка бандла (libmpv/libsmb2 из бандла, лицензии, флейвор ядра) |
| android | APK arm64-v8a и armeabi-v7a, подпись одноразовым ключом через `sign.sh`, проверка лицензий нативных библиотек, `REQUEST_INSTALL_PACKAGES` |
| assemble | подпись .rpm одноразовым ключом GPG через `import-key.sh`, лицензии и LGPL-исходники, `SHA256SUMS` с подписью одноразовым ключом minisign, заметки релиза из шаблона `release-app.yml`, ожидаемые файлы и ссылки в заметках |

Итог — «можно выпускать» или список сломанных этапов с выдержкой из журнала. Журналы и файлы: `~/.cache/kinolune-precheck/runs/<время>-<коммит>/` (последние пять запусков).

Устройство:

- Сборки идут в контейнере (`tools/release/precheck.Dockerfile`, Ubuntu 24.04 как у раннера) от имени пользователя; Flutter, Rust (rustup), Java и Android SDK — с хоста: `flutter`, `rustup`, `java` в `PATH`, задан `ANDROID_HOME`. Нужны `docker`, `git`, `curl`. Телефоны и `adb` не трогает.
- Кэши между запусками: Cargo, pub, Gradle, libmpv/libsmb2 и их исходники — в `--cache` (по умолчанию `~/.cache/kinolune-precheck`); `build/` и `.dart_tool/` приложения — в самом клоне. Первый запуск дольше (образ, Gradle, ядро на Rust для трёх целей).
- Если рецепт `tools/mpv` изменился и `prebuilt.yml` на main ещё не опубликовал сборку, этап native скажет, какой командой собрать её самому (около часа).
- `--only general,linux` — отдельные этапы; `--keep-going` — не останавливаться на первой ошибке; `--jobs N` — ограничение CPU (по умолчанию 4, сервер общий).
- Нет здесь: Windows и macOS (нужны их раннеры), тест обновления «alpha.N → alpha.N+1» и установка пакетов в контейнерах `linux-packages` — это проверяет dry_run.

## 2. dry_run на GitHub (Windows и macOS)

```sh
tools/release/precheck.sh 0.1.0-alpha.19 --notes notes.md --github       # запустить и не ждать
tools/release/precheck.sh 0.1.0-alpha.19 --notes notes.md --github-wait  # дождаться
```

После успешного precheck запускает `release-app.yml` через `workflow_dispatch` (это и есть dry run: собирается всё, ничего не публикуется) с `platforms=windows` и `platforms=macos` на текущей ветке и печатает ссылки. Нужен `gh auth login`. То же вручную: `gh workflow run release-app.yml -f version=0.1.0-alpha.19 -f platforms=windows` (и `macos`; `all` — все платформы).

## 3. Тег

Только когда precheck сказал «можно выпускать» и оба прогона dry_run зелёные:

```sh
git tag -a app-v0.1.0-alpha.19 -F notes.md && git push origin app-v0.1.0-alpha.19
```

## Права на запуск в CI

Тот же `tools/release/check_ci_exec.py` работает в Hygiene (`exec-bits`) на каждом PR: скрипт, который workflow вызывает по пути, должен быть в git с режимом `100755` (`git update-index --chmod=+x <файл>`).
