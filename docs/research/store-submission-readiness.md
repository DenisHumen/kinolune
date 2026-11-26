<!-- SPDX-License-Identifier: MPL-2.0 -->

# Исследование публикации Kinolune в магазинах приложений

**Срез репозитория и требований:** 2026-11-26, `origin/main` `0828d1a6`
**Назначение:** план подготовки и подачи приложения, не юридическое заключение и не обещание одобрения. Официальные правила меняются: перед подачей нужно повторно сверить требования платформ.

## Краткий вывод

По ROADMAP целевые каналы — Apple App Store для iPhone/iPad и Apple TV, Mac App Store, Google Play для телефонов/планшетов и Android TV, Microsoft Store, F-Droid и Flathub. Это охват уже записанных целей проекта; Amazon Appstore и Samsung Galaxy Store в списке целей не указаны. Несколько платформенных приложений и store-пакетов ещё не готовы к отправке.

В репозитории правильно задуманы флейворы `store/full`: магазины получают сборку без архивации Twitch и внешних ссылок на донаты; F-Droid/Flathub — `full`, но для F-Droid отключается проприетарный Cast SDK. Это полезная основа, но она не заменяет разрешения на сторонние сервисы, законченные платформенные клиенты, production-сборки, подписи и испытания на устройствах.

**Текущие артефакты не следует загружать в production.** Это не означает, что продукт принципиально не подходит магазинам: главные препятствия решаемы, но часть требует решений владельца или подтверждений правообладателей.

### Оценка вероятности

Проценты ниже — экспертная оценка, не статистика. Они отвечают на вопрос: **каков шанс, что первая полная подача пройдёт после реализации ROADMAP, закрытия технических и правовых блокеров и подготовки карточки?** Они не оценивают текущие сборки. Если отправить артефакты на дату среза, у большинства каналов шанс одобрения практически нулевой: нет готового релизного пакета или требуемой функциональности.

| Канал | Шанс первой полной подачи | Почему | Основной риск |
|---|---:|---|---|
| Apple App Store — iPhone/iPad | **55%** | Есть общая Flutter/Rust-кодовая база и store-флейвор; нет iOS-проекта и Apple-плеера для релиза | Права на сторонние сервисы/контент; незавершённая Apple-реализация |
| Mac App Store | **50%** | Есть macOS-версия для прямого распространения, но sandbox выключен | Sandbox-путь к медиатеке и лицензии нативных библиотек |
| Apple App Store — tvOS / Apple TV | **45%** | Есть собираемый SwiftUI-каркас, нет готовой медиатеки и TV-сценариев | До TV-006…011 нет законченного продукта для review |
| Google Play — телефон/планшет | **70%** | Есть Android-клиент, store AAB в CI и compile-time gate | Проверочный AAB без libmpv; подпись и API target |
| Google Play — Android TV | **50%** | В manifest есть Leanback launcher, баннер и optional touchscreen | Не завершён TV UX; 16 KB для native-библиотек не доказано |
| Microsoft Store | **80%** | Есть Windows-клиент и установщик; MSIX и некоторые Win32 installers принимаются | Нет отдельного store-пакета/конвейера и сертификации |
| Flathub | **55%** | Открытая Linux GUI-программа подходит по типу | Нет Flatpak manifest; проект очень молодой, а Flathub требует стабильный выпуск и осмысленную историю разработки |
| F-Droid | **40%** | MPL-2.0, исключение Cast SDK и Twitch anti-feature предусмотрены | Flutter/Rust/FFmpeg source-build; требование к истории разработки |

**Шанс пройти все каналы с первой полной подачи:** ориентировочно **5–15%**. При итерациях после замечаний магазинов оценка eventual-публикации во всех каналах — **25–40%**, если получены необходимые разрешения и закрыты обязательные работы. На оценку сильнее всего влияют ответы по TMDB/Twitch, совместимый Apple native stack и source-build для F-Droid. Проценты — ориентир для планирования, не гарантия результата.

