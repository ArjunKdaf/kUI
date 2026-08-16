# mkxp-z — third-party components

The mkxp-z runtime kUI ships is a prebuilt aarch64 binary of **mkxp-z** (a fork
of Ancurio/Amaryllis Kulla's *mkxp*), a GPL-2.0-or-later engine that runs RPG
Maker XP / VX / VX Ace games. Unlike the classic `falcon-mkxp` engine (MRI 2.x),
mkxp-z binds against **modern MRI (CRuby) 3.x**.

The dependency list below is derived from the **authoritative mkxp-z build
files** — `src/meson.build` (the linked `global_dependencies` array) and
`binding/meson.build` (the Ruby binding) — plus the third-party sources vendored
in the mkxp-z source tree. Copyright lines were fetched from each project's own
upstream `COPYING`/`LICENSE` where possible (see per-row citation); those are
marked accurate. The **exact bundled versions and copyright years baked into
this particular lifted aarch64 binary cannot be recovered from the binary
alone**, so version-specific years should be treated as the upstream project's
current range, not a guarantee of the shipped revision. Anything genuinely
uncertain is flagged inline.

## Licensing outcome for the shipped binary

mkxp-z's own code is **GPL-2.0-or-later**. The PortMaster mkxp-z build enables
HTTPS by default, which links **OpenSSL (Apache-2.0)**; per the upstream README
this makes the *resulting binary* effectively **GPL-3.0**. Every component below
is under a license compatible with that GPL redistribution.

## Linked libraries (from `src/meson.build` `global_dependencies` + Ruby binding)

| Component | License | Copyright holder(s) | Upstream URL |
|---|---|---|---|
| Ruby (MRI / CRuby, 3.x) | Ruby License OR BSD-2-Clause (dual) | Ruby is copyrighted free software by Yukihiro Matsumoto \<matz@netlab.jp\> and contributors | https://www.ruby-lang.org/ |
| SDL2 | zlib | Copyright (C) 1997-2026 Sam Lantinga \<slouken@libsdl.org\> | https://www.libsdl.org/ |
| SDL2_image | zlib | Copyright (C) 1997-2025 Sam Lantinga \<slouken@libsdl.org\> | https://github.com/libsdl-org/SDL_image |
| SDL2_ttf | zlib | Copyright (C) 1997-2025 Sam Lantinga \<slouken@libsdl.org\> | https://github.com/libsdl-org/SDL_ttf |
| SDL2_sound (icculus SDL_sound 2.x) | zlib | Copyright (c) 2001-2026 Ryan C. Gordon \<icculus@icculus.org\> and others | https://github.com/icculus/SDL_sound |
| OpenAL Soft | LGPL-2.1-or-later | Copyright (C) Chris Robinson and the OpenAL Soft contributors | https://openal-soft.org/ |
| libvorbis / vorbisfile | BSD-3-Clause | Copyright (c) 2002-2020 Xiph.Org Foundation | https://xiph.org/vorbis/ |
| libogg | BSD-3-Clause | Copyright (c) 2002, Xiph.Org Foundation | https://xiph.org/ogg/ |
| libtheora | BSD-3-Clause | Copyright (C) 2002-2009 Xiph.Org Foundation | https://www.theora.org/ |
| FreeType (freetype2) | FreeType License (FTL, BSD-style w/ credit) OR GPL-2.0 (dual) | The FreeType Project is copyright (C) 1996-2000 by David Turner, Robert Wilhelm, and Werner Lemberg | https://freetype.org/ |
| libpng | PNG Reference Library License version 2 | Copyright (c) 1995-2026 The PNG Reference Library Authors (incl. Cosmin Truta; Glenn Randers-Pehrson; Andreas Dilger; Guy Eric Schalnat, Group 42, Inc.) | http://www.libpng.org/pub/png/libpng.html |
| zlib | zlib License | Copyright (C) 1995-2026 Jean-loup Gailly and Mark Adler | https://zlib.net/ |
| PhysicsFS (PhysFS, `>=2.1`) | zlib License | Copyright (c) 2001-2026 Ryan C. Gordon \<icculus@icculus.org\> and others | https://icculus.org/physfs/ |
| pixman (`pixman-1`) | MIT License | Copyright holders incl. The Open Group; Digital Equipment Corporation; Keith Packard; SuSE, Inc.; and other pixman contributors | https://www.pixman.org/ |
| Boost (Unordered + hash headers, via `src/util/boost-hash.h`) | Boost Software License 1.0 | Copyright (C) the respective Boost authors | https://www.boost.org/ |
| bzip2 (libbz2) | bzip2 license (BSD-style) | Copyright (C) 1996-2019 Julian R Seward | https://sourceware.org/bzip2/ |
| uchardet | MPL-1.1 (tri-licensed MPL-1.1 / GPL-2.0+ / LGPL-2.1+) | Copyright (C) Mozilla Foundation and the uchardet contributors (BYVoid) | https://www.freedesktop.org/wiki/Software/uchardet/ |
| libiconv (non-glibc / `iconv` + `charset`, platform-conditional) | LGPL-2.1-or-later | Copyright (C) Free Software Foundation, Inc. | https://www.gnu.org/software/libiconv/ |
| OpenGL / OpenGL ES (`dependency('gl')`; system driver, not bundled) | vendor/system driver | N/A (provided by the device GPU driver) | https://www.khronos.org/opengl/ |

### Bundled inside SDL2_image (JPEG XL decode path)

The mkxp-z build pulls SDL2_image with its JPEG-XL modules, which statically
absorb these libraries:

| Component | License | Copyright holder(s) | Upstream URL |
|---|---|---|---|
| libjxl (`jxl_dec`) | BSD-3-Clause | Copyright (c) the JPEG XL Project Authors | https://github.com/libjxl/libjxl |
| Brotli (`brotlidec`, `brotlicommon`) | MIT License | Copyright (c) the Brotli Authors (Google) | https://github.com/google/brotli |
| Highway (`hwy`) | Apache-2.0 (also BSD-3-Clause) | Copyright (c) Google LLC and the Highway contributors | https://github.com/google/highway |

### Bundled in the squashfs `libs/` runtime dir (transitive runtime deps)

The shipped `Data/PortMaster/libs/mkxp-z.squashfs` carries a `libs/` directory
of prebuilt `.so` files loaded at runtime alongside the mkxp-z binary. Besides
the libraries already listed above, the following were confirmed present by
`unsquashfs` of the shipped squashfs but were not previously credited. They are
transitive dependencies pulled in by **Ruby** (`libcrypt`, `libreadline`, and
`libtinfo` via readline) and by **FluidSynth** (`libinstpatch`, `libjack`).
Several are **copyleft**; their corresponding-source obligations are recorded in
`SOURCE.md`. Each row's license was fetched from the project's own upstream
license/source at assembly time (see the added citations in **Sources**).

| Component | License | Copyright holder(s) | Upstream URL |
|---|---|---|---|
| libxcrypt (`libcrypt.so.1`) | LGPL-2.1-or-later | Copyright Thorsten Kukuk, Björn Esser, Zack Weinberg, and Free Software Foundation, Inc. | https://github.com/besser82/libxcrypt |
| libInstPatch (`libinstpatch-1.0.so.2`) — **COPYLEFT** | LGPL-2.1-**only** | Copyright (C) 1999-2014 Element Green \<element@elementsofsound.org\> | https://github.com/swami/libinstpatch |
| JACK Audio Connection Kit — client library (`libjack.so.0`) — **COPYLEFT** | LGPL-2.1-or-later | Copyright (C) 2001 Paul Davis; Copyright (C) 2004 Jack O'Quin | https://github.com/jackaudio/jack2 |
| GNU Readline (`libreadline.so.8`) — **COPYLEFT (GPL)** | GPL-3.0-or-later | Copyright (C) 1987-2025 Free Software Foundation, Inc. | https://git.savannah.gnu.org/cgit/readline.git |
| ncurses terminfo (`libtinfo.so.6`) | ncurses license (MIT/X11-style permissive) | Copyright 2018-2023 Thomas E. Dickey; Copyright 1998-2018 Free Software Foundation, Inc. | https://invisible-island.net/ncurses/ |

Notes on these bundled runtime libraries:

- **`libcrypt.so.1` is credited as libxcrypt (LGPL-2.1-or-later).** A bundled
  `libcrypt.so.1` shipped in a portable `libs/` dir is almost certainly
  **libxcrypt** (the independent implementation that modern distros use for the
  `libcrypt.so.1` SONAME), whose overall license is **LGPL-2.1-or-later** per
  its `LICENSING` file. If it were instead glibc's historical `libcrypt`, that
  code is also **LGPL-2.1-or-later**, so the license outcome is the same either
  way. It reaches the binary as a transitive dependency of Ruby.
- **libInstPatch is LGPL-2.1-only** (not "or later"): its `COPYING` states
  *"This software is restricted to version 2.1 of the LGPL only"* and each source
  header says *"version 2.1 of the License only."* It is a FluidSynth SoundFont
  dependency.
- **libjack is the JACK client library (LGPL-2.1-or-later)**, not the JACK
  server. The jack2 client headers (`common/jack/jack.h`) are LGPL-2.1-or-later;
  only the JACK server/daemon is GPL-2.0. Only the LGPL client library
  (`libjack.so.0`) is bundled here (a FluidSynth audio backend).
- **GNU Readline is GPL-3.0-or-later** (its `COPYING` is GPLv3 and every source
  header reads *"either version 3 of the License, or (at your option) any later
  version"*). It is a Ruby dependency. This is GPL copyleft and consistent with
  the "binary is effectively GPL-3.0" outcome already stated above.
- **`libtinfo.so.6` is the terminfo library from ncurses**, under the permissive
  MIT/X11-style ncurses license (verbatim `COPYING` fetched from the ncurses
  mirror). It is a dependency of Readline.

## Vendored source files (in-tree, compiled directly into mkxp-z)

| Component | License | Copyright holder(s) | Upstream URL |
|---|---|---|---|
| libnsgif (`src/display/libnsgif/`) | MIT License | Copyright 2004 Richard Wilson \<richard.wilson@netsurf-browser.org\>; Copyright 2008 Sean Fox \<dyntryx@gmail.com\> | https://www.netsurf-browser.org/projects/libnsgif/ |
| TheoraPlay (`src/theoraplay/`) | zlib License | Written by Ryan C. Gordon | https://github.com/icculus/theoraplay |
| json5pp (`src/util/json5pp.hpp`, config parser) | MIT License | Copyright (c) 2019 Shuta Kimura | https://github.com/kimushu/json5pp |
| rapidcsv (`src/util/rapidcsv.h`) | BSD-3-Clause | Copyright (C) 2017-2024 Kristofer Berggren | https://github.com/d99kris/rapidcsv |
| sigslot (`src/util/sigslot/signal.hpp`) | MIT License | Copyright (c) Pierre-Antoine Lacaze | https://github.com/palacaze/sigslot |
| MiniFFI (`src/*`, optional `use_miniffi`) | GPL-2.0-or-later (part of mkxp-z) | mkxp-z contributors | https://github.com/mkxp-z/mkxp-z |

## Optional / conditional components

Enabled by build options; presence in this specific aarch64 binary is noted
where known.

| Component | License | Copyright holder(s) | Upstream URL |
|---|---|---|---|
| OpenSSL (HTTPS; `enable-https` default = ON; provides **both** `libssl.so.1.1` and `libcrypto.so.1.1`, both bundled in the squashfs `libs/`) | Apache-2.0 | Copyright (c) The OpenSSL Project Authors | https://www.openssl.org/ |
| FluidSynth (MIDI; only if `shared_fluid` build option) | LGPL-2.1-or-later | Copyright (C) the FluidSynth developers | https://www.fluidsynth.org/ |
| Steamworks SDK (only if `steamworks_path` set — **proprietary; NOT expected in a PortMaster build**) | Proprietary (Valve SDK) | Copyright (C) Valve Corporation | https://partner.steamgames.com/ |

## License texts

The full verbatim license texts for the third-party components credited above
are available from their upstream projects (URLs above) and on request; the
authoritative text for each is that project's own LICENSE/COPYING file.

## Notes and caveats

- **Binary is effectively GPL-3.0.** With OpenSSL linked (the default HTTPS
  build), mkxp-z's own README states the resulting binaries are licensed under
  GPLv3. The corresponding-source offer in `SOURCE.md` covers this.
- **SDL_sound is the icculus 2.x line (zlib).** The build requests
  `SDL2_sound`, i.e. the modern icculus/SDL_sound 2.x fork whose `LICENSE.txt`
  is zlib — distinct from the older LGPL SDL_sound 1.x used by classic mkxp.
- **Ruby is dual-licensed** (the Ruby License or the 2-clause BSD License);
  redistribution under either satisfies GPL compatibility. The exact 3.x point
  release bundled here is not recoverable from the binary.
- **FreeType / libpng** may reach the binary transitively through SDL2_ttf /
  SDL2_image and can be statically absorbed into those libraries in a
  PortMaster-style static build.
- **Copyright years** reflect each upstream project's current range as fetched
  at assembly time; the precise years embedded in the bundled revisions were not
  individually re-derived from the binary. Component **licenses** are stable and
  accurate.
- **libiconv / charset** are only linked on non-glibc platforms (the meson file
  gates them behind the platform branch); on a typical glibc aarch64 PortMaster
  target iconv comes from glibc and libiconv may be absent.

## Sources

- mkxp-z linked-dependency list (`global_dependencies`):
  https://github.com/mkxp-z/mkxp-z/blob/master/src/meson.build
- mkxp-z Ruby binding: https://github.com/mkxp-z/mkxp-z/blob/master/binding/meson.build
- mkxp-z README (license note, OpenSSL/GPLv3): https://github.com/mkxp-z/mkxp-z/blob/master/README.md
- SDL2 / SDL2_image / SDL2_ttf LICENSE.txt: https://github.com/libsdl-org
- icculus SDL_sound LICENSE.txt: https://github.com/icculus/SDL_sound/blob/main/LICENSE.txt
- zlib copyright: https://github.com/madler/zlib/blob/master/zlib.h
- libpng LICENSE: https://github.com/pnggroup/libpng/blob/master/LICENSE
- Xiph ogg/vorbis/theora COPYING: https://github.com/xiph
- PhysFS LICENSE.txt: https://github.com/icculus/physfs/blob/main/LICENSE.txt
- pixman COPYING: https://gitlab.freedesktop.org/pixman/pixman/-/blob/master/COPYING
- Boost Software License 1.0: https://www.boost.org/LICENSE_1_0.txt
- bzip2 LICENSE: https://sourceware.org/git/?p=bzip2.git;a=blob_plain;f=LICENSE
- OpenAL Soft COPYING/README: https://github.com/kcat/openal-soft
- Ruby COPYING: https://github.com/ruby/ruby/blob/master/COPYING
- json5pp LICENSE: https://github.com/kimushu/json5pp/blob/master/LICENSE
- rapidcsv header (BSD-3): https://github.com/d99kris/rapidcsv
- libnsgif source header (MIT): https://github.com/mkxp-z/mkxp-z/blob/master/src/display/libnsgif/libnsgif.c
- libxcrypt LICENSING (LGPL-2.1-or-later): https://github.com/besser82/libxcrypt/blob/master/LICENSING
- libInstPatch COPYING (LGPL-2.1-only) + source header: https://github.com/swami/libinstpatch/blob/master/COPYING and .../libinstpatch/IpatchBase.c
- JACK client-library header (LGPL-2.1-or-later): https://github.com/jackaudio/jack2/blob/develop/common/jack/jack.h
- GNU Readline COPYING (GPLv3) + source header: https://git.savannah.gnu.org/cgit/readline.git/plain/COPYING and .../plain/readline.c
- ncurses COPYING (MIT/X11-style permissive): https://github.com/mirror/ncurses/blob/master/COPYING (upstream https://invisible-island.net/ncurses/)
- Per-library upstream project sites, linked in the tables above.
