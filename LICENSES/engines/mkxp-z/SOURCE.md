# mkxp-z — GPL corresponding source

The mkxp-z runtime shipped by kUI is a **prebuilt aarch64 binary** of
**mkxp-z**, a fork of Ancurio/Amaryllis Kulla's *mkxp*. mkxp-z's own code is
licensed **GPL-2.0-or-later**; because the shipped binary is an HTTPS-enabled
build that links **OpenSSL (Apache-2.0)**, the resulting binary is distributed
under the **GNU General Public License version 3** (see below). The complete
corresponding source is offered here.

## Upstream

```
Ancurio/mkxp   (original, GPL-2.0-or-later,
      │         Copyright (C) 2013 - 2021 Amaryllis Kulla <ancurio@mapleshrine.eu>)
      ▼
mkxp-z/mkxp-z  ("MKXP but a bit supercharged" — modern MRI 3.x, HTTPS,
                 aarch64/ARM support; the canonical mkxp-z repo, maintained
                 by Roza and contributors)
```

- Original mkxp: https://github.com/Ancurio/mkxp — GPL-2.0-or-later
- mkxp-z (canonical, tagged releases): https://github.com/mkxp-z/mkxp-z
- mkxp-z license file (verbatim GPL v2, 339 lines):
  https://github.com/mkxp-z/mkxp-z/blob/master/COPYING

Individual `src/*.cpp` files carry the mkxp copyright header:

```
** This file is part of mkxp.
** Copyright (C) 2013 - 2021 Amaryllis Kulla <ancurio@mapleshrine.eu>
** mkxp is free software: you can redistribute it and/or modify
** it under the terms of the GNU General Public License as published by
** the Free Software Foundation, either version 2 of the License, or
** (at your option) any later version.
```

## License of the shipped binary (GPLv3)

The mkxp-z README states, verbatim:

> mkxp-z is licensed under the GNU General Public License v2+. However, if you
> build mkxp-z with the `enable-https` option turned on (which is the default),
> you will also need to comply with OpenSSL's Apache v2 license, which in
> practice means that the resulting binaries are licensed under GPLv3.

PortMaster mkxp-z ports ship the default (HTTPS-enabled) build, so the binary
kUI carries falls under GPLv3. The corresponding source offered here (mkxp-z +
its dependencies, incl. OpenSSL) satisfies both GPL-2.0-or-later and GPL-3.0.

## Best available version identifier for the shipped build

**The exact commit used to build the specific lifted aarch64 binary kUI ships
is NOT verified.** The binary was lifted from a PortMaster-style mkxp-z RPG
Maker port rather than built by kUI. This is flagged explicitly rather than
guessed.

What can be stated:

- It is a **mkxp-z** build (the "-z" line specifically — modern MRI 3.x, ARM /
  aarch64 support, HTTPS by default), not classic mkxp / falcon-mkxp.
- The current upstream release at assembly time is **mkxp-z v2.4.2** (the
  `meson.build` `project(... version: '2.4.2' ...)`); the shipped PortMaster
  binary is expected to correspond to **v2.4.x** but this has not been
  confirmed against this binary.
- PortMaster mkxp-z ports do **not** use a shared `mkxp-z.squashfs` runtime;
  each port bundles its own mkxp-z binary. There is no single PortMaster
  "runtime version" string to cite — the version lives in the binary itself.

### How to recover the exact version from the binary

mkxp-z compiles in `MKXPZ_VERSION` and `MKXPZ_GIT_HASH` (see `meson.build`,
which sets `-DMKXPZ_VERSION` and `-DMKXPZ_GIT_HASH` from
`git rev-parse --short HEAD`). The precise revision can therefore be read off
the shipped binary with:

```
strings <mkxp-z binary> | grep -iE 'mkxp|MKXPZ|[0-9]+\.[0-9]+\.[0-9]+'
```

The embedded short git hash pins the corresponding upstream commit exactly.

## Obtaining corresponding source

To obtain the complete corresponding source for the GPL engine in this binary:

```
git clone https://github.com/mkxp-z/mkxp-z
# then check out the tag/commit reported by the binary, e.g.:
#   git checkout v2.4.2      (or the MKXPZ_GIT_HASH read via strings)
```

with the original mkxp tree as its lineage parent:

```
git clone https://github.com/Ancurio/mkxp
```

