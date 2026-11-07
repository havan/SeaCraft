## Download

| You have | Download | Start it |
| --- | --- | --- |
| Windows, 64-bit | `seacraft-*-x86_64-pc-windows-msvc.zip` | Unzip, then double-click `seacraft-gui.exe` |
| Linux, most distributions | `SeaCraft-*-x86_64.AppImage` | `chmod +x` it, then run it |
| Linux with Flatpak | `SeaCraft-*-x86_64.flatpak` | `flatpak install --user` it, then find SeaCraft in your menu |
| Linux, plain folder | `seacraft-*-x86_64-unknown-linux-gnu.tar.gz` | Unpack, then run `./seacraft-gui` |

To begin, choose **Open an example** on the start screen. `SHA256SUMS` holds the checksums.

**Windows:** the programs are not signed, so SmartScreen may warn you; choose *More info*, then *Run anyway*,
if you trust this page. This build was compiled and tested automatically, but nobody has opened the GUI on
Windows by hand yet. Please report what you find.

**Flatpak on NVIDIA:** the window needs the `org.freedesktop.Platform.GL.nvidia` extension that matches your
driver. If SeaCraft fails to open, run `flatpak update`, which normally installs it.

## Not validated

Most calculations have not been checked against tank tests, sea trials or approved software, and results may
be wrong. A `PASS` against a criterion is not approval. Do not rely on SeaCraft for design or safety decisions
without independent checks by a qualified person. It is provided as is, with no warranty; see `DISCLAIMER.md`.

## Licence and source

SeaCraft is free software under `GPL-3.0-or-later`. The source of this release is the tag it was built from,
and GitHub attaches its source archives below. See `CHANGELOG.md` for what changed.
