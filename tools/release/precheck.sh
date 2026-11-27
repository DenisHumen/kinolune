#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
#
# Pre-release check on the build server (PKG-025): one command before every
# release tag that runs, in minutes and on the machine it is started on, what
# release-app.yml runs on GitHub for Linux and Android, with the same
# packaging/ and tools/ scripts, so a broken script or package fails here and
# not an hour into the release run (alpha.17: a script without the executable
# bit).
#
#   tools/release/precheck.sh [<version>] [options]
#
#   <version>        release version without the tag prefix (0.1.0-alpha.19);
#                    default 0.0.0-precheck. With a version, the tag
#                    app-v<version> must not exist on origin yet.
#   --notes FILE     release notes: the text the annotated tag will carry
#                    (first line = subject). Default: the message of the tag
#                    app-v<version> when it exists locally.
#   --github         also start release-app.yml on GitHub as a dry run
#                    (workflow_dispatch: nothing is published) for Windows
#                    and macOS, on the current branch; needs `gh auth login`.
#   --github-wait    like --github and wait for both runs.
#   --only LIST      run only these stages (comma-separated, see below).
#   --keep-going     do not stop at the first failed stage.
#   --jobs N         CPU limit and parallel jobs (default: half the CPUs, at most 16; PA_PRECHECK_JOBS).
#   --cache DIR      caches and run outputs (default PA_PRECHECK_CACHE or
#                    ${XDG_CACHE_HOME:-~/.cache}/kinolune-precheck).
#
# Stages: general (executable bits of every script CI runs, version, tag,
# release notes), native (our LGPL libmpv from the `prebuilt` release, libsmb2
# built from tools/libsmb2; licence gates), assets (fonts, anime database),
# linux (flutter build linux, tar.gz + AppImage, .deb + .rpm, bundle smoke
# checks), android (APKs arm64-v8a and armeabi-v7a, signed with a throwaway
# keystore, native-library licence gate), assemble (rpm signed through
# packaging/linux/import-key.sh with a throwaway GPG key, licences and LGPL
# sources, SHA256SUMS signed with a throwaway minisign key, release notes,
# expected files). Windows and macOS need their own runners: --github.
#
# It builds in the checkout it belongs to (keep a separate clone for it):
# Flutter and Gradle outputs stay in build/ and .dart_tool/ between runs,
# Cargo, pub and Gradle caches and the native libraries stay in the cache
# directory, so a run without native-library changes takes minutes. The
# builds run in a container (tools/release/precheck.Dockerfile, Ubuntu 24.04
# like the runner) as the current user; Flutter, Rust (rustup), Java and the
# Android SDK come from the host: `flutter`, `rustup` and `java` on PATH,
# ANDROID_HOME set. Needs docker, git, curl; never touches adb devices.
# Containers are named ${PA_PRECHECK_CONTAINER_PREFIX:-kinolune-precheck}-<stage>-<pid>.
#
# Exit status 0 means «можно выпускать» for Linux and Android.
# shellcheck disable=SC2154 # LIBMPV_*/LIBSMB2_* come from $RUN/native.env (load_native)
set -euo pipefail

SELF="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/$(basename "${BASH_SOURCE[0]}")"
ROOT="$(cd "$(dirname "$SELF")/../.." && pwd)"
APP_DIR="$ROOT/clients/flutter/app"
STAGES=(general native assets linux android assemble)
# Kept in sync with release-app.yml (the Android job builds these two for
# people; x86_64 and universal are test builds there).
ABIS=(arm64-v8a armeabi-v7a)
# Linux is built for the machine it runs on (no emulation): x86_64 or, on an
# arm64 machine, aarch64 (PKG-026). Names follow release-app.yml.
case "$(uname -m)" in
    x86_64) LINUX_T=linux-x86_64 LINUX_ARCH=x86_64 FLUTTER_ARCH=x64 DEB_ARCH=amd64 OTHER_RE='(aarch64|arm64)\.(AppImage|deb|rpm)$' ;;
    aarch64 | arm64) LINUX_T=linux-arm64 LINUX_ARCH=aarch64 FLUTTER_ARCH=arm64 DEB_ARCH=arm64 OTHER_RE='(x86_64|amd64)\.(AppImage|deb|rpm)$' ;;
    *) printf 'precheck: no Linux build for %s\n' "$(uname -m)" >&2; exit 2 ;;
esac

say() { printf '%s\n' "$*" >&2; }
die() { say "precheck: $*"; exit 2; }
usage() { sed -n '4,49p' "$SELF" >&2; exit 2; }

# --- stage bodies (run inside the container, cwd = $ROOT) -------------------

# Value of `NAME: "value"` in a workflow's env block.
wf_env() { sed -nE "s/^  $2: \"([^\"]*)\".*/\1/p" "$ROOT/.github/workflows/$1" | head -n1; }

