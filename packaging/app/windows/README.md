<!-- SPDX-License-Identifier: MPL-2.0 -->

# Kinolune для Windows (PKG-020, PKG-022)

Две сборки для прямой загрузки из GitHub Releases, Windows 10 и 11, x64:

- **установщик** `kinolune-<версия>-windows-x64-setup.exe` (PKG-022, Inno Setup) — ставится для
  текущего пользователя без прав администратора в `%LOCALAPPDATA%\Programs\Kinolune`, ярлыки в
  «Пуске» и на рабочем столе, удаление — «Параметры → Приложения». Дальше приложение обновляется само:
  находит новый релиз, скачивает установщик, проверяет его по подписанным `SHA256SUMS` (minisign) и по
  «Перезапустить» (или при выходе) тихо ставит поверх и открывает новую версию;
- **портативный zip** `kinolune-<версия>-windows-x64.zip` — распакуйте и запускайте `Kinolune.exe` из
  любой папки; сам не обновляется, только сообщает о новой версии.

Решения — [ADR-0093](../../../docs/adr/0093-windows-app-packaging.md) (zip) и
[ADR-0109](../../../docs/adr/0109-desktop-installer-and-auto-update.md) (установщик и обновление).

## Установщик (PKG-022)

- Скрипт — [`kinolune.iss`](kinolune.iss), сборка — [`installer.sh`](installer.sh) из готового zip
  (те же файлы, что прошли проверки `package.sh`), Inno Setup — [`fetch-inno.sh`](fetch-inno.sh)
  (закреплены версия и SHA-256). Inno Setup — свободная программа под своей лицензией.
- Тихая установка: `kinolune-<v>-windows-x64-setup.exe /VERYSILENT /SUPPRESSMSGBOXES /NORESTART`;
  удаление: `%LOCALAPPDATA%\Programs\Kinolune\unins000.exe /VERYSILENT /SUPPRESSMSGBOXES /NORESTART`.
  Приложение при обновлении добавляет `/UPDATE=1` (подождать, пока оно закроется) и `/RELAUNCH=1`
  (запустить новую версию в конце).
- Медиатека, настройки и журналы (`%APPDATA%\app.kinolune`) при обновлении и удалении остаются.
- CI (`windows` в `release-app.yml`): тихая установка, проверка файлов, ярлыков и записи в
  «Приложениях», запуск 20 с, тихое удаление. `windows-update`: установленная версия `0.0.1-e2e.1`
  сама обновляется до `0.0.1-e2e.2` с локального тестового релиза и перезапускается новой версией.

## Как запустить (для владельца и пользователей)

1. Скачайте `kinolune-<версия>-windows-x64.zip` — из релиза или из артефакта `windows` прогона CI
   (Actions → прогон «Release app» → Artifacts; GitHub отдаёт артефакт как zip, внутри — наш zip).
2. Распакуйте **всю** папку в любое место, куда можно писать (например, `Документы\Kinolune`).
   `Kinolune.exe` нужны DLL и папка `data` рядом с ним.
3. Запустите `Kinolune.exe`. Сборка пока не подписана (подпись — PKG-005), поэтому SmartScreen может
   показать «Система Windows защитила ваш компьютер». Нажмите **«Подробнее»** → **«Выполнить в любом
   случае»** (англ. **More info** → **Run anyway**). Это нужно один раз для каждой скачанной версии.
   Можно и заранее: правый щелчок по zip → «Свойства» → галочка «Разблокировать» → OK, потом
   распаковать.

Данные (медиатека, настройки, журналы) — в `%APPDATA%\app.kinolune\Kinolune`, пароли сетевых
источников — в «Диспетчере учётных данных» Windows. Удаление папки программы их не удаляет.
Второй запуск `Kinolune.exe` не открывает вторую копию, а поднимает уже открытое окно.

Обновление портативной копии — скачать новый zip и распаковать поверх старой папки (или в новую);
установленная копия обновляется сама.

## Что внутри

