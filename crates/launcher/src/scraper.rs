//! Scraper engine — Rust port of workspace/all/scraper/scraper.c (kUI native C rewrite).
//!
//! Logic only: network (via /usr/bin/curl), name matching, and file writing.
//! No UI — runs on a background worker thread, polled via `Scraper::progress()`.
//!
//! Layout (relative to the SD root, i.e. the parent of the Roms dir):
//!   Artwork/.cache/matches/<TAG>.in.txt            ROM list POSTed to the matcher
//!   Artwork/.cache/matches/<TAG>.boxart.out.txt    cached "rom\turl" match list
//!   Artwork/.cache/metadata/<TAG>_<type>.dat       cached libretro-database .dat
//!   Artwork/<Platform>/boxart/<rom>.png            downloaded image cache
//!   Roms/<Platform>/.media/<base>.png              image copied for the launcher
//!   Roms/<Platform>/.media/<base>.info             year=/genre=/developer=/publisher=

#![allow(dead_code)]

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

const MATCHER_URL: &str = "https://matching-images-is.bittersweet.rip";
const LIBRETRO_DB_URL: &str =
    "https://raw.githubusercontent.com/libretro/libretro-database/master/metadat";
const PATCH_MAX_W: u32 = 390;
const PATCH_MAX_H: u32 = 396;
/// Internal sentinel: worker stopped by `cancel()`, not an error.
const CANCELLED: &str = "__cancelled__";

// =============================================
// Public API
// =============================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Job {
    /// Boxart + metadata, skipping anything already cached (C action 4/22).
    DownloadMissing,
    /// Boxart only (C action 7/25).
    ImagesOnly,
    /// .info files only (C action 8/26).
    MetadataOnly,
    /// Drop the cached match list, then boxart + metadata (C action 14/24).
    DownloadAll,
    /// Downscale .media PNGs to fit 390x396 (C action 5/23).
    PatchImages,
    /// Remove Artwork/<platform> and Roms/<platform>/.media (C action 6).
    DeleteArtwork,
}

#[derive(Clone, Debug, Default)]
pub struct Progress {
    /// User-visible status line, mirrors the C strings ("Scanning ROMs...", ...).
    pub phase: String,
    pub done: usize,
    pub total: usize,
    pub finished: bool,
    pub error: Option<String>,
}

pub struct Scraper {
    progress: Arc<Mutex<Progress>>,
    cancel: Arc<AtomicBool>,
    _handle: JoinHandle<()>,
}

impl Scraper {
    /// Spawn a worker for `job` over one platform dir, or every platform under
    /// `roms_root` when `platform_dir` is `None`. The Artwork cache lives next
    /// to the Roms dir (i.e. under `roms_root.parent()`).
    pub fn start(roms_root: &Path, platform_dir: Option<PathBuf>, job: Job) -> Scraper {
        let progress = Arc::new(Mutex::new(Progress::default()));
        let cancel = Arc::new(AtomicBool::new(false));
        let ctx = Ctx {
            roms_root: roms_root.to_path_buf(),
            sd_root: roms_root
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| roms_root.to_path_buf()),
            progress: progress.clone(),
            cancel: cancel.clone(),
        };
        let handle = std::thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run_job(&ctx, platform_dir.as_deref(), job)
            }));
            let mut p = ctx.progress.lock().unwrap();
            match result {
                Ok(Ok(())) => {}
                Ok(Err(e)) if e == CANCELLED => p.phase = "Cancelled".into(),
                Ok(Err(e)) => {
                    p.phase = format!("Error: {e}");
                    p.error = Some(e);
                }
                Err(_) => {
                    let e = "scraper worker panicked".to_string();
                    p.phase = format!("Error: {e}");
                    p.error = Some(e);
                }
            }
            p.finished = true;
        });
        Scraper { progress, cancel, _handle: handle }
    }

    /// Cheap snapshot of the current state.
    pub fn progress(&self) -> Progress {
        self.progress.lock().unwrap().clone()
    }

    /// Best-effort stop; the flag is checked between items.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

// =============================================
// Worker context
// =============================================

