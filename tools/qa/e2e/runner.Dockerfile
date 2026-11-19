# SPDX-License-Identifier: MPL-2.0
#
# QA-016: the throwaway Linux desktop the live e2e scenarios run in (the
# build server has no GTK). The packages of tool/perf/linux.Dockerfile
# (QA-006) plus ffmpeg, Python and unzip for building the test library,
# and a Russian locale for the screenshots. Flutter, Rust and the pub
# cache come from the host as volumes; it runs as the host user. ffmpeg
# (GPL in Ubuntu) only makes test files here; nothing of it ships.
FROM ubuntu:24.04
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
      build-essential ca-certificates clang cmake curl ffmpeg git libgl1-mesa-dri libgtk-3-dev \
      liblzma-dev libmpv-dev libsecret-1-dev libstdc++-12-dev locales ninja-build pkg-config procps \
      python3 unzip x11-apps xauth xvfb xz-utils \
    && sed -i 's/^# *\(ru_RU.UTF-8\|en_US.UTF-8\)/\1/' /etc/locale.gen && locale-gen \
    && rm -rf /var/lib/apt/lists/*
