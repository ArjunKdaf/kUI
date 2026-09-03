# Port Forge engine runtime licenses

kUI ships the Port Forge engine runtimes as squashfs images in
`Data/PortMaster/libs/`. Each is built from unmodified upstream sources and
governed by its own upstream license. Under `LICENSES/engines/<engine>/`:
`LICENSE` reproduces the **engine's own** license text verbatim (and, where an
engine bundles them inline, its components' texts — e.g. Ren'Py); `THIRD-PARTY.md`
**credits** each bundled component (name, license, copyright, upstream URL) with
its full verbatim text available at that upstream / on request; and `SOURCE.md`
gives the GPL/LGPL corresponding-source pointers. For the GPL engines,
corresponding source is available per each `SOURCE.md`.

| Engine | Upstream | Version | License | Shipped |
|---|---|---|---|---|
| mkxp-z | github.com/mkxp-z/mkxp-z | ~v2.4.2 | GPL-2.0-or-later¹ | lifted binary |
| falcon-mkxp | github.com/pk-2000/Falcon-mkxp (fork of Ancurio/mkxp) | fork build² | GPL-2.0-or-later | lifted binary |
| Ren'Py | github.com/renpy/renpy (+ ptitSeb/gl4es) | 8.3.4 SDK | MIT core + bundled LGPL/PSF/etc.³ | lifted from SDK |
| Solarus | gitlab.com/solarus-games/solarus | v1.6.5 | GPL-3.0 | lifted binary |
| TheXTech | github.com/TheXTech/TheXTech | v1.3.7 (+hotfix1) | GPL-3.0-or-later | lifted binary |

Notes:

1. **mkxp-z**: the engine's own code is GPL-2.0-or-later, but the shipped
   build links OpenSSL for HTTPS, which makes the *combined binary* effectively
   GPL-3.0-or-later. The exact revision is recoverable from the binary
   (`MKXPZ_GIT_HASH`); ~v2.4.2 is the best-effort identifier. This is the modern
   engine (MRI Ruby 3.x); `falcon-mkxp` is the classic engine (MRI Ruby 2.x) for
   older RPG Maker games.
2. **falcon-mkxp**: a fork in the Ancurio/mkxp → pk-2000 → JeremyRand lineage.
   The exact fork commit for the lifted binary is not embedded and is marked
   unverified in its `SOURCE.md`.
3. **Ren'Py**: the engine core is MIT/X11; the SDK bundles components under
   other licenses (CPython/PSF, pygame_sdl2/LGPL-2.1, FFmpeg/LGPL, FreeType,
   libpng, zlib, …). `engines/renpy/LICENSE` reproduces the SDK's own
   `LICENSE.txt` in full so every bundled body is present; gl4es (MIT) is added
   by kUI for GL→GLES translation. SDL2 is an external dependency here, supplied
   by the device (not bundled in this runtime).

Common libraries such as SDL2, OpenAL, libvorbis/ogg, zlib and pixman are
device-provided for most engines and not bundled — but this varies per engine.
Each is
credited as a dependency in the relevant engine's `THIRD-PARTY.md`, which states
whether it is bundled or device-provided.
