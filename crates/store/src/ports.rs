//! Native PortMaster-ecosystem client: ports.json catalog (fetched or
//! cached), a tier-1 device filter (aarch64, glibc 2.28, no analog sticks),
//! and install/remove of ports + their shared runtimes.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::http;
use crate::json::Json;
use crate::zip;

const PORTS_JSON_URL: &str =
    "https://github.com/PortsMaster/PortMaster-New/releases/latest/download/ports.json";
/// Device ceiling: the stock glibc is 2.28.
const MAX_GLIBC: (u64, u64) = (2, 28);

#[derive(Debug, Clone)]
pub struct PortEntry {
    pub zip_name: String,
    pub title: String,
    pub desc: String,
    pub inst: String,
    pub genres: Vec<String>,
    pub rtr: bool,
    pub runtimes: Vec<String>,
    pub size: u64,
    pub md5: String,
    pub url: String,
    pub gamedir: String,
    pub script: String,
}

#[derive(Debug, Clone)]
pub struct RuntimeEntry {
    pub name: String,
    pub md5: String,
    pub size: u64,
    pub url: String,
}

#[derive(Debug, Clone, Default)]
pub struct Catalog {
    pub ports: Vec<PortEntry>,
    pub hidden: usize,
    pub runtimes: Vec<RuntimeEntry>,
}

fn scripts_dir(sd_root: &Path) -> PathBuf {
    sd_root.join("Roms/Ports (PORTS)")
}

fn data_ports_dir(sd_root: &Path) -> PathBuf {
    sd_root.join("Data/ports")
}

fn libs_dir(sd_root: &Path) -> PathBuf {
    sd_root.join("Data/PortMaster/libs")
}

fn kui_shared_dir(sd_root: &Path) -> PathBuf {
    sd_root.join(".userdata/shared/kui")
}

fn cache_path(sd_root: &Path) -> PathBuf {
    kui_shared_dir(sd_root).join("ports.json")
}

/// Box art path for a port: `.media/<script stem>.png` next to the scripts.
fn media_path(sd_root: &Path, script: &str) -> PathBuf {
    let stem = script.strip_suffix(".sh").unwrap_or(script);
    scripts_dir(sd_root).join(".media").join(format!("{stem}.png"))
}

pub fn catalog(sd_root: &Path) -> Result<Catalog, String> {
    let cache = cache_path(sd_root);
    let fetch_err = match http::fetch_text(PORTS_JSON_URL, &[]) {
        Ok(body) => match parse_catalog(&body, sd_root) {
            Ok(cat) => {
                if let Some(parent) = cache.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                let _ = fs::write(&cache, &body);
                return Ok(cat);
            }
            Err(e) => e,
        },
        Err(e) => e,
    };
    let body = fs::read_to_string(&cache)
        .map_err(|e| format!("{fetch_err}; no usable cache ({}: {e})", cache.display()))?;
    parse_catalog(&body, sd_root).map_err(|e| format!("{fetch_err}; cached copy is bad: {e}"))
}

fn parse_catalog(body: &str, sd_root: &Path) -> Result<Catalog, String> {
    let root = Json::parse(body)?;
    let runtimes = parse_runtimes(&root)?;
    let available: Vec<&str> = runtimes.iter().map(|r| r.name.as_str()).collect();

    let ports_obj = match root.get("ports") {
        Some(Json::Obj(fields)) => fields,
        _ => return Err("ports.json: missing \"ports\" object".into()),
    };
    let mut ports = Vec::new();
    let mut hidden = 0usize;
    for (zip_name, p) in ports_obj {
        match parse_port(zip_name, p, &available) {
            // an installed port stays listed even when filtered — the
            // user must always be able to see and remove it
            Some((entry, visible)) => {
                if visible || installed(sd_root, &entry) {
                    ports.push(entry);
                } else {
                    hidden += 1;
                }
            }
            None => hidden += 1,
        }
    }
    ports.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));
    Ok(Catalog { ports, hidden, runtimes })
}

