# Solarus runtime — third-party components

This directory documents the **Solarus engine runtime** that kUI ships for its
Port Forge action-RPG quests. The runtime was lifted from the PortMaster
`solarus-1.6.5` runtime (a LuaJIT-enabled Solarus build; the embedded build
path in the binaries is `/root/compile/solarus_luajit/build`).

The Solarus engine itself (`solarus-1.6.5` launcher binary + `libsolarus.so.1`)
is licensed **GPL-3.0** — its license is in [`LICENSE`](./LICENSE) and its
corresponding source is described in [`SOURCE.md`](./SOURCE.md). This file
credits the **other** libraries the runtime bundles or dynamically links.

All copyright lines below were fetched verbatim from each project's upstream
license file (see the cited URL). Versions marked "unverified" could not be
pinned exactly from the shipped binaries; the license and holder are still
correct but the exact release/year range is flagged.

## What the runtime actually links

`readelf -d libsolarus.so.1` (aarch64) reports these `NEEDED` libraries:

```
libSDL2-2.0.so.0   libSDL2_image-2.0.so.0   libSDL2_ttf-2.0.so.0
libopenal.so.1     libphysfs.so.1           libvorbis.so.0
libvorbisfile.so.3 libmodplug.so.1          libluajit-5.1.so.2
libstdc++ libgcc_s libpthread libm libc (toolchain / system)
```

- **Bundled inside the runtime** — kUI's repacked `solarus.squashfs` folds these
  in alongside `libsolarus.so.1`, so Port Forge ports ship only the quest (no
  per-port `libs.aarch64/`, unlike the upstream PortMaster ports):
  `libluajit-5.1.so.2`, `libphysfs.so.1`, `libmodplug.so.1`.
- **Device-provided** (supplied by the host OS on the handheld, linked but not
  bundled here): SDL2, SDL2_image, SDL2_ttf, OpenAL Soft, libvorbis,
  libvorbisfile, libogg.
- **Transitive image/compression codecs** pulled in via SDL2_image on the
  device: libpng, zlib (and, depending on the device image, libjpeg / libwebp /
  libtiff — not enumerated here).

They are all credited below regardless of who ships the actual `.so`, because
the runtime cannot run without them.

---

## LuaJIT

- **Component:** `libluajit-5.1.so.2` (bundled). Version reported by the binary:
  **LuaJIT 2.1.0-beta3**.
- **License:** MIT.
- **Copyright:** `Copyright (C) 2005-2017 Mike Pall. All rights reserved.`
  (2.1.0-beta3-era `COPYRIGHT`; the current `v2.1` tree reads
  `Copyright (C) 2005-2026 Mike Pall`. LuaJIT also embeds Lua 5.1/5.2 code,
  `Copyright (C) 1994-2012 Lua.org, PUC-Rio`, likewise MIT.)
- **URL:** https://luajit.org/ — source https://github.com/LuaJIT/LuaJIT
- **Source cited:** https://raw.githubusercontent.com/LuaJIT/LuaJIT/v2.1/COPYRIGHT
- **Flag:** the exact `2005-2017` year range is the documented beta3-era value;
  the MIT license text is unchanged across LuaJIT versions.

## PhysicsFS (PhysFS)

- **Component:** `libphysfs.so.1` (bundled). Version from the binary build path:
  **3.0.1**.
- **License:** zlib/libpng license.
- **Copyright:** `Copyright (c) 2001-2017 Ryan C. Gordon and others.`
  (`Ryan C. Gordon <icculus@icculus.org>`)
- **URL:** https://icculus.org/physfs/ — source https://github.com/icculus/physfs
- **Source cited:** https://raw.githubusercontent.com/icculus/physfs/release-3.0.1/LICENSE.txt

## libmodplug

- **Component:** `libmodplug.so.1` (bundled). Version unverified (no version
  string exposed by the shipped binary).
- **License:** **Public domain.** The upstream `COPYING` states verbatim:
  `ModPlug-XMMS and libmodplug are now in the public domain.`
