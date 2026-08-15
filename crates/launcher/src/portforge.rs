//! Port Forge — turn a user's RPG Maker game (a folder or its `.exe`)
//! into a standard PortMaster port on the card, driven by the bundled
//! mkxp-z runtime. Offline, local: the user brings the game, kUI brings
//! the ARM engine. This module is the import ENGINE (no UI); the Control
//! Panel screen calls `forge()` on a background thread.
//!
//! The recipe (see docs/PORTFORGE.md): resolve to the folder holding
//! Game.ini, strip the
//! Windows engine, place the data at the port GAMEDIR root, write the
//! mkxp-z launch script (mount squashfs + shared swap) + mkxp.json +
//! controls, and lift boxart from Graphics/Titles.

use kui_store::Json;
use std::path::{Path, PathBuf};

// --- card path helpers (mirror kui_store::ports; trivial joins) ---
fn scripts_dir(sd_root: &Path) -> PathBuf {
    sd_root.join("Roms/Ports (PORTS)")
}
fn data_ports_dir(sd_root: &Path) -> PathBuf {
    sd_root.join("Data/ports")
}
fn libs_dir(sd_root: &Path) -> PathBuf {
    sd_root.join("Data/PortMaster/libs")
}
fn media_dir(sd_root: &Path) -> PathBuf {
    scripts_dir(sd_root).join(".media")
}

/// The mkxp-z runtime the emitted ports mount. Shipped with kUI's ports
/// control layer (from Jeod's RHH-Ports, GPL); Port Forge only uses it.
pub fn runtime_present(sd_root: &Path) -> bool {
    libs_dir(sd_root).join("mkxp-z.squashfs").is_file()
}

/// Windows / packaging files that must NOT go into the port — the engine
/// is mkxp-z, not `Game.exe`, and the rest is cruft. Matched by exact
/// name (case-insensitive) or by these extensions.
const STRIP_NAMES: &[&str] = &[
    "game.exe",
    "install_or_update.bat",
    "launch-wine.sh",
    ".ds_store",
    "savefile.lnk",
    "credits.txt",
    "readme.txt",
    "readme.md",
];
const STRIP_EXTS: &[&str] = &["exe", "dll", "bat", "lnk"];
const STRIP_DIRS: &[&str] = &[".git", ".idea", "required_by_installer_updater"];

fn is_stripped(name: &str, is_dir: bool) -> bool {
    let lower = name.to_ascii_lowercase();
    if is_dir {
        return STRIP_DIRS.contains(&lower.as_str());
    }
    if STRIP_NAMES.contains(&lower.as_str()) {
        return true;
    }
    Path::new(&lower)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| STRIP_EXTS.contains(&e))
}

/// A slug for `Data/ports/<slug>` and the payload dir: lowercase,
/// alphanumerics kept, everything else → nothing. Falls back to "game".
fn slugify(title: &str) -> String {
    let s: String = title
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect::<String>()
        .to_ascii_lowercase();
    if s.is_empty() { "game".into() } else { s }
}

