<!-- SPDX-License-Identifier: MPL-2.0 -->

# ADR-NNNN: macOS-сборка Flutter-приложения: universal zip с ad-hoc подписью, наш LGPL libmpv во фреймворках, без App Sandbox

- **Статус:** Предложен
- **Дата:** 2026-11-15
- **Решают:** мейнтейнер (владелец делегировал решения по сборкам); подпись и нотаризация — владелец (PKG-008, PKG-012)
- **Связано:** `PKG-019`, `PKG-008`, `PKG-012`, `PLAY-002`, `PLAY-013`, `LIB-019`, `CORE-013`; [ADR-0011](0011-maintainer-decisions-release-0-1.md), [ADR-0012](0012-release-0-1-player-media-kit.md), [ADR-0037](0037-platform-secret-stores.md)

## Контекст и проблема

Владелец хочет попробовать Kinolune на своём Mac сейчас, до Apple Developer Program (PKG-012) и
нотаризации (PKG-008). Mac у проекта нет: ни у разработчиков, ни на сервере сборки. Всё, что
касается macOS, собирается и проверяется только в GitHub Actions на `macos-15`.

Ограничения:

- Плеер — media_kit (ADR-0012). На macOS его плагин `media_kit_video` собирает настоящий
  видеовывод, только если в `pubspec.lock` есть пакет `media_kit_libs_macos_video`, и линкуется с
  `-framework Mpv`. Dart-часть открывает `Mpv.framework/Mpv`. Готовый пакет скачивает архив
  libmpv-darwin-build, лицензионный состав которого мы не проверяли. В клиентах GPL запрещён, FFmpeg
  и mpv — только LGPL (CLAUDE.md).
- У нас есть своя LGPL-сборка libmpv для `macos-arm64` и `macos-x86_64` (`tools/mpv`, `libmpv.yml`)
  с лицензионным гейтом: `libmpv.2.dylib` и FFmpeg отдельными dylib (`@rpath/libavcodec.63.dylib`
  и т. д.), install name самой libmpv — абсолютный путь сборки.
- Rust-ядро собирается хуком Native Assets (`native_toolchain_rust`), который принимает только
  таргеты из `core/crates/pa-frb/rust-toolchain.toml`.
- Медиатека хранит пути к папкам между запусками; security-scoped bookmarks у приложения нет.

## Варианты

1. **Наш libmpv во фреймворках, universal, ad-hoc, без песочницы** — выбран.
2. Upstream `media_kit_libs_macos_video` — быстро, но лицензионный состав чужой сборки не проверен,
   и это противоречит PLAY-013 (на Android мы ушли от чужих бинарников по той же причине).
3. Только arm64 — меньше времени CI, но пришлось бы исключать x86_64 из сборки Flutter и всех
   подов (Flutter собирает release для macOS universal по умолчанию). Без Mac такую настройку не
   проверить, а universal бесплатен: оба среза libmpv уже собираются.
4. С App Sandbox — нужен для Mac App Store, но без security-scoped bookmarks приложение теряет
   доступ к папкам медиатеки после перезапуска.

## Решение

- **Раннер** `clients/flutter/app/macos` (`flutter create`, Flutter 3.50): bundle id `app.kinolune`
  (как `applicationId` на Android), `Kinolune.app`, macOS 12+, иконка из `design/brand` по сетке
  macOS (тело 824 из 1024), `NSLocalNetworkUsageDescription` и `NSBonjourServices` с типами
  `pa-discovery` (как у tvOS-приложения).
- **Entitlements:** `app-sandbox = false`. Ключи `network.client`, `network.server` (loopback-прокси,
  LAN-синхронизация, сопряжение) и `files.user-selected.read-only` записаны заранее для будущей
  сборки с песочницей; без песочницы они ни на что не влияют. `keychain-access-groups` нет: для него
  нужен Team ID. Секреты: Rust-ядро — в login keychain (`pa-secrets`, macOS native), Dart-хранилище
  (`flutter_secure_storage`) — тоже в login keychain (`usesDataProtectionKeychain: false`), иначе
  без entitlement запись отказывает и ключ TMDB живёт только до перезапуска.
