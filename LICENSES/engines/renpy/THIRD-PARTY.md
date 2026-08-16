# Ren'Py runtime — third-party components

This lists the third-party components bundled in the Ren'Py engine runtime that
kUI ships (Port Forge / visual novels).

- **Ren'Py version:** 8.3.4.24120703 "Second Star to the Right"
- **Source of the lifted runtime:** the official Ren'Py SDK for aarch64.
- **Authoritative component manifest:** `root/renpy/LICENSE.txt` in the lifted
  SDK, which enumerates the projects Ren'Py binaries can include and reproduces
  each license body verbatim. That file is reproduced verbatim in `LICENSE` in
  this directory. The list below **mirrors the SDK's own manifest** and adds
  gl4es (bundled by kUI, not by the SDK).

Full verbatim license texts for every entry below live in `LICENSE` in this
directory (from the SDK file) except gl4es, whose verbatim MIT text is appended
to the end of `LICENSE`.

## Verification notes (what is actually present in the lifted runtime)

Ren'Py links most native dependencies statically into
`lib/py3-linux-aarch64/librenpython.so`, so they do not appear as separate
`.so` files. Confirmed by inspecting that binary and the SDK license manifest:

- **FFmpeg 4.3.1** — confirmed by version string + `avcodec_*` symbols in
  `librenpython.so`. (LGPL — see SOURCE.md.)
- **libpng** — confirmed ("Application built with libpng-" string present).
- **HarfBuzz** — confirmed by 461 `hb_*` symbols (incl. `hb_version_string`) in
  `librenpython.so`. (Old MIT — see SOURCE.md.)
- Other components below are listed per the SDK's own manifest
  (`root/renpy/LICENSE.txt`); they are statically linked and not individually
  version-stamped in a way that is trivially extractable, so their presence is
  taken from the SDK manifest rather than re-derived. Where a component the task
  asked about is **not** in the SDK manifest, it is flagged as UNCERTAIN below.

---

## Components (per the Ren'Py 8.3.4 SDK manifest)

### CPython
- **License:** Python Software Foundation License (PSF); the SDK file titles the
  reproduced text "Python License".
- **Copyright:** Copyright (c) 2001-present Python Software Foundation. (The SDK
  file reproduces the older PSF agreement wording referencing "Copyright (c)
  2001, 2002 Python Software Foundation".)
- **Version:** Python 3.12 (from `lib/python3.12/` in the runtime; a 3.9 tree is
  also present under `lib/python3.9/`).
- **URL:** https://www.python.org/  •  https://docs.python.org/3/license.html

### pygame_sdl2
- **License:** MIT License and GNU LGPL (dual-component, per SDK manifest:
  "Pygame_SDL2 (MIT License, GNU LGPL)").
- **Copyright:** Copyright (c) the Ren'Py / pygame_sdl2 authors (Tom Rothamel and
  contributors); pygame_sdl2 derives from Pygame — Copyright (c) Pygame authors.
- **URL:** https://github.com/renpy/pygame_sdl2

### SDL2  (external dependency; NOT present in the lifted tree)
- **Status:** SDL2 is an external `DT_NEEDED` dependency of the Ren'Py binaries
  but does **not** ship inside this runtime — kUI must supply it. Credited as a
  dependency, not claimed as bundled here.
- **License:** Zlib License.
- **Copyright:** Copyright (c) 1997-2024 Sam Lantinga and the SDL contributors.
- **URL:** https://www.libsdl.org/  •  https://github.com/libsdl-org/SDL

### SDL2_image  (external dependency; NOT present in the lifted tree)
- **Status:** Like SDL2, an external `DT_NEEDED` dependency not present in the
  lifted tree; kUI must supply it. Credited as a dependency, not bundled here.
- **License:** Zlib License (per SDK manifest).
- **Copyright:** Copyright (c) 1997-2024 Sam Lantinga and contributors.
- **URL:** https://github.com/libsdl-org/SDL_image