/// A safe file name for the `.sh` and boxart (keep spaces, drop path/odd
/// chars). Falls back to the slug.
fn safe_name(title: &str, slug: &str) -> String {
    let s: String = title
        .chars()
        .map(|c| if c.is_alphanumeric() || " _-!'.".contains(c) { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if s.trim().is_empty() { slug.to_string() } else { s.trim().to_string() }
}

/// The RMXP-game marker: a **root-level `.ini`** whose `Library=` line
/// names an RGSS runtime. RPG Maker names this after the executable —
/// `Game.ini` for a `Game.exe` launcher, `Foo.ini` for `Foo.exe`, etc. —
/// so we scan for any matching `.ini` rather than hardcoding `Game.ini`.
/// `Game.ini` wins if present.
fn find_rmxp_ini(dir: &Path) -> Option<PathBuf> {
    let mut inis: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .and_then(|x| x.to_str())
                .is_some_and(|x| x.eq_ignore_ascii_case("ini"))
        })
        .collect();
    // prefer Game.ini, then a stable order so the pick is deterministic
    inis.sort_by_key(|p| {
        let n = p.file_name().map(|s| s.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        (n != "game.ini", n)
    });
    for p in inis {
        if let Ok(text) = std::fs::read_to_string(&p)
            && text.lines().any(|l| {
                let u = l.trim().to_ascii_uppercase();
                u.starts_with("LIBRARY=") && u.contains("RGSS")
            })
        {
            return Some(p);
        }
    }
    None
}

/// True if `dir` is directly an RPG Maker game (holds a root-level RMXP
/// `.ini`). The picker uses this to tag forge-able folders.
pub fn is_rmxp_game(dir: &Path) -> bool {
    dir.is_dir() && find_rmxp_ini(dir).is_some()
}

/// True if `dir` is a **Port Forge Web package**: a pre-built port carrying a
/// `portforge.json` manifest. Built off-device by the web tool (which handles
/// any engine + its dependencies), it just needs placing. The picker tags these
/// so selecting one INSTALLS (moves into place) instead of forging. This is
/// engine-agnostic — the device never parses what engine made the package.
pub fn is_port_package(dir: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(dir.join("portforge.json")) else {
        return false;
    };
    Json::parse(&text)
        .ok()
        .and_then(|j| j.get("format").and_then(Json::as_str).map(|s| s == "portforge-port"))
        .unwrap_or(false)
}

/// Read an RMXP `.ini`'s `Title=`. Falls back to the folder name.
fn read_title(ini: &Path) -> Result<String, String> {
    let text = std::fs::read_to_string(ini)
        .map_err(|e| format!("reading {}: {e}", ini.display()))?;
    let mut title = None;
    for line in text.lines() {
        let l = line.trim();
        if let Some(v) = l.strip_prefix("Title=").or_else(|| l.strip_prefix("Title =")) {
            title = Some(v.trim().to_string());
        }
    }
    Ok(title.filter(|t| !t.is_empty()).unwrap_or_else(|| {
        ini.parent()
            .and_then(|d| d.file_name())
            .map_or_else(|| "Game".into(), |s| s.to_string_lossy().into_owned())
    }))
}

/// Resolve the picked folder to the directory holding its RMXP `.ini`. The
/// picker only offers folders that already contain a root-level `.ini` (see
/// `is_rmxp_game`), so this is simply that folder.
fn resolve_game_dir(source: &Path) -> Result<PathBuf, String> {
    if source.is_dir() && find_rmxp_ini(source).is_some() {
        Ok(source.to_path_buf())
    } else {
        Err("no RPG Maker game (.ini) found in that folder".into())
    }
}

/// mkxp-z reads `Game.ini` and mounts `Game.<rgss archive>` by name at
/// runtime (its `execName` defaults to "Game"). Games named after their
/// executable (e.g. `Foo.ini` + `Foo.rgssad`) must be normalized to
/// `Game.*` inside the port or the engine won't find them. Idempotent.
fn normalize_engine_names(port_dir: &Path, src_ini_name: &str) {
    if !src_ini_name.eq_ignore_ascii_case("Game.ini") {
        let _ = std::fs::copy(port_dir.join(src_ini_name), port_dir.join("Game.ini"));
    }
    for ext in ["rgssad", "rgss2a", "rgss3a"] {
        if port_dir.join(format!("Game.{ext}")).exists() {
            continue;
        }
        if let Ok(rd) = std::fs::read_dir(port_dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p.extension()
                    .and_then(|x| x.to_str())
                    .is_some_and(|x| x.eq_ignore_ascii_case(ext))
                {
                    let _ = std::fs::rename(&p, port_dir.join(format!("Game.{ext}")));
                    break;
                }
            }
        }
    }
}

/// Total byte size of a file/tree (recursive). Used to size the progress
/// bar before placing the game.
fn tree_size(p: &Path) -> u64 {
    if is_symlink(p) {
        return 0;
    }
    if p.is_dir() {
        std::fs::read_dir(p)
            .map_or(0, |rd| rd.flatten().map(|e| tree_size(&e.path())).sum())
    } else {
        std::fs::metadata(p).map_or(0, |m| m.len())
    }
}

/// Bytes to be placed = sum of the non-stripped top-level entries. Matches
/// exactly what `place_game_data` will move/copy, so the bar reaches 100%.
fn place_total(src: &Path) -> u64 {
    let Ok(rd) = std::fs::read_dir(src) else { return 0 };
    let mut total = 0;
    for e in rd.flatten() {
        let name = e.file_name();
        let name = name.to_string_lossy();
        let p = e.path();
        if is_stripped(&name, p.is_dir()) {
            continue;
        }
        total += tree_size(&p);
    }
    total
}

