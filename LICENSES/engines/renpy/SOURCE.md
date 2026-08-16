# Ren'Py runtime — corresponding source (GPL/LGPL compliance)

The Ren'Py engine runtime kUI ships bundles several components under the GNU
LGPL. This file records where the corresponding source for those components can
be obtained, per the LGPL's source-availability obligation. (The Ren'Py engine
core itself is MIT and gl4es is MIT; those impose no source-delivery duty beyond
retaining their notices, but their upstreams are listed too for completeness.)

- **Runtime shipped:** Ren'Py 8.3.4.24120703 "Second Star to the Right",
  lifted from the official Ren'Py SDK for aarch64.
- **kUI-added component:** gl4es (MIT).

## What Ren'Py itself says about LGPL compliance

The SDK's own license file (`root/renpy/LICENSE.txt`, reproduced verbatim as
`LICENSE`) states that, for LGPL-compliance purposes, all source code Ren'Py
depends on lives in these repositories:

- https://github.com/renpy/renpy         — Ren'Py engine
- https://github.com/renpy/pygame_sdl2   — Pygame_SDL2
- https://github.com/renpy/renpy-build   — Dependencies (the native libraries)
- https://github.com/renpy/renpyweb      — Web support

Ren'Py Build downloads source from other git repositories as needed. The SDK
also suggests this wording for downstream distributors:

> This program contains free software licensed under a number of licenses,
> including the GNU Lesser General Public License. A complete list of software
> is available at http://www.renpy.org/doc/html/license.html.

## Ren'Py engine (MIT) — matching source

- **Component:** Ren'Py 8.3.4 engine + standard library.
- **License:** MIT/X11.
- **Matching source:** https://github.com/renpy/renpy at tag
  **8.3.4.24120703** (commit `8fdbdcfd034c303be564398b70aa34c894d4a29e`).
  There is no bare `8.3.4` git tag upstream; the dotted build tag is the
  corresponding-source pointer.
- **SDK downloads:** https://www.renpy.org/latest.html and the release archive
  at https://www.renpy.org/dl/8.3.4/

## LGPL components — corresponding source

For each LGPL component below, the exact source used by the Ren'Py binaries is
built/collected via `renpy-build`; the canonical upstream is also given. To
obtain the precise version linked into `librenpython.so`, use the pinned
revision in `renpy-build` for the 8.3.4 release.

### FFmpeg 4.3.1 (LGPL-2.1+)
- **Confirmed version:** 4.3.1 (from `librenpython.so` version string).
- **Corresponding source (exact):** the FFmpeg 4.3.1 sources as pinned by
  https://github.com/renpy/renpy-build (release/8.3 lineage).
- **Canonical upstream:** https://ffmpeg.org/releases/ffmpeg-4.3.1.tar.xz
  •  git: https://github.com/FFmpeg/FFmpeg (tag `n4.3.1`).

### FriBidi (LGPL-2.1-or-later)
- **Corresponding source:** as pinned in https://github.com/renpy/renpy-build
- **Canonical upstream:** https://github.com/fribidi/fribidi (release tarballs
  under Releases).

### chardet (LGPL-2.1+)
- **Location in runtime:** `lib/python3.12/chardet/` (pure Python; the shipped
  `.py`/bytecode is itself the source form).
- **Corresponding source:** https://github.com/chardet/chardet
- **PyPI:** https://pypi.org/project/chardet/

### libusb (LGPL-2.1-or-later)
- **Corresponding source:** as pinned in https://github.com/renpy/renpy-build
- **Canonical upstream:** https://github.com/libusb/libusb

### pygame_sdl2 (MIT + LGPL portions)
- **Corresponding source:** https://github.com/renpy/pygame_sdl2 (matching the
  8.3.4 release).

## gl4es (MIT) — matching source (bundled by kUI)

- **Component:** gl4es — OpenGL to OpenGL ES translation layer
  (`gl4es/libGL.so.1`, `gl4es/libEGL.so.1`).
- **License:** MIT (no copyleft source-delivery obligation; listed for
  completeness and to record the build origin).
- **Upstream source:** https://github.com/ptitSeb/gl4es
- **Note:** MIT imposes only notice retention; the verbatim MIT text and
  copyright are in `LICENSE`. The exact revision built for this runtime is not
  version-stamped in the binary (a build path string `/root/source/gl4es/lib`
  is present); if an exact-revision obligation ever arises, rebuild from the
  gl4es `master` branch at the time the runtime was produced.

## HarfBuzz (Old MIT) — bundled, permissive

- **Component:** HarfBuzz text-shaping library, statically linked into
  `librenpython.so` (461 `hb_*` symbols, including `hb_version_string`,
  confirmed present in the shipped binary).
- **License:** HarfBuzz "Old MIT" license — permissive, MIT-style. It imposes
  **no** copyleft corresponding-source delivery duty; only notice retention.
  The verbatim COPYING text is reproduced in `LICENSE` (in the kUI-added
  "Additional bundled components" section).
- **Upstream source:** https://github.com/harfbuzz/harfbuzz
- **Note:** HarfBuzz is not enumerated in the SDK manifest; it is present
  transitively (FreeType built with HarfBuzz shaping). Listed here for
  completeness — no source-availability obligation attaches.

## Other bundled licenses

Zlib/BSD/MIT/PSF/PNG/IJG/Apache/MPL components (SDL2 family, zlib, libpng,
libjpeg-turbo, libwebp, aom, libavif, CPython, requests, urllib3, GLEW,
tinyfiledialogs, bzip2, certifi, and the shipped Python packages idna
(BSD-3-Clause), ecdsa (MIT), pyasn1 (BSD-2-Clause) and future (MIT)) are
permissive and impose no
corresponding-source delivery duty beyond retaining their notices, which are in
`LICENSE`. Their upstream URLs are in THIRD-PARTY.md. certifi (MPL-2.0) requires
source availability for the certifi files themselves — obtainable at
https://github.com/certifi/python-certifi ; the shipped files under
`lib/python3.12/certifi/` are already in source form.

## Notes / uncertainties

- The precise pinned revisions for the native LGPL libraries (FFmpeg, FriBidi,
  libusb) come from `renpy-build` for the 8.3.4 release; only FFmpeg's version
  (4.3.1) was independently confirmed from the shipped binary.
- HarfBuzz IS bundled (confirmed by `hb_*` symbols in `librenpython.so`) but is
  not listed in the SDK manifest; it is a permissive Old-MIT component recorded
  above, with no corresponding-source obligation.
- SDL2_mixer is **not** listed in the SDK manifest and is not claimed as
  bundled (see THIRD-PARTY.md); no source obligation is recorded for it here.