- **libmpv:** локальный пакет `clients/flutter/packages/media_kit_libs_macos_video` (наш, MPL-2.0,
  без кода upstream) с podspec, который встраивает `Frameworks/*.framework`.
  `tools/mpv/bundle.sh macos` делает из артефактов `libmpv-macos-*` фреймворки `Mpv`, `Avcodec`,
  `Avfilter`, `Avformat`, `Avutil`, `Swresample`, `Swscale` (обход зависимостей `@rpath` от
  `libmpv.2.dylib`): склеивает срезы `lipo`, ставит install name
  `@rpath/<Name>.framework/Versions/A/<Name>`, переписывает ссылки, добавляет rpath
  `@loader_path/../../..`, кладёт лицензии и оба `BUILDINFO` в `Mpv.framework/Resources/licenses`,
  подписывает ad-hoc и создаёт `.symlinks/mpv/macos` — путь, где `media_kit_video` ищет
  `Mpv.framework`. Podspec без фреймворков останавливает `pod install` с подсказкой. `pa_player`
  открывает `Contents/Frameworks/Mpv.framework/Mpv` по пути (как `lib/libmpv.so.2` на Linux).
- **libsmb2:** артефакт `libsmb2-apple` (`macos-arm64`, `macos-x86_64`) склеивается в
  `Contents/Frameworks/libsmb2.1.dylib`; `pa-smb2` уже ищет его там
  (`@executable_path/../Frameworks/libsmb2.1.dylib`). В `targets.sh` libsmb2 исправлена проверка
  `check_smb2.c` на macOS (у clang из `xcrun --find` нет SDK по умолчанию, теперь `SDKROOT`).
- **Ядро:** в `rust-toolchain.toml` pa-frb добавлены `aarch64-apple-darwin` и `x86_64-apple-darwin`.
- **Упаковка** `packaging/app/macos/package.sh`: копия `.app`, libsmb2, `LICENSE`,
  `THIRD-PARTY-NOTICES.md`, README и лицензии библиотек в `Contents/Resources/licenses`, гейты
  (`-Dgpl=false` в `Mpv.framework`, никаких GPL/nonfree-библиотек в `otool -L` всех Mach-O, ядро на
  месте), переподпись ad-hoc изнутри наружу с сохранением entitlements, `codesign --verify --deep
  --strict`, `ditto` в `kinolune-<версия>-macos.zip`. Zip, а не DMG: DMG без подписи и нотаризации
  ничего не даёт, а распаковка zip двойным щелчком привычна; DMG — в PKG-008.
- **CI:** job `macos` в `release-app.yml`. Тег `app-v*` — zip в релизе, строка в таблице загрузок и
  инструкция «правый щелчок → Открыть / `xattr -dr com.apple.quarantine`» в заметках; исходники
  macOS-срезов libmpv и libsmb2 входят в архивы соответствия LGPL. Ручной запуск с
  `platforms=macos` собирает только macOS (`libmpv.yml` и `libsmb2.yml` получили вход
  `apple_only`; их Apple-job теперь идут и в релизных сборках, libsmb2 — только macOS-срезы).
  Смоук: распакованный zip запускается, через 30 с процесс жив, `screencapture` снимает экран;
  журнал, скриншот, `Podfile.lock` и изменения проекта раннера, которые вносят CocoaPods и SwiftPM,
  выкладываются артефактом `macos-smoke`.

### Последствия

- Хорошо: в бандле только наши LGPL-сборки с открытым рецептом, как на Linux и Android.
- Хорошо: владелец может скачать и запустить сборку без аккаунта Apple, Intel-Mac тоже работает.
- Плохо: Gatekeeper предупреждает при первом запуске; обход нужно объяснять (README, заметки
  релиза). Обновлять приложение — вручную, до PKG-008/PKG-018.
- Плохо: без песочницы и hardened runtime сборка не годится для Mac App Store и нотаризации как есть.
- Плохо: релиз ждёт ещё один macOS-job (сборка ядра под две архитектуры), и сбой macOS-сборки
  останавливает весь релиз.
- Ограничение: libmpv собран без Cocoa (`gl-cocoa`), поэтому GL-интероп VideoToolbox недоступен —
  декодирование через `videotoolbox-copy` или программное; на 4K HEVC возможна высокая нагрузка CPU.
- Ограничение: без ad-hoc переподписи после `install_name_tool` arm64 macOS не загрузит
  фреймворки; переподпись встроена в `bundle.sh` и `package.sh`.

## Как проверяем соблюдение

- `release-app.yml`, job `macos`: гейты `package.sh`, `tools/flavor/check-artifacts.sh full`,
  смоук запуска со скриншотом (`docs/screenshots/progress/macos-build/`).
- `tools/mpv/bundle.sh macos` падает, если `Mpv.framework` ссылается на `@rpath/lib*.dylib` (ссылка
  не переписана) или симлинк для `media_kit_video` не ведёт к фреймворку.

## Открытые вопросы (владелец)

- Developer ID, нотаризация, hardened runtime и DMG (PKG-008 после PKG-012).
- Сборка для Mac App Store: песочница плюс security-scoped bookmarks для папок медиатеки.
