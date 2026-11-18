<!-- SPDX-License-Identifier: MPL-2.0 -->

# Troubleshooting

## Reporting a problem

1. Repeat what does not work.
2. Settings → Diagnostics → **Export diagnostic report**. Kinolune saves a zip archive with logs and crash reports (usually to Downloads) and shows where.
3. Press **Open GitHub issue** — a pre-filled bug report opens in your browser. Describe what you did and what happened, and attach the archive.

Kinolune never sends anything by itself. You can open and read the archive first: passwords and keys are masked in the logs, but the names of your files and folders are there — remove anything you do not want to share. GitHub issues are public.

Logs are written while "Keep logs and crash reports" is on. "Delete logs and crash reports" erases them.

## Common questions

**A movie got the wrong title.** Open its page → "Fix match" and pick the right one. More in [Recognition](03-recognition.md).

**Episodes are in the wrong places.** "Fix match" → "Type and numbering" → "Change numbering", or choose another episode order on the show page.

**New files do not appear.** Settings → Library & sources → "Scan now", or "Rescan" on the Sources screen. Check that "Add files to the library" is on for that source.

**A network folder shows "Unavailable".** Press "Check connection" on the Sources screen and read the reason. Make sure the computer or NAS is on and on the same network. For SMB servers that hide their share list, type the share name ([Sources](02-sources.md#hidden-shares)).

**Video stutters over the network.** "Speed test" on the Sources screen tells you whether the speed is enough for 4K. For a trip, download the movie beforehand.

**"This file can't be played".** ISO images do not play yet; the message gives the reason. For Blu-ray folders (`BDMV`) the longest playlist plays.

**No sound, or the wrong subtitles.** Choose the track in the Audio or Subtitles tab — the choice is remembered for the show. Automatic choice is in Settings → Playback → Languages and sound.

**Linux: "Video playback needs the mpv library".** This only happens with self-built copies: the AppImage and tar.gz already include libmpv. Install libmpv with your package manager as the bar suggests.

**Linux: a warning that passwords are kept in an encrypted file.** The system has no Secret Service (GNOME Keyring or KWallet). Kinolune works and keeps passwords in an encrypted file. To use the system storage, install and run GNOME Keyring or KWallet.

**Simplified mode: only "Folder on this device" is available.** The Kinolune core did not start. Reinstall the app from the releases page; if that does not help, report the problem (above).

**Android: "App not installed" when updating.** The old version was signed with a different key. Uninstall it and install the new one.

**Windows: "Windows protected your PC".** More info → Run anyway. See [Installing](01-install.md#windows).

**macOS: "Kinolune can't be opened".** See step 2 in [Installing](01-install.md#macos).

**The computer does not see the NAS in the list.** Type the address. On Windows, allow Kinolune on private networks in the firewall.

**Devices do not pair.** Both must be on the same network, and the code is valid for a few minutes. Guest networks and "client isolation" on the router stop devices from seeing each other.

**Where is the version?** Settings → About → This build: the version, the build variant (full or store) and the core version. Include them in a bug report.
