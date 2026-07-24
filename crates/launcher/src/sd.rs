//! SD card layout: paths, platform scanning, carousel art resolution.
//! The layout is a frozen compat contract.

use std::path::{Path, PathBuf};

const NEWLINE_CH: char = 0x0A as char;
const NL: &str = "\n";

pub struct Sd {
    pub root: PathBuf,
}

#[derive(Debug, Clone)]
pub struct PlatformEntry {
    /// Folder name as-is, e.g. "Game Boy (GB)". (Used by Lists/Covers next.)
    #[allow(dead_code)]
    pub folder: String,
    /// Display name without the tag, e.g. "Game Boy".
    pub display: String,
    /// Uppercase tag, e.g. "GB".
    pub tag: String,
    /// Full path to the ROM folder.
    pub dir: PathBuf,
    /// Visible ROM files, sorted.
    pub roms: Vec<String>,
}

impl Sd {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn roms_dir(&self) -> PathBuf {
        self.root.join("Roms")
    }

    pub fn carousel_res(&self) -> PathBuf {
        self.root.join(".system/res/carousel")
    }

    /// Emu pak launch script for a tag (system paks, then user Emus dir).
    pub fn emu_launch(&self, tag: &str) -> Option<PathBuf> {
        let sys = self
            .root
            .join(format!(".system/tg5040/paks/Emus/{tag}.pak/launch.sh"));
        if sys.is_file() {
            return Some(sys);
        }
        let user = self.root.join(format!("Emus/tg5040/{tag}.pak/launch.sh"));
        user.is_file().then_some(user)
    }

    /// Scan `Roms/` for non-empty platform folders, alphabetical.
    pub fn scan_platforms(&self) -> Vec<PlatformEntry> {
        let mut out = Vec::new();
        let Ok(rd) = std::fs::read_dir(self.roms_dir()) else {
            return out;
        };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || !e.path().is_dir() {
                continue;
            }
            let roms = visible_files(&e.path());
            if roms.is_empty() {
                continue;
            }
            let (display, tag) = split_tag(&name);
            out.push(PlatformEntry {
                folder: name,
                display,
                tag,
                dir: e.path(),
                roms,
            });
        }
        out.sort_by_key(|p| p.display.to_lowercase());
        out
    }

    /// Carousel background PNG for an entry: folder-name key first, tag second.
    pub fn carousel_bg(&self, entry: &PlatformEntry) -> Option<PathBuf> {
        let res = self.carousel_res();
        let by_folder = res.join(format!("{}.png", art_key(&entry.display)));
        if by_folder.is_file() {
            return Some(by_folder);
        }
        let by_tag = res.join(format!("{}.png", entry.tag.to_lowercase()));
        by_tag.is_file().then_some(by_tag)
    }

    /// Carousel logo PNG, same resolution rules under `logos/`.
    pub fn carousel_logo(&self, entry: &PlatformEntry) -> Option<PathBuf> {
        let res = self.carousel_res().join("logos");
        let by_folder = res.join(format!("{}.png", art_key(&entry.display)));
        if by_folder.is_file() {
            return Some(by_folder);
        }
        let by_tag = res.join(format!("{}.png", entry.tag.to_lowercase()));
        by_tag.is_file().then_some(by_tag)
    }
}

/// Theme colors. Roles: c1 main, c2 accent, c4 list text, c5 selected
/// text, c6 hint text.
pub struct Theme {
    pub c1: [f32; 4],
    pub c2: [f32; 4],
    pub c4: [f32; 4],
    /// Hint/info color — unused until the hub's info surfaces (M2).
    #[allow(dead_code)]
    pub c6: [f32; 4],
    /// Background clear color when no art is shown.
    pub c7: [f32; 4],
}



impl Theme {
    /// Build from the config store.
    pub fn from_config(cfg: &kui_config::Config) -> Self {
        Self {
            c1: cfg.get_color("theme.color1", 0x00FF55),
            c2: cfg.get_color("theme.color2", 0xB3FFB3),
            c4: cfg.get_color("theme.color4", 0x505050),
            c6: cfg.get_color("theme.color6", 0xFFFFFF),
            c7: cfg.get_color("theme.color7", 0x000000),
        }
    }
}

impl Sd {
    fn pins_path(&self) -> PathBuf {
        self.root.join(".userdata/shared/kui/pins.txt")
    }

