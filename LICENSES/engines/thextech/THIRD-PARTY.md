# TheXTech — Third-Party Components

TheXTech is licensed GPL-3.0-or-later (see `LICENSE`). It statically links a
number of third-party libraries into the single `thextech` executable, plus one
external input-helper binary shipped alongside it (`gptokeyb`).

The list below reflects what is **actually present in the aarch64 binary kUI
ships** (the PortMaster MegaVerse TheXTech port, engine version 1.3.7). It was
built by two means, both cited:

1. **Binary evidence** — symbol/version strings read directly from the shipped
   `thextech` ELF (`strings`/`readelf`). Where a library announces its own
   version or copyright, that line is quoted verbatim below.
2. **Upstream metadata** — TheXTech's `.gitmodules` (tag `v1.3.7`) and the
   WohlSoft `AudioCodecs` collection README, which enumerate the vendored
   dependencies and their licenses.

Dynamic dependencies of the binary (from `readelf -d`): `libSDL2-2.0.so.0`,
`libGLESv2.so.2`, and the standard C/C++ runtime. Everything else listed here is
statically linked in.

> Copyright lines marked "(standard upstream notice)" are the well-known
> canonical notice for that project rather than a string extracted from this
> particular binary. Items marked **[FLAG]** are uncertain — verify against the
> named upstream before relying on them.

---

## Confirmed linked into the `thextech` binary

### SDL2 — dynamically linked
- License: zlib License
- Copyright: © 1997–2024 Sam Lantinga <slouken@libsdl.org> (standard upstream notice)
- URL: https://github.com/libsdl-org/SDL
- Evidence: `NEEDED libSDL2-2.0.so.0`; "Compiled with SDL %d.%d.%d headers, running with SDL %d.%d.%d"

### SDL Mixer X (audio mixer) + AudioCodecs (audio decoders)
TheXTech uses WohlSoft's SDL-Mixer-X and its bundled AudioCodecs collection.
The individual codecs confirmed present in the binary are listed separately
below; SDL-Mixer-X itself is:
- License: zlib License (fork of SDL_mixer)
- Copyright: © Vitaly Novichkov "Wohlstand"; original SDL_mixer © Sam Lantinga
- URL: https://github.com/WohlSoft/SDL-Mixer-X , https://github.com/WohlSoft/AudioCodecs

### zlib
- License: zlib License
- Copyright (verbatim from binary): "deflate 1.2.13 Copyright 1995-2022 Jean-loup Gailly and Mark Adler"
- URL: https://zlib.net
- Evidence: version 1.2.13 strings present

### libpng
- License: PNG Reference Library License (libpng license)
- Copyright: © 1995–2023 The PNG Reference Library Authors (Glenn Randers-Pehrson et al.) (standard upstream notice)
- URL: http://www.libpng.org/pub/png/libpng.html
- Evidence (verbatim from binary): "libpng version 1.6.40 - June 21, 2023"

### FreeType
- License: FreeType License (FTL, BSD-style) OR GPL-2.0 (dual)
- Copyright: © 1996–2023 David Turner, Robert Wilhelm, and Werner Lemberg / The FreeType Project (standard upstream notice)
- URL: https://freetype.org (TheXTech fork: https://github.com/TheXTech/freetype)
- Evidence: FreeType/`FT_*` symbols present. NOTE: HarfBuzz is a listed submodule
  but **no HarfBuzz symbols were found in this binary** — this build links
  FreeType without HarfBuzz shaping.

### libogg
- License: BSD 3-Clause
- Copyright: © 2002–2020 Xiph.Org Foundation (standard upstream notice)
- URL: https://xiph.org/ogg/
- Evidence: `OggS` / `ogg` symbols present

### libvorbis
- License: BSD 3-Clause
- Copyright: © 2002–2020 Xiph.Org Foundation (standard upstream notice)
- URL: https://xiph.org/vorbis/
- Evidence: numerous `vorbis` symbols present

### libopus
- License: BSD 3-Clause
- Copyright: © 2001–2011 Xiph.Org, Skype Limited, Octasic, Jean-Marc Valin, Timothy B. Terriberry, et al. (standard upstream notice)
- URL: https://opus-codec.org
- Evidence: `Opus`/`OPUS` symbols present

### libADLMIDI (OPL3 FM synth MIDI)
- License: LGPL-3.0-or-later OR GPL-3.0-or-later (library core). Bundled OPL3
  emulator cores carry their own licenses (e.g. Nuked OPL3 LGPL-2.1+, DOSBox
  OPL GPL-2.0) — see upstream. **[FLAG]** exact emulator-core licenses vary.
- Copyright: © Vitaly Novichkov "Wohlstand"; emulator cores © their respective authors
- URL: https://github.com/Wohlstand/libADLMIDI
- Evidence: many `ADLMIDI_*` symbols present