#[derive(Clone)]
struct Ctx {
    roms_root: PathBuf,
    sd_root: PathBuf,
    progress: Arc<Mutex<Progress>>,
    cancel: Arc<AtomicBool>,
}

impl Ctx {
    fn artwork_root(&self) -> PathBuf {
        self.sd_root.join("Artwork")
    }
    fn matches_cache(&self) -> PathBuf {
        self.artwork_root().join(".cache/matches")
    }
    fn metadata_cache(&self) -> PathBuf {
        self.artwork_root().join(".cache/metadata")
    }
    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
    fn check_cancel(&self) -> Result<(), String> {
        if self.cancelled() { Err(CANCELLED.into()) } else { Ok(()) }
    }
    fn set_phase(&self, phase: impl Into<String>) {
        let mut p = self.progress.lock().unwrap();
        p.phase = phase.into();
    }
    fn set_counts(&self, done: usize, total: usize) {
        let mut p = self.progress.lock().unwrap();
        p.done = done;
        p.total = total;
    }
}

// =============================================
// Job dispatch
// =============================================

fn run_job(ctx: &Ctx, platform_dir: Option<&Path>, job: Job) -> Result<(), String> {
    match platform_dir {
        Some(dir) => {
            let name = dir
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or_else(|| format!("bad platform dir: {}", dir.display()))?
                .to_string();
            let tag = extract_tag(&name);
            run_platform(ctx, &name, &tag, job)
        }
        None => {
            let platforms = scan_platforms(&ctx.roms_root);
            let count = platforms.len();
            for (i, (name, tag)) in platforms.iter().enumerate() {
                ctx.check_cancel()?;
                ctx.set_phase(format!("{}: {} ({}/{})...", all_label(job), name, i + 1, count));
                run_platform(ctx, name, tag, job)?;
            }
            ctx.set_phase("Done! All platforms processed.");
            Ok(())
        }
    }
}

fn all_label(job: Job) -> &'static str {
    match job {
        Job::DownloadMissing => "Download Missing (All)",
        Job::ImagesOnly => "Download Images Only (All)",
        Job::MetadataOnly => "Download Metadata Only (All)",
        Job::DownloadAll => "Download All (refresh, All)",
        Job::PatchImages => "Patch All Images",
        Job::DeleteArtwork => "Delete Artwork & Metadata (All)",
    }
}

fn run_platform(ctx: &Ctx, platform_name: &str, tag: &str, job: Job) -> Result<(), String> {
    match job {
        Job::DownloadMissing => {
            fetch_artwork(ctx, platform_name, tag, "boxart")?;
            fetch_metadata(ctx, platform_name, tag)
        }
        Job::ImagesOnly => fetch_artwork(ctx, platform_name, tag, "boxart"),
        Job::MetadataOnly => fetch_metadata(ctx, platform_name, tag),
        Job::DownloadAll => {
            // Drop the cached match list so the server is asked again.
            let _ = fs::remove_file(ctx.matches_cache().join(format!("{tag}.boxart.out.txt")));
            fetch_artwork(ctx, platform_name, tag, "boxart")?;
            fetch_metadata(ctx, platform_name, tag)
        }
        Job::PatchImages => patch_images(ctx, platform_name),
        Job::DeleteArtwork => delete_artwork(ctx, platform_name),
    }
}

// =============================================
// Platform scanning (C scan_platforms, minus the "All Platforms" pseudo-entry)
// =============================================

/// Directories under the Roms root that contain at least one non-hidden entry
/// whose name doesn't contain ".txt". Returns (dir_name, tag) sorted
/// case-insensitively.
fn scan_platforms(roms_root: &Path) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let Ok(entries) = fs::read_dir(roms_root) else { return out };
    for ent in entries.flatten() {
        let name = ent.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let path = ent.path();
        if !path.is_dir() {
            continue;
        }
        let Ok(rents) = fs::read_dir(&path) else { continue };
        let has_roms = rents.flatten().any(|r| {
            let n = r.file_name().to_string_lossy().into_owned();
            !n.starts_with('.') && !n.contains(".txt")
        });
        if !has_roms {
            continue;
        }
        let tag = extract_tag(&name);
        out.push((name, tag));
    }
    out.sort_by_key(|a| a.0.to_lowercase());
    out
}

