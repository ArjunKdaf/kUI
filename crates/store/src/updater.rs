//! OTA updater: GitHub releases for ArjunKdaf/kUI, staged extraction to the
//! SD card, and an apply step that is safe for the running binaries
//! (copy to `<dest>.new`, then rename over).

use std::fs;
use std::path::{Path, PathBuf};

use crate::http;
use crate::json::Json;
use crate::zip;

const RELEASES_URL: &str = "https://api.github.com/repos/ArjunKdaf/kUI/releases";
const USER_AGENT: &str = "User-Agent: kui-store/0.1";
const OTA_ZIP: &str = "/tmp/kui-ota.zip";
const STAGING_DIR: &str = "/mnt/SDCARD/.kui-staged";
const SDCARD: &str = "/mnt/SDCARD";

#[derive(Debug, Clone)]
pub struct Release {
    pub tag: String,
    pub name: String,
    /// Download URL of the smallest `.zip` asset — the OS-update payload,
    /// which is always smaller than a full fresh-install card image.
    pub ota_url: Option<String>,
}

/// Fetch up to 10 releases of ArjunKdaf/kUI (newest first, as GitHub
/// returns them).
pub fn fetch_releases() -> Result<Vec<Release>, String> {
    let body = http::fetch_text(&format!("{RELEASES_URL}?per_page=10"), &[USER_AGENT])?;
    parse_releases(&body)
}

fn parse_releases(body: &str) -> Result<Vec<Release>, String> {
    let root = Json::parse(body)?;
    let arr = root
        .as_arr()
        .ok_or_else(|| "github releases: expected a JSON array".to_string())?;
    let mut out = Vec::new();
    for r in arr.iter().take(10) {
        let tag = r
            .get("tag_name")
            .and_then(Json::as_str)
            .unwrap_or("")
            .to_string();
        let name = match r.get("name").and_then(Json::as_str) {
            Some(n) if !n.is_empty() => n.to_string(),
            _ => tag.clone(),
        };
        let assets = r.get("assets").and_then(Json::as_arr).unwrap_or(&[]);
        // Pick the SMALLEST .zip asset. The OS-update payload is always
        // smaller than a full fresh-install card image, so this stays
        // correct no matter the asset name or upload order.
        let ota_url = assets
            .iter()
            .filter_map(|a| {
                let name = a.get("name").and_then(Json::as_str)?;
                if !name.to_ascii_lowercase().ends_with(".zip") {
                    return None;
                }
                let url = a.get("browser_download_url").and_then(Json::as_str)?;
                let size = a.get("size").and_then(Json::as_f64).unwrap_or(f64::MAX);
                Some((size as u64, url.to_string()))
            })
            .min_by_key(|(size, _)| *size)
            .map(|(_, url)| url);
        out.push(Release { tag, name, ota_url });
    }
    Ok(out)
}

/// Download an OTA zip and extract it into the staging directory
/// `/mnt/SDCARD/.kui-staged/`, stripping the single top-level directory
/// (`kUI-<ver>/...`) if all entries share one. Returns the staging dir.
/// `progress` is called once with the downloaded size in bytes.
pub fn download_and_stage(url: &str, mut progress: impl FnMut(u64)) -> Result<PathBuf, String> {
    let zip_path = Path::new(OTA_ZIP);
    http::download(url, zip_path, &[USER_AGENT])?;
    let size = fs::metadata(zip_path)
        .map_err(|e| format!("stat {OTA_ZIP}: {e}"))?
        .len();
    progress(size);

    let staged = PathBuf::from(STAGING_DIR);
    if staged.exists() {
        fs::remove_dir_all(&staged).map_err(|e| format!("clear {STAGING_DIR}: {e}"))?;
    }
    let result = zip::unzip_strip_toplevel(zip_path, &staged);
    let _ = fs::remove_file(zip_path);
    result?;
    Ok(staged)
}