Погрешность каждой оценки велика — ориентировочно **±15–20 процентных пунктов**: неизвестны решения будущих ревьюеров, аккаунты разработчика и ответы правообладателей.

## Состояние проекта по репозиторию

Проверены `docs/STRATEGY.md`, `docs/ROADMAP.md`, ADR-0007/0011/0041/0153, `docs/dev/flavors.md`, `docs/legal/`, Android manifest/Gradle и workflow, Apple-проекты/entitlements и упаковщики.

### Что уже заложено

- [ADR-0007](../adr/0007-store-full-build-flavors.md) и [описание флейворов](../dev/flavors.md): `store` — Apple, Google Play, Microsoft Store; `full` — GitHub Releases, F-Droid, Flathub, AUR, winget, ISO и Docker.
- `store` должен исключать Twitch archive/download и ссылки на донаты. F-Droid использует `full` без Cast SDK.
- `CORE-013` и `tools/flavor/check-artifacts.sh` проверяют store/full-содержимое Rust, Android AAB и Apple framework.
- Android AAB из `.github/workflows/flavors.yml` — только CI-проверка; он собирается с `-Pkinolune.allowMissingLibmpv=true`, поэтому не подтверждает работу воспроизведения.
- В `clients/flutter/app/android/app/build.gradle.kts` release-signing по умолчанию назначает debug-ключ. Для Play нужен отдельный pipeline с постоянным upload key и Play App Signing. GitHub Release — full build, а не артефакт Google Play.

### Текущие блокеры по платформам

- **iOS/iPadOS:** на проверенном HEAD отсутствует `clients/flutter/app/ios` с проектом приложения. Apple-плеер и интеграция Flutter не завершены (`PLAY-APL-002…013`). Релизный Apple license gate намеренно отвергает текущий MPVKit: его FFmpeg собран с LGPL-3.0, тогда как политика проекта требует LGPL-2.1. Нужна собственная совместимая сборка (`PLAY-APL-013`).
- **macOS:** GitHub Release приложение есть, но `Runner/Release.entitlements` выставляет `com.apple.security.app-sandbox = false`; комментарий сообщает, что медиатека хранит обычные пути без security-scoped bookmarks. Приложение сейчас нацелено на macOS 12. Mac App Store требует sandbox и пакет Xcode, поэтому текущий DMG для прямой установки не подходит как MAS-пакет.
- **tvOS:** `clients/apple/TVApp` пока SwiftUI-каркас с тестами флейвора. Основные приложение и TV-сценарии — незавершённые `TV-006…011`: медиатека, пультовая навигация, источники, player OSD, Top Shelf и настройки.
- **Android:** в manifest уже есть Leanback launcher и `touchscreen required=false`. Проверочный AAB собирается без libmpv и не является подписанной production-сборкой. `targetSdk` наследуется из Flutter toolchain, его значение надо проверить в фактическом AAB.
- **Android TV:** задачи `TV-002…004` не закрыты: D-pad/focus, TV-компоненты, профиль производительности. Помимо AAB и 32-/64-bit Google требует поддержку 16 KB страниц; Rust, FFmpeg/libmpv, libsmb2 и прочие `.so` требуют проверки.
- **Microsoft Store:** открыты `PKG-005` (упаковка/подпись) и `PKG-007` (Store). Текущий Windows installer для прямого распространения не равен готовому store-пакету.
- **Flathub:** нет Flatpak manifest и AppStream metadata (`PKG-003`).
- **F-Droid:** `PKG-010` не закрыт. Inclusion Policy просит минимум три месяца активной разработки, если исключение не применимо. История git начинается 2026-11-13; на срез прошло 13 дней. При непрерывной активности обычный путь к включению откроется после 2027-02-13.

### Владелец, приватность и права

