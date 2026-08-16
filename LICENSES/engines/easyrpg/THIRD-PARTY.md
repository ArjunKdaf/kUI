# EasyRPG engine runtime — third-party components

The EasyRPG runtime kUI ships (Port Forge, RPG Maker 2000/2003) is **built from
source** for aarch64 against the kUI toolchain (glibc 2.28). The program itself
is **EasyRPG Player 0.8.1.1** (GPL-3.0-or-later); its verbatim license and that
of **liblcf** are in `LICENSE`. This file enumerates the other third-party
libraries that actually reach the shipped runtime.

Two categories are distinguished, because they carry different obligations:

1. **Statically linked** into the `easyrpg-player` binary. These become part of
   the combined GPLv3 work; the corresponding source is covered by `SOURCE.md`.
2. **Bundled shared libraries** shipped as `.so` files in `easyrpg/libs/`,
   loaded dynamically at runtime.

The set below was verified against the build tree at `~/dev/easyrpg-runtime`
(`pkg/easyrpg/libs/`, `staging/lib/*.a`, `src/`) and against the `NEEDED`
entries of the actual aarch64 binary (`readelf -d`). Copyright lines are the
canonical upstream holders; versions are taken from the source trees / the
version strings embedded in the shipped `.so` files.

## 1. Statically linked into `easyrpg-player`

| Component | Version | License (SPDX) | Copyright holder(s) | Upstream |
|---|---|---|---|---|
| liblcf | 0.8.1 | MIT | Copyright (c) 2014-2025 liblcf authors | https://github.com/EasyRPG/liblcf |
| fmt (fmtlib) | 9.1.0 | MIT | Copyright (c) 2012 - present, Victor Zverovich | https://github.com/fmtlib/fmt |
| inih | git snapshot | BSD-3-Clause | Copyright (c) 2009, Ben Hoyt | https://github.com/benhoyt/inih |
| mpg123 (libmpg123 / libout123 / libsyn123) | 1.33.7 | LGPL-2.1-or-later | Copyright (c) 1995-2020 by Michael Hipp and others | https://www.mpg123.de/ |

Notes:

- **liblcf** and **fmt** are permissive MIT; their full verbatim texts are in
  the source trees (`src/liblcf/COPYING`, `src/fmt/LICENSE.rst`). fmt's license
  additionally grants the standard "optional exception" permitting redistribution
  of embedded machine-code portions without the notices.
- **inih** carries the New (3-clause) BSD license, copyright Ben Hoyt
  (`src/inih/LICENSE.txt`).
- **mpg123** is **LGPL-2.1-or-later** and is statically linked here. The
  corresponding source (`mpg123-1.33.7`) is included in the build tree
  (`~/dev/easyrpg-runtime/src/mpg123-src`, from the upstream
  `mpg123-1.33.7.tar.bz2`); see `SOURCE.md`. The mpg123 `COPYING` states the
  "About box" credit line quoted above; per-author detail is in its `AUTHORS`
  file.

## 2. Bundled shared libraries (`easyrpg/libs/*.so`)

| Library (soname) | Version | License (SPDX) | Copyright holder(s) | Upstream |
|---|---|---|---|---|
| libpng (libpng12.so.0) | 1.2.56 | Libpng (PNG Reference Library License v1) | Copyright (c) 2000-2002, 2004, 2006-2015 Glenn Randers-Pehrson; (c) 1996-1997 Andreas Dilger; (c) 1995-1996 Guy Eric Schalnat, Group 42, Inc. | http://www.libpng.org/pub/png/libpng.html |
| libsndfile (libsndfile.so.1) | 1.0.28 | LGPL-2.1-or-later | Copyright (c) 1999-2016 Erik de Castro Lopo &lt;erikd@mega-nerd.com&gt; | https://libsndfile.github.io/libsndfile/ |
| libsamplerate (libsamplerate.so.0) | 0.2.2 | BSD-2-Clause | Copyright (c) 2012-2016, Erik de Castro Lopo &lt;erikd@mega-nerd.com&gt; | https://libsndfile.github.io/libsamplerate/ |
| libvorbis (libvorbis.so.0) | 1.3.5 | BSD-3-Clause | Copyright (c) 2002-2008 Xiph.Org Foundation | https://xiph.org/vorbis/ |
| libvorbisfile (libvorbisfile.so.3) | Xiph | BSD-3-Clause | Copyright (c) 2002-2008 Xiph.Org Foundation | https://xiph.org/vorbis/ |
| libogg (libogg.so.0) | Xiph | BSD-3-Clause | Copyright (c) 2002, Xiph.Org Foundation | https://xiph.org/ogg/ |