/// Decrypt an RGSSAD **v1** archive (RGSS1/RMXP) and write its members as
/// loose files under `dest` (never overwriting a file already there). The
/// cipher: a key seeded at 0xDEADCAFE advances `k = k*7+3` per header field
/// and per name byte; file data is XORed with a local copy of the key,
/// byte-by-byte, advancing every 4 bytes. Returns the number of files, or an
/// error (unknown magic / unsupported version) so the caller can leave the
/// archive in place.
fn extract_rgssad(archive: &Path, dest: &Path) -> Result<usize, String> {
    let data = std::fs::read(archive).map_err(|e| format!("read rgssad: {e}"))?;
    if data.len() < 8 || &data[0..7] != b"RGSSAD\0" {
        return Err("not an RGSSAD archive".into());
    }
    if data[7] != 1 {
        return Err(format!("unsupported RGSSAD version {}", data[7]));
    }
    let adv = |k: u32| k.wrapping_mul(7).wrapping_add(3);
    let mut key: u32 = 0xDEAD_CAFE;
    let mut pos = 8usize;
    let mut count = 0usize;
    while pos + 4 <= data.len() {
        let namelen =
            (u32::from_le_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]) ^ key)
                as usize;
        key = adv(key);
        pos += 4;
        if namelen == 0 || namelen > 1000 || pos + namelen > data.len() {
            break;
        }
        let mut name = Vec::with_capacity(namelen);
        for _ in 0..namelen {
            name.push(data[pos] ^ (key & 0xFF) as u8);
            key = adv(key);
            pos += 1;
        }
        let name = String::from_utf8_lossy(&name).replace('\\', "/");
        if pos + 4 > data.len() {
            break;
        }
        let size =
            (u32::from_le_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]) ^ key)
                as usize;
        key = adv(key);
        pos += 4;
        if size > data.len() || pos + size > data.len() {
            break;
        }
        let mut buf = data[pos..pos + size].to_vec();
        let mut dk = key;
        for (i, b) in buf.iter_mut().enumerate() {
            *b ^= (dk >> (8 * (i & 3))) as u8;
            if i & 3 == 3 {
                dk = adv(dk);
            }
        }
        pos += size;
        // reject path escapes from an untrusted archive (../ and absolute or
        // rooted names — an absolute join would discard `dest`)
        if Path::new(&name).components().any(|c| {
            matches!(
                c,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        }) {
            continue;
        }
        // skip if a loose file already exists there
        let out = dest.join(&name);
        if !out.exists() {
            if let Some(parent) = out.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            std::fs::write(&out, &buf).map_err(|e| format!("write {}: {e}", out.display()))?;
        }
        count += 1;
    }
    Ok(count)
}

/// Copy the game data into `dest`, skipping the Windows engine + cruft, and
/// any symlink (games don't ship them; a self-referential one would loop).
/// The user's folder must survive, so this always copies. `bump` is fed the
/// byte count of each entry placed, to drive the progress bar.
fn place_game_data(src: &Path, dest: &Path, bump: &mut dyn FnMut(u64)) -> Result<(), String> {
    std::fs::create_dir_all(dest).map_err(|e| format!("mkdir {}: {e}", dest.display()))?;
    let rd = std::fs::read_dir(src).map_err(|e| format!("read {}: {e}", src.display()))?;
    for e in rd.flatten() {
        let name = e.file_name();
        let name = name.to_string_lossy();
        let p = e.path();
        if is_symlink(&p) {
            continue;
        }
        let is_dir = p.is_dir();
        if is_stripped(&name, is_dir) {
            continue;
        }
        let target = dest.join(&*name);
        if is_dir {
            copy_dir(&p, &target, bump)?;
        } else {
            std::fs::copy(&p, &target)
                .map_err(|e| format!("copy {}: {e}", p.display()))?;
            bump(std::fs::metadata(&target).map_or(0, |m| m.len()));
        }
    }
    Ok(())
}

/// True if `p` is a symlink (used to avoid following them into loops/escapes).
fn is_symlink(p: &Path) -> bool {
    std::fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_symlink())
}

fn copy_dir(src: &Path, dest: &Path, bump: &mut dyn FnMut(u64)) -> Result<(), String> {
    std::fs::create_dir_all(dest).map_err(|e| format!("mkdir {}: {e}", dest.display()))?;
    let rd = std::fs::read_dir(src).map_err(|e| format!("read {}: {e}", src.display()))?;
    for e in rd.flatten() {
        let p = e.path();
        if is_symlink(&p) {
            continue;
        }
        let target = dest.join(e.file_name());
        if p.is_dir() {
            copy_dir(&p, &target, bump)?;
        } else {
            std::fs::copy(&p, &target).map_err(|e| format!("copy {}: {e}", p.display()))?;
            bump(std::fs::metadata(&target).map_or(0, |m| m.len()));
        }
    }
    Ok(())
}