- `DEC-007` ещё открыт: не определены аккаунты публикации и юрисдикция. Имя продавца Apple зависит от типа аккаунта; для организации требуются подтверждённое юрлицо, D‑U‑N‑S Number и полномочия подписанта.
- `docs/legal/privacy-policy.md` обозначена как черновик: в тексте осталось рабочее имя `project_alpha`, нет установленного публичного URL, privacy-contact email отмечен как «добавить перед публикацией». `docs/legal/store-privacy.md` — черновик ответов, а не заполненные формы консолей.
- `LEGAL-001` (письменный ответ TMDB) и `LEGAL-006` (Twitch ToS/Developer Agreement/client ID) остаются owner-задачами. Apple требует подтверждать права на сторонний контент/сервисы и запрещает загрузку/сохранение медиа без явного разрешения источника. Выключить Twitch downloads в store build необходимо, но этого недостаточно для подтверждения права на playback.
- `LEGAL-009` (полный аудит лицензий) не закрыт. Apple release gate сейчас намеренно не проходит LGPL-проверку. Для Linux/F-Droid/Flathub надо проверить распространение каждой библиотеки, набора данных, шрифта и иконки.
- `DEC-004` пока закрепляет рабочее имя Kinolune до ручной проверки товарного знака. В `ADR-0007` статус `DEC-003` обозначен ожидающим решения, хотя `ADR-0011` уже решил убрать донаты из первых store-релизов; эту устаревшую формулировку стоит согласовать.

## Общий чек-лист для подачи

### Аккаунты и право публикации

1. Зафиксировать владельца/юрлицо, страну, контакт, ответственного за платежи (если они появятся) и права на название, код, библиотеки и распространяемый контент.
2. Создать аккаунты заранее и завершить KYC. Apple Developer Program — **$99 в год**; организация проходит отдельную проверку. Google Play Console — **$25 разово**. Для новых личных Play-аккаунтов, созданных после 2023-11-13, до production нужен closed test: 12 testers должны непрерывно быть opted-in 14 дней; правило зависит от типа и даты аккаунта. Microsoft Partner Center account нужен для публикации.
3. Проверить Google developer verification и регистрацию package name `app.kinolune`. Поскольку APK уже распространяются вне Play, для регистрации может потребоваться доказательство контроля подписи.
4. Не хранить ключи подписи, API keys, сертификаты, Apple API key или Play service credentials в репозитории. Иметь безопасные резервные копии и план ротации.

### Сборка и материалы

1. Для каждого магазина собрать отдельный подписанный production-артефакт с постоянным bundle/application ID, монотонной версией и обновлением поверх предыдущего релиза. Store/full pipelines разделить.
2. Зафиксировать матрицу платформа × архитектура × минимальная ОС × флейвор × SDK/API × подпись × native libs × privacy declaration. Проверять финальный upload, а не промежуточный CI artifact.
3. Прогнать smoke и UI-тесты на целевых симуляторах и физических устройствах: старт, навигация, воспроизведение, разрешения, фон/сон, доступность, языки, стабильность и обновление.
4. Обеспечить ревьюеру доступ ко всем функциям. Если нужен вход или устройство — подготовить демо-режим/тестовые данные и инструкции без личных токенов. `PKG-013` уже предлагает короткие CC BY-ролики для демо; при включении проверить источник, лицензию и атрибуцию.
5. Заполнить карточку: имя, описание, категория, возрастной рейтинг/content rating, availability/pricing, privacy/support URL, контакт, screenshots/video, иконки, языки ru/en/uk, лицензии/атрибуции. Скриншоты — от фактической версии для нужной платформы.
6. Проверить, что описание не обещает функций, которых нет в store flavor, и не создаёт впечатления аффилиации с TMDB/Twitch/Apple/Google.
7. Запустить flavor/license gates именно по upload-артефакту.

### Приватность, закон и безопасность