/// Tag between the last '(' and the following ')' in a platform dir name,
/// e.g. "Game Boy (GB)" -> "GB". Empty if absent.
fn extract_tag(name: &str) -> String {
    if let Some(p) = name.rfind('(') {
        let rest = &name[p + 1..];
        if let Some(e) = rest.find(')') {
            return rest[..e].to_string();
        }
    }
    String::new()
}

// =============================================
// curl (device /usr/bin/curl, no HTTP crates)
// =============================================

fn curl() -> Command {
    let device = Path::new("/usr/bin/curl");
    let mut cmd = Command::new(if device.exists() { Path::new("/usr/bin/curl") } else { Path::new("curl") });
    // -f fail on HTTP errors, -k accept device's stale CA bundle, -sS quiet but
    // report errors, -L follow redirects (same flags the C used), plus timeouts.
    cmd.arg("-fksSL")
        .args(["--connect-timeout", "10", "--max-time", "120"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    cmd
}

/// GET `url` into `out`. On failure the (possibly empty/partial) output file is
/// removed so a failed download is never mistaken for a cached one.
fn curl_get(url: &str, out: &Path) -> bool {
    let ok = curl()
        .arg("-o")
        .arg(out)
        .arg(url)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !ok {
        let _ = fs::remove_file(out);
    }
    ok
}

/// POST `body_file` (text/plain) to `url`, response into `out`.
fn curl_post_file(url: &str, body_file: &Path, out: &Path) -> bool {
    let ok = curl()
        .args(["-X", "POST", "-H", "Content-Type: text/plain", "--data-binary"])
        .arg(format!("@{}", body_file.display()))
        .arg("-o")
        .arg(out)
        .arg(url)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !ok {
        let _ = fs::remove_file(out);
    }
    ok
}

// =============================================
// Artwork (C fetch_artwork)
// =============================================

fn fetch_artwork(ctx: &Ctx, platform_name: &str, tag: &str, art_type: &str) -> Result<(), String> {
    let rom_dir = ctx.roms_root.join(platform_name);
    let cache_dir = ctx.matches_cache();
    fs::create_dir_all(&cache_dir).map_err(|e| format!("mkdir {}: {e}", cache_dir.display()))?;

    let rom_list = cache_dir.join(format!("{tag}.in.txt"));
    let match_file = cache_dir.join(format!("{tag}.{art_type}.out.txt"));

    // Build the ROM list: every non-hidden entry name, one per line.
    ctx.set_phase("Scanning ROMs...");
    {
        let mut f = File::create(&rom_list).map_err(|e| format!("write {}: {e}", rom_list.display()))?;
        if let Ok(entries) = fs::read_dir(&rom_dir) {
            for ent in entries.flatten() {
                let name = ent.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') {
                    continue;
                }
                writeln!(f, "{name}").map_err(|e| e.to_string())?;
            }
        }
    }

    // Ask the matching server, unless a cached match list exists.
    // POST /matches/<tag>/<art_type> with the ROM list as text/plain; the
    // response is "rom_filename\timage_url" lines.
    if !match_file.exists() {
        ctx.set_phase("Fetching matches from server...");
        let url = format!("{MATCHER_URL}/matches/{tag}/{art_type}");
        if !curl_post_file(&url, &rom_list, &match_file) {
            return Err("Failed to fetch matches from server".into());
        }
    }

    let art_cache = ctx.artwork_root().join(platform_name).join(art_type);
    fs::create_dir_all(&art_cache).map_err(|e| format!("mkdir {}: {e}", art_cache.display()))?;

    let Ok(text) = fs::read_to_string(&match_file) else {
        ctx.set_phase("No matches found");
        return Ok(());
    };
    let entries: Vec<(&str, &str)> = text
        .lines()
        .map(str::trim_end)
        .filter(|l| l.len() >= 2)
        .filter_map(|l| l.split_once('\t'))
        .collect();
    let total = entries.len();
    ctx.set_counts(0, total);

    // Download each image into the Artwork cache (skip ones already there).
    let mut count = 0usize;
    for (rom_name, url) in &entries {
        ctx.check_cancel()?;
        let img_path = art_cache.join(format!("{rom_name}.png"));
        count += 1;
        ctx.set_counts(count, total);
        if img_path.exists() {
            continue;
        }
        if count.is_multiple_of(5) {
            ctx.set_phase(format!("Downloading {count}/{total}..."));
        }
        // Individual failures are non-fatal, as in the C.
        let _ = curl_get(url, &img_path);
    }

    // Copy into the platform's .media folder, named after the ROM base name.
    ctx.set_phase("Copying to .media...");
    let media_dir = rom_dir.join(".media");
    fs::create_dir_all(&media_dir).map_err(|e| format!("mkdir {}: {e}", media_dir.display()))?;
    for (rom_name, _) in &entries {
        let base = match rom_name.rfind('.') {
            Some(dot) => &rom_name[..dot],
            None => rom_name,
        };
        let src = art_cache.join(format!("{rom_name}.png"));
        let dst = media_dir.join(format!("{base}.png"));
        if src.exists() {
            let _ = fs::copy(&src, &dst);
        }
    }

    ctx.set_phase(format!("Done! {count} images"));
    Ok(())
}

// =============================================
// Metadata (C fetch_metadata) — libretro-database .dat files
// =============================================

const META_TYPES: [&str; 4] = ["genre", "developer", "publisher", "releaseyear"];

fn fetch_metadata(ctx: &Ctx, platform_name: &str, tag: &str) -> Result<(), String> {
    let Some(libretro_name) = libretro_system_name(tag) else {
        ctx.set_phase(format!("No metadata for {tag}"));
        return Ok(());
    };

    ctx.set_phase("Downloading metadata...");
    let cache_dir = ctx.metadata_cache();
    fs::create_dir_all(&cache_dir).map_err(|e| format!("mkdir {}: {e}", cache_dir.display()))?;

    // URL-encode the system name (spaces only, like the C).
    let encoded = libretro_name.replace(' ', "%20");

    for meta in META_TYPES {
        ctx.check_cancel()?;
        let dat_file = cache_dir.join(format!("{tag}_{meta}.dat"));
        if !dat_file.exists() {
            // Some systems lack some .dat files; failure is non-fatal.
            let _ = curl_get(&format!("{LIBRETRO_DB_URL}/{meta}/{encoded}.dat"), &dat_file);
        }
    }

    // Parse each .dat once into canonical-name -> value.
    let maps: Vec<HashMap<String, Option<String>>> = META_TYPES
        .iter()
        .map(|meta| parse_dat(&cache_dir.join(format!("{tag}_{meta}.dat")), meta))
        .collect();

    let rom_dir = ctx.roms_root.join(platform_name);
    let media_dir = rom_dir.join(".media");
    fs::create_dir_all(&media_dir).map_err(|e| format!("mkdir {}: {e}", media_dir.display()))?;

    let mut meta_count = 0usize;
    let mut total = 0usize;
    let Ok(entries) = fs::read_dir(&rom_dir) else { return Ok(()) };
    for ent in entries.flatten() {
        ctx.check_cancel()?;
        let name = ent.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || name.contains(".txt") {
            continue;
        }
        if !ent.path().is_file() {
            continue;
        }

        total += 1;
        let rom_base = match name.rfind('.') {
            Some(dot) => &name[..dot],
            None => name.as_str(),
        };
        let canon = canonicalize_name(rom_base);
        if canon.is_empty() {
            continue;
        }

        let lookup = |i: usize| maps[i].get(&canon).and_then(|v| v.clone());
        let genre = lookup(0);
        let developer = lookup(1);
        let publisher = lookup(2);
        let year = lookup(3);

        if year.is_some() || genre.is_some() || developer.is_some() || publisher.is_some() {
            let info_path = media_dir.join(format!("{rom_base}.info"));
            let mut info = String::new();
            if let Some(v) = &year {
                info.push_str(&format!("year={v}\n"));
            }
            if let Some(v) = &genre {
                info.push_str(&format!("genre={v}\n"));
            }
            if let Some(v) = &developer {
                info.push_str(&format!("developer={v}\n"));
            }
            if let Some(v) = &publisher {
                info.push_str(&format!("publisher={v}\n"));
            }
            if fs::write(&info_path, info).is_ok() {
                meta_count += 1;
            }
        }

        ctx.set_counts(meta_count, total);
        if total.is_multiple_of(10) {
            ctx.set_phase(format!("Processing {total} ROMs..."));
        }
    }

    ctx.set_phase(format!("Metadata: {meta_count}/{total} ROMs"));
    Ok(())
}

/// Parse a libretro metadat .dat into canonical-game-name -> field value.
///
/// Mirrors the C scan: a `comment "..."` line names the game; the first line
/// after it containing `keyword` supplies the value (first quoted string on
/// that line); a line starting with ')' ends the block. Only the FIRST block
/// for a given canonical name counts — later duplicates are ignored even if
/// the first block had no value, exactly like the C's early `break`.
fn parse_dat(path: &Path, keyword: &str) -> HashMap<String, Option<String>> {
    let mut map: HashMap<String, Option<String>> = HashMap::new();
    let Ok(file) = File::open(path) else { return map };
    let reader = BufReader::new(file);
    // Canonical name of the block we're currently collecting a value for.
    let mut current: Option<String> = None;
    for line in reader.lines() {
        let Ok(line) = line else { break };
        if let Some(pos) = line.find("comment \"") {
            let rest = &line[pos + "comment \"".len()..];
            if let Some(q2) = rest.find('"') {
                let canon = canonicalize_name(&rest[..q2]);
                if !canon.is_empty() {
                    if map.contains_key(&canon) {
                        current = None; // duplicate: first block already claimed it
                    } else {
                        map.insert(canon.clone(), None);
                        current = Some(canon);
                    }
                }
            }
        }
        // Value line (checked on the comment line too, matching the C's order).
        if current.is_some() && line.contains(keyword) {
            if let Some(q1) = line.find('"') {
                let rest = &line[q1 + 1..];
                if let Some(q2) = rest.find('"') {
                    let canon = current.as_ref().unwrap();
                    if let Some(slot) = map.get_mut(canon)
                        && slot.is_none() {
                            *slot = Some(rest[..q2].to_string());
                        }
                }
            }
            current = None; // C breaks out of the block here
        }
        // End of a game block.
        if line.trim_start_matches([' ', '\t']).starts_with(')') {
            current = None;
        }
    }
    map
}

// =============================================
// Name canonicalization (C canonicalize_name)
// =============================================

/// Lowercase, strip everything from the first '(' or '[' (region/revision
/// tags), drop punctuation that varies between filenames and DB entries,
/// collapse whitespace. "Tetris (USA, Europe) [!]" -> "tetris".
fn canonicalize_name(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut last_was_space = true;
    for c in input.chars() {
        match c {
            '(' | '[' => break,
            '.' | ',' | '\'' | '!' | '?' | '_' | '-' | ':' | ';' => {}
            ' ' | '\t' => {
                if !last_was_space {
                    out.push(' ');
                    last_was_space = true;
                }
            }
            _ => {
                out.push(c.to_ascii_lowercase());
                last_was_space = false;
            }
        }
    }
    while out.ends_with(' ') {
        out.pop();
    }
    out
}

// =============================================
// Tag -> libretro-database system name (ported verbatim from the C).
// Anything missing here = no metadata fetched. Tags with no realistic libretro
// coverage (hacks, ports, Pico-8, Arduboy, Uzebox) are intentionally omitted.
// =============================================

fn libretro_system_name(tag: &str) -> Option<&'static str> {
    const MAP: &[(&str, &str)] = &[
        // Nintendo
        ("GB", "Nintendo - Game Boy"),
        ("GBC", "Nintendo - Game Boy Color"),
        ("GBA", "Nintendo - Game Boy Advance"),
        ("FC", "Nintendo - Nintendo Entertainment System"),
        ("SFC", "Nintendo - Super Nintendo Entertainment System"),
        ("FDS", "Nintendo - Family Computer Disk System"),
        ("VB", "Nintendo - Virtual Boy"),
        ("PKM", "Nintendo - Pokemon Mini"),
        ("NDS", "Nintendo - Nintendo DS"),
        // Sega
        ("MD", "Sega - Mega Drive - Genesis"),
        ("SMS", "Sega - Master System - Mark III"),
        ("GG", "Sega - Game Gear"),
        ("32X", "Sega - 32X"),
        ("SEGACD", "Sega - Mega-CD - Sega CD"),
        ("SG1000", "Sega - SG-1000"),
        // Sony
        ("PS", "Sony - PlayStation"),
        // NEC
        ("PCE", "NEC - PC Engine - TurboGrafx 16"),
        ("SGFX", "NEC - PC Engine SuperGrafx"),
        ("PCECD", "NEC - PC Engine CD - TurboGrafx-CD"),
        // Atari
        ("LYNX", "Atari - Lynx"),
        ("A2600", "Atari - 2600"),
        ("A5200", "Atari - 5200"),
        ("A7800", "Atari - 7800"),
        ("A800", "Atari - 8-bit"),
        ("JAGUAR", "Atari - Jaguar"),
        ("JAGUARCD", "Atari - Jaguar"),
        // SNK
        ("NGP", "SNK - Neo Geo Pocket"),
        ("NGPC", "SNK - Neo Geo Pocket Color"),
        ("NEOCD", "SNK - Neo Geo CD"),
        // Bandai / GCE / Mattel / Magnavox
        ("WSC", "Bandai - WonderSwan Color"),
        ("WS", "Bandai - WonderSwan"),
        ("VECTREX", "GCE - Vectrex"),
        ("INTV", "Mattel - Intellivision"),
        ("O2", "Magnavox - Odyssey2"),
        // Watara / Welback / Fairchild
        ("SV", "Watara - Supervision"),
        ("MEGADUCK", "Welback - Mega Duck"),
        ("CHANF", "Fairchild - Channel F"),
        // Microsoft / Amstrad / Sinclair
        ("MSX", "Microsoft - MSX"),
        ("MSX2", "Microsoft - MSX2"),
        ("CPC", "Amstrad - CPC"),
        ("GX4000", "Amstrad - GX4000"),
        ("ZXS", "Sinclair - ZX Spectrum"),
        // Commodore — libretro lumps Amiga variants under one .dat
        ("C64", "Commodore - 64"),
        ("VIC", "Commodore - VIC-20"),
        ("PLUS4", "Commodore - Plus-4"),
        ("PET", "Commodore - PET"),
        ("C128", "Commodore - 128"),
        ("CD32", "Commodore - Amiga"),
        ("PUAE", "Commodore - Amiga"),
        // Coleco / 3DO
        ("COLECO", "Coleco - ColecoVision"),
        ("3DO", "The 3DO Company - 3DO"),
        // Arcade / DOS
        ("FBN", "FBNeo - Arcade Games"),
        ("DOS", "DOS"),
    ];
    MAP.iter()
        .find(|(t, _)| t.eq_ignore_ascii_case(tag))
        .map(|(_, name)| *name)
}