stage_general() {
    local fail=0
    echo "== Скрипты, которые CI запускает по пути, должны быть исполняемыми"
    python3 -I "$ROOT/tools/release/check_ci_exec.py" --root "$ROOT" --worktree || fail=1

    echo "== Версия $VERSION"
    if ! [[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$ ]]; then
        echo "ОШИБКА: '$VERSION' — не semver (x.y.z или x.y.z-pre), release-app.yml её не примет"; fail=1
    fi
    [ -f "$APP_DIR/pubspec.yaml" ] && [ -f "$APP_DIR/pubspec.lock" ] \
        || { echo "ОШИБКА: нет pubspec.yaml/pubspec.lock приложения (релиз ставит зависимости с --enforce-lockfile)"; fail=1; }

    local tag="app-v$VERSION"
    if [ "$VERSION" != 0.0.0-precheck ]; then
        if git ls-remote --exit-code --tags origin "refs/tags/$tag" >/dev/null 2>&1; then
            echo "ОШИБКА: тег $tag уже есть на origin — выпуск с этой версией упадёт на публикации"; fail=1
        fi
    fi

    echo "== Заметки к выпуску"
    local msg="$RUN/message.md"
    : > "$msg"
    if [ -n "$NOTES" ]; then
        if [ ! -s "$NOTES" ]; then
            echo "ОШИБКА: файл заметок $NOTES пуст или не найден"; fail=1
        else
            # `git tag -a -F` keeps the file as the message: subject, blank line, body.
            sed '/-----BEGIN/,$d' "$NOTES" > "$msg"
            echo "из файла $NOTES: «$(head -n1 "$msg")»"
        fi
    elif git rev-parse -q --verify "refs/tags/$tag" >/dev/null; then
        if [ "$(git cat-file -t "$tag")" != tag ]; then
            echo "ОШИБКА: $tag — лёгкий тег без текста: заметки релиза будут только «Kinolune $VERSION.» (нужен git tag -a)"; fail=1
        else
            local subject body
            subject="$(git tag -l --format='%(contents:subject)' "$tag")"
            body="$(git tag -l --format='%(contents:body)' "$tag")"
            printf '%s\n\n%s\n' "$subject" "$body" | sed '/-----BEGIN/,$d' > "$msg"
            [ "$(git rev-parse "$tag^{commit}")" = "$(git rev-parse HEAD)" ] \
                || { echo "ОШИБКА: тег $tag указывает не на проверяемый коммит"; fail=1; }
            echo "из тега $tag: «$subject»"
            [ -n "$body" ] || echo "ВНИМАНИЕ: у тега нет текста кроме заголовка («Что нового» пусто)"
        fi
    else
        echo "ВНИМАНИЕ: заметки не проверены — нет --notes и тега $tag (будет «Kinolune $VERSION.»)"
    fi

    echo "== Рабочее дерево"
    if [ -n "$(git status --porcelain --untracked-files=no)" ]; then
        echo "ВНИМАНИЕ: есть незакоммиченные изменения — проверяется не то, что уйдёт в тег:"
        git status --short --untracked-files=no | head -n 20
    fi
    echo "коммит $(git rev-parse --short HEAD): $(git log -1 --format=%s)"
    return "$fail"
}

# libmpv: content-addressed prebuilt (tools/ci/prebuilt.sh, the same build CI
# uses), cached by key. libsmb2: built from tools/libsmb2 (a minute per
# target), cached by a hash of its recipe.
stage_native() {
    local t key dir fail=0
    export ANDROID_NDK_PACKAGE MESON_VERSION NINJA_VERSION
    ANDROID_NDK_PACKAGE="$(wf_env libmpv.yml ANDROID_NDK_PACKAGE)"
    MESON_VERSION="$(wf_env libmpv.yml MESON_VERSION)"
    NINJA_VERSION="$(wf_env libmpv.yml NINJA_VERSION)"
    : "${ANDROID_NDK_PACKAGE:?}" "${MESON_VERSION:?}" "${NINJA_VERSION:?}"
    export GITHUB_REPOSITORY="${GITHUB_REPOSITORY:-$GH_REPO}"
    mkdir -p "$CACHE/native"
    : > "$RUN/native.env"
    for t in "$LINUX_T" "${ABIS[@]/#/android-}"; do
        key="$(tools/ci/prebuilt.sh key libmpv "$t")"
        dir="$CACHE/native/$key"
        if [ -d "$dir/lib" ]; then
            echo "libmpv $t: из кэша ($key)"
        else
            echo "libmpv $t: скачиваю $key из выпуска prebuilt"
            local code=0
            tools/ci/prebuilt.sh fetch "$key" "$dir" || code=$?
            if [ "$code" = 3 ]; then
                echo "ОШИБКА: libmpv $key не опубликован: рецепт tools/mpv изменился, а prebuilt.yml на main ещё не собрал его."
                echo "  Дождитесь prebuilt.yml или соберите сами (около часа):"
                echo "  tools/mpv/docker-build.sh $t && cp -a tools/mpv/out/$t '$dir'"
                fail=1; continue
            elif [ "$code" != 0 ]; then
                fail=1; continue
            fi
        fi
        tools/mpv/check-license.sh --artifact "$dir" || fail=1
        [[ $t == android-* ]] && { tools/mpv/bundle.sh check-android "$dir" || fail=1; }
        echo "LIBMPV_${t//-/_}=$dir" >> "$RUN/native.env"
    done
    # Sources of that libmpv for the release's LGPL bundle (assemble).
    local recipe src
    recipe="$(tools/ci/prebuilt.sh recipe libmpv)"
    src="$CACHE/native/libmpv-sources-$recipe.tar.xz"
    if [ ! -s "$src" ]; then
        curl -fsSL --retry 3 -o "$src.part" \
            "https://github.com/$GITHUB_REPOSITORY/releases/download/prebuilt/libmpv-sources-$recipe.tar.xz" \
            && mv "$src.part" "$src" || { echo "ОШИБКА: нет libmpv-sources-$recipe.tar.xz в выпуске prebuilt"; fail=1; }
    fi
    echo "LIBMPV_SOURCES=$src" >> "$RUN/native.env"

    local smb out
    smb="$(cd "$ROOT" && find tools/libsmb2 -type f -not -path '*/.cache/*' -not -path '*/build/*' -not -path '*/out/*' \
        | LC_ALL=C sort | xargs sha256sum | sha256sum | cut -c1-16)"
    out="$CACHE/native/libsmb2-$smb"
    export PA_SMB2_CACHE="$CACHE/native/libsmb2-src" PA_SMB2_WORK="$CACHE/native/libsmb2-work" PA_SMB2_OUT="$out"
    export PA_SMB2_JOBS="$JOBS"
    export ANDROID_NDK_HOME="$ANDROID_HOME/${ANDROID_NDK_PACKAGE/;//}"
    for t in "$LINUX_T" "${ABIS[@]/#/android-}"; do
        if [ -d "$out/$t/lib" ]; then
            echo "libsmb2 $t: из кэша ($smb)"
        else
            echo "libsmb2 $t: сборка из tools/libsmb2"
            tools/libsmb2/build.sh "$t" || { fail=1; continue; }
        fi
        tools/libsmb2/check-license.sh --artifact "$out/$t" || fail=1
        [[ $t == android-* ]] && { tools/libsmb2/bundle.sh check-android "$out/$t" || fail=1; }
        echo "LIBSMB2_${t//-/_}=$out/$t" >> "$RUN/native.env"
    done
    if [ ! -s "$CACHE/native/libsmb2-sources-$smb.tar.xz" ]; then
        tools/libsmb2/build.sh sources || fail=1
        cp "$out"/project_alpha-libsmb2-sources-*.tar.xz "$CACHE/native/libsmb2-sources-$smb.tar.xz" || fail=1
    fi
    echo "LIBSMB2_SOURCES=$CACHE/native/libsmb2-sources-$smb.tar.xz" >> "$RUN/native.env"
    return "$fail"
}