Notes:

- These are **unmodified** shared libraries shipped alongside the binary so the
  runtime resolves the same audio/image stack on-device. **libpng 1.2.56**,
  **libsndfile 1.0.28** and **libsamplerate 0.2.2** versions are taken from the
  version strings embedded in the shipped `.so` files. **libvorbis.so** does
  embed its version (**1.3.5 / 20150105**); only **libvorbisfile.so.3** and
  **libogg.so.0** lack an embedded version string, so their exact Xiph point
  releases are **not individually verified from the binaries** — the license
  and copyright holder (Xiph.Org) are stable and accurate regardless.
- **libsndfile** is LGPL-2.1; it is shipped as a dynamically-linked `.so`, so
  the LGPL relinking obligation is satisfied by the separate shared library.
- The `libsndfile` and `mpg123` `COPYING` files reproduce the FSF's LGPL v2.1
  text (which itself is "Copyright (C) 1991, 1999 Free Software Foundation,
  Inc."); the *software* copyright holders are the ones listed above.

## 3. Provided by the device (NOT bundled by kUI)

These are `NEEDED` dynamic dependencies of the binary that are satisfied by the
device system and are **not** shipped in `easyrpg/libs/`. Listed for
completeness / attribution; kUI redistributes none of them here.

| Library | Typical license | Copyright holder(s) | Upstream |
|---|---|---|---|
| SDL2 (libSDL2-2.0.so.0) | zlib | Copyright (C) 1997-2024 Sam Lantinga | https://www.libsdl.org/ |
| zlib (libz.so.1) | Zlib | Copyright (C) 1995-2024 Jean-loup Gailly and Mark Adler | https://zlib.net/ |
| pixman (libpixman-1.so.0) | MIT | Copyright the pixman contributors (Red Hat, Keith Packard, et al.) | https://www.pixman.org/ |
| ALSA (libasound.so.2) | LGPL-2.1-or-later | Copyright the ALSA project authors | https://www.alsa-project.org/ |
| libstdc++ / libgcc_s | GPL-3.0 with GCC Runtime Library Exception | Copyright Free Software Foundation, Inc. | https://gcc.gnu.org/ |
| glibc (libc / libm / libdl / libpthread) | LGPL-2.1-or-later | Copyright Free Software Foundation, Inc. | https://www.gnu.org/software/libc/ |

## License texts

The full verbatim license texts for the third-party components credited above
are available from their upstream projects (URLs above) and on request; the
authoritative text for each is that project's own LICENSE/COPYING file.

## Scope notes / corrections to the initial component guess

The build tree was treated as authoritative. Relative to an early guessed
dependency list:

- **libsamplerate 0.2.2 IS bundled** (in `easyrpg/libs/`) and is included
  above — it was absent from the initial guess.
- **zlib and pixman are NOT bundled** — they are device-provided dynamic
  dependencies (`NEEDED` in the binary, no `.so` in `libs/`), so they are listed
  under section 3, not as bundled components.
- **speexdsp, opusfile and wildmidi are NOT present** anywhere in the build
  tree and are **not** linked into or shipped with this runtime; they are
  omitted.

## Sources

- Build tree: `~/dev/easyrpg-runtime` (`pkg/easyrpg/libs/`, `staging/lib/*.a`,
  `src/`, `pkg/easyrpg/LICENSES/SOURCES.txt`) and `readelf -d` of the shipped
  `easyrpg-player` aarch64 binary.
- liblcf license: https://github.com/EasyRPG/liblcf/blob/0.8.1/COPYING
- fmt license: https://github.com/fmtlib/fmt/blob/9.1.0/LICENSE.rst
- inih license: https://github.com/benhoyt/inih/blob/master/LICENSE.txt
- mpg123 license: https://www.mpg123.de/ (COPYING, LGPL v2.1)
- libpng license: http://www.libpng.org/pub/png/src/libpng-LICENSE.txt
- libsndfile: https://github.com/libsndfile/libsndfile/blob/master/COPYING
- libsamplerate: https://github.com/libsndfile/libsamplerate/blob/master/COPYING
- libogg: https://github.com/xiph/ogg/blob/master/COPYING
- libvorbis: https://github.com/xiph/vorbis/blob/master/COPYING