/// Tier-2 curation: ports proven broken on this device pending real
/// fixes (wrong resolution, dead controls, engine quirks). Grown by
/// on-device playtesting.
const BLOCKED: &[&str] = &[
    "cave.story-evo.zip", // ignores gamepad + renders at wrong resolution
    "maxpayne.zip",       // setup requires rsync (not on device)
    // Full-catalog on-device audit (2026-08-13): ready-to-run ports that
    // crash at launch with a clear failure in the log (segfault / missing
    // library / undefined symbol / fatal). Needs-data ports that only
    // failed for lack of data are deliberately NOT here.
    "armagetronad.zip",
    "billyfrontier.zip",
    "biolab.zip",
    "blupimania.zip",
    "cromagrally.zip",
    "daserbe.zip",
    "doukutsu-rs.zip",
    "enigma.zip",
    "freedroid.zip",
    "frozen-bubble.zip",
    "loguploader.zip",
    "lxdredhoney.zip",
    "meandmyshadow.zip",
    "moonlight.zip",
    "nanosaur2.zip",
    "ottomatic.zip",
    "plaqueattackremake.zip",
    "rlone.zip",
    "srb2.zip",
    "srb2kart.zip",
    "supertransball2.zip",
    "supertuxkart.zip",
    "tiny-crate.zip",
    "vitasnake.zip",
    "xmoto.zip",
    // Visual-pass over the full-catalog audit screenshots (2026-08-13):
    // ready-to-run ports that never rendered — the frame is the kUI
    // launcher (game drew nothing) or plain black. Confirmed by eye,
    // matching Arjun's own reports (yatka / pyxeljack / shackle).
    "anarch.zip",
    "bitriot.zip",
    "blockout2.zip",
    "breakhack.zip",
    "candycrisis.zip",
    "colorlines.zip",
    "crabjuice.zip",
    "crystalien.zip",
    "cutefamehalloweenbash.zip",
    "daserbe2.zip",
    "deathchase3d.zip",
    "dodgindiamonds2d.zip",
    "doomengines.zip",
    "entropipes.zip",
    "fishfillets.zip",
    "formlessstar.zip",
    "halloween3d.zip",
    "hypercycles.zip",
    "icytower.zip",
    "justkissheralready.zip",
    "justkisshimalready.zip",
    "koreader.zip",
    "littlerunmo.zip",
    "magerecall.zip",
    "matchstickelegy.zip",
    "meanderland.zip",
    "megaball.zip",
    "mightymike.zip",
    "minetest.zip",
    "mreader.zip",
    "netsurf.zip",
    "nothing.zip",
    "npuzzle.zip",
    "openclaw.zip",
    "overwrite.zip",
    "passage.zip",
    "pekka-kana-2.zip",
    "plantsvszombiesnd.zip",
    "pyxeljack.zip",
    "restore.portmaster.zip",
    "shackle.zip",
    "shackolantern.zip",
    "snapdragon.zip",
    "sprucechat.zip",
    "stingyseating.zip",
    "swordofjade.zip",
    "syasokoban.zip",
    "tenjutsu48h.zip",
    "thesaloon.zip",
    "ticoban.zip",
    "triggerrally.zip",
    "vampiregarden.zip",
    "vikingsofmidgard.zip",
    "wetspot2.zip",
    "whichsausagemate.zip",
    "widelands.zip",
    "xenofightersr.zip",
    "xga.zip",
    "yatka.zip",
    // On-device playtest after the full ready-to-play fill (2026-08-13).
    // Unplayable on the Hammer, confirmed by Arjun, no global fix:
    "pingus.zip",                     // no controls
    "lugaruhd.zip",                   // no controls
    "thesphinxoftime.zip",            // mouse works but can't start a game
    "maxdownforce.zip",               // steering needs an analog stick
    "baconthulhu.zip",                // audio, no video
    "digger.zip",                     // audio, no video
    "inertiablast.zip",               // sound, no video
    "dokimon.zip",                    // auto-closes
    "manicminer.zip",                 // colored borders, no game
    "alephone-marathon.zip",          // won't open
    "alephone-marathoninfinity.zip",  // won't open
    "moonlightnew.zip",               // streaming client, needs a host
    "multris.zip",                    // wrong resolution
];

/// The aarch64 runtimes from `utils`. Bare runtime names are the aarch64
/// builds; `.armhf.squashfs` / `.x86_64.squashfs` variants carry a
/// `runtime_arch` naming their arch. Image/gameinfo bundles are not runtimes.
fn parse_runtimes(root: &Json) -> Result<Vec<RuntimeEntry>, String> {
    let utils = match root.get("utils") {
        Some(Json::Obj(fields)) => fields,
        _ => return Err("ports.json: missing \"utils\" object".into()),
    };
    let mut out = Vec::new();
    for (name, u) in utils {
        if !name.ends_with(".squashfs") || name.starts_with("images.") || name == "gameinfo.zip" {
            continue;
        }
        let aarch64 = match u.get("runtime_arch").and_then(Json::as_str) {
            Some(arch) => arch == "aarch64",
            None => !name.ends_with(".armhf.squashfs") && !name.ends_with(".x86_64.squashfs"),
        };
        if !aarch64 {
            continue;
        }
        out.push(RuntimeEntry {
            name: name.clone(),
            md5: u.get("md5").and_then(Json::as_str).unwrap_or("").to_string(),
            size: u.get("size").and_then(Json::as_f64).unwrap_or(0.0) as u64,
            url: u.get("url").and_then(Json::as_str).unwrap_or("").to_string(),
        });
    }
    Ok(out)
}