### SDL2_ttf  (neither NEEDED nor used by this runtime)
- **Status:** SDL2_ttf is neither a `DT_NEEDED` dependency nor otherwise used by
  this runtime — Ren'Py uses its own FreeType/HarfBuzz text path. Listed in the
  SDK manifest but not applicable to this build.
- **License:** Zlib License (per SDK manifest).
- **Copyright:** Copyright (c) 1997-2024 Sam Lantinga and contributors.
- **URL:** https://github.com/libsdl-org/SDL_ttf

### SDL2_mixer — NOT in SDK manifest (task-requested; FLAGGED)
- **Status:** UNCERTAIN / likely NOT bundled. The Ren'Py 8.3.4 SDK manifest
  lists SDL2, SDL2_image and SDL2_ttf but **does not list SDL2_mixer**. Ren'Py
  does its own audio decoding/mixing via FFmpeg rather than SDL2_mixer. Not
  claimed as present unless later evidence shows otherwise.
- **URL (reference only):** https://github.com/libsdl-org/SDL_mixer

### FreeType
- **License (as stated by the SDK manifest):** the SDK's own manifest lists
  "Freetype (Zlib License)".
- **FLAG:** Upstream FreeType is normally dual-licensed under the **FreeType
  License (FTL, a BSD-style license with a credit clause)** or **GPLv2**, not
  Zlib. The SDK manifest's "Zlib" label appears inaccurate for FreeType proper.
  We mirror the SDK's statement but note this discrepancy; treat FreeType as
  FTL/GPLv2 for compliance purposes. Verify against the FreeType build Ren'Py
  ships if precise terms matter.
- **Copyright:** Copyright (c) 1996-2024 David Turner, Robert Wilhelm, and Werner
  Lemberg.
- **URL:** https://freetype.org/  •  https://gitlab.freedesktop.org/freetype/freetype

