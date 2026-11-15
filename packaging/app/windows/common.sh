# SPDX-License-Identifier: MPL-2.0
#
# Shared helpers of packaging/app/windows/*.sh (PKG-020). Sourced, not run.

# A Python 3 that really runs: on Windows `python3` may be the Microsoft Store
# stub, so try both names.
py=""
for _cand in python3 python; do
    if command -v "$_cand" >/dev/null 2>&1 && "$_cand" -c 'import sys; sys.exit(sys.version_info < (3, 8))' >/dev/null 2>&1; then
        py="$(command -v "$_cand")"
        break
    fi
done
[ -n "$py" ] || { echo "python 3.8+ not found" >&2; exit 1; }
unset _cand

# SHA-256 of a file (no sha256sum dependency).
sha() {
    "$py" -I -c 'import hashlib, sys; print(hashlib.sha256(open(sys.argv[1], "rb").read()).hexdigest())' "$1" | tr -d '\r'
}

# "<file>\t<imported dll>" lines for PE files, without CR.
pe_imports() {
    "$py" -I "$here/pe_imports.py" "$@" | tr -d '\r'
}
