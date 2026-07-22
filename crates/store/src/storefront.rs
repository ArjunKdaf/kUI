//! PakDek storefront: manifest fetch, installed-version detection,
//! install and remove.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use crate::http;
use crate::json::Json;
use crate::zip;

const STOREFRONT_URL: &str = "https://raw.githubusercontent.com/ArjunKdaf/kUI/main/storefront.json";
const PLATFORM: &str = "tg5040";
const SDCARD: &str = "/mnt/SDCARD";

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
/// `tg5040` platform and are not disabled are returned.
pub fn fetch_storefront() -> Result<Vec<Pak>, String> {
    let body = http::fetch_text(STOREFRONT_URL, &[])?;
    parse_storefront(&body)
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

/// Download and install (or update) a pak. `progress` receives short
/// human-readable status lines.
pub fn install_pak(pak: &Pak, mut progress: impl FnMut(&str)) -> Result<(), String> {
    let url = format!(
        "{}/releases/download/{}/{}",
        pak.repo_url.trim_end_matches('/'),
        pak.version,
        pak.release_filename
    );
    let tmp_zip = PathBuf::from("/tmp").join(&pak.release_filename);
    progress(&format!("Downloading {} {}...", pak.name, pak.version));
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
    Ok(())
}

/// Remove an installed pak (rm -rf semantics: succeeds if already absent).
pub fn remove_pak(pak: &Pak) -> Result<(), String> {
    let dir = install_dir(pak);
    if !dir.exists() {
        return Ok(());
    }
    fs::remove_dir_all(&dir).map_err(|e| format!("remove {}: {e}", dir.display()))
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