/// Parse one catalog port; `None` = an unusable items list (nothing to
/// install); `Some((entry, false))` = parseable but hidden by the device
/// filter or curation.
fn parse_port(
    zip_name: &str,
    p: &Json,
    available_runtimes: &[&str],
) -> Option<(PortEntry, bool)> {
    let attr = p.get("attr")?;
    let s = |v: Option<&Json>| v.and_then(Json::as_str).unwrap_or("").to_string();
    let strings = |v: Option<&Json>| -> Vec<String> {
        v.and_then(Json::as_arr)
            .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
            .unwrap_or_default()
    };

    let arch = strings(attr.get("arch"));
    let runtimes = strings(attr.get("runtime"));
    let reqs = strings(attr.get("reqs"));
    let visible = (arch.is_empty() || arch.iter().any(|a| a == "aarch64"))
        && !glibc_exceeds(attr.get("min_glibc").and_then(Json::as_str).unwrap_or(""), MAX_GLIBC)
        && runtimes.iter().all(|r| available_runtimes.contains(&r.as_str()))
        // westonpack (Wayland shim) is not supported on this device yet
        && !runtimes.iter().any(|r| r.starts_with("weston_pkg"))
        && !reqs.iter().any(|r| req_unmeetable(r))
        && !BLOCKED.contains(&zip_name);

    // items = the zip's top level: one launch .sh plus the game directory.
    // Anything else (multi-script packs like darkplaces.zip, malformed
    // entries) is skipped.
    let items = strings(p.get("items"));
    let scripts: Vec<&String> =
        items.iter().filter(|i| i.ends_with(".sh") && !i.contains('/')).collect();
    let dirs: Vec<&String> = items.iter().filter(|i| !i.ends_with(".sh")).collect();
    if scripts.len() != 1 || dirs.is_empty() {
        return None;
    }
    let script = scripts[0].clone();
    let gamedir = dirs[0].trim_end_matches('/').to_string();
    if gamedir.is_empty() {
        return None;
    }

    let source = p.get("source")?;
    Some((PortEntry {
        zip_name: zip_name.to_string(),
        title: s(attr.get("title")),
        desc: s(attr.get("desc")),
        inst: s(attr.get("inst")),
        genres: strings(attr.get("genres")),
        rtr: attr.get("rtr").and_then(Json::as_bool).unwrap_or(false),
        runtimes,
        size: source.get("size").and_then(Json::as_f64).unwrap_or(0.0) as u64,
        md5: s(source.get("md5")),
        url: s(source.get("url")),
        gamedir,
        script,
    }, visible))
}

/// `min_glibc` like "2.34" compared against the device ceiling. Empty or
/// unparseable strings pass (the field is best-effort upstream).
fn glibc_exceeds(min_glibc: &str, max: (u64, u64)) -> bool {
    let s = min_glibc.trim();
    if s.is_empty() {
        return false;
    }
    let mut it = s.split('.');
    let major = it.next().and_then(|x| x.parse::<u64>().ok());
    let minor = it.next().and_then(|x| x.parse::<u64>().ok()).unwrap_or(0);
    match major {
        Some(major) => (major, minor) > max,
        None => false,
    }
}

/// A req in `attr.reqs` is unmeetable when every '|'-alternative fails on
/// this device. Observed values in the live ports.json (2026-08-11):
/// analog_1/analog_2 combos (40 ports — no sticks here), "opengl" (desktop
/// GL, none on the GE8300), "!trimui" (explicitly excluded on TrimUI
/// hardware), "2gb"/"4gb" RAM floors (device has 1gb), "ultra" (top-tier
/// SoCs). Meetable and ignored: "power", "!lowres", "4:3", "aarch64",
/// other CFW negations, and anything unknown (permissive by default).
fn req_unmeetable(req: &str) -> bool {
    fn alt_fails(alt: &str) -> bool {
        alt.starts_with("analog")
            || alt == "opengl"
            || alt == "!trimui"
            || alt == "ultra"
            || (alt.ends_with("gb")
                && alt
                    .trim_end_matches("gb")
                    .parse::<u64>()
                    .is_ok_and(|gb| gb > 1))
    }
    !req.is_empty() && req.split('|').all(alt_fails)
}

pub fn installed(sd_root: &Path, port: &PortEntry) -> bool {
    scripts_dir(sd_root).join(&port.script).exists()
}

pub fn runtime_installed(sd_root: &Path, name: &str) -> bool {
    libs_dir(sd_root).join(name).exists()
}

fn md5_file(path: &Path) -> Result<String, String> {
    let out = Command::new("md5sum")
        .arg(path)
        .output()
        .map_err(|e| format!("failed to run md5sum: {e}"))?;
    if !out.status.success() {
        return Err(format!("md5sum failed ({}) for {}", out.status, path.display()));
    }
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .next()
        .map(str::to_string)
        .ok_or_else(|| format!("md5sum produced no output for {}", path.display()))
}

fn verify_md5(path: &Path, expected: &str) -> Result<(), String> {
    if expected.is_empty() {
        return Ok(()); // no checksum published: nothing to verify against
    }
    let got = md5_file(path)?;
    if got.eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        Err(format!("md5 mismatch for {}: expected {expected}, got {got}", path.display()))
    }
}