- **Copyright:** none asserted (public domain). Original author: Olivier
  Lapicque; maintained fork by Konstanty Bialkowski.
- **URL:** https://github.com/Konstanty/libmodplug (maintained fork);
  historical: http://modplug-xmms.sourceforge.net/
- **Source cited:** https://raw.githubusercontent.com/Konstanty/libmodplug/master/COPYING
- **Flag:** task noted "public domain / MIT-ish — verify": **verified public
  domain** per upstream `COPYING`. Exact bundled version not pinned.

## SDL2 (Simple DirectMedia Layer)

- **Component:** `libSDL2-2.0.so.0` (device-provided; linked). Version
  unverified (device build).
- **License:** zlib license.
- **Copyright:** `Copyright (C) 1997-2022 Sam Lantinga <slouken@libsdl.org>`
- **URL:** https://www.libsdl.org/ — source https://github.com/libsdl-org/SDL
- **Source cited:** https://raw.githubusercontent.com/libsdl-org/SDL/release-2.0.20/LICENSE.txt
- **Flag:** copyright year range reflects the release-2.0.20 `LICENSE.txt`; the
  device's exact SDL2 version is not pinned.

## SDL2_image

- **Component:** `libSDL2_image-2.0.so.0` (device-provided; linked).
- **License:** zlib license.
- **Copyright:** `Copyright (C) 1997-2025 Sam Lantinga <slouken@libsdl.org>`
- **URL:** https://github.com/libsdl-org/SDL_image
- **Source cited:** https://raw.githubusercontent.com/libsdl-org/SDL_image/SDL2/LICENSE.txt
- **Flag:** year range from the current `SDL2` branch `LICENSE.txt`; device
  version not pinned. Pulls in libpng/zlib (below) and possibly libjpeg/libwebp/
  libtiff for other formats.

## SDL2_ttf

- **Component:** `libSDL2_ttf-2.0.so.0` (device-provided; linked).
- **License:** zlib license.
- **Copyright:** `Copyright (C) 1997-2025 Sam Lantinga <slouken@libsdl.org>`
  (also embeds FreeType, licensed under the FreeType License (FTL) / GPL-2.0 at
  the user's option — credit FreeType if the device's SDL2_ttf statically links
  it).
- **URL:** https://github.com/libsdl-org/SDL_ttf — FreeType: https://freetype.org/
- **Source cited:** https://raw.githubusercontent.com/libsdl-org/SDL_ttf/SDL2/LICENSE.txt
- **Flag:** device version and whether FreeType is bundled vs. system are not
  pinned.

## OpenAL Soft

- **Component:** `libopenal.so.1` (device-provided; linked).
- **License:** **LGPL.** The upstream `COPYING` is the *GNU Library General
  Public License, Version 2, June 1991* (`Copyright (C) 1991 Free Software
  Foundation, Inc.`); OpenAL Soft self-describes as "LGPL-licensed" and is
  commonly packaged as **LGPL-2.1+**.
- **Copyright:** project by Chris Robinson ("kcat") and contributors; forked
  from the original Loki/Creative OpenAL sample implementation.
- **URL:** https://openal-soft.org/ — source https://github.com/kcat/openal-soft
- **Source cited:** https://raw.githubusercontent.com/kcat/openal-soft/master/COPYING
  and https://raw.githubusercontent.com/kcat/openal-soft/master/README.md
- **Flag:** the bundled `COPYING` is literally the *Library* GPL v2 (LGPL-2.0);
  the task's "LGPL-2.1" label matches the project's usual packaging but the
  exact LGPL point version depends on the device build. LGPL requires
  corresponding source — see [`SOURCE.md`](./SOURCE.md).

## libvorbis / libvorbisfile

- **Component:** `libvorbis.so.0`, `libvorbisfile.so.3` (device-provided;
  linked).