    /// Pinned ROMs as SD-relative paths.
    pub fn pins(&self) -> Vec<String> {
        std::fs::read_to_string(self.pins_path())
            .map(|t| {
                t.lines()
                    .map(|l| l.split('\t').next().unwrap_or(l).to_string())
                    .filter(|l| !l.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Toggle a pin; returns true if now pinned.
    pub fn pin_toggle(&self, rom_abs: &Path) -> bool {
        let Ok(rel) = rom_abs.strip_prefix(&self.root) else { return false };
        let rel = format!("/{}", rel.display());
        let mut pins = self.pins();
        let now_pinned = if let Some(i) = pins.iter().position(|p| p == &rel) {
            pins.remove(i);
            false
        } else {
            pins.push(rel);
            true
        };
        if let Some(dir) = self.pins_path().parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(self.pins_path(), pins.join("
") + "
");
        now_pinned
    }

    /// WIPE: delete the ROM, its boxart/metadata, and purge it from
    /// recents and pins.
    pub fn wipe_game(&self, rom_abs: &Path) {
        let _ = std::fs::remove_file(rom_abs);
        if let (Some(dir), Some(stem)) = (rom_abs.parent(), rom_abs.file_stem()) {
            let stem = stem.to_string_lossy();
            let _ = std::fs::remove_file(dir.join(".media").join(format!("{stem}.png")));
            let _ = std::fs::remove_file(dir.join(".media").join(format!("{stem}.info")));
        }
        if let Ok(rel) = rom_abs.strip_prefix(&self.root) {
            let rel = format!("/{}", rel.display());
            // recents
            if let Ok(t) = std::fs::read_to_string(self.recent_path()) {
                let kept: Vec<&str> = t
                    .lines()
                    .filter(|l| l.split('\t').next() != Some(rel.as_str()))
                    .collect();
                let _ = std::fs::write(self.recent_path(), kept.join("
") + "
");
            }
            // pins
            let pins: Vec<String> = self.pins().into_iter().filter(|p| p != &rel).collect();
            let _ = std::fs::write(self.pins_path(), pins.join("
") + "
");
        }
    }

    /// Is a ROM pinned?
    pub fn is_pinned(&self, rom_abs: &Path) -> bool {
        let Ok(rel) = rom_abs.strip_prefix(&self.root) else { return false };
        let rel = format!("/{}", rel.display());
        self.pins().contains(&rel)
    }
}

/// One recents line: SD-relative path + optional alias.
pub struct Recent {
    /// Absolute path (resolved against the SD root).
    pub rom: PathBuf,
    pub alias: String,
}

impl Sd {
    fn recent_path(&self) -> PathBuf {
        self.root.join(".userdata/shared/kui/recents.txt")
    }

    /// Read recents, newest first. Entries whose ROM vanished are skipped.
    pub fn recents(&self) -> Vec<Recent> {
        let Ok(text) = std::fs::read_to_string(self.recent_path()) else {
            return Vec::new();
        };
        text.lines()
            .filter_map(|l| {
                let (rel, alias) = l.split_once('\t').unwrap_or((l, ""));
                if rel.is_empty() {
                    return None;
                }
                let rom = self.root.join(rel.trim_start_matches('/'));
                rom.is_file().then(|| Recent {
                    rom,
                    alias: if alias.is_empty() {
                        rel.rsplit('/').next().unwrap_or(rel).to_string()
                    } else {
                        alias.to_string()
                    },
                })
            })
            .collect()
    }

    /// Drop a rom from recents.
    pub fn remove_recent(&self, rom_abs: &Path) {
        let Ok(rel) = rom_abs.strip_prefix(&self.root) else { return };
        let rel = format!("/{}", rel.display());
        let Ok(text) = std::fs::read_to_string(self.recent_path()) else {
            return;
        };
        let lines: Vec<&str> =
            text.lines().filter(|l| l.split('\t').next() != Some(rel.as_str())).collect();
        let _ = std::fs::write(self.recent_path(), lines.join("\n") + "\n");
    }

    /// Prepend a launch to recents (dedupe, newest first, cap 24).
    pub fn add_recent(&self, rom_abs: &Path, alias: &str) {
        let Ok(rel) = rom_abs.strip_prefix(&self.root) else { return };
        let rel = format!("/{}", rel.display());
        let mut lines: Vec<String> = std::fs::read_to_string(self.recent_path())
            .map(|t| t.lines().map(str::to_string).collect())
            .unwrap_or_default();
        lines.retain(|l| l.split('\t').next() != Some(rel.as_str()));
        lines.insert(0, format!("{rel}\t{alias}"));
        lines.truncate(24);
        if let Some(dir) = self.recent_path().parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(self.recent_path(), lines.join("
") + "
");
    }

    /// Collections: `Collections/*.txt`, `.disabled` ones skipped.
    pub fn collections(&self) -> Vec<(String, PathBuf)> {
        let mut out = Vec::new();
        if let Ok(rd) = std::fs::read_dir(self.root.join("Collections")) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if let Some(base) = name.strip_suffix(".txt") {
                    out.push((base.to_string(), e.path()));
                }
            }
        }
        out.sort_by_key(|c| c.0.to_lowercase());
        out
    }

    /// ROM paths listed in a collection file, existing files only.
    pub fn collection_games(&self, file: &Path) -> Vec<PathBuf> {
        std::fs::read_to_string(file)
            .map(|t| {
                t.lines()
                    .filter(|l| !l.trim().is_empty())
                    .map(|l| self.root.join(l.trim().trim_start_matches('/')))
                    .filter(|p| p.is_file())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Carousel art by raw key (specials: "recents", "collections", "the_dude").
    pub fn carousel_bg_key(&self, key: &str) -> Option<PathBuf> {
        let p = self.carousel_res().join(format!("{key}.png"));
        p.is_file().then_some(p)
    }

    pub fn carousel_logo_key(&self, key: &str) -> Option<PathBuf> {
        let p = self.carousel_res().join("logos").join(format!("{key}.png"));
        p.is_file().then_some(p)
    }

    /// Create a new empty collection; returns its path.
    pub fn collection_create(&self, name: &str) -> Option<PathBuf> {
        let dir = self.root.join("Collections");
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join(format!("{name}.txt"));
        if p.exists() {
            return None;
        }
        std::fs::write(&p, "").ok()?;
        Some(p)
    }

    pub fn collection_delete(&self, file: &Path) {
        let _ = std::fs::remove_file(file);
    }

    /// Append a ROM (SD-relative) to a collection if not present.
    pub fn collection_add(&self, file: &Path, rom_abs: &Path) {
        let Ok(rel) = rom_abs.strip_prefix(&self.root) else { return };
        let rel = format!("/{}", rel.display());
        let cur = std::fs::read_to_string(file).unwrap_or_default();
        if cur.lines().any(|l| l.trim() == rel) {
            return;
        }
        let mut out = cur;
        if !out.is_empty() && !out.ends_with(NEWLINE_CH) {
            out.push(NEWLINE_CH);
        }
        out.push_str(&rel);
        out.push(NEWLINE_CH);
        let _ = std::fs::write(file, out);
    }

    /// Remove a ROM (by absolute path) from a collection.
    pub fn collection_remove(&self, file: &Path, rom_abs: &Path) {
        let Ok(rel) = rom_abs.strip_prefix(&self.root) else { return };
        let rel = format!("/{}", rel.display());
        if let Ok(cur) = std::fs::read_to_string(file) {
            let kept: Vec<&str> =
                cur.lines().filter(|l| l.trim() != rel).collect();
            let _ = std::fs::write(file, kept.join(NL) + NL);
        }
    }

    /// Boot logo BMPs shipped on the card.
    pub fn bootlogos(&self) -> Vec<PathBuf> {
        let dir = self.root.join(".system/tg5040/Bootlogo.pak/brick");
        let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.path())
                    .filter(|p| {
                        p.extension().map(|e| e.eq_ignore_ascii_case("bmp")).unwrap_or(false)
                    })
                    .collect()
            })
            .unwrap_or_default();
        // kUI Rusted (this generation's flagship) first, then kUI-family,
        // then the rest alphabetical
        v.sort_by_key(|p| {
            let name = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            (name != "kUI Rusted", !name.starts_with("kUI"), name.to_lowercase())
        });
        v
    }

    /// Theme variants (folders with carousel art).
    pub fn theme_variants(&self) -> Vec<(String, PathBuf)> {
        let dir = self.root.join(".system/themes/artbook");
        let mut v: Vec<(String, PathBuf)> = std::fs::read_dir(dir)
            .map(|rd| {
                rd.flatten()
                    .filter(|e| e.path().is_dir())
                    .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
                    .collect()
            })
            .unwrap_or_default();
        v.sort_by_key(|(n, _)| n.clone());
        v
    }

    /// Apply a theme variant: copy its PNGs over the carousel art.
    pub fn theme_apply(&self, variant: &Path) -> std::io::Result<()> {
        let dst = self.root.join(".system/res/carousel");
        for entry in std::fs::read_dir(variant)?.flatten() {
            let p = entry.path();
            if p.is_file() {
                std::fs::copy(&p, dst.join(entry.file_name()))?;
            } else if p.is_dir() {
                // logos/ subdir
                let sub = dst.join(entry.file_name());
                let _ = std::fs::create_dir_all(&sub);
                for e2 in std::fs::read_dir(&p)?.flatten() {
                    if e2.path().is_file() {
                        std::fs::copy(e2.path(), sub.join(e2.file_name()))?;
                    }
                }
            }
        }
        Ok(())
    }

    /// The "(TAG)" of the platform folder above a ROM path. Walks up so
    /// nested layouts (a disks/ subfolder, a per-game folder) still
    /// resolve to their platform.
    pub fn tag_of_rom(&self, rom_abs: &Path) -> Option<String> {
        let mut dir = rom_abs.parent();
        while let Some(d) = dir {
            if d == self.root {
                break;
            }
            if let Some(name) = d.file_name() {
                let (_, tag) = split_tag(&name.to_string_lossy());
                if !tag.is_empty() {
                    return Some(tag);
                }
            }
            dir = d.parent();
        }
        None
    }
}

/// Parsed `.media/<rom>.info` metadata sidecar (scraper output).
pub struct GameInfo {
    pub year: String,
    pub genre: String,
    pub developer: String,
    pub publisher: String,
}

impl GameInfo {
    /// The kUI footer line: year · publisher · genre (developer fills in
    /// for a missing publisher). Empty fields are skipped.
    pub fn footer(&self) -> String {
        let publisher = if self.publisher.is_empty() { &self.developer } else { &self.publisher };
        [self.year.as_str(), publisher, self.genre.as_str()]
            .iter()
            .filter(|s| !s.is_empty())
            .copied()
            .collect::<Vec<_>>()
            .join(" \u{b7} ")
    }
}

impl Sd {
    /// Boxart next to an absolute ROM path.
    pub fn boxart_for(&self, rom_abs: &Path) -> Option<PathBuf> {
        let dir = rom_abs.parent()?;
        let stem = rom_abs.file_stem()?.to_string_lossy();
        let p = dir.join(".media").join(format!("{stem}.png"));
        p.is_file().then_some(p)
    }

    /// Metadata next to an absolute ROM path.
    pub fn game_info_for(&self, rom_abs: &Path) -> Option<GameInfo> {
        let dir = rom_abs.parent()?;
        let stem = rom_abs.file_stem()?.to_string_lossy();
        let p = dir.join(".media").join(format!("{stem}.info"));
        parse_info(&p)
    }

    /// Folder background: per-folder `.media/bg.png`, else global `.media/bg.png`.
    pub fn folder_bg(&self, entry: &PlatformEntry) -> Option<PathBuf> {
        let per = entry.dir.join(".media/bg.png");
        if per.is_file() {
            return Some(per);
        }
        let global = self.root.join(".media/bg.png");
        global.is_file().then_some(global)
    }

    /// The OG font (BPreplay), with font1 as fallback.
    pub fn font(&self) -> Option<PathBuf> {
        for f in ["font2.ttf", "font1.ttf"] {
            let p = self.root.join(".system/res").join(f);
            if p.is_file() {
                return Some(p);
            }
        }
        None
    }
}

fn parse_info(p: &Path) -> Option<GameInfo> {
    let text = std::fs::read_to_string(p).ok()?;
    let mut info = GameInfo {
        year: String::new(),
        genre: String::new(),
        developer: String::new(),
        publisher: String::new(),
    };
    for line in text.lines() {
        if let Some((k, v)) = line.split_once('=') {
            let v = v.trim().to_string();
            match k.trim() {
                "year" => info.year = v,
                "genre" => info.genre = v,
                "developer" => info.developer = v,
                "publisher" => info.publisher = v,
                _ => {}
            }
        }
    }
    Some(info)
}

/// "Game Boy (GB)" -> ("Game Boy", "GB"). No tag -> name as-is, empty tag.
fn split_tag(folder: &str) -> (String, String) {
    if let Some(open) = folder.rfind('(')
        && let Some(close) = folder.rfind(')')
        && close > open
    {
        let display = folder[..open].trim().to_string();
        let tag = folder[open + 1..close].trim().to_uppercase();
        return (display, tag);
    }
    (folder.to_string(), String::new())
}

/// Art lookup key: lowercase, spaces -> underscores.
fn art_key(display: &str) -> String {
    display.to_lowercase().replace(' ', "_")
}

fn visible_files(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.path().is_file())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| !n.starts_with('.'))
                .collect()
        })
        .unwrap_or_default();
    v.sort_by_key(|a| a.to_lowercase());
    v
}
