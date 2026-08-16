# Corresponding source — Solarus 1.6.5 engine runtime

The Solarus engine (`solarus-1.6.5` launcher binary + `libsolarus.so.1`) that
kUI ships is licensed under the **GNU General Public License, version 3**
(see [`LICENSE`](./LICENSE)). GPL-3.0 requires that the complete corresponding
source for the distributed binaries be made available. This file records where
that source is and how to obtain a matching copy.

## What was shipped

- **Engine:** Solarus **1.6.5** (aarch64), a LuaJIT-enabled build.
- **Provenance:** the binaries were lifted from the **PortMaster
  `solarus-1.6.5` runtime** (`solarus-1.6.5.squashfs`). The build path embedded
  in the binaries is `/root/compile/solarus_luajit/build`, i.e. Solarus built
  against LuaJIT rather than the reference Lua interpreter.
- **Files:** `solarus-1.6.5` (launcher), `libsolarus.so.1` (engine library).
  Ports reference this runtime via their `port.json`
  (`"runtime": ["solarus-1.6.5.squashfs"]`).

kUI did not modify the Solarus source; it repackages the upstream 1.6.5 build.
If any patch is ever applied, that patch must be published alongside this
source reference to keep GPL corresponding-source complete.

## Upstream source (Solarus, GPL-3.0)

Solarus development moved from GitHub to GitLab; the `1.6.5` release is tagged
on GitLab. Both locations serve the same project.

- **Canonical repo (tagged 1.6.5):** https://gitlab.com/solarus-games/solarus
  - Tag: **`v1.6.5`**
  - License files as shipped in the tag:
    - https://gitlab.com/solarus-games/solarus/-/raw/v1.6.5/license.txt
    - https://gitlab.com/solarus-games/solarus/-/raw/v1.6.5/license_gpl.txt
      (the verbatim GPL-3.0 body reproduced in [`LICENSE`](./LICENSE);
      md5 `d32239bcb673463ab874e80d47fae504`)
- **GitHub mirror:** https://github.com/solarus-games/solarus
  (note: at time of writing the GitHub mirror does not carry a `1.6.5` tag;
  use the GitLab tag above for the exact source.)
- **Project home:** https://www.solarus-games.org/

### Obtaining the matching source

```sh
# From the canonical GitLab repo, checked out at the exact release tag:
git clone https://gitlab.com/solarus-games/solarus.git
cd solarus
git checkout v1.6.5

# Or download the tagged tarball directly:
#   https://gitlab.com/solarus-games/solarus/-/archive/v1.6.5/solarus-v1.6.5.tar.gz
```

Build instructions for this tag are in the repo's `compilation.txt` /
`readme.md`. The kUI runtime is the LuaJIT variant: configure Solarus to link
against LuaJIT (`libluajit-5.1`) instead of plain Lua. See the PortMaster
runtime build for reference (below).

## PortMaster runtime build reference

The exact runtime binaries kUI lifted come from PortMaster's Solarus runtime.
Its build recipe (how the `solarus-1.6.5.squashfs` is produced, including the
LuaJIT link) is public:

- PortMaster runtimes / build scripts:
  https://github.com/PortsMaster/PortMaster-Runtime
- General PortMaster project: https://github.com/PortsMaster

## LGPL dependency source (OpenAL Soft)

The runtime links **OpenAL Soft**, which is LGPL-licensed (see
[`THIRD-PARTY.md`](./THIRD-PARTY.md)). LGPL also requires corresponding source
to be available. OpenAL Soft is provided by the device OS image, not bundled by
kUI, but its source is:

- https://github.com/kcat/openal-soft — https://openal-soft.org/

## Other bundled libraries

The permissively licensed bundled libraries (LuaJIT — MIT, PhysFS — zlib,
libmodplug — public domain) do not carry a copyleft source-distribution
obligation, but their upstreams and copyright are recorded in
[`THIRD-PARTY.md`](./THIRD-PARTY.md) for completeness and attribution:

- LuaJIT 2.1.0-beta3 — https://github.com/LuaJIT/LuaJIT
- PhysFS 3.0.1 — https://github.com/icculus/physfs
- libmodplug — https://github.com/Konstanty/libmodplug