- Завершить и опубликовать privacy policy: данные, цели, получатели, сроки хранения, права и контакт должны соответствовать коду. Инвентарь в репозитории полезен, но политика сейчас черновая.
- Заполнять Apple App Privacy и Google Play Data safety только после анализа конечного бинарника и всех SDK. Не копировать готовые ответы из `store-privacy.md` механически: Apple смотрит, доступны ли переданные данные дольше реального обслуживания запроса; Google учитывает ephemeral processing, user-initiated transfer и service providers. Проверить реальные retention внешних API и фактический состав сборки.
- Google требует privacy policy ссылку и в Play Console, и внутри приложения; URL должен быть публичным, доступным глобально, на обычной веб-странице (не PDF), содержать идентификацию приложения/разработчика, контакт приватности, типы данных/получателей, безопасность, хранение и удаление. Если позже появится аккаунт Kinolune, Google требует удалить его как из приложения, так и через веб-путь.
- Проверить Apple `PrivacyInfo.xcprivacy`, Required Reason API, privacy manifests/signatures перечисленных SDK и export compliance для криптографии. В проверенном дереве `PrivacyInfo.xcprivacy` не найден.
- Заполнить возрастные анкеты по фактическим функциям/контенту. Приложение индексирует пользовательские медиа и показывает внешние метаданные/стримы — рейтинг нельзя угадывать.
- Документировать разрешения на TMDB/Twitch, трейлеры, субтитры, API, обложки, шрифты и демонстрационные медиа. Физическое исключение функции из store build снижает риск только по этой функции; спорные вопросы требуют владельца/юриста.

## Требования и путь подачи по каналам

### Apple App Store — iPhone и iPad

**Потребуются:** Apple Developer Program; iOS/iPadOS target; Xcode archive; App Store Connect record; подписанный build; privacy policy/App Privacy; возрастная анкета и export compliance; скриншоты iPhone/iPad; TestFlight; Review Notes.

**Технически:** создать iOS app project, интегрировать Apple player bridge, завершить `PLAY-APL-002…013`, собрать собственные LGPL-2.1 FFmpeg/libmpv и пройти аудит SDK/разрешений.

**Подача:** Xcode Organizer/Transporter либо `xcodebuild` + App Store Connect API key → TestFlight → выбрать build, заполнить анкеты/локализации → Submit for Review. Сейчас Apple-релизного пакета нет.

**Риски:** право на стороннее воспроизведение (App Review Guidelines 5.2.2–5.2.3), полнота/стабильность (2.1), точность privacy label, рабочий demo path и функциональность. Нужна тестовая медиатека без личных файлов владельца.

### Mac App Store

Нужен отдельный macOS build через App Store Connect, собранный/упакованный средствами Xcode. **MAS требует App Sandbox** и не принимает текущую конфигурацию direct DMG как есть; приложение должно быть self-contained `.app`, без стороннего installer, незапрошенного autostart и фонового процесса.

Реализовать security-scoped bookmarks для папок пользователя; проверить локальную медиатеку, LAN, SMB/NFS/WebDAV и открытие файлов в sandbox. Пересобрать framework/dylib под архитектуры и пройти license check.

**Подача:** App Store Connect platform macOS → Xcode archive/signing → тестирование (TestFlight for Mac, если доступно для конфигурации) → review. Developer ID/notarization — отдельный канал прямой дистрибуции, не замена MAS.

### Apple App Store — tvOS / Apple TV

Нужны tvOS build/card, icon, Top Shelf, screenshots, privacy/export/age формы, TestFlight и инструкции. Сценарии должны работать с Siri Remote; Apple требует, чтобы app не требовал другого контроллера без ясного уведомления.

Закрыть `TV-006…011`: storage/library/detail, источники/pairing, player OSD, Top Shelf, настройки/профили. Проверять focus на симуляторе и физическом Apple TV, а не только сборку.