| Компонент | Откуда | Где в папке |
|---|---|---|
| Приложение Flutter (флейвор `full`) | `clients/flutter/app`, `flutter build windows --release` | `Kinolune.exe`, `flutter_windows.dll`, `*_plugin.dll`, `data/` |
| Rust-ядро `pa_frb` | Native Assets (`pa_core`), `x86_64-pc-windows-msvc` | `pa_frb.dll` |
| libmpv (mpv `-Dgpl=false`) и FFmpeg (LGPL 2.1+) | наша сборка [`tools/mpv`](../../../tools/mpv/README.md), артефакт `libmpv-windows-x86_64` | `libmpv-2.dll`, `avcodec-*.dll`, `avfilter-*.dll`, `avformat-*.dll`, `avutil-*.dll`, `swresample-*.dll`, `swscale-*.dll` |
| ANGLE (OpenGL ES на Direct3D 11, BSD-3-Clause) | архив `ANGLE.7z` v1.0.1, закреплён SHA-256 в `mpv-libs.sh` | `libEGL.dll`, `libGLESv2.dll` |
| libsmb2 (LGPL 2.1+) | [`tools/libsmb2`](../../../tools/libsmb2/README.md), артефакт `libsmb2-windows-x86_64` | `libsmb2.dll` |
| Visual C++ runtime | Redist Visual Studio раннера | `msvcp140.dll`, `vcruntime140.dll`, `vcruntime140_1.dll` |
| Лицензии | `LICENSE`, `THIRD-PARTY-NOTICES.md`, тексты libmpv/FFmpeg/libsmb2/ANGLE | `licenses/`, `README-windows.txt` |

Готовые библиотеки media_kit для Windows (`media_kit_libs_windows_video`, архив
libmpv-win32-video-build) **не используются**: их лицензионный состав мы не проверяли. Вместо них
приложение подключает локальный пакет
[`clients/flutter/packages/media_kit_libs_windows_video`](../../../clients/flutter/packages/media_kit_libs_windows_video)
с тем же именем: он включает в CMake `MEDIA_KIT_LIBS_AVAILABLE` и кладёт нашу libmpv и ANGLE туда,
где их ждёт `media_kit_video`.

## Как собирается

CI: job `windows` в [`release-app.yml`](../../../.github/workflows/release-app.yml) (тег `app-v*` —
файл в релизе; ручной запуск — артефакты):

```bash
gh workflow run release-app.yml --ref <ветка> -f version=0.1.0-dev.1 -f platforms=windows
```

`platforms=windows` собирает только Windows (плюс libmpv и libsmb2 для `windows-x86_64`) и
выкладывает артефакты `windows` (zip) и `windows-smoke` (журналы и скриншот запуска).

Шаги (на Windows с Visual Studio 2022 «Desktop development with C++», Flutter, rustup и Git Bash
их можно повторить локально):

```bash
# 1. libmpv и ANGLE -> пакет (артефакт CI libmpv-windows-x86_64 или tools/mpv/docker-build.sh windows-x86_64)
packaging/app/windows/mpv-libs.sh clients/flutter/packages/media_kit_libs_windows_video/windows/libs \
  <libmpv-windows-x86_64>
# 2. приложение
cd clients/flutter/app
CARGO_KINOLUNE_FLAVOR=full flutter build windows --release --dart-define=KINOLUNE_FLAVOR=full
# 3. libsmb2, VC++ runtime, лицензии, zip
PA_VCREDIST_DIR="<VS>/VC/Redist/MSVC/<версия>/x64/Microsoft.VC143.CRT" \
packaging/app/windows/package.sh clients/flutter/app/build/windows/x64/runner/Release \
  0.1.0-dev.1 dist <libsmb2-windows-x86_64>
```

Без шага 1 CMake останавливается с подсказкой. `mpv-libs.sh` работает и на Linux (нужны `curl`,
`python3`, `cmake`).

## Проверки в CI

- `mpv-libs.sh`: артефакт — `windows-x86_64`, mpv `-Dgpl=false` (BUILDINFO и строка в DLL), FFmpeg
  «LGPL version 2.1 or later», все импортируемые `av*/sw*` DLL на месте; ANGLE и его LICENSE
  совпадают по SHA-256.
- `package.sh`: `libmpv-2.dll` в сборке — та же, что прошла гейт; в DLL FFmpeg нет
  `--enable-gpl/nonfree/version3`; ни один PE не импортирует GPL/nonfree-библиотеки; ядро на месте;
  все импорты разрешаются в папку или `System32`.
- `tools/flavor/check-artifacts.sh full` на `pa_frb.dll`.
- Смоук: распакованное приложение запускается, второй запуск выходит, через 30 с процесс жив, в
  журналах нет `Unhandled Exception`, снимок экрана.

## Не сделано

- Подпись кода и установщик с автообновлением — PKG-005 (решение владельца: SignPath или Certum).
- Своя сборка ANGLE, Windows на ARM64.
