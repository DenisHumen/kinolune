<!-- SPDX-License-Identifier: MPL-2.0 -->

# Installing and updating

Download Kinolune from [GitHub Releases](https://github.com/DenisHumen/kinolune/releases). Pick the newest release and, under "Assets", download the file for your device:

| Device | File |
|---|---|
| Windows 10 and 11 (64-bit) | `kinolune-<version>-windows-x64-setup.exe` — installer (recommended), or `…-windows-x64.zip` — portable copy |
| Mac with Apple silicon (M1 or newer) | `kinolune-<version>-macos-arm64.dmg` |
| Mac with an Intel processor | `kinolune-<version>-macos-x86_64.dmg` |
| Linux (x86_64) | `kinolune-<version>-x86_64.AppImage` (recommended), `…-linux-amd64.deb` or `…-linux-x86_64.rpm` |
| Linux on ARM (aarch64: Raspberry Pi 5, ARM laptops) | `kinolune-<version>-aarch64.AppImage`, `…-linux-arm64.deb` or `…-linux-aarch64.rpm` |
| Android phone, tablet, Android TV, Google TV | `kinolune-<version>-android-universal.apk` works everywhere. If you know the processor: `arm64-v8a` (nearly all modern devices), `armeabi-v7a` (old and budget TV boxes), `x86_64` (emulators) |

There are no builds for iPhone, iPad or Apple TV yet.

GitHub builds are "full" builds: they include in-app updates and Twitch downloads and recording. Store builds (once they exist) are updated by the store.

## Windows

1. Run `kinolune-<version>-windows-x64-setup.exe`. No administrator rights are needed: Kinolune is installed for your user only, with shortcuts in the Start menu and on the desktop.
2. The build is not code-signed yet, so Windows may show "Windows protected your PC". Click **More info → Run anyway**. This is needed once per downloaded version.
3. The first time Kinolune searches for network folders, Windows asks about the firewall. Allow Kinolune on **private** networks, or computers and NAS boxes on your home network will not be found.

To uninstall: Settings → Apps → Kinolune → Uninstall. Your library and settings stay; network passwords are kept in Windows Credential Manager.

**Portable copy (zip).** Unpack the whole archive to any folder you can write to (for example Documents\Kinolune) and run `Kinolune.exe`. Keep the exe together with the other files. A portable copy does not update itself: the app tells you about a new version, and you unpack it yourself.

## macOS

macOS 12 or newer is required.

1. Open the downloaded `.dmg` and drag **Kinolune.app** onto the **Applications** folder in the same window.
2. The app is not notarized by Apple yet, so macOS blocks the first start ("Kinolune can't be opened" or "Apple could not verify…"). Allow it **once**, either way:
   - **Without Terminal:** in Finder, right-click (or Control-click) Kinolune.app → **Open** → **Open** again. On macOS 15 and newer, if there is no Open button: System Settings → Privacy & Security → next to the Kinolune message, **Open Anyway**.
   - **In Terminal** (Applications → Utilities → Terminal), paste this command and press Return:

     ```bash
     xattr -dr com.apple.quarantine /Applications/Kinolune.app
     ```

3. Start Kinolune from Applications. macOS may ask whether Kinolune may find devices on the local network and open folders such as Downloads or Documents. Allow it, or network folders, media servers and sync will not work.

If you install a new version by hand from a DMG, step 2 is needed again. Updating from inside the app (below) does everything for you.

## Linux

You need a distribution with glibc 2.39 or newer (for example Ubuntu 24.04+, Fedora 40+), GTK 3, and PipeWire or PulseAudio. The video engine (libmpv and FFmpeg) is included.

1. Make the file executable: tick "Allow executing file as program" in its properties, or run

   ```bash
   chmod +x kinolune-*.AppImage
   ```

2. Start the AppImage with a double-click or from a terminal.

If the AppImage does not start and mentions FUSE, install `libfuse2` (on Ubuntu 24.04: `libfuse2t64`), or use the `tar.gz`: unpack it and run `./kinolune`.

`kinolune://` links and "Open With → Kinolune" appear once the AppImage is integrated into your menu (for example with Gear Lever or AppImageLauncher).

## Android, Android TV and Google TV

1. Download the APK to the device (on a TV, a downloader app or a USB stick is handy).
2. Open the file. Android asks you to allow installs from this source (the browser or file manager): allow it and go back.
3. Tap Install.

On Android TV and Google TV the app appears in the launcher with the Kinolune banner and works with the remote right away.

If Android says the app was "not installed" while Kinolune is already there, the old version was signed with a different key (as the very first alphas were). Uninstall the old version and install the new one. The library is then scanned again.

## First start

On the first start a five-step assistant opens: language, "How will you watch?" (currently "Just this app"), profile name and colour, the first source, and pairing with your other devices. Any step can be skipped with "Skip". "Try the demo library" shows the app with made-up movies and saves nothing.

You can open the assistant again: Settings → About → Setup assistant.

## Updates

**How you learn about a new version.** A bar appears at the top of the app: "Version X is available" with "What's new", "Download" and "Later". The same is shown in Settings → About → Updates, which also has "Check for updates".

**How to update.**

| Installed as | What happens |
|---|---|
| Windows installer, macOS, Linux AppImage | The update is downloaded and verified in the background. When it is ready, press "Restart". On Windows it is also installed when you quit the app. Kinolune never restarts by itself in the middle of a movie |
| Android | "Download" → system installer → Update. If Android asks, allow Kinolune to install apps ("Allow installs", then "Try again") |
| Windows zip, Linux tar.gz, .deb/.rpm packages, Flatpak, Snap | The app only tells you about the new version; "Download" opens the right file in your browser |

Every downloaded file is checked against the release signature. If the file is damaged or tampered with, Kinolune deletes it and installs nothing.

**Settings** (Settings → About → Updates):

- "Check for updates automatically" — 5 seconds after start and every 6 hours. Turn it off to check only with the button.
- "When a new version comes out": "Update automatically" (default) or "Only tell me".
- "Update channel": "Stable releases" or "Pre-releases too (alpha, beta)". While you run an alpha or beta, pre-releases are selected by default, so there is nothing to change.

Your library, watch progress and settings are kept across updates.
