# SPDX-License-Identifier: MPL-2.0
#
# PKG-025: the build environment of tools/release/precheck.sh, close to the
# ubuntu-24.04 runner of release-app.yml: the packages of its Linux job plus
# what the runner image brings (git, curl, Node.js, Python with PyYAML, rpm
# for rpmsign, gpg, zstd) and a C toolchain for libsmb2. Flutter, Rust, Java
# and the Android SDK come from the host as volumes; the container runs as the
# host user. Nothing from this image ships.
FROM ubuntu:24.04
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
      build-essential ca-certificates clang cmake curl file git gnupg jq libasound2t64 libdrm2 libegl1 \
      libgtk-3-dev liblzma-dev libpulse0 libsecret-1-dev libstdc++-12-dev libva-drm2 libva2 ninja-build \
      nodejs patch patchelf pkg-config python3 python3-yaml rpm unzip xz-utils zip zstd \
    && rm -rf /var/lib/apt/lists/*