/// Read a PNG's pixel dimensions from its IHDR (first 24 bytes). None if
/// it isn't a PNG or is truncated.
fn png_dims(path: &Path) -> Option<(u32, u32)> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).ok()?;
    let mut hdr = [0u8; 24];
    f.read_exact(&mut hdr).ok()?;
    if &hdr[0..8] != b"\x89PNG\r\n\x1a\n" {
        return None;
    }
    let w = u32::from_be_bytes([hdr[16], hdr[17], hdr[18], hdr[19]]);
    let h = u32::from_be_bytes([hdr[20], hdr[21], hdr[22], hdr[23]]);
    Some((w, h))
}

/// Pick a boxart from `Graphics/Titles/`. Some RPG Maker title screens are
/// LAYERED (logo + background + particle/shine/effect layers composited at
/// runtime), so "largest file" or "has 'logo' in the name" grabs junk (a
/// bare wordmark, a gradient, an effect sheet). Instead: keep only
/// full-frame images (both sides ≥ 200 px — drops wordmarks/particles),
/// boost title-ish names, penalize obvious layer names, tiebreak on size.
/// Falls back to the largest PNG if nothing is full-frame. Copied (not
/// resized) to the port's `.media/<name>.png`.
fn place_boxart(game_dir: &Path, sd_root: &Path, safe: &str) -> Result<(), String> {
    let titles = game_dir.join("Graphics").join("Titles");
    let Ok(rd) = std::fs::read_dir(&titles) else { return Ok(()) };
    const POS: &[&str] = &["title", "intro", "splash", "cover", "boxart", "logo"];
    const NEG: &[&str] = &[
        "background", "bg", "effect", "particle", "shine", "bar", "overlay",
        "sil", "light", "gradient", "credit", "under", "oculta",
    ];
    let mut best: Option<(i64, PathBuf)> = None;
    let mut fallback: Option<(u64, PathBuf)> = None; // largest PNG, any size
    for e in rd.flatten() {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()).map(|x| x.eq_ignore_ascii_case("png")) != Some(true) {
            continue;
        }
        let name = p.file_name().map(|s| s.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        let size = std::fs::metadata(&p).map_or(0, |m| m.len());
        if fallback.as_ref().map(|(s, _)| size > *s).unwrap_or(true) {
            fallback = Some((size, p.clone()));
        }
        // full-frame art only — skips wordmark logos, particle/effect sheets
        if png_dims(&p).map(|(w, h)| w.min(h) >= 200).unwrap_or(false) {
            let mut score: i64 = size as i64;
            if POS.iter().any(|k| name.contains(k)) {
                score += 3_000_000;
            }
            if NEG.iter().any(|k| name.contains(k)) {
                score -= 3_000_000;
            }
            if best.as_ref().map(|(s, _)| score > *s).unwrap_or(true) {
                best = Some((score, p));
            }
        }
    }
    let chosen = best.map(|(_, p)| p).or(fallback.map(|(_, p)| p));
    if let Some(src) = chosen {
        let md = media_dir(sd_root);
        std::fs::create_dir_all(&md).map_err(|e| format!("mkdir .media: {e}"))?;
        std::fs::copy(&src, md.join(format!("{safe}.png")))
            .map_err(|e| format!("copy boxart: {e}"))?;
    }
    Ok(())
}

/// Generic fallback mkxp.json — written ONLY for a game that doesn't ship
/// its own config. Sane defaults that let the game run on **vanilla** mkxp-z
/// (no engine hacks, no customScript): auto-detect the RGSS version,
/// fullscreen with a preserved aspect ratio, and `subImageFix` for the
/// PowerVR driver quirk where tiles/text sometimes fail to render. A game
/// this can't run belongs on a future legacy-RGSS backend, not a shim.
const MKXP_JSON: &str = r#"{
    "rgssVersion": 0,
    "fullscreen": true,
    "winResizable": false,
    "anyAltToggleFS": true,
    "fixedAspectRatio": true,
    "smoothScaling": 1,
    "vsync": false,
    "frameSkip": false,
    "subImageFix": true,
    "enableBlitting": false,
    "JITEnable": true,
    "JITMinCalls": 5000
}
"#;

// The legacy RMXP-compat loader (a customScript monkey-patch that forced
// pre-mkxp-z / Ruby-1.8-era games onto mkxp-z) is intentionally NOT used:
// Port Forge keeps the engine OG. It's preserved as reference at
// docs/legacy-rgss-loader.rb — the spec for a future legacy-RGSS backend.