- **License:** BSD 3-clause (Xiph.Org).
- **Copyright:** `Copyright (c) 2002-2020 Xiph.org Foundation`
- **URL:** https://xiph.org/vorbis/ — source https://github.com/xiph/vorbis
- **Source cited:** https://raw.githubusercontent.com/xiph/vorbis/master/COPYING

## libogg

- **Component:** `libogg.so.0` (pulled in by vorbis; device-provided).
- **License:** BSD 3-clause (Xiph.Org).
- **Copyright:** `Copyright (c) 2002, Xiph.org Foundation`
- **URL:** https://xiph.org/ogg/ — source https://github.com/xiph/ogg
- **Source cited:** https://raw.githubusercontent.com/xiph/ogg/master/COPYING

## libpng

- **Component:** transitive via SDL2_image (device-provided). Version
  unverified.
- **License:** PNG Reference Library License (version 2) — a permissive,
  BSD/MIT-style license.
- **Copyright (upstream `LICENSE`, libpng16):**
  - `Copyright (c) 1995-2026 The PNG Reference Library Authors.`
  - `Copyright (c) 2018-2026 Cosmin Truta.`
  - `Copyright (c) 2000-2002, 2004, 2006-2018 Glenn Randers-Pehrson.`
  - `Copyright (c) 1996-1997 Andreas Dilger.`
  - `Copyright (c) 1995-1996 Guy Eric Schalnat, Group 42, Inc.`
- **URL:** http://www.libpng.org/pub/png/libpng.html — source
  https://github.com/pnggroup/libpng
- **Source cited:** https://raw.githubusercontent.com/pnggroup/libpng/libpng16/LICENSE
- **Flag:** the upstream `libpng16` branch header currently shows year ranges
  extending to the current year; the device's shipped libpng version and its
  exact copyright years are not pinned.

## zlib

- **Component:** transitive via SDL2_image / libpng (device-provided). Version
  unverified.
- **License:** zlib license.
- **Copyright:** `Copyright (C) 1995-2017 Jean-loup Gailly and Mark Adler`
  (from the `zlib.h` of the v1.2.11 release cited below).
- **URL:** https://zlib.net/ — source https://github.com/madler/zlib
- **Source cited:** https://raw.githubusercontent.com/madler/zlib/v1.2.11/zlib.h
- **Flag:** v1.2.11 used as the reference for the copyright line; the device's
  exact zlib version is not pinned.

---

## License notice texts

The full verbatim license texts for the components credited above are available
from their upstream projects (URLs above) and on request; the authoritative text
is each project's own `LICENSE`/`COPYING`.

The one bundled library whose license strictly requires its notice to travel
with the binary is **LuaJIT** (MIT). Its short MIT notice is reproduced here for
completeness (verbatim from the LuaJIT `COPYRIGHT` file; the beta3-era copyright
line is `Copyright (C) 2005-2017 Mike Pall`):

```
Copyright (C) 2005-2026 Mike Pall. All rights reserved.

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in
all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT.  IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
THE SOFTWARE.

[ MIT license: https://www.opensource.org/licenses/mit-license.php ]
```

(LuaJIT also embeds Lua 5.1/5.2 code, `Copyright (C) 1994-2012 Lua.org,
PUC-Rio`, under the same MIT terms.)

---

### Notes / verification method

- Linkage (`NEEDED` entries) was read directly from the shipped aarch64
  `libsolarus.so.1` and `solarus-1.6.5` binaries.
- LuaJIT version (`LuaJIT 2.1.0-beta3`) and PhysFS version (`3.0.1`, from the
  build path `/build/libphysfs-.../libphysfs-3.0.1/...`) were read from strings
  embedded in the binaries.
- Copyright lines and license identities were taken verbatim from each
  project's upstream license file at the cited raw URL. (WebFetch's summarizer
  declines to reproduce full license bodies, so exact bytes were retrieved
  directly from the cited upstream files.)
- "Device-provided" libraries are credited as runtime dependencies even though
  the actual shared objects are supplied by the handheld's OS image, not by
  kUI. Where a version could not be confirmed from the shipped artifacts it is
  marked **unverified** rather than guessed.