pub fn install_port(
    sd_root: &Path,
    port: &PortEntry,
    catalog: &Catalog,
    progress: &mut dyn FnMut(String),
) -> Result<(), String> {
    let work = kui_shared_dir(sd_root);
    fs::create_dir_all(&work).map_err(|e| format!("mkdir {}: {e}", work.display()))?;

    // Preflight: the zip and its unpacked tree coexist during staging.
    // 3x the archive plus slack is a safe ceiling; failing here beats a
    // cryptic death mid-unpack on a full card.
    let missing_rt: u64 = port
        .runtimes
        .iter()
        .filter(|n| !runtime_installed(sd_root, n))
        .filter_map(|n| catalog.runtimes.iter().find(|r| &r.name == n))
        .map(|r| r.size)
        .sum();
    let needed = port.size * 3 + missing_rt + 64 * 1024 * 1024;
    if let Some(free) = free_bytes(sd_root)
        && free < needed
    {
        return Err(format!(
            "Not enough space: need ~{}, card has {} free",
            human_size(needed),
            human_size(free)
        ));
    }

    for name in &port.runtimes {
        if runtime_installed(sd_root, name) {
            continue;
        }
        let rt = catalog
            .runtimes
            .iter()
            .find(|r| &r.name == name)
            .ok_or_else(|| format!("runtime {name} not in catalog"))?;
        let tmp = work.join(name);
        download_with_progress(
            &rt.url,
            &tmp,
            rt.size,
            &format!("runtime {name}"),
            progress,
        )?;
        let checked = verify_md5(&tmp, &rt.md5);
        if let Err(e) = checked {
            let _ = fs::remove_file(&tmp);
            return Err(e);
        }
        let libs = libs_dir(sd_root);
        fs::create_dir_all(&libs).map_err(|e| format!("mkdir {}: {e}", libs.display()))?;
        let dest = libs.join(name);
        fs::rename(&tmp, &dest)
            .map_err(|e| format!("rename {} -> {}: {e}", tmp.display(), dest.display()))?;
    }

    let tmp_zip = work.join(&port.zip_name);
    download_with_progress(&port.url, &tmp_zip, port.size, &port.title, progress)?;
    let result = unpack_port(sd_root, port, &tmp_zip, progress);
    let _ = fs::remove_file(&tmp_zip);
    result?;
    progress("Done".to_string());
    Ok(())
}

/// Free bytes on the filesystem holding `path` (`df -kP`, busybox and
/// desktop compatible). None when df is unavailable/unparseable.
fn free_bytes(path: &Path) -> Option<u64> {
    let out = Command::new("df")
        .arg("-kP")
        .arg(path)
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let avail_kb: u64 = text.lines().last()?.split_whitespace().nth(3)?.parse().ok()?;
    Some(avail_kb * 1024)
}

/// Spawn curl and watch the .part file grow, reporting percent. No
/// wall-clock limit (ports run to gigabytes); curl aborts on stall.
fn download_with_progress(
    url: &str,
    dest: &Path,
    total: u64,
    label: &str,
    progress: &mut dyn FnMut(String),
) -> Result<(), String> {
    progress(format!("Downloading {label} ({})...", human_size(total)));
    let tmp = PathBuf::from(format!("{}.part", dest.display()));
    let _ = fs::remove_file(&tmp);
    let mut child = http::download_cmd(url, dest, &[])
        .spawn()
        .map_err(|e| format!("failed to run curl: {e}"))?;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    let _ = fs::remove_file(&tmp);
                    return Err(format!("download failed ({status}) for {url}"));
                }
                break;
            }
            Ok(None) => {
                if total > 0 {
                    let got = fs::metadata(&tmp).map(|m| m.len()).unwrap_or(0);
                    progress(format!(
                        "Downloading {label} ({})... {}%",
                        human_size(total),
                        got * 100 / total
                    ));
                }
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
            Err(e) => {
                let _ = child.kill();
                return Err(format!("curl wait: {e}"));
            }
        }
    }
    fs::rename(&tmp, dest)
        .map_err(|e| format!("rename {} -> {}: {e}", tmp.display(), dest.display()))
}

fn unpack_port(
    sd_root: &Path,
    port: &PortEntry,
    tmp_zip: &Path,
    progress: &mut dyn FnMut(String),
) -> Result<(), String> {
    verify_md5(tmp_zip, &port.md5)?;

    // Stage through the safe unzip, then move the two known top-level
    // pieces into place: the game dir under Data/ports/, the script next
    // to the roms.
    progress(format!("Installing {}...", port.title));
    let stage = kui_shared_dir(sd_root).join(".port-stage");
    let _ = fs::remove_dir_all(&stage);
    let unpacked = zip::unzip(tmp_zip, &stage);
    let result = unpacked.and_then(|()| place_staged(sd_root, port, &stage));
    let _ = fs::remove_dir_all(&stage);
    result
}