stage_assets() {
    echo "== Шрифты (design/scripts/fonts-fetch.mjs)"
    node design/scripts/fonts-fetch.mjs
    [ -f design/flutter/pa_design/fonts/InterVariable.ttf ] || { echo "ОШИБКА: Inter не скачан"; return 1; }
    echo "== База аниме (tools/anime-mappings/build.sh, META-030)"
    rustup toolchain install >/dev/null
    CARGO_TARGET_DIR="$CACHE/cargo-target/anime-db" tools/anime-mappings/build.sh "$RUN/anime-db"
    install -m 644 "$RUN/anime-db/anime-offline-ids.jsonl.zst" \
        "$RUN/anime-db/LICENSE.anime-offline-database" "$APP_DIR/assets/anime-db/"
}

load_native() {
    [ -f "$RUN/native.env" ] || { echo "ОШИБКА: этап native не выполнен в этом запуске"; return 1; }
    # shellcheck disable=SC1091
    set -a; . "$RUN/native.env"; set +a
}

flutter_common() {
    printf '%s\n' --release --build-name "$VERSION" --build-number "$BUILD_NUMBER" \
        "--dart-define=KINOLUNE_VERSION=$VERSION" "--dart-define=KINOLUNE_FLAVOR=$KINOLUNE_FLAVOR" \
        "--dart-define=KINOLUNE_TWITCH_CLIENT_ID=${KINOLUNE_TWITCH_CLIENT_ID:-}"
}

