<!-- SPDX-License-Identifier: MPL-2.0 -->

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="design/brand/export/logo/lockup-dark.png">
    <img alt="Kinolune" src="design/brand/export/logo/lockup-light.png" width="420">
  </picture>
</p>

<p align="center"><b>A free media player for your home: movies, TV shows, anime, music and Twitch on all your devices — no cloud, no accounts.</b></p>

<p align="center">
  <a href="LICENSE"><img alt="License: MPL-2.0" src="https://img.shields.io/badge/license-MPL--2.0-blue.svg"></a>
  <a href="https://github.com/DenisHumen/kinolune/actions/workflows/rust.yml"><img alt="Rust" src="https://github.com/DenisHumen/kinolune/actions/workflows/rust.yml/badge.svg"></a>
  <a href="https://github.com/DenisHumen/kinolune/actions/workflows/flutter.yml"><img alt="Flutter" src="https://github.com/DenisHumen/kinolune/actions/workflows/flutter.yml/badge.svg"></a>
  <a href="https://github.com/DenisHumen/kinolune/actions/workflows/hygiene.yml"><img alt="Hygiene" src="https://github.com/DenisHumen/kinolune/actions/workflows/hygiene.yml/badge.svg"></a>
  <a href="https://github.com/DenisHumen/kinolune/releases"><img alt="Latest release" src="https://img.shields.io/github/v/release/DenisHumen/kinolune?include_prereleases&amp;label=release"></a>
  <img alt="Platforms" src="https://img.shields.io/badge/platforms-Linux%20%7C%20Windows%20%7C%20macOS%20%7C%20Android-informational">
</p>

<p align="center"><a href="README.md">Русский</a> · <b>English</b></p>

<p align="center">
  <img alt="Kinolune home screen on a computer" src="docs/screenshots/0.1/desktop-home.png" width="640">
</p>
<p align="center">
  <img alt="Kinolune on a phone" src="docs/screenshots/0.1/phone-home.png" height="300">
  &nbsp;
  <img alt="Kinolune on a TV" src="docs/screenshots/0.1/tv-home.png" height="300">
</p>

<sub>Screenshots show a demo library with our own placeholder artwork, no third-party art.</sub>

## Features

- a library of movies, TV shows and anime with accurate recognition from file and folder names;
- playback of almost any format, HDR/Dolby Vision, audio passthrough;
- sources: local folders, SMB, NFS, WebDAV, cloud drives, Plex/Jellyfin/Emby, our own media server;
- a music player (mp3, flac, alac, opus, dsd…), gapless playback, lyrics;
- a comfortable Twitch client: live streams with chat, VODs, favourite streamers;
- continue watching on any device, with no cloud: everything stays local;
- our own NAS Linux distribution with a simple installer and RAID.

The project runs no servers and has no accounts: the app runs on your devices, requests to TMDB and other services go directly from your device, and there is no telemetry. The product name is **Kinolune** (ADR-0011); the repository was called `project_alpha` until 2026-11-15, and GitHub redirects old links.

## Status

🚧 **Early alpha builds** for Linux, Windows, macOS and Android. Current progress: [docs/STATUS.md](docs/STATUS.md); the full plan: [docs/ROADMAP.md](docs/ROADMAP.md); live per-task progress: [the `progress` branch](https://github.com/DenisHumen/kinolune/blob/progress/README.md) (documents are in Russian).

## Download

Builds are in [GitHub Releases](https://github.com/DenisHumen/kinolune/releases): Linux (`.deb`, `.rpm`, AppImage), Windows (installer), macOS (`.dmg`), Android (APK).

### Install on Linux

Ubuntu 24.04+, Debian 13+, Linux Mint 22+:

```sh
curl -fsSL https://denishumen.github.io/kinolune/apt/kinolune.sources | sudo tee /etc/apt/sources.list.d/kinolune.sources >/dev/null
sudo apt update && sudo apt install kinolune
```

Fedora:

```sh
curl -fsSL https://denishumen.github.io/kinolune/rpm/kinolune.repo | sudo tee /etc/yum.repos.d/kinolune.repo >/dev/null
sudo dnf install kinolune
```

Kinolune then updates with the system (`sudo apt upgrade`, `sudo dnf upgrade`). The `.deb` and `.rpm` from a release add the same repository when installed.

## Contributing

Pull requests are welcome: see [CONTRIBUTING](.github/CONTRIBUTING.md) (English summary at the end) and the [Code of Conduct](.github/CODE_OF_CONDUCT.md). AI agents: see [AGENTS.md](AGENTS.md). Questions and ideas go to [Discussions](https://github.com/DenisHumen/kinolune/discussions). Report vulnerabilities privately only, see [SECURITY](.github/SECURITY.md).

Where things live:

| Directory | What's inside |
|---|---|
| [`core/`](core/) | Rust core: sources, recognition, library, sync |
| [`clients/`](clients/) | the Flutter app and native players (Android, Apple, desktop) |
| [`server/`](server/) | the media server |
| [`distro/`](distro/) | Kinolune OS, the NAS distribution |
| [`packaging/`](packaging/) | packages and installers |
| [`design/`](design/) | brand, tokens, icons |
| [`config/`](config/) | the signed remote config |
| [`tools/`](tools/) | library builds, releases, QA and test media, roadmap |
| [`docs/`](docs/) | documentation, ADRs, plans |

## Donate

Donation platforms are still being set up; once they are live, links will appear here and on the repository's **Sponsor** button. Bug reports, wrong-match examples and pull requests already help a lot.

## Documentation

Most documentation is in Russian for now; the user guide is in Russian and English.

- [docs/user/](docs/user/README.md): user guide in Russian and English (installing, sources, recognition, playback)
- [docs/STRATEGY.md](docs/STRATEGY.md): strategy and key decisions
- [docs/ROADMAP.md](docs/ROADMAP.md): detailed plan and task checklist
- [docs/adr/](docs/adr/): architecture decision records
- [docs/dev/architecture.md](docs/dev/architecture.md): architecture for developers
- [docs/research/](docs/research/): research notes
- [docs/legal/privacy-policy.md](docs/legal/privacy-policy.md): privacy policy

## License

Code is licensed under [MPL-2.0](LICENSE). The repository follows the [REUSE](https://reuse.software/) specification: each file's licence is in its header or in [REUSE.toml](REUSE.toml), licence texts are in [LICENSES/](LICENSES/).

Kinolune is not affiliated with Infuse/Firecore, Plex, Jellyfin, Emby, Twitch, TMDB or any other service mentioned; their names belong to their owners.
