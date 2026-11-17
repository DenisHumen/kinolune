SPDX-License-Identifier: MPL-2.0

Kinolune for macOS (Apple silicon and Intel, macOS 12 or newer)
================================================================

This build is not notarized: it is signed ad hoc, without an Apple Developer
ID. macOS therefore blocks the first start ("Kinolune can't be opened" or
"Apple could not verify ..."). To open it once:

  1. Open kinolune-<version>-macos-<arch>.dmg and drag Kinolune.app onto
     the Applications link next to it.
  2. Either right-click (Control-click) Kinolune.app, choose Open, then Open
     again in the dialog; on macOS 15 or newer, if there is no Open button,
     open System Settings > Privacy & Security and click "Open Anyway" next
     to the Kinolune message.
     Or, in Terminal:
       xattr -dr com.apple.quarantine /Applications/Kinolune.app
  3. After that it starts normally.

macOS may ask whether Kinolune may find devices on the local network (for
shared folders, media servers and sync) and access folders such as Downloads
or Documents when you add them: allow it for those features to work.

libmpv, FFmpeg (our LGPL build) and libsmb2 are included in the app.

Licences: LICENSE (MPL-2.0) covers Kinolune itself; THIRD-PARTY-NOTICES.md and
the files next to it cover third-party code. libmpv/ covers the bundled libmpv
and FFmpeg (LGPL-2.1-or-later), libsmb2/ the bundled libsmb2
(LGPL-2.1-or-later); their corresponding source is attached to the same
GitHub Release. All of this is in Kinolune.app/Contents/Resources/licenses.

Source code: https://github.com/DenisHumen/kinolune