/// A sane default gptk (mkxp-z reads the pad natively; this is the
/// keyboard fallback + hotkeys). Copied verbatim.
const GPTK: &str = "back = esc\nstart = enter\na = z\nb = x\nx = a\ny = s\nl1 = q\nr1 = w\nup = up\ndown = down\nleft = left\nright = right\n";

/// Build the launch `.sh` for slug `<slug>`, mounting the mkxp-z runtime,
/// binding stdlib, sharing one swapfile across all Port Forge games.
fn launch_script(slug: &str) -> String {
    format!(
        r#"#!/bin/bash
XDG_DATA_HOME=${{XDG_DATA_HOME:-$HOME/.local/share}}
if [ -d "/opt/system/Tools/PortMaster/" ]; then controlfolder="/opt/system/Tools/PortMaster"
elif [ -d "/opt/tools/PortMaster/" ]; then controlfolder="/opt/tools/PortMaster"
elif [ -d "$XDG_DATA_HOME/PortMaster/" ]; then controlfolder="$XDG_DATA_HOME/PortMaster"
else controlfolder="/roms/ports/PortMaster"; fi
source $controlfolder/control.txt
[ -f "${{controlfolder}}/mod_${{CFW_NAME}}.txt" ] && source "${{controlfolder}}/mod_${{CFW_NAME}}.txt"
get_controls
GAMEDIR="/$directory/ports/{slug}"
MKXPZ_RUNTIME="$controlfolder/libs/mkxp-z.squashfs"
MKXPZ="$HOME/mkxp-z"
cd "$GAMEDIR" || exit 1
> "$GAMEDIR/log.txt" && exec > >(tee "$GAMEDIR/log.txt") 2>&1
# A force-quit (kui_portrun kills the process group) skips normal cleanup,
# leaking the stdlib bind + squashfs mounts and hanging the NEXT launch.
# Trap our own exit for the graceful case, and defensively clear any leaked
# mounts BEFORE mounting (unmount the bind first so the squashfs frees).
pf_cleanup() {{
    $ESUDO umount "$GAMEDIR/stdlib" 2>/dev/null || $ESUDO umount -l "$GAMEDIR/stdlib" 2>/dev/null
    $ESUDO umount "$MKXPZ" 2>/dev/null || $ESUDO umount -l "$MKXPZ" 2>/dev/null
    [ -n "$SWAP" ] && $ESUDO swapoff "$SWAP" 2>/dev/null
}}
trap pf_cleanup EXIT INT TERM
$ESUDO umount "$GAMEDIR/stdlib" 2>/dev/null || $ESUDO umount -l "$GAMEDIR/stdlib" 2>/dev/null || true
if [ -f "$MKXPZ_RUNTIME" ]; then
    $ESUDO mkdir -p "$MKXPZ"
    $ESUDO umount "$MKXPZ" 2>/dev/null || $ESUDO umount -l "$MKXPZ" 2>/dev/null || true
    $ESUDO mount "$MKXPZ_RUNTIME" "$MKXPZ"
else
    pm_message "mkxp-z runtime missing."; sleep 5; pm_finish; exit 1
fi
mkdir -p "$GAMEDIR/config"
[ -L "$GAMEDIR/stdlib" ] && rm -f "$GAMEDIR/stdlib"
[ -e "$GAMEDIR/stdlib" ] && [ ! -L "$GAMEDIR/stdlib" ] && rm -rf "$GAMEDIR/stdlib" 2>/dev/null
bind_directories "$GAMEDIR/stdlib" "$MKXPZ/stdlib"
export SDL_GAMECONTROLLERCONFIG="$sdl_controllerconfig"
export XDG_DATA_HOME="$GAMEDIR/config"
export LC_ALL=C
export LANG=C
# Some RGSS games OOM-kill mkxp-z on low-RAM handhelds without swap.
# Find a writable non-FAT partition and share ONE 2GB swapfile across all
# Port Forge games. FAT/exFAT candidates fail mkswap/swapon and are skipped.
SWAP=""
for d in /mnt/UDISK /userdata /storage /mnt/mmc /roms2; do
    {{ [ -d "$d" ] && touch "$d/.pfsw" 2>/dev/null; }} || continue
    rm -f "$d/.pfsw"
    cand="$d/portforge.swap"
    if [ ! -f "$cand" ]; then
        $ESUDO dd if=/dev/zero of="$cand" bs=1M count=2048 2>/dev/null
        $ESUDO chmod 600 "$cand"
        $ESUDO mkswap "$cand" >/dev/null 2>&1 || {{ $ESUDO rm -f "$cand"; continue; }}
    fi
    $ESUDO swapon "$cand" 2>/dev/null && {{ SWAP="$cand"; break; }}
done
$GPTOKEYB "mkxp-z.aarch64" -c "$GAMEDIR/{slug}.gptk" &
export LD_LIBRARY_PATH="$MKXPZ/libs:$LD_LIBRARY_PATH"
export SRCDIR="$GAMEDIR"
pm_platform_helper "$MKXPZ/mkxp-z.aarch64" >/dev/null
"$MKXPZ/mkxp-z.aarch64"
pf_cleanup
pm_finish
"#
    )
}

