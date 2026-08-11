//! PakDek storefront: manifest fetch, installed-version detection,
//! install and remove.

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use crate::http;
use crate::json::Json;
use crate::zip;

const STOREFRONT_URL: &str = "https://raw.githubusercontent.com/ArjunKdaf/kUI/main/storefront.json";
const USER_AGENT: &str = "User-Agent: kui-store/0.1";
const PLATFORM: &str = "tg5040";
const SDCARD: &str = "/mnt/SDCARD";

/// Where PakDek keeps its per-pak root baselines and the removal log.
const PAKDEK_STATE: &str = "/mnt/SDCARD/.userdata/shared/pakdek";

/// SD-root top-level entries kUI owns. Even if a pak's baseline somehow
/// missed one, leftover cleanup never touches these. (Every dotfile and
/// every `kui*` entry is also spared, unconditionally.)
const PROTECTED_ROOT: &[&str] = &[
    "Roms",
    "Bios",
    "Saves",
    "States",
    "Cheats",
    "Collections",
    "Screenshots",
    "Recordings",
    "Artwork",
    "Overlays",
    "Shaders",
    "Themes",
    "Emus",
    "Tools",
    "bin",
    "LICENSES",
    "README.txt",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PakKind {
    Emu,
    Tool,
}

#[derive(Debug, Clone)]
pub struct Pak {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub author: String,
    pub repo_url: String,
    pub release_filename: String,
    pub kind: PakKind,
    pub categories: Vec<String>,
}

/// Fetch and parse the storefront manifest. Only paks that list the
/// `tg5040` platform and are not disabled are returned. For paks that are
/// already installed, the pinned version is replaced with the repo's latest
/// release tag (best-effort), so update detection tracks upstream releases
/// rather than the storefront pin.
pub fn fetch_storefront() -> Result<Vec<Pak>, String> {
    let body = http::fetch_text(STOREFRONT_URL, &[])?;
    let mut paks = parse_storefront(&body)?;
    refresh_installed_versions(&mut paks);
    Ok(paks)
}

/// For installed paks only (a handful, vs the whole store), swap the pinned
/// version for the repo's latest release tag. One GitHub API call per
/// installed pak; a failed lookup (offline, rate-limited, non-GitHub repo)
/// keeps the pin. Only reached when the storefront fetch itself succeeded,
/// so the network is already known to be up.
fn refresh_installed_versions(paks: &mut [Pak]) {
    for pak in paks.iter_mut() {
        if installed_version(pak).is_none() {
            continue;
        }
        if let Ok((tag, _)) = latest_release(pak) {
            pak.version = tag;
        }
    }
}

fn parse_storefront(body: &str) -> Result<Vec<Pak>, String> {
    let root = Json::parse(body)?;
    let paks = root
        .get("paks")
        .and_then(Json::as_arr)
        .ok_or("storefront.json: missing \"paks\" array")?;
    let mut out = Vec::new();
    for p in paks {
        if p.get("disabled").and_then(Json::as_bool).unwrap_or(false) {
            continue;
        }
        let on_platform = p
            .get("platforms")
            .and_then(Json::as_arr)
            .is_some_and(|ps| ps.iter().any(|x| x.as_str() == Some(PLATFORM)));
        if !on_platform {
            continue;
        }
        let s = |key: &str| p.get(key).and_then(Json::as_str).unwrap_or("").to_string();
        let release_filename = s("release_filename");
        if release_filename.is_empty() {
            continue; // nothing installable
        }
        // storefront_name is the display name; "name" is the short pak name.
        let name = match p.get("storefront_name").and_then(Json::as_str) {
            Some(n) if !n.is_empty() => n.to_string(),
            _ => s("name"),
        };
        let kind = match p.get("type").and_then(Json::as_str) {
            Some("EMU") => PakKind::Emu,
            _ => PakKind::Tool,
        };
        let categories = p
            .get("categories")
            .and_then(Json::as_arr)
            .map(|cs| cs.iter().filter_map(|c| c.as_str().map(str::to_string)).collect())
            .unwrap_or_default();
        out.push(Pak {
            id: s("id"),
            name,
            version: s("version"),
            description: s("description"),
            author: s("author"),
            repo_url: s("repo_url"),
            release_filename,
            kind,
            categories,
        });
    }
    Ok(out)
}

/// Installed folder name: release filename minus `.zip`
/// (`Grout.pak.zip` -> `Grout.pak`; `PORTS.pakz` -> `PORTS.pak`).
fn pak_folder(pak: &Pak) -> String {
    let f = &pak.release_filename;
    if let Some(base) = f.strip_suffix(".zip") {
        base.to_string()
    } else if let Some(base) = f.strip_suffix(".pakz") {
        format!("{base}.pak")
    } else {
        f.clone()
    }
}

fn install_base(kind: PakKind) -> PathBuf {
    match kind {
        // User paks never go in .system.
        PakKind::Tool => PathBuf::from(format!("{SDCARD}/Tools/{PLATFORM}")),
        PakKind::Emu => PathBuf::from(format!("{SDCARD}/Emus/{PLATFORM}")),
    }
}

fn install_dir(pak: &Pak) -> PathBuf {
    install_base(pak.kind).join(pak_folder(pak))
}

/// The version recorded in the installed pak's `pak.json`, if the pak is
/// installed and its manifest is readable.
pub fn installed_version(pak: &Pak) -> Option<String> {
    let manifest = install_dir(pak).join("pak.json");
    let body = fs::read_to_string(manifest).ok()?;
    let root = Json::parse(&body).ok()?;
    root.get("version").and_then(Json::as_str).map(str::to_string)
}

/// The GitHub `releases/latest` API URL for a pak's repo, if it is one.
fn api_latest_url(repo_url: &str) -> Option<String> {
    let repo = repo_url
        .trim_end_matches('/')
        .strip_prefix("https://github.com/")?;
    Some(format!("https://api.github.com/repos/{repo}/releases/latest"))
}

/// The latest GitHub release for a pak's repo: `(tag, download URL)` of the
/// asset named `release_filename`. Errors (offline, rate-limited, non-GitHub
/// repo, no such asset) leave the caller on the pinned storefront data.
fn latest_release(pak: &Pak) -> Result<(String, String), String> {
    let url = api_latest_url(&pak.repo_url)
        .ok_or_else(|| format!("not a GitHub repo: {}", pak.repo_url))?;
    let body = http::fetch_text(&url, &[USER_AGENT])?;
    parse_latest_release(&body, &pak.release_filename)
}

/// Pull `(tag_name, asset URL)` out of a GitHub `releases/latest` response.
fn parse_latest_release(body: &str, release_filename: &str) -> Result<(String, String), String> {
    let root = Json::parse(body)?;
    let tag = root
        .get("tag_name")
        .and_then(Json::as_str)
        .filter(|t| !t.is_empty())
        .ok_or("github latest release: missing tag_name")?
        .to_string();
    let assets = root.get("assets").and_then(Json::as_arr).unwrap_or(&[]);
    let url = assets
        .iter()
        .find(|a| a.get("name").and_then(Json::as_str) == Some(release_filename))
        .and_then(|a| a.get("browser_download_url").and_then(Json::as_str))
        .ok_or_else(|| format!("latest release has no asset {release_filename}"))?
        .to_string();
    Ok((tag, url))
}

/// Download and install (or update) a pak. Installs the repo's LATEST
/// release when it can be resolved; the pinned storefront version/URL is
/// the fallback (offline, rate-limited). `progress` receives short
/// human-readable status lines. Returns the version that was installed.
pub fn install_pak(pak: &Pak, mut progress: impl FnMut(&str)) -> Result<String, String> {
    // Record the SD-root baseline once, before this pak touches anything,
    // so removal can later spot the files it drops at root — even ones
    // that only appear at first launch, not at install. Preserved across
    // updates (an update re-runs install_pak but must keep the original).
    record_root_baseline(pak);

    progress("Checking latest release...");
    let (version, url) = match latest_release(pak) {
        Ok((tag, url)) => (tag, url),
        Err(_) => (
            pak.version.clone(),
            format!(
                "{}/releases/download/{}/{}",
                pak.repo_url.trim_end_matches('/'),
                pak.version,
                pak.release_filename
            ),
        ),
    };
    let tmp_zip = PathBuf::from("/tmp").join(&pak.release_filename);
    progress(&format!("Downloading {} {}...", pak.name, version));
    http::download(&url, &tmp_zip, &[])?;

    let dest = install_dir(pak);
    progress("Extracting...");
    let result = zip::unzip(&tmp_zip, &dest);
    let _ = fs::remove_file(&tmp_zip);
    result?;

    let post = dest.join("post_install.sh");
    if post.is_file() {
        progress("Running post-install script...");
        let status = Command::new("sh")
            .arg(&post)
            .current_dir(&dest)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|e| format!("failed to run post_install.sh: {e}"))?;
        if !status.success() {
            return Err(format!("post_install.sh failed ({status})"));
        }
    }
    progress("Done");
    Ok(version)
}