### FriBidi
- **License:** GNU LGPL (LGPL-2.1-or-later upstream; SDK manifest: "Fribidi
  (GNU LGPL)").
- **Copyright:** Copyright (c) the GNU FriBidi authors (Behdad Esfahbod and
  contributors); portions Copyright (c) 2004 Sharif FarsiWeb, Inc.
- **URL:** https://github.com/fribidi/fribidi  •  See SOURCE.md.

### FFmpeg  (confirmed present: 4.3.1)
- **License:** GNU LGPL (SDK manifest: "ffmpeg (GNU LGPL) (libav in some older
  versions, also GNU LGPL)"). Note: FFmpeg is LGPL-2.1+ in its default build;
  enabling GPL-only components would make it GPL. The SDK ships the LGPL build.
- **Copyright:** Copyright (c) 2000-2020 the FFmpeg developers.
- **Version:** 4.3.1 (confirmed via version string in `librenpython.so`).
- **URL:** https://ffmpeg.org/  •  See SOURCE.md.

### HarfBuzz  (CONFIRMED present; not in SDK manifest — added by kUI docs)
- **Status:** CONFIRMED bundled. HarfBuzz is statically linked into
  `librenpython.so` — 461 `hb_*` symbols, including `hb_version_string`, are
  present in the shipped binary. It is **not listed** in the Ren'Py 8.3.4 SDK
  manifest (present transitively: FreeType is built with HarfBuzz shaping).
- **License:** HarfBuzz "Old MIT" license (permissive, MIT-style; no copyleft
  source-delivery duty — notice retention only). Verbatim COPYING text is
  appended to `LICENSE` in the kUI-added "Additional bundled components" section.
- **Copyright:** Copyright (c) Behdad Esfahbod and others — including Google,
  Inc., Red Hat, Inc., Mozilla Foundation, and further contributors listed in
  the reproduced COPYING notice.
- **URL:** https://github.com/harfbuzz/harfbuzz  •  See SOURCE.md.

### libjpeg-turbo
- **License:** IJG License, Modified (3-clause) BSD License, and Zlib License
  (per SDK manifest, which reproduces the IJG "Jpeg License" text).
- **Copyright:** IJG portions Copyright (C) 1991-1998, Thomas G. Lane (per the
  SDK file); libjpeg-turbo portions Copyright (C) the libjpeg-turbo contributors
  (D. R. Commander and others).
- **URL:** https://libjpeg-turbo.org/  •  https://github.com/libjpeg-turbo/libjpeg-turbo

### libpng  (confirmed present)
- **License:** PNG Reference Library License (the "PNG License"; SDK manifest:
  "libpng (PNG License)").
- **Copyright:** Copyright (c) the PNG Reference Library authors — the
  Contributing Authors and Group 42, Inc. (as reproduced in the SDK file).
- **URL:** http://www.libpng.org/  •  https://github.com/pnggroup/libpng

### zlib
- **License:** Zlib License.
- **Copyright:** Copyright (C) 1995-2024 Jean-loup Gailly and Mark Adler.
- **URL:** https://zlib.net/

### bzip2 / libbzip2
- **License:** bzip2 License (BSD-style; SDK manifest: "bzip2 (Bzip2 License)").
- **Copyright:** Copyright (C) 1996-2005 Julian R Seward (per the SDK file).
- **URL:** https://sourceware.org/bzip2/

### libwebp
- **License:** Modified BSD License + WebM patent grant (SDK manifest: "libwebp
  (Modified BSD License, Patent License)").
- **Copyright:** Copyright (c) 2010, Google Inc. All rights reserved (per the SDK
  file).
- **URL:** https://developers.google.com/speed/webp  •  https://chromium.googlesource.com/webm/libwebp

### aom (AV1)
- **License:** AOM BSD License + Alliance for Open Media Patent License 1.0.
- **Copyright:** Copyright (c) 2016, Alliance for Open Media. All rights reserved
  (per the SDK file).
- **URL:** https://aomedia.googlesource.com/aom/

### libavif
- **License:** AOM BSD License (per SDK manifest).
- **Copyright:** Copyright 2019 Joe Drago; Copyright (c) 2018-2019 VideoLAN and
  dav1d authors (per the SDK file).
- **URL:** https://github.com/AOMediaCodec/libavif

### tinyfiledialogs
- **License:** Zlib License (per SDK manifest).
- **Copyright:** Copyright (c) 2014-2024 Guillaume Vareille.
- **URL:** https://sourceforge.net/projects/tinyfiledialogs/

### zsync
- **License:** Artistic License (SDK manifest: "zsync (Artistic License)"). The
  SDK file reproduces "The Artistic License, Version 2.0beta4". `zsync` /
  `zsyncmake` binaries are present under `lib/py3-linux-aarch64/`.
- **Copyright:** Copyright (C) the zsync authors (Colin Phipps). The reproduced
  license header notes "Copyright (C) 2000, Larry Wall" (Artistic License text).
- **URL:** http://zsync.moria.org.uk/

### GLEW
- **Status:** Listed per SDK manifest; not independently confirmed present in
  this runtime.
- **License:** Modified BSD License, MIT License (per SDK manifest).
- **Copyright:** Copyright (c) the GLEW authors (Nate Robins, Milan Ikits,
  Marcelo Magallon, Lev Povalahev).
- **URL:** https://github.com/nigels-com/glew

### requests
- **License:** Apache License 2.0.
- **Copyright:** Copyright (c) Kenneth Reitz and the Python requests contributors.
- **URL:** https://github.com/psf/requests

### certifi
- **License:** Mozilla Public License 2.0. (Present under `lib/python3.12/certifi/`.)
- **Copyright:** Copyright (c) Kenneth Reitz and certifi contributors.
- **URL:** https://github.com/certifi/python-certifi

### urllib3
- **License:** MIT License.
- **Copyright:** Copyright (c) the urllib3 contributors (Andrey Petrov et al.).
- **URL:** https://github.com/urllib3/urllib3

### chardet
- **License:** GNU LGPL (SDK manifest: "chardet (GNU LGPL)"). Present under
  `lib/python3.12/chardet/`.
- **Copyright:** Copyright (c) the chardet contributors; portions Copyright (c)
  Mark Pilgrim and the Mozilla Universal Charset Detector authors.
- **URL:** https://github.com/chardet/chardet  •  See SOURCE.md.

### libusb
- **Status:** Listed per SDK manifest; not independently confirmed present in
  this runtime.
- **License:** GNU LGPL (SDK manifest: "libusb (GNU LGPL)", LGPL-2.1-or-later
  upstream).
- **Copyright:** Copyright (c) the libusb authors.
- **URL:** https://libusb.info/  •  See SOURCE.md.

---

## Shipped Python packages NOT enumerated in the SDK manifest

These pure-Python packages are present under `lib/python3.12/` (and the parallel
`lib/python3.9/` tree) but are not named in the SDK's own license manifest. All
are permissive. The idna, ecdsa, pyasn1 and future notices are reproduced in
`LICENSE` (kUI-added "Additional bundled components" section); the rest are
credited below with license, copyright and upstream, and their full texts are
available at those upstreams / on request (see "License texts").

**This list is not a closed sweep.** The bundled Python trees carry more
third-party modules than the SDK manifest names; the entries below are the ones
confirmed present and credited so far. Each was verified by locating it in
`lib/python3.12/` and `lib/python3.9/` before being added. More may exist;
absence from this list is not a claim that nothing else is bundled.

### idna
- **License:** BSD-3-Clause.
- **Copyright:** Copyright (c) 2013-2026 Kim Davies and contributors.
- **URL:** https://github.com/kjd/idna  •  https://pypi.org/project/idna/

### ecdsa
- **License:** MIT License. (Portions written in 2005 by Peter Pearson placed in
  the public domain.)
- **Copyright:** Copyright (c) 2010 Brian Warner.
- **URL:** https://github.com/tlsfuzzer/python-ecdsa  •  https://pypi.org/project/ecdsa/

### pyasn1
- **License:** BSD-2-Clause.
- **Copyright:** Copyright (c) 2005-2020 Ilya Etingof.
- **URL:** https://github.com/pyasn1/pyasn1  •  https://pypi.org/project/pyasn1/

### future
- **License:** MIT License.
- **Copyright:** Copyright (c) 2013-2024 Python Charmers, Australia.
- **URL:** https://github.com/PythonCharmers/python-future  •  https://pypi.org/project/future/

### rsa
- **License:** Apache License 2.0. (Present as `lib/python3.12/rsa/` and
  `lib/python3.9/rsa/`.)
- **Copyright:** Copyright 2011 Sybren A. Stüvel <sybren@stuvel.eu>.
- **URL:** https://github.com/sybrenstuvel/python-rsa  •  https://pypi.org/project/rsa/

### six
- **License:** MIT License. (Present as the single-file module
  `lib/python3.12/six.pyc` and `lib/python3.9/six.pyc`.)
- **Copyright:** Copyright (c) 2010-2024 Benjamin Peterson.
- **URL:** https://github.com/benjaminp/six  •  https://pypi.org/project/six/

### PySocks (`socks`)
- **License:** BSD-3-Clause. (Present as the single-file module
  `lib/python3.12/socks.pyc` and `lib/python3.9/socks.pyc`; import name `socks`,
  distribution name PySocks.)
- **Copyright:** Copyright 2006 Dan-Haim. All rights reserved.
- **URL:** https://github.com/Anorov/PySocks  •  https://pypi.org/project/PySocks/

### websockets
- **License:** BSD-3-Clause. (Present as `lib/python3.12/websockets/` and
  `lib/python3.9/websockets/`.)
- **Copyright:** Copyright (c) Aymeric Augustin and contributors.
- **URL:** https://github.com/python-websockets/websockets  •  https://pypi.org/project/websockets/

### pefile (with `ordlookup`)
- **License:** MIT License. (Present as the single-file module
  `lib/python3.12/pefile.pyc` plus the companion package
  `lib/python3.12/ordlookup/`; both also present under `lib/python3.9/`.
  `ordlookup` ships alongside pefile and shares its authorship and license.)
- **Copyright:** Copyright (c) 2004-2024 Ero Carrera.
- **URL:** https://github.com/erocarrera/pefile  •  https://pypi.org/project/pefile/

### pyobjus
- **License:** MIT License. (Present as `lib/python3.12/pyobjus/` and
  `lib/python3.9/pyobjus/`. A Python↔Objective-C bridge used on Apple platforms;
  it is shipped in this tree even though this runtime is Linux/aarch64, so it is
  credited here.)
- **Copyright:** Copyright (c) 2010-2017 Kivy Team and other contributors.
- **URL:** https://github.com/kivy/pyobjus  •  https://pypi.org/project/pyobjus/

---

## Bundled fonts

These TrueType fonts ship inside the Ren'Py runtime and carry their own
licenses. The Ren'Py 8 fonts live under `root/renpy/common/`; the Quicksand
theme fonts live under the Ren'Py 7 tree at
`root7/renpy/common/_theme_awt/`. Each was verified present in the tree. The
Twemoji CC-BY-4.0 attribution is reproduced in `LICENSE`; the SIL OFL fonts
(_OpenDyslexic3, Quicksand) are covered by `LICENSES/OFL.txt`; the remaining font
notices (DejaVu / Bitstream Vera) are available from the upstreams above / on
request. Several also ship an upstream notice file (`*.txt` / `OFL.txt`)
alongside the font in the tree.

### DejaVuSans.ttf / DejaVuSans-Bold.ttf
- **License:** Bitstream Vera Fonts license + DejaVu changes (public domain);
  Arev-derived glyphs under the Tavmjong Bah Arev license. Permissive but
  **attribution/notice required** (the copyright and permission notices must
  accompany the fonts). Present under `root/renpy/common/` (and mirrored in
  `root7/renpy/common/`); shipped with `DejaVuSans.txt`.
- **Copyright:** Copyright (c) 2003 Bitstream, Inc. (Bitstream Vera); DejaVu
  changes in the public domain; Arev glyphs Copyright (c) 2006 Tavmjong Bah.
- **URL:** https://dejavu-fonts.github.io/

### TwemojiCOLRv0.ttf — CC-BY-4.0, REQUIRES ATTRIBUTION
- **License:** Emoji **graphics** under Creative Commons Attribution 4.0
  International (**CC-BY-4.0**); accompanying code under MIT. CC-BY-4.0 requires
  attribution to the creator, the copyright notice, a link to the license, and
  an indication of any changes. Present under `root/renpy/common/` (and
  `root7/renpy/common/`); shipped with `TwemojiCOLRv0.txt`.
- **Copyright:** Copyright 2019 Twitter, Inc and other contributors.
- **URL:** https://github.com/twitter/twemoji  •  https://github.com/Emoji-COLRv0/Emoji-COLRv0
- **Attribution notice (required):** reproduced verbatim in `LICENSE`.

### _OpenDyslexic3-Regular.ttf
- **License:** SIL Open Font License 1.1 (OFL-1.1). Reserved Font Name
  "OpenDyslexic". Present under `root/renpy/common/` (and `root7/renpy/common/`);
  shipped with `_OpenDyslexic3-Regular.txt` (full OFL text).
- **Copyright:** Copyright (c) 2019-07-29 Abbie Gonzalez
  (https://abbiecod.es), with Reserved Font Name OpenDyslexic; Copyright (c)
  12/2012 - 2019.
- **URL:** https://www.opendyslexic.org/  •  https://github.com/antijingoist/opendyslexic

### Quicksand-Regular.ttf / Quicksand-Bold.ttf
- **License:** SIL Open Font License 1.1 (OFL-1.1). Reserved Font Name
  "Quicksand". Present under `root7/renpy/common/_theme_awt/`; shipped with
  `OFL.txt` (full OFL text) in that directory.
- **Copyright:** Copyright (c) 2011 Andrew Paglinawan
  (www.andrewpaglinawan.com), with Reserved Font Name "Quicksand".
- **URL:** https://fonts.google.com/specimen/Quicksand

---

## Bundled by kUI (not part of the Ren'Py SDK)

### gl4es  (confirmed present: `gl4es/libGL.so.1`, `gl4es/libEGL.so.1`)
- **Purpose:** OpenGL -> OpenGL ES translation layer, so the Ren'Py GL renderer
  runs on the device's GLES driver.
- **License:** MIT License.
- **Copyright:** Copyright (c) 2016-2018 Sebastien Chevalier; Copyright (c)
  2013-2016 Ryan Hileman. (Verbatim MIT text is appended to `LICENSE`.)
- **URL:** https://github.com/ptitSeb/gl4es  •  See SOURCE.md.

---

## Components listed by the SDK manifest but not applicable to this platform

The SDK manifest also lists these; they are platform/build-specific and are not
expected in this Linux/aarch64 runtime, but are noted for completeness because
they appear in the reproduced SDK license file:

- **pyobjc** (MIT) — macOS only. https://github.com/ronaldoussoren/pyobjc
- **py2exe** (MIT) — Windows packaging only. https://github.com/py2exe/py2exe
- **pyjnius** (MIT) — Android only. https://github.com/kivy/pyjnius
- **ANGLE** (3-clause BSD) — used on some platforms for GL translation.
  https://github.com/google/angle

---

## License texts

The full verbatim license texts for the third-party components credited above
are available from their upstream projects (URLs above) and on request; the
authoritative text for each is that project's own LICENSE/COPYING file. The
Ren'Py SDK's own bundled-component texts, plus gl4es, HarfBuzz, idna, ecdsa,
pyasn1, future, and the Twemoji CC-BY-4.0 attribution, are reproduced in full in
this directory's `LICENSE`; the SIL OFL fonts are covered by `LICENSES/OFL.txt`.

---

## Sources

- SDK's own manifest and verbatim license bodies: `root/renpy/LICENSE.txt` in the
  lifted Ren'Py 8.3.4 SDK (reproduced as `LICENSE` here).
- Ren'Py version: `root/renpy/vc_version.py` — `8.3.4.24120703`.
- FFmpeg version and libpng presence: strings in
  `lib/py3-linux-aarch64/librenpython.so`.
- gl4es copyright/license: https://github.com/ptitSeb/gl4es (LICENSE), fetched
  verbatim.
- HarfBuzz presence: 461 `hb_*` symbols (incl. `hb_version_string`) in
  `lib/py3-linux-aarch64/librenpython.so`. License notice fetched verbatim from
  https://raw.githubusercontent.com/harfbuzz/harfbuzz/main/COPYING.
- idna / ecdsa / pyasn1 / future: present under `lib/python3.12/` (and
  `lib/python3.9/`). License notices fetched verbatim from their upstream
  repositories (kjd/idna, tlsfuzzer/python-ecdsa, pyasn1/pyasn1,
  PythonCharmers/python-future).
- rsa / six / PySocks (`socks`) / websockets / pefile (+ `ordlookup`) / pyobjus:
  present under `lib/python3.12/` and `lib/python3.9/` (verified by locating each
  module/package in the tree). License notices fetched verbatim via curl from
  their upstream repositories (sybrenstuvel/python-rsa, benjaminp/six,
  Anorov/PySocks, python-websockets/websockets, erocarrera/pefile, kivy/pyobjus).
- Bundled fonts: DejaVuSans / DejaVuSans-Bold / TwemojiCOLRv0 /
  _OpenDyslexic3-Regular present under `root/renpy/common/` (and mirrored under
  `root7/renpy/common/`); Quicksand-Regular / Quicksand-Bold present under
  `root7/renpy/common/_theme_awt/`. All confirmed present in the tree. Notices
  taken from the upstream `*.txt` / `OFL.txt` files shipped alongside the fonts;
  the OpenDyslexic copyright line was cross-checked via curl from
  antijingoist/opendyslexic (the shipped `.txt` omits the dated copyright line).
- Upstream project URLs cited per component above.

Entries marked **FLAGGED / UNCERTAIN** (SDL2_mixer, FreeType license label) are
called out because they were requested but either are not in the SDK manifest or
the SDK's stated license appears inconsistent with the upstream. HarfBuzz, once
uncertain, is now CONFIRMED bundled and documented above. No copyright holder or
license has been invented; where a holder is stated it comes from the SDK file
or the component's canonical upstream.