/// Forge a port from `source` (a game folder). `progress(fraction, phase)`
/// reports a 0.0–1.0 bar fraction plus a phase label; the copy phase (the
/// slow part) drives the bar by bytes placed. Returns the display name.
pub fn forge(
    sd_root: &Path,
    source: &Path,
    progress: &mut dyn FnMut(f32, &str),
) -> Result<String, String> {
    if !runtime_present(sd_root) {
        return Err("mkxp-z runtime not installed (Data/PortMaster/libs/mkxp-z.squashfs)".into());
    }
    progress(0.0, "Finding the game…");
    let game_dir = resolve_game_dir(source)?;

    let ini = find_rmxp_ini(&game_dir)
        .ok_or_else(|| "no RPG Maker game (.ini) found".to_string())?;
    let ini_name = ini
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Game.ini".into());
    let title = read_title(&ini)?;
    let base_slug = slugify(&title);
    let safe = safe_name(&title, &base_slug);
    // Payload slug. Re-forging the same game (its launcher already exists)
    // reuses its slug and overwrites; a DIFFERENT game that slugifies to the
    // same name takes the next free <slug>N instead of clobbering it.
    let mut slug = base_slug.clone();
    if !scripts_dir(sd_root).join(format!("{safe}.sh")).is_file() {
        let mut n = 1u32;
        while data_ports_dir(sd_root).join(&slug).exists() {
            n += 1;
            slug = format!("{base_slug}{n}");
        }
    }
    progress(0.04, "Building the port…");

    let port_dir = data_ports_dir(sd_root).join(&slug);
    if port_dir.exists() {
        std::fs::remove_dir_all(&port_dir).map_err(|e| format!("clear old port: {e}"))?;
    }
    // Placing files is the long phase → drive the bar 0.05..0.92 by bytes.
    let total = place_total(&game_dir).max(1);
    let mut done: u64 = 0;
    let place_phase = "Copying game files…";
    progress(0.05, place_phase);
    place_game_data(&game_dir, &port_dir, &mut |b| {
        done = done.saturating_add(b);
        let frac = (done as f32 / total as f32).min(1.0);
        progress(0.05 + 0.87 * frac, place_phase);
    })?;
    // Engine stays OG. If the game ships its own mkxp.json (a modern RGSS
    // game configured for mkxp-z — its resolution, keybinds, patch system,
    // soundfont, execName), keep it and run natively, untouched. Only a game
    // WITHOUT one is normalized to Game.*, has its archive unpacked, and gets
    // a generic fallback config. No compat loader is ever imposed — a game
    // that can't run on vanilla mkxp-z is a job for a future legacy-RGSS
    // backend, not a shim bolted onto the modern engine.
    if !port_dir.join("mkxp.json").is_file() {
        // normalize <exe>.ini/.rgssad → Game.* so mkxp-z finds them
        normalize_engine_names(&port_dir, &ini_name);
        // Unpack an RGSS v1 archive to loose files: mkxp-z reads it for
        // graphics/load_data, but Ruby's FileTest.exist? (which some games use
        // to check maps) only sees the real FS, so a packed game fails those
        // checks. Delete the archive ONLY if files were actually written.
        progress(0.93, "Unpacking game archive…");
        let arc = port_dir.join("Game.rgssad");
        if arc.is_file()
            && let Ok(n) = extract_rgssad(&arc, &port_dir)
            && n > 0
        {
            let _ = std::fs::remove_file(&arc);
        }
        progress(0.94, "Writing the port…");
        std::fs::write(port_dir.join("mkxp.json"), MKXP_JSON).map_err(|e| format!("write mkxp.json: {e}"))?;
    }
    std::fs::write(port_dir.join(format!("{slug}.gptk")), GPTK).map_err(|e| format!("write gptk: {e}"))?;
    let sd = scripts_dir(sd_root);
    std::fs::create_dir_all(&sd).map_err(|e| format!("mkdir scripts: {e}"))?;
    std::fs::write(sd.join(format!("{safe}.sh")), launch_script(&slug))
        .map_err(|e| format!("write launch script: {e}"))?;

    // boxart from Graphics/Titles, read from the placed copy
    progress(0.97, "Fetching boxart…");
    let _ = place_boxart(&port_dir, sd_root, &safe);

    progress(1.0, "Done");
    Ok(title)
}