/// Apply a staged update: every file under `staged` is copied to
/// `/mnt/SDCARD/<relative path>` via `<dest>.new` + rename (safe for the
/// running binaries). Afterwards the staging dir is removed.
pub fn apply_staged(staged: &Path) -> Result<(), String> {
    apply_tree(staged, staged, Path::new(SDCARD))?;
    fs::remove_dir_all(staged).map_err(|e| format!("remove {}: {e}", staged.display()))
}

fn apply_tree(base: &Path, dir: &Path, root: &Path) -> Result<(), String> {
    let entries = fs::read_dir(dir).map_err(|e| format!("read dir {}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("read dir {}: {e}", dir.display()))?;
        let path = entry.path();
        let ftype = entry
            .file_type()
            .map_err(|e| format!("stat {}: {e}", path.display()))?;
        if ftype.is_dir() {
            apply_tree(base, &path, root)?;
        } else if ftype.is_file() {
            let rel = path
                .strip_prefix(base)
                .map_err(|e| format!("strip prefix {}: {e}", path.display()))?;
            let dest = root.join(rel);
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent)
                    .map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
            }
            let tmp = PathBuf::from(format!("{}.new", dest.display()));
            fs::copy(&path, &tmp)
                .map_err(|e| format!("copy {} -> {}: {e}", path.display(), tmp.display()))?;
            fs::rename(&tmp, &dest)
                .map_err(|e| format!("rename {} -> {}: {e}", tmp.display(), dest.display()))?;
        }
        // symlinks and other special files are not expected in OTA zips
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_releases_and_picks_ota_asset() {
        let body = r#"[
          {
            "tag_name": "v0.3.0", "name": "kUI 0.3.0",
            "assets": [
              { "name": "kUI-source.tar.gz", "browser_download_url": "https://x/src.tar.gz" },
              { "name": "kUI-0.3.0-OTA.zip", "browser_download_url": "https://x/ota.zip" },
              { "name": "kUI-0.3.0-full.zip", "browser_download_url": "https://x/full.zip" }
            ]
          },
          {
            "tag_name": "v0.2.0", "name": "",
            "assets": [
              { "name": "kUI-0.2.0-full.zip", "browser_download_url": "https://x/old-full.zip" }
            ]
          },
          { "tag_name": "v0.1.0", "name": "first", "assets": [] }
        ]"#;
        let rels = parse_releases(body).unwrap();
        assert_eq!(rels.len(), 3);
        assert_eq!(rels[0].tag, "v0.3.0");
        assert_eq!(rels[0].name, "kUI 0.3.0");
        assert_eq!(rels[0].ota_url.as_deref(), Some("https://x/ota.zip"));
        assert_eq!(rels[1].name, "v0.2.0"); // falls back to the tag
        assert_eq!(rels[1].ota_url.as_deref(), Some("https://x/old-full.zip"));
        assert_eq!(rels[2].ota_url, None);
    }

    #[test]
    fn caps_at_ten_releases() {
        let one = r#"{ "tag_name": "vX", "name": "x", "assets": [] }"#;
        let body = format!("[{}]", vec![one; 14].join(","));
        let rels = parse_releases(&body).unwrap();
        assert_eq!(rels.len(), 10);
    }

    #[test]
    fn apply_tree_copies_via_rename() {
        let base = std::env::temp_dir().join(format!("kui-store-apply-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let staged = base.join("staged");
        let root = base.join("root");
        fs::create_dir_all(staged.join("bin")).unwrap();
        fs::create_dir_all(&root).unwrap();
        fs::write(staged.join("bin/kui"), b"new binary").unwrap();
        fs::write(staged.join("version.txt"), b"v2").unwrap();
        // pre-existing file gets replaced
        fs::create_dir_all(root.join("bin")).unwrap();
        fs::write(root.join("bin/kui"), b"old binary").unwrap();

        apply_tree(&staged, &staged, &root).unwrap();
        assert_eq!(fs::read(root.join("bin/kui")).unwrap(), b"new binary");
        assert_eq!(fs::read(root.join("version.txt")).unwrap(), b"v2");
        assert!(!root.join("bin/kui.new").exists());
        let _ = fs::remove_dir_all(&base);
    }
}
