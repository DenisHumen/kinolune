#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
#
# CI-only Gradle network settings (CI-013).
#
# Maven Central answers bursts from shared GitHub runners with HTTP 429. Gradle
# retries 429, 408, 5xx and timeouts, but by default only 3 times from 1 s; after
# that it marks the repository "disabled due to earlier error" and the whole
# build fails before compiling anything. Here: 8 attempts with exponential
# backoff from 2 s (2+4+…+128 ≈ 4 min worst case) and longer HTTP timeouts.
#
# Written to the Gradle user home (~/.gradle/gradle.properties), so it applies
# to the Flutter wrapper, the AAR build and every included build, and never
# to a developer's local build: nothing in the projects' own gradle files
# changes. Dependency and wrapper caching is gradle/actions/setup-gradle in
# each workflow; run this after it.
#
# Usage: tools/ci/gradle-network.sh
set -euo pipefail

home="${GRADLE_USER_HOME:-$HOME/.gradle}"
mkdir -p "$home"
props="$home/gradle.properties"

# Idempotent: a second call in the same job adds nothing.
if [ -f "$props" ] && grep -q '^# CI-013 network' "$props"; then
  exit 0
fi

{
  echo '# CI-013 network: retries with backoff and longer timeouts'
  # Dependency resolution (ErrorHandlingModuleComponentRepository): attempts
  # before the repository is disabled, and the first pause (doubled each time).
  echo 'systemProp.org.gradle.internal.repository.max.tentatives=8'
  echo 'systemProp.org.gradle.internal.repository.initial.backoff=2000'
  echo 'systemProp.org.gradle.internal.http.connectionTimeout=60000'
  echo 'systemProp.org.gradle.internal.http.socketTimeout=120000'
} >> "$props"

echo "Gradle network settings -> $props"
