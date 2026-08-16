# TheXTech — Corresponding Source (GPL-3.0)

TheXTech is licensed GPL-3.0-or-later. kUI ships an unmodified, prebuilt
aarch64 binary of the engine (lifted from the PortMaster MegaVerse TheXTech
port). Under the GPL, the complete corresponding source code must be available.
kUI applies no source-level modifications to the engine, so the corresponding
source is TheXTech upstream at the version described below.

## Upstream

- Project: TheXTech (WohlSoft team)
- Repository: https://github.com/TheXTech/TheXTech
- License: GPL-3.0-or-later

Because TheXTech builds against submodules, obtain a matching tree with:

```
git clone --recurse-submodules https://github.com/TheXTech/TheXTech.git
git checkout v1.3.7            # then re-sync submodules:
git submodule update --init --recursive
```

## Version this binary corresponds to

Best identifier: **TheXTech 1.3.7** (upstream git tag `v1.3.7`).

Evidence and reasoning:
- The PortMaster port's own notes (`thextech.md`) state: *"This port is
  1.3.7-stable version with bugfix for rocknix gamepad detection."* There is no
  literal `1.3.7-stable` git tag upstream; the canonical release is tagged
  `v1.3.7` (released 2025-01-20).
- The shipped `thextech` binary and port files are dated **2025-01-22**, which
  coincides with upstream `v1.3.7-hotfix1` (2025-01-22), described upstream as a
  **"PortMaster build fix."** So the shipped binary most plausibly corresponds
  to `v1.3.7` with the `v1.3.7-hotfix1` PortMaster fix applied.
- Build recipe per the port's `thextech.md` (Ubuntu 20.04):
  `cmake -DCMAKE_BUILD_TYPE=MinSizeRel -DPORTMASTER=ON ..`
- The binary reports build target `GNU/Linux 3.7.0`, aarch64, dynamically linked
  against `libSDL2-2.0.so.0` and `libGLESv2.so.2`; BuildID
  `0d429ba64ee771af4837aa9e036a52b352ac6866`.

**[FLAG] Uncertainty:** the exact upstream commit is not embedded in the binary,
and the port additionally carries a "rocknix gamepad detection" bugfix that may
be a port-local patch not present in any upstream tag. The precise corresponding
source is therefore either `v1.3.7` or `v1.3.7-hotfix1`; if an exact
byte-for-byte match is required, request the source (including any port-local
patch) from the port author.

## Where to get matching source if the tag is later found insufficient

1. Upstream tags: https://github.com/TheXTech/TheXTech/tags
   (candidates: `v1.3.7`, `v1.3.7-hotfix1`, `v1.3.7-hotfix2`)
2. PortMaster MegaVerse port author: **ddrsoul** (porter, per `port.json`).
   Any port-specific patches (e.g. the rocknix gamepad fix) would live in the
   port packaging rather than upstream.
3. WohlSoft submodule sources are pinned by the submodule commits recorded in
   the TheXTech tree at the chosen tag.

## Copyleft (LGPL) statically-linked dependencies

Besides TheXTech's own GPL-3.0 code, the shipped `thextech` binary statically
links several **copyleft** libraries (see `THIRD-PARTY.md` for the binary
evidence). Because they are copyleft, their corresponding source must remain
available. All are unmodified upstream versions vendored via WohlSoft's
AudioCodecs collection (https://github.com/WohlSoft/AudioCodecs) or the WohlSoft
component repos; their complete source is available at the upstream repositories
below:

- **game-music-emu (GME)** — LGPL-2.1-or-later.
  Upstream source: https://github.com/libgme/game-music-emu
- **FluidLite** — LGPL-2.1-or-later.
  Upstream source: https://github.com/divideconcept/FluidLite
- **libADLMIDI** (OPL3 FM-synth MIDI) — LGPL-3.0-or-later OR GPL-3.0-or-later
  (repo `LICENSE` is LGPLv3, with `LICENSE.GPL-3.txt` / `LICENSE.LGPL-2.1.txt`
  also shipped). Upstream source: https://github.com/Wohlstand/libADLMIDI
- **libOPNMIDI** (YM2612/OPN2 FM-synth MIDI) — LGPL-3.0-or-later OR
  GPL-3.0-or-later (same license layout as libADLMIDI).
  Upstream source: https://github.com/Wohlstand/libOPNMIDI
- **FreeImageLite** (WohlSoft modded FreeImage — image loading) — triple
  licensed FreeImage Public License v1.0 OR GPL-2.0 OR GPL-3.0 (the repo ships
  `license-fi.txt`, `license-gplv2.txt`, and `license-gplv3.txt`). If the GPL
  option is relied on rather than the FIPL, the GPL corresponding source is the
  upstream tree at https://github.com/WohlSoft/libFreeImage (the same code is
  also carried in the Moondust/PGE ecosystem).

The LGPL libraries further allow a user to relink the application against a
modified version of the library; the engine links them (and the FIPL/GPL
FreeImageLite) as ordinary object code within the redistributable single binary,
and the source needed to exercise the corresponding-source and relink rights is
the upstream source cited here.

## Sources
- Port notes: `~/dev/thextech-runtime/port/thextech/thextech.md`
- Port metadata: `~/dev/thextech-runtime/port/thextech/port.json` (porter: ddrsoul)
- Shipped binary metadata: `~/dev/thextech-runtime/port/thextech/thextech` (`readelf`/`file`; dated 2025-01-22)
- Upstream releases/tags: https://github.com/TheXTech/TheXTech/releases