/// Move `src` onto `dest`: an atomic `rename` when they share a filesystem
/// (instant, no byte copy — the common case, since the package sits on the same
/// card), else a recursive copy + remove for a cross-mount package. `bump`
/// receives the moved byte count so the caller can drive a progress bar.
fn move_path(src: &Path, dest: &Path, bump: &mut dyn FnMut(u64)) -> Result<(), String> {
    if std::fs::rename(src, dest).is_ok() {
        bump(tree_size(dest));
        return Ok(());
    }
    if src.is_dir() {
        copy_dir(src, dest, bump)?;
    } else {
        std::fs::copy(src, dest).map_err(|e| format!("copy {}: {e}", src.display()))?;
        bump(std::fs::metadata(dest).map_or(0, |m| m.len()));
    }
    remove_any(src)
}

fn remove_any(p: &Path) -> Result<(), String> {
    let r = if p.is_dir() { std::fs::remove_dir_all(p) } else { std::fs::remove_file(p) };
    r.map_err(|e| format!("remove {}: {e}", p.display()))
}

/// Install a Port Forge Web package `pkg` (a folder with a `portforge.json`
/// manifest) by MOVING its pieces into their card homes — `Data/ports/<slug>`,
/// the launch `.sh` in `Roms/Ports (PORTS)`, boxart in `.media`, and any shared
/// deps it declares (e.g. an RTP). Shared deps already on the card are skipped
/// (deduped) and the package's copy discarded. Because it moves rather than
/// copies, the package is consumed — there is no leftover to ask about — and
/// placing the `.sh` IS registration, so the port then shows up in Ports.
/// `progress(fraction, phase)` drives the bar. Engine-agnostic by design.
pub fn install_package(
    sd_root: &Path,
    pkg: &Path,
    progress: &mut dyn FnMut(f32, &str),
) -> Result<String, String> {
    progress(0.0, "Reading package…");
    let text = std::fs::read_to_string(pkg.join("portforge.json"))
        .map_err(|e| format!("read manifest: {e}"))?;
    let j = Json::parse(&text).map_err(|e| format!("bad manifest: {e}"))?;
    if j.get("format").and_then(Json::as_str) != Some("portforge-port") {
        return Err("not a Port Forge package".into());
    }
    let slug = j.get("slug").and_then(Json::as_str).unwrap_or("").to_string();
    let script = j.get("script").and_then(Json::as_str).unwrap_or("").to_string();
    if slug.is_empty() || script.is_empty() {
        return Err("manifest missing slug/script".into());
    }
    let title = j.get("title").and_then(Json::as_str).unwrap_or(&slug).to_string();
    let boxart = j.get("boxart").and_then(Json::as_str).map(str::to_string);
    let shared: Vec<String> = j
        .get("shared")
        .and_then(Json::as_arr)
        .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
        .unwrap_or_default();

    // Install plan: (card-relative path, dedupe-if-already-present?). All paths
    // follow PortMaster convention, derived from the manifest — no engine
    // knowledge needed. The port dir is required; the rest may be absent.
    let mut plan: Vec<(PathBuf, bool)> = vec![(PathBuf::from("Data/ports").join(&slug), false)];
    for s in &shared {
        plan.push((PathBuf::from(s), true));
    }
    plan.push((PathBuf::from("Roms/Ports (PORTS)").join(&script), false));
    if let Some(b) = &boxart {
        plan.push((PathBuf::from("Roms/Ports (PORTS)/.media").join(b), false));
    }
    let port_rel = PathBuf::from("Data/ports").join(&slug);

    // total bytes we'll actually move (deduped shared deps don't count)
    let mut total = 0u64;
    for (rel, dedupe) in &plan {
        if *dedupe && sd_root.join(rel).exists() {
            continue;
        }
        total += tree_size(&pkg.join(rel));
    }
    let total = total.max(1);
    let mut done = 0u64;

    for (rel, dedupe) in &plan {
        let src = pkg.join(rel);
        let dest = sd_root.join(rel);
        if *dedupe && dest.exists() {
            let _ = remove_any(&src); // already installed → drop the package copy
            continue;
        }
        if !src.exists() {
            if *rel == port_rel {
                return Err(format!("package missing {}", rel.display()));
            }
            continue; // optional piece (boxart / a shared dep) not in this package
        }
        progress(0.05 + 0.87 * (done as f32 / total as f32).min(1.0), "Installing…");
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?;
        }
        if dest.exists() {
            let _ = remove_any(&dest); // replace on reinstall
        }
        move_path(&src, &dest, &mut |bp| {
            done = done.saturating_add(bp);
            progress(0.05 + 0.87 * (done as f32 / total as f32).min(1.0), "Installing…");
        })?;
    }

    // the package is consumed — clear its emptied shell (manifest + husk dirs)
    progress(0.97, "Cleaning up…");
    let _ = std::fs::remove_dir_all(pkg);
    progress(1.0, "Done");
    Ok(title)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_and_safe() {
        assert_eq!(slugify("Café Story 3"), "cafstory3");
        assert_eq!(slugify("!!!"), "game");
        assert_eq!(safe_name("HELP! NO BRAKE", "x"), "HELP! NO BRAKE");
        assert_eq!(safe_name("a/b:c", "x"), "a b c");
    }

    #[test]
    fn strip_rules() {
        assert!(is_stripped("Game.exe", false));
        assert!(is_stripped("RGSS104E.dll", false));
        assert!(is_stripped(".git", true));
        assert!(!is_stripped("Game.ini", false));
        assert!(!is_stripped("Data", true));
        assert!(!is_stripped("Graphics", true));
    }

    // Build a Port Forge Web package under `root` (game + shared RTP asset +
    // launch script + boxart + manifest). `rtp_byte` seeds the RTP asset so a
    // dedupe test can tell an overwrite from a skip.
    #[cfg(test)]
    fn make_package(root: &Path, rtp_byte: &str, with_boxart: bool) {
        use std::fs;
        let port = root.join("Data/ports/foo");
        fs::create_dir_all(&port).unwrap();
        fs::write(port.join("Game.ini"), "game").unwrap();
        let rtp = root.join("Data/ports/.rtp/RPGVXAce/Graphics");
        fs::create_dir_all(&rtp).unwrap();
        fs::write(rtp.join("Vehicle.png"), rtp_byte).unwrap();
        let scripts = root.join("Roms/Ports (PORTS)");
        fs::create_dir_all(scripts.join(".media")).unwrap();
        fs::write(scripts.join("Foo.sh"), "#!/bin/bash").unwrap();
        let boxart = if with_boxart {
            fs::write(scripts.join(".media/Foo.png"), "box").unwrap();
            "\"boxart\":\"Foo.png\","
        } else {
            ""
        };
        fs::write(
            root.join("portforge.json"),
            format!(
                r#"{{"format":"portforge-port","version":1,"title":"Foo","slug":"foo","script":"Foo.sh",{boxart}"shared":["Data/ports/.rtp/RPGVXAce"]}}"#
            ),
        )
        .unwrap();
    }

    #[test]
    fn install_moves_dedupes_and_consumes() {
        use std::fs;
        let base =
            std::env::temp_dir().join(format!("pf_install_{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let sd = base.join("sd");

        let pkg = base.join("pkg");
        make_package(&pkg, "first", true);
        assert!(is_port_package(&pkg));
        assert!(!is_port_package(&sd.join("nope")));

        let title = install_package(&sd, &pkg, &mut |_, _| {}).unwrap();
        assert_eq!(title, "Foo");
        assert!(sd.join("Data/ports/foo/Game.ini").is_file());
        assert!(sd.join("Data/ports/.rtp/RPGVXAce/Graphics/Vehicle.png").is_file());
        assert!(sd.join("Roms/Ports (PORTS)/Foo.sh").is_file(), "the .sh registers it");
        assert!(sd.join("Roms/Ports (PORTS)/.media/Foo.png").is_file());
        assert!(!pkg.exists(), "package is consumed, no leftover");

        // reinstall with the RTP already present → shared dep is deduped
        // (original kept, not overwritten), package still consumed
        let pkg2 = base.join("pkg2");
        make_package(&pkg2, "second", false);
        install_package(&sd, &pkg2, &mut |_, _| {}).unwrap();
        assert_eq!(
            fs::read_to_string(sd.join("Data/ports/.rtp/RPGVXAce/Graphics/Vehicle.png")).unwrap(),
            "first",
            "shared RTP deduped, not re-copied",
        );
        assert!(!pkg2.exists());

        let _ = fs::remove_dir_all(&base);
    }
}