fn place_staged(sd_root: &Path, port: &PortEntry, stage: &Path) -> Result<(), String> {
    let staged_dir = stage.join(&port.gamedir);
    if !staged_dir.is_dir() {
        return Err(format!("zip has no {}/ directory", port.gamedir));
    }
    // Catalog script names drift from the archives (case/spacing/typos —
    // 13 ports in the 2026-08 snapshot). The zip is the truth: when the
    // declared name is missing, take the archive's sole top-level .sh and
    // normalize it to the catalog name on install.
    let mut staged_script = stage.join(&port.script);
    if !staged_script.is_file() {
        let mut shs: Vec<PathBuf> = fs::read_dir(stage)
            .map_err(|e| format!("read stage: {e}"))?
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.is_file() && p.extension().is_some_and(|x| x.eq_ignore_ascii_case("sh"))
            })
            .collect();
        if shs.len() != 1 {
            return Err(format!(
                "zip has no {} script ({} candidate scripts)",
                port.script,
                shs.len()
            ));
        }
        staged_script = shs.remove(0);
    }

    let data_dir = data_ports_dir(sd_root);
    fs::create_dir_all(&data_dir).map_err(|e| format!("mkdir {}: {e}", data_dir.display()))?;
    let game_dest = data_dir.join(&port.gamedir);
    if game_dest.exists() {
        fs::remove_dir_all(&game_dest)
            .map_err(|e| format!("clear {}: {e}", game_dest.display()))?;
    }
    fs::rename(&staged_dir, &game_dest)
        .map_err(|e| format!("rename {} -> {}: {e}", staged_dir.display(), game_dest.display()))?;

    let scripts = scripts_dir(sd_root);
    fs::create_dir_all(&scripts).map_err(|e| format!("mkdir {}: {e}", scripts.display()))?;
    let script_dest = scripts.join(&port.script);
    fs::rename(&staged_script, &script_dest).map_err(|e| {
        format!("rename {} -> {}: {e}", staged_script.display(), script_dest.display())
    })?;

    let cover = game_dest.join("cover.png");
    if cover.is_file() {
        let art = media_path(sd_root, &port.script);
        if let Some(parent) = art.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
        }
        fs::copy(&cover, &art)
            .map_err(|e| format!("copy {} -> {}: {e}", cover.display(), art.display()))?;
    }
    Ok(())
}

/// Remove a port's script, payload dir, and box art. Shared runtimes stay
/// (other ports may use them). rm -rf semantics.
pub fn remove_port(sd_root: &Path, port: &PortEntry) -> Result<(), String> {
    remove_port_files(sd_root, &port.script, &port.gamedir)
}

/// The actual removal, by script name + payload dir. Shared by the store
/// Uninstall (which has a catalog PortEntry) and the game-list Wipe (which
/// has only the installed script — see [`uninstall_script`]).
fn remove_port_files(sd_root: &Path, script: &str, gamedir: &str) -> Result<(), String> {
    let script_path = scripts_dir(sd_root).join(script);
    if script_path.exists() {
        fs::remove_file(&script_path)
            .map_err(|e| format!("remove {}: {e}", script_path.display()))?;
    }
    if !gamedir.is_empty() {
        let dir = data_ports_dir(sd_root).join(gamedir);
        if dir.exists() {
            fs::remove_dir_all(&dir).map_err(|e| format!("remove {}: {e}", dir.display()))?;
        }
    }
    let _ = fs::remove_file(media_path(sd_root, script));
    Ok(())
}

/// Uninstall an installed port given ONLY its script path — the game-list
/// Wipe has no catalog entry. The payload dir is read from the script
/// (PortMaster's convention is `GAMEDIR="…/<gamedir>"`, so the last path
/// component of the GAMEDIR assignment is the dir under Data/ports), then
/// the exact same removal as [`remove_port`] runs. If the gamedir can't be
/// parsed the script + box art still go; only the payload might linger.
pub fn uninstall_script(sd_root: &Path, script_path: &Path) -> Result<(), String> {
    let script = script_path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| format!("not a script path: {}", script_path.display()))?
        .to_string();
    let gamedir = gamedir_of(script_path).unwrap_or_default();
    remove_port_files(sd_root, &script, &gamedir)
}

/// Parse the payload dir name from a port script's `GAMEDIR=` line. Takes
/// the first assignment and its last path component, so both direct forms
/// (`GAMEDIR=/$directory/ports/pingus`) and indirected ones
/// (`GAMEDIR="$PORTDIR/multris"`) resolve to `pingus` / `multris`.
fn gamedir_of(script_path: &Path) -> Option<String> {
    let text = fs::read_to_string(script_path).ok()?;
    for line in text.lines() {
        if let Some(rest) = line.trim().strip_prefix("GAMEDIR=") {
            let val = rest.trim().trim_matches(['"', '\'']);
            // trim a trailing slash first: `.../ports/PigeonAscent/` would
            // otherwise rsplit to an empty last component and lose the dir.
            let name =
                val.trim_end_matches('/').rsplit('/').next().unwrap_or("").trim_matches(['"', '\'']);
            if !name.is_empty() && !name.contains('$') {
                return Some(name.to_string());
            }
        }
    }
    None
}