### libOPNMIDI (YM2612/OPN2 FM synth MIDI)
- License: LGPL-3.0-or-later OR GPL-3.0-or-later (core); bundled emulator cores
  have their own licenses (GPL/LGPL/MIT mix) — see upstream. **[FLAG]** as above.
- Copyright: © Vitaly Novichkov "Wohlstand"; emulator cores © their respective authors
- URL: https://github.com/Wohlstand/libOPNMIDI
- Evidence: `OPNMIDI` / `OpnMidiSequencer` symbols present

### FLAC decoding — dr_flac
- License: Public Domain (Unlicense) OR MIT-0 (dual, single-header library)
- Copyright: David Reid (standard upstream notice)
- URL: https://github.com/mackron/dr_libs
- Evidence: `DRFLAC` / `drflac_*` symbols present. NOTE: FLAC support in this
  build is provided by **dr_flac**, not Xiph libFLAC (no `FLAC__*` symbols found).
  AudioCodecs lists libFLAC (BSD-3 / GPL) but it does not appear compiled into
  this binary. **[FLAG]** if precise FLAC provenance matters, verify.

### MP3 decoding — dr_mp3
- License: Public Domain (Unlicense) OR MIT-0 (dual, single-header library)
- Copyright: David Reid (standard upstream notice)
- URL: https://github.com/mackron/dr_libs
- Evidence: `drmp3_*` / `drmp3dec_*` symbols and `music_drmp3.c` present. NOTE:
  MP3 support in this build is provided by **dr_mp3**, not LAME. The dr_mp3.h
  header states its license is "Choice of public domain or MIT-0." No MP3
  *encoder* (LAME) is present.

### game-music-emu (GME — chiptune / console music player)
- License: **LGPL-2.1-or-later** (copyleft)
- Copyright: © Shay Green (blargg) and contributors (standard upstream notice)
- URL: https://github.com/libgme/game-music-emu
- Evidence: 187 `gme_*` / `Music_Emu` symbols and `music_gme.c` present. Upstream
  `license.txt` is the GNU LGPL v2.1 text.

### FluidLite (SoundFont/SF2 software MIDI synth)
- License: **LGPL-2.1-or-later** (copyleft)
- Copyright (verbatim from upstream `LICENSE`): "FluidLite (c) 2016 Robin Lobel"
- URL: https://github.com/divideconcept/FluidLite
- Evidence: `fluid_*` / `new_fluid_*` / `delete_fluid_*` symbols and
  `music_fluidlite.c` present. Upstream notice: "either version 2.1 of the
  License, or (at your option) any later version."

### libModPlug / OpenMPT (tracker module playback)
- License: Public Domain (libModPlug) / BSD-3 (OpenMPT) — see upstream
- Copyright: Olivier Lapicque and the OpenMPT / ModPlug contributors (standard upstream notice)
- URL: https://lib.openmpt.org , https://github.com/Konstanty/libmodplug
- Evidence: `OpenMPT` strings present

### libxmp (Extended Module Player — second tracker/module player)
- License: MIT
- Copyright (verbatim from upstream `docs/COPYING`): "Extended Module Player
  Copyright (C) 1996-2026 Claudio Matsuoka and Hipolito Carraro Jr"
- URL: https://github.com/libxmp/libxmp
- Evidence: 392 `xmp_*` / `libxmp_*` symbols and `music_xmp.c` present. This build
  links libxmp alongside libModPlug as a second module-format player.

### TiMidity-SDL (software MIDI wavetable synth)
- License: Artistic License
- Copyright: Tuukka Toivonen and contributors; SDL adaptation by the SDL_mixer authors (standard upstream notice)
- URL: https://github.com/WohlSoft/AudioCodecs (bundled)
- Evidence: `TIMIDITY` / `Timidity` / `timidity` strings and `music_timidity.c` present

### PxTone / PxTone Collage (.ptcop / .pttune music)
- License: Custom permissive license by the author (Studio Pixel). **Resolved:**
  the bespoke short license (originally Japanese) is reproduced verbatim in the
  WohlSoft `libpxtone` `LICENSE` file and grants, in the author's translation:
  *"The source code (content of 'src' and 'include' folders) required for
  playback can be used free of charge. Modification are okay. No special
  permission is required. We leave it to you to clarify the usage. We are not
  responsible for any problems caused by using this software."* Redistribution
  of the playback source and modifications is therefore expressly permitted;
  attribution is optional and no warranty is given. This satisfies the terms for
  the static link kUI ships.
- Copyright: © Studio Pixel / Daisuke "Pixel" Amaya (standard upstream notice)
- URL: bundled via https://github.com/WohlSoft/AudioCodecs — license text at
  https://github.com/Wohlstand/libpxtone (`LICENSE`); origin https://pxtone.org/
- Evidence: large number of `pxtn*` / `PTCOLLAGE-*` strings present

