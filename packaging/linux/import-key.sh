#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
#
# Import the repository signing key (PKG-023) from $PA_PKG_REPO_GPG_KEY into
# $GNUPGHOME and print its fingerprint. The secret holds the passphrase-less
# private key, ASCII-armored (base64 of it is accepted too). The key must be
# the one whose public half is committed (kinolune-repo-public.asc), so a
# wrong secret fails here instead of producing a repository nobody can use.
# PA_REPO_KEY names another public key file (tools/release/precheck.sh signs
# with a throwaway key; build-packages.sh takes the same variable).
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
die() { printf 'import-key.sh: %s\n' "$*" >&2; exit 1; }

: "${GNUPGHOME:?set GNUPGHOME to a private temporary directory}"
[ -n "${PA_PKG_REPO_GPG_KEY:-}" ] || die "PA_PKG_REPO_GPG_KEY is empty (repository secret missing)"
mkdir -p -m 700 "$GNUPGHOME"

if printf '%s' "$PA_PKG_REPO_GPG_KEY" | grep -q -- '-----BEGIN PGP PRIVATE KEY BLOCK-----'; then
    printf '%s\n' "$PA_PKG_REPO_GPG_KEY" | gpg --batch --quiet --import 2>/dev/null
else
    printf '%s' "$PA_PKG_REPO_GPG_KEY" | base64 -d | gpg --batch --quiet --import 2>/dev/null
fi

pub="${PA_REPO_KEY:-$here/kinolune-repo-public.asc}"
[ -f "$pub" ] || die "no public key: $pub"
want=$(gpg --batch --with-colons --show-keys "$pub" | awk -F: '/^fpr:/ {print $10; exit}')
gpg --batch --list-secret-keys "$want" >/dev/null 2>&1 \
    || die "the secret key is not the committed repository key $want"
printf '%s\n' "$want"