**Подача:** подписанный tvOS archive в App Store Connect, отдельные tvOS metadata/screenshots и Review Notes. Сейчас в `TVApp/project.yml` указан tvOS 17 и store flavor.

### Google Play — телефоны/планшеты

**Пакет:** подписанный Android App Bundle (`.aab`), upload key и Play App Signing, постоянный package ID `app.kinolune`, version code/name, отдельные store/full pipelines. Не загружать текущий CI `store-android`: он собран с `allowMissingLibmpv` и не подтверждает playback.

**Консоль и listing:** App content, privacy, Data safety, target audience, content rating, app access, permissions/background services, icon/screenshots/feature graphic, локали и страны. С 2026-08-31 новая app/update должна target API 36+. Gradle наследует `targetSdk` из Flutter toolchain; надо проверить эффективное значение в AAB и повторно сверить актуальное правило при отправке.

**Тестирование:** internal → closed/open tracks согласно аккаунту и политике. Для новых личных аккаунтов после 2023-11-13 closed test требует 12 opted-in testers непрерывно 14 дней до production access.

**Разрешения Play, требующие отдельного обоснования:** manifest запрашивает `READ_MEDIA_VIDEO`. Для Android 13+ Google разрешает `READ_MEDIA_VIDEO` если picker недостаточен для основной функции; для медиатеки нужно заявить широкий доступ и доказать, почему выбор отдельных файлов не подходит. Если пользователь не даёт широкий доступ, приложение должно предложить разумный ограниченный путь (например, системный picker) или урезанный режим. В manifest также есть foreground service типа `dataSync` для фоновых загрузок. На target Android 14+ описать FGS в Play Console, назначение, эффект задержки/прерывания и дать видео, показывающее явное действие пользователя; проверить, не подходит ли user-initiated data transfer job. Permission должен давать полезную основную функцию, быть видимым пользователю и останавливаться пользователем.

**Native требования:** проверить 64-bit и 16 KB page sizes для всех 64-bit `.so` (Rust, FFmpeg/libmpv, libsmb2, Flutter/plugins). NDK r28+ выравнивает ELF на 16 KB по умолчанию, но prebuilt libraries тоже нужно пересобрать. Проверить AAB/ELF alignment и запуск на Android 15+ 16 KB emulator.

### Google Play — Android TV / Google TV

TV — отдельный device experience даже при том же package. Нужны Android TV quality checks, AAB, TV screenshots, listing, banner/icon и тестирование пультом. В manifest уже есть `LEANBACK_LAUNCHER`, `android.software.leanback` и banner, но этого недостаточно.

Google требует присутствие в Leanback launcher, успешный запуск, актуальный TV screenshot и App Bundle. С 2026-08-01 обязательны 32/64-bit и 16 KB page support. Проверить минимальный SDK для широкого набора ТВ (текущая TV quality guide указывает API 31 или ниже), закрыть `TV-002…004`, проверить D-pad/focus, Back, сон/возврат, память, 1080p/4K и физическое устройство.

### Microsoft Store

Предпочтителен **MSIX/MSIXBundle**: Store подписывает MSIX после certification и может хостить пакет. Win32 MSI/EXE тоже допустимы, но запуск установки должен быть silent (допустим UAC), installer — offline/standalone, без скачивания payload; требуется версионированный HTTPS URL, где бинарник не меняют после отправки. MSI/EXE надо подписывать самостоятельно.

**Подача:** Partner Center → верификация → резервирование имени → package identity/capabilities → Store flavor → Windows App Certification Kit → age/privacy/listing/screenshots → certification. Microsoft Store policies v7.20 опубликованы 2026-09-15 и действуют с 2026-10-22; требуют сертификации, полезности и точной карточки.

В проекте открыты `PKG-005` и `PKG-007`. Выбрать MSIX (предпочтительно) или существующий Win32 installer; не переиспользовать неподписанный full GitHub Release.

### F-Droid

