SPDX-License-Identifier: MPL-2.0

Kinolune for Windows (portable, x64)
====================================

Requirements: Windows 10 or 11, 64-bit (x64).

Start
-----
1. Unpack the whole zip to any folder you can write to, for example
   Documents\Kinolune. Keep all files together: Kinolune.exe needs the DLLs
   and the "data" folder next to it.
2. Double-click Kinolune.exe.

The build is not code-signed yet, so Windows SmartScreen may show
"Windows protected your PC" on the first start. Click "More info", then
"Run anyway". This is needed once per downloaded version.
If Windows marked the zip as downloaded from the internet, you can also
right-click the zip -> Properties -> tick "Unblock" -> OK before unpacking.

Your library, settings and logs are kept in
%APPDATA%\app.kinolune\Kinolune; passwords for network sources are kept in
the Windows Credential Manager. Deleting the program folder does not delete
them.

Update: download the new zip from the GitHub Releases page and unpack it over
the old folder (or into a new one). Or use the installer
(kinolune-<version>-windows-x64-setup.exe): an installed copy updates itself.

Licences: Kinolune is MPL-2.0. Third-party notices and licence texts are in
the "licenses" folder; libmpv and FFmpeg are LGPL-2.1-or-later and can be
replaced with your own build of the same version (see licenses\libmpv).

Source code: https://github.com/DenisHumen/kinolune