/// Remove an installed pak: its install dir, plus the files it left at the
/// SD-card root (config/settings, stray binaries, licenses). rm -rf
/// semantics — succeeds if the dir is already absent.
pub fn remove_pak(pak: &Pak) -> Result<(), String> {
    // Clean root leftovers first, while the baseline still exists.
    clean_root_leftovers(pak);

    let dir = install_dir(pak);
    if dir.exists() {
        fs::remove_dir_all(&dir).map_err(|e| format!("remove {}: {e}", dir.display()))?;
    }
    let _ = fs::remove_file(baseline_path(pak));
    Ok(())
}

/// Per-pak baseline file: the SD-root snapshot taken at install time.
fn baseline_path(pak: &Pak) -> PathBuf {
    PathBuf::from(PAKDEK_STATE).join(format!("{}.baseline", pak_folder(pak)))
}

/// The SD-root top-level entry names, as they are right now.
fn root_entries() -> Vec<String> {
    match fs::read_dir(SDCARD) {
        Ok(rd) => rd
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// Snapshot the SD root for `pak`, unless a baseline already exists (an
/// update must not overwrite the original install's snapshot).
fn record_root_baseline(pak: &Pak) {
    let path = baseline_path(pak);
    if path.exists() {
        return;
    }
    let _ = fs::create_dir_all(PAKDEK_STATE);
    let _ = fs::write(&path, root_entries().join("\n"));
}

/// Every other installed pak's baseline (for conservative attribution).
fn other_baselines(exclude: &Pak) -> Vec<HashSet<String>> {
    let me = baseline_path(exclude);
    let mut out = Vec::new();
    if let Ok(rd) = fs::read_dir(PAKDEK_STATE) {
        for e in rd.flatten() {
            let p = e.path();
            if p == me || p.extension().and_then(|x| x.to_str()) != Some("baseline") {
                continue;
            }
            if let Ok(body) = fs::read_to_string(&p) {
                out.push(body.lines().map(str::to_string).collect());
            }
        }
    }
    out
}

/// Which of `current`'s root entries are this pak's leftovers: they appeared
/// after its baseline, aren't kUI-owned, and — for safety when several paks
/// are installed — every *other* pak's baseline already contained them, so
/// they're unambiguously this pak's. Pure, so it can be unit-tested.
fn leftover_candidates(
    baseline: &HashSet<String>,
    current: &[String],
    others: &[HashSet<String>],
) -> Vec<String> {
    current
        .iter()
        .filter(|name| {
            let n = name.as_str();
            !baseline.contains(*name)
                && !n.starts_with('.')
                && !n.starts_with("kui")
                && !PROTECTED_ROOT.contains(&n)
                // if any other installed pak also lacked this at its
                // install, ownership is ambiguous — leave it alone.
                && others.iter().all(|b| b.contains(*name))
        })
        .cloned()
        .collect()
}

/// Delete the root files `pak` introduced. Best-effort and logged; a pak
/// installed before baselines existed has none, so nothing is touched.
fn clean_root_leftovers(pak: &Pak) {
    let baseline: HashSet<String> = match fs::read_to_string(baseline_path(pak)) {
        Ok(b) => b.lines().map(str::to_string).collect(),
        Err(_) => return,
    };
    let others = other_baselines(pak);
    let current = root_entries();
    for name in leftover_candidates(&baseline, &current, &others) {
        let p = PathBuf::from(SDCARD).join(&name);
        let res = if p.is_dir() {
            fs::remove_dir_all(&p)
        } else {
            fs::remove_file(&p)
        };
        log_removal(pak, &name, res.is_ok());
    }
}

/// Append a line to PakDek's removal log — an audit trail of what leftover
/// cleanup deleted, since it touches the SD-card root.
fn log_removal(pak: &Pak, entry: &str, ok: bool) {
    use std::io::Write as _;
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let line = format!(
        "{secs} {} {} {entry}\n",
        pak_folder(pak),
        if ok { "removed" } else { "FAILED" },
    );
    let log = PathBuf::from(PAKDEK_STATE).join("removals.log");
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(&log) {
        let _ = f.write_all(line.as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "name": "kUI PakDek",
      "paks": [
        {
          "id": "jF8cW5zN2m", "storefront_name": "Grout", "name": "Grout",
          "version": "v4.8.1.0", "type": "TOOL",
          "description": "A RomM client.", "author": "The RomM Team",
          "repo_url": "https://github.com/rommapp/grout",
          "release_filename": "Grout.pak.zip",
          "platforms": ["tg5040", "tg5050"],
          "categories": ["ROM Management"],
          "disabled": false
        },
        {
          "id": "fW6pX3kB9v", "storefront_name": "Nintendo DS (DraStic)", "name": "NDS",
          "version": "0.11.0", "type": "EMU",
          "description": "Runs drastic.", "author": "josegonzalez",
          "repo_url": "https://github.com/josegonzalez/minui-nintendo-ds-pak",
          "release_filename": "NDS.pak.zip",
          "platforms": ["tg5040"], "categories": ["Emulators"], "disabled": false
        },
        {
          "id": "off1", "storefront_name": "Disabled Thing", "name": "Off",
          "version": "1.0", "type": "TOOL", "description": "", "author": "",
          "repo_url": "https://example.com/x", "release_filename": "Off.pak.zip",
          "platforms": ["tg5040"], "categories": [], "disabled": true
        },
        {
          "id": "other1", "storefront_name": "Wrong Platform", "name": "WP",
          "version": "1.0", "type": "TOOL", "description": "", "author": "",
          "repo_url": "https://example.com/y", "release_filename": "WP.pak.zip",
          "platforms": ["miyoomini"], "categories": [], "disabled": false
        },
        {
          "id": "gQ7hT4dK9c", "storefront_name": "Portmaster", "name": "PORTS",
          "version": "2.11.1", "type": "EMU", "description": "PortMaster.",
          "author": "ben16w", "repo_url": "https://github.com/ben16w/minui-portmaster",
          "release_filename": "PORTS.pakz",
          "platforms": ["tg5040"], "categories": ["Emulators", "Ports"], "disabled": false
        }
      ]
    }"#;

    #[test]
    fn parses_and_filters() {
        let paks = parse_storefront(SAMPLE).unwrap();
        let ids: Vec<&str> = paks.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["jF8cW5zN2m", "fW6pX3kB9v", "gQ7hT4dK9c"]);
        let grout = &paks[0];
        assert_eq!(grout.name, "Grout");
        assert_eq!(grout.kind, PakKind::Tool);
        assert_eq!(grout.version, "v4.8.1.0");
        assert_eq!(grout.categories, ["ROM Management"]);
        assert_eq!(paks[1].kind, PakKind::Emu);
        assert_eq!(paks[1].name, "Nintendo DS (DraStic)");
    }

    fn set(names: &[&str]) -> HashSet<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn leftovers_single_pak() {
        // Baseline had Roms + LICENSES; after install/use the root also has
        // the pak's droppings. Only the genuinely-new, non-owned entries go.
        let baseline = set(&["Roms", "LICENSES", "Bios"]);
        let current = [
            "Roms".into(),        // predates the pak
            "LICENSES".into(),    // predates the pak
            "Bios".into(),        // predates the pak
            "config.json".into(), // dropped by the pak -> remove
            "calculator".into(),  // dropped by the pak -> remove
            "LICENSE".into(),     // dropped by the pak -> remove
        ];
        let got = leftover_candidates(&baseline, &current, &[]);
        assert_eq!(got, ["config.json", "calculator", "LICENSE"]);
    }

    #[test]
    fn leftovers_spare_protected_and_kui_and_dotfiles() {
        let baseline = set(&[]);
        let current = [
            "Roms".into(),          // protected
            "Emus".into(),          // protected
            "README.txt".into(),    // protected (kUI's)
            "kui-launcher".into(),  // kui*
            ".userdata".into(),     // dotfile
            "settings.json".into(), // real leftover -> remove
        ];
        let got = leftover_candidates(&baseline, &current, &[]);
        assert_eq!(got, ["settings.json"]);
    }

    #[test]
    fn leftovers_skip_when_another_pak_could_own_it() {
        // Removing pak A. `mystery` appeared after A. Pak B's baseline also
        // lacks `mystery` (it appeared after B too) -> ambiguous, keep it.
        // `onlyA` is in B's baseline (predates B) -> unambiguously A's.
        let baseline_a = set(&["Roms"]);
        let current = ["Roms".into(), "mystery".into(), "onlyA".into()];
        let baseline_b = set(&["Roms", "onlyA"]);
        let got = leftover_candidates(&baseline_a, &current, &[baseline_b]);
        assert_eq!(got, ["onlyA"]);
    }

    #[test]
    fn latest_release_api_url() {
        assert_eq!(
            api_latest_url("https://github.com/rommapp/grout").as_deref(),
            Some("https://api.github.com/repos/rommapp/grout/releases/latest")
        );
        // trailing slash tolerated; non-GitHub repos have no API URL
        assert_eq!(
            api_latest_url("https://github.com/ben16w/minui-portmaster/").as_deref(),
            Some("https://api.github.com/repos/ben16w/minui-portmaster/releases/latest")
        );
        assert_eq!(api_latest_url("https://example.com/x"), None);
    }

    #[test]
    fn parses_latest_release() {
        let body = r#"{
          "tag_name": "v4.9.0",
          "assets": [
            { "name": "Grout-source.tar.gz", "browser_download_url": "https://x/src.tar.gz" },
            { "name": "Grout.pak.zip", "browser_download_url": "https://x/Grout.pak.zip" }
          ]
        }"#;
        let (tag, url) = parse_latest_release(body, "Grout.pak.zip").unwrap();
        assert_eq!(tag, "v4.9.0");
        assert_eq!(url, "https://x/Grout.pak.zip");
        // no asset with the storefront's filename -> error (pinned fallback)
        assert!(parse_latest_release(body, "NDS.pak.zip").is_err());
        // rate-limit / error bodies have no tag_name -> error
        assert!(parse_latest_release(r#"{ "message": "rate limited" }"#, "x.zip").is_err());
    }

    #[test]
    fn folders_and_paths() {
        let paks = parse_storefront(SAMPLE).unwrap();
        assert_eq!(pak_folder(&paks[0]), "Grout.pak");
        assert_eq!(pak_folder(&paks[2]), "PORTS.pak"); // .pakz
        assert_eq!(
            install_dir(&paks[0]),
            PathBuf::from("/mnt/SDCARD/Tools/tg5040/Grout.pak")
        );
        assert_eq!(
            install_dir(&paks[1]),
            PathBuf::from("/mnt/SDCARD/Emus/tg5040/NDS.pak")
        );
    }
}
