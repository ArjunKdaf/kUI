# falcon-mkxp — GPL corresponding source

The falcon-mkxp runtime shipped by kUI is a **prebuilt aarch64 binary** of
**Falcon-mkxp**, a fork of Ancurio's *mkxp*. mkxp and all its forks are
licensed **GPL-2.0-or-later**, so the complete corresponding source is offered
here.

## Upstream lineage

```
Ancurio/mkxp  (original, GPL-2.0-or-later, Copyright (C) 2013 Jonas Kulla)
      │
      ▼
pk-2000/Falcon-mkxp   ("A fork of Ancurio's mkxp with unlocked resolution.
      │                 Supports all game resolutions." — ships prebuilt
      │                 Linux/Windows binaries; canonical Falcon-mkxp repo)
      ▼
JeremyRand/Falcon-mkxp (further community fork of pk-2000's tree)
```

- Original mkxp: https://github.com/Ancurio/mkxp — GPL-2.0-or-later
- Falcon-mkxp (canonical, has releases): https://github.com/pk-2000/Falcon-mkxp
- Falcon-mkxp (later fork): https://github.com/JeremyRand/Falcon-mkxp

Both Falcon-mkxp repositories ship an identical, verbatim GPL version 2
`COPYING` file (339 lines; byte-for-byte identical between the two forks). The
individual `src/*.cpp` files carry the mkxp copyright header:

```
** This file is part of mkxp.
** Copyright (C) 2013 Jonas Kulla <Nyocurio@gmail.com>
** mkxp is free software: you can redistribute it and/or modify
** it under the terms of the GNU General Public License as published by
** the Free Software Foundation, either version 2 of the License, or
** (at your option) any later version.
```

## Best available version identifier for the shipped build

**The exact fork and commit used to build the specific lifted aarch64 binary
kUI ships is NOT verified.** The binary was lifted from a PortMaster-style
RPG Maker port ("falcon-mkxp" classic engine) rather than built by kUI, and
the binary itself does not embed an authoritative upstream commit hash that
has been confirmed. This is flagged explicitly rather than guessed.

What can be stated with confidence:

- It is a **Falcon-mkxp** build (the "falcon" name is specific to this fork
  line), i.e. Ancurio/mkxp + the pk-2000 "unlocked resolution" changes.
- It binds **MRI (CRuby) 2.x**, matching the classic Falcon-mkxp binding
  (mkxp README: "written against 2.0"; qmake default looks for `ruby-2.1.pc`).
- The most likely upstream is **github.com/pk-2000/Falcon-mkxp** (the repo
  that publishes prebuilt binaries), or a downstream repackaging of it within
  the PortMaster RPG Maker runtime ecosystem.

## Obtaining corresponding source

To obtain the complete, corresponding source for the GPL-2.0-or-later engine
in this binary, clone the canonical Falcon-mkxp repository:

```
git clone https://github.com/pk-2000/Falcon-mkxp
```

with the original mkxp tree as its parent:

```
git clone https://github.com/Ancurio/mkxp
```

If you require the exact revision matching the shipped binary and it cannot be
determined from the above, request it from the kUI maintainer, who will
identify the precise PortMaster runtime build the binary was lifted from, or
substitute a from-source build of the pinned Falcon-mkxp commit.

## Third-party bundled libraries

Libraries linked into the binary (Ruby, SDL2 family, OpenAL Soft, libvorbis/
ogg, FreeType, libpng, zlib, PhysFS, pixman, Boost, etc.) are enumerated with
their own licenses, copyright holders, and source URLs in `THIRD-PARTY.md`
alongside this file.

### Additional copyleft dependencies (source offer)

Beyond the GPL-2.0-or-later engine itself, the shipped
`Data/PortMaster/libs/falcon-mkxp.squashfs` bundles the following **copyleft**
shared libraries under `libs/`, whose corresponding source is likewise offered
here (verified present via `unsquashfs`):

- **libmikmod** (`libmikmod.so.3`) — GNU Library General Public License,
  version 2 or later (LGPL-2.0-or-later; the "or later" grant covers
  LGPL-2.1). Complete corresponding source:

  ```
  git clone https://github.com/sezero/mikmod
  ```

  (upstream project: https://mikmod.sourceforge.net/)

The remaining bundled LGPL libraries (SDL_sound 1.x, OpenAL Soft, libsigc++,
libxcrypt) are shipped as separate, dynamically-linked `.so` files. Dynamic
linking satisfies the LGPL **relinking** condition (the user can replace the
library and re-link the combined work). That is a distinct obligation from
providing the library's source: LGPL-2.1 still requires the corresponding
source for **each LGPL library itself** to be made available. It is — the
complete corresponding source for each is available from its upstream below,
and on request:

- **SDL_sound 1.x** (`libSDL_sound-1.0.so.1`) — LGPL-2.1-or-later —
  https://github.com/Ancurio/SDL_sound (Ancurio's fork; the classic 1.x line
  linked by Falcon-mkxp). The generic icculus upstream is
  https://github.com/icculus/SDL_sound.
- **OpenAL Soft** — LGPL-2.1-or-later — https://github.com/kcat/openal-soft
- **libsigc++ 2.0** (`libsigc-2.0.so.0`) — LGPL-2.1-or-later —
  https://github.com/libsigcplusplus/libsigcplusplus (the `libsigc++-2` line;
  libsigc++ 3.x is LGPL-3.0).
- **libxcrypt** (`libcrypt.so.1`) — LGPL-2.1-or-later —
  https://github.com/besser82/libxcrypt

The other bundled shared libraries carry non-copyleft terms (libFLAC and
libtheoradec — BSD-3-Clause; libModPlug — public domain); all libraries are
itemized in `THIRD-PARTY.md`.

## Sources

- https://github.com/Ancurio/mkxp
- https://github.com/pk-2000/Falcon-mkxp
- https://github.com/JeremyRand/Falcon-mkxp
- https://github.com/pk-2000/Falcon-mkxp/blob/master/COPYING
- https://github.com/pk-2000/Falcon-mkxp/blob/master/README.md
- libmikmod (bundled copyleft dep) source + license:
  https://github.com/sezero/mikmod —
  https://raw.githubusercontent.com/sezero/mikmod/master/libmikmod/COPYING.LIB
- LGPL corresponding-source origins for the bundled LGPL `.so` libraries:
  - SDL_sound 1.x (LGPL-2.1-or-later): https://github.com/Ancurio/SDL_sound
    (generic upstream: https://github.com/icculus/SDL_sound)
  - OpenAL Soft (LGPL-2.1-or-later): https://github.com/kcat/openal-soft
  - libsigc++ 2.0 (LGPL-2.1-or-later): https://github.com/libsigcplusplus/libsigcplusplus
  - libxcrypt (LGPL-2.1-or-later): https://github.com/besser82/libxcrypt
