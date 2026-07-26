//! RetroArch-style cheat download: fetch the running game's .cht from
//! the libretro-database repo — the same files RetroArch's online
//! updater ships — picked by normalized-name match against the rom stem.
//! Two GitHub API calls (cht/ listing -> per-system tree, which dodges
//! the contents API's 1000-entry cap) plus the raw file itself.

use std::path::Path;

/// kUI platform tag -> libretro-database `cht/` directory.
fn db_dir(tag: &str) -> Option<&'static str> {
    Some(match tag {
        "GB" => "Nintendo - Game Boy",
        "GBC" => "Nintendo - Game Boy Color",
        "GBA" | "GBH" => "Nintendo - Game Boy Advance",
        "FC" | "NES" => "Nintendo - Nintendo Entertainment System",
        "FDS" => "Nintendo - Family Computer Disk System",
        "SFC" | "SNES" => "Nintendo - Super Nintendo Entertainment System",
        "VB" => "Nintendo - Virtual Boy",
        "N64" => "Nintendo - Nintendo 64",
        "MD" => "Sega - Mega Drive - Genesis",
        "32X" => "Sega - 32X",
        "SMS" => "Sega - Master System - Mark III",
        "GG" => "Sega - Game Gear",
        "SEGACD" | "MEGACD" => "Sega - Mega-CD - Sega CD",
        "SG1000" => "Sega - SG-1000",
        "PS" => "Sony - PlayStation",
        "PCE" => "NEC - PC Engine - TurboGrafx 16",
        "SGFX" => "NEC - PC Engine SuperGrafx",
        "PCECD" => "NEC - PC Engine CD - TurboGrafx-CD",
        "LYNX" => "Atari - Lynx",
        "A2600" => "Atari - 2600",
        "A7800" => "Atari - 7800",
        "JAGUAR" => "Atari - Jaguar",
        "WS" => "Bandai - WonderSwan",
        "WSC" => "Bandai - WonderSwan Color",
        "NGP" => "SNK - Neo Geo Pocket",
        "NGPC" => "SNK - Neo Geo Pocket Color",
        "MSX" => "Microsoft - MSX",
        _ => return None,
    })
}

/// Device curl, CA bundle from the card (same approach as kui-ra).
fn fetch(url: &str) -> Result<Vec<u8>, String> {
    let tmp = std::env::temp_dir().join(format!("kui-cheatdl-{}", std::process::id()));
    let curl = if Path::new("/usr/bin/curl").exists() { "/usr/bin/curl" } else { "curl" };
    let mut cmd = std::process::Command::new(curl);
    cmd.args(["-s", "--max-time", "20", "-L", "-A", "kUI-cheatdl"]);
    let ca = "/mnt/SDCARD/.system/res/cacert.pem";
    if Path::new(ca).is_file() {
        cmd.arg("--cacert").arg(ca);
    } else if curl == "/usr/bin/curl" {
        cmd.arg("-k");
    }
    cmd.arg("-o").arg(&tmp).arg(url);
    let st = cmd.status().map_err(|e| format!("curl: {e}"))?;
    let body = std::fs::read(&tmp).unwrap_or_default();
    let _ = std::fs::remove_file(&tmp);
    if !st.success() || body.is_empty() {
        return Err("download failed (network?)".into());
    }
    Ok(body)
}

/// All string values for `"key": "..."` in a JSON blob, in order.
/// Tolerates optional whitespace after the colon; good enough for the
/// GitHub API's own output (no escaped quotes in these file names).
fn json_strings(json: &str, key: &str) -> Vec<String> {
    let pat = format!("\"{key}\"");
    let mut out = Vec::new();
    let mut rest = json;
    while let Some(i) = rest.find(&pat) {
        rest = &rest[i + pat.len()..];
        let Some(c) = rest.find(':') else { break };
        let after = rest[c + 1..].trim_start();
        if let Some(v) = after.strip_prefix('"')
            && let Some(end) = v.find('"')
        {
            out.push(v[..end].to_string());
        }
        rest = &rest[c + 1..];
    }
    out
}

/// Lowercased alphanumerics with every (...)/[...] group dropped —
/// "Super Metroid (Japan, USA) (En,Ja)" and "Super Metroid" both
/// normalize to "supermetroid".
fn norm(s: &str) -> String {
    let mut out = String::new();
    let mut depth = 0u32;
    for c in s.chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = depth.saturating_sub(1),
            _ if depth == 0 && c.is_ascii_alphanumeric() => {
                out.push(c.to_ascii_lowercase());
            }
            _ => {}
        }
    }
    out
}

fn enc(s: &str) -> String {
    let mut o = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                o.push(b as char)
            }
            _ => o.push_str(&format!("%{b:02X}")),
        }
    }
    o
}

/// Fetch the best-matching cheat file for `stem` into `dest`.
/// Returns (cheat count, matched database name).
pub fn download(tag: &str, stem: &str, dest: &Path) -> Result<(usize, String), String> {
    let dir = db_dir(tag).ok_or_else(|| format!("no cheat database for {tag}"))?;

    // cht/ listing pairs each system dir with its tree sha
    let listing = fetch("https://api.github.com/repos/libretro/libretro-database/contents/cht")?;
    let listing = String::from_utf8_lossy(&listing);
    let names = json_strings(&listing, "name");
    let shas = json_strings(&listing, "sha");
    let sha = names
        .iter()
        .position(|n| n == dir)
        .and_then(|i| shas.get(i))
        .ok_or_else(|| format!("{dir}: not in cheat database"))?;

    let tree = fetch(&format!(
        "https://api.github.com/repos/libretro/libretro-database/git/trees/{sha}"
    ))?;
    let files = json_strings(&String::from_utf8_lossy(&tree), "path");

    // best match: exact normalized name, then prefix either way;
    // prefer USA/World regions, then the shortest (least-suffixed) name
    let want = norm(stem);
    if want.is_empty() {
        return Err("unusable rom name".into());
    }
    let region_rank = |name: &str| -> u32 {
        let l = name.to_ascii_lowercase();
        if l.contains("(usa") || l.contains("(world") {
            0
        } else if l.contains("(japan, usa") {
            1
        } else {
            2
        }
    };
    let mut best: Option<(u32, u32, usize, &String)> = None;
    for f in &files {
        let Some(base) = f.strip_suffix(".cht") else { continue };
        let n = norm(base);
        let tier = if n == want {
            0
        } else if !n.is_empty() && (n.starts_with(&want) || want.starts_with(&n)) {
            1
        } else {
            continue;
        };
        let cand = (tier, region_rank(base), f.len(), f);
        if best.as_ref().map(|b| cand < *b).unwrap_or(true) {
            best = Some(cand);
        }
    }
    let (.., file) = best.ok_or_else(|| format!("no cheats found for \"{stem}\""))?;

    let body = fetch(&format!(
        "https://raw.githubusercontent.com/libretro/libretro-database/master/cht/{}/{}",
        enc(dir),
        enc(file)
    ))?;
    let text = String::from_utf8_lossy(&body);
    let count: usize = text
        .lines()
        .find_map(|l| {
            let (k, v) = l.split_once('=')?;
            (k.trim() == "cheats").then(|| v.trim().parse().ok())?
        })
        .unwrap_or(0);
    if count == 0 {
        return Err(format!("empty cheat file for \"{stem}\""));
    }
    if let Some(p) = dest.parent() {
        let _ = std::fs::create_dir_all(p);
    }
    std::fs::write(dest, body.as_slice()).map_err(|e| format!("write: {e}"))?;
    Ok((count, file.trim_end_matches(".cht").to_string()))
}