Нужны признанная FOSS-лицензия, открытые исходники и сборка на инфраструктуре F-Droid без непроверенных proprietary SDK. Inclusion Policy допускает Anti-Features с соответствующими метками; для проекта предполагаются отключение Google Cast и Twitch/non-free network label.

Подготовить `fdroiddata` metadata/recipe, source/tag, version/code, лицензию, permissions, screenshots/description; обеспечить build Rust/Flutter/JNI/FFmpeg/mpv из исходников, без готовых vendor binaries. Сначала прогнать fdroidserver/build tools локально, затем подать inclusion merge request в `fdroiddata` и отвечать на review.

F-Droid ожидает минимум 3 месяца активной разработки, если исключение не подходит. При непрерывной активности обычный путь откроется после 2027-02-13. Reproducible build — best practice, а не формальная предпосылка inclusion; его разумно сделать сразу, поскольку поздняя смена подписи вынудит пользователей переустановить приложение.

### Flathub

Подготовить stable Linux Flatpak из исходников: manifest, AppStream metainfo, desktop file, icon, license files каждого модуля и минимум permissions. Все источники/зависимости должны быть заданы заранее; сеть во время build недоступна. Runtime network access описать permissions приложения; для файлов использовать XDG portals там, где доступны.

Подтвердить app ID контролируемым доменом или code-hosting project. Для `app.kinolune` заранее подготовить proof контроля домена либо выбрать допустимый брендированный ID. Проверить право распространять все библиотеки, шрифты, metadata и artwork. Flathub требует полный английский перевод UI, пользовательских текстов, desktop file, metainfo и документации, стабильный выпуск и meaningful development history; проектная история на срезе длится только 13 дней, что повышает риск раннего отказа. Новые приложения подают в stable, не beta: PR в `flathub/flathub` с base веткой `new-pr`, затем поддерживается созданный Flathub repo.

**Отдельное ограничение Flathub:** опубликованная Inclusion Policy запрещает AI-generated и AI-assisted содержимое в Flatpak manifest. Когда дойдёте до упаковки, manifest нужно будет написать и проверить без генерации или помощи ИИ; при подаче повторно сверить действующую редакцию правила.

## Последовательность подготовки

1. Закрыть `DEC-007`, проверить права на бренд, получить ответы/разрешения `LEGAL-001` и `LEGAL-006`, выполнить `LEGAL-009`.
2. Довести функциональность и QA до релизного уровня; зафиксировать scope store/full.
3. Завершить Apple, Android/Android TV, Windows Store, Flatpak и F-Droid pipelines: подписи, API floors, native-library checks, privacy/permissions, source recipes.
4. На финальных бинарниках собрать licenses/SBOM/NOTICE, privacy/Data safety и permissions inventory; опубликовать готовые privacy policy и support/contact page.
5. Собрать evidence pack: функции по флейворам, SDK/API и права, тесты на устройствах/ОС, 16 KB/architecture proof, privacy worksheet, screenshots, review instructions, версии/подписи, listing ru/en/uk.
6. Начать с internal/TestFlight/closed tracks, потом production review. Замечания превращать в конкретные требования, после изменений повторно проверять бинарник и декларации.
7. Linux-каналы подавать после стабилизации source build; учесть минимальный возраст проекта для F-Droid и волонтёрское ревью.
8. На каждом релизе повторять policy check, privacy diff SDK/permissions, подпись, flavor/license gates, device smoke и upgrade test.

## Проверенные официальные источники

Ссылки ведут на первичные материалы платформ. Срез требований — 2026-11-26; при фактической подаче правила надо перепроверить.

### Apple

