<!-- SPDX-License-Identifier: MPL-2.0 -->

# ADR-NNNN: Windows-сборка Flutter-приложения: портативный zip x64 без подписи, наш LGPL libmpv и ANGLE в локальном пакете, VC++ runtime рядом с exe

- **Статус:** Предложен
- **Дата:** 2026-11-15
- **Решают:** мейнтейнер (владелец делегировал решения по сборкам); подпись кода и установщик — владелец (PKG-005: SignPath или Certum)
- **Связано:** `PKG-020`, `PKG-005`, `PKG-019`, `PLAY-002`, `PLAY-013`, `LIB-019`, `CORE-013`; [ADR-0012](0012-release-0-1-player-media-kit.md), [ADR-0037](0037-platform-secret-stores.md), [ADR-0089](0089-macos-app-packaging.md)

## Контекст и проблема

Владелец хочет скачать и попробовать Kinolune на Windows. Windows-машины у проекта нет: сборка и
проверка — только в GitHub Actions на раннерах `windows-*`. Подпись кода и установщик (MSIX, Inno
Setup или Velopack) — задача PKG-005, она ждёт сертификат (SignPath или Certum) от владельца.

Ограничения:

- Плеер — media_kit (ADR-0012). На Windows его плагин `media_kit_video` собирает настоящий
  видеовывод, только если CMake-опция `MEDIA_KIT_LIBS_AVAILABLE` включена, и берёт заголовки mpv и
  `libmpv.dll.a` из `${CMAKE_BINARY_DIR}/libmpv`, а ANGLE (EGL/GLES поверх Direct3D 11) — из
  `${CMAKE_BINARY_DIR}/ANGLE`. Готовый пакет `media_kit_libs_windows_video` скачивает чужую сборку
  libmpv (libmpv-win32-video-build, лицензионный состав не проверен) и архив ANGLE по MD5.
  В клиентах GPL запрещён, FFmpeg и mpv — только LGPL.
- Наша LGPL-сборка `windows-x86_64` уже есть (`tools/mpv`, mingw-w64 в Docker, `libmpv.yml`):
  `libmpv-2.dll`, FFmpeg отдельными DLL, `libmpv.dll.a`; libgcc/libstdc++/winpthread вшиты
  статически, DLL зависят только от системных. То же для libsmb2 (`libsmb2.dll`).
- Rust-ядро собирается хуком Native Assets только для таргетов из
  `core/crates/pa-frb/rust-toolchain.toml`. Ядро (`pa-secrets` — Credential Manager через
  `windows-native-keyring-store`, `pa-smb2` — `LoadLibrary("libsmb2.dll")`) уже собирается под
  Windows; `cargo check --target x86_64-pc-windows-gnu -p pa-frb --features full` проходит.

## Варианты

1. **Портативный zip, наш libmpv и прикреплённый ANGLE в локальном пакете, без подписи** — выбран.
2. Upstream `media_kit_libs_windows_video` — быстро, но чужая libmpv и противоречит PLAY-013.
3. Сразу установщик (MSIX/Inno Setup) — без подписи MSIX не ставится, а неподписанный установщик
   SmartScreen встречает так же, как exe; это работа PKG-005 вместе с подписью.
4. Своя сборка ANGLE из исходников (depot_tools, gn, ninja) — долго (часы CI) и отдельная задача;
   пока берём тот же бинарный ANGLE, что и media_kit, но с закреплённым SHA-256 и лицензией.

## Решение

- **Раннер** `clients/flutter/app/windows` (`flutter create`, Flutter 3.50): `Kinolune.exe`, иконка
  `design/brand/export/windows/app.ico`, VERSIONINFO (ProductName/FileDescription `Kinolune`,
  CompanyName `app.kinolune` — от него `path_provider` строит каталог данных
  `%APPDATA%\app.kinolune\Kinolune`, менять нельзя без миграции данных), окно 1280×720,
  **один экземпляр**: именованный mutex `Local\app.kinolune.Kinolune`; второй запуск поднимает
  окно первого и выходит (вторая копия на той же базе медиатеки не нужна).
- **libmpv:** локальный пакет `clients/flutter/packages/media_kit_libs_windows_video` (наш, MPL-2.0,
  без кода upstream) с тем же именем и `pluginClass`. Его CMake включает
  `MEDIA_KIT_LIBS_AVAILABLE`, копирует `windows/libs/libmpv` и `windows/libs/ANGLE` в каталоги, где
  их ждёт `media_kit_video`, и отдаёт DLL в `bundled_libraries` (Flutter кладёт их рядом с exe).
  Без `windows/libs` сборка останавливается с подсказкой. `windows/libs` готовит
  `packaging/app/windows/mpv-libs.sh` из артефакта `libmpv-windows-x86_64`: гейт (`target:
  windows-x86_64`, mpv `-Dgpl=false` в BUILDINFO и в самой DLL, FFmpeg «LGPL version 2.1 or later»),
  `libmpv-2.dll` и ровно те `av*/sw*` DLL, которые она импортирует (обход импорт-таблиц PE своим
  `pe_imports.py`, без dumpbin/objdump; `avdevice` не попадает), плоские заголовки mpv (media_kit
  включает `<client.h>`), лицензии и BUILDINFO. Скрипт лежит в `packaging/app/windows`, а не в
  `tools/mpv`: `tools/mpv/*.sh` входят в хэш рецепта libmpv.