// =============================================
// Patch images (C patch_images) — downscale to fit 390x396
// =============================================

fn patch_images(ctx: &Ctx, platform_name: &str) -> Result<(), String> {
    let media_dir = ctx.roms_root.join(platform_name).join(".media");
    if !media_dir.exists() {
        ctx.set_phase("No images to patch");
        return Ok(());
    }

    ctx.set_phase("Patching images...");

    let mut files: Vec<PathBuf> = Vec::new();
    if let Ok(entries) = fs::read_dir(&media_dir) {
        for ent in entries.flatten() {
            let name = ent.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || name == "bg.png" || name == "bglist.png" {
                continue;
            }
            let is_png = name
                .rfind('.')
                .map(|d| name[d..].eq_ignore_ascii_case(".png"))
                .unwrap_or(false);
            if !is_png {
                continue;
            }
            files.push(ent.path());
        }
    }

    let grand_total = files.len();
    let mut patched = 0usize;
    let mut skipped = 0usize;
    let mut total = 0usize;

    for path in &files {
        ctx.check_cancel()?;
        total += 1;
        ctx.set_counts(total, grand_total);

        let Ok((w, h, rgba)) = kui_gfx::decode_png(path) else { continue };

        if w <= PATCH_MAX_W && h <= PATCH_MAX_H {
            skipped += 1;
            continue;
        }

        // Scale to fit within max dimensions (truncating, like the C).
        let scale = (PATCH_MAX_W as f32 / w as f32).min(PATCH_MAX_H as f32 / h as f32);
        let new_w = ((w as f32 * scale) as u32).max(1);
        let new_h = ((h as f32 * scale) as u32).max(1);

        let scaled = resize_nearest(&rgba, w, h, new_w, new_h);
        if kui_gfx::encode_png(path, new_w, new_h, &scaled).is_ok() {
            patched += 1;
        }

        if total.is_multiple_of(10) {
            ctx.set_phase(format!("Patching {total}..."));
        }
    }

    ctx.set_phase(format!("Patched {patched}, skipped {skipped} (of {total})"));
    Ok(())
}

