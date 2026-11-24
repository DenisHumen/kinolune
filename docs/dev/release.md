# Выпуск приложения: precheck → dry_run → тег

Порядок выпуска Kinolune (PKG-025, решение владельца после сбоя alpha.17: скрипт без права на запуск нашёлся только через час сборки на GitHub). Тег ставится только когда прошли обе проверки. Сборка и публикация — `.github/workflows/release-app.yml`, шаги владельца (ключи, секреты) — [packaging/app/README.md](../../packaging/app/README.md).

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
