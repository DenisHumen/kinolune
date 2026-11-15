<!-- SPDX-License-Identifier: MPL-2.0 -->

# Kinolune для macOS (PKG-019)

Сборка для прямой загрузки из GitHub Releases: `kinolune-<версия>-macos.zip` с `Kinolune.app`,
universal (Apple silicon и Intel), macOS 12 и новее. Решения — ADR-NNNN.

## Как поставить (для владельца и пользователей)

Приложение подписано ad-hoc, без Apple Developer ID, и не нотаризовано (это PKG-008 и PKG-012),
поэтому Gatekeeper не пускает его при первом запуске («Не удаётся открыть Kinolune» или «Apple не может
проверить…»).

1. Скачайте `kinolune-<версия>-macos.zip` (из релиза или артефакта `macos` прогона CI) и распакуйте
   двойным щелчком. Перенесите `Kinolune.app` в «Программы».
2. Откройте один раз в обход Gatekeeper — любым способом:
   - щёлкните `Kinolune.app` правой кнопкой (или с Control) → «Открыть» → «Открыть» в диалоге;
   - на macOS 15 и новее кнопки «Открыть» в диалоге может не быть: «Системные настройки» →
     «Конфиденциальность и безопасность» → внизу «Всё равно открыть» рядом с сообщением о Kinolune;
   - или в Терминале: `xattr -dr com.apple.quarantine /Applications/Kinolune.app`.
3. Дальше приложение запускается как обычно.

macOS спросит разрешение на поиск устройств в локальной сети (общие папки, медиасерверы,
синхронизация) и на доступ к «Загрузкам», «Документам» и съёмным дискам, когда вы их добавите. Без
этих разрешений соответствующие функции не работают.

## Что внутри

| Компонент | Откуда | Где в бандле |
|---|---|---|
| Приложение Flutter (флейвор `full`) | `clients/flutter/app`, `flutter build macos --release` | `Contents/MacOS/Kinolune` |
| Rust-ядро `pa_frb` | Native Assets (`pa_core`), обе архитектуры | `Contents/Frameworks/pa_frb.framework` |
| libmpv (mpv `-Dgpl=false`) и FFmpeg (LGPL 2.1+) | наша сборка [`tools/mpv`](../../../tools/mpv/README.md), артефакты `libmpv-macos-arm64` и `libmpv-macos-x86_64` | `Contents/Frameworks/{Mpv,Avcodec,Avfilter,Avformat,Avutil,Swresample,Swscale}.framework` |
| libsmb2 (LGPL 2.1+) | [`tools/libsmb2`](../../../tools/libsmb2/README.md), артефакт `libsmb2-apple` | `Contents/Frameworks/libsmb2.1.dylib` |
| Лицензии | `LICENSE`, `THIRD-PARTY-NOTICES.md`, тексты libmpv/FFmpeg/libsmb2 | `Contents/Resources/licenses/` |

Готовые библиотеки media_kit для macOS (`media_kit_libs_macos_video`, архив libmpv-darwin-build) **не
используются**: мы не проверяли, что в них нет GPL-частей, а наша сборка проходит лицензионный гейт
`tools/mpv`. Вместо них приложение подключает локальный пакет
[`clients/flutter/packages/media_kit_libs_macos_video`](../../../clients/flutter/packages/media_kit_libs_macos_video)
с тем же именем: под этим именем его ищет podspec `media_kit_video`, чтобы собрать настоящий
видеовывод и слинковаться с `Mpv.framework`.

## Как собирается

CI: job `macos` в [`release-app.yml`](../../../.github/workflows/release-app.yml) (тег `app-v*` —
файл в релизе; ручной запуск — артефакты):

```bash
gh workflow run release-app.yml --ref <ветка> -f version=0.1.0-dev.1 -f platforms=macos
```

`platforms=macos` собирает только macOS (плюс libmpv и libsmb2 для Apple) и выкладывает артефакты
`macos` (zip) и `macos-smoke` (журнал и скриншот запуска).

Шаги (на Mac с Xcode, Flutter и rustup их можно повторить локально):

```bash
# 1. libmpv -> фреймворки в пакете (артефакты CI libmpv-macos-* или tools/mpv/build.sh macos-arm64)
tools/mpv/bundle.sh macos clients/flutter/packages/media_kit_libs_macos_video/macos/Frameworks \
  <libmpv-macos-arm64> [<libmpv-macos-x86_64>]
# 2. приложение
cd clients/flutter/app
CARGO_KINOLUNE_FLAVOR=full flutter build macos --release --dart-define=KINOLUNE_FLAVOR=full
# 3. libsmb2, лицензии, ad-hoc подпись, zip
packaging/app/macos/package.sh clients/flutter/app/build/macos/Build/Products/Release/Kinolune.app \
  0.1.0-dev.1 dist <libsmb2-macos-arm64> [<libsmb2-macos-x86_64>]
```

Без шага 1 `pod install` останавливается с подсказкой: podspec пакета требует `Mpv.framework`.
С одним артефактом (`macos-arm64`) фреймворки получаются только arm64 — тогда и приложение
собирайте для одной архитектуры.

## Проверки в CI

- `package.sh`: в `Mpv.framework` есть `-Dgpl=false` и нет `-Dgpl=true`, ни один Mach-O в бандле не
  ссылается на GPL/nonfree-библиотеки, ядро `pa_frb` на месте, подпись проходит
  `codesign --verify --deep --strict`, entitlements сохранены.
- `tools/flavor/check-artifacts.sh full` на zip.
- Смоук: распакованное приложение запускается, через 30 с процесс жив, `screencapture` снимает экран.

## Не сделано

- Нотаризация, Developer ID, hardened runtime, DMG, Sparkle/Homebrew — PKG-008 (нужен PKG-012).
- App Sandbox: выключен, пока у приложения нет security-scoped bookmarks для папок медиатеки
  (ADR-NNNN). Сборке для Mac App Store он понадобится.
- Аппаратное декодирование с GL-интеропом VideoToolbox: libmpv собран без Cocoa (`gl-cocoa`), видео
  декодируется через `videotoolbox-copy` или программно.