/// Nearest-neighbor RGBA resample. Fine for downscaling boxart.
fn resize_nearest(src: &[u8], sw: u32, sh: u32, dw: u32, dh: u32) -> Vec<u8> {
    let mut dst = vec![0u8; dw as usize * dh as usize * 4];
    for y in 0..dh {
        let sy = (y as u64 * sh as u64 / dh as u64) as u32;
        for x in 0..dw {
            let sx = (x as u64 * sw as u64 / dw as u64) as u32;
            let si = ((sy * sw + sx) * 4) as usize;
            let di = ((y * dw + x) * 4) as usize;
            if si + 4 <= src.len() {
                dst[di..di + 4].copy_from_slice(&src[si..si + 4]);
            }
        }
    }
    dst
}

// =============================================
// Delete artwork & metadata (C delete_artwork)
// =============================================

fn delete_artwork(ctx: &Ctx, platform_name: &str) -> Result<(), String> {
    ctx.set_phase("Deleting artwork & metadata...");
    // .info files live inside .media, so removing it clears metadata too.
    let _ = fs::remove_dir_all(ctx.artwork_root().join(platform_name));
    let _ = fs::remove_dir_all(ctx.roms_root.join(platform_name).join(".media"));
    ctx.set_phase("Deleted");
    Ok(())
}