stage_linux() {
    load_native || return 1
    local dist="$RUN/dist" common=() lm="LIBMPV_${LINUX_T//-/_}" ls="LIBSMB2_${LINUX_T//-/_}"
    mkdir -p "$dist"
    mapfile -t common < <(flutter_common)
    (cd core/crates/pa-frb && rustup toolchain install >/dev/null)
    echo "== flutter build linux ($KINOLUNE_FLAVOR)"
    (
        cd "$APP_DIR"
        export PKG_CONFIG_PATH="${!lm}/lib/pkgconfig"
        pkg-config --modversion mpv
        flutter pub get --enforce-lockfile --suppress-analytics
        rm -rf "build/linux/$FLUTTER_ARCH/release/bundle"
        flutter build linux --suppress-analytics "${common[@]}"
    )
    echo "== tar.gz и AppImage (packaging/app/linux/package.sh)"
    PA_LIBMPV_DIR="${!lm}" PA_LIBSMB2_DIR="${!ls}" \
        PA_APPIMAGE_TOOLS="$CACHE/appimage-tools" \
        packaging/app/linux/package.sh "$APP_DIR/build/linux/$FLUTTER_ARCH/release/bundle" "$VERSION" "$dist"
    echo "== .deb и .rpm (packaging/linux/build-packages.sh)"
    packaging/linux/build-packages.sh "$dist/kinolune-$VERSION-linux-$LINUX_ARCH.tar.gz" "$VERSION" "$dist"

    echo "== Проверка бандла (как «Smoke-test the bundle links» в release-app.yml)"
    local tmp dir exe
    tmp="$(mktemp -d)"
    tar -xzf "$dist/kinolune-$VERSION-linux-$LINUX_ARCH.tar.gz" -C "$tmp"
    dir="$tmp/kinolune-$VERSION-linux-$LINUX_ARCH"
    exe=$(find "$dir" -maxdepth 1 -type f -perm -u+x | head -n1)
    if ldd "$exe" "$dir"/lib/*.so* | grep 'not found'; then echo "ОШИБКА: неразрешённые библиотеки"; return 1; fi
    ldd "$dir/lib/libmedia_kit_video_plugin.so" | grep -F "libmpv.so.2 => $dir/lib/libmpv.so.2" \
        || { echo "ОШИБКА: media_kit_video грузит не нашу libmpv"; return 1; }
    [ -f "$dir/licenses/libmpv/BUILDINFO.txt" ] || { echo "ОШИБКА: нет лицензий libmpv"; return 1; }
    [ -f "$dir/lib/libsmb2.so.1" ] || { echo "ОШИБКА: нет libsmb2.so.1 в бандле"; return 1; }
    python3 -I -c 'import ctypes, sys; ctypes.CDLL(sys.argv[1]).smb2_init_context' "$dir/lib/libsmb2.so.1" \
        || { echo "ОШИБКА: libsmb2.so.1 из бандла не загружается"; return 1; }
    [ -f "$dir/licenses/libsmb2/BUILDINFO.txt" ] && [ -f "$dir/licenses/libsmb2/libsmb2-COPYING" ] \
        || { echo "ОШИБКА: нет лицензий libsmb2"; return 1; }
    if nm -D --defined-only "$dir/lib/libpa_frb.so" | grep ' smb2_'; then
        echo "ОШИБКА: libpa_frb.so определяет smb2_* (libsmb2 влинкован)"; return 1
    fi
    tools/flavor/check-artifacts.sh "$KINOLUNE_FLAVOR" "$dir/lib/libpa_frb.so"
    rm -rf "$tmp"
    ls -l "$dist"
}

stage_android() {
    load_native || return 1
    local dist="$RUN/dist" common=() abi apks jni platforms=""
    mkdir -p "$dist"
    mapfile -t common < <(flutter_common)
    jni=clients/flutter/packages/media_kit_libs_android_video/android/src/main/jniLibs
    echo "== Наши libmpv и libsmb2 в jniLibs (tools/mpv/bundle.sh, tools/libsmb2/bundle.sh)"
    rm -rf "$jni"
    local mpv=() smb=() v
    for abi in "${ABIS[@]}"; do
        v="LIBMPV_android_${abi//-/_}"; mpv+=("${!v}")
        v="LIBSMB2_android_${abi//-/_}"; smb+=("${!v}")
    done
    tools/mpv/bundle.sh android "$jni" "${mpv[@]}"
    tools/libsmb2/bundle.sh android "$jni" "${smb[@]}"
    (cd core/crates/pa-frb && rustup toolchain install >/dev/null)

    for abi in "${ABIS[@]}"; do
        case "$abi" in arm64-v8a) platforms+="android-arm64," ;; armeabi-v7a) platforms+="android-arm," ;; x86_64) platforms+="android-x64," ;; esac
    done
    echo "== flutter build apk --split-per-abi (${platforms%,})"
    apks="$APP_DIR/build/app/outputs/flutter-apk"
    (
        cd "$APP_DIR"
        flutter pub get --enforce-lockfile --suppress-analytics
        rm -f "$apks"/*.apk
        flutter build apk --suppress-analytics "${common[@]}" --target-platform "${platforms%,}" --split-per-abi
    )
    for abi in "${ABIS[@]}"; do
        cp "$apks/app-$abi-release.apk" "$dist/kinolune-$VERSION-android-$abi.apk"
        unzip -l "$dist/kinolune-$VERSION-android-$abi.apk" "lib/$abi/libpa_frb.so" >/dev/null \
            || { echo "ОШИБКА: в APK $abi нет lib/$abi/libpa_frb.so (ядро на Rust)"; return 1; }
        tools/flavor/check-artifacts.sh "$KINOLUNE_FLAVOR" "$dist/kinolune-$VERSION-android-$abi.apk"
    done
    local aapt2 apk
    aapt2="$(find "$ANDROID_HOME/build-tools" -mindepth 2 -maxdepth 2 -name aapt2 | sort -V | tail -n1)"
    for apk in "$dist"/*.apk; do
        # Not `| grep -q`: with pipefail an early grep exit fails the pipe.
        "$aapt2" dump permissions "$apk" > "$RUN/permissions.txt"
        grep -q REQUEST_INSTALL_PACKAGES "$RUN/permissions.txt" \
            || { echo "ОШИБКА: $apk без REQUEST_INSTALL_PACKAGES (обновление из приложения)"; return 1; }
    done

    echo "== Подпись APK одноразовым ключом (packaging/app/android/sign.sh)"
    local ks mode
    ks="$(mktemp -d)"
    keytool -genkeypair -keystore "$ks/throwaway.p12" -storetype PKCS12 -alias precheck -keyalg RSA \
        -keysize 2048 -validity 1 -dname "CN=Kinolune precheck" -storepass precheck -keypass precheck >/dev/null 2>&1
    mode="$(ANDROID_KEYSTORE_BASE64="$(base64 -w0 "$ks/throwaway.p12")" ANDROID_KEY_ALIAS=precheck \
        ANDROID_KEYSTORE_PASSWORD=precheck ANDROID_KEY_PASSWORD=precheck \
        packaging/app/android/sign.sh "$dist"/*.apk)"
    rm -rf "$ks"
    [ "$mode" = release ] || { echo "ОШИБКА: sign.sh вернул '$mode', ожидалось release"; return 1; }

    echo "== Лицензии нативных библиотек (packaging/app/android/check-native-licences.sh)"
    mkdir -p "$RUN/notes"
    packaging/app/android/check-native-licences.sh "$RUN/notes/native-licences.md" "$dist"/*.apk > "$RUN/gate.out"
    tail -n2 "$RUN/gate.out"
    grep -qx libmpv=yes "$RUN/gate.out" || { echo "ОШИБКА: в APK или ABI нет набора libmpv из tools/mpv"; return 1; }
    grep -qx libsmb2=yes "$RUN/gate.out" || { echo "ОШИБКА: в APK или ABI нет libsmb2.so"; return 1; }
    ls -l "$dist"/*.apk
}

# What the `assemble` job does with the Linux and Android files.
stage_assemble() {
    load_native || return 1
    local dist="$RUN/dist" rel="$RUN/release" tmp f
    rm -rf "$rel"; mkdir -p "$rel/assets"
    tmp="$(mktemp -d)"

    echo "== Подпись .rpm одноразовым ключом GPG через packaging/linux/import-key.sh"
    local rpm fpr
    rpm="$(ls "$dist"/kinolune-*-linux-"$LINUX_ARCH".rpm)"
    export GNUPGHOME="$tmp/gnupg-gen"
    mkdir -m 700 "$GNUPGHOME"
    gpg --batch --quiet --passphrase '' --quick-gen-key "Kinolune precheck (throwaway) <precheck@invalid>" rsa3072 sign 1d
    gpg --batch --armor --export > "$tmp/throwaway-public.asc"
    local secret
    secret="$(gpg --batch --armor --export-secret-keys)"
    export GNUPGHOME="$tmp/gnupg"
    fpr="$(PA_PKG_REPO_GPG_KEY="$secret" PA_REPO_KEY="$tmp/throwaway-public.asc" packaging/linux/import-key.sh)"
    rpmsign --define "__gpg $(command -v gpg)" --define "_gpg_name $fpr" --define "_gpg_path $GNUPGHOME" \
        --define "_gpg_digest_algo sha512" --addsign "$rpm"
    mkdir -p "$tmp/rpmdb"
    rpmkeys --dbpath "$tmp/rpmdb" --import "$tmp/throwaway-public.asc"
    rpmkeys --dbpath "$tmp/rpmdb" --checksig "$rpm"
    unset GNUPGHOME

    echo "== Файлы, лицензии, LGPL-исходники, SHA256SUMS"
    cp "$dist"/* "$rel/assets/"
    rm -f "$rel/assets/kinolune-$VERSION-linux-$LINUX_ARCH.tar.gz"
    local lic="kinolune-$VERSION-licenses" tp="kinolune-$VERSION-third-party"
    mkdir -p "$tmp/$tp" "$tmp/$lic/licenses/libmpv" "$tmp/$lic/licenses/libsmb2"
    cp LICENSE packaging/app/THIRD-PARTY-NOTICES.md "$tmp/$lic/"
    packaging/app/fetch-licences.sh "$tmp/$lic/licenses"
    local lm="LIBMPV_${LINUX_T//-/_}" ls="LIBSMB2_${LINUX_T//-/_}"
    local libmpv=("${!lm}") libsmb2=("${!ls}") abi v d
    for abi in "${ABIS[@]}"; do
        v="LIBMPV_android_${abi//-/_}"; libmpv+=("${!v}")
        v="LIBSMB2_android_${abi//-/_}"; libsmb2+=("${!v}")
    done
    PA_LIBMPV_SOURCES="$LIBMPV_SOURCES" packaging/app/lgpl-sources.sh \
        "$tmp/$tp/kinolune-$VERSION-libmpv-sources.tar.xz" "$rel/libmpv.md" "${libmpv[@]}"
    cp "$rel/libmpv.md" "$tmp/$lic/LIBMPV-COMPONENTS.md"
    for d in "${libmpv[@]}"; do cp "$d"/share/licenses/* "$tmp/$lic/licenses/libmpv/"; done
    PA_LIBSMB2_SOURCES="$LIBSMB2_SOURCES" packaging/app/libsmb2-sources.sh \
        "$tmp/$tp/kinolune-$VERSION-libsmb2-sources.tar.xz" "$rel/libsmb2.md" "${libsmb2[@]}"
    cp "$rel/libsmb2.md" "$tmp/$lic/LIBSMB2.md"
    cp "$RUN/notes/native-licences.md" "$tmp/$lic/ANDROID-NATIVE-LIBRARIES.md"
    for d in "${libsmb2[@]}"; do cp "$d"/share/licenses/* "$tmp/$lic/licenses/libsmb2/"; done
    cp "${!ls}/BUILDINFO.txt" "$tmp/$lic/licenses/libsmb2/BUILDINFO.txt"
    tar --sort=name --owner=0 --group=0 --numeric-owner -C "$tmp" -czf "$tmp/$tp/$lic.tar.gz" "$lic"
    cp packaging/app/THIRD-PARTY-README.md "$tmp/$tp/README.md"
    tar --sort=name --owner=0 --group=0 --numeric-owner -C "$tmp" -cf "$rel/assets/$tp.tar" "$tp"
    (cd "$rel/assets" && sha256sum -- * > SHA256SUMS)

    echo "== Подпись SHA256SUMS одноразовым ключом minisign (rsign2, как в release-app.yml)"
    local rsign_ver
    rsign_ver="$(sed -nE 's/.*cargo install --locked --quiet rsign2@([0-9.]+).*/\1/p' .github/workflows/release-app.yml | head -n1)"
    if [ ! -x "$CACHE/tools/rsign-$rsign_ver/bin/rsign" ]; then
        CARGO_TARGET_DIR="$CACHE/cargo-target/tools" \
            cargo install --locked --quiet --root "$CACHE/tools/rsign-$rsign_ver" "rsign2@$rsign_ver"
    fi
    local rsign="$CACHE/tools/rsign-$rsign_ver/bin/rsign"
    "$rsign" generate -W -f -p "$tmp/test.pub" -s "$tmp/test.key" >/dev/null
    "$rsign" sign -W -s "$tmp/test.key" -x "$rel/assets/SHA256SUMS.minisig" \
        -t "timestamp:$(date +%s)	file:SHA256SUMS	hashed" "$rel/assets/SHA256SUMS" >/dev/null
    "$rsign" verify -p "$tmp/test.pub" -x "$rel/assets/SHA256SUMS.minisig" "$rel/assets/SHA256SUMS"
    # The release key itself must be a real one (the app checks against it).
    jq -e -r '.updates.release_public_key | select(type == "string" and length > 40)' config/remote-config.json >/dev/null \
        || { echo "ОШИБКА: в config/remote-config.json нет updates.release_public_key"; return 1; }

    echo "== Заметки релиза (как шаг «Release notes»)"
    local notes="$rel/notes.md" BASE="https://github.com/$GH_REPO/releases/download/app-v$VERSION"
    # The table and help text live in release-app.yml; take them from there so
    # the check covers the file names the release will link to.
    python3 -I - "$ROOT/.github/workflows/release-app.yml" "$RUN/message.md" "$VERSION" "$BASE" > "$notes" <<'PY'
import re, subprocess, sys, yaml
wf, message, version, base = sys.argv[1:5]
doc = yaml.safe_load(open(wf, encoding="utf-8"))
script = next(s["run"] for s in doc["jobs"]["assemble"]["steps"] if s.get("name") == "Release notes")
# Only the block that writes the file; the message comes from --notes / the tag.
body = script[script.index("{\n"):script.index('} > "$notes"') + 1]
env = {"VERSION": version, "BASE": base, "SIGNING": "release", "PATH": "/usr/bin:/bin",
       "message": open(message, encoding="utf-8").read().strip()}
sys.stdout.write(subprocess.run(["bash", "-euo", "pipefail", "-c", body], env=env, check=True,
                                capture_output=True, text=True).stdout)
PY
    head -n 3 "$notes"

    echo "== Ожидаемые файлы (Linux и Android; Windows и macOS — в dry_run на GitHub)"
    local want=("kinolune-$VERSION-$LINUX_ARCH.AppImage" "kinolune-$VERSION-linux-$DEB_ARCH.deb"
                "kinolune-$VERSION-linux-$LINUX_ARCH.rpm" "kinolune-$VERSION-third-party.tar" SHA256SUMS SHA256SUMS.minisig)
    for abi in "${ABIS[@]}"; do want+=("kinolune-$VERSION-android-$abi.apk"); done
    local fail=0 have
    for f in "${want[@]}"; do
        if [ ! -s "$rel/assets/$f" ]; then echo "ОШИБКА: нет $f"; fail=1; fi
    done
    for have in "$rel"/assets/*; do
        have="$(basename "$have")"
        case " ${want[*]} " in *" $have "*) ;; *) echo "ОШИБКА: лишний файл в выпуске: $have"; fail=1 ;; esac
    done
    # Every Linux/Android link of the notes points at a file of the release
    # (Linux files of the other architecture are built on its own runner).
    for f in $(grep -oE "kinolune-$VERSION-((x86_64|aarch64)\.AppImage|linux-[a-z0-9_]+\.(deb|rpm)|android-[a-z0-9_-]+\.apk)" "$notes" | sort -u); do
        [[ $f =~ $OTHER_RE ]] && continue
        [ -s "$rel/assets/$f" ] || { echo "ОШИБКА: заметки ссылаются на $f, а его нет среди файлов"; fail=1; }
    done
    rm -rf "$tmp"
    [ "$fail" = 0 ] && ls -l "$rel/assets"
    return "$fail"
}

# --- inside the container ---------------------------------------------------

if [ "${1:-}" = --stage ]; then
    cd "$ROOT"
    stage="$2"
    case " ${STAGES[*]} " in *" $stage "*) ;; *) die "unknown stage $stage" ;; esac
    "stage_$stage"
    exit $?
fi

# --- host: options ----------------------------------------------------------

VERSION=0.0.0-precheck
NOTES=""
GITHUB=0
GITHUB_WAIT=0
ONLY=""
KEEP_GOING=0
half=$(( $(nproc 2>/dev/null || echo 8) / 2 )); (( half < 1 )) && half=1; (( half > 16 )) && half=16
JOBS="${PA_PRECHECK_JOBS:-$half}"
CACHE="${PA_PRECHECK_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/kinolune-precheck}"
while [ $# -gt 0 ]; do
    case "$1" in
        --notes) NOTES="$(cd "$(dirname "$2")" && pwd)/$(basename "$2")"; shift ;;
        --github) GITHUB=1 ;;
        --github-wait) GITHUB=1; GITHUB_WAIT=1 ;;
        --only) ONLY="$2"; shift ;;
        --keep-going) KEEP_GOING=1 ;;
        --jobs) JOBS="$2"; shift ;;
        --cache) CACHE="$2"; shift ;;
        -h|--help) usage ;;
        -*) die "unknown option $1" ;;
        *) VERSION="${1#app-v}" ;;
    esac
    shift
done
[[ "$JOBS" =~ ^[1-9][0-9]*$ ]] || die "--jobs needs a number"
mkdir -p "$CACHE"
CACHE="$(cd "$CACHE" && pwd)"
selected=("${STAGES[@]}")
if [ -n "$ONLY" ]; then
    IFS=, read -r -a selected <<<"$ONLY"
    for s in "${selected[@]}"; do case " ${STAGES[*]} " in *" $s "*) ;; *) die "unknown stage $s (${STAGES[*]})" ;; esac; done
fi

GH_REPO="$(git -C "$ROOT" remote get-url origin 2>/dev/null | sed -E 's#^(git@github.com:|https://github.com/)##; s#\.git$##')"
[[ "$GH_REPO" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]] || GH_REPO=DenisHumen/kinolune
SHA="$(git -C "$ROOT" rev-parse --short HEAD)"
RUN="$CACHE/runs/$(date +%Y%m%d-%H%M%S)-$SHA"
mkdir -p "$RUN/logs"
# Keep the last five runs.
ls -1d "$CACHE"/runs/*/ 2>/dev/null | sort | head -n -5 | xargs -r rm -rf

# --- host: the container ----------------------------------------------------

need() { command -v "$1" >/dev/null || die "$1 is required on PATH ($2)"; }
need docker "the builds run in a container"
need git ""
real_dir() { (cd "$1" && pwd -P); }

IMAGE=""
MOUNTS=()
CENV=()
prepare_container() {
    [ -n "$IMAGE" ] && return 0
    need flutter "Flutter SDK $(sed -nE 's/^  FLUTTER_VERSION: "([^"]*)".*/\1/p' "$ROOT/.github/workflows/release-app.yml")"
    need rustup "Rust via rustup"
    need java "JDK 21"
    : "${ANDROID_HOME:?set ANDROID_HOME to the Android SDK}"
    local flutter_root cargo_bin rustup_home java_home git_common
    flutter_root="$(real_dir "$(dirname "$(readlink -f "$(command -v flutter)")")/..")"
    cargo_bin="$(real_dir "$(dirname "$(command -v rustup)")")"
    rustup_home="$(real_dir "${RUSTUP_HOME:-$HOME/.rustup}")"
    java_home="$(real_dir "${JAVA_HOME:-$(dirname "$(readlink -f "$(command -v java)")")/..}")"
    ANDROID_HOME="$(real_dir "$ANDROID_HOME")"
    git_common="$(cd "$ROOT" && real_dir "$(git rev-parse --git-common-dir)")"

    local tag
    tag="kinolune-precheck:$(sha256sum < "$ROOT/tools/release/precheck.Dockerfile" | cut -c1-12)"
    if ! docker image inspect "$tag" >/dev/null 2>&1; then
        say "   образ $tag (один раз, несколько минут)"
        docker build -q -t "$tag" -f "$ROOT/tools/release/precheck.Dockerfile" "$ROOT/tools/release" >/dev/null
    fi
    IMAGE="$tag"
    mkdir -p "$CACHE/home" "$CACHE/cargo" "$CACHE/pub" "$CACHE/gradle"
    local d
    for d in "$ROOT" "$CACHE" "$flutter_root" "$rustup_home" "$ANDROID_HOME" "$git_common"; do
        MOUNTS+=(-v "$d:$d")
    done
    MOUNTS+=(-v "$cargo_bin:$cargo_bin:ro" -v "$java_home:$java_home:ro")
    CENV=(
        -e "HOME=$CACHE/home" -e "RUSTUP_HOME=$rustup_home" -e "CARGO_HOME=$CACHE/cargo"
        -e "PUB_CACHE=$CACHE/pub" -e "GRADLE_USER_HOME=$CACHE/gradle" -e "ANDROID_HOME=$ANDROID_HOME"
        -e "ANDROID_SDK_ROOT=$ANDROID_HOME" -e "JAVA_HOME=$java_home"
        -e "PATH=$cargo_bin:$flutter_root/bin:$java_home/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
        -e "CARGO_BUILD_JOBS=$JOBS" -e "GRADLE_OPTS=-Dorg.gradle.workers.max=$JOBS"
        -e "KINOLUNE_FLAVOR=full" -e "CARGO_KINOLUNE_FLAVOR=full"
        -e "VERSION=$VERSION" -e "BUILD_NUMBER=1" -e "NOTES=$NOTES" -e "RUN=$RUN" -e "CACHE=$CACHE"
        -e "JOBS=$JOBS" -e "GH_REPO=$GH_REPO" -e "FLUTTER_SUPPRESS_ANALYTICS=true" -e "CI=true"
    )
    [ "$NOTES" = "" ] || MOUNTS+=(-v "$(dirname "$NOTES"):$(dirname "$NOTES"):ro")
}

run_stage() { # stage -> exit status; output in $RUN/logs
    prepare_container || return 1
    docker run --rm --init --name "${PA_PRECHECK_CONTAINER_PREFIX:-kinolune-precheck}-$1-$$" \
        -u "$(id -u):$(id -g)" --cpus "$JOBS" "${MOUNTS[@]}" "${CENV[@]}" -w "$ROOT" "$IMAGE" \
        nice -n 10 bash "$SELF" --stage "$1"
}

# --- host: run --------------------------------------------------------------

declare -A TITLE=(
    [general]="Общие проверки (права на запуск, версия, тег, заметки)"
    [native]="Нативные библиотеки (libmpv, libsmb2, лицензии)"
    [assets]="Шрифты и база аниме"
    [linux]="Linux: сборка, AppImage, .deb, .rpm"
    [android]="Android: APK arm64-v8a и armeabi-v7a, подпись, лицензии"
    [assemble]="Сборка выпуска: подпись .rpm и SHA256SUMS, лицензии, заметки"
)
fmt() { printf '%dм%02dс' $(($1 / 60)) $(($1 % 60)); }
start_all=$SECONDS
failed=()
say "Kinolune precheck $VERSION — $SHA, кэш $CACHE"
say "Журналы: $RUN/logs"
n=0
for s in "${selected[@]}"; do
    n=$((n + 1))
    say "[$n/${#selected[@]}] ${TITLE[$s]} ..."
    t0=$SECONDS
    log="$RUN/logs/$n-$s.log"
    rc=0
    # The general stage is quick and needs no container when PyYAML is here:
    # a broken executable bit fails in seconds, before any image is built.
    if [ "$s" = general ] && python3 -c 'import yaml' 2>/dev/null; then
        (export VERSION NOTES RUN CACHE GH_REPO APP_DIR; cd "$ROOT" && bash "$SELF" --stage general) > "$log" 2>&1 || rc=$?
    else
        run_stage "$s" > "$log" 2>&1 || rc=$?
    fi
    if [ "$rc" = 0 ]; then
        say "      ok ($(fmt $((SECONDS - t0))))"
        grep -E '^ВНИМАНИЕ' "$log" | sed 's/^/      /' >&2 || true
    else
        say "      СЛОМАНО ($(fmt $((SECONDS - t0)))), журнал $log:"
        grep -E '^(ОШИБКА|error|FAIL)|::error::' "$log" | head -n 20 | sed 's/^/      /' >&2 || true
        tail -n 25 "$log" | sed 's/^/      | /' >&2
        failed+=("$s")
        [ "$KEEP_GOING" = 1 ] || break
    fi
done

if [ "$GITHUB" = 1 ] && [ "${#failed[@]}" = 0 ]; then
    say "[GitHub] release-app.yml dry_run для Windows и macOS ..."
    need gh "GitHub CLI"
    gh auth status >/dev/null 2>&1 || die "gh is not logged in (gh auth login)"
    ref="$(git -C "$ROOT" rev-parse --abbrev-ref HEAD)"
    [ "$(git -C "$ROOT" rev-parse HEAD)" = "$(git -C "$ROOT" ls-remote origin "refs/heads/$ref" | cut -f1)" ] \
        || say "      ВНИМАНИЕ: $ref на origin не совпадает с проверенным коммитом $SHA"
    ids=()
    for p in windows macos; do
        gh workflow run release-app.yml -R "$GH_REPO" --ref "$ref" -f version="$VERSION" -f platforms="$p"
    done
    sleep 8
    mapfile -t ids < <(gh run list -R "$GH_REPO" -w release-app.yml -e workflow_dispatch -b "$ref" -L 2 --json databaseId --jq '.[].databaseId')
    for id in "${ids[@]}"; do say "      https://github.com/$GH_REPO/actions/runs/$id"; done
    if [ "$GITHUB_WAIT" = 1 ]; then
        for id in "${ids[@]}"; do
            gh run watch -R "$GH_REPO" "$id" --exit-status >/dev/null 2>&1 || failed+=("github-run-$id")
        done
    else
        say "      тег — только когда оба прогона зелёные"
    fi
fi

say ""
say "Время: $(fmt $((SECONDS - start_all))). Файлы: $RUN/release"
if [ "${#failed[@]}" = 0 ]; then
    if [ -n "$ONLY" ]; then say "ИТОГ: выбранные этапы прошли (${selected[*]}) — для выпуска нужен полный прогон"
    else say "ИТОГ: можно выпускать (Linux и Android). Windows и macOS — release-app.yml dry_run на GitHub."; fi
    exit 0
fi
say "ИТОГ: выпускать нельзя — сломано: ${failed[*]}"
exit 1
