# falcon-mkxp — third-party components

The falcon-mkxp runtime kUI ships is a prebuilt aarch64 binary of
**Falcon-mkxp** (a fork of Ancurio's *mkxp*), a GPL-2.0-or-later engine that
runs older-Ruby RPG Maker XP / VX / VX Ace games. mkxp binds against **MRI
(CRuby) 2.x** — an *older* Ruby than the modern mkxp-z line (which uses Ruby
3.x). The original RGSS runtime in RPG Maker XP used MRI **1.8**, and mkxp's
own README notes it is "written against 2.0"; the classic Falcon-mkxp binding
targets this MRI 2.x lineage rather than 3.x. Games' own bundled Ruby scripts
may assume 1.8-era semantics.

The engine links (statically and/or dynamically) the libraries below. This
list is derived from the upstream Falcon-mkxp `README.md` "Dependencies /
Building" section and standard mkxp build configuration. Copyright lines are
the canonical upstream holders for each project; the **exact bundled versions,
copyright years, and sub-license choices for this particular lifted aarch64
binary cannot be verified from the binary alone** and are flagged where
relevant. Each entry cites the authoritative upstream project.

Every component below is under a license compatible with GPL-2.0-or-later
redistribution.

| Component | License | Copyright holder(s) | Upstream URL |
|---|---|---|---|
| Ruby (MRI/CRuby, 2.x) | Ruby License OR BSD-2-Clause (dual) | Copyright (C) Yukihiro Matsumoto and Ruby contributors | https://www.ruby-lang.org/ |
| SDL2 | zlib | Copyright (C) 1997–2020 Sam Lantinga <slouken@libsdl.org> | https://www.libsdl.org/ |
| SDL2_image | zlib | Copyright (C) 1997–2020 Sam Lantinga | https://github.com/libsdl-org/SDL_image |
| SDL2_ttf | zlib | Copyright (C) 2001–2020 Sam Lantinga | https://github.com/libsdl-org/SDL_ttf |
| SDL_sound 1.x (Ancurio's fork) | LGPL-2.1-or-later (verified — see note below) | Copyright (C) 2001 Ryan C. Gordon and contributors; fork by Ancurio | https://github.com/Ancurio/SDL_sound |
| OpenAL Soft | LGPL-2.1-or-later | Copyright (C) Chris Robinson and the OpenAL Soft contributors | https://openal-soft.org/ |
| libvorbis / vorbisfile | BSD-3-Clause | Copyright (C) 2002–2020 Xiph.Org Foundation | https://xiph.org/vorbis/ |
| libogg | BSD-3-Clause | Copyright (C) 2002–2019 Xiph.Org Foundation | https://xiph.org/ogg/ |
| FreeType (via SDL2_ttf) | FreeType License (BSD-style w/ credit) OR GPL-2.0 (dual) | Copyright (C) 1996–present David Turner, Robert Wilhelm, Werner Lemberg | https://freetype.org/ |
| libpng (via SDL2_image) | PNG Reference Library License (libpng) | Copyright (C) 1995–present the PNG Reference Library Authors | http://www.libpng.org/pub/png/libpng.html |
| zlib | zlib License | Copyright (C) 1995–present Jean-loup Gailly and Mark Adler | https://zlib.net/ |
| PhysicsFS (PhysFS) | zlib License | Copyright (C) 2001–present Ryan C. Gordon and contributors | https://icculus.org/physfs/ |
| pixman | MIT License | Copyright (C) The pixman contributors (Red Hat, SUSE, Keith Packard, et al.) | https://www.pixman.org/ |
| Boost (Unordered, Program_options headers) | Boost Software License 1.0 | Copyright (C) the respective Boost authors | https://www.boost.org/ |
| libsigc++ 2.0 | LGPL-2.1-or-later | Copyright (C) the libsigc++ development team | https://libsigcplusplus.github.io/libsigcplusplus/ |

## Bundled shared libraries — verified in the shipped squashfs (`libs/`)

Unlike the table above (derived from the upstream README), the entries here
were confirmed by **`unsquashfs`-listing the actual shipped
`Data/PortMaster/libs/falcon-mkxp.squashfs`**. Every file below is physically
present under `squashfs-root/libs/`. The audio codecs are decoders reached
through **SDL_sound** (see the SDL_sound row above); `libtheoradec` is a video
decoder reached transitively through the SDL_sound / Xiph chain. All are under
licenses compatible with GPL-2.0-or-later redistribution.

| Component (SONAME in `libs/`) | License | Copyright holder(s) | Upstream URL |
|---|---|---|---|
| **libmikmod** (`libmikmod.so.3`) | LGPL-2.0-or-later — source headers read "GNU **Library** General Public License … version 2 … or (at your option) any later version"; the "or later" grant covers LGPL-2.1. COPYLEFT. | Copyright (C) Jean-Paul Mikkers, Jake Stine, Raphaël Assenat, Miodrag Vallat, Ozkan Sezer, and the libmikmod contributors | https://mikmod.sourceforge.net/ (source mirror: https://github.com/sezero/mikmod) |
| **libFLAC** (`libFLAC.so.8`) | BSD-3-Clause (`COPYING.Xiph`) | Copyright (C) 2000–2009 Josh Coalson; Copyright (C) 2011–present Xiph.Org Foundation | https://xiph.org/flac/ |
| **libtheora** — decoder (`libtheoradec.so.1`) | BSD-3-Clause (`COPYING`) plus the On2 VP3 patent-non-assertion statement (`LICENSE`) | Copyright (C) 2002–2009 Xiph.Org Foundation | https://theora.org/ (source: https://github.com/xiph/theora) |
| **libModPlug** (`libmodplug.so.1`) | Public domain (`COPYING`: "ModPlug-XMMS and libmodplug are now in the public domain"; source headers: "This source code is public domain") | Olivier Lapicque, Adam Goode, and contributors (dedicated to the public domain) | https://github.com/Konstanty/libmodplug |
| **libxcrypt** (`libcrypt.so.1`) | LGPL-2.1-or-later (`LICENSING`) — identified as **libxcrypt** (not glibc's `libcrypt`) via the `XCRYPT_2.0` / `XCRYPT_4.3` / `XCRYPT_4.4` versioned symbols and `xcrypt_gensalt*` exports in the shipped `.so`. Reached transitively via Ruby. | Copyright (C) Thorsten Kukuk, Björn Esser, Zack Weinberg, and contributors | https://github.com/besser82/libxcrypt |

For completeness, `unsquashfs -l` also confirmed these libs already covered by
the main table: `libruby-2.7.so.2.7` (the shipped MRI is specifically **2.7**),
`libSDL_sound-1.0.so.1`, `libogg.so.0`, `libphysfs.so.1`, `libsigc-2.0.so.0`,
and `libboost_program_options.so.1.71.0` (Boost **1.71.0**).

## Optional / conditional components

These appear in the upstream build options and may or may not be present in
the shipped aarch64 binary — **presence unverified for this build**:

| Component | License | Copyright holder(s) | Upstream URL |
|---|---|---|---|
| FluidSynth (MIDI, loaded at runtime if present) | LGPL-2.1-or-later | Copyright (C) the FluidSynth developers | https://www.fluidsynth.org/ |
| libiconv (INI_ENCODING, optional) | LGPL-2.1-or-later | Copyright (C) Free Software Foundation, Inc. | https://www.gnu.org/software/libiconv/ |
| libguess (INI_ENCODING, optional) | BSD-2-Clause | Copyright (C) William Pitcock and contributors | https://github.com/kaniini/libguess |
| mruby (alternative binding — NOT used for RGSS games) | MIT License | Copyright (C) mruby developers | https://github.com/mruby/mruby |

## License texts

The full verbatim license texts for the third-party components credited above
are available from their upstream projects (URLs above) and on request; the
authoritative text for each is that project's own LICENSE/COPYING file.

## Notes and caveats

- **SDL_sound license — RESOLVED (LGPL-2.1-or-later).** Falcon-mkxp's README
  links Ancurio's SDL_sound fork specifically (the 1.x line;
  `libSDL_sound-1.0.1` per the build). The fork's own license was fetched
  (curled 2026-08-15) and confirmed **LGPL-2.1-or-later**: `LICENSE.txt`
  reproduces the GNU Lesser General Public License version 2.1, and the
  `SDL_sound.h` header reads *"under the terms of the GNU Lesser General Public
  License … either version 2.1 of the License, or (at your option) any later
  version."* (The unrelated modern icculus SDL_sound 2.x line is zlib; that is
  not what this engine bundles.) Corresponding source is recorded in
  `SOURCE.md`.
- **Ruby license:** Ruby is dual-licensed (the Ruby License or the
  2-clause BSD License); redistribution under either satisfies GPL-2.0
  compatibility. The precise 2.x point release bundled in this binary is not
  recorded upstream and is not recoverable from the binary.
- **FreeType / libpng** reach the binary transitively through SDL2_ttf and
  SDL2_image respectively; they may be statically absorbed into those
  libraries in a PortMaster-style build.
- **Theora / video — correction:** any prior claim that this runtime ships no
  video codec ("classic mkxp has no video", libtheora absent) is **incorrect**
  for the shipped build. `unsquashfs` confirms `libs/libtheoradec.so.1` is
  physically bundled; libtheora is credited above accordingly.
- Copyright **years** above reflect the upstream projects' ranges as of this
  writing (knowledge cutoff Jan 2026); the exact years embedded in the bundled
  revisions were not individually re-fetched and should be treated as
  approximate. Component **licenses** are stable and accurate.

## Sources

- Falcon-mkxp README (dependency list, Ruby-version note):
  https://github.com/pk-2000/Falcon-mkxp/blob/master/README.md
- Falcon-mkxp COPYING (GPL-2.0):
  https://github.com/pk-2000/Falcon-mkxp/blob/master/COPYING
- mkxp source header (Copyright 2013 Jonas Kulla):
  https://github.com/pk-2000/Falcon-mkxp/blob/master/src/main.cpp
- Per-library upstream project sites, linked in the tables above.
- Bundled-library licenses verified upstream (curled 2026-08-15):
  - libmikmod (GNU Library GPL v2-or-later, per source headers):
    https://raw.githubusercontent.com/sezero/mikmod/master/libmikmod/mmio/mmio.c
    and https://raw.githubusercontent.com/sezero/mikmod/master/libmikmod/COPYING.LIB
  - libFLAC (BSD-3-Clause): https://raw.githubusercontent.com/xiph/flac/master/COPYING.Xiph
  - libtheora (BSD-3-Clause + On2 VP3 patent note):
    https://raw.githubusercontent.com/xiph/theora/master/COPYING
    and https://raw.githubusercontent.com/xiph/theora/master/LICENSE
  - libModPlug (public domain):
    https://raw.githubusercontent.com/Konstanty/libmodplug/master/COPYING
  - libxcrypt (LGPL-2.1-or-later):
    https://raw.githubusercontent.com/besser82/libxcrypt/master/LICENSING
  - SDL_sound 1.x (Ancurio's fork; LGPL-2.1-or-later):
    https://raw.githubusercontent.com/Ancurio/SDL_sound/master/LICENSE.txt
    and header https://raw.githubusercontent.com/Ancurio/SDL_sound/master/SDL_sound.h