// =============================================
// Tests
// =============================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonicalize_strips_tags_and_punct() {
        assert_eq!(canonicalize_name("Tetris (USA, Europe) [!]"), "tetris");
        assert_eq!(canonicalize_name("Tetris (World) (Rev 1)"), "tetris");
        assert_eq!(
            canonicalize_name("Legend of Zelda, The - Link's Awakening"),
            "legend of zelda the links awakening"
        );
        assert_eq!(canonicalize_name("Dr. Mario"), "dr mario");
        assert_eq!(canonicalize_name("  Foo   Bar  "), "foo bar");
        assert_eq!(canonicalize_name("(proto)"), "");
    }

    #[test]
    fn tag_extraction() {
        assert_eq!(extract_tag("Game Boy (GB)"), "GB");
        assert_eq!(extract_tag("Weird (A) (GBC)"), "GBC");
        assert_eq!(extract_tag("No Tag"), "");
    }

    #[test]
    fn libretro_map_lookup() {
        assert_eq!(libretro_system_name("GB"), Some("Nintendo - Game Boy"));
        assert_eq!(libretro_system_name("gb"), Some("Nintendo - Game Boy"));
        assert_eq!(libretro_system_name("P8"), None); // Pico-8: intentionally excluded
    }

    #[test]
    fn resize_dims() {
        let src = vec![255u8; 4 * 4 * 4];
        let out = resize_nearest(&src, 4, 4, 2, 2);
        assert_eq!(out.len(), 2 * 2 * 4);
        assert!(out.iter().all(|&b| b == 255));
    }
}
