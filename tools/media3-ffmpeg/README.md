<!-- SPDX-License-Identifier: MPL-2.0 -->

# FFmpeg-расширение Media3 (PLAY-AND-004)

Аудиодекодер `FfmpegAudioRenderer` для плеера Media3 на Android (TrueHD, DTS/DTS-HD, E-AC-3, AC-3 и всё, что
умеет FFmpeg) поверх **нашего LGPL FFmpeg** из [`tools/ffmpeg`](../ffmpeg/README.md). Решение и обоснование —
ADR-NNNN.

| Файл | Назначение |
|---|---|
| `versions.sh` | Media3 1.11.1: коммит и SHA-256 каждого файла, NDK, список обязательных декодеров |
| `build.sh` | `fetch`, `<android-target> <ffmpeg-out>`, `merge` |
| `check-license.sh` | Лицензионный гейт: LGPL FFmpeg, динамическая линковка, декодеры, только аудиочасть |

## Как собрать

Нужен FFmpeg для того же ABI — артефакт `libmpv-android-<abi>` (CI `libmpv.yml`, или
`clients/flutter/app/tool/native_libs.sh fetch`) либо `tools/ffmpeg/out/android-<abi>` — и NDK r28c.

```bash
export ANDROID_NDK_HOME=$ANDROID_HOME/ndk/28.2.13676358
tools/media3-ffmpeg/build.sh android-arm64-v8a ~/.cache/kinolune-native/libmpv-android-arm64-v8a
just android ffmpeg-ext arm64-v8a      # то же из кэша native_libs.sh
```

Результат — `tools/media3-ffmpeg/out/android-<abi>/`: `jniLibs/<abi>/libffmpegJNI.so`, `java/` (аудиоклассы
расширения), `proguard-rules.txt`, `share/licenses/`, `BUILDINFO.txt` (версии, SHA-256 входов и выхода). Несколько ABI в
один каталог — `build.sh merge <dest> out/android-arm64-v8a out/android-x86_64`. Каталог подключается к Gradle:
`-PpaFfmpegExtDir=<каталог>` в `clients/android/player` и `pa_player_android` (`just android test-device` берёт
`out/android-x86_64` сам). В APK должны быть `libavcodec.so`, `libswresample.so`, `libavutil.so` — это набор libmpv,
который приложение уже везёт.

## Лицензии

- `ffmpeg_jni.cc` и Java-классы — Apache-2.0 (androidx/media), текст лицензии кладётся в `share/licenses/`.
- FFmpeg — LGPL-2.1-or-later, **только динамически**: `check-license.sh` требует `NEEDED libavcodec.so`,
  `libswresample.so`, `libavutil.so`, запрещает определённые символы FFmpeg и строки его лицензии внутри
  `libffmpegJNI.so` и прогоняет `tools/ffmpeg/check-license.sh --artifact` на линкуемом FFmpeg. GPL-сборка FFmpeg
  гейт не проходит (канарейка в CI).
- Декодеры `truehd`, `mlp`, `dca`, `eac3`, `ac3` — LGPL, в `tools/ffmpeg` включены по умолчанию (отключены только
  энкодеры); гейт падает, если их нет в `libavcodec.so`.

## Воспроизводимость

Тот же NDK, те же заголовки и библиотеки FFmpeg → те же байты `libffmpegJNI.so`: без build id, пути исходников
заменены (`-ffile-prefix-map`), libc++ статически. SHA-256 выхода — в `BUILDINFO.txt`.
