# SPDX-License-Identifier: MPL-2.0
#
# QA-006: a throwaway Linux desktop for tool/perf_bench.sh on hosts without
# GTK (a build server). Same packages as the Linux job of flutter.yml, plus
# Xvfb and Mesa (llvmpipe). Flutter, Rust and the pub cache come from the
# host as volumes; run as the host user:
#
#   docker build -t kinolune-perf-linux -f tool/perf/linux.Dockerfile tool/perf
#   docker run --rm --name kinolune-perf -u "$(id -u):$(id -g)" \
#     -e HOME=$HOME -v $HOME:$HOME -w "$PWD" kinolune-perf-linux \
#     bash -c 'export PATH=$HOME/opt/flutter/bin:$HOME/.cargo/bin:$PATH; tool/perf_bench.sh linux'
FROM ubuntu:24.04
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
      ca-certificates clang cmake curl git libgl1-mesa-dri libgtk-3-dev liblzma-dev \
      libmpv-dev libsecret-1-dev libstdc++-12-dev ninja-build pkg-config unzip \
      xauth xvfb xz-utils \
    && rm -rf /var/lib/apt/lists/*
