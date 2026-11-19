<!-- SPDX-License-Identifier: MPL-2.0 -->

# Kinolune — лицензии и исходники сторонних библиотек · Third-party licences and sources

Kinolune распространяется по лицензии MPL-2.0. В этом архиве лежат:

- `kinolune-<версия>-licenses.tar.gz` — уведомления (`THIRD-PARTY-NOTICES.md`), тексты всех лицензий, состав libmpv/FFmpeg (`LIBMPV-COMPONENTS.md`), libsmb2 (`LIBSMB2.md`) и проверка нативных библиотек Android (`ANDROID-NATIVE-LIBRARIES.md`);
- `kinolune-<версия>-libmpv-sources.tar.xz` — исходный код libmpv, FFmpeg и их LGPL-зависимостей ровно тех версий, что в сборках (LGPL-2.1, раздел 6);
- `kinolune-<версия>-libsmb2-sources.tar.xz` — исходный код libsmb2 (LGPL-2.1).

Эти библиотеки подключены динамически (отдельные файлы `libmpv`/`libsmb2` внутри приложения), поэтому их можно заменить своей сборкой из этих исходников. Как собрать их так же, как мы, — `tools/mpv/` и `tools/libsmb2/` в репозитории.

---

Kinolune is MPL-2.0. This archive holds the third-party notices and licence texts, plus the corresponding source code of the LGPL libraries shipped in the builds (libmpv, FFmpeg, libsmb2; LGPL-2.1 §6). They are linked dynamically (separate `libmpv`/`libsmb2` files inside the app) and can be replaced with your own build; our build scripts are in `tools/mpv/` and `tools/libsmb2/`.
