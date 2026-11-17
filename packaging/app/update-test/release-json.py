#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
#
# The CI update test (PKG-022, release-app.yml `windows-update`,
# `macos-update`): writes <dir>/releases, a stand-in for the GitHub releases
# API that lists one pre-release app-v<version> with every file of
# <dir>/assets, served from <base>/assets/<name>. The app under test is built
# with --dart-define=KINOLUNE_UPDATES_API=<base>/releases.
#
#   release-json.py <dir> <version> <base-url>
import json
import os
import sys


def main() -> None:
    root, version, base = sys.argv[1], sys.argv[2], sys.argv[3].rstrip("/")
    assets_dir = os.path.join(root, "assets")
    names = sorted(os.listdir(assets_dir))
    for needed in ("SHA256SUMS", "SHA256SUMS.minisig"):
        if needed not in names:
            sys.exit(f"release-json.py: {needed} missing from {assets_dir}")
    tag = f"app-v{version}"
    release = {
        "tag_name": tag,
        "name": f"Kinolune {version}",
        "draft": False,
        "prerelease": "-" in version,
        "body": "CI update test (PKG-022).\n\n## Downloads\n",
        "html_url": f"{base}/{tag}",
        "published_at": "2026-11-17T00:00:00Z",
        "assets": [
            {
                "name": n,
                "size": os.path.getsize(os.path.join(assets_dir, n)),
                "browser_download_url": f"{base}/assets/{n}",
            }
            for n in names
        ],
    }
    with open(os.path.join(root, "releases"), "w", encoding="utf-8") as f:
        json.dump([release], f, indent=2)
    print(json.dumps(release, indent=2))


if __name__ == "__main__":
    main()