### FreeImageLite (libFreeImage — image loading, WohlSoft modded FreeImage)
- License: FreeImage Public License v1.0 OR GPL-2.0 OR GPL-3.0 (triple).
  **Resolved:** the WohlSoft `libFreeImage` repo ships all three license bodies
  — `license-fi.txt` (FreeImage Public License v1.0), `license-gplv2.txt`, and
  `license-gplv3.txt` — so a redistributor may elect the FIPL or either GPL
  option. See [`SOURCE.md`](./SOURCE.md) for the GPL corresponding-source
  pointer if the GPL option is relied on.
- Copyright: © Hervé Drolon, Floris van den Berg and the FreeImage contributors; modifications © Vitaly Novichkov
- URL: https://github.com/WohlSoft/libFreeImage
- Evidence: many `FreeImage*` symbols present

### {fmt} (string formatting)
- License: MIT
- Copyright: © 2012–present Victor Zverovich and {fmt} contributors (standard upstream notice)
- URL: https://github.com/fmtlib/fmt
- Evidence: `fmt::` symbols present (vendored in TheXTech source, not a submodule)

### PGE File Library (PGE-FL — SMBX level/world file formats)
- License: MIT
- Copyright: © Vitaly Novichkov "Wohlstand" (standard upstream notice)
- URL: https://github.com/WohlSoft/PGE-File-Library-STL
- Evidence: `PGE_*` / `*_PGEFF` symbols present

---

## License notice texts

Many of the statically linked libraries above are credited by name and upstream
only (e.g. the BSD ogg/vorbis/opus, the MIT {fmt}/PGE-FL/libxmp, the LGPL
GME/FluidLite/libADLMIDI/libOPNMIDI, FreeType, and PxTone). The full verbatim
license texts for the components credited above are available from their
upstream projects (URLs above) and on request; the authoritative text is each
project's own `LICENSE`/`COPYING`. Corresponding source for the copyleft
components is pointed to in [`SOURCE.md`](./SOURCE.md).

---

## Shipped alongside the engine (separate binary, NOT part of TheXTech)

### gptokeyb — PortMaster gamepad-to-keyboard input helper
- License: GPL-2.0 (full text in the port's `licenses/LICENSE.GPTOKEYB`)
- Copyright: The gptokeyb / PortMaster maintainers (kloptops / Jacob Smith and
  contributors), building on earlier work. **[FLAG]** — the bundled
  `LICENSE.GPTOKEYB` contains **only** the verbatim GPL-2.0 license text and no
  project-specific copyright line; the specific holder is not asserted in the
  shipped file. Confirm at upstream.
- URL: https://github.com/PortsMaster/gptokeyb
- Note: This is the PortMaster launcher's input mapper. It is **not** TheXTech
  and no `gptokeyb` symbols appear in the `thextech` binary. TheXTech itself uses
  the SDL GameController API for input.

---

## Listed as TheXTech submodules but NOT found in this build
For completeness, these appear in TheXTech's `.gitmodules` (v1.3.7) but produced
no symbols in the shipped aarch64 binary, i.e. they are not compiled into the
PortMaster build kUI ships:

- **HarfBuzz** (text shaping) — FreeType built without it here.
- **LuaJIT**, **luabind**, **luau** (Lua scripting) — no scripting symbols found.
- **thextech-discord-rpc** (Discord Rich Presence) — MIT; disabled in this build.
- **GLEW** (OpenGL extension loader; License: Modified BSD / MIT; upstream
  https://github.com/TheXTech/thextech-glew-cmake) — a real upstream submodule,
  but this GLES/aarch64 build does **not** link it: no `glew*` / `__glew`
  symbols are present. Earlier "GLEW" string hits were substrings of the engine
  symbol `LoadSingleWorld` (`…s_LoadSingleWorld…`), not the library.
- **WavPack** (audio codec; License: BSD-3; upstream https://www.wavpack.com) —
  listed by AudioCodecs but **not** linked here: no `Wavpack*` symbols. The lone
  `WAVPACK` string is a format-magic tag, not evidence of the decoder.
- **mbediso**, **angle-shader-translator**, **DirManager**, **IniProcessor**,
  **FileMapper** — build-support/utility submodules; not separately evidenced in
  the binary strings scan (some may be inlined). **[FLAG]** presence unverified.

---

## Sources
- Shipped binary: `~/dev/thextech-runtime/port/thextech/thextech` (aarch64 ELF, BuildID 0d429ba6…) — `readelf -d`, `strings`
- Bundled port licenses: `~/dev/thextech-runtime/port/thextech/licenses/{LICENSE.THEXTECH,LICENSE.GPTOKEYB}`
- TheXTech `.gitmodules` @ tag `v1.3.7`: https://github.com/TheXTech/TheXTech/blob/v1.3.7/.gitmodules
- TheXTech README (dependency overview): https://github.com/TheXTech/TheXTech
- AudioCodecs license listing: https://github.com/WohlSoft/AudioCodecs