fn human_size(bytes: u64) -> String {
    if bytes >= 1024 * 1024 * 1024 {
        format!("{:.1} GiB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    } else if bytes >= 1024 * 1024 {
        format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{} KiB", bytes.div_ceil(1024))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kui-ports-test-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A port object with the given attr/items fragments spliced in. The
    /// extras come first because `Json::get` returns the first match.
    fn port_json(attr_extra: &str, items: &str) -> String {
        format!(
            r#"{{
              "items": {items},
              "attr": {{
                {attr_extra}
                "title": "T", "desc": "D", "inst": "I", "genres": ["g"],
                "rtr": false, "runtime": [], "reqs": [], "arch": [],
                "min_glibc": ""
              }},
              "source": {{ "md5": "aa", "size": 100, "url": "https://x/p.zip" }}
            }}"#
        )
    }

    const UTILS: &str = r#"{
        "gameinfo.zip": { "md5": "g", "size": 1, "url": "https://x/gameinfo.zip" },
        "images.000.zip": { "md5": "i", "size": 1, "url": "https://x/images.000.zip" },
        "mono-6.12.0.122-aarch64.squashfs":
          { "runtime_name": "mono-6.12.0.122-aarch64.squashfs", "runtime_arch": "aarch64",
            "md5": "m1", "size": 262144000, "url": "https://x/mono.squashfs" },
        "godot_4.3.squashfs":
          { "runtime_name": "godot_4.3.squashfs", "runtime_arch": "aarch64",
            "md5": "m2", "size": 5, "url": "https://x/godot.squashfs" },
        "godot_4.3.armhf.squashfs":
          { "runtime_name": "godot_4.3.squashfs", "runtime_arch": "armhf",
            "md5": "m3", "size": 5, "url": "https://x/godot-armhf.squashfs" },
        "godot_4.3.x86_64.squashfs":
          { "runtime_name": "godot_4.3.squashfs", "runtime_arch": "x86_64",
            "md5": "m4", "size": 5, "url": "https://x/godot-x86.squashfs" }
    }"#;

    fn catalog_with(ports_body: &str) -> Catalog {
        parse_catalog(&format!(r#"{{ "ports": {{ {ports_body} }}, "utils": {UTILS} }}"#), Path::new("/nonexistent")).unwrap()
    }

    #[test]
    fn parses_ports_and_runtimes() {
        let zelda = port_json(
            r#""runtime": ["godot_4.3.squashfs"], "arch": ["aarch64"], "rtr": true,"#,
            r#"["Zelda.sh", "zelda/"]"#,
        );
        let apple = port_json("", r#"["Apple.sh", "apple"]"#);
        let cat = catalog_with(&format!(
            r#""zelda.zip": {zelda}, "apple.zip": {apple}"#
        ));
        assert_eq!(cat.hidden, 0);
        assert_eq!(cat.ports.len(), 2);
        // runtimes: aarch64 squashfs only, no images/gameinfo, no other arches
        let names: Vec<&str> = cat.runtimes.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["mono-6.12.0.122-aarch64.squashfs", "godot_4.3.squashfs"]);
        assert_eq!(cat.runtimes[0].size, 262144000);
        assert_eq!(cat.runtimes[0].md5, "m1");
        let p = cat.ports.iter().find(|p| p.zip_name == "zelda.zip").unwrap();
        assert_eq!(p.title, "T");
        assert_eq!(p.desc, "D");
        assert_eq!(p.inst, "I");
        assert_eq!(p.genres, ["g"]);
        assert!(p.rtr);
        assert_eq!(p.runtimes, ["godot_4.3.squashfs"]);
        assert_eq!(p.size, 100);
        assert_eq!(p.md5, "aa");
        assert_eq!(p.url, "https://x/p.zip");
        assert_eq!(p.gamedir, "zelda"); // trailing '/' stripped
        assert_eq!(p.script, "Zelda.sh");
        let a = cat.ports.iter().find(|p| p.zip_name == "apple.zip").unwrap();
        assert_eq!(a.gamedir, "apple"); // bare dir entry also fine
    }

    #[test]
    fn sorts_by_title_case_insensitive() {
        let mk = |title: &str, script: &str| {
            format!(
                r#"{{
                  "items": ["{script}", "d/"],
                  "attr": {{ "title": "{title}" }},
                  "source": {{ "md5": "", "size": 0, "url": "" }}
                }}"#
            )
        };
        let cat = catalog_with(&format!(
            r#""b.zip": {}, "a.zip": {}, "c.zip": {}"#,
            mk("banana", "b.sh"),
            mk("Cherry", "c.sh"),
            mk("Apricot", "a.sh"),
        ));
        let titles: Vec<&str> = cat.ports.iter().map(|p| p.title.as_str()).collect();
        assert_eq!(titles, ["Apricot", "banana", "Cherry"]);
    }

    #[test]
    fn hides_wrong_arch() {
        let armhf = port_json(r#""arch": ["armhf"],"#, r#"["A.sh", "a/"]"#);
        let multi = port_json(r#""arch": ["armhf", "x86_64"],"#, r#"["B.sh", "b/"]"#);
        let ok = port_json(r#""arch": ["aarch64", "x86_64"],"#, r#"["C.sh", "c/"]"#);
        let empty = port_json(r#""arch": [],"#, r#"["D.sh", "d/"]"#);
        let cat = catalog_with(&format!(
            r#""a.zip": {armhf}, "b.zip": {multi}, "c.zip": {ok}, "d.zip": {empty}"#
        ));
        assert_eq!(cat.hidden, 2);
        let kept: Vec<&str> = cat.ports.iter().map(|p| p.zip_name.as_str()).collect();
        assert_eq!(kept, ["c.zip", "d.zip"]);
    }

    #[test]
    fn hides_new_glibc() {
        let new = port_json(r#""min_glibc": "2.32","#, r#"["A.sh", "a/"]"#);
        let ok = port_json(r#""min_glibc": "2.28","#, r#"["B.sh", "b/"]"#);
        let cat = catalog_with(&format!(r#""a.zip": {new}, "b.zip": {ok}"#));
        assert_eq!(cat.hidden, 1);
        assert_eq!(cat.ports[0].zip_name, "b.zip");
    }

    #[test]
    fn glibc_comparison() {
        assert!(!glibc_exceeds("", (2, 28)));
        assert!(!glibc_exceeds("2.28", (2, 28)));
        assert!(!glibc_exceeds("2.27", (2, 28)));
        assert!(!glibc_exceeds("1.99", (2, 28)));
        assert!(glibc_exceeds("2.29", (2, 28)));
        assert!(glibc_exceeds("2.32", (2, 28)));
        assert!(glibc_exceeds("2.34", (2, 28)));
        assert!(glibc_exceeds("3.0", (2, 28)));
        assert!(glibc_exceeds("3", (2, 28)));
        // "2.34" > "2.28" must be numeric, not lexicographic: "2.100" > "2.28"
        assert!(glibc_exceeds("2.100", (2, 28)));
        assert!(!glibc_exceeds("junk", (2, 28)));
    }

    #[test]
    fn hides_missing_runtime() {
        let gone = port_json(
            r#""runtime": ["godot-3.2.3.squashfs"],"#, // no aarch64 build in utils
            r#"["A.sh", "a/"]"#,
        );
        let armhf_only = port_json(
            r#""runtime": ["godot_4.3.armhf.squashfs"],"#,
            r#"["B.sh", "b/"]"#,
        );
        let ok = port_json(r#""runtime": ["mono-6.12.0.122-aarch64.squashfs"],"#, r#"["C.sh", "c/"]"#);
        let cat = catalog_with(&format!(
            r#""a.zip": {gone}, "b.zip": {armhf_only}, "c.zip": {ok}"#
        ));
        assert_eq!(cat.hidden, 2);
        assert_eq!(cat.ports[0].zip_name, "c.zip");
    }

    #[test]
    fn hides_analog_required() {
        let one = port_json(r#""reqs": ["analog_1"],"#, r#"["A.sh", "a/"]"#);
        let two = port_json(r#""reqs": ["analog_2"],"#, r#"["B.sh", "b/"]"#);
        let either = port_json(r#""reqs": ["analog_1|analog_2"],"#, r#"["C.sh", "c/"]"#);
        let other = port_json(r#""reqs": ["power", "!lowres"],"#, r#"["D.sh", "d/"]"#);
        let cat = catalog_with(&format!(
            r#""a.zip": {one}, "b.zip": {two}, "c.zip": {either}, "d.zip": {other}"#
        ));
        assert_eq!(cat.hidden, 3);
        assert_eq!(cat.ports[0].zip_name, "d.zip");
    }

    #[test]
    fn hides_unusable_items() {
        let no_script = port_json("", r#"["onlydir/"]"#);
        let two_scripts = port_json("", r#"["A.sh", "B.sh", "ab/"]"#);
        let no_dir = port_json("", r#"["C.sh"]"#);
        let ok = port_json("", r#"["D.sh", "d/"]"#);
        let cat = catalog_with(&format!(
            r#""a.zip": {no_script}, "b.zip": {two_scripts}, "c.zip": {no_dir}, "d.zip": {ok}"#
        ));
        assert_eq!(cat.hidden, 3);
        assert_eq!(cat.ports[0].zip_name, "d.zip");
    }

    fn sample_port() -> PortEntry {
        PortEntry {
            zip_name: "2048.zip".into(),
            title: "2048".into(),
            desc: String::new(),
            inst: String::new(),
            genres: Vec::new(),
            rtr: true,
            runtimes: Vec::new(),
            size: 0,
            md5: String::new(),
            url: String::new(),
            gamedir: "2048".into(),
            script: "2048.sh".into(),
        }
    }

    #[test]
    fn install_paths() {
        let sd = scratch("paths");
        let port = sample_port();
        assert_eq!(
            scripts_dir(&sd).join(&port.script),
            sd.join("Roms/Ports (PORTS)/2048.sh")
        );
        assert_eq!(
            data_ports_dir(&sd).join(&port.gamedir),
            sd.join("Data/ports/2048")
        );
        assert_eq!(libs_dir(&sd), sd.join("Data/PortMaster/libs"));
        assert_eq!(cache_path(&sd), sd.join(".userdata/shared/kui/ports.json"));
        assert_eq!(
            media_path(&sd, "2048.sh"),
            sd.join("Roms/Ports (PORTS)/.media/2048.png")
        );
        let _ = fs::remove_dir_all(&sd);
    }

    #[test]
    fn installed_and_runtime_checks() {
        let sd = scratch("installed");
        let port = sample_port();
        assert!(!installed(&sd, &port));
        fs::create_dir_all(scripts_dir(&sd)).unwrap();
        fs::write(scripts_dir(&sd).join("2048.sh"), b"#!/bin/sh\n").unwrap();
        assert!(installed(&sd, &port));

        assert!(!runtime_installed(&sd, "godot_4.3.squashfs"));
        fs::create_dir_all(libs_dir(&sd)).unwrap();
        fs::write(libs_dir(&sd).join("godot_4.3.squashfs"), b"x").unwrap();
        assert!(runtime_installed(&sd, "godot_4.3.squashfs"));
        let _ = fs::remove_dir_all(&sd);
    }

    #[test]
    fn place_staged_moves_pieces_and_art() {
        let sd = scratch("place");
        let port = sample_port();
        let stage = kui_shared_dir(&sd).join(".port-stage");
        fs::create_dir_all(stage.join("2048")).unwrap();
        fs::write(stage.join("2048/game.bin"), b"data").unwrap();
        fs::write(stage.join("2048/cover.png"), b"png").unwrap();
        fs::write(stage.join("2048.sh"), b"#!/bin/sh\n").unwrap();

        place_staged(&sd, &port, &stage).unwrap();
        assert_eq!(fs::read(sd.join("Data/ports/2048/game.bin")).unwrap(), b"data");
        assert!(sd.join("Roms/Ports (PORTS)/2048.sh").is_file());
        assert_eq!(
            fs::read(sd.join("Roms/Ports (PORTS)/.media/2048.png")).unwrap(),
            b"png"
        );

        // and remove undoes all three, leaving runtimes alone
        fs::create_dir_all(libs_dir(&sd)).unwrap();
        fs::write(libs_dir(&sd).join("rt.squashfs"), b"x").unwrap();
        remove_port(&sd, &port).unwrap();
        assert!(!sd.join("Data/ports/2048").exists());
        assert!(!sd.join("Roms/Ports (PORTS)/2048.sh").exists());
        assert!(!sd.join("Roms/Ports (PORTS)/.media/2048.png").exists());
        assert!(libs_dir(&sd).join("rt.squashfs").exists());
        // removing again is fine
        remove_port(&sd, &port).unwrap();
        let _ = fs::remove_dir_all(&sd);
    }

    #[test]
    fn place_staged_missing_pieces_error() {
        let sd = scratch("place-miss");
        let port = sample_port();
        let stage = kui_shared_dir(&sd).join(".port-stage");
        fs::create_dir_all(&stage).unwrap();
        let err = place_staged(&sd, &port, &stage).unwrap_err();
        assert!(err.contains("no 2048/ directory"), "got: {err}");
        fs::create_dir_all(stage.join("2048")).unwrap();
        let err = place_staged(&sd, &port, &stage).unwrap_err();
        assert!(err.contains("no 2048.sh script"), "got: {err}");
        let _ = fs::remove_dir_all(&sd);
    }

    #[test]
    fn catalog_falls_back_to_cache() {
        // Network is unreachable in tests (bad URL host resolution aside,
        // fetch of the real URL may also work); we only exercise the pure
        // cache path: a pre-seeded cache parses when handed to parse_catalog.
        let body = format!(
            r#"{{ "ports": {{ "a.zip": {} }}, "utils": {UTILS} }}"#,
            port_json("", r#"["A.sh", "a/"]"#)
        );
        let cat = parse_catalog(&body, Path::new("/nonexistent")).unwrap();
        assert_eq!(cat.ports.len(), 1);
        assert_eq!(cat.hidden, 0);
    }

    #[test]
    fn md5_helper_matches_md5sum() {
        // md5sum is required on the device and in the dev shell.
        let sd = scratch("md5");
        let f = sd.join("x.bin");
        fs::write(&f, b"hello kui\n").unwrap();
        let sum = md5_file(&f).unwrap();
        assert_eq!(sum.len(), 32);
        verify_md5(&f, &sum).unwrap();
        verify_md5(&f, &sum.to_uppercase()).unwrap();
        verify_md5(&f, "").unwrap(); // no published checksum: skip
        assert!(verify_md5(&f, "00000000000000000000000000000000").is_err());
        let _ = fs::remove_dir_all(&sd);
    }
}