- **ANGLE:** архив `ANGLE.7z` v1.0.1 (тот же, что у media_kit 1.0.11), закреплён SHA-256; берутся
  заголовки, import-библиотеки, `libEGL.dll`, `libGLESv2.dll` и то, что они импортируют из архива
  (сейчас ничего). `d3dcompiler_47.dll`, SwiftShader и `vulkan-1.dll` не кладём: первый есть в
  Windows 10+, остальные D3D11-бэкенду не нужны. LICENSE ANGLE (BSD-3-Clause) скачивается с
  закреплённого коммита и проверяется по SHA-256 (в репозитории чужих текстов лицензий нет).
- **libsmb2:** артефакт `libsmb2-windows-x86_64` → `libsmb2.dll` рядом с exe; `pa-smb2` находит его
  через порядок поиска DLL (каталог приложения первым). SMB на Windows работает так же, как на Linux.
- **Ядро:** в `rust-toolchain.toml` pa-frb добавлен `x86_64-pc-windows-msvc`; `pa_frb.dll` Native
  Assets кладёт рядом с exe.
- **Секреты:** ядро — Windows Credential Manager (DPAPI, `pa-secrets`, ADR-0037), Dart —
  `flutter_secure_storage` (тоже Credential Manager). `pa_platform_secrets` нужен только Android.
- **VC++ runtime:** Flutter-движок, плагины и ядро (MSVC) импортируют `msvcp140.dll`,
  `vcruntime140.dll`, `vcruntime140_1.dll`. Без VC++ Redistributable Windows их не имеет, поэтому
  `package.sh` кладёт их рядом с exe (app-local развёртывание, которое Microsoft разрешает для
  файлов из `VC\Redist`); источник — `PA_VCREDIST_DIR` (в CI — папка Redist Visual Studio раннера).
- **Упаковка** `packaging/app/windows/package.sh`: копия `Release/` в папку
  `kinolune-<версия>-windows-x64/`, libsmb2, VC++ runtime, `README-windows.txt` (SmartScreen:
  «Подробнее → Выполнить в любом случае»), `licenses/` (LICENSE, THIRD-PARTY-NOTICES, libmpv,
  libsmb2, ANGLE). Гейты: `libmpv-2.dll` побайтно та, что прошла `mpv-libs.sh`, и `-Dgpl=false`;
  в DLL FFmpeg нет `--enable-gpl/nonfree/version3`; ни один PE не импортирует GPL/nonfree-библиотеку
  и таких DLL в папке нет; `pa_frb.dll` на месте; на Windows каждый импорт каждого PE разрешается
  в папку или в `System32`. Zip — Python `zipfile` (одинаково в Git Bash и Linux).
- **CI:** job `windows` в `release-app.yml` на `windows-2025`. Тег `app-v*` — zip в релизе, строка в
  таблице загрузок и абзац про SmartScreen в заметках; исходники Windows-сборок libmpv и libsmb2
  входят в архивы соответствия LGPL. Ручной запуск с `platforms=windows` собирает только Windows
  (`libmpv.yml` и `libsmb2.yml` получили вход `windows_only`; Windows-срезы теперь входят и в
  релизные сборки, libmpv для Windows берётся из кэша `main`). Смоук: распакованный zip
  запускается, второй запуск должен выйти (один экземпляр), через 30 с процесс жив, в журналах нет
  `Unhandled Exception`, снимок экрана PowerShell (`CopyFromScreen`); журналы и снимок —
  артефакт `windows-smoke`. Флейвор ядра — `tools/flavor/check-artifacts.sh full` на `pa_frb.dll`.

### Последствия

- Хорошо: в папке только наши LGPL-сборки libmpv/FFmpeg/libsmb2 с открытым рецептом и гейтами,
  как на Linux, Android и macOS; запуск из любой папки, без прав администратора.
- Плохо: SmartScreen предупреждает при первом запуске каждой версии, пока нет подписи (PKG-005).
  Обновление — вручную (страница релиза, PKG-018 для Windows показывает ссылку).
- Плохо: ANGLE — чужой бинарник (BSD-3-Clause, без GPL), собранный автором media_kit; рецепта
  сборки у нас нет. Он закреплён по SHA-256, но воспроизводимость — следующий шаг.
- Плохо: релиз ждёт ещё один job (сборка ядра и плагинов MSVC, около 20–30 минут), сбой
  Windows-сборки останавливает весь релиз.
- Ограничение: libmpv собрана без `egl-angle`/`d3d11` интеропа для render API OpenGL, поэтому
  аппаратное декодирование — `d3d11va-copy`/`dxva2-copy` или программное; на 4K HEVC возможна
  высокая нагрузка CPU.

## Как проверяем соблюдение

- `release-app.yml`, job `windows`: гейты `mpv-libs.sh` и `package.sh`, проверка флейвора, смоук
  запуска со скриншотом (`docs/screenshots/progress/windows-build/`).
- CMake `media_kit_libs_windows_video` падает без `windows/libs` — сборка с чужой libmpv не
  получится незаметно.

## Открытые вопросы (владелец)

- Подпись кода (SignPath Foundation или сертификат Certum) и установщик (MSIX / Inno Setup /
  Velopack) с автообновлением — PKG-005.
- Своя сборка ANGLE из исходников (или переход на D3D11-интероп без ANGLE) — отдельная задача.
- Windows на ARM64 — после появления раннеров и сборки libmpv `windows-arm64`.
