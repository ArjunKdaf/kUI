# EasyRPG engine runtime — GPL corresponding source

The EasyRPG runtime kUI ships (Port Forge, RPG Maker 2000/2003) is
**built from source** for aarch64 against the kUI toolchain — it is **not** a
lifted or prebuilt third-party binary. The program is **EasyRPG Player
0.8.1.1**, licensed **GPL-3.0-or-later**; this file offers the complete
corresponding source as required by the GPL.

## Primary components and exact revisions

Both were built from their tagged upstream releases (git tags verified in the
build tree at `~/dev/easyrpg-runtime/src`):

| Component | Version / tag | Commit | License | Upstream |
|---|---|---|---|---|
| EasyRPG Player | `0.8.1.1` | `78328fa29f465315291e161130e6682f69410370` | GPL-3.0-or-later | https://github.com/EasyRPG/Player |
| liblcf | `0.8.1` | `92c4450a1bc1acb58bd02bbb99b57e5036919cdf` | MIT | https://github.com/EasyRPG/liblcf |

Direct source tarballs / tags:

- EasyRPG Player 0.8.1.1: https://github.com/EasyRPG/Player/releases/tag/0.8.1.1
  (`git clone https://github.com/EasyRPG/Player && git checkout 0.8.1.1`)
- liblcf 0.8.1: https://github.com/EasyRPG/liblcf/releases/tag/0.8.1
  (`git clone https://github.com/EasyRPG/liblcf && git checkout 0.8.1`)

Because the runtime is built from unmodified upstream source, the corresponding
source **is the upstream source at the tags above** — no kUI patches were
applied to Player or liblcf.

## Statically linked GPL/LGPL dependencies

The `easyrpg-player` binary statically links **mpg123 1.33.7**
(LGPL-2.1-or-later), so its corresponding source is offered too. It was built
from the upstream release tarball, which is retained verbatim in the build tree:

- mpg123 1.33.7: https://www.mpg123.de/ — tarball
  `~/dev/easyrpg-runtime/src/mpg123-1.33.7.tar.bz2` (unpacked at
  `~/dev/easyrpg-runtime/src/mpg123-src`).

The other statically linked libraries are permissive (liblcf — MIT, fmt 9.1.0 —
MIT, inih — BSD-3-Clause); their sources are at the upstreams listed in
`THIRD-PARTY.md`.

The bundled shared `.so` audio/image libraries are dynamically linked and
enumerated in `THIRD-PARTY.md`. One of them is copyleft: **libsndfile**
(`libsndfile.so.1`, LGPL-2.1-or-later). Shipping it as a separate,
dynamically-linked `.so` satisfies the LGPL **relinking** condition — but that
is a distinct obligation from providing the library's own source. LGPL-2.1
still requires the corresponding source for the libsndfile library itself to be
made available, and it is — from upstream and on request:

- **libsndfile** (`libsndfile.so.1`) — LGPL-2.1-or-later —
  https://github.com/libsndfile/libsndfile (project site
  https://libsndfile.github.io/libsndfile/).

The remaining bundled shared libraries are permissive (libpng — Libpng
license; libsamplerate — BSD-2-Clause; libvorbis / libvorbisfile / libogg —
BSD-3-Clause); their sources are at the upstreams listed in `THIRD-PARTY.md`.

## Build recipe / where the source lives

The complete build tree, toolchain file and staging output live on the kUI
maintainer's machine at:

```
~/dev/easyrpg-runtime/
  toolchain.cmake                     # aarch64 cross toolchain (kUI)
  src/Player/                         # EasyRPG Player 0.8.1.1 (git @ tag 0.8.1.1)
  src/liblcf/                         # liblcf 0.8.1 (git @ tag 0.8.1)
  src/fmt/                            # fmt 9.1.0 (git @ tag 9.1.0)
  src/inih/                           # inih (git snapshot)
  src/mpg123-1.33.7.tar.bz2 + src/mpg123-src/   # mpg123 1.33.7
  staging/                            # built static libs + headers
  pkg/easyrpg/                        # packaged runtime (binary + libs/ + LICENSES/)
  easyrpg.squashfs                    # final on-device image
  pkg/easyrpg/LICENSES/SOURCES.txt    # upstream build notes
```

The cross-compile targets aarch64 / glibc 2.28. SDL2, zlib, pixman and ALSA are
provided by the device and are not rebuilt here (see `THIRD-PARTY.md` §3).

## Obtaining corresponding source

Anyone entitled to the corresponding source under the GPL can reconstruct the
exact build by cloning the two primary repos at the tags above plus the pinned
mpg123 1.33.7 tarball, then cross-compiling with the kUI aarch64 toolchain. If
the archived `~/dev/easyrpg-runtime` tree or the toolchain file is needed,
request it from the kUI maintainer.

## Sources

- https://github.com/EasyRPG/Player (tag 0.8.1.1)
- https://github.com/EasyRPG/liblcf (tag 0.8.1)
- https://github.com/fmtlib/fmt (tag 9.1.0)
- https://github.com/benhoyt/inih
- https://www.mpg123.de/ (mpg123 1.33.7)
- libsndfile (bundled LGPL-2.1-or-later dep) corresponding source:
  https://github.com/libsndfile/libsndfile
- Build notes: `~/dev/easyrpg-runtime/pkg/easyrpg/LICENSES/SOURCES.txt`