If the exact revision matching the shipped binary cannot be determined from the
embedded `MKXPZ_GIT_HASH`, request it from the kUI maintainer, who will identify
the precise PortMaster mkxp-z build the binary was lifted from, or substitute a
from-source build of the pinned mkxp-z commit.

## Third-party bundled libraries

Libraries linked into the binary (Ruby 3.x, the SDL2 family, OpenAL Soft,
libvorbis/ogg/theora, FreeType, libpng, zlib, PhysFS, pixman, Boost, bzip2,
uchardet, OpenSSL, and vendored single-header libraries) are enumerated with
their own licenses, copyright holders, and source URLs in `THIRD-PARTY.md`
alongside this file.

## Additional copyleft corresponding-source obligations (squashfs `libs/`)

The shipped `Data/PortMaster/libs/mkxp-z.squashfs` bundles further prebuilt
`.so` runtime libraries (confirmed via `unsquashfs`) that are transitive
dependencies of Ruby and FluidSynth. Three of them are **copyleft** and their
complete corresponding source is offered here (the others — libxcrypt LGPL-2.1+
and ncurses/`libtinfo` MIT-style — are permissive/LGPL and fully described in
`THIRD-PARTY.md`):

- **GNU Readline** (`libreadline.so.8`) — **GPL-3.0-or-later** (a Ruby
  dependency). Corresponding source:

  ```
  git clone https://git.savannah.gnu.org/git/readline.git
  # or release tarballs: https://ftp.gnu.org/gnu/readline/
  ```

- **libInstPatch** (`libinstpatch-1.0.so.2`) — **LGPL-2.1-only** (a FluidSynth
  SoundFont dependency). Corresponding source:

  ```
  git clone https://github.com/swami/libinstpatch
  ```

- **JACK Audio Connection Kit — client library** (`libjack.so.0`) —
  **LGPL-2.1-or-later** (a FluidSynth audio backend; only the LGPL client
  library is bundled, not the GPL server). Corresponding source:

  ```
  git clone https://github.com/jackaudio/jack2
  ```

As with the mkxp-z binary itself, the exact upstream revisions baked into these
prebuilt `.so` files cannot be recovered from the binaries alone; the repos
above are the authoritative corresponding-source origins, and the specific
version can be read from each library's SONAME / embedded version strings.

## LGPL corresponding source for linked / optional libraries

Some of the libraries listed in `THIRD-PARTY.md` are LGPL. They reach the
binary as separate, dynamically-linked `.so` files (which satisfies the LGPL
relinking condition), but LGPL-2.1 still requires the corresponding source for
each LGPL library itself to be made available. It is — from upstream and on
request:

- **OpenAL Soft** — LGPL-2.1-or-later — https://github.com/kcat/openal-soft
  (linked audio backend; listed in `THIRD-PARTY.md`).
- **FluidSynth** — LGPL-2.1-or-later — https://github.com/FluidSynth/fluidsynth
  (optional MIDI backend; present only in `shared_fluid` builds, and its
  SoundFont/audio dependencies libInstPatch and libjack are already offered
  above).
- **libiconv** — LGPL-2.1-or-later — https://www.gnu.org/software/libiconv/
  (git://git.savannah.gnu.org/libiconv.git; linked only on non-glibc platforms,
  so it may be absent from this glibc aarch64 build).

## Sources

- https://github.com/mkxp-z/mkxp-z
- https://github.com/Ancurio/mkxp
- https://github.com/mkxp-z/mkxp-z/blob/master/COPYING
- https://github.com/mkxp-z/mkxp-z/blob/master/README.md
- https://github.com/mkxp-z/mkxp-z/blob/master/meson.build (version + git-hash defines)
- https://github.com/mkxp-z/mkxp-z/blob/master/src/main.cpp (copyright header)
- GNU Readline (GPLv3): https://git.savannah.gnu.org/cgit/readline.git and https://ftp.gnu.org/gnu/readline/
- libInstPatch (LGPL-2.1-only): https://github.com/swami/libinstpatch
- JACK2 (client lib LGPL-2.1-or-later): https://github.com/jackaudio/jack2
- OpenAL Soft (LGPL-2.1-or-later): https://github.com/kcat/openal-soft
- FluidSynth (LGPL-2.1-or-later, optional MIDI): https://github.com/FluidSynth/fluidsynth
- libiconv (LGPL-2.1-or-later, non-glibc only): https://www.gnu.org/software/libiconv/