- [App Review Guidelines](https://developer.apple.com/app-store/review/guidelines/) — completeness/stability, Apple TV remote, Mac sandbox/packaging, third-party services, IP и media downloads (§2.1, §2.4.3–2.4.5, §5.2.2–5.2.3).
- [Developer Program enrollment](https://developer.apple.com/help/account/membership/program-enrollment/) — $99/year, физлицо/организация, D‑U‑N‑S и verification.
- [App privacy details](https://developer.apple.com/app-store/app-privacy-details/) и [Manage app privacy](https://developer.apple.com/help/app-store-connect/manage-app-information/manage-app-privacy) — privacy labels, collection и partners.
- [Third-party SDK requirements](https://developer.apple.com/support/third-party-sdk-requirements/) и [Privacy manifests](https://developer.apple.com/documentation/bundleresources/adding-a-privacy-manifest-to-your-app-or-third-party-sdk) — SDK signatures, manifests, Required Reason APIs.
- [Mac App Sandbox](https://developer.apple.com/documentation/xcode/configuring-the-macos-app-sandbox) и [macOS distribution](https://developer.apple.com/macos/distribution) — MAS и direct Developer ID.
- [App submission](https://developer.apple.com/app-store/submitting/) и [Screenshot specifications](https://developer.apple.com/help/app-store-connect/reference/app-information/screenshot-specifications).

### Google / Android

- [Target API requirements](https://support.google.com/googleplay/android-developer/answer/11926878) — API 36 с 2026-08-31; Android TV exception API 34.
- [TV app quality](https://developer.android.com/develop/adaptive-apps/quality-guidelines/tv-app-quality) и [Distribute to Android TV](https://developer.android.com/training/tv/publishing/distribute) — AAB, TV criteria, 32/64-bit, 16 KB с 2026-08-01.
- [16 KB page-size support](https://developer.android.com/guide/practices/page-sizes) — native dependencies и тестирование.
- [Play Console signup](https://support.google.com/googleplay/android-developer/answer/6112435) — $25 one-time.
- [Personal-account testing requirements](https://support.google.com/googleplay/android-developer/answer/14151465) — 12 testers/14 days для затронутых аккаунтов.
- [Data safety form](https://support.google.com/googleplay/android-developer/answer/10787469) — data, ephemeral processing, SDK responsibility.
- [Play User Data policy](https://support.google.com/googleplay/android-developer/answer/10144311) — policy link in app/store, privacy contact, disclosures, data deletion.
- [Restricted permissions](https://support.google.com/googleplay/android-developer/answer/16935362) и [Foreground Service declarations](https://support.google.com/googleplay/android-developer/answer/13392821) — video-library access and `dataSync` review evidence.
- [Android developer verification](https://developer.android.com/developer-verification) — текущий rollout identity/package registration.

### Microsoft, F-Droid, Flathub

- Microsoft: [Store readiness](https://learn.microsoft.com/en-us/windows/apps/publish/store-readiness-checklist), [MSIX requirements](https://learn.microsoft.com/en-us/windows/apps/publish/publish-your-app/msix/app-package-requirements), [MSI/EXE requirements](https://learn.microsoft.com/en-us/windows/apps/publish/publish-your-app/msi/app-package-requirements), [certification](https://learn.microsoft.com/en-us/windows/apps/publish/publish-your-app/msix/app-certification-process), [Store policies](https://learn.microsoft.com/en-us/windows/apps/publish/store-policies).
- F-Droid: [Inclusion Policy](https://f-droid.org/en/docs/Inclusion_Policy/) и [Submitting Quick Start](https://f-droid.org/en/docs/Submitting_to_F-Droid_Quick_Start_Guide/).
- Flathub: [Requirements](https://docs.flathub.org/docs/for-app-authors/requirements) и [submission process](https://docs.flathub.org/docs/for-app-authors/submission).

## Границы исследования

Проверены исходники, ROADMAP и публичные правила. Не загружались сборки в консоли, не проводились тесты на реальных Apple TV/Android TV, аудит всех third-party binaries и юридические запросы к сервисам. Непубличные ответы review-команд и будущие сведения об аккаунтах неизвестны. Процент — ориентир для планирования, не замена решению магазина или заключению юриста.
DOC
