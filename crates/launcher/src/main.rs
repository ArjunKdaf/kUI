//! kUI launcher: Carousel (The Dude + platforms; Recents and Collections
//! live in the quick menu), game lists, launching through the pak contract.
//!
//! Boot-loop contract (the card's legacy launch loop):
//!   exit 0 after writing /tmp/next  -> runner evals the command, relaunches us
//!   exit 0 without /tmp/next        -> runner relaunches us (fresh UI state)
//!   exit 66                         -> runner stops (dev: back to C launcher)

mod art;
mod dude;
mod fn_page;
mod hub;
mod portforge;
mod scraper;
mod sd;
mod ui;

use std::collections::HashMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::Instant;

use art::{Art, Loader};
use glow::HasContext as _;
use kui_gfx::text::Font;
use kui_gfx::{Renderer, WHITE};
use kui_hal::sdl::SdlVideo;
use kui_hal::tg5040;
use kui_hal::{Button, ButtonState, InputEvent};
use sd::Sd;
use ui::{Pill, Repeat};

const EXIT_QUIT: i32 = 66;
/// Artbook panel source is 454x1080; scale to screen height.
const PANEL_SRC: (f32, f32) = (454.0, 1080.0);
const DIM: f32 = 0.45;

const ROW_H: f32 = 64.0;
const PILL_H: u32 = 52;
/// How long the launch-failure toast stays up.
const TOAST_TIME: std::time::Duration = std::time::Duration::from_millis(2500);
const LIST_FONT: u32 = 30;
const META_FONT: u32 = 22;

// art loader kinds
const K_BG: u32 = 0;
const K_LOGO: u32 = 1;
const K_BOX: u32 = 2;
const K_FBG: u32 = 3;
/// fbg cache key for the root list background (outside tile index range).
const ROOT_FBG: usize = 0xFFFF;


/// Sprite rects for the asset sheet (1x coords; sheet is @2x, 256px).
/// (x, y, w, h)
const A_BATTERY: (f32, f32, f32, f32) = (47.0, 51.0, 17.0, 10.0);
const A_BATTERY_FILL: (f32, f32, f32, f32) = (81.0, 33.0, 12.0, 6.0);
/// Full battery-body overlay with a lightning-bolt knockout (charging).
const A_BATTERY_BOLT: (f32, f32, f32, f32) = (91.0, 51.0, 16.0, 10.0);
const A_WIFI: (f32, f32, f32, f32) = (1.0, 104.0, 12.0, 12.0);
const A_BRIGHTNESS: (f32, f32, f32, f32) = (1.0, 85.0, 19.0, 19.0);
const A_VOLUME: (f32, f32, f32, f32) = (21.0, 85.0, 19.0, 19.0);
const A_BLUETOOTH: (f32, f32, f32, f32) = (53.0, 104.0, 12.0, 12.0);
const SHEET: f32 = 256.0;

fn asset_uv(rect: (f32, f32, f32, f32)) -> [f32; 4] {
    [
        rect.0 * 2.0 / SHEET,
        rect.1 * 2.0 / SHEET,
        rect.2 * 2.0 / SHEET,
        rect.3 * 2.0 / SHEET,
    ]
}

/// Top-right tray data, refreshed lazily. Battery always; WiFi/BT only
/// when the radio is up (no clock — Arjun's call).
struct Status {
    batt: Option<u8>,
    charging: bool,
    wifi: bool,
    bt: bool,
    refreshed: Instant,
}

impl Status {
    fn new() -> Self {
        Self {
            batt: None,
            charging: false,
            wifi: false,
            bt: false,
            refreshed: Instant::now() - std::time::Duration::from_secs(60),
        }
    }

    fn refresh(&mut self, on_device: bool) {
        if self.refreshed.elapsed() < std::time::Duration::from_secs(10) {
            return;
        }
        self.refreshed = Instant::now();
        if on_device {
            self.batt = std::fs::read_to_string("/tmp/percBat")
                .ok()
                .and_then(|s| s.trim().parse::<u8>().ok());
            self.charging =
                std::fs::read_to_string("/sys/class/power_supply/axp2202-usb/online")
                    .map(|s| s.trim() == "1")
                    .unwrap_or(false);
            self.wifi = proc_running("wpa_supplicant");
            self.bt = proc_running("bluetoothd");
        }
    }

}

/// UI mode. Named for what the modes show (SPEC decision).
#[derive(Clone, Copy, PartialEq, Eq)]
enum UiMode {
    Lists,
    Covers,
    Carousel,
}

impl UiMode {
    fn from_config(cfg: &kui_config::Config) -> Self {
        match cfg.get_or("ui.mode", "carousel") {
            "lists" => UiMode::Lists,
            "covers" => UiMode::Covers,
            _ => UiMode::Carousel,
        }
    }
}

/// A carousel tile.
enum Tile {
    Dude,
    Platform(usize),
}

impl Tile {
    /// (special art key, display-name fallback)
    fn art_key(&self, platforms: &[sd::PlatformEntry]) -> (String, String) {
        match self {
            Tile::Dude => ("the_dude".into(), "The Dude".into()),
            Tile::Platform(i) => (String::new(), platforms[*i].display.clone()),
        }
    }
}

/// One row of the (generalized) list screen.
struct Row {
    label: String,
    action: RowAction,
}

enum RowAction {
    Launch(PathBuf),
    /// Launch a random game from the surrounding list. Rendered as a
    /// permanently pinned first row; not pinnable/wipeable itself.
    LaunchRandom,
    OpenCollection(PathBuf),
    OpenSmartCollection(String),
    OpenTile(usize),
    PickPlatform(usize),
    PickGame(PathBuf),
    NewCollection,
    OpenPaks,
    LaunchPak(PathBuf),
}

/// What produced the current list (drives platform-hop and back behavior).
enum ListKind {
    Root,
    Platform(usize),
    Recents,
    CollectionsIndex,
    Collection(PathBuf),
    /// A built-in franchise collection's game list (identity lives in the
    /// rows; back returns to the index, so no key is needed here).
    SmartCollection,
    PickPlatform(PathBuf),
    PickGame(PathBuf),
    /// Installed paks, shown as the "Paks" collection.
    Paks,
}

#[derive(Clone)]
struct CoreEntry {
    label: String,
    stem: String,
    path: PathBuf,
    tags: Vec<String>,
}

enum OskTarget {
    Collection,
    CollectionRename { dir: PathBuf },
    WifiPass { ssid: String },
    /// Writes the buffer to this config key, then returns to the page.
    ConfigValue { key: String, page: usize, row: usize },
    FileRename { path: PathBuf, dirs: [PathBuf; 2], active: usize },
    NewFolder { dirs: [PathBuf; 2], active: usize },
}

struct BtDev {
    mac: String,
    name: String,
    paired: bool,
    connected: bool,
}

struct WifiNet {
    ssid: String,
    signal: i32,
    secured: bool,
    saved: Option<i32>,
    current: bool,
}

#[derive(Clone, Copy)]
enum PickBack {
    HubPage(usize),
    Led,
}

enum Screen {
    ColorPick { key: String, back: PickBack, rgb: [i32; 3], orig: [i32; 3], channel: usize },
    HubIndex { selected: usize },
    LedEditor { row: usize, profile: usize, light: usize },
    CoreList { cores: Vec<CoreEntry>, selected: usize, scroll: usize },
    CoreOpts {
        core: String,
        tags: Vec<String>,
        defs: Vec<kui_libretro::VarDef>,
        selected: usize,
        scroll: usize,
        list: Vec<CoreEntry>,
        list_pos: (usize, usize),
    },
    Osk { buf: String, pos: usize, target: OskTarget },
    Wifi { nets: Vec<WifiNet>, selected: usize, scroll: usize },
    Bt { devs: Vec<BtDev>, selected: usize, scroll: usize },
    /// (name, "total · plays · avg"), plus the library-wide header line.
    GameTime { rows: Vec<(String, String)>, header: String, selected: usize, scroll: usize },
    /// Battery history graph; span in hours (6/12/24/48).
    Battery { samples: Vec<(u64, i32, bool)>, span_h: u64 },
    /// Live button tester; exit by holding MENU.
    InputTest,
    Files {
        panes: [FilePane; 2],
        active: usize,
        /// Some(row) = the START action menu is open on that action index.
        menu: Option<usize>,
        armed_delete: bool,
    },
    /// Port Forge: browse (folders only) to an extracted RPG Maker game
    /// folder. Selecting a detected game hands off to PortForgeRun; a plain
    /// folder just descends.
    PortForge { pane: FilePane },
    /// Port Forge is building the port from `source` (a game folder). Shows
    /// progress, then a delete-original prompt (Y delete / B keep). Either
    /// way it exits to the Control Panel — never back to the picker.
    PortForgeRun { source: PathBuf },
    /// (label, platform dir; None = all platforms)
    ScraperPlatforms { rows: Vec<(String, Option<PathBuf>)>, selected: usize, scroll: usize },
    /// (label, Some(tag) = one platform, None = all); ret = hub (page, item)
    CheatPlatforms {
        rows: Vec<(String, Option<String>)>,
        selected: usize,
        scroll: usize,
        ret: (usize, usize),
    },
    /// same picker for RA PreFetch
    PrefetchPlatforms {
        rows: Vec<(String, Option<String>)>,
        selected: usize,
        scroll: usize,
        ret: (usize, usize),
    },
    ScraperMenu { label: String, dir: Option<PathBuf>, selected: usize },
    PakCats { cats: Vec<(String, usize, usize)>, selected: usize, scroll: usize },
    PakDek {
        paks: Vec<kui_store::Pak>,
        title: String,
        selected: usize,
        scroll: usize,
        cat_sel: usize,
    },
    PortCats {
        cats: Vec<(String, usize)>,
        selected: usize,
        scroll: usize,
        // scoped to ready-to-play ports (the Ready to Play sub-level)
        rtr: bool,
    },
    Ports {
        ports: Vec<kui_store::ports::PortEntry>,
        title: String,
        selected: usize,
        scroll: usize,
        cat_sel: usize,
        // opened from the Ready to Play sub-level (back returns there)
        rtr: bool,
    },
    Updater { releases: Vec<kui_store::Release>, selected: usize },
    ScraperRun { job: scraper::Scraper, label: String, dir: Option<PathBuf>, menu_sel: usize },
    BootLogo { idx: usize },
    Themes { idx: usize },
    HubPage { page: usize, selected: usize },
    Carousel,
    List {
        kind: ListKind,
        rows: Vec<Row>,
        selected: usize,
        scroll: usize,
        show_art: bool,
        tag: Option<String>,
    },
    Dude,
    Quick {
        items: Vec<QuickItem>,
        selected: usize,
        prev: Box<Screen>,
        probed: Instant,
        /// In-flight radio change: (is_wifi, target_on, deadline).
        pending: Option<(bool, bool, Instant)>,
    },
    /// Horizontal recents carousel; A resumes, Y removes.
    Switcher { entries: Vec<sd::Recent>, idx: usize, prev: Box<Screen> },
}

struct QuickItem {
    label: String,
    /// Optional accent-highlighted value drawn after the label (e.g. the
    /// radio state "On"/"Off"/"...").
    value: Option<String>,
    desc: &'static str,
    action: QuickAction,
}

#[derive(Clone, Copy, PartialEq)]
enum QuickAction {
    Recents,
    Collections,
    Settings,
    Wifi,
    Bluetooth,
    Reboot,
    Poweroff,
}

const OSK_CHARS: &str =
    "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789 -_.!@#$%&*+=?";
const OSK_COLS: usize = 13;

use tg5040::leds::{
    LIGHTS as LED_LIGHTS, PROFILES as LED_PROFILES, apply_light as led_apply,
    apply_profile as led_apply_profile, profile_get as led_get, profile_key as led_key,
};

/// Effective defaults for the theme color keys (what Theme::from_config
/// falls back to) — settings rows must show reality, not "000000".
fn theme_color_default(key: &str) -> u32 {
    match key {
        "theme.color1" => 0x00FF55,
        "theme.color2" => 0xB3FFB3,
        "theme.color4" => 0x505050,
        // hints/descriptions/headers: white by default (Arjun's call)
        "theme.color6" => 0xFFFFFF,
        // notification text: black by default (Arjun's call)
        "theme.color8" => 0x000000,
        _ => 0x000000,
    }
}

/// Brick factory white-point gain per channel (CONTRACT: on, R100 G92 B58).
fn cal_gain_default(ch: &str) -> i32 {
    match ch {
        "r" => 100,
        "g" => 92,
        _ => 58,
    }
}

/// Push the cfg's white-point calibration to the panel.
fn apply_displaycal_cfg(cfg: &kui_config::Config) {
    tg5040::apply_displaycal(
        cfg.get_or("cal.enabled", "on") == "on",
        cfg.get_i32("cal.gain.r", 100).clamp(0, 200) as u32,
        cfg.get_i32("cal.gain.g", 92).clamp(0, 200) as u32,
        cfg.get_i32("cal.gain.b", 58).clamp(0, 200) as u32,
    );
}

/// Mirror a value we just applied into kuid's live state dir, so the
/// OSDs (ours and the frontend's) track it (CONTRACT /tmp/kui).
fn write_live_state(name: &str, val: i32) {
    let _ = std::fs::create_dir_all("/tmp/kui");
    let _ = std::fs::write(format!("/tmp/kui/{name}"), val.to_string());
}

/// Is the headphone/BT/USB volume slot the live audio route? kuid's sink
/// file covers BT/USB; the jack switch covers wired headphones.
fn headphone_active() -> bool {
    let sink = std::fs::read_to_string("/tmp/kui/sink");
    if matches!(&sink, Ok(s) if s.trim() != "0") {
        return true;
    }
    tg5040::switch_states().1.unwrap_or(false)
}

/// SPEC services: governor — shared with the frontend via the HAL. The
/// frontend re-asserts the per-game profile after session.sh forces
/// performance around every launch; the launcher asserts the global one.
fn apply_power_profile(profile: &str) {
    tg5040::apply_power_profile(profile);
}

/// Is a process running on the device (pidof)?
fn proc_running(name: &str) -> bool {
    std::process::Command::new("sh")
        .args(["-c", &format!("pidof {name} >/dev/null 2>&1")])
        .status()
        .map(|st| st.success())
        .unwrap_or(false)
}

/// Quick menu re-opened on top of `prev`, cursor on the entry the user
/// is returning from — Recents/Collections/Control Panel live in the
/// quick menu, so leaving them lands back there, not on the carousel.
fn quick_return(
    sd: &Sd,
    on_device: bool,
    now: std::time::Instant,
    prev: Screen,
    want: QuickAction,
) -> Screen {
    let items = quick_items(sd, on_device);
    let selected = items.iter().position(|it| it.action == want).unwrap_or(0);
    Screen::Quick { items, selected, prev: Box::new(prev), probed: now, pending: None }
}

fn quick_items(sd: &Sd, on_device: bool) -> Vec<QuickItem> {
    let mut v = Vec::new();
    if !sd.recents().is_empty() {
        v.push(QuickItem {
            label: "Recents".into(),
            value: None,
            desc: "Recently played games",
            action: QuickAction::Recents,
        });
    }
    // Collections live only here in the quick menu, so the entry always
    // shows — it is the sole way in to create or browse them.
    v.push(QuickItem {
        label: "Collections".into(),
        value: None,
        desc: "Your game collections",
        action: QuickAction::Collections,
    });
    v.push(QuickItem {
        label: "Control Panel".into(),
        value: None,
        desc: "Every setting and tool in one place",
        action: QuickAction::Settings,
    });
    let wifi_on = on_device && proc_running("wpa_supplicant");
    let bt_on = on_device && proc_running("bluetoothd");
    v.push(QuickItem {
        label: "WiFi".into(),
        value: Some(if wifi_on { "On" } else { "Off" }.into()),
        desc: "Takes a few seconds to change.",
        action: QuickAction::Wifi,
    });
    v.push(QuickItem {
        label: "Bluetooth".into(),
        value: Some(if bt_on { "On" } else { "Off" }.into()),
        desc: "Takes a few seconds to change.",
        action: QuickAction::Bluetooth,
    });
    v.push(QuickItem { label: "Reboot".into(), value: None, desc: "Restart the device", action: QuickAction::Reboot });
    v.push(QuickItem { label: "Poweroff".into(), value: None, desc: "Shut down safely", action: QuickAction::Poweroff });
    v
}

fn main() {
    // hidden mode: enumerate a core's options in a disposable process
    // (a crashing core must never take the launcher down)
    let args: Vec<String> = std::env::args().collect();
    if args.len() == 4 && args[1] == "--enum-core" {
        match kui_libretro::enumerate_options(
            std::path::Path::new(&args[2]),
            std::path::Path::new(&args[3]),
        ) {
            Ok(defs) => {
                for d in defs {
                    println!("{}\x1f{}\x1f{}", d.key, d.desc, d.choices.join("|"));
                }
                std::process::exit(0);
            }
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
    }
    std::process::exit(run());
}

/// Enumerate core options via a child process; a core crash costs a
/// child, not the launcher — and a core that hangs in retro_init costs
/// 5 seconds and a kill, not the whole device (the power button is
/// serviced by the event loop this call blocks).
fn enumerate_core_safely(
    path: &std::path::Path,
    system_dir: &std::path::Path,
) -> Result<Vec<kui_libretro::VarDef>, String> {
    use std::io::Read;
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut child = std::process::Command::new(me)
        .args([
            "--enum-core",
            &path.display().to_string(),
            &system_dir.display().to_string(),
        ])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    // drain on threads: a chatty core (pcsx_rearmed logs a lot) must not
    // fill the 64KB pipe and wedge against our try_wait loop
    let mut c_out = child.stdout.take().unwrap();
    let mut c_err = child.stderr.take().unwrap();
    let out_h = std::thread::spawn(move || {
        let mut v = Vec::new();
        let _ = c_out.read_to_end(&mut v);
        v
    });
    let err_h = std::thread::spawn(move || {
        let mut v = Vec::new();
        let _ = c_err.read_to_end(&mut v);
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let status = loop {
        match child.try_wait().map_err(|e| e.to_string())? {
            Some(s) => break s,
            None if std::time::Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = out_h.join();
                let _ = err_h.join();
                return Err("core enumeration timed out".into());
            }
            None => std::thread::sleep(std::time::Duration::from_millis(30)),
        }
    };
    let stdout = out_h.join().unwrap_or_default();
    let _ = err_h.join();
    if !status.success() {
        return Err(format!("core enumeration failed ({status})"));
    }
    let text = String::from_utf8_lossy(&stdout);
    Ok(text
        .lines()
        .filter_map(|l| {
            let mut parts = l.split('\x1f');
            let key = parts.next()?.to_string();
            let desc = parts.next().unwrap_or("").to_string();
            let choices =
                parts.next().unwrap_or("").split('|').map(|s| s.to_string()).collect();
            Some(kui_libretro::VarDef { key, desc, choices })
        })
        .collect())
}

fn run() -> i32 {
    let on_device = std::env::var("DEVICE").is_ok();
    let sd_root = std::env::var("SDCARD_PATH").unwrap_or_else(|_| {
        if on_device { "/mnt/SDCARD".into() } else { "./sdcard".into() }
    });
    let sd = Sd::new(&sd_root);

    let mut platforms = sd.scan_platforms();
    // pak contract: boot hooks once per real boot (/tmp clears on reboot)
    if !std::path::Path::new("/tmp/kui_boot_hooks_done").exists() {
        let _ = std::fs::write("/tmp/kui_boot_hooks_done", "");
        run_hooks("boot.d", &[]);
    }
    // close the previous play session: /tmp/kui_session -> playlog line
    if let Ok(stamp) = std::fs::read_to_string("/tmp/kui_session") {
        let _ = std::fs::remove_file("/tmp/kui_session");
        let mut it = stamp.trim().splitn(3, '\t');
        if let (Some(start), Some(rel), Some(alias)) = (it.next(), it.next(), it.next())
            && let Ok(start) = start.parse::<u64>()
            && let Ok(now) = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
        {
            // clamp guards clock jumps; sessions under a minute don't count
            run_hooks(
                "post-launch.d",
                &[
                    ("HOOK_PHASE", "post".to_string()),
                    ("HOOK_TYPE", "rom".to_string()),
                    ("HOOK_ROM_PATH", rel.to_string()),
                ],
            );
            let secs = now.as_secs().saturating_sub(start).min(12 * 3600);
            if secs >= 60 {
                let log = sd.root.join(".userdata/shared/kui/playlog.txt");
                let mut lines: Vec<String> = std::fs::read_to_string(&log)
                    .map(|t| t.lines().map(str::to_string).collect())
                    .unwrap_or_default();
                lines.push(format!("{start}\t{secs}\t{rel}\t{alias}"));
                let keep = lines.len().saturating_sub(5000);
                let _ = std::fs::write(&log, lines[keep..].join("\n") + "\n");
            }
        }
    }
    let fe_cfg = kui_config::Config::load(&sd.root.join(".userdata/shared"));
    {
        let off = fe_cfg.get_i32("tz.utc", 99);
        if (-12..=14).contains(&off) {
            // POSIX sign is inverted: local UTC-3 is TZ=UTC+3
            unsafe { std::env::set_var("TZ", format!("UTC{:+}", -off)) };
        }
    }
    retain_launchable(&mut platforms, &sd, &fe_cfg);

    if platforms.is_empty() {
        eprintln!("no launchable platforms under {sd_root}/Roms");
    }

    let mut v = match SdlVideo::new("kUI", on_device) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("video init failed: {e}");
            return EXIT_QUIT;
        }
    };
    let joy = v.sdl.joystick().ok();
    let _sticks: Vec<_> = joy
        .map(|j| {
            (0..j.num_joysticks().unwrap_or(0))
                .filter_map(|i| j.open(i).ok())
                .collect()
        })
        .unwrap_or_default();

    let mut r = match Renderer::new(&v.gl) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("renderer init failed: {e}");
            return EXIT_QUIT;
        }
    };
    let boot_cfg = kui_config::Config::load(&sd.root.join(".userdata/shared"));
    let mut font = {
        let p = resolve_font(&sd.root.join(".system/res"), boot_cfg.get_or("theme.font", "0"));
        Font::load(&v.gl, &p).ok().or_else(|| sd.font().and_then(|p| Font::load(&v.gl, &p).ok()))
    };
    if let Some(f) = font.as_mut() {
        f.set_bold(boot_cfg.get_or("theme.font_style", "normal") == "bold");
    }
    if font.is_none() {
        eprintln!("no font found under {sd_root}/.system/res — text disabled");
    }
    let pill_inner = match Pill::new(&v.gl, PILL_H - 16) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("pill init failed: {e}");
            return EXIT_QUIT;
        }
    };
    let pill = match Pill::new(&v.gl, PILL_H) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("pill init failed: {e}");
            return EXIT_QUIT;
        }
    };
    let mut status = Status::new();
    // legacy import (from a migrated 0.81a card) is handled once by the
    // bridge migration; the launcher reads only OUR config.
    let mut cfg = kui_config::Config::load(&sd.root.join(".userdata/shared"));
    let build_tiles = |nplat: usize| -> (Vec<Tile>, usize) {
        let mut tiles: Vec<Tile> = vec![Tile::Dude];
        let anchor = tiles.len().saturating_sub(1);
        for i in 0..nplat {
            tiles.push(Tile::Platform(i));
        }
        (tiles, anchor)
    };
    let (mut tiles, mut dude_tile) = build_tiles(platforms.len());
    let mut n = tiles.len();
    let mut theme = sd::Theme::from_config(&cfg);
    let stock_ver = std::fs::read_to_string("/etc/version")
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "unknown".into());
    let busybox_ver = std::process::Command::new("sh")
        .args(["-c", "strings /bin/busybox 2>/dev/null | grep -m1 -o 'BusyBox v[0-9.]*'"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".into());
    let hub_pages = hub::pages(
        if on_device { "TrimUI Brick" } else { "desktop" },
        &stock_ver,
        &busybox_ver,
    );
    if on_device {
        // apply persisted startup settings
        tg5040::set_colortemp(cfg.get_i32("display.colortemp", 20));
        tg5040::set_contrast(cfg.get_i32("display.contrast", 0));
        tg5040::set_saturation(cfg.get_i32("display.saturation", 0));
        tg5040::set_exposure(cfg.get_i32("display.exposure", 0));
        let vol_key = if headphone_active() { "audio.volume_hp" } else { "audio.volume" };
        tg5040::set_volume_percent(cfg.get_i32(vol_key, 40));
        led_apply_profile(&cfg, 0);
        apply_power_profile(cfg.get_or("power.profile", "auto"));
        if cfg.get_or("dev.ssh_on_boot", "off") == "on" {
            let _ = std::process::Command::new("sh")
                .args([
                    "-c",
                    "(/etc/init.d/sshd start || /etc/init.d/S50sshd start) >/dev/null 2>&1 &",
                ])
                .spawn();
        }
    }
    let assets_tex = kui_gfx::load_png(&v.gl, &sd.root.join(".system/res/assets@2x.png"))
        .map_err(|e| eprintln!("assets sheet: {e}"))
        .ok();
    let menu_icon = kui_gfx::decode_png_bytes(include_bytes!("../assets/trimui_menu.png"))
        .ok()
        .and_then(|(w, h, px)| kui_gfx::texture_from_rgba(&v.gl, w, h, &px).ok());
    let select_icon = kui_gfx::decode_png_bytes(include_bytes!("../assets/select.png"))
        .ok()
        .and_then(|(w, h, px)| kui_gfx::texture_from_rgba(&v.gl, w, h, &px).ok());
    let start_icon = kui_gfx::decode_png_bytes(include_bytes!("../assets/start.png"))
        .ok()
        .and_then(|(w, h, px)| kui_gfx::texture_from_rgba(&v.gl, w, h, &px).ok());

    // All carousel art resident for the session; one worker decodes async.
    // Decode order radiates out from the tile we'll land on, so a return
    // from a game paints its neighborhood first (no placeholder pop).
    let land_tile = std::fs::read_to_string("/tmp/kui_last.txt")
        .ok()
        .map(|l| std::path::PathBuf::from(l.trim()))
        .and_then(|last| {
            let dir = last.parent()?.to_path_buf();
            let pi = platforms.iter().position(|p| p.dir == dir)?;
            tiles.iter().position(|t| matches!(t, Tile::Platform(x) if *x == pi))
        })
        .unwrap_or_else(|| {
            tiles.iter().position(|t| matches!(t, Tile::Dude)).unwrap_or(0)
        });
    let loader = Loader::new(2);
    let mut bg: HashMap<usize, Art> = HashMap::new();
    let mut logo: HashMap<usize, Art> = HashMap::new();
    request_carousel_art(&loader, &mut bg, &mut logo, &sd, &tiles, &platforms, land_tile);
    let mut fbg: HashMap<usize, Art> = HashMap::new();
    // Root-list background (Covers mode): global bg, else the stock wallpaper.
    {
        // no branded fallback wallpaper: a real bg.png or plain black
        let global = sd.root.join(".media/bg.png");
        let pick = global.is_file().then_some(global);
        fbg.insert(ROOT_FBG, match pick {
            Some(p) => {
                loader.request(art::key(K_FBG, ROOT_FBG), p);
                Art::Pending
            }
            None => Art::Missing,
        });
    }
    let mut boxart: HashMap<usize, Art> = HashMap::new();
    let mut infos: HashMap<usize, Option<String>> = HashMap::new();

    let mut ui_mode = UiMode::from_config(&cfg);
    // Cold boot centers on The Dude.
    let mut tile: usize = dude_tile;
    let mut screen = if ui_mode == UiMode::Carousel {
        Screen::Carousel
    } else {
        open_root_list(&platforms, &tiles, dude_tile)
    };
    // Returning from a game (same boot): land back in that platform's
    // list with the cursor on the game. /tmp clears on reboot, so cold
    // boots still open at The Dude.
    let mut remember: HashMap<usize, (usize, usize)> = HashMap::new();
    let mut returned_from_game = false;
    // NOUI mode: a .noui file on the card boots straight into the last
    // game; the frontend's Quit powers off instead of returning here.
    if on_device
        && sd.root.join(".noui").is_file()
        && !std::path::Path::new("/tmp/kui_noui_done").exists()
        && let Ok(last) = std::fs::read_to_string("/tmp/kui_last.txt")
            .or_else(|_| std::fs::read_to_string(sd.root.join(".userdata/shared/kui/last.txt")))
    {
        let _ = std::fs::write("/tmp/kui_noui_done", "");
        let rom = PathBuf::from(last.trim());
        if rom.is_file() {
            let label = rom
                .file_stem()
                .map(|s2| s2.to_string_lossy().into_owned())
                .unwrap_or_default();
            if let LaunchResult::Started(code) = launch_rom(&sd, &fe_cfg, &rom, &label, on_device) {
                return code;
            }
        }
    }
    if let Ok(last) = std::fs::read_to_string("/tmp/kui_last.txt") {
        let _ = std::fs::remove_file("/tmp/kui_last.txt");
        let last = std::path::PathBuf::from(last.trim());
        if last.is_file()
            && let Some(dir) = last.parent()
            && let Some(pi) = platforms.iter().position(|p| p.dir == dir)
            && let Some(t) = tiles
                .iter()
                .position(|t| matches!(t, Tile::Platform(x) if *x == pi))
        {
            // land on ROOT at the platform; the game list is one press away
            // with the cursor pre-seeded on the game
            tile = t;
            returned_from_game = true;
            screen = root_screen(ui_mode, &platforms, &tiles, tile);
            // seed the list cursor without open_tile_mode (its art/bg
            // requests are for a real list screen, not the carousel)
            let p = &platforms[pi];
            let mut names: Vec<(bool, String, PathBuf)> = p
                .roms
                .iter()
                .map(|rom| {
                    let abs = p.dir.join(rom);
                    (sd.is_pinned(&abs), clean_name(rom), abs)
                })
                .collect();
            names.sort_by_key(|(pinned, name, _)| (!*pinned, name.to_lowercase()));
            if let Some(i) = names.iter().position(|(_, _, abs)| *abs == last) {
                // +1: the Random row sits above every game
                let i = i + 1;
                remember.insert(
                    tile,
                    (i, i.saturating_sub(5).min((names.len() + 1).saturating_sub(11))),
                );
            }
        }
    }
    // Landing straight on the carousel after a game: hold the black
    // transition briefly until the visible window is dressed — reads as
    // part of the exit fade, never as loading. Overlay returns are
    // curtained and skip this.
    if returned_from_game
        && matches!(screen, Screen::Carousel | Screen::List { .. })
    {
        let t0 = Instant::now();
        while t0.elapsed().as_millis() < 800 {
            while let Some((k, res)) = loader.try_recv() {
                let (kind, i) = art::split(k);
                let map = match kind {
                    K_BG => &mut bg,
                    K_LOGO => &mut logo,
                    _ => &mut fbg,
                };
                if let Some(slot @ Art::Pending) = map.get_mut(&i) {
                    *slot = match res
                        .and_then(|(w, h, px)| kui_gfx::texture_from_rgba(&v.gl, w, h, &px))
                    {
                        Ok(t) => Art::Ready(t),
                        Err(_) => Art::Missing,
                    };
                }
            }
            let n_tiles = tiles.len().max(1);
            let ready = (-3i32..=3).all(|d| {
                let i = wrap(tile, d, n_tiles);
                !matches!(bg.get(&i), Some(Art::Pending))
                    && !matches!(logo.get(&i), Some(Art::Pending))
            });
            if ready {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
    let mut roll_state: HashMap<String, (String, Instant)> = HashMap::new();
    let mut switcher_art: HashMap<usize, Option<kui_gfx::Texture>> = HashMap::new();
    let mut dude_state: Option<dude::Dude> = None;
    let mut wifi_scan_at: Option<Instant> = None;
    let mut hub_scroll = 0usize;
    // Control Panel index: grouped, separators are render-only
    let hub_rows: Vec<HubRow> = build_hub_rows(&hub_pages);
    let hub_order: Vec<usize> = hub_rows
        .iter()
        .filter_map(|r2| match r2 {
            HubRow::Page(i) => Some(*i),
            HubRow::Header(_) => None,
        })
        .collect();
    let mut hub_page_scroll = 0usize;
    let mut bt_scan_at: Option<Instant> = None;
    let mut ra_auth_msg: Option<String> = None;
    let mut sc_cap: Option<usize> = None;
    let mut input_pressed = [false; INPUT_LABELS.len()];
    let mut input_fn: Option<bool> = None;
    let mut input_jack: Option<bool> = None;
    let mut input_sw_at: Option<std::time::Instant> = None;
    let mut input_menu_hold: Option<Instant> = None;
    let mut file_clip: Option<(PathBuf, bool)> = None; // (path, cut)
    let mut sc_captured: Option<String> = None;
    // RA prefetch worker: (done, total, last message, finished)
    let ra_pf: std::sync::Arc<std::sync::Mutex<(usize, usize, String, bool)>> =
        std::sync::Arc::new(std::sync::Mutex::new((0, 0, String::new(), true)));
    let ra_pf_cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    // cheat-pack worker, same shape
    let cheat_pf: std::sync::Arc<std::sync::Mutex<(usize, usize, String, bool)>> =
        std::sync::Arc::new(std::sync::Mutex::new((0, 0, String::new(), true)));
    let cheat_pf_cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    // shared status line for store/updater workers ("" = idle)
    let store_msg: std::sync::Arc<std::sync::Mutex<String>> =
        std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    type Fetched<T> = std::sync::Arc<std::sync::Mutex<Option<Result<T, String>>>>;
    let pak_fetch: Fetched<Vec<kui_store::Pak>> =
        std::sync::Arc::new(std::sync::Mutex::new(None));
    let ports_fetch: Fetched<kui_store::ports::Catalog> =
        std::sync::Arc::new(std::sync::Mutex::new(None));
    let mut ports_all = kui_store::ports::Catalog::default();
    // any install/remove happened: rescan the library on ports exit
    let mut ports_dirty = false;
    // Port Forge sets this when a forge succeeds; leaving the screen then
    // rescans platforms so the new port shows on the carousel without a reboot.
    let forge_dirty =
        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    // After a successful forge, the source the user picked (folder or .zip)
    // is a leftover duplicate — Port Forge offers to delete it to reclaim
    // space (X on the Done screen). This holds that path until acted on.
    let forge_del: std::sync::Arc<std::sync::Mutex<Option<std::path::PathBuf>>> =
        std::sync::Arc::new(std::sync::Mutex::new(None));
    // Forge progress-bar fraction (0.0–1.0), driven by bytes copied.
    let forge_pct: std::sync::Arc<std::sync::Mutex<f32>> =
        std::sync::Arc::new(std::sync::Mutex::new(0.0));
    // Set true when the async delete-original thread finishes, so the run
    // screen leaves to the Control Panel without freezing the UI on the
    // remove_dir_all (thousands of files on FAT32 = seconds).
    let forge_del_done =
        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    // In-flight game/port wipes. The heavy delete (port payload = hundreds
    // of MB / thousands of files) runs on a thread so the list never freezes;
    // the row is dropped instantly. Non-zero → the Y hint reads "Wiping…".
    let wiping = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    // Files browser paste/delete: recursive copy/remove of a whole folder is
    // slow, so it runs on a thread too. `files_busy` > 0 shows the verb
    // (Pasting…/Deleting…) as the screen note; `files_dirty` triggers a pane
    // refresh once the op finishes (the changed entry appears/vanishes then).
    let files_busy = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let files_verb: std::sync::Arc<std::sync::Mutex<&'static str>> =
        std::sync::Arc::new(std::sync::Mutex::new(""));
    let files_dirty = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    // Ports install/uninstall queue. A / uninstall enqueue a job and return
    // instantly; one background worker drains the channel FIFO so the cursor
    // never blocks. `port_jobs` maps zip_name -> row status ("Queued" /
    // "Installing…" / "Removing…"); the entry is dropped when the job ends.
    // A removed port keeps its Installed-view row (labelled "Removed") until
    // the category is re-entered, so the list never reflows under the cursor.
    enum PortJob {
        Install(kui_store::ports::PortEntry, kui_store::ports::Catalog),
        Remove(kui_store::ports::PortEntry),
    }
    let port_jobs: std::sync::Arc<
        std::sync::Mutex<std::collections::HashMap<String, String>>,
    > = std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));
    let (job_tx, job_rx) = std::sync::mpsc::channel::<PortJob>();
    {
        let jobs = port_jobs.clone();
        let hdr = store_msg.clone();
        let root = sd.root.clone();
        std::thread::spawn(move || {
            for job in job_rx {
                let (zip, title, installing) = match &job {
                    PortJob::Install(p, _) => (p.zip_name.clone(), p.title.clone(), true),
                    PortJob::Remove(p) => (p.zip_name.clone(), p.title.clone(), false),
                };
                if let Ok(mut m) = jobs.lock() {
                    m.insert(
                        zip.clone(),
                        if installing { "Installing…" } else { "Removing…" }.into(),
                    );
                }
                let res = match job {
                    PortJob::Install(p, cat) => kui_store::ports::install_port(
                        &root,
                        &p,
                        &cat,
                        &mut |st| {
                            if let Ok(mut m) = hdr.lock() {
                                *m = st;
                            }
                        },
                    ),
                    PortJob::Remove(p) => kui_store::ports::remove_port(&root, &p),
                };
                if let Ok(mut m) = jobs.lock() {
                    m.remove(&zip);
                }
                if let Ok(mut m) = hdr.lock() {
                    *m = match res {
                        Ok(()) => format!(
                            "Done — {title} {}",
                            if installing { "installed" } else { "removed" }
                        ),
                        Err(e) => format!(
                            "{} failed: {e}",
                            if installing { "Install" } else { "Remove" }
                        ),
                    };
                }
            }
        });
    }
    // (pak id, version) of a just-finished install: the worker resolves the
    // LATEST release, which can differ from the version pinned in the list.
    let pak_installed: std::sync::Arc<std::sync::Mutex<Option<(String, String)>>> =
        std::sync::Arc::new(std::sync::Mutex::new(None));
    let mut pak_all: Vec<kui_store::Pak> = Vec::new();
    let rel_fetch: Fetched<Vec<kui_store::Release>> =
        std::sync::Arc::new(std::sync::Mutex::new(None));
    let mut dude_menu = 0usize;
    let mut dude_sel = 0usize;
    let mut dude_text = String::new();
    let mut dude_armed = false;
    // where a START-press opened The Dude from, so leaving returns there
    // (like the switcher/quick menu). None => entered from root/quest.
    let mut dude_prev: Option<Box<Screen>> = None;
    // returning from a quest launch lands back on The Dude — and its
    // back-exit goes to the root centered on the Dude tile
    if std::fs::remove_file("/tmp/kui_dude").is_ok() {
        if let Some(t) = tiles.iter().position(|t| matches!(t, Tile::Dude)) {
            tile = t;
        }
        screen = Screen::Dude;
    }
    // the frontend's MENU+SELECT chord lands here: open the switcher directly
    if std::fs::remove_file("/tmp/kui_switcher").is_ok() {
        let entries = sd.recents();
        if !entries.is_empty() {
            let prev = std::mem::replace(&mut screen, Screen::Carousel);
            screen = Screen::Switcher { entries, idx: 0, prev: Box::new(prev) };
        }
    }
    let mut marquee_start = Instant::now();
    let mut last_marquee_sel = usize::MAX;
    let mut h_rep = Repeat::new();
    let mut v_rep = Repeat::new();
    let mut s_rep = Repeat::new();
    const PRESS_COOLDOWN: std::time::Duration = std::time::Duration::from_millis(180);
    let mut last_confirm = Instant::now() - PRESS_COOLDOWN;
    let mut last_back = Instant::now() - PRESS_COOLDOWN;
    let mut last_menu = Instant::now() - PRESS_COOLDOWN;
    let mut last_pin = Instant::now() - PRESS_COOLDOWN;
    let mut menu_held = false;
    let mut menu_combo = false;
    let mut last_input = Instant::now();
    // (is_brightness, visible until) — the value is read live from kuid's
    // /tmp/kui state files each frame the bar is up
    let mut osd: Option<(bool, Instant)> = None;
    // (message, visible until) — launch failures surface here
    let mut toast: Option<(String, Instant)> = None;
    // which volume key is held (kuid repeats the apply; we keep the bar up)
    let mut vol_held: i32 = 0;
    let mut last_wipe = Instant::now() - PRESS_COOLDOWN;
    let mut bootlogo_tex: Option<kui_gfx::Texture> = None;
    let mut bootlogo_applied = false;
    let mut themes_tex: Option<kui_gfx::Texture> = None;
    // color picker target when editing LED colors: (profile, light)
    let mut led_pick_target: Option<(usize, usize)> = None;
    // Date & Time page state: (y, mo, d, h, mi), probed lazily
    let mut dt: (i32, i32, i32, i32, i32) = (2026, 1, 1, 0, 0);
    let mut dt_probed = Instant::now() - std::time::Duration::from_secs(60);
    // Y-Y wipe confirmation: (row index, armed at)
    let mut wipe_armed: Option<(usize, Instant)> = None;
    // per-tile cursor memory for this session: tile -> (selected, scroll)

    let mut first_frame_at: Option<()> = None;
    loop {
        // ---- input ----
        let mut quit = false;
        let mut select_btn = false;
        let mut confirm = false;
        let mut back = false;
        let mut menu = false;
        let mut power = false;
        let mut pin_btn = false;
        let mut wipe_btn = false;
        let mut start_btn = false;
        let mut vol_btn: i32 = 0;
        let mut any_input = false;
        for ev in v.events.poll_iter() {
            use sdl2::event::Event;
            use sdl2::keyboard::Keycode;
            if let Event::Quit { .. } = ev {
                // SDL delivers SIGTERM (killall during deploys) as Quit:
                // exit 0 so the boot loop relaunches us with the flag kept.
                // Leaving to C kUI is Select (rc 66), nothing else.
                if on_device {
                    return 0;
                }
                quit = true;
            }
            match &ev {
                Event::KeyDown { keycode: Some(k), repeat: false, .. } => match *k {
                    Keycode::Escape => quit = true,
                    Keycode::Left => h_rep.held[2][0] = true,
                    Keycode::Right => h_rep.held[2][1] = true,
                    Keycode::Up => v_rep.held[2][0] = true,
                    Keycode::Down => v_rep.held[2][1] = true,
                    Keycode::Return => confirm = true,
                    Keycode::Backspace => back = true,
                    Keycode::M => menu = true,
                    Keycode::PageUp => s_rep.held[2][0] = true,
                    Keycode::PageDown => s_rep.held[2][1] = true,
                    _ => {}
                },
                Event::KeyUp { keycode: Some(k), .. } => match *k {
                    Keycode::Left => h_rep.held[2][0] = false,
                    Keycode::Right => h_rep.held[2][1] = false,
                    Keycode::Up => v_rep.held[2][0] = false,
                    Keycode::Down => v_rep.held[2][1] = false,
                    Keycode::PageUp => s_rep.held[2][0] = false,
                    Keycode::PageDown => s_rep.held[2][1] = false,
                    _ => {}
                },
                _ => {}
            }
            match tg5040::map_event(&ev) {
                Some(InputEvent::Dpad { up, down, left, right }) => {
                    h_rep.held[0][0] = left;
                    h_rep.held[0][1] = right;
                    v_rep.held[0][0] = up;
                    v_rep.held[0][1] = down;
                }
                Some(InputEvent::FnSwitch(on)) => input_fn = Some(on),
                Some(InputEvent::Jack(on)) => input_jack = Some(on),
                Some(InputEvent::Button(b, state)) => {
                    let down = state == ButtonState::Pressed;
                    match b {
                        _ if matches!(screen, Screen::InputTest) => {
                            if let Some(i2) = input_slot(b) {
                                input_pressed[i2] = down;
                            }
                            // a volume release leaking in here must stop repeat
                            if !down && matches!(b, Button::VolUp | Button::VolDown) {
                                vol_held = 0;
                            }
                            if matches!(b, Button::Menu) {
                                input_menu_hold =
                                    if down { Some(std::time::Instant::now()) } else { None };
                            }
                        }
                        Button::L1 => s_rep.held[0][0] = down,
                        Button::R1 => s_rep.held[0][1] = down,
                        Button::A if down => confirm = true,
                        Button::B if down => back = true,
                        Button::Menu => {
                            if down {
                                menu_held = true;
                                menu_combo = false;
                            } else {
                                // only a press we saw counts: a release leaking
                                // out of the input tester must not open the menu
                                if menu_held && !menu_combo {
                                    menu = true;
                                }
                                menu_held = false;
                            }
                        }
                        _ if sc_cap.is_some()
                            && down
                            && !matches!(b, Button::Menu | Button::Power) =>
                        {
                            sc_captured = Some(if menu_held {
                                menu_combo = true;
                                format!("menu+{}", lbutton_name(b))
                            } else {
                                lbutton_name(b).to_string()
                            });
                        }
                        Button::Power if down => power = true,
                        Button::X if down => pin_btn = true,
                        Button::Y if down => wipe_btn = true,
                        Button::Start if down => start_btn = true,
                        Button::VolUp => {
                            if down {
                                vol_btn = 1;
                                vol_held = 1;
                            } else if vol_held > 0 {
                                vol_held = 0;
                            }
                        }
                        Button::VolDown => {
                            if down {
                                vol_btn = -1;
                                vol_held = -1;
                            } else if vol_held < 0 {
                                vol_held = 0;
                            }
                        }
                        Button::Select if down => select_btn = true,
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        if confirm || back || menu || power || pin_btn || wipe_btn || start_btn
            || vol_btn != 0
            || vol_held != 0
            || h_rep.holding()
            || v_rep.holding()
            || s_rep.holding()
        {
            any_input = true;
        }
        if any_input {
            last_input = now_hint();
        }
        if quit {
            return EXIT_QUIT;
        }
        // volume keys: volume, or brightness while MENU is held — kuid
        // applies (and repeats) globally; we only keep the OSD up, and it
        // stays up as long as a key is held
        if (vol_btn != 0 || vol_held != 0) && on_device {
            if menu_held {
                menu_combo = true;
            }
            osd = Some((menu_held, now_hint() + std::time::Duration::from_millis(1200)));
        }
        // auto sleep on idle
        let auto_min = cfg.get_i32("power.auto_sleep_min", 5);
        if on_device
            && cfg.get_or("dev.no_sleep", "off") != "on"
            && auto_min > 0
            && last_input.elapsed() > std::time::Duration::from_secs(auto_min as u64 * 60)
        {
            v.deep_sleep(&cfg);
            last_input = now_hint();
            h_rep.clear();
            v_rep.clear();
            s_rep.clear();
            vol_held = 0;
        }
        // Power button: sleep from anywhere (C kUI behavior). Wake returns
        // to exactly the screen we left.
        if power && on_device {
            v.deep_sleep(&cfg);
            last_input = now_hint();
            h_rep.clear();
            v_rep.clear();
            s_rep.clear();
            vol_held = 0;
        }

        let now = Instant::now();
        if confirm {
            if now - last_confirm < PRESS_COOLDOWN {
                confirm = false;
            } else {
                last_confirm = now;
            }
        }
        if back {
            if now - last_back < PRESS_COOLDOWN {
                back = false;
            } else {
                last_back = now;
            }
        }
        if menu {
            if now - last_menu < PRESS_COOLDOWN {
                menu = false;
            } else {
                last_menu = now;
            }
        }
        if pin_btn {
            if now - last_pin < PRESS_COOLDOWN {
                pin_btn = false;
            } else {
                last_pin = now;
            }
        }
        if wipe_btn {
            if now - last_wipe < PRESS_COOLDOWN {
                wipe_btn = false;
            } else {
                last_wipe = now;
            }
        }
        // SELECT toggles the game switcher from anywhere (like MENU);
        // the OSK keeps its keys to itself
        if select_btn
            && sc_cap.is_none()
            && !matches!(screen, Screen::Osk { .. } | Screen::InputTest)
        {
            screen = match screen {
                Screen::Switcher { prev, .. } => *prev,
                other => {
                    let entries = sd.recents();
                    if entries.is_empty() {
                        other
                    } else {
                        for (_, t) in switcher_art.drain() {
                            if let Some(t) = t {
                                r.drop_texture(&v.gl, t);
                            }
                        }
                        Screen::Switcher { entries, idx: 0, prev: Box::new(other) }
                    }
                }
            };
            v_rep.clear();
        }
        // START toggles The Dude from anywhere: press again to leave,
        // the way MENU toggles the quick menu and SELECT the switcher
        if start_btn
            && sc_cap.is_none()
            && !matches!(
                screen,
                Screen::Osk { .. } | Screen::Files { .. } | Screen::InputTest
            )
        {
            screen = match screen {
                Screen::Dude => {
                    dude_state = None;
                    // leave: back to where START opened it
                    match dude_prev.take() {
                        Some(p) => *p,
                        None => root_screen(ui_mode, &platforms, &tiles, tile),
                    }
                }
                other => {
                    dude_state = None; // fresh open: greeting + quest crediting
                    dude_prev = Some(Box::new(other));
                    Screen::Dude
                }
            };
            v_rep.clear();
        }
        // MENU toggles the quick menu from anywhere
        if menu && sc_cap.is_none() && !matches!(screen, Screen::InputTest) {
            screen = match screen {
                Screen::Quick { prev, .. } => *prev,
                other => Screen::Quick {
                    items: quick_items(&sd, on_device),
                    selected: 0,
                    prev: Box::new(other),
                    probed: now,
                    pending: None,
                },
            };
            v_rep.clear();
        }
        let h_step = h_rep.step(now);
        let v_step = v_rep.step(now);
        let s_step = s_rep.step(now);

        // ---- update ----
        let mut next_screen: Option<Screen> = None;
        match &mut screen {
            Screen::Carousel => {
                let mv = if h_step != 0 { h_step } else { s_step };
                if mv != 0 {
                    tile = wrap(tile, mv, n);
                }
                // B at root is a no-op: leaving the OS is Select, deliberately
                if confirm {
                    next_screen = open_tile_mode(
                        &sd, &loader, &mut fbg, &mut boxart, &mut infos, &platforms, &tiles,
                        tile, ui_mode, &remember,
                    );
                    v_rep.clear();
                }
            }
            Screen::Dude => {
                if dude_state.is_none() {
                    // the Dude's game pool = the launcher's playable library
                    // (platforms already filtered to those with an emulator)
                    let library: Vec<String> = platforms
                        .iter()
                        .flat_map(|p| {
                            p.roms.iter().map(move |r| format!("{}/{}", p.folder, r))
                        })
                        .collect();
                    let d = dude::Dude::open(
                        &sd.root.join(".userdata/shared"),
                        &sd.root.join("Roms"),
                        library,
                    );
                    dude_text = d.greeting();
                    dude_state = Some(d);
                    dude_menu = 0;
                    dude_sel = 0;
                    dude_armed = false;
                }
                if let Some(d) = dude_state.as_mut() {
                    if v_step != 0 {
                        dude_menu = wrap(dude_menu, v_step, d.menu().len());
                        dude_sel = 0;
                        dude_armed = false;
                    }
                    let item = d.menu()[dude_menu];
                    if h_step != 0 {
                        let n2 = match item {
                            "Dude Quests" => d.dude_quests().len(),
                            "Achievements" => {
                                let t: usize =
                                    d.achievement_pages().iter().map(|p| p.len()).sum();
                                t.div_ceil(12)
                            }
                            "Play History" => d.history_lines().len().div_ceil(4).max(1),
                            _ => 0,
                        };
                        if n2 > 0 {
                            dude_sel = wrap(dude_sel, h_step, n2);
                        }
                    }
                    if confirm {
                        match item {
                            "Talk" => {
                                dude_text = d.talk();
                                d.save();
                            }
                            "Dude Quests" => {
                                if let Some(rom) = d.accept_quest(dude_sel) {
                                    let label = rom
                                        .file_stem()
                                        .map(|s| s.to_string_lossy().into_owned())
                                        .unwrap_or_default();
                                    let _ = std::fs::write("/tmp/kui_dude", "");
                                    match launch_rom(&sd, &cfg, &rom, &label, on_device) {
                                        LaunchResult::Started(code) => return code,
                                        LaunchResult::Fail(msg) => {
                                            toast = Some((msg, now_hint() + TOAST_TIME));
                                        }
                                        LaunchResult::NoOp => {}
                                    }
                                }
                            }
                            "Reset Progress" => {
                                if dude_armed {
                                    d.reset();
                                    dude_armed = false;
                                    dude_text = d.greeting();
                                } else {
                                    dude_armed = true;
                                }
                            }
                            _ => {}
                        }
                    }
                    if back {
                        dude_state = None;
                        next_screen = Some(match dude_prev.take() {
                            Some(p) => *p,
                            None => root_screen(ui_mode, &platforms, &tiles, tile),
                        });
                    }
                }
            }
            Screen::HubIndex { selected } => {
                if v_step != 0 && !hub_order.is_empty() {
                    let pos = hub_order
                        .iter()
                        .position(|i| i == selected)
                        .unwrap_or(0);
                    *selected = hub_order[wrap(pos, v_step, hub_order.len())];
                }
                let sel_row = hub_rows
                    .iter()
                    .position(|r2| matches!(r2, HubRow::Page(i) if i == selected))
                    .unwrap_or(0);
                let visible = 10usize.min(hub_rows.len());
                if sel_row < hub_scroll {
                    hub_scroll = sel_row;
                }
                if sel_row >= hub_scroll + visible {
                    hub_scroll = sel_row + 1 - visible;
                }
                // a group's header rides along with its first entry
                if hub_scroll == sel_row
                    && sel_row > 0
                    && matches!(hub_rows[sel_row - 1], HubRow::Header(_))
                {
                    hub_scroll = sel_row - 1;
                }
                if back {
                    // mode/theme may have changed: rebuild everything
                    ui_mode = UiMode::from_config(&cfg);
                    theme = sd::Theme::from_config(&cfg);
                    let (t2, d2) = build_tiles(platforms.len());
                    tiles = t2;
                    dude_tile = d2;
                    n = tiles.len();
                    tile = tile.min(n.saturating_sub(1));
                    let _ = dude_tile;
                    // Control Panel opens from the quick menu: land back there
                    next_screen = Some(quick_return(
                        &sd,
                        on_device,
                        now,
                        root_screen(ui_mode, &platforms, &tiles, tile),
                        QuickAction::Settings,
                    ));
                    v_rep.clear();
                } else if confirm {
                    if hub_pages[*selected].title == "LED Control" {
                        next_screen =
                            Some(Screen::LedEditor { row: 0, profile: 0, light: 0 });
                    } else if hub_pages[*selected].title == "PakDek" {
                        if let Ok(mut g) = pak_fetch.lock() {
                            *g = None;
                        }
                        let pf2 = pak_fetch.clone();
                        std::thread::spawn(move || {
                            let res = kui_store::fetch_storefront();
                            if let Ok(mut g) = pf2.lock() {
                                *g = Some(res);
                            }
                        });
                        pak_all.clear();
                        next_screen = Some(Screen::PakCats {
                            cats: Vec::new(),
                            selected: 0,
                            scroll: 0,
                        });
                        v_rep.clear();
                    } else if hub_pages[*selected].title == "Ports" {
                        if let Ok(mut g) = ports_fetch.lock() {
                            *g = None;
                        }
                        let pf2 = ports_fetch.clone();
                        let root = sd.root.clone();
                        std::thread::spawn(move || {
                            let res = kui_store::ports::catalog(&root);
                            if let Ok(mut g) = pf2.lock() {
                                *g = Some(res);
                            }
                        });
                        ports_all = kui_store::ports::Catalog::default();
                        next_screen = Some(Screen::PortCats {
                            cats: Vec::new(),
                            selected: 0,
                            scroll: 0,
                            rtr: false,
                        });
                        v_rep.clear();
                    } else if hub_pages[*selected].title == "Port Forge" {
                        if let Ok(mut m) = store_msg.lock() {
                            m.clear();
                        }
                        next_screen = Some(Screen::PortForge { pane: dir_pane(sd.root.clone()) });
                        v_rep.clear();
                    } else if hub_pages[*selected].title == "Updater" {
                        if let Ok(mut g) = rel_fetch.lock() {
                            *g = None;
                        }
                        let rf = rel_fetch.clone();
                        std::thread::spawn(move || {
                            let res = kui_store::fetch_releases();
                            if let Ok(mut g) = rf.lock() {
                                *g = Some(res);
                            }
                        });
                        next_screen =
                            Some(Screen::Updater { releases: Vec::new(), selected: 0 });
                        v_rep.clear();
                    } else if hub_pages[*selected].title == "Scraper" {
                        let mut rows: Vec<(String, Option<PathBuf>)> =
                            vec![("All Platforms".into(), None)];
                        for p in &platforms {
                            rows.push((p.display.clone(), Some(p.dir.clone())));
                        }
                        next_screen = Some(Screen::ScraperPlatforms {
                            rows,
                            selected: 0,
                            scroll: 0,
                        });
                        v_rep.clear();
                    } else if hub_pages[*selected].title == "Input" {
                        input_pressed = [false; INPUT_LABELS.len()];
                        input_menu_hold = None;
                        next_screen = Some(Screen::InputTest);
                        v_rep.clear();
                    } else if hub_pages[*selected].title == "Files" {
                        next_screen = Some(Screen::Files {
                            panes: [
                                file_pane(sd.root.clone()),
                                file_pane(sd.root.clone()),
                            ],
                            active: 0,
                            menu: None,
                            armed_delete: false,
                        });
                        v_rep.clear();
                    } else if hub_pages[*selected].title == "Battery" {
                        next_screen = Some(Screen::Battery {
                            samples: battlog_read(&sd),
                            span_h: 24,
                        });
                        v_rep.clear();
                    } else if hub_pages[*selected].title == "Game Tracker" {
                        let (rows, header) = gametime_rows(&sd);
                        next_screen =
                            Some(Screen::GameTime { rows, header, selected: 0, scroll: 0 });
                        v_rep.clear();
                    } else if hub_pages[*selected].title == "Core Options" {
                        let dir = std::env::var("CORES_PATH")
                            .map(PathBuf::from)
                            .unwrap_or_else(|_| sd.root.join(".system/tg5040/cores"));
                        // console names per core via the native table
                        let mut by_core: HashMap<String, (Vec<String>, Vec<String>)> =
                            HashMap::new();
                        for p in &platforms {
                            if let Some(stem) = resolve_core(&cfg, &sd, &p.tag) {
                                let e = by_core.entry(stem).or_default();
                                e.0.push(p.display.clone());
                                e.1.push(p.tag.clone());
                            }
                        }
                        let mut cores: Vec<CoreEntry> = std::fs::read_dir(&dir)
                            .map(|rd| {
                                rd.flatten()
                                    .map(|e| e.path())
                                    .filter(|p| {
                                        p.extension().map(|e| e == "so").unwrap_or(false)
                                    })
                                    .map(|p| {
                                        let stem = p
                                            .file_stem()
                                            .map(|s| {
                                                s.to_string_lossy().replace("_libretro", "")
                                            })
                                            .unwrap_or_default();
                                        let (consoles, tags) = by_core
                                            .get(&stem)
                                            .cloned()
                                            .unwrap_or_default();
                                        let label = if consoles.is_empty() {
                                            stem.clone()
                                        } else {
                                            format!("{} ({stem})", consoles.join(", "))
                                        };
                                        CoreEntry { label, stem, path: p, tags }
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();
                        // only cores that actually serve games on this card
                        cores.retain(|c| !c.tags.is_empty());
                        cores.sort_by_key(|c| c.label.to_lowercase());
                        next_screen =
                            Some(Screen::CoreList { cores, selected: 0, scroll: 0 });
                    } else if hub_pages[*selected].title == "Boot Logo" {
                        next_screen = Some(Screen::BootLogo { idx: 0 });
                    } else if hub_pages[*selected].title == "Themes" {
                        next_screen = Some(Screen::Themes { idx: 0 });
                    } else {
                        next_screen = Some(Screen::HubPage { page: *selected, selected: 0 });
                    }
                    v_rep.clear();
                }
            }
            Screen::HubPage { page, selected } => {
                let items = &hub_pages[*page].items;
                if v_step != 0 && !items.is_empty() {
                    *selected = wrap(*selected, v_step, items.len());
                }
                if *selected < hub_page_scroll {
                    hub_page_scroll = *selected;
                }
                if *selected >= hub_page_scroll + 9 {
                    hub_page_scroll = *selected + 1 - 9;
                }
                // refresh datetime cache while on the page
                if hub_pages[*page].title == "Date & Time"
                    && on_device
                    && dt_probed.elapsed() > std::time::Duration::from_secs(5)
                {
                    dt_probed = Instant::now();
                    if let Ok(out) = std::process::Command::new("date")
                        .arg("+%Y %m %d %H %M")
                        .output()
                        && let Ok(txt) = String::from_utf8(out.stdout)
                    {
                        let parts: Vec<i32> =
                            txt.split_whitespace().filter_map(|p| p.parse().ok()).collect();
                        if parts.len() == 5 {
                            dt = (parts[0], parts[1], parts[2], parts[3], parts[4]);
                        }
                    }
                }
                if h_step != 0
                    && let Some(item) = items.get(*selected)
                {
                    if item.key.starts_with("dt.") {
                        if on_device {
                            match item.key {
                                "dt.year" => dt.0 = (dt.0 + h_step).clamp(2020, 2100),
                                "dt.month" => dt.1 = (dt.1 + h_step - 1).rem_euclid(12) + 1,
                                "dt.day" => dt.2 = (dt.2 + h_step - 1).rem_euclid(31) + 1,
                                "dt.hour" => dt.3 = (dt.3 + h_step).rem_euclid(24),
                                "dt.minute" => dt.4 = (dt.4 + h_step).rem_euclid(60),
                                _ => {}
                            }
                            let _ = std::process::Command::new("sh")
                                .args([
                                    "-c",
                                    &format!(
                                        "date -s '{:04}-{:02}-{:02} {:02}:{:02}:00' && hwclock -w",
                                        dt.0, dt.1, dt.2, dt.3, dt.4
                                    ),
                                ])
                                .status();
                        }
                    } else if item.key == "dev.ssh" {
                        if on_device {
                            let on = proc_running("dropbear sshd");
                            let verb = if on { "stop" } else { "start" };
                            let _ = std::process::Command::new("sh")
                                .args([
                                    "-c",
                                    &format!(
                                        "(/etc/init.d/sshd {verb} || /etc/init.d/S50sshd {verb}) >/dev/null 2>&1 &"
                                    ),
                                ])
                                .spawn();
                        }
                    } else if item.key == "theme.font" {
                        let opts = font_options(&sd);
                        let cur = cfg.get_or("theme.font", "0").to_string();
                        let idx = opts.iter().position(|(v, _)| *v == cur).unwrap_or(0);
                        let ni = (idx as i32 + h_step).rem_euclid(opts.len() as i32) as usize;
                        cfg.set("theme.font", opts[ni].0.as_str());
                        let _ = cfg.save();
                        let p = resolve_font(
                            &sd.root.join(".system/res"),
                            cfg.get_or("theme.font", "0"),
                        );
                        if let Ok(mut nf) = Font::load(&v.gl, &p) {
                            nf.set_bold(
                                cfg.get_or("theme.font_style", "normal") == "bold",
                            );
                            font = Some(nf);
                        }
                    } else if item.key == "cal.enabled" {
                        let on = cfg.get_or("cal.enabled", "on") == "on";
                        cfg.set("cal.enabled", if on { "off" } else { "on" });
                        let _ = cfg.save();
                        if on_device {
                            apply_displaycal_cfg(&cfg);
                        }
                    } else if let Some(ch) = item.key.strip_prefix("cal.gain.") {
                        let cur = cfg.get_i32(item.key, cal_gain_default(ch));
                        cfg.set(item.key, (cur + h_step * 5).clamp(0, 200));
                        let _ = cfg.save();
                        if on_device {
                            apply_displaycal_cfg(&cfg);
                        }
                    } else if item.key.starts_with("fn.") {
                        // persisted in kui.cfg only; kuid applies on FN toggle
                        if fn_page::FN_NUM.contains(&item.key) {
                            let (lo, hi) = fn_page::fn_num_range(item.key);
                            let cur = cfg.get_i32(item.key, fn_page::fn_num_default(item.key));
                            let next = if cur == fn_page::NO_CHANGE {
                                if h_step > 0 { lo } else { hi }
                            } else {
                                let n2 = cur + h_step;
                                if n2 < lo || n2 > hi {
                                    fn_page::NO_CHANGE
                                } else {
                                    n2
                                }
                            };
                            cfg.set(item.key, next);
                            let _ = cfg.save();
                        } else if fn_page::FN_TURBO.contains(&item.key)
                            || item.key == "fn.leds"
                        {
                            let on =
                                cfg.get_or(item.key, fn_page::fn_toggle_default(item.key)) == "on";
                            cfg.set(item.key, if on { "off" } else { "on" });
                            let _ = cfg.save();
                        } else if item.key == "fn.dpad" {
                            // one row, two contract keys
                            let state = (
                                cfg.get_or("fn.dpad_disable", "off") == "on",
                                cfg.get_or("fn.joystick", "off") == "on",
                            );
                            let next = match state {
                                (false, false) => (true, true), // Dpad -> Joystick
                                (true, true) => (false, true),  // Joystick -> Both
                                _ => (false, false),            // Both -> Dpad
                            };
                            cfg.set("fn.dpad_disable", if next.0 { "on" } else { "off" });
                            cfg.set("fn.joystick", if next.1 { "on" } else { "off" });
                            let _ = cfg.save();
                        }
                    } else if item.key.starts_with("radio.") {
                        if on_device {
                            let is_wifi = item.key.ends_with("wifi");
                            let daemon = if is_wifi { "wpa_supplicant" } else { "bluetoothd" };
                            let script = if is_wifi {
                                "/etc/wifi/wifi_init.sh"
                            } else {
                                "/etc/bluetooth/bt_init.sh"
                            };
                            let verb = if proc_running(daemon) { "stop" } else { "start" };
                            let _ = std::process::Command::new("sh")
                                .args(["-c", &format!("{script} {verb} >/dev/null 2>&1 &")])
                                .spawn();
                            // persist: kuid restores this state at boot
                            cfg.set(item.key, if verb == "start" { "on" } else { "off" });
                            let _ = cfg.save();
                        }
                    } else if hub::adjust(&mut cfg, item, h_step) {
                        // live side effects
                        match item.key {
                            "display.brightness" => {
                                if on_device {
                                    let b = cfg.get_i32("display.brightness", 90);
                                    tg5040::set_raw_brightness(tg5040::brightness_raw(b));
                                    write_live_state("bright", b);
                                }
                            }
                            "display.colortemp" => {
                                if on_device {
                                    tg5040::set_colortemp(cfg.get_i32("display.colortemp", 20));
                                }
                            }
                            "display.contrast" => {
                                if on_device {
                                    tg5040::set_contrast(cfg.get_i32("display.contrast", 0));
                                }
                            }
                            "display.saturation" => {
                                if on_device {
                                    tg5040::set_saturation(cfg.get_i32("display.saturation", 0));
                                }
                            }
                            "display.exposure" => {
                                if on_device {
                                    tg5040::set_exposure(cfg.get_i32("display.exposure", 0));
                                }
                            }
                            "audio.volume" => {
                                // speaker slot: only live when nothing is plugged
                                if on_device && !headphone_active() {
                                    let v2 = cfg.get_i32("audio.volume", 40);
                                    tg5040::set_volume_percent(v2);
                                    write_live_state("vol", v2);
                                }
                            }
                            "audio.volume_hp" => {
                                if on_device && headphone_active() {
                                    let v2 = cfg.get_i32("audio.volume_hp", 40);
                                    tg5040::set_volume_percent(v2);
                                    write_live_state("vol", v2);
                                }
                            }
                            "ui.mode" => ui_mode = UiMode::from_config(&cfg),
                            "theme.font" | "theme.font_style" => {
                                let p = resolve_font(
                                    &sd.root.join(".system/res"),
                                    cfg.get_or("theme.font", "0"),
                                );
                                if let Ok(mut nf) = Font::load(&v.gl, &p) {
                                    nf.set_bold(
                                        cfg.get_or("theme.font_style", "normal") == "bold",
                                    );
                                    font = Some(nf);
                                }
                            }
                            "power.profile"
                                if on_device => {
                                    apply_power_profile(cfg.get_or("power.profile", "auto"));
                                }
                            _ => {}
                        }
                    }
                }
                if confirm
                    && let Some(item) = items.get(*selected)
                    && item.key == "radio.wifi"
                {
                    next_screen =
                        Some(Screen::Wifi { nets: Vec::new(), selected: 0, scroll: 0 });
                    v_rep.clear();
                } else if confirm
                    && let Some(item) = items.get(*selected)
                    && (item.key == "ra.user" || item.key == "ra.pass")
                {
                    next_screen = Some(Screen::Osk {
                        buf: cfg.get_or(item.key, "").to_string(),
                        pos: 0,
                        target: OskTarget::ConfigValue {
                            key: item.key.to_string(),
                            page: *page,
                            row: *selected,
                        },
                    });
                    v_rep.clear();
                } else if confirm
                    && let Some(item) = items.get(*selected)
                    && item.key == "radio.bluetooth"
                {
                    next_screen = Some(Screen::Bt { devs: Vec::new(), selected: 0, scroll: 0 });
                    v_rep.clear();
                } else if confirm
                    && let Some(item) = items.get(*selected)
                    && (item.key.starts_with("reset.")
                        || matches!(item.kind, hub::ItemKind::Action))
                {
                    match item.key {
                        "dt.ntp" => {
                            if on_device {
                                let _ = std::process::Command::new("sh")
                                    .args([
                                        "-c",
                                        "(ntpd -q -n -p pool.ntp.org && hwclock -w) >/dev/null 2>&1 &",
                                    ])
                                    .spawn();
                            }
                        }
                        "reset.system" => {
                            for k in [
                                "audio.volume",
                                "power.auto_sleep_min",
                                "power.safe_poweroff",
                                "power.profile",
                                "save.format",
                                "save.extracted",
                            ] {
                                cfg.remove_prefix(k);
                            }
                            let _ = cfg.save();
                        }
                        "reset.fn" => {
                            // everything back to Unchanged / Off
                            cfg.remove_prefix("fn.");
                            let _ = cfg.save();
                        }
                        "ra.prefetch" => {
                            let running =
                                ra_pf.lock().map(|g| !g.3).unwrap_or(false);
                            if running {
                                ra_pf_cancel
                                    .store(true, std::sync::atomic::Ordering::Relaxed);
                            } else {
                                let user = cfg.get_or("ra.user", "").to_string();
                                let token = cfg.get_or("ra.token", "").to_string();
                                if user.is_empty() || token.is_empty() {
                                    if let Ok(mut g) = ra_pf.lock() {
                                        *g = (0, 0, "Authenticate first".into(), true);
                                    }
                                } else {
                                    let mut rows: Vec<(String, Option<String>)> =
                                        vec![("All Platforms".into(), None)];
                                    for p in &platforms {
                                        rows.push((p.display.clone(), Some(p.tag.clone())));
                                    }
                                    next_screen = Some(Screen::PrefetchPlatforms {
                                        rows,
                                        selected: 0,
                                        scroll: 0,
                                        ret: (*page, *selected),
                                    });
                                    v_rep.clear();
                                }
                            }
                        }
                        "cheats.download" => {
                            let running =
                                cheat_pf.lock().map(|g| !g.3).unwrap_or(false);
                            if running {
                                cheat_pf_cancel
                                    .store(true, std::sync::atomic::Ordering::Relaxed);
                            } else {
                                // only platforms the cheat database covers
                                let mut rows: Vec<(String, Option<String>)> =
                                    vec![("All Platforms".into(), None)];
                                for p in &platforms {
                                    if kui_cheatdl::has_db(&p.tag) {
                                        rows.push((p.display.clone(), Some(p.tag.clone())));
                                    }
                                }
                                next_screen = Some(Screen::CheatPlatforms {
                                    rows,
                                    selected: 0,
                                    scroll: 0,
                                    ret: (*page, *selected),
                                });
                                v_rep.clear();
                            }
                        }
                        "ra.auth" => {
                            let user = cfg.get_or("ra.user", "").to_string();
                            let pass = cfg.get_or("ra.pass", "").to_string();
                            if user.is_empty() || pass.is_empty() {
                                ra_auth_msg = Some("Enter credentials first".into());
                            } else {
                                // device curl lacks --data-urlencode: encode here
                                let url = format!(
                                    "https://retroachievements.org/dorequest.php?r=login2&u={}&p={}",
                                    urlenc(&user),
                                    urlenc(&pass)
                                );
                                // absolute path: the boot loop's PATH is minimal.
                                // The device ships no CA store; kUI carries one.
                                let ca = "/mnt/SDCARD/.system/res/cacert.pem";
                                let mut args: Vec<String> =
                                    vec!["-s".into(), "--max-time".into(), "8".into()];
                                if std::path::Path::new(ca).is_file() {
                                    args.push("--cacert".into());
                                    args.push(ca.into());
                                } else {
                                    args.push("-k".into());
                                }
                                args.push(url.clone());
                                let out = std::process::Command::new("/usr/bin/curl")
                                    .args(&args)
                                    .output()
                                    .map(|o| {
                                        if !o.stderr.is_empty() {
                                            eprintln!(
                                                "ra auth curl: {}",
                                                String::from_utf8_lossy(&o.stderr)
                                            );
                                        }
                                        String::from_utf8_lossy(&o.stdout).into_owned()
                                    })
                                    .unwrap_or_else(|e| {
                                        eprintln!("ra auth spawn: {e}");
                                        String::new()
                                    });
                                let token = out
                                    .split("\"Token\":\"")
                                    .nth(1)
                                    .and_then(|t| t.split('"').next())
                                    .map(str::to_string);
                                if out.contains("\"Success\":true")
                                    && let Some(tok) = token
                                {
                                    cfg.set("ra.token", &tok);
                                    let _ = cfg.save();
                                    ra_auth_msg = Some(format!("Logged in as {user}"));
                                } else if out.is_empty() {
                                    ra_auth_msg = Some("No connection".into());
                                } else {
                                    ra_auth_msg = Some("Login failed".into());
                                }
                            }
                        }
                        "dev.clean_dots" => {
                            let _ = std::process::Command::new("sh")
                                .args([
                                    "-c",
                                    "find /mnt/SDCARD \\( -name '._*' -o -name '.DS_Store' -o -name 'Thumbs.db' \\) -delete >/dev/null 2>&1 &",
                                ])
                                .spawn();
                        }
                        "reset.theme" => {
                            cfg.remove_prefix("theme.color");
                            cfg.remove_prefix("theme.font");
                            let _ = cfg.save();
                            theme = sd::Theme::from_config(&cfg);
                        }
                        "reset.display" => {
                            cfg.remove_prefix("display.");
                            cfg.remove_prefix("cal.");
                            let _ = cfg.save();
                            if on_device {
                                tg5040::set_raw_brightness(tg5040::brightness_raw(90));
                                write_live_state("bright", 90);
                                tg5040::set_colortemp(20);
                                tg5040::set_contrast(0);
                                tg5040::set_saturation(0);
                                tg5040::set_exposure(0);
                                apply_displaycal_cfg(&cfg);
                            }
                        }
                        _ => {}
                    }
                } else if confirm
                    && let Some(item) = items.get(*selected)
                    && item.key.starts_with("theme.color")
                {
                    let cur = cfg
                        .get(item.key)
                        .and_then(|s| u32::from_str_radix(s.trim_start_matches("0x"), 16).ok())
                        .unwrap_or_else(|| theme_color_default(item.key));
                    let rgb = [
                        ((cur >> 16) & 0xFF) as i32,
                        ((cur >> 8) & 0xFF) as i32,
                        (cur & 0xFF) as i32,
                    ];
                    next_screen = Some(Screen::ColorPick {
                        key: item.key.to_string(),
                        back: PickBack::HubPage(*page),
                        rgb,
                        orig: rgb,
                        channel: 0,
                    });
                    v_rep.clear();
                }
                if back {
                    next_screen = Some(Screen::HubIndex { selected: *page });
                    v_rep.clear();
                }
            }
            Screen::ColorPick { key, back: pick_back, rgb, orig, channel } => {
                if v_step != 0 {
                    *channel = (*channel as i32 + v_step).rem_euclid(3) as usize;
                }
                let fine = h_step;
                let coarse = s_step * 16;
                let delta = fine + coarse;
                if delta != 0 {
                    rgb[*channel] = (rgb[*channel] + delta).clamp(0, 255);
                    // live preview
                    let hex = format!("{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2]);
                    cfg.set(key, &hex);
                    match pick_back {
                        PickBack::HubPage(_) => theme = sd::Theme::from_config(&cfg),
                        PickBack::Led => {
                            if on_device
                                && let Some((p, l)) = led_pick_target
                            {
                                led_apply(&cfg, p, l);
                            }
                        }
                    }
                }
                let finish = if confirm {
                    let hex = format!("{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2]);
                    cfg.set(key, &hex);
                    let _ = cfg.save();
                    true
                } else if back {
                    let hex = format!("{:02X}{:02X}{:02X}", orig[0], orig[1], orig[2]);
                    cfg.set(key, &hex);
                    true
                } else {
                    false
                };
                if finish {
                    match pick_back {
                        PickBack::HubPage(pg) => {
                            theme = sd::Theme::from_config(&cfg);
                            let it = hub_pages[*pg]
                                .items
                                .iter()
                                .position(|i| i.key == key.as_str())
                                .unwrap_or(0);
                            next_screen = Some(Screen::HubPage { page: *pg, selected: it });
                        }
                        PickBack::Led => {
                            if let Some((p, l)) = led_pick_target {
                                if on_device {
                                    led_apply(&cfg, p, l);
                                }
                                next_screen =
                                    Some(Screen::LedEditor { row: 3, profile: p, light: l });
                            } else {
                                next_screen = Some(Screen::HubIndex { selected: 0 });
                            }
                        }
                    }
                    v_rep.clear();
                }
            }
            Screen::LedEditor { row, profile, light } => {
                const ROWS: usize = 6; // profile, light, effect, color, speed, brightness
                if v_step != 0 {
                    *row = (*row as i32 + v_step).rem_euclid(ROWS as i32) as usize;
                }
                if h_step != 0 || s_step != 0 {
                    let d = if h_step != 0 { h_step } else { s_step };
                    match *row {
                        0 => {
                            *profile =
                                (*profile as i32 + d).rem_euclid(LED_PROFILES.len() as i32) as usize;
                            if on_device {
                                led_apply_profile(&cfg, *profile); // preview profile
                            }
                        }
                        1 => {
                            *light =
                                (*light as i32 + d).rem_euclid(LED_LIGHTS.len() as i32) as usize;
                        }
                        2 => {
                            let cur = led_get(&cfg, *profile, *light, "effect");
                            let next = (cur as i32 + d).rem_euclid(8);
                            cfg.set(&led_key(*profile, *light, "effect"), next);
                            let _ = cfg.save();
                            if on_device {
                                led_apply(&cfg, *profile, *light);
                            }
                        }
                        4 => {
                            let cur = led_get(&cfg, *profile, *light, "duration");
                            let next = ((cur as i32 + d * 100).clamp(100, 5000)) as i64;
                            cfg.set(&led_key(*profile, *light, "duration"), next);
                            let _ = cfg.save();
                            if on_device {
                                led_apply(&cfg, *profile, *light);
                            }
                        }
                        5 => {
                            let cur = led_get(&cfg, *profile, *light, "brightness");
                            let next = ((cur as i32 + d * 5).clamp(0, 100)) as i64;
                            cfg.set(&led_key(*profile, *light, "brightness"), next);
                            let _ = cfg.save();
                            if on_device {
                                led_apply(&cfg, *profile, *light);
                            }
                        }
                        _ => {}
                    }
                }
                if confirm && *row == 3 {
                    let cur = led_get(&cfg, *profile, *light, "color") as u32;
                    let rgb = [
                        ((cur >> 16) & 0xFF) as i32,
                        ((cur >> 8) & 0xFF) as i32,
                        (cur & 0xFF) as i32,
                    ];
                    led_pick_target = Some((*profile, *light));
                    next_screen = Some(Screen::ColorPick {
                        key: led_key(*profile, *light, "color"),
                        back: PickBack::Led,
                        rgb,
                        orig: rgb,
                        channel: 0,
                    });
                    v_rep.clear();
                }
                if back {
                    // restore whatever profile is actually operating
                    if on_device {
                        let active = std::fs::read_to_string("/tmp/kui_profile")
                            .ok()
                            .and_then(|s| s.trim().parse::<usize>().ok())
                            .filter(|p| *p < LED_PROFILES.len())
                            .unwrap_or(0);
                        led_apply_profile(&cfg, active);
                    }
                    next_screen = Some(Screen::HubIndex {
                        selected: hub_pos(&hub_pages, "LED Control"),
                    });
                    v_rep.clear();
                }
            }
            Screen::CoreList { cores, selected, scroll } => {
                let n2 = cores.len();
                if v_step != 0 && n2 > 0 {
                    *selected = wrap(*selected, v_step, n2);
                    let visible = 10usize;
                    if *selected < *scroll {
                        *scroll = *selected;
                    }
                    if *selected >= *scroll + visible {
                        *scroll = *selected + 1 - visible;
                    }
                }
                if confirm && let Some(entry) = cores.get(*selected) {
                    let bios_root = std::env::var("BIOS_PATH")
                        .unwrap_or_else(|_| format!("{sd_root}/Bios"));
                    let mut bios_dir = PathBuf::from(&bios_root);
                    if let Some(tag) = entry.tags.first() {
                        bios_dir.push(tag);
                    }
                    // a core that refuses to enumerate still gets its
                    // frontend rows (Scaling, Effect)
                    let defs =
                        enumerate_core_safely(&entry.path, &bios_dir).unwrap_or_else(|e| {
                            eprintln!("core enumerate failed: {e}");
                            Vec::new()
                        });
                    next_screen = Some(Screen::CoreOpts {
                        core: entry.stem.clone(),
                        tags: entry.tags.clone(),
                        defs,
                        selected: 0,
                        scroll: 0,
                        list: cores.clone(),
                        list_pos: (*selected, *scroll),
                    });
                    v_rep.clear();
                }
                if back {
                    next_screen = Some(Screen::HubIndex {
                        selected: hub_pos(&hub_pages, "Core Options"),
                    });
                    v_rep.clear();
                }
            }
            Screen::CoreOpts { core, tags, defs, selected, scroll, list, list_pos } => {
                // synthetic rows: Scaling, Effect, FF, Dpad Mode, then
                // the 11 shortcuts
                let n2 = defs.len() + 4 + LSC.len();
                if v_step != 0 {
                    *selected = wrap(*selected, v_step, n2);
                    let visible = 10usize;
                    if *selected < *scroll {
                        *scroll = *selected;
                    }
                    if *selected >= *scroll + visible {
                        *scroll = *selected + 1 - visible;
                    }
                }
                if h_step != 0 && *selected == 0 {
                    // Scaling default for every console this core serves
                    const CHOICES: [&str; 4] = ["", "native", "aspect", "fullscreen"];
                    let cur = tags
                        .first()
                        .map(|t| cfg.get_or(&format!("fe.{t}.scaling"), "").to_string())
                        .unwrap_or_default();
                    let idx = CHOICES.iter().position(|c| *c == cur).unwrap_or(0);
                    let ni = (idx as i32 + h_step).rem_euclid(CHOICES.len() as i32) as usize;
                    for t in tags.iter() {
                        let key = format!("fe.{t}.scaling");
                        if CHOICES[ni].is_empty() {
                            cfg.remove_prefix(&key);
                        } else {
                            cfg.set(&key, CHOICES[ni]);
                        }
                    }
                    let _ = cfg.save();
                }
                if h_step != 0 && *selected == 1 {
                    // Screen effect default for every console this core serves
                    const ECHOICES: [&str; 3] = ["", "grid", "line"];
                    let cur = tags
                        .first()
                        .map(|t| cfg.get_or(&format!("fe.{t}.effect"), "").to_string())
                        .unwrap_or_default();
                    let idx = ECHOICES.iter().position(|c| *c == cur).unwrap_or(0);
                    let ni = (idx as i32 + h_step).rem_euclid(ECHOICES.len() as i32) as usize;
                    for t in tags.iter() {
                        let key = format!("fe.{t}.effect");
                        if ECHOICES[ni].is_empty() {
                            cfg.remove_prefix(&key);
                        } else {
                            cfg.set(&key, ECHOICES[ni]);
                        }
                    }
                    let _ = cfg.save();
                }
                if h_step != 0 && *selected == 2 {
                    // FF speed default for every console this core serves
                    const FCHOICES: [&str; 8] = ["", "2", "3", "4", "5", "6", "7", "8"];
                    let cur = tags
                        .first()
                        .map(|t| cfg.get_or(&format!("fe.{t}.ff"), "").to_string())
                        .unwrap_or_default();
                    let idx = FCHOICES.iter().position(|c| *c == cur).unwrap_or(0);
                    let ni = (idx as i32 + h_step).rem_euclid(FCHOICES.len() as i32) as usize;
                    for t in tags.iter() {
                        let key = format!("fe.{t}.ff");
                        if FCHOICES[ni].is_empty() {
                            cfg.remove_prefix(&key);
                        } else {
                            cfg.set(&key, FCHOICES[ni]);
                        }
                    }
                    let _ = cfg.save();
                }
                if h_step != 0 && *selected == 3 {
                    // Dpad mode default for every console this core serves:
                    // stick = dpad drives the left analog (stickless device)
                    const DCHOICES: [&str; 3] = ["", "dpad", "stick"];
                    let cur = tags
                        .first()
                        .map(|t| cfg.get_or(&format!("fe.{t}.dpad"), "").to_string())
                        .unwrap_or_default();
                    let idx = DCHOICES.iter().position(|c| *c == cur).unwrap_or(0);
                    let ni = (idx as i32 + h_step).rem_euclid(DCHOICES.len() as i32) as usize;
                    for t in tags.iter() {
                        let key = format!("fe.{t}.dpad");
                        if DCHOICES[ni].is_empty() {
                            cfg.remove_prefix(&key);
                        } else {
                            cfg.set(&key, DCHOICES[ni]);
                        }
                    }
                    let _ = cfg.save();
                }
                if (confirm || pin_btn) && (4..4 + LSC.len()).contains(selected) {
                    let row = *selected - 4;
                    if pin_btn {
                        // X: back to the built-in default for every console
                        for t in tags.iter() {
                            cfg.remove_prefix(&format!("fe.{t}.shortcut.{}", LSC[row].0));
                        }
                        let _ = cfg.save();
                    } else {
                        sc_cap = Some(row);
                    }
                }
                if let Some(row) = sc_cap
                    && let Some(val) = sc_captured.take()
                {
                    for t in tags.iter() {
                        cfg.set(&format!("fe.{t}.shortcut.{}", LSC[row].0), &val);
                    }
                    let _ = cfg.save();
                    sc_cap = None;
                }
                if h_step != 0
                    && *selected > 3 + LSC.len()
                    && let Some(def) = defs.get(*selected - 4 - LSC.len())
                {
                    let ckey = format!("core.{core}.{}", def.key);
                    let cur = cfg
                        .get(&ckey)
                        .map(|s| s.to_string())
                        .or_else(|| {
                            kui_libretro::kui_option_default(core, &def.key).map(str::to_string)
                        })
                        .or_else(|| def.choices.first().cloned())
                        .unwrap_or_default();
                    let idx =
                        def.choices.iter().position(|c| *c == cur).unwrap_or(0);
                    let ni = (idx as i32 + h_step)
                        .rem_euclid(def.choices.len().max(1) as i32)
                        as usize;
                    if let Some(nv) = def.choices.get(ni) {
                        cfg.set(&ckey, nv);
                        let _ = cfg.save();
                    }
                }
                if back {
                    next_screen = Some(Screen::CoreList {
                        cores: list.clone(),
                        selected: list_pos.0,
                        scroll: list_pos.1,
                    });
                    v_rep.clear();
                }
            }
            Screen::Wifi { nets, selected, scroll } => {
                let on = proc_running("wpa_supplicant");
                // kick a scan when the radio is up and the list is stale
                if on && nets.is_empty() && wifi_scan_at.is_none() {
                    let _ = std::process::Command::new("sh")
                        .args(["-c", "wpa_cli -p /etc/wifi/sockets -i wlan0 scan >/dev/null 2>&1"])
                        .spawn();
                    wifi_scan_at = Some(now);
                }
                if let Some(t) = wifi_scan_at
                    && now >= t
                    && now - t >= std::time::Duration::from_secs(3)
                {
                    *nets = wifi_scan_collect();
                    wifi_scan_at = None;
                }
                let total = 1 + nets.len();
                if v_step != 0 {
                    *selected = wrap(*selected, v_step, total);
                    let visible = 10usize;
                    if *selected < *scroll {
                        *scroll = *selected;
                    }
                    if *selected >= *scroll + visible {
                        *scroll = *selected + 1 - visible;
                    }
                }
                if confirm {
                    if *selected == 0 {
                        // toggle the radio; persist for kuid's boot restore
                        let verb = if on { "stop" } else { "start" };
                        let _ = std::process::Command::new("sh")
                            .args([
                                "-c",
                                &format!("/etc/wifi/wifi_init.sh {verb} >/dev/null 2>&1 &"),
                            ])
                            .spawn();
                        cfg.set("radio.wifi", if on { "off" } else { "on" });
                        let _ = cfg.save();
                        nets.clear();
                        wifi_scan_at =
                            (!on).then(|| now + std::time::Duration::from_secs(5));
                    } else if let Some(net) = nets.get(*selected - 1) {
                        if let Some(id) = net.saved {
                            let _ = std::process::Command::new("sh")
                                .args([
                                    "-c",
                                    &format!(
                                        "wpa_cli -p /etc/wifi/sockets -i wlan0 select_network {id} >/dev/null 2>&1; wpa_cli -p /etc/wifi/sockets -i wlan0 save_config >/dev/null 2>&1"
                                    ),
                                ])
                                .spawn();
                            wifi_scan_at = Some(now + std::time::Duration::from_secs(3));
                            nets.clear();
                        } else if net.secured {
                            next_screen = Some(Screen::Osk {
                                buf: String::new(),
                                pos: 0,
                                target: OskTarget::WifiPass { ssid: net.ssid.clone() },
                            });
                        } else {
                            wifi_connect_spawn(&net.ssid, None);
                            wifi_scan_at = Some(now + std::time::Duration::from_secs(4));
                            nets.clear();
                        }
                        v_rep.clear();
                    }
                }
                if wipe_btn
                    && *selected > 0
                    && let Some(net) = nets.get(*selected - 1)
                    && let Some(id) = net.saved
                {
                    let _ = std::process::Command::new("sh")
                        .args([
                            "-c",
                            &format!(
                                "wpa_cli -p /etc/wifi/sockets -i wlan0 remove_network {id} >/dev/null 2>&1; wpa_cli -p /etc/wifi/sockets -i wlan0 save_config >/dev/null 2>&1"
                            ),
                        ])
                        .status();
                    nets.clear();
                    wifi_scan_at = Some(now);
                }
                if pin_btn {
                    nets.clear();
                    wifi_scan_at = None; // restarts the scan next frame
                }
                if back {
                    next_screen = Some(Screen::HubPage {
                        page: hub_pos(&hub_pages, "Connectivity"),
                        selected: 0,
                    });
                    v_rep.clear();
                }
            }
            Screen::PakCats { cats, selected, scroll } => {
                if cats.is_empty()
                    && let Ok(mut g) = pak_fetch.lock()
                    && let Some(res) = g.take()
                {
                    match res {
                        Ok(list) => {
                            pak_all = list;
                            *cats = pak_categories(&pak_all);
                        }
                        Err(e) => {
                            if let Ok(mut m) = store_msg.lock() {
                                *m = format!("Fetch failed: {e}");
                            }
                        }
                    }
                }
                if cats.is_empty() && !pak_all.is_empty() {
                    *cats = pak_categories(&pak_all);
                }
                if v_step != 0 && !cats.is_empty() {
                    *selected = wrap(*selected, v_step, cats.len());
                    let visible = 10usize;
                    if *selected < *scroll {
                        *scroll = *selected;
                    }
                    if *selected >= *scroll + visible {
                        *scroll = *selected + 1 - visible;
                    }
                }
                if confirm && let Some((cat, _, _)) = cats.get(*selected) {
                    let filtered: Vec<kui_store::Pak> = if cat == "Installed" {
                        pak_all
                            .iter()
                            .filter(|p| kui_store::installed_version(p).is_some())
                            .cloned()
                            .collect()
                    } else {
                        pak_all
                            .iter()
                            .filter(|p| p.categories.iter().any(|c2| c2 == cat))
                            .cloned()
                            .collect()
                    };
                    next_screen = Some(Screen::PakDek {
                        paks: filtered,
                        title: cat.clone(),
                        selected: 0,
                        scroll: 0,
                        cat_sel: *selected,
                    });
                    v_rep.clear();
                }
                if back {
                    if let Ok(mut m) = store_msg.lock() {
                        m.clear();
                    }
                    next_screen = Some(Screen::HubIndex {
                        selected: hub_pos(&hub_pages, "PakDek"),
                    });
                    v_rep.clear();
                }
            }
            Screen::PakDek { paks, selected, scroll, cat_sel, .. } => {
                // A finished install resolved the LATEST release; sync the
                // listed version so the row reads "installed", not a stale
                // "UPDATE" against the storefront pin.
                if let Ok(mut d) = pak_installed.lock()
                    && let Some((id, ver)) = d.take()
                {
                    for p in paks.iter_mut().chain(pak_all.iter_mut()) {
                        if p.id == id {
                            p.version = ver.clone();
                        }
                    }
                }
                if v_step != 0 && !paks.is_empty() {
                    *selected = wrap(*selected, v_step, paks.len());
                    let visible = pakdek_visible_rows();
                    if *selected < *scroll {
                        *scroll = *selected;
                    }
                    if *selected >= *scroll + visible {
                        *scroll = *selected + 1 - visible;
                    }
                }
                let busy = store_msg
                    .lock()
                    .map(|m| {
                        !m.is_empty() && !m.starts_with("Done") && !m.contains("failed")
                    })
                    .unwrap_or(false);
                if confirm
                    && !busy
                    && let Some(p) = paks.get(*selected)
                    && kui_store::installed_version(p).as_deref()
                        != Some(p.version.as_str())
                {
                    let p2 = p.clone();
                    let msg = store_msg.clone();
                    let done = pak_installed.clone();
                    if let Ok(mut m) = msg.lock() {
                        *m = format!("Installing {}...", p2.name);
                    }
                    std::thread::spawn(move || {
                        let r2 = kui_store::install_pak(&p2, |st| {
                            if let Ok(mut m) = msg.lock() {
                                *m = st.to_string();
                            }
                        });
                        if let Ok(mut m) = msg.lock() {
                            *m = match r2 {
                                Ok(ver) => {
                                    if let Ok(mut d) = done.lock() {
                                        *d = Some((p2.id.clone(), ver));
                                    }
                                    format!("Done — {} installed", p2.name)
                                }
                                Err(e) => format!("Install failed: {e}"),
                            };
                        }
                    });
                }
                if wipe_btn
                    && !busy
                    && let Some(p) = paks.get(*selected)
                    && kui_store::installed_version(p).is_some()
                {
                    let ok = kui_store::remove_pak(p).is_ok();
                    if let Ok(mut m) = store_msg.lock() {
                        *m = if ok {
                            format!("Done — {} removed", p.name)
                        } else {
                            "Remove failed".into()
                        };
                    }
                }
                if back {
                    if let Ok(mut m) = store_msg.lock() {
                        m.clear();
                    }
                    let cats2 = pak_categories(&pak_all);
                    let sel2 = (*cat_sel).min(cats2.len().saturating_sub(1));
                    next_screen = Some(Screen::PakCats {
                        cats: cats2,
                        selected: sel2,
                        scroll: sel2.saturating_sub(visible_rows() - 1),
                    });
                    v_rep.clear();
                }
            }
            Screen::PortCats { cats, selected, scroll, rtr } => {
                let rtr = *rtr;
                if cats.is_empty()
                    && let Ok(mut g) = ports_fetch.lock()
                    && let Some(res) = g.take()
                {
                    match res {
                        Ok(c) => {
                            ports_all = c;
                            *cats = port_categories(&ports_all, &sd.root, rtr);
                        }
                        Err(e) => {
                            if let Ok(mut m) = store_msg.lock() {
                                *m = format!("Fetch failed: {e}");
                            }
                        }
                    }
                }
                if cats.is_empty() && !ports_all.ports.is_empty() {
                    *cats = port_categories(&ports_all, &sd.root, rtr);
                }
                if v_step != 0 && !cats.is_empty() {
                    *selected = wrap(*selected, v_step, cats.len());
                    let visible = 10usize;
                    if *selected < *scroll {
                        *scroll = *selected;
                    }
                    if *selected >= *scroll + visible {
                        *scroll = *selected + 1 - visible;
                    }
                }
                if confirm && let Some((cat, _)) = cats.get(*selected) {
                    // top-level "Ready to Play" opens its own genre sub-level
                    if !rtr && cat == "Ready to Play" {
                        next_screen = Some(Screen::PortCats {
                            cats: port_categories(&ports_all, &sd.root, true),
                            selected: 0,
                            scroll: 0,
                            rtr: true,
                        });
                        v_rep.clear();
                    } else {
                        // within a scope, "All …" = the whole pool, a genre
                        // name = that genre; the scope (rtr) narrows the pool
                        let key = cat.to_lowercase();
                        let all_here = cat == "All Ports" || cat == "All Ready to Play";
                        let filtered: Vec<kui_store::ports::PortEntry> = ports_all
                            .ports
                            .iter()
                            .filter(|p| !rtr || p.rtr)
                            .filter(|p| {
                                if cat == "Installed" {
                                    kui_store::ports::installed(&sd.root, p)
                                } else if all_here {
                                    true
                                } else {
                                    p.genres.contains(&key)
                                }
                            })
                            .cloned()
                            .collect();
                        next_screen = Some(Screen::Ports {
                            ports: filtered,
                            title: cat.clone(),
                            selected: 0,
                            scroll: 0,
                            cat_sel: *selected,
                            rtr,
                        });
                        v_rep.clear();
                    }
                }
                if back {
                    if let Ok(mut m) = store_msg.lock() {
                        m.clear();
                    }
                    if rtr {
                        // leaving the Ready to Play sub-level: back to the top
                        next_screen = Some(Screen::PortCats {
                            cats: port_categories(&ports_all, &sd.root, false),
                            selected: 0,
                            scroll: 0,
                            rtr: false,
                        });
                        v_rep.clear();
                    } else {
                        if ports_dirty {
                            // library changed: rescan platforms + rebuild the
                            // carousel so new ports appear without a reboot
                            ports_dirty = false;
                            platforms = sd.scan_platforms();
                            retain_launchable(&mut platforms, &sd, &cfg);
                            let (t2, d2) = build_tiles(platforms.len());
                            tiles = t2;
                            let _ = d2;
                            tile = tile.min(tiles.len().saturating_sub(1));
                            bg.clear();
                            logo.clear();
                            fbg.retain(|k, _| *k == ROOT_FBG);
                            boxart.clear();
                            infos.clear();
                            remember.clear();
                            request_carousel_art(
                                &loader, &mut bg, &mut logo, &sd, &tiles, &platforms, tile,
                            );
                        }
                        next_screen = Some(Screen::HubIndex {
                            selected: hub_pos(&hub_pages, "Ports"),
                        });
                        v_rep.clear();
                    }
                }
            }
            Screen::Ports { ports, title: _, selected, scroll, cat_sel, rtr } => {
                if v_step != 0 && !ports.is_empty() {
                    *selected = wrap(*selected, v_step, ports.len());
                    let visible = pakdek_visible_rows();
                    if *selected < *scroll {
                        *scroll = *selected;
                    }
                    if *selected >= *scroll + visible {
                        *scroll = *selected + 1 - visible;
                    }
                }
                if h_step != 0 && !ports.is_empty() {
                    let visible = pakdek_visible_rows();
                    let target =
                        *selected as i64 + h_step as i64 * visible as i64;
                    *selected =
                        target.clamp(0, ports.len() as i64 - 1) as usize;
                    if *selected < *scroll {
                        *scroll = *selected;
                    }
                    if *selected >= *scroll + visible {
                        *scroll = *selected + 1 - visible;
                    }
                }
                // A queues an install for an available port; the uninstall
                // button queues a removal for an installed one. Both return at
                // once — one background worker drains the queue FIFO — so you
                // can select many in a row. An already-queued port is skipped
                // so a double-press can't double-enqueue.
                if confirm
                    && let Some(p) = ports.get(*selected)
                    && !kui_store::ports::installed(&sd.root, p)
                    && port_jobs
                        .lock()
                        .map(|mut m| {
                            let dup = m.contains_key(&p.zip_name);
                            if !dup {
                                m.insert(p.zip_name.clone(), "Queued".into());
                            }
                            !dup
                        })
                        .unwrap_or(false)
                {
                    let _ = job_tx.send(PortJob::Install(p.clone(), ports_all.clone()));
                    ports_dirty = true;
                }
                if wipe_btn
                    && let Some(p) = ports.get(*selected)
                    && kui_store::ports::installed(&sd.root, p)
                    && port_jobs
                        .lock()
                        .map(|mut m| {
                            let dup = m.contains_key(&p.zip_name);
                            if !dup {
                                m.insert(p.zip_name.clone(), "Queued".into());
                            }
                            !dup
                        })
                        .unwrap_or(false)
                {
                    let _ = job_tx.send(PortJob::Remove(p.clone()));
                    ports_dirty = true;
                }
                if back {
                    if let Ok(mut m) = store_msg.lock() {
                        m.clear();
                    }
                    let cats2 = port_categories(&ports_all, &sd.root, *rtr);
                    let sel2 = (*cat_sel).min(cats2.len().saturating_sub(1));
                    next_screen = Some(Screen::PortCats {
                        cats: cats2,
                        selected: sel2,
                        scroll: sel2.saturating_sub(9),
                        rtr: *rtr,
                    });
                    v_rep.clear();
                }
            }
            Screen::Updater { releases, selected } => {
                if releases.is_empty()
                    && let Ok(mut g) = rel_fetch.lock()
                    && let Some(res) = g.take()
                {
                    match res {
                        Ok(list) => *releases = list,
                        Err(e) => {
                            if let Ok(mut m) = store_msg.lock() {
                                *m = format!("Fetch failed: {e}");
                            }
                        }
                    }
                }
                if v_step != 0 && !releases.is_empty() {
                    *selected = wrap(*selected, v_step, releases.len());
                }
                let busy = store_msg
                    .lock()
                    .map(|m| m.ends_with("...") && !m.contains("failed"))
                    .unwrap_or(false);
                if confirm
                    && !busy
                    && let Some(rel) = releases.get(*selected)
                {
                    if let Some(url) = rel.ota_url.clone() {
                        let msg = store_msg.clone();
                        let tag = rel.tag.clone();
                        if let Ok(mut m) = msg.lock() {
                            *m = format!("Downloading {tag}...");
                        }
                        std::thread::spawn(move || {
                            let set = |s2: String| {
                                if let Ok(mut m) = msg.lock() {
                                    *m = s2;
                                }
                            };
                            match kui_store::download_and_stage(&url, |_| {}) {
                                Ok(staged) => {
                                    set("Installing...".into());
                                    match kui_store::apply_staged(&staged) {
                                        Ok(()) => {
                                            set("Update installed — rebooting...".into());
                                            std::thread::sleep(
                                                std::time::Duration::from_secs(2),
                                            );
                                            let _ = std::fs::write("/tmp/reboot", "");
                                            std::process::exit(0);
                                        }
                                        Err(e) => set(format!("Install failed: {e}")),
                                    }
                                }
                                Err(e) => set(format!("Download failed: {e}")),
                            }
                        });
                    } else if let Ok(mut m) = store_msg.lock() {
                        *m = "No download for this release".into();
                    }
                }
                if back && !busy {
                    if let Ok(mut m) = store_msg.lock() {
                        m.clear();
                    }
                    next_screen = Some(Screen::HubIndex {
                        selected: hub_pos(&hub_pages, "Updater"),
                    });
                    v_rep.clear();
                }
            }
            Screen::ScraperPlatforms { rows, selected, scroll } => {
                if v_step != 0 && !rows.is_empty() {
                    *selected = wrap(*selected, v_step, rows.len());
                    let visible = 10usize;
                    if *selected < *scroll {
                        *scroll = *selected;
                    }
                    if *selected >= *scroll + visible {
                        *scroll = *selected + 1 - visible;
                    }
                }
                if confirm && let Some((label, dir)) = rows.get(*selected) {
                    next_screen = Some(Screen::ScraperMenu {
                        label: label.clone(),
                        dir: dir.clone(),
                        selected: 0,
                    });
                    v_rep.clear();
                }
                if back {
                    next_screen = Some(Screen::HubIndex {
                        selected: hub_pos(&hub_pages, "Scraper"),
                    });
                    v_rep.clear();
                }
            }
            Screen::PrefetchPlatforms { rows, selected, scroll, ret } => {
                if v_step != 0 && !rows.is_empty() {
                    *selected = wrap(*selected, v_step, rows.len());
                    let visible = 10usize;
                    if *selected < *scroll {
                        *scroll = *selected;
                    }
                    if *selected >= *scroll + visible {
                        *scroll = *selected + 1 - visible;
                    }
                }
                if confirm && let Some((_, tag_sel)) = rows.get(*selected) {
                    let user = cfg.get_or("ra.user", "").to_string();
                    let token = cfg.get_or("ra.token", "").to_string();
                    let roms: Vec<PathBuf> = platforms
                        .iter()
                        .filter(|p| {
                            tag_sel.as_deref().map(|t| t == p.tag).unwrap_or(true)
                        })
                        .flat_map(|p| {
                            p.roms.iter().map(|r2| p.dir.join(r2)).collect::<Vec<_>>()
                        })
                        .collect();
                    let pf = ra_pf.clone();
                    let cancel = ra_pf_cancel.clone();
                    cancel.store(false, std::sync::atomic::Ordering::Relaxed);
                    if let Ok(mut g) = pf.lock() {
                        *g = (0, roms.len(), "Starting...".into(), false);
                    }
                    std::thread::spawn(move || {
                        let mut cached = 0usize;
                        let total = roms.len();
                        for (i, rom) in roms.iter().enumerate() {
                            if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                                if let Ok(mut g) = pf.lock() {
                                    *g = (
                                        i,
                                        total,
                                        format!("Cancelled ({cached} cached)"),
                                        true,
                                    );
                                }
                                return;
                            }
                            if kui_ra::prefetch_game(&user, &token, rom).is_ok() {
                                cached += 1;
                            }
                            if let Ok(mut g) = pf.lock() {
                                *g = (i + 1, total, format!("{cached} cached"), false);
                            }
                        }
                        if let Ok(mut g) = pf.lock() {
                            *g = (
                                total,
                                total,
                                format!("Done — {cached} games cached"),
                                true,
                            );
                        }
                    });
                    next_screen =
                        Some(Screen::HubPage { page: ret.0, selected: ret.1 });
                    v_rep.clear();
                }
                if back {
                    next_screen =
                        Some(Screen::HubPage { page: ret.0, selected: ret.1 });
                    v_rep.clear();
                }
            }
            Screen::CheatPlatforms { rows, selected, scroll, ret } => {
                if v_step != 0 && !rows.is_empty() {
                    *selected = wrap(*selected, v_step, rows.len());
                    let visible = 10usize;
                    if *selected < *scroll {
                        *scroll = *selected;
                    }
                    if *selected >= *scroll + visible {
                        *scroll = *selected + 1 - visible;
                    }
                }
                if confirm && let Some((_, tag_sel)) = rows.get(*selected) {
                    // (tag, Cheats/<TAG>, rom stems) per covered platform;
                    // existing .cht files are never re-fetched
                    let cheats_root = sd.root.join("Cheats");
                    let groups: Vec<(String, PathBuf, Vec<String>)> = platforms
                        .iter()
                        .filter(|p| kui_cheatdl::has_db(&p.tag))
                        .filter(|p| {
                            tag_sel.as_deref().map(|t| t == p.tag).unwrap_or(true)
                        })
                        .map(|p| {
                            (
                                p.tag.clone(),
                                cheats_root.join(&p.tag),
                                p.roms
                                    .iter()
                                    .map(|r2| {
                                        Path::new(r2)
                                            .file_stem()
                                            .map(|s| s.to_string_lossy().into_owned())
                                            .unwrap_or_else(|| r2.clone())
                                    })
                                    .collect(),
                            )
                        })
                        .collect();
                    let total: usize = groups.iter().map(|(.., s)| s.len()).sum();
                    let pf = cheat_pf.clone();
                    let cancel = cheat_pf_cancel.clone();
                    cancel.store(false, std::sync::atomic::Ordering::Relaxed);
                    if let Ok(mut g) = pf.lock() {
                        *g = (0, total, "Starting...".into(), false);
                    }
                    std::thread::spawn(move || {
                        let (mut got, mut done) = (0usize, 0usize);
                        for (tag2, dir2, stems) in groups {
                            let ix = kui_cheatdl::index(&tag2).ok();
                            for stem in stems {
                                if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                                    if let Ok(mut g) = pf.lock() {
                                        *g = (
                                            done,
                                            total,
                                            format!("Cancelled ({got} fetched)"),
                                            true,
                                        );
                                    }
                                    return;
                                }
                                done += 1;
                                let dest = dir2.join(format!("{stem}.cht"));
                                if !dest.exists()
                                    && let Some(ix2) = &ix
                                    && let Some(f2) = ix2.best_match(&stem)
                                    && ix2.fetch_into(&f2, &dest).is_ok()
                                {
                                    got += 1;
                                }
                                if let Ok(mut g) = pf.lock() {
                                    *g = (done, total, format!("{got} fetched"), false);
                                }
                            }
                        }
                        if let Ok(mut g) = pf.lock() {
                            *g =
                                (total, total, format!("Done — {got} cheat files"), true);
                        }
                    });
                    next_screen =
                        Some(Screen::HubPage { page: ret.0, selected: ret.1 });
                    v_rep.clear();
                }
                if back {
                    next_screen =
                        Some(Screen::HubPage { page: ret.0, selected: ret.1 });
                    v_rep.clear();
                }
            }
            Screen::ScraperMenu { label, dir, selected } => {
                let n2 = SCRAPER_ACTIONS.len();
                if v_step != 0 {
                    *selected = wrap(*selected, v_step, n2);
                }
                if confirm {
                    let job = match *selected {
                        0 => scraper::Job::DownloadMissing,
                        1 => scraper::Job::ImagesOnly,
                        2 => scraper::Job::MetadataOnly,
                        3 => scraper::Job::DownloadAll,
                        4 => scraper::Job::PatchImages,
                        _ => scraper::Job::DeleteArtwork,
                    };
                    let s2 = scraper::Scraper::start(
                        &sd.root.join("Roms"),
                        dir.clone(),
                        job,
                    );
                    next_screen = Some(Screen::ScraperRun {
                        job: s2,
                        label: label.clone(),
                        dir: dir.clone(),
                        menu_sel: *selected,
                    });
                    v_rep.clear();
                }
                if back {
                    let mut rows: Vec<(String, Option<PathBuf>)> =
                        vec![("All Platforms".into(), None)];
                    for p in &platforms {
                        rows.push((p.display.clone(), Some(p.dir.clone())));
                    }
                    let sel = rows
                        .iter()
                        .position(|(_, d)| d == dir)
                        .unwrap_or(0);
                    next_screen = Some(Screen::ScraperPlatforms {
                        rows,
                        selected: sel,
                        scroll: 0,
                    });
                    v_rep.clear();
                }
            }
            Screen::ScraperRun { job, label, dir, menu_sel } => {
                if back {
                    job.cancel();
                    next_screen = Some(Screen::ScraperMenu {
                        label: label.clone(),
                        dir: dir.clone(),
                        selected: *menu_sel,
                    });
                    v_rep.clear();
                }
            }
            Screen::Files { panes, active, menu, armed_delete } => {
                // an async paste/delete finished → re-read both panes so the
                // changed entry appears/vanishes (selection/scroll clamped)
                if files_dirty.swap(false, std::sync::atomic::Ordering::Relaxed) {
                    for p in panes.iter_mut() {
                        p.rows = file_rows(&p.dir);
                        p.selected = p.selected.min(p.rows.len().saturating_sub(1));
                        p.scroll = p.scroll.min(p.selected);
                    }
                }
                if let Some(mi) = menu {
                    // START action menu is open; acts on the active pane
                    let acts = file_actions(&file_clip);
                    if v_step != 0 {
                        *mi = wrap(*mi, v_step, acts.len());
                        *armed_delete = false;
                    }
                    if confirm {
                        let pane = &panes[*active];
                        match acts[*mi] {
                            "Copy" | "Cut" => {
                                if let Some(row) = pane.rows.get(pane.selected) {
                                    file_clip =
                                        Some((row.path.clone(), acts[*mi] == "Cut"));
                                }
                                *menu = None;
                            }
                            "Paste" => {
                                // recursive copy/move on a thread so a big
                                // folder never freezes the browser
                                if let Some((src, cut)) = file_clip.take() {
                                    let dest = pane.dir.clone();
                                    let busy = files_busy.clone();
                                    let dirty = files_dirty.clone();
                                    busy.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                    if let Ok(mut vb) = files_verb.lock() {
                                        *vb = "Pasting…";
                                    }
                                    std::thread::spawn(move || {
                                        let _ = file_paste(&src, &dest, cut);
                                        dirty.store(true, std::sync::atomic::Ordering::Relaxed);
                                        busy.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                                    });
                                }
                                *menu = None;
                            }
                            "Rename" => {
                                if let Some(row) = pane.rows.get(pane.selected) {
                                    next_screen = Some(Screen::Osk {
                                        buf: row.name.clone(),
                                        pos: 0,
                                        target: OskTarget::FileRename {
                                            path: row.path.clone(),
                                            dirs: [
                                                panes[0].dir.clone(),
                                                panes[1].dir.clone(),
                                            ],
                                            active: *active,
                                        },
                                    });
                                }
                                *menu = None;
                            }
                            "New Folder" => {
                                next_screen = Some(Screen::Osk {
                                    buf: String::new(),
                                    pos: 0,
                                    target: OskTarget::NewFolder {
                                        dirs: [
                                            panes[0].dir.clone(),
                                            panes[1].dir.clone(),
                                        ],
                                        active: *active,
                                    },
                                });
                                *menu = None;
                            }
                            "Delete" => {
                                if *armed_delete {
                                    // recursive remove on a thread; the row
                                    // vanishes when the pane refreshes on done
                                    if let Some(row) = pane.rows.get(pane.selected) {
                                        let path = row.path.clone();
                                        let is_dir = row.is_dir;
                                        let busy = files_busy.clone();
                                        let dirty = files_dirty.clone();
                                        busy.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                        if let Ok(mut vb) = files_verb.lock() {
                                            *vb = "Deleting…";
                                        }
                                        std::thread::spawn(move || {
                                            let _ = if is_dir {
                                                std::fs::remove_dir_all(&path)
                                            } else {
                                                std::fs::remove_file(&path)
                                            };
                                            dirty.store(true, std::sync::atomic::Ordering::Relaxed);
                                            busy.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                                        });
                                    }
                                    *armed_delete = false;
                                    *menu = None;
                                } else {
                                    *armed_delete = true;
                                }
                            }
                            "Quit" => {
                                next_screen = Some(Screen::HubIndex {
                                    selected: hub_pos(&hub_pages, "Files"),
                                });
                                *menu = None;
                            }
                            _ => {}
                        }
                        // both panes may show the touched folder
                        if menu.is_none() {
                            for p in panes.iter_mut() {
                                p.rows = file_rows(&p.dir);
                                p.selected =
                                    p.selected.min(p.rows.len().saturating_sub(1));
                                p.scroll = p.scroll.min(p.selected);
                            }
                        }
                    }
                    if back || start_btn {
                        *menu = None;
                        *armed_delete = false;
                    }
                } else {
                    if h_step != 0 {
                        *active = if h_step < 0 { 0 } else { 1 };
                    }
                    let pane = &mut panes[*active];
                    if v_step != 0 && !pane.rows.is_empty() {
                        pane.selected = (pane.selected as i32 + v_step)
                            .clamp(0, pane.rows.len() as i32 - 1)
                            as usize;
                        let visible = 10usize;
                        if pane.selected < pane.scroll {
                            pane.scroll = pane.selected;
                        }
                        if pane.selected >= pane.scroll + visible {
                            pane.scroll = pane.selected + 1 - visible;
                        }
                    }
                    if confirm
                        && let Some(row) = pane.rows.get(pane.selected)
                        && row.is_dir
                    {
                        pane.dir = row.path.clone();
                        pane.rows = file_rows(&pane.dir);
                        pane.selected = 0;
                        pane.scroll = 0;
                    }
                    if start_btn {
                        // opens in empty folders too: Paste/New Folder/Quit
                        // don't need a selection (the rest are row-guarded)
                        *menu = Some(0);
                        *armed_delete = false;
                    }
                    if back {
                        if pane.dir == sd.root {
                            next_screen = Some(Screen::HubIndex {
                                selected: hub_pos(&hub_pages, "Files"),
                            });
                        } else {
                            let child = pane.dir.clone();
                            if let Some(parent) = pane.dir.parent() {
                                pane.dir = parent.to_path_buf();
                            }
                            pane.rows = file_rows(&pane.dir);
                            pane.selected = pane
                                .rows
                                .iter()
                                .position(|r2| r2.path == child)
                                .unwrap_or(0);
                            pane.scroll = pane.selected.saturating_sub(9);
                        }
                        v_rep.clear();
                    }
                }
            }
            Screen::PortForge { pane } => {
                if v_step != 0 && !pane.rows.is_empty() {
                    pane.selected =
                        (pane.selected as i32 + v_step).clamp(0, pane.rows.len() as i32 - 1) as usize;
                    let visible = 10usize;
                    if pane.selected < pane.scroll {
                        pane.scroll = pane.selected;
                    }
                    if pane.selected >= pane.scroll + visible {
                        pane.scroll = pane.selected + 1 - visible;
                    }
                }
                if confirm && let Some(row) = pane.rows.get(pane.selected) {
                    if row.is_package {
                        // a Port Forge Web package → install it (move into
                        // place). Reuses the forge progress screen; no delete
                        // prompt (install consumes the package, no leftover).
                        let src = row.path.clone();
                        let root = sd.root.clone();
                        let msg = store_msg.clone();
                        let dirty = forge_dirty.clone();
                        let pct = forge_pct.clone();
                        if let Ok(mut m) = msg.lock() {
                            *m = "Preparing…".into();
                        }
                        if let Ok(mut d) = forge_del.lock() {
                            *d = None;
                        }
                        if let Ok(mut p) = forge_pct.lock() {
                            *p = 0.0;
                        }
                        std::thread::spawn(move || {
                            let r = portforge::install_package(&root, &src, &mut |frac, s| {
                                if let Ok(mut m) = msg.lock() {
                                    *m = s.to_string();
                                }
                                if let Ok(mut p) = pct.lock() {
                                    *p = frac;
                                }
                            });
                            if let Ok(mut m) = msg.lock() {
                                *m = match r {
                                    Ok(t) => {
                                        dirty.store(true, std::sync::atomic::Ordering::Relaxed);
                                        format!("Done — {t} is ready in Ports.")
                                    }
                                    Err(e) => format!("Failed: {e}"),
                                };
                            }
                        });
                        next_screen = Some(Screen::PortForgeRun { source: row.path.clone() });
                    } else if row.is_game {
                        // detected RPG Maker game → close the picker, hand off
                        // to the progress screen, and start the forge thread
                        let src = row.path.clone();
                        let root = sd.root.clone();
                        let msg = store_msg.clone();
                        let dirty = forge_dirty.clone();
                        let del = forge_del.clone();
                        let pct = forge_pct.clone();
                        if let Ok(mut m) = msg.lock() {
                            *m = "Preparing…".into();
                        }
                        if let Ok(mut d) = forge_del.lock() {
                            *d = None;
                        }
                        if let Ok(mut p) = forge_pct.lock() {
                            *p = 0.0;
                        }
                        std::thread::spawn(move || {
                            let r = portforge::forge(&root, &src, &mut |frac, s| {
                                if let Ok(mut m) = msg.lock() {
                                    *m = s.to_string();
                                }
                                if let Ok(mut p) = pct.lock() {
                                    *p = frac;
                                }
                            });
                            if let Ok(mut m) = msg.lock() {
                                *m = match r {
                                    Ok(t) => {
                                        dirty.store(true, std::sync::atomic::Ordering::Relaxed);
                                        if let Ok(mut d) = del.lock() {
                                            *d = Some(src.clone());
                                        }
                                        format!("Done — {t} is ready in Ports.")
                                    }
                                    Err(e) => format!("Failed: {e}"),
                                };
                            }
                        });
                        next_screen = Some(Screen::PortForgeRun { source: row.path.clone() });
                    } else {
                        // a plain folder → descend to keep browsing
                        pane.dir = row.path.clone();
                        pane.rows = dir_rows(&pane.dir);
                        pane.selected = 0;
                        pane.scroll = 0;
                    }
                    v_rep.clear();
                }
                if back {
                    if pane.dir == sd.root {
                        if let Ok(mut m) = store_msg.lock() {
                            m.clear();
                        }
                        next_screen = Some(Screen::HubIndex {
                            selected: hub_pos(&hub_pages, "Port Forge"),
                        });
                    } else {
                        let child = pane.dir.clone();
                        if let Some(parent) = pane.dir.parent() {
                            pane.dir = parent.to_path_buf();
                        }
                        pane.rows = dir_rows(&pane.dir);
                        pane.selected =
                            pane.rows.iter().position(|r2| r2.path == child).unwrap_or(0);
                        pane.scroll = pane.selected.saturating_sub(9);
                    }
                    v_rep.clear();
                }
            }
            Screen::PortForgeRun { .. } => {
                // Busy while the forge thread is still working (the status is a
                // progress line, not a terminal "Done"/"Failed" one).
                let busy = store_msg
                    .lock()
                    .map(|m| !m.is_empty() && !m.starts_with("Done") && !m.contains("Failed"))
                    .unwrap_or(false);
                let can_delete = forge_del.lock().map(|d| d.is_some()).unwrap_or(false);
                // On success: Y deletes the original folder (async, so the UI
                // shows a "Deleting…" message instead of freezing), B keeps it.
                // On failure: only B. Either exit routes to the Control Panel.
                let mut leave = false;
                if !busy && wipe_btn && can_delete {
                    if let Some(path) = forge_del.lock().ok().and_then(|mut d| d.take()) {
                        if let Ok(mut m) = store_msg.lock() {
                            *m = "Deleting original files…".into();
                        }
                        let done = forge_del_done.clone();
                        let msg = store_msg.clone();
                        std::thread::spawn(move || {
                            let _ = if path.is_dir() {
                                std::fs::remove_dir_all(&path)
                            } else {
                                std::fs::remove_file(&path)
                            };
                            if let Ok(mut m) = msg.lock() {
                                *m = "Done — original files deleted.".into();
                            }
                            done.store(true, std::sync::atomic::Ordering::Relaxed);
                        });
                    }
                } else if !busy && back {
                    if let Ok(mut d) = forge_del.lock() {
                        *d = None;
                    }
                    leave = true;
                }
                // the async delete finished → leave to the Control Panel
                if forge_del_done.swap(false, std::sync::atomic::Ordering::Relaxed) {
                    leave = true;
                }
                if leave {
                    // a forge happened: rescan so the new port appears on the
                    // carousel without a reboot (same as ports install)
                    if forge_dirty.swap(false, std::sync::atomic::Ordering::Relaxed) {
                        ports_dirty = false;
                        platforms = sd.scan_platforms();
                        retain_launchable(&mut platforms, &sd, &cfg);
                        let (t2, _d2) = build_tiles(platforms.len());
                        tiles = t2;
                        tile = tile.min(tiles.len().saturating_sub(1));
                        bg.clear();
                        logo.clear();
                        fbg.retain(|k, _| *k == ROOT_FBG);
                        boxart.clear();
                        infos.clear();
                        remember.clear();
                        request_carousel_art(
                            &loader, &mut bg, &mut logo, &sd, &tiles, &platforms, tile,
                        );
                    }
                    if let Ok(mut m) = store_msg.lock() {
                        m.clear();
                    }
                    next_screen = Some(Screen::HubIndex {
                        selected: hub_pos(&hub_pages, "Port Forge"),
                    });
                    v_rep.clear();
                }
            }
            Screen::InputTest => {
                // EV_SW switches never reach SDL; poll them directly
                if input_sw_at
                    .is_none_or(|t| now - t >= std::time::Duration::from_millis(200))
                {
                    let (fnv, jack) = tg5040::switch_states();
                    if fnv.is_some() {
                        input_fn = fnv;
                    }
                    if jack.is_some() {
                        input_jack = jack;
                    }
                    input_sw_at = Some(now);
                }
                // dpad state comes through the pad bits set by the dpad arm
                input_pressed[0] = v_rep.held[0][0];
                input_pressed[1] = v_rep.held[0][1];
                input_pressed[2] = h_rep.held[0][0];
                input_pressed[3] = h_rep.held[0][1];
                if let Some(t) = input_menu_hold
                    && now - t >= std::time::Duration::from_millis(900)
                {
                    input_menu_hold = None;
                    next_screen = Some(Screen::HubIndex {
                        selected: hub_pos(&hub_pages, "Input"),
                    });
                    v_rep.clear();
                }
            }
            Screen::Battery { span_h, .. } => {
                if h_step != 0 {
                    const SPANS: [u64; 4] = [6, 12, 24, 48];
                    let cur = SPANS.iter().position(|s2| s2 == span_h).unwrap_or(2);
                    *span_h = SPANS
                        [(cur as i32 + h_step).rem_euclid(SPANS.len() as i32) as usize];
                }
                if back {
                    next_screen = Some(Screen::HubIndex {
                        selected: hub_pos(&hub_pages, "Battery"),
                    });
                    v_rep.clear();
                }
            }
            Screen::GameTime { rows, selected, scroll, .. } => {
                // </> jumps a full page, up/down steps
                let step = v_step + h_step * visible_rows() as i32;
                if step != 0 && !rows.is_empty() {
                    *selected =
                        (*selected as i32 + step).clamp(0, rows.len() as i32 - 1) as usize;
                    let visible = 10usize;
                    if *selected < *scroll {
                        *scroll = *selected;
                    }
                    if *selected >= *scroll + visible {
                        *scroll = *selected + 1 - visible;
                    }
                }
                if back {
                    next_screen = Some(Screen::HubIndex {
                        selected: hub_pos(&hub_pages, "Game Tracker"),
                    });
                    v_rep.clear();
                }
            }
            Screen::Bt { devs, selected, scroll } => {
                let on = proc_running("bluetoothd");
                if on && devs.is_empty() && bt_scan_at.is_none() {
                    let _ = std::process::Command::new("sh")
                        .args([
                            "-c",
                            "(bluetoothctl --timeout 6 scan on) >/dev/null 2>&1 &",
                        ])
                        .spawn();
                    bt_scan_at = Some(now);
                }
                if let Some(t) = bt_scan_at
                    && now - t >= std::time::Duration::from_secs(7)
                {
                    *devs = bt_collect();
                    bt_scan_at = None;
                }
                let total = 1 + devs.len();
                if v_step != 0 {
                    *selected = wrap(*selected, v_step, total);
                    let visible = 10usize;
                    if *selected < *scroll {
                        *scroll = *selected;
                    }
                    if *selected >= *scroll + visible {
                        *scroll = *selected + 1 - visible;
                    }
                }
                if confirm {
                    if *selected == 0 {
                        let verb = if on { "stop" } else { "start" };
                        let _ = std::process::Command::new("sh")
                            .args([
                                "-c",
                                &format!(
                                    "/etc/bluetooth/bt_init.sh {verb} >/dev/null 2>&1 &"
                                ),
                            ])
                            .spawn();
                        cfg.set("radio.bluetooth", if on { "off" } else { "on" });
                        let _ = cfg.save();
                        devs.clear();
                        bt_scan_at = (!on).then(|| now + std::time::Duration::from_secs(4));
                    } else if let Some(dev) = devs.get(*selected - 1) {
                        let mac = dev.mac.clone();
                        let script = if dev.paired {
                            format!("bluetoothctl connect {mac}")
                        } else {
                            format!(
                                "bluetoothctl pair {mac} && bluetoothctl trust {mac} && bluetoothctl connect {mac}"
                            )
                        };
                        let _ = std::process::Command::new("sh")
                            .args(["-c", &format!("({script}) >/dev/null 2>&1 &")])
                            .spawn();
                        bt_scan_at = Some(now + std::time::Duration::from_secs(4));
                        devs.clear();
                        v_rep.clear();
                    }
                }
                if wipe_btn
                    && *selected > 0
                    && let Some(dev) = devs.get(*selected - 1)
                    && dev.paired
                {
                    let _ = std::process::Command::new("sh")
                        .args(["-c", &format!("bluetoothctl remove {} >/dev/null 2>&1", dev.mac)])
                        .status();
                    devs.clear();
                    bt_scan_at = Some(now);
                }
                if pin_btn {
                    devs.clear();
                    bt_scan_at = None;
                }
                if back {
                    next_screen = Some(Screen::HubPage {
                        page: hub_pos(&hub_pages, "Connectivity"),
                        selected: 1,
                    });
                    v_rep.clear();
                }
            }
            Screen::Osk { buf, pos, target } => {
                let n_chars = OSK_CHARS.chars().count();
                if h_step != 0 {
                    *pos = (*pos as i32 + h_step).rem_euclid(n_chars as i32) as usize;
                }
                if v_step != 0 {
                    let row = *pos / OSK_COLS;
                    let col = *pos % OSK_COLS;
                    let rows_n = n_chars.div_ceil(OSK_COLS);
                    let nr = (row as i32 + v_step).rem_euclid(rows_n as i32) as usize;
                    *pos = (nr * OSK_COLS + col).min(n_chars - 1);
                }
                if confirm
                    && let Some(c) = OSK_CHARS.chars().nth(*pos)
                    && buf.chars().count() < 63
                {
                    buf.push(c);
                }
                if pin_btn {
                    buf.pop();
                }
                if start_btn {
                    match target {
                        OskTarget::Collection => {
                            let name = buf.trim().to_string();
                            if !name.is_empty() {
                                let _ = sd.collection_create(&name);
                            }
                            next_screen =
                                Some(open_collections_index(&sd, &mut boxart, &mut infos, &platforms));
                        }
                        OskTarget::CollectionRename { dir } => {
                            let name = buf.trim();
                            if !name.is_empty()
                                && let Some(parent) = dir.parent().map(|p| p.to_path_buf())
                            {
                                let _ = std::fs::rename(&*dir, parent.join(name));
                            }
                            next_screen =
                                Some(open_collections_index(&sd, &mut boxart, &mut infos, &platforms));
                        }
                        OskTarget::WifiPass { ssid } => {
                            wifi_connect_spawn(ssid, Some(buf));
                            wifi_scan_at = Some(now + std::time::Duration::from_secs(4));
                            next_screen =
                                Some(Screen::Wifi { nets: Vec::new(), selected: 0, scroll: 0 });
                        }
                        OskTarget::FileRename { path, dirs, active } => {
                            let name = buf.trim();
                            if !name.is_empty()
                                && let Some(parent) = path.parent().map(|p| p.to_path_buf())
                            {
                                let _ = std::fs::rename(&*path, parent.join(name));
                            }
                            next_screen = Some(files_reopen(dirs, *active));
                        }
                        OskTarget::NewFolder { dirs, active } => {
                            let name = buf.trim();
                            if !name.is_empty() {
                                let _ = std::fs::create_dir_all(dirs[*active].join(name));
                            }
                            next_screen = Some(files_reopen(dirs, *active));
                        }
                        OskTarget::ConfigValue { key, page, row } => {
                            if buf.is_empty() {
                                cfg.remove_prefix(key);
                            } else {
                                cfg.set(key, buf.as_str());
                            }
                            let _ = cfg.save();
                            next_screen =
                                Some(Screen::HubPage { page: *page, selected: *row });
                        }
                    }
                    v_rep.clear();
                } else if back {
                    next_screen = Some(match target {
                        OskTarget::Collection | OskTarget::CollectionRename { .. } => {
                            open_collections_index(&sd, &mut boxart, &mut infos, &platforms)
                        }
                        OskTarget::WifiPass { .. } => {
                            Screen::Wifi { nets: Vec::new(), selected: 0, scroll: 0 }
                        }
                        OskTarget::ConfigValue { page, row, .. } => {
                            Screen::HubPage { page: *page, selected: *row }
                        }
                        OskTarget::FileRename { dirs, active, .. } => {
                            files_reopen(dirs, *active)
                        }
                        OskTarget::NewFolder { dirs, active } => files_reopen(dirs, *active),
                    });
                    v_rep.clear();
                }
            }
            Screen::BootLogo { idx } => {
                let logos = sd.bootlogos();
                if !logos.is_empty() && h_step != 0 {
                    *idx = (*idx as i32 + h_step).rem_euclid(logos.len() as i32) as usize;
                    if let Some(t) = bootlogo_tex.take() {
                        r.drop_texture(&v.gl, t);
                    }
                }
                if confirm
                    && on_device
                    && let Some(p) = logos.get(*idx)
                {
                    let _ = std::process::Command::new("sh")
                        .args([
                            "-c",
                            &format!(
                                "mkdir -p /mnt/boot && mount -t vfat /dev/mmcblk0p1 /mnt/boot && cp '{}' /mnt/boot/bootlogo.bmp && sync && umount /mnt/boot",
                                p.display()
                            ),
                        ])
                        .status();
                    bootlogo_applied = true;
                }
                if back {
                    if let Some(t) = bootlogo_tex.take() {
                        r.drop_texture(&v.gl, t);
                    }
                    bootlogo_applied = false;
                    next_screen = Some(Screen::HubIndex {
                        selected: hub_pos(&hub_pages, "Boot Logo"),
                    });
                    v_rep.clear();
                }
            }
            Screen::Themes { idx } => {
                let variants = sd.theme_variants();
                if !variants.is_empty() && h_step != 0 {
                    *idx = (*idx as i32 + h_step).rem_euclid(variants.len() as i32) as usize;
                    if let Some(t) = themes_tex.take() {
                        r.drop_texture(&v.gl, t);
                    }
                }
                if confirm && let Some((_, path)) = variants.get(*idx)
                    && sd.theme_apply(path).is_ok() && on_device {
                        // relaunch to rebuild the resident carousel art
                        return 0;
                    }
                if pin_btn {
                    // X: back to the default look (colors + font)
                    cfg.remove_prefix("theme.color");
                    cfg.remove_prefix("theme.font");
                    let _ = cfg.save();
                    theme = sd::Theme::from_config(&cfg);
                }
                if back {
                    if let Some(t) = themes_tex.take() {
                        r.drop_texture(&v.gl, t);
                    }
                    next_screen = Some(Screen::HubIndex {
                        selected: hub_pos(&hub_pages, "Themes"),
                    });
                    v_rep.clear();
                }
            }
            Screen::Switcher { entries, idx, .. } => {
                if h_step != 0 && !entries.is_empty() {
                    *idx = wrap(*idx, h_step, entries.len());
                }
                let mut close = false;
                if wipe_btn && !entries.is_empty() {
                    sd.remove_recent(&entries[*idx].rom);
                    entries.remove(*idx);
                    for (_, t) in switcher_art.drain() {
                        if let Some(t) = t {
                            r.drop_texture(&v.gl, t);
                        }
                    }
                    if entries.is_empty() {
                        close = true;
                    } else if *idx >= entries.len() {
                        *idx = entries.len() - 1;
                    }
                }
                if confirm
                    && let Some(ent) = entries.get(*idx)
                {
                    // launch-origin memory: leaving this game returns here
                    let _ = std::fs::write("/tmp/kui_switcher", "");
                    match launch_rom(&sd, &cfg, &ent.rom, &ent.alias, on_device) {
                        LaunchResult::Started(code) => return code,
                        LaunchResult::Fail(msg) => {
                            toast = Some((msg, now_hint() + TOAST_TIME));
                        }
                        LaunchResult::NoOp => {}
                    }
                    let _ = std::fs::remove_file("/tmp/kui_switcher");
                }
                if back || close {
                    if let Screen::Switcher { prev, .. } =
                        std::mem::replace(&mut screen, Screen::Carousel)
                    {
                        screen = *prev;
                    }
                    v_rep.clear();
                }
            }
            Screen::Quick { items, selected, probed, pending, .. } => {
                // Radios take seconds to change (BT waits for hci0 for up to
                // ~7s). Labels re-probe every second; an in-flight change
                // keeps "..." until the TARGET state is observed or a 12s
                // deadline passes — the label never claims a stale state
                // mid-transition.
                if on_device && now - *probed > std::time::Duration::from_secs(1) {
                    *probed = now;
                    for it in items.iter_mut() {
                        let is_wifi = match it.action {
                            QuickAction::Wifi => true,
                            QuickAction::Bluetooth => false,
                            _ => continue,
                        };
                        let name = if is_wifi { "WiFi" } else { "Bluetooth" };
                        let daemon = if is_wifi { "wpa_supplicant" } else { "bluetoothd" };
                        let on = proc_running(daemon);
                        it.label = name.into();
                        match *pending {
                            Some((pw, target, deadline)) if pw == is_wifi => {
                                if on == target || now > deadline {
                                    it.value = Some(if on { "On" } else { "Off" }.into());
                                    *pending = None;
                                } else {
                                    it.value = Some("...".into());
                                }
                            }
                            _ => {
                                it.value = Some(if on { "On" } else { "Off" }.into());
                            }
                        }
                    }
                }
                if v_step != 0 && !items.is_empty() {
                    *selected = wrap(*selected, v_step, items.len());
                }
                if back {
                    // close, restoring what was underneath
                    if let Screen::Quick { prev, .. } =
                        std::mem::replace(&mut screen, Screen::Carousel)
                    {
                        screen = *prev;
                    }
                } else if confirm && !items.is_empty() {
                    match items[*selected].action {
                        QuickAction::Collections => {
                            next_screen =
                                Some(open_collections_index(&sd, &mut boxart, &mut infos, &platforms));
                            v_rep.clear();
                        }
                        QuickAction::Recents => {
                            boxart.clear();
                            infos.clear();
                            next_screen = Some(Screen::List {
                                kind: ListKind::Recents,
                                rows: sd
                                    .recents()
                                    .into_iter()
                                    .map(|rc| Row {
                                        label: rc.alias,
                                        action: RowAction::Launch(rc.rom),
                                    })
                                    .collect(),
                                selected: 0,
                                scroll: 0,
                                show_art: true,
                                tag: None,
                            });
                            v_rep.clear();
                        }
                        QuickAction::Settings => {
                            next_screen = Some(Screen::HubIndex { selected: 0 });
                            v_rep.clear();
                        }
                        QuickAction::Wifi | QuickAction::Bluetooth => {
                            // The verb follows the LABEL (what the user sees),
                            // not an instant probe — pressing mid-transition
                            // ("...") is ignored, so teardown races can't flip
                            // the action to the wrong direction.
                            let is_wifi =
                                matches!(items[*selected].action, QuickAction::Wifi);
                            // the verb follows the VALUE the user sees, not an
                            // instant probe; pressing mid-transition ("...")
                            // is ignored so teardown races can't misfire
                            let verb = match items[*selected].value.as_deref() {
                                Some("Off") => Some("start"),
                                Some("On") => Some("stop"),
                                _ => None,
                            };
                            if let Some(verb) = verb {
                                if on_device {
                                    let script = if is_wifi {
                                        "/etc/wifi/wifi_init.sh"
                                    } else {
                                        "/etc/bluetooth/bt_init.sh"
                                    };
                                    let _ = std::process::Command::new("sh")
                                        .args([
                                            "-c",
                                            &format!("{script} {verb} >/dev/null 2>&1 &"),
                                        ])
                                        .spawn();
                                    // persist: kuid restores this state at boot
                                    cfg.set(
                                        if is_wifi { "radio.wifi" } else { "radio.bluetooth" },
                                        if verb == "start" { "on" } else { "off" },
                                    );
                                    let _ = cfg.save();
                                    items[*selected].value = Some("...".into());
                                    *pending = Some((
                                        is_wifi,
                                        verb == "start",
                                        now + std::time::Duration::from_secs(12),
                                    ));
                                } else {
                                    println!("radio toggle (desktop no-op)");
                                }
                            }
                        }
                        QuickAction::Reboot => {
                            if on_device && std::fs::File::create("/tmp/reboot").is_ok() {
                                return 0;
                            }
                            println!("reboot (desktop no-op)");
                        }
                        QuickAction::Poweroff => {
                            if on_device && std::fs::File::create("/tmp/poweroff").is_ok() {
                                return 0;
                            }
                            println!("poweroff (desktop no-op)");
                        }
                    }
                }
            }
            Screen::List { kind, rows, selected, scroll, show_art, .. } => {
                if let ListKind::Platform(pi) = kind
                    && s_step != 0
                    && !platforms.is_empty()
                {
                    // L1/R1: hop platforms without leaving the list;
                    // park the cursor so hopping back restores it
                    remember.insert(tile, (*selected, *scroll));
                    let np = wrap(*pi, s_step, platforms.len());
                    tile = tiles
                        .iter()
                        .position(|t| matches!(t, Tile::Platform(x) if *x == np))
                        .unwrap_or_else(|| {
            tiles.iter().position(|t| matches!(t, Tile::Dude)).unwrap_or(0)
        });
                    next_screen = open_tile_mode(
                        &sd, &loader, &mut fbg, &mut boxart, &mut infos, &platforms, &tiles,
                        tile, ui_mode, &remember,
                    );
                }
                let len = rows.len();
                let visible = visible_rows();
                if v_step != 0 && len > 0 {
                    *selected = wrap(*selected, v_step, len);
                }
                if h_step != 0 && len > 0 {
                    let target = *selected as i64 + h_step as i64 * visible as i64;
                    *selected = target.clamp(0, len as i64 - 1) as usize;
                }
                if v_step != 0 || h_step != 0 || confirm || back {
                    wipe_armed = None;
                }
                if (v_step != 0 || h_step != 0) && len > 0 {
                    if *selected < *scroll {
                        *scroll = *selected;
                    }
                    if *selected >= *scroll + visible {
                        *scroll = *selected + 1 - visible;
                    }
                }
                if back {
                    if !matches!(kind, ListKind::Root | ListKind::Collection(_) | ListKind::SmartCollection | ListKind::PickPlatform(_) | ListKind::PickGame(_)) {
                        remember.insert(tile, (*selected, *scroll));
                    }
                    v_rep.clear();
                    h_rep.clear();
                    match kind {
                        ListKind::Root => {} // B at root: no-op (Select leaves)
                        ListKind::Collection(_) | ListKind::SmartCollection => {
                            next_screen =
                                Some(open_collections_index(&sd, &mut boxart, &mut infos, &platforms));
                        }
                        ListKind::PickPlatform(col) => {
                            let col = col.clone();
                            next_screen = Some(open_collection_detail(&sd, &col));
                        }
                        ListKind::PickGame(col) => {
                            let col = col.clone();
                            next_screen = Some(open_pick_platform(&platforms, &col));
                        }
                        // quick-menu surfaces return to the quick menu
                        ListKind::Recents => {
                            next_screen = Some(quick_return(
                                &sd,
                                on_device,
                                now,
                                root_screen(ui_mode, &platforms, &tiles, tile),
                                QuickAction::Recents,
                            ));
                        }
                        ListKind::CollectionsIndex => {
                            next_screen = Some(quick_return(
                                &sd,
                                on_device,
                                now,
                                root_screen(ui_mode, &platforms, &tiles, tile),
                                QuickAction::Collections,
                            ));
                        }
                        _ => next_screen = Some(root_screen(ui_mode, &platforms, &tiles, tile)),
                    }
                } else if confirm && len > 0 {
                    match &rows[*selected].action {
                        RowAction::OpenTile(t) => {
                            let t = *t;
                            tile = t;
                            next_screen = open_tile_mode(
                                &sd, &loader, &mut fbg, &mut boxart, &mut infos, &platforms,
                                &tiles, tile, ui_mode, &remember,
                            );
                            v_rep.clear();
                        }
                        RowAction::Launch(rom) => {
                            match launch_rom(&sd, &cfg, rom, &rows[*selected].label, on_device) {
                                LaunchResult::Started(code) => return code,
                                LaunchResult::Fail(msg) => {
                                    toast = Some((msg, now_hint() + TOAST_TIME));
                                }
                                LaunchResult::NoOp => {}
                            }
                        }
                        RowAction::LaunchRandom => {
                            let candidates: Vec<&PathBuf> = rows
                                .iter()
                                .filter_map(|row| match &row.action {
                                    RowAction::Launch(rom) => Some(rom),
                                    _ => None,
                                })
                                .collect();
                            if let Some(&rom) = candidates.get(rand_below(candidates.len())) {
                                let label = clean_name(
                                    rom.file_name().and_then(|n| n.to_str()).unwrap_or(""),
                                );
                                match launch_rom(&sd, &cfg, rom, &label, on_device) {
                                    LaunchResult::Started(code) => return code,
                                    LaunchResult::Fail(msg) => {
                                        toast = Some((msg, now_hint() + TOAST_TIME));
                                    }
                                    LaunchResult::NoOp => {}
                                }
                            }
                        }
                        RowAction::OpenCollection(path) => {
                            next_screen = Some(open_collection_detail(&sd, path));
                            boxart.clear();
                            infos.clear();
                            v_rep.clear();
                        }
                        RowAction::OpenSmartCollection(key) => {
                            next_screen = Some(open_smart_collection_detail(&platforms, key));
                            boxart.clear();
                            infos.clear();
                            v_rep.clear();
                        }
                        RowAction::PickPlatform(pi) => {
                            if let ListKind::PickPlatform(col) = kind {
                                let col = col.clone();
                                let p = &platforms[*pi];
                                let rows2: Vec<Row> = p
                                    .roms
                                    .iter()
                                    .map(|rom| Row {
                                        label: clean_name(rom),
                                        action: RowAction::PickGame(p.dir.join(rom)),
                                    })
                                    .collect();
                                next_screen = Some(Screen::List {
                                    kind: ListKind::PickGame(col),
                                    rows: rows2,
                                    selected: 0,
                                    scroll: 0,
                                    show_art: false,
                                    tag: None,
                                });
                                v_rep.clear();
                            }
                        }
                        RowAction::OpenPaks => {
                            let rows2: Vec<Row> = installed_paks(&sd)
                                .into_iter()
                                .map(|(name, script)| Row {
                                    label: name,
                                    action: RowAction::LaunchPak(script),
                                })
                                .collect();
                            next_screen = Some(Screen::List {
                                kind: ListKind::Paks,
                                rows: rows2,
                                selected: 0,
                                scroll: 0,
                                show_art: false,
                                tag: None,
                            });
                            v_rep.clear();
                        }
                        RowAction::LaunchPak(script) => {
                            if on_device {
                                run_hooks(
                                    "pre-launch.d",
                                    &[
                                        ("HOOK_PHASE", "pre".to_string()),
                                        ("HOOK_TYPE", "pak".to_string()),
                                        (
                                            "HOOK_EMU_PATH",
                                            script.display().to_string(),
                                        ),
                                    ],
                                );
                                let cmd = shell_quote(&script.display().to_string());
                                if std::fs::File::create("/tmp/next")
                                    .and_then(|mut f2| {
                                        use std::io::Write as _;
                                        f2.write_all(cmd.as_bytes())
                                    })
                                    .is_ok()
                                {
                                    return 0;
                                }
                            }
                        }
                        RowAction::NewCollection => {
                            next_screen = Some(Screen::Osk {
                                buf: String::new(),
                                pos: 0,
                                target: OskTarget::Collection,
                            });
                            v_rep.clear();
                        }
                        RowAction::PickGame(rom) => {
                            if let ListKind::PickGame(col) = kind {
                                sd.collection_add(col, rom);
                                let col = col.clone();
                                next_screen = Some(open_collection_detail(&sd, &col));
                                boxart.clear();
                                infos.clear();
                                v_rep.clear();
                            }
                        }
                    }
                } else if pin_btn && matches!(kind, ListKind::CollectionsIndex) {
                    // X renames the collection under the cursor; on the
                    // "+ New Collection" row it creates instead
                    match rows.get(*selected).map(|r2| &r2.action) {
                        Some(RowAction::OpenCollection(dir)) => {
                            let name = dir
                                .file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_default();
                            next_screen = Some(Screen::Osk {
                                buf: name,
                                pos: 0,
                                target: OskTarget::CollectionRename { dir: dir.clone() },
                            });
                        }
                        _ => {
                            next_screen = Some(Screen::Osk {
                                buf: String::new(),
                                pos: 0,
                                target: OskTarget::Collection,
                            });
                        }
                    }
                    v_rep.clear();
                } else if pin_btn
                    && let ListKind::Collection(col) = kind
                {
                    let col = col.clone();
                    next_screen = Some(open_pick_platform(&platforms, &col));
                    v_rep.clear();
                } else if wipe_btn
                    && matches!(kind, ListKind::CollectionsIndex)
                    && let Some(Row { action: RowAction::OpenCollection(path), .. }) =
                        rows.get(*selected)
                {
                    match wipe_armed {
                        Some((ai, at))
                            if ai == *selected
                                && now - at < std::time::Duration::from_secs(2) =>
                        {
                            sd.collection_delete(path);
                            rows.remove(*selected);
                            if *selected >= rows.len() && *selected > 0 {
                                *selected -= 1;
                            }
                            wipe_armed = None;
                        }
                        _ => wipe_armed = Some((*selected, now)),
                    }
                } else if wipe_btn
                    && matches!(kind, ListKind::CollectionsIndex)
                    && let Some(Row { action: RowAction::OpenSmartCollection(key), .. }) =
                        rows.get(*selected)
                {
                    // wiping a default collection dismisses it for good —
                    // persisted in userdata, so an update never revives it.
                    match wipe_armed {
                        Some((ai, at))
                            if ai == *selected
                                && now - at < std::time::Duration::from_secs(2) =>
                        {
                            sd.smart_dismiss(key);
                            rows.remove(*selected);
                            if *selected >= rows.len() && *selected > 0 {
                                *selected -= 1;
                            }
                            wipe_armed = None;
                        }
                        _ => wipe_armed = Some((*selected, now)),
                    }
                } else if wipe_btn
                    && let ListKind::Collection(col) = kind
                    && let Some(Row { action: RowAction::Launch(rom), .. }) = rows.get(*selected)
                {
                    match wipe_armed {
                        Some((ai, at))
                            if ai == *selected
                                && now - at < std::time::Duration::from_secs(2) =>
                        {
                            sd.collection_remove(col, rom);
                            rows.remove(*selected);
                            if *selected >= rows.len() && *selected > 0 {
                                *selected -= 1;
                            }
                            boxart.clear();
                            infos.clear();
                            wipe_armed = None;
                        }
                        _ => wipe_armed = Some((*selected, now)),
                    }
                } else if pin_btn
                    && matches!(kind, ListKind::Platform(_))
                    && let Some(Row { action: RowAction::Launch(rom), .. }) = rows.get(*selected)
                {
                    let rom = rom.clone();
                    let pinned = sd.pin_toggle(&rom);
                    // re-sort in place: pinned rows first, "> " prefix
                    for row in rows.iter_mut() {
                        if let RowAction::Launch(r) = &row.action {
                            let is_p = sd.is_pinned(r);
                            let bare = row.label.trim_start_matches("> ").to_string();
                            row.label = if is_p { format!("> {bare}") } else { bare };
                        }
                    }
                    rows.sort_by_key(|row| {
                        let random = matches!(row.action, RowAction::LaunchRandom);
                        let pinned = row.label.starts_with("> ");
                        (!random, !pinned, row.label.to_lowercase())
                    });
                    if let Some(i) = rows.iter().position(
                        |row| matches!(&row.action, RowAction::Launch(r) if *r == rom),
                    ) {
                        *selected = i;
                        *scroll = (*selected).saturating_sub(5);
                    }
                    boxart.clear();
                    infos.clear();
                    let _ = pinned;
                } else if wipe_btn
                    && matches!(kind, ListKind::Platform(_) | ListKind::Recents)
                    && let Some(Row { action: RowAction::Launch(rom), .. }) = rows.get(*selected)
                {
                    match wipe_armed {
                        Some((armed_idx, at))
                            if armed_idx == *selected
                                && now - at < std::time::Duration::from_secs(2) =>
                        {
                            let rom = rom.clone();
                            let tag = sd.tag_of_rom(&rom);
                            let is_port = tag.as_deref() == Some("PORTS");
                            // FAST, on the UI thread: drop the row + purge the
                            // small per-game config keys (trailing dot so
                            // "Mario" can't match "Mario Bros" keys) so the
                            // cursor never blocks.
                            if let (Some(t), Some(stem)) = (
                                &tag,
                                rom.file_stem().map(|s| s.to_string_lossy().into_owned()),
                            ) {
                                cfg.remove_prefix(&format!("game.{t}.{stem}."));
                                let _ = cfg.save();
                            }
                            rows.remove(*selected);
                            if *selected >= rows.len() && *selected > 0 {
                                *selected -= 1;
                            }
                            // Also drop it from the cached platform rom list —
                            // every platform rebuilds its list from
                            // platforms[pi].roms (loaded once at startup), so
                            // without this the wiped game reappears as a ghost
                            // on re-entering the platform (the files are already
                            // gone; only the stale cache still lists it).
                            if let ListKind::Platform(pi) = kind {
                                let pi = *pi;
                                let pdir = platforms[pi].dir.clone();
                                platforms[pi].roms.retain(|r| pdir.join(r) != rom);
                            }
                            boxart.clear();
                            infos.clear();
                            wipe_armed = None;
                            // SLOW, on a thread: the actual file deletion. A
                            // port wipe is a full uninstall (payload dir under
                            // Data/ports is hundreds of MB / thousands of files
                            // → seconds on FAT32); wipe_game clears rom, box
                            // art, saves, states. Off the UI thread so the list
                            // stays live; the Y hint reads "Wiping…" meanwhile.
                            let root = sd.root.clone();
                            let flying = wiping.clone();
                            flying.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            std::thread::spawn(move || {
                                let sd2 = Sd::new(root);
                                if is_port {
                                    let _ = kui_store::ports::uninstall_script(&sd2.root, &rom);
                                }
                                sd2.wipe_game(&rom);
                                flying.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                            });
                        }
                        _ => wipe_armed = Some((*selected, now)),
                    }
                } else if !v_rep.holding()
                    && *show_art
                    && ui_mode != UiMode::Lists
                    && let Some(Row { action: RowAction::Launch(rom), .. }) = rows.get(*selected)
                {
                    let sel = *selected;
                    boxart.entry(sel).or_insert_with(|| match sd.boxart_for(rom) {
                        Some(path) => {
                            loader.request(art::key(K_BOX, sel), path);
                            Art::Pending
                        }
                        None => Art::Missing,
                    });
                    infos
                        .entry(sel)
                        .or_insert_with(|| sd.game_info_for(rom).map(|i| i.footer()));
                } else if !v_rep.holding()
                    && *show_art
                    && ui_mode != UiMode::Lists
                    && matches!(kind, ListKind::CollectionsIndex)
                {
                    // the collections index shows each collection's own panel
                    let sel = *selected;
                    if let Some(key) = rows.get(sel).and_then(|r| collection_key(&r.action)) {
                        boxart.entry(sel).or_insert_with(|| match sd.collection_bg_key(&key) {
                            Some(path) => {
                                loader.request(art::key(K_BOX, sel), path);
                                Art::Pending
                            }
                            None => Art::Missing,
                        });
                    }
                }
            }
        }
        if let Some(s) = next_screen {
            screen = s;
        }

        // ---- uploads, max 2 per frame ----
        for _ in 0..2 {
            let Some((k, res)) = loader.try_recv() else { break };
            let (kind, i) = art::split(k);
            let map = match kind {
                K_BG => &mut bg,
                K_LOGO => &mut logo,
                K_BOX => &mut boxart,
                _ => &mut fbg,
            };
            if let Some(slot @ Art::Pending) = map.get_mut(&i) {
                *slot = match res
                    .and_then(|(w, h, px)| kui_gfx::texture_from_rgba(&v.gl, w, h, &px))
                {
                    Ok(t) => Art::Ready(t),
                    Err(e) => {
                        eprintln!("art load: {e}");
                        Art::Missing
                    }
                };
            }
        }

        // ---- render ----
        let (sw, sh) = v.drawable_size();
        let clear = [theme.c7[0], theme.c7[1], theme.c7[2]];
        r.begin_frame(&v.gl, sw, sh, clear);

        match &screen {
            Screen::ColorPick { rgb, channel, .. } => {
                if let Some(f) = font.as_mut() {
                    let labels = ["R", "G", "B"];
                    let top = sh as f32 * 0.24;
                    for (i, lab) in labels.iter().enumerate() {
                        let y = top + i as f32 * ROW_H;
                        let lh = f.line_height(LIST_FONT);
                        let pill_y = y + (ROW_H - PILL_H as f32) / 2.0;
                        let text_y = pill_y + (PILL_H as f32 - lh) / 2.0;
                        let sel = i == *channel;
                        let bar_x = 140.0;
                        let bar_w = sw as f32 * 0.5;
                        let sel_c = theme.c1;
                        let tc = if sel { sel_c } else { theme.c4 };
                        f.draw(&r, &v.gl, lab, 56.0, text_y, LIST_FONT, tc);
                        // channel bar
                        let frac = rgb[i] as f32 / 255.0;
                        r.rect(&v.gl, bar_x, y + ROW_H / 2.0 - 4.0, bar_w, 8.0, [0.25, 0.25, 0.25, 1.0]);
                        let ch_col = match i {
                            0 => [1.0, 0.2, 0.2, 1.0],
                            1 => [0.2, 1.0, 0.3, 1.0],
                            _ => [0.3, 0.4, 1.0, 1.0],
                        };
                        r.rect(&v.gl, bar_x, y + ROW_H / 2.0 - 4.0, bar_w * frac, 8.0, ch_col);
                        let val = format!("{:3}", rgb[i]);
                        f.draw(&r, &v.gl, &val, bar_x + bar_w + 24.0, text_y, LIST_FONT, tc);
                    }
                    // live swatch
                    let swatch = [
                        rgb[0] as f32 / 255.0,
                        rgb[1] as f32 / 255.0,
                        rgb[2] as f32 / 255.0,
                        1.0,
                    ];
                    r.rect(&v.gl, sw as f32 - 200.0, top - 20.0, 160.0, 160.0, swatch);
                    let hex = format!("{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2]);
                    let hw = f.measure(&v.gl, &hex, 22);
                    f.draw(&r, &v.gl, &hex, sw as f32 - 200.0 + (160.0 - hw) / 2.0, top + 150.0, 22, WHITE);
                }
            }
            Screen::CoreList { cores, selected, scroll } => {
                if let Some(f) = font.as_mut() {
                    // left-aligned, scraper-style
                    let top = 56.0;
                    let visible = 10usize;
                    for row in 0..visible.min(cores.len()) {
                        let idx = scroll + row;
                        if idx >= cores.len() {
                            break;
                        }
                        let y = top + row as f32 * ROW_H;
                        let lh = f.line_height(LIST_FONT);
                        let pill_y = y + (ROW_H - PILL_H as f32) / 2.0;
                        let text_y = pill_y + (PILL_H as f32 - lh) / 2.0;
                        if idx == *selected {
                            let sel_c =
                                theme.c1;
                            draw_roll(
                                f, &r, &v.gl, &mut roll_state, "corelist",
                                &cores[idx].label, 56.0, text_y, pill_y,
                                PILL_H as f32, LIST_FONT, sel_c,
                                sw as f32 - 128.0, now,
                            );
                        } else {
                            let name =
                                f.fit(&v.gl, &cores[idx].label, LIST_FONT, sw as f32 - 128.0);
                            f.draw(&r, &v.gl, &name, 56.0, text_y, LIST_FONT, theme.c4);
                        }
                    }
                }
            }
            Screen::CoreOpts { core, tags, defs, selected, scroll, .. } => {
                if let Some(f) = font.as_mut() {
                    let total = defs.len() + 4 + LSC.len();
                    let visible = 10usize;
                    let top = 56.0;
                    for row in 0..visible.min(total) {
                        let idx = scroll + row;
                        if idx >= total {
                            break;
                        }
                        let (label, val) = if idx == 0 {
                            let cur = tags
                                .first()
                                .map(|t| cfg.get_or(&format!("fe.{t}.scaling"), "").to_string())
                                .unwrap_or_default();
                            let disp = match cur.as_str() {
                                "native" => "Native",
                                "aspect" => "Aspect",
                                "fullscreen" => "Fullscreen",
                                _ => "Default",
                            };
                            ("Scaling".to_string(), disp.to_string())
                        } else if idx == 1 {
                            let cur = tags
                                .first()
                                .map(|t| cfg.get_or(&format!("fe.{t}.effect"), "").to_string())
                                .unwrap_or_default();
                            let disp = match cur.as_str() {
                                "grid" => "LCD Grid",
                                "line" => "Scanlines",
                                _ => "Off",
                            };
                            ("Effect".to_string(), disp.to_string())
                        } else if idx == 2 {
                            let cur = tags
                                .first()
                                .map(|t| cfg.get_or(&format!("fe.{t}.ff"), "").to_string())
                                .unwrap_or_default();
                            let disp = if cur.is_empty() {
                                "Default".to_string()
                            } else {
                                format!("{cur}x")
                            };
                            ("FF Speed".to_string(), disp)
                        } else if idx == 3 {
                            let cur = tags
                                .first()
                                .map(|t| cfg.get_or(&format!("fe.{t}.dpad"), "").to_string())
                                .unwrap_or_default();
                            let disp = match cur.as_str() {
                                "stick" => "Left Stick",
                                "dpad" => "Dpad",
                                _ => "Default (Dpad)",
                            };
                            ("Dpad Mode".to_string(), disp.to_string())
                        } else if (4..4 + LSC.len()).contains(&idx) {
                            let (key, label, dflt) = LSC[idx - 4];
                            let disp = if sc_cap == Some(idx - 4) && idx == *selected {
                                "Press a button...".to_string()
                            } else {
                                let cur = tags
                                    .first()
                                    .map(|t| {
                                        cfg.get_or(&format!("fe.{t}.shortcut.{key}"), "")
                                            .to_string()
                                    })
                                    .unwrap_or_default();
                                if cur.is_empty() {
                                    format!("Default ({})", lshortcut_disp(dflt))
                                } else {
                                    lshortcut_disp(&cur)
                                }
                            };
                            (label.to_string(), disp)
                        } else {
                            let def = &defs[idx - 4 - LSC.len()];
                            let ckey = format!("core.{core}.{}", def.key);
                            (
                                if def.desc.is_empty() {
                                    def.key.clone()
                                } else {
                                    def.desc.clone()
                                },
                                cfg.get(&ckey)
                                    .map(|s| s.to_string())
                                    .or_else(|| {
                                        kui_libretro::kui_option_default(core, &def.key)
                                            .map(str::to_string)
                                    })
                                    .or_else(|| def.choices.first().cloned())
                                    .unwrap_or_default(),
                            )
                        };
                        let y = top + row as f32 * ROW_H;
                        let lh = f.line_height(LIST_FONT);
                        let pill_y = y + (ROW_H - PILL_H as f32) / 2.0;
                        let text_y = pill_y + (PILL_H as f32 - lh) / 2.0;
                        let vw = f.measure(&v.gl, &val, LIST_FONT);
                        let vx = sw as f32 - 48.0 - vw;
                        if idx == *selected {
                            let sel_c =
                                theme.c1;
                            draw_roll(
                                f, &r, &v.gl, &mut roll_state, "coreopts", &label, 56.0,
                                text_y, pill_y, PILL_H as f32, LIST_FONT, sel_c,
                                sw as f32 * 0.55, now,
                            );
                            f.draw(&r, &v.gl, &val, vx, text_y, LIST_FONT, sel_c);
                        } else {
                            let shown = f.fit(&v.gl, &label, LIST_FONT, sw as f32 * 0.55);
                            f.draw(&r, &v.gl, &shown, 56.0, text_y, LIST_FONT, theme.c4);
                            f.draw(&r, &v.gl, &val, vx, text_y, LIST_FONT, theme.c2);
                        }
                    }
                }
            }
            Screen::PakCats { cats, selected, scroll } => {
                if let Some(f) = font.as_mut() {
                    let head = store_msg.lock().map(|m| m.clone()).unwrap_or_default();
                    let head = if !head.is_empty() {
                        head
                    } else if cats.is_empty() {
                        "Fetching pak list...".to_string()
                    } else {
                        "Browse community paks".to_string()
                    };
                    f.draw(&r, &v.gl, &head, 32.0, 20.0, 22, theme.c6);
                    let top = 56.0;
                    let visible = 10usize;
                    for row in 0..visible.min(cats.len()) {
                        let idx = scroll + row;
                        if idx >= cats.len() {
                            break;
                        }
                        let (name, count, updates) = &cats[idx];
                        let value = if *updates > 0 {
                            format!("{updates} UPDATE")
                        } else {
                            format!("{count} paks")
                        };
                        let y = top + row as f32 * ROW_H;
                        let lh = f.line_height(LIST_FONT);
                        let pill_y = y + (ROW_H - PILL_H as f32) / 2.0;
                        let text_y = pill_y + (PILL_H as f32 - lh) / 2.0;
                        let vw = f.measure(&v.gl, &value, LIST_FONT);
                        let vx = sw as f32 - 48.0 - vw;
                        if idx == *selected {
                            let sel_c =
                                theme.c1;
                            f.draw(&r, &v.gl, name, 56.0, text_y, LIST_FONT, sel_c);
                            f.draw(&r, &v.gl, &value, vx, text_y, LIST_FONT, sel_c);
                        } else {
                            f.draw(&r, &v.gl, name, 56.0, text_y, LIST_FONT, theme.c4);
                            let vc = if *updates > 0 { theme.c2 } else { theme.c6 };
                            f.draw(&r, &v.gl, &value, vx, text_y, LIST_FONT, vc);
                        }
                    }
                }
            }
            Screen::PakDek { paks, title, selected, scroll, .. } => {
                if let Some(f) = font.as_mut() {
                    let head = store_msg.lock().map(|m| m.clone()).unwrap_or_default();
                    let head = if !head.is_empty() { head } else { title.clone() };
                    f.draw(&r, &v.gl, &head, 32.0, 20.0, 22, theme.c6);
                    let top = 56.0;
                    let row_h2 = 82.0;
                    let visible = pakdek_visible_rows();
                    for row in 0..visible.min(paks.len()) {
                        let idx = scroll + row;
                        if idx >= paks.len() {
                            break;
                        }
                        let p = &paks[idx];
                        let inst = kui_store::installed_version(p);
                        let value = match &inst {
                            Some(v3) if *v3 != p.version => "UPDATE".to_string(),
                            Some(_) => "Installed".to_string(),
                            None => p.version.clone(),
                        };
                        let y = top + row as f32 * row_h2;
                        let lh = f.line_height(LIST_FONT);
                        let pill_y = y;
                        let text_y = y + (PILL_H as f32 - lh) / 2.0;
                        let vw = f.measure(&v.gl, &value, LIST_FONT);
                        let vx = sw as f32 - 48.0 - vw;
                        let sub = format!("{} — by {}", p.description, p.author);
                        if idx == *selected {
                            let sel_c =
                                theme.c1;
                            draw_roll(
                                f, &r, &v.gl, &mut roll_state, "pakdek", &p.name, 56.0,
                                text_y, pill_y, PILL_H as f32, LIST_FONT, sel_c,
                                vx - 56.0 - 24.0, now,
                            );
                            f.draw(&r, &v.gl, &value, vx, text_y, LIST_FONT, sel_c);
                            draw_roll(
                                f, &r, &v.gl, &mut roll_state, "pakdesc", &sub, 56.0,
                                y + PILL_H as f32 + 2.0, y + PILL_H as f32, 24.0, 18,
                                theme.c6, sw as f32 - 112.0, now,
                            );
                        } else {
                            let shown =
                                f.fit(&v.gl, &p.name, LIST_FONT, vx - 56.0 - 24.0);
                            f.draw(&r, &v.gl, &shown, 56.0, text_y, LIST_FONT, theme.c4);
                            let vc = if value == "UPDATE" { theme.c2 } else { theme.c6 };
                            f.draw(&r, &v.gl, &value, vx, text_y, LIST_FONT, vc);
                            let s2 = f.fit(&v.gl, &sub, 18, sw as f32 - 112.0);
                            f.draw(
                                &r, &v.gl, &s2, 56.0,
                                y + PILL_H as f32 + 2.0, 18, theme.c6,
                            );
                        }
                    }
                }
            }
            Screen::PortCats { cats, selected, scroll, rtr } => {
                if let Some(f) = font.as_mut() {
                    let head = store_msg.lock().map(|m| m.clone()).unwrap_or_default();
                    let head = if !head.is_empty() {
                        head
                    } else if cats.is_empty() {
                        "Fetching ports...".to_string()
                    } else if *rtr {
                        "Ready to Play".to_string()
                    } else {
                        format!("Ports — {} available", ports_all.ports.len())
                    };
                    f.draw(&r, &v.gl, &head, 32.0, 20.0, 22, theme.c6);
                    let top = 56.0;
                    let visible = 10usize;
                    for row in 0..visible.min(cats.len()) {
                        let idx = scroll + row;
                        if idx >= cats.len() {
                            break;
                        }
                        let (name, count) = &cats[idx];
                        let value = format!("{count} ports");
                        let y = top + row as f32 * ROW_H;
                        let lh = f.line_height(LIST_FONT);
                        let pill_y = y + (ROW_H - PILL_H as f32) / 2.0;
                        let text_y = pill_y + (PILL_H as f32 - lh) / 2.0;
                        let vw = f.measure(&v.gl, &value, LIST_FONT);
                        let vx = sw as f32 - 48.0 - vw;
                        if idx == *selected {
                            let sel_c = theme.c1;
                            f.draw(&r, &v.gl, name, 56.0, text_y, LIST_FONT, sel_c);
                            f.draw(&r, &v.gl, &value, vx, text_y, LIST_FONT, sel_c);
                        } else {
                            f.draw(&r, &v.gl, name, 56.0, text_y, LIST_FONT, theme.c4);
                            f.draw(&r, &v.gl, &value, vx, text_y, LIST_FONT, theme.c6);
                        }
                    }
                }
            }
            Screen::Ports { ports, title, selected, scroll, .. } => {
                if let Some(f) = font.as_mut() {
                    let head = store_msg.lock().map(|m| m.clone()).unwrap_or_default();
                    let head = if !head.is_empty() { head } else { title.clone() };
                    f.draw(&r, &v.gl, &head, 32.0, 20.0, 22, theme.c6);
                    let top = 56.0;
                    let row_h2 = 82.0;
                    let visible = pakdek_visible_rows();
                    for row in 0..visible.min(ports.len()) {
                        let idx = scroll + row;
                        if idx >= ports.len() {
                            break;
                        }
                        let p = &ports[idx];
                        let job = port_jobs
                            .lock()
                            .ok()
                            .and_then(|m| m.get(&p.zip_name).cloned());
                        let active = job.is_some();
                        let inst = kui_store::ports::installed(&sd.root, p);
                        let value = if let Some(st) = job {
                            st
                        } else if inst {
                            "Installed".to_string()
                        } else if title == "Installed" {
                            // was installed when this list was built; a queued
                            // removal finished. Keep the row so the list never
                            // reflows under the cursor — it only leaves on the
                            // next entry to the Installed category.
                            "Removed".to_string()
                        } else if p.size >= 1024 * 1024 {
                            format!("{} MB", p.size / (1024 * 1024))
                        } else {
                            format!("{} KB", (p.size / 1024).max(1))
                        };
                        let y = top + row as f32 * row_h2;
                        let lh = f.line_height(LIST_FONT);
                        let pill_y = y;
                        let text_y = y + (PILL_H as f32 - lh) / 2.0;
                        let vw = f.measure(&v.gl, &value, LIST_FONT);
                        let vx = sw as f32 - 48.0 - vw;
                        let mut sub = p.desc.clone();
                        if !p.rtr {
                            sub = format!("NEEDS GAME FILES — {sub}");
                        }
                        if idx == *selected {
                            let sel_c = theme.c1;
                            draw_roll(
                                f, &r, &v.gl, &mut roll_state, "ports", &p.title, 56.0,
                                text_y, pill_y, PILL_H as f32, LIST_FONT, sel_c,
                                vx - 56.0 - 24.0, now,
                            );
                            f.draw(&r, &v.gl, &value, vx, text_y, LIST_FONT, sel_c);
                            draw_roll(
                                f, &r, &v.gl, &mut roll_state, "portdesc", &sub, 56.0,
                                y + PILL_H as f32 + 2.0, y + PILL_H as f32, 24.0, 18,
                                theme.c6, sw as f32 - 112.0, now,
                            );
                        } else {
                            let shown =
                                f.fit(&v.gl, &p.title, LIST_FONT, vx - 56.0 - 24.0);
                            f.draw(&r, &v.gl, &shown, 56.0, text_y, LIST_FONT, theme.c4);
                            let vc = if inst || active { theme.c2 } else { theme.c6 };
                            f.draw(&r, &v.gl, &value, vx, text_y, LIST_FONT, vc);
                            let s2 = f.fit(&v.gl, &sub, 18, sw as f32 - 112.0);
                            f.draw(
                                &r, &v.gl, &s2, 56.0,
                                y + PILL_H as f32 + 2.0, 18, theme.c6,
                            );
                        }
                    }
                }
            }
            Screen::Updater { releases, selected } => {
                if let Some(f) = font.as_mut() {
                    let head = store_msg
                        .lock()
                        .map(|m| m.clone())
                        .unwrap_or_default();
                    let head = if !head.is_empty() {
                        head
                    } else if releases.is_empty() {
                        "Checking for updates...".to_string()
                    } else {
                        format!("Installed: {}", hub::KUI_VERSION)
                    };
                    f.draw(&r, &v.gl, &head, 32.0, 20.0, 22, theme.c6);
                    let top = 56.0;
                    for (i, rel) in releases.iter().take(10).enumerate() {
                        let label = if rel.tag.contains(hub::KUI_VERSION) {
                            format!("{}  (installed)", rel.tag)
                        } else {
                            rel.tag.clone()
                        };
                        let y = top + i as f32 * ROW_H;
                        let lh = f.line_height(LIST_FONT);
                        let pill_y = y + (ROW_H - PILL_H as f32) / 2.0;
                        let text_y = pill_y + (PILL_H as f32 - lh) / 2.0;
                        if i == *selected {
                            let sel_c =
                                theme.c1;
                            f.draw(&r, &v.gl, &label, 56.0, text_y, LIST_FONT, sel_c);
                        } else {
                            f.draw(&r, &v.gl, &label, 56.0, text_y, LIST_FONT, theme.c4);
                        }
                    }
                }
            }
            Screen::ScraperPlatforms { rows, selected, scroll } => {
                if let Some(f) = font.as_mut() {
                    let top = 56.0;
                    let visible = 10usize;
                    for row in 0..visible.min(rows.len()) {
                        let idx = scroll + row;
                        if idx >= rows.len() {
                            break;
                        }
                        let (label, _) = &rows[idx];
                        let y = top + row as f32 * ROW_H;
                        let lh = f.line_height(LIST_FONT);
                        let pill_y = y + (ROW_H - PILL_H as f32) / 2.0;
                        let text_y = pill_y + (PILL_H as f32 - lh) / 2.0;
                        if idx == *selected {
                            let sel_c =
                                theme.c1;
                            draw_roll(
                                f, &r, &v.gl, &mut roll_state, "scraper", label, 56.0,
                                text_y, pill_y, PILL_H as f32, LIST_FONT, sel_c,
                                sw as f32 - 128.0, now,
                            );
                        } else {
                            let shown = f.fit(&v.gl, label, LIST_FONT, sw as f32 - 128.0);
                            f.draw(&r, &v.gl, &shown, 56.0, text_y, LIST_FONT, theme.c4);
                        }
                    }
                }
            }
            Screen::CheatPlatforms { rows, selected, scroll, .. }
            | Screen::PrefetchPlatforms { rows, selected, scroll, .. } => {
                if let Some(f) = font.as_mut() {
                    let head = if matches!(screen, Screen::CheatPlatforms { .. }) {
                        "Download cheat files for..."
                    } else {
                        "Cache achievement data for..."
                    };
                    f.draw(&r, &v.gl, head, 32.0, 20.0, 22, theme.c6);
                    let top = 56.0;
                    let visible = 10usize;
                    for row in 0..visible.min(rows.len()) {
                        let idx = scroll + row;
                        if idx >= rows.len() {
                            break;
                        }
                        let (label, _) = &rows[idx];
                        let y = top + row as f32 * ROW_H;
                        let lh = f.line_height(LIST_FONT);
                        let pill_y = y + (ROW_H - PILL_H as f32) / 2.0;
                        let text_y = pill_y + (PILL_H as f32 - lh) / 2.0;
                        if idx == *selected {
                            let sel_c = theme.c1;
                            draw_roll(
                                f, &r, &v.gl, &mut roll_state, "platpick", label, 56.0,
                                text_y, pill_y, PILL_H as f32, LIST_FONT, sel_c,
                                sw as f32 - 128.0, now,
                            );
                        } else {
                            let shown = f.fit(&v.gl, label, LIST_FONT, sw as f32 - 128.0);
                            f.draw(&r, &v.gl, &shown, 56.0, text_y, LIST_FONT, theme.c4);
                        }
                    }
                }
            }
            Screen::ScraperMenu { label, selected, .. } => {
                if let Some(f) = font.as_mut() {
                    f.draw(&r, &v.gl, label, 32.0, 20.0, 22, theme.c6);
                    let top = 56.0;
                    for (i, act) in SCRAPER_ACTIONS.iter().enumerate() {
                        let y = top + i as f32 * ROW_H;
                        let lh = f.line_height(LIST_FONT);
                        let pill_y = y + (ROW_H - PILL_H as f32) / 2.0;
                        let text_y = pill_y + (PILL_H as f32 - lh) / 2.0;
                        if i == *selected {
                            let sel_c =
                                theme.c1;
                            f.draw(&r, &v.gl, act, 56.0, text_y, LIST_FONT, sel_c);
                        } else {
                            f.draw(&r, &v.gl, act, 56.0, text_y, LIST_FONT, theme.c4);
                        }
                    }
                }
            }
            Screen::ScraperRun { job, label, .. } => {
                if let Some(f) = font.as_mut() {
                    let p = job.progress();
                    f.draw(&r, &v.gl, label, 32.0, 20.0, 22, theme.c6);
                    let (phase, col) = if let Some(e) = &p.error {
                        (format!("Error: {e}"), theme.c2)
                    } else if p.finished {
                        // completed: the last progress string becomes the
                        // result summary, prefixed so it never reads as a
                        // frozen counter
                        (format!("Done — {}", p.phase), theme.c1)
                    } else {
                        (p.phase.clone(), theme.c4)
                    };
                    let pw2 = f.measure(&v.gl, &phase, 26);
                    f.draw(
                        &r,
                        &v.gl,
                        &phase,
                        (sw as f32 - pw2) / 2.0,
                        sh as f32 * 0.4,
                        26,
                        col,
                    );
                    if p.finished && p.error.is_none() {
                        let hint = "Press B to return";
                        let hw = f.measure(&v.gl, hint, 18);
                        f.draw(&r, &v.gl, hint, (sw as f32 - hw) / 2.0, sh as f32 * 0.4 + 40.0, 18, theme.c6);
                    }
                    if p.total > 0 && !p.finished {
                        let bw = sw as f32 * 0.6;
                        let bx = (sw as f32 - bw) / 2.0;
                        let by = sh as f32 * 0.5;
                        r.rect(&v.gl, bx, by, bw, 10.0, [0.25, 0.25, 0.25, 1.0]);
                        r.rect(
                            &v.gl,
                            bx,
                            by,
                            bw * (p.done as f32 / p.total as f32),
                            10.0,
                            theme.c1,
                        );
                    }
                }
            }
            Screen::Battery { samples, span_h } => {
                if let Some(f) = font.as_mut() {
                    let now_e = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    let span_s = span_h * 3600;
                    let t0 = now_e.saturating_sub(span_s);
                    let (gx, gy, gw2, gh2) =
                        (72.0, 84.0, sw as f32 - 72.0 - 48.0, sh as f32 - 84.0 - 150.0);
                    // frame + quarter grid
                    for q in 0..=4 {
                        let y = gy + gh2 * (q as f32 / 4.0);
                        r.rect(&v.gl, gx, y, gw2, 1.0, [0.25, 0.25, 0.25, 1.0]);
                        let lbl = format!("{}%", 100 - q * 25);
                        let lw2 = f.measure(&v.gl, &lbl, 16);
                        f.draw(&r, &v.gl, &lbl, gx - lw2 - 8.0, y - 8.0, 16, theme.c6);
                    }
                    let win: Vec<&(u64, i32, bool)> =
                        samples.iter().filter(|(t, _, _)| *t >= t0).collect();
                    for (t, pct, chg) in &win {
                        let x = gx + ((t - t0) as f32 / span_s as f32) * gw2;
                        let y = gy + gh2 * (1.0 - (*pct as f32 / 100.0));
                        let col = if *chg { [0.2, 0.9, 0.4, 1.0] } else { theme.c1 };
                        r.rect(&v.gl, x, y - 2.0, 3.0, 4.0, col);
                    }
                    // header: current state + discharge rate + projection
                    let cur = status.batt.unwrap_or(0);
                    let mut head = format!("Battery {cur}%");
                    if status.charging {
                        head.push_str(" · charging");
                    } else {
                        // rate from the last discharging stretch in the window
                        let disc: Vec<&&(u64, i32, bool)> =
                            win.iter().filter(|(_, _, c)| !c).collect();
                        if let (Some(first), Some(last)) = (disc.first(), disc.last())
                            && last.0 > first.0
                            && first.1 > last.1
                        {
                            let hrs = (last.0 - first.0) as f32 / 3600.0;
                            let rate = (first.1 - last.1) as f32 / hrs;
                            if rate > 0.1 {
                                head.push_str(&format!(
                                    " · -{rate:.1}%/h · ≈{:.1}h left",
                                    cur as f32 / rate
                                ));
                            }
                        }
                    }
                    f.draw(&r, &v.gl, &head, 32.0, 20.0, 22, theme.c6);
                    let span_lbl = format!("last {span_h}h");
                    let slw = f.measure(&v.gl, &span_lbl, 22);
                    f.draw(&r, &v.gl, &span_lbl, sw as f32 - 48.0 - slw, 20.0, 22, theme.c6);
                }
            }
            Screen::Files { panes, active, menu, armed_delete } => {
                if let Some(f) = font.as_mut() {
                    let pane_w = (sw as f32 - 64.0 - 24.0) / 2.0;
                    for (pi, pane) in panes.iter().enumerate() {
                        let px = 32.0 + pi as f32 * (pane_w + 24.0);
                        let rel = pane.dir.strip_prefix(&sd.root).unwrap_or(&pane.dir);
                        let head = if rel.as_os_str().is_empty() {
                            "SD Card".to_string()
                        } else {
                            format!("SD Card/{}", rel.display())
                        };
                        let hc = if pi == *active { theme.c6 } else { theme.c4 };
                        let shown = f.fit(&v.gl, &head, 20, pane_w);
                        f.draw(&r, &v.gl, &shown, px, 22.0, 20, hc);
                        let top = 56.0;
                        let visible = 10usize;
                        if pane.rows.is_empty() {
                            f.draw(
                                &r, &v.gl, "Empty folder", px + 20.0, top + 18.0,
                                LIST_FONT, theme.c4,
                            );
                        }
                        for row in 0..visible.min(pane.rows.len()) {
                            let idx = pane.scroll + row;
                            if idx >= pane.rows.len() {
                                break;
                            }
                            let fr = &pane.rows[idx];
                            let name = if fr.is_dir {
                                format!("{}/", fr.name)
                            } else {
                                fr.name.clone()
                            };
                            let y = top + row as f32 * ROW_H;
                            let lh = f.line_height(LIST_FONT);
                            let ilh = f.line_height(22);
                            let pill_y = y + (ROW_H - PILL_H as f32) / 2.0;
                            let text_y = pill_y + (PILL_H as f32 - lh) / 2.0;
                            let info_y = pill_y + (PILL_H as f32 - ilh) / 2.0;
                            let vw = f.measure(&v.gl, &fr.info, 22);
                            let vx = px + pane_w - 16.0 - vw;
                            let name_w = vx - (px + 20.0) - 16.0;
                            let is_sel = idx == pane.selected;
                            if is_sel && pi == *active {
                                let sel_c =
                                    theme.c1;
                                draw_roll(
                                    f, &r, &v.gl, &mut roll_state,
                                    if pi == 0 { "files0" } else { "files1" }, &name,
                                    px + 20.0, text_y, pill_y, PILL_H as f32,
                                    LIST_FONT, sel_c, name_w, now,
                                );
                                f.draw(&r, &v.gl, &fr.info, vx, info_y, 22, sel_c);
                            } else {
                                let nc = if is_sel {
                                    // inactive pane's cursor, pill-less: dimmed accent
                                    [theme.c1[0], theme.c1[1], theme.c1[2], 0.55]
                                } else if fr.is_dir {
                                    // folders track the active pane, like the
                                    // SD Card header: light here, grey away
                                    if pi == *active { theme.c6 } else { theme.c4 }
                                } else {
                                    theme.c4
                                };
                                let shown =
                                    f.fit(&v.gl, &name, LIST_FONT, name_w);
                                f.draw(
                                    &r, &v.gl, &shown, px + 20.0, text_y, LIST_FONT,
                                    nc,
                                );
                                f.draw(&r, &v.gl, &fr.info, vx, info_y, 22, theme.c2);
                            }
                        }
                    }
                    if let Some(mi) = menu {
                        // heavy scrim: panes drop to ~10% so the menu owns
                        // the screen (Arjun: 0.75 read too bright)
                        r.rect(&v.gl, 0.0, 0.0, sw as f32, sh as f32, [0.0, 0.0, 0.0, 0.9]);
                        let acts = file_actions(&file_clip);
                        let total_h = acts.len() as f32 * ROW_H;
                        let mtop = (sh as f32 - total_h) / 2.0;
                        let cx = sw as f32 / 2.0;
                        let pane = &panes[*active];
                        if let Some(fr) = pane.rows.get(pane.selected) {
                            let hfw = f.measure(&v.gl, &fr.name, 22);
                            f.draw(
                                &r, &v.gl, &fr.name, cx - hfw / 2.0,
                                mtop - 22.0 - f.line_height(22), 22, theme.c6,
                            );
                        }
                        for (i, act) in acts.iter().enumerate() {
                            let label = if *act == "Delete" && *armed_delete {
                                "Sure?"
                            } else {
                                act
                            };
                            let y = mtop + i as f32 * ROW_H;
                            let lh = f.line_height(LIST_FONT);
                            let pill_y = y + (ROW_H - PILL_H as f32) / 2.0;
                            let text_y = pill_y + (PILL_H as f32 - lh) / 2.0;
                            let tw = f.measure(&v.gl, label, LIST_FONT);
                            if i == *mi {
                                let sel_c = theme.c1;
                                f.draw(
                                    &r, &v.gl, label, cx - tw / 2.0, text_y, LIST_FONT,
                                    sel_c,
                                );
                            } else {
                                f.draw(
                                    &r, &v.gl, label, cx - tw / 2.0, text_y, LIST_FONT,
                                    theme.c4,
                                );
                            }
                        }
                    }
                }
            }
            Screen::PortForge { pane } => {
                if let Some(f) = font.as_mut() {
                    let rel = pane.dir.strip_prefix(&sd.root).unwrap_or(&pane.dir);
                    let head = if rel.as_os_str().is_empty() {
                        "Browse to your extracted RPG Maker game folder".to_string()
                    } else {
                        format!("SD Card/{}", rel.display())
                    };
                    let shown = f.fit(&v.gl, &head, 22, sw as f32 - 64.0);
                    f.draw(&r, &v.gl, &shown, 32.0, 20.0, 22, theme.c6);
                    let top = 56.0;
                    let visible = 10usize;
                    if pane.rows.is_empty() {
                        f.draw(&r, &v.gl, "No folders here", 52.0, top + 18.0, LIST_FONT, theme.c4);
                    }
                    for row in 0..visible.min(pane.rows.len()) {
                        let idx = pane.scroll + row;
                        if idx >= pane.rows.len() {
                            break;
                        }
                        let fr = &pane.rows[idx];
                        let is_game = fr.is_game;
                        let is_pkg = fr.is_package;
                        let name = if fr.is_dir { format!("{}/", fr.name) } else { fr.name.clone() };
                        let value = if is_pkg {
                            "PORT".to_string()
                        } else if is_game {
                            "GAME".to_string()
                        } else {
                            fr.info.clone()
                        };
                        let y = top + row as f32 * ROW_H;
                        let lh = f.line_height(LIST_FONT);
                        let ilh = f.line_height(22);
                        let pill_y = y + (ROW_H - PILL_H as f32) / 2.0;
                        let text_y = pill_y + (PILL_H as f32 - lh) / 2.0;
                        let info_y = pill_y + (PILL_H as f32 - ilh) / 2.0;
                        let vw = f.measure(&v.gl, &value, 22);
                        let vx = sw as f32 - 48.0 - vw;
                        let name_w = vx - 52.0 - 16.0;
                        if idx == pane.selected {
                            let sel_c = theme.c1;
                            draw_roll(
                                f, &r, &v.gl, &mut roll_state, "pforge", &name, 52.0, text_y,
                                pill_y, PILL_H as f32, LIST_FONT, sel_c, name_w, now,
                            );
                            f.draw(&r, &v.gl, &value, vx, info_y, 22, sel_c);
                        } else {
                            let nc = if is_game || is_pkg {
                                theme.c2
                            } else if fr.is_dir {
                                theme.c6
                            } else {
                                theme.c4
                            };
                            let shown = f.fit(&v.gl, &name, LIST_FONT, name_w);
                            f.draw(&r, &v.gl, &shown, 52.0, text_y, LIST_FONT, nc);
                            let vc = if is_game || is_pkg { theme.c2 } else { theme.c4 };
                            f.draw(&r, &v.gl, &value, vx, info_y, 22, vc);
                        }
                    }
                }
            }
            Screen::PortForgeRun { source } => {
                if let Some(f) = font.as_mut() {
                    let msg = store_msg.lock().map(|m| m.clone()).unwrap_or_default();
                    let busy =
                        !msg.is_empty() && !msg.starts_with("Done") && !msg.contains("Failed");
                    let can_delete = forge_del.lock().map(|d| d.is_some()).unwrap_or(false);
                    let title = "Port Forge";
                    let tw = f.measure(&v.gl, title, 28);
                    f.draw(&r, &v.gl, title, (sw as f32 - tw) / 2.0, sh as f32 * 0.30, 28, theme.c1);
                    let sline = f.fit(&v.gl, &msg, 22, sw as f32 - 80.0);
                    let slw = f.measure(&v.gl, &sline, 22);
                    let sc = if msg.contains("Failed") { theme.c4 } else { theme.c2 };
                    f.draw(&r, &v.gl, &sline, (sw as f32 - slw) / 2.0, sh as f32 * 0.45, 22, sc);
                    if busy {
                        // progress bar (mirrors the scraper): dim track + fill
                        let bw = sw as f32 * 0.6;
                        let bx = (sw as f32 - bw) / 2.0;
                        let by = sh as f32 * 0.55;
                        r.rect(&v.gl, bx, by, bw, 10.0, [0.25, 0.25, 0.25, 1.0]);
                        if msg.starts_with("Deleting") {
                            // delete has no byte count — an indeterminate,
                            // ping-ponging segment shows it's still working
                            let seg = bw * 0.25;
                            let ph = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .map(|d| d.as_secs_f32())
                                .unwrap_or(0.0)
                                % 2.0;
                            let pp = (ph - 1.0).abs(); // 0→1→0 over 2s
                            r.rect(&v.gl, bx + (bw - seg) * pp, by, seg, 10.0, theme.c1);
                        } else {
                            let frac = forge_pct.lock().map(|p| *p).unwrap_or(0.0).clamp(0.0, 1.0);
                            r.rect(&v.gl, bx, by, bw * frac, 10.0, theme.c1);
                        }
                    } else if can_delete {
                        let fname = source
                            .file_name()
                            .map(|s| s.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        let q = format!("Delete the original files (\u{201C}{fname}\u{201D}) to free space?");
                        let q = f.fit(&v.gl, &q, 20, sw as f32 - 80.0);
                        let qw = f.measure(&v.gl, &q, 20);
                        f.draw(&r, &v.gl, &q, (sw as f32 - qw) / 2.0, sh as f32 * 0.55, 20, theme.c4);
                    }
                }
            }
            Screen::InputTest => {
                if let Some(f) = font.as_mut() {
                    f.draw(
                        &r, &v.gl, "Press anything - every control lights up", 32.0,
                        20.0, 22, theme.c6,
                    );
                    let cols = 4usize;
                    let grid_top = 76.0;
                    let cell_w = (sw as f32 - 64.0) / cols as f32;
                    let cell_h = 96.0;
                    let pw2 = cell_w - 24.0;
                    for (i, lab) in INPUT_LABELS.iter().enumerate() {
                        let col = i % cols;
                        let row = i / cols;
                        let x = 32.0 + col as f32 * cell_w;
                        let y = grid_top + row as f32 * cell_h;
                        let pill_y = y + (cell_h - PILL_H as f32) / 2.0;
                        let lh = f.line_height(LIST_FONT);
                        let text_y = pill_y + (PILL_H as f32 - lh) / 2.0;
                        let on = input_pressed[i];
                        let pc = if on { theme.c1 } else { [1.0, 1.0, 1.0, 0.08] };
                        pill.draw(&r, &v.gl, x + (cell_w - pw2) / 2.0, pill_y, pw2, pc);
                        let tw = f.measure(&v.gl, lab, LIST_FONT);
                        let tc = if on { theme.c7 } else { theme.c4 };
                        f.draw(&r, &v.gl, lab, x + (cell_w - tw) / 2.0, text_y, LIST_FONT, tc);
                    }
                    let sy = grid_top + 5.0 * cell_h + 16.0;
                    let state = |o: Option<bool>| match o {
                        Some(true) => "On",
                        Some(false) => "Off",
                        None => "-",
                    };
                    let line = format!(
                        "FN slider: {}    Headphones: {}",
                        state(input_fn),
                        match input_jack {
                            Some(true) => "In",
                            Some(false) => "Out",
                            None => "-",
                        }
                    );
                    let tw = f.measure(&v.gl, &line, LIST_FONT);
                    f.draw(
                        &r, &v.gl, &line, (sw as f32 - tw) / 2.0, sy, LIST_FONT,
                        theme.c2,
                    );
                }
            }
            Screen::GameTime { rows, header, selected, scroll } => {
                if let Some(f) = font.as_mut() {
                    f.draw(&r, &v.gl, header, 32.0, 20.0, 22, theme.c6);
                    let top = 56.0;
                    let visible = 10usize;
                    for row in 0..visible.min(rows.len()) {
                        let idx = scroll + row;
                        if idx >= rows.len() {
                            break;
                        }
                        let (name, value) = &rows[idx];
                        let y = top + row as f32 * ROW_H;
                        let lh = f.line_height(LIST_FONT);
                        let pill_y = y + (ROW_H - PILL_H as f32) / 2.0;
                        let text_y = pill_y + (PILL_H as f32 - lh) / 2.0;
                        let vw = f.measure(&v.gl, value, LIST_FONT);
                        let vx = sw as f32 - 48.0 - vw;
                        if idx == *selected {
                            let sel_c =
                                theme.c1;
                            draw_roll(
                                f, &r, &v.gl, &mut roll_state, "gametime", name, 56.0,
                                text_y, pill_y, PILL_H as f32, LIST_FONT, sel_c,
                                vx - 56.0 - 24.0, now,
                            );
                            f.draw(&r, &v.gl, value, vx, text_y, LIST_FONT, sel_c);
                        } else {
                            let shown = f.fit(&v.gl, name, LIST_FONT, vx - 56.0 - 24.0);
                            f.draw(&r, &v.gl, &shown, 56.0, text_y, LIST_FONT, theme.c4);
                            f.draw(&r, &v.gl, value, vx, text_y, LIST_FONT, theme.c2);
                        }
                    }
                }
            }
            Screen::Bt { devs, selected, scroll } => {
                if let Some(f) = font.as_mut() {
                    let on = proc_running("bluetoothd");
                    let head = if !on {
                        "Bluetooth is off".to_string()
                    } else if let Some(d) = devs.iter().find(|d| d.connected) {
                        format!("Connected: {}", d.name)
                    } else if bt_scan_at.is_some() {
                        "Scanning...".to_string()
                    } else {
                        "Not connected".to_string()
                    };
                    f.draw(&r, &v.gl, &head, 32.0, 20.0, 22, theme.c6);
                    let top = 56.0;
                    let visible = 10usize;
                    for row in 0..visible.min(1 + devs.len()) {
                        let idx = scroll + row;
                        if idx > devs.len() {
                            break;
                        }
                        let (label, value) = if idx == 0 {
                            (
                                "Bluetooth".to_string(),
                                if on { "On".to_string() } else { "Off".to_string() },
                            )
                        } else {
                            let dev = &devs[idx - 1];
                            let val = if dev.connected {
                                "Connected".to_string()
                            } else if dev.paired {
                                "paired".to_string()
                            } else {
                                String::new()
                            };
                            (dev.name.clone(), val)
                        };
                        let y = top + row as f32 * ROW_H;
                        let lh = f.line_height(LIST_FONT);
                        let pill_y = y + (ROW_H - PILL_H as f32) / 2.0;
                        let text_y = pill_y + (PILL_H as f32 - lh) / 2.0;
                        let vw = f.measure(&v.gl, &value, LIST_FONT);
                        let vx = sw as f32 - 48.0 - vw;
                        if idx == *selected {
                            let sel_c =
                                theme.c1;
                            draw_roll(
                                f, &r, &v.gl, &mut roll_state, "bt", &label, 56.0,
                                text_y, pill_y, PILL_H as f32, LIST_FONT, sel_c,
                                sw as f32 * 0.6, now,
                            );
                            f.draw(&r, &v.gl, &value, vx, text_y, LIST_FONT, sel_c);
                        } else {
                            let shown = f.fit(&v.gl, &label, LIST_FONT, sw as f32 * 0.6);
                            f.draw(&r, &v.gl, &shown, 56.0, text_y, LIST_FONT, theme.c4);
                            f.draw(&r, &v.gl, &value, vx, text_y, LIST_FONT, theme.c2);
                        }
                    }
                }
            }
            Screen::Wifi { nets, selected, scroll } => {
                if let Some(f) = font.as_mut() {
                    let on = proc_running("wpa_supplicant");
                    let head = if !on {
                        "WiFi is off".to_string()
                    } else if let Some((ssid, ip)) = wifi_status() {
                        format!("Connected: {ssid} ({ip})")
                    } else if wifi_scan_at.is_some() {
                        "Scanning...".to_string()
                    } else {
                        "Not connected".to_string()
                    };
                    f.draw(&r, &v.gl, &head, 32.0, 20.0, 22, theme.c6);
                    let top = 56.0;
                    let visible = 10usize;
                    for row in 0..visible.min(1 + nets.len()) {
                        let idx = scroll + row;
                        if idx > nets.len() {
                            break;
                        }
                        let (label, value) = if idx == 0 {
                            (
                                "WiFi".to_string(),
                                if on { "On".to_string() } else { "Off".to_string() },
                            )
                        } else {
                            let net = &nets[idx - 1];
                            let pct = ((net.signal + 100) * 2).clamp(0, 100);
                            let mut val = format!("{pct}%");
                            if net.secured {
                                val.push_str(" *");
                            }
                            if net.saved.is_some() {
                                val.push_str(" saved");
                            }
                            if net.current {
                                val = "Connected".to_string();
                            }
                            (net.ssid.clone(), val)
                        };
                        let y = top + row as f32 * ROW_H;
                        let lh = f.line_height(LIST_FONT);
                        let pill_y = y + (ROW_H - PILL_H as f32) / 2.0;
                        let text_y = pill_y + (PILL_H as f32 - lh) / 2.0;
                        let vw = f.measure(&v.gl, &value, LIST_FONT);
                        let vx = sw as f32 - 48.0 - vw;
                        if idx == *selected {
                            let sel_c =
                                theme.c1;
                            draw_roll(
                                f, &r, &v.gl, &mut roll_state, "wifi", &label, 56.0,
                                text_y, pill_y, PILL_H as f32, LIST_FONT, sel_c,
                                sw as f32 * 0.55, now,
                            );
                            f.draw(&r, &v.gl, &value, vx, text_y, LIST_FONT, sel_c);
                        } else {
                            let shown = f.fit(&v.gl, &label, LIST_FONT, sw as f32 * 0.55);
                            f.draw(&r, &v.gl, &shown, 56.0, text_y, LIST_FONT, theme.c4);
                            f.draw(&r, &v.gl, &value, vx, text_y, LIST_FONT, theme.c2);
                        }
                    }
                }
            }
            Screen::Osk { buf, pos, .. } => {
                if let Some(f) = font.as_mut() {
                    // typed name
                    let shown = if buf.is_empty() { "_" } else { buf.as_str() };
                    let tw = f.measure(&v.gl, shown, 36);
                    f.draw(&r, &v.gl, shown, (sw as f32 - tw) / 2.0, 90.0, 36, WHITE);
                    // grid
                    let cell = 74.0;
                    let n_chars = OSK_CHARS.chars().count();
                    let rows_n = n_chars.div_ceil(OSK_COLS);
                    let gw = OSK_COLS as f32 * cell;
                    let gx = (sw as f32 - gw) / 2.0;
                    let gy = 190.0;
                    for (i, c) in OSK_CHARS.chars().enumerate() {
                        let col = i % OSK_COLS;
                        let row = i / OSK_COLS;
                        let x = gx + col as f32 * cell;
                        let y = gy + row as f32 * cell;
                        let label = c.to_string();
                        let cw = f.measure(&v.gl, &label, 30);
                        if i == *pos {
                            pill.draw(&r, &v.gl, x + 4.0, y + (cell - PILL_H as f32) / 2.0, cell - 8.0, theme.c1);
                            f.draw(&r, &v.gl, &label, x + (cell - cw) / 2.0, y + 20.0, 30, theme.c7);
                        } else {
                            f.draw(&r, &v.gl, &label, x + (cell - cw) / 2.0, y + 20.0, 30, theme.c4);
                        }
                    }
                    let _ = rows_n;
                }
            }
            Screen::BootLogo { idx } => {
                let logos = sd.bootlogos();
                if bootlogo_tex.is_none()
                    && let Some(p) = logos.get(*idx)
                    && let Ok((w, h, px)) = kui_gfx::decode_bmp(p)
                    && let Ok(t) = kui_gfx::texture_from_rgba(&v.gl, w, h, &px)
                {
                    bootlogo_tex = Some(t);
                }
                if let Some(t) = &bootlogo_tex {
                    let scale = ((sw as f32 - 128.0) / t.w as f32)
                        .min((sh as f32 - 128.0) / t.h as f32)
                        .min(1.0);
                    let (dw, dh) = (t.w as f32 * scale, t.h as f32 * scale);
                    r.draw(&v.gl, t, (sw as f32 - dw) / 2.0, (sh as f32 - dh) / 2.0, dw, dh, WHITE);
                }
                if let (Some(f), Some(p)) = (font.as_mut(), logos.get(*idx)) {
                    let name = stem_of(p);
                    let label = if bootlogo_applied {
                        format!("{name}  (applied!)")
                    } else {
                        name
                    };
                    let tw = f.measure(&v.gl, &label, 24);
                    text_smoked(f, &r, &v.gl, &label, (sw as f32 - tw) / 2.0, 24.0, 24, theme.c1);
                }
            }
            Screen::Themes { idx } => {
                let variants = sd.theme_variants();
                if themes_tex.is_none()
                    && let Some((_, path)) = variants.get(*idx)
                {
                    let prev = path.join("sfc.png");
                    if let Ok((w, h, px)) = kui_gfx::decode_png(&prev)
                        && let Ok(t) = kui_gfx::texture_from_rgba(&v.gl, w, h, &px)
                    {
                        themes_tex = Some(t);
                    }
                }
                if let Some(t) = &themes_tex {
                    let scale = (sh as f32 - 160.0) / t.h as f32;
                    let (dw, dh) = (t.w as f32 * scale, t.h as f32 * scale);
                    r.draw(&v.gl, t, (sw as f32 - dw) / 2.0, 100.0, dw, dh, WHITE);
                }
                if let (Some(f), Some((name, _))) = (font.as_mut(), variants.get(*idx)) {
                    let tw = f.measure(&v.gl, name, 24);
                    text_smoked(f, &r, &v.gl, name, (sw as f32 - tw) / 2.0, 24.0, 24, theme.c1);
                }
            }
            Screen::LedEditor { row, profile, light } => {
                if let Some(f) = font.as_mut() {
                    let effect_idx = led_get(&cfg, *profile, *light, "effect") as usize;
                    let effect_name = tg5040::leds::EFFECTS
                        .get(effect_idx)
                        .map(|(_, n)| *n)
                        .unwrap_or("?");
                    let color = led_get(&cfg, *profile, *light, "color") as u32;
                    let rows: [(&str, String); 6] = [
                        ("Profile", LED_PROFILES[*profile].1.to_string()),
                        ("Light", LED_LIGHTS[*light].1.to_string()),
                        ("Effect", effect_name.to_string()),
                        ("Color", format!("{color:06X}")),
                        ("Speed", format!("{} ms", led_get(&cfg, *profile, *light, "duration"))),
                        ("Brightness", format!("{}", led_get(&cfg, *profile, *light, "brightness"))),
                    ];
                    let top = 56.0;
                    for (i, (lab, val)) in rows.iter().enumerate() {
                        let y = top + i as f32 * ROW_H;
                        let lh = f.line_height(LIST_FONT);
                        let pill_y = y + (ROW_H - PILL_H as f32) / 2.0;
                        let text_y = pill_y + (PILL_H as f32 - lh) / 2.0;
                        let sel = i == *row;
                        let sel_c = theme.c1;
                        let tc = if sel { sel_c } else { theme.c4 };
                        f.draw(&r, &v.gl, lab, 56.0, text_y, LIST_FONT, tc);
                        let vw = f.measure(&v.gl, val, LIST_FONT);
                        f.draw(&r, &v.gl, val, sw as f32 - 48.0 - vw, text_y, LIST_FONT, tc);
                        if i == 3 {
                            let c = [
                                ((color >> 16) & 0xFF) as f32 / 255.0,
                                ((color >> 8) & 0xFF) as f32 / 255.0,
                                (color & 0xFF) as f32 / 255.0,
                                1.0,
                            ];
                            r.rect(
                                &v.gl,
                                sw as f32 - 48.0 - vw - 40.0,
                                y + ROW_H / 2.0 - 14.0,
                                28.0,
                                28.0,
                                c,
                            );
                        }
                    }
                }
            }
            Screen::HubIndex { selected } => {
                if let Some(f) = font.as_mut() {
                    // anchored between the tray and the bottom bars
                    let visible = 10usize.min(hub_rows.len());
                    let row_h2 = 56.0;
                    let top = 56.0;
                    let cx = sw as f32 / 2.0;
                    for (ri, hr) in
                        hub_rows.iter().enumerate().skip(hub_scroll).take(visible)
                    {
                        let y = top + (ri - hub_scroll) as f32 * row_h2;
                        match hr {
                            HubRow::Header(name) => {
                                // small label with thin rules — never pilled,
                                // never focused
                                let hf: u32 = 18;
                                let hw = f.measure(&v.gl, name, hf);
                                let lh2 = f.line_height(hf);
                                let ty = y + (row_h2 - lh2) / 2.0;
                                let ly = y + row_h2 / 2.0;
                                let gap2 = 18.0;
                                let rule_w = 120.0;
                                r.rect(
                                    &v.gl,
                                    cx - hw / 2.0 - gap2 - rule_w,
                                    ly,
                                    rule_w,
                                    1.0,
                                    [0.35, 0.35, 0.35, 1.0],
                                );
                                r.rect(
                                    &v.gl,
                                    cx + hw / 2.0 + gap2,
                                    ly,
                                    rule_w,
                                    1.0,
                                    [0.35, 0.35, 0.35, 1.0],
                                );
                                f.draw(&r, &v.gl, name, cx - hw / 2.0, ty, hf, theme.c1);
                            }
                            HubRow::Page(i) => {
                                let p = &hub_pages[*i];
                                let lh = f.line_height(LIST_FONT);
                                let pill_y = y + (row_h2 - PILL_H as f32) / 2.0;
                                let text_y = pill_y + (PILL_H as f32 - lh) / 2.0;
                                let tw = f.measure(&v.gl, p.title, LIST_FONT);
                                if i == selected {
                                    let sel_c = theme.c1;
                                    f.draw(
                                        &r, &v.gl, p.title, cx - tw / 2.0, text_y,
                                        LIST_FONT, sel_c,
                                    );
                                } else {
                                    f.draw(
                                        &r, &v.gl, p.title, cx - tw / 2.0, text_y,
                                        LIST_FONT, theme.c4,
                                    );
                                }
                            }
                        }
                    }
                }
            }
            Screen::HubPage { page, selected } => {
                if let Some(f) = font.as_mut() {
                    let pg = &hub_pages[*page];
                    let top = 56.0;
                    // 9 rows fit above the description and hint bars
                    let _ = selected;
                    for (i, item) in
                        pg.items.iter().enumerate().skip(hub_page_scroll).take(9)
                    {
                        let y = top + (i - hub_page_scroll) as f32 * ROW_H;
                        let lh = f.line_height(LIST_FONT);
                        let pill_y = y + (ROW_H - PILL_H as f32) / 2.0;
                        let text_y = pill_y + (PILL_H as f32 - lh) / 2.0;
                        let value = if item.key == "theme.font" {
                            let opts = font_options(&sd);
                            let cur = cfg.get_or("theme.font", "0").to_string();
                            opts.iter()
                                .find(|(v2, _)| *v2 == cur)
                                .map(|(_, d)| d.clone())
                                .unwrap_or(cur)
                        } else if item.key == "cal.enabled" {
                            if cfg.get_or("cal.enabled", "on") == "on" {
                                "On".into()
                            } else {
                                "Off".into()
                            }
                        } else if let Some(ch) = item.key.strip_prefix("cal.gain.") {
                            cfg.get_i32(item.key, cal_gain_default(ch)).to_string()
                        } else if item.key.starts_with("fn.") {
                            if fn_page::FN_NUM.contains(&item.key) {
                                fn_page::fn_num_display(
                                    item.key,
                                    cfg.get_i32(item.key, fn_page::fn_num_default(item.key)),
                                )
                            } else if fn_page::FN_TURBO.contains(&item.key)
                                || item.key == "fn.leds"
                            {
                                if cfg.get_or(item.key, fn_page::fn_toggle_default(item.key))
                                    == "on"
                                {
                                    "On".into()
                                } else {
                                    "Off".into()
                                }
                            } else if item.key == "fn.dpad" {
                                match (
                                    cfg.get_or("fn.dpad_disable", "off") == "on",
                                    cfg.get_or("fn.joystick", "off") == "on",
                                ) {
                                    (true, true) => "Joystick".into(),
                                    (false, true) => "Both".into(),
                                    _ => "Dpad".into(),
                                }
                            } else {
                                String::new()
                            }
                        } else if item.key == "ra.prefetch" {
                            match ra_pf.lock() {
                                Ok(g) if g.3 && g.2.is_empty() => String::new(),
                                Ok(g) if g.3 => g.2.clone(),
                                Ok(g) => format!("{}/{} — {}", g.0, g.1, g.2),
                                Err(_) => String::new(),
                            }
                        } else if item.key == "cheats.download" {
                            match cheat_pf.lock() {
                                Ok(g) if g.3 && g.2.is_empty() => String::new(),
                                Ok(g) if g.3 => g.2.clone(),
                                Ok(g) => format!("{}/{} — {}", g.0, g.1, g.2),
                                Err(_) => String::new(),
                            }
                        } else if item.key == "ra.auth" {
                            ra_auth_msg.clone().unwrap_or_else(|| {
                                if cfg.get_or("ra.token", "").is_empty() {
                                    String::new()
                                } else {
                                    "Logged in".to_string()
                                }
                            })
                        } else if item.key == "ra.user" {
                            cfg.get_or("ra.user", "").to_string()
                        } else if item.key == "ra.pass" {
                            if cfg.get_or("ra.pass", "").is_empty() {
                                String::new()
                            } else {
                                "********".to_string()
                            }
                        } else if item.key.starts_with("radio.") {
                            let daemon = if item.key.ends_with("wifi") {
                                "wpa_supplicant"
                            } else {
                                "bluetoothd"
                            };
                            if on_device && proc_running(daemon) { "On".into() } else { "Off".into() }
                        } else if item.key == "dev.ssh" {
                            if on_device && proc_running("dropbear sshd") {
                                "On".into()
                            } else {
                                "Off".into()
                            }
                        } else if item.key.starts_with("dt.") {
                            match item.key {
                                "dt.year" => format!("{:04}", dt.0),
                                "dt.month" => format!("{:02}", dt.1),
                                "dt.day" => format!("{:02}", dt.2),
                                "dt.hour" => format!("{:02}", dt.3),
                                _ => format!("{:02}", dt.4),
                            }
                        } else if item.key.starts_with("theme.color") {
                            match cfg.get(item.key) {
                                Some(v) => v.to_uppercase(),
                                None => format!("{:06X}", theme_color_default(item.key)),
                            }
                        } else {
                            hub::display_value(&cfg, item)
                        };
                        let editable = !matches!(item.kind, hub::ItemKind::Info(_));
                        if item.key.starts_with("theme.color") {
                            let c = cfg.get_color(item.key, theme_color_default(item.key));
                            let sw_x = sw as f32 - 48.0 - 36.0
                                - f.measure(&v.gl, &value, LIST_FONT);
                            r.rect(&v.gl, sw_x, y + ROW_H / 2.0 - 14.0, 28.0, 28.0, c);
                        }
                        let vw = f.measure(&v.gl, &value, LIST_FONT);
                        let vx = sw as f32 - 48.0 - vw;
                        if i == *selected {
                            let sel_c =
                                theme.c1;
                            f.draw(&r, &v.gl, item.label, 56.0, text_y, LIST_FONT, sel_c);
                            f.draw(&r, &v.gl, &value, vx, text_y, LIST_FONT, sel_c);
                        } else {
                            f.draw(&r, &v.gl, item.label, 56.0, text_y, LIST_FONT, theme.c4);
                            let vc = if editable { theme.c2 } else { theme.c4 };
                            f.draw(&r, &v.gl, &value, vx, text_y, LIST_FONT, vc);
                        }
                    }
                }
            }
            Screen::Carousel => {
                let ph = sh as f32;
                let pw = PANEL_SRC.0 * ph / PANEL_SRC.1;
                let skew = sw as f32 / 12.0;
                let pstep = pw - skew;
                let cx = sw as f32 / 2.0 - pw / 2.0;
                for d in [-3i32, 3, -2, 2, -1, 1, 0] {
                    let i = wrap(tile, d, n);
                    let x = cx + d as f32 * pstep;
                    let tint = if d == 0 { WHITE } else { [DIM, DIM, DIM, 1.0] };
                    match bg.get(&i) {
                        Some(Art::Ready(t)) => r.draw(&v.gl, t, x, 0.0, pw, ph, tint),
                        _ => r.rect(
                            &v.gl,
                            x + 4.0,
                            4.0,
                            pw - 8.0,
                            ph - 8.0,
                            [0.10, 0.14, 0.10, tint[0]],
                        ),
                    }
                    if d == 0 {
                        // logos follow the theme (white/grayscale art tints
                        // cleanly); a smoky hardcoded-black contour under the
                        // letterforms (offset copies of the logo's own alpha,
                        // not a blob) keeps them readable over busy art
                        if let Some(Art::Ready(t)) = logo.get(&i) {
                            let (lw, lh) = (t.w as f32, t.h as f32);
                            let (lx, ly) = (x + (pw - lw) / 2.0, ph * 0.78 - lh / 2.0);
                            for (rad, a) in [(5.0f32, 0.16f32), (2.5, 0.38)] {
                                for (dx, dy) in [
                                    (rad, 0.0),
                                    (-rad, 0.0),
                                    (0.0, rad),
                                    (0.0, -rad),
                                    (rad * 0.7, rad * 0.7),
                                    (-rad * 0.7, rad * 0.7),
                                    (rad * 0.7, -rad * 0.7),
                                    (-rad * 0.7, -rad * 0.7),
                                ] {
                                    r.draw(&v.gl, t, lx + dx, ly + dy, lw, lh, [0.0, 0.0, 0.0, a]);
                                }
                            }
                            r.draw(&v.gl, t, lx, ly, lw, lh, theme.c1);
                        } else if let Some(f) = font.as_mut() {
                            let (_, name) = tiles[i].art_key(&platforms);
                            let tw = f.measure(&v.gl, &name, 36);
                            let (tx, ty2) = (x + (pw - tw) / 2.0, ph * 0.78 - 18.0);
                            for (dx, dy) in [(2.0, 0.0), (-2.0, 0.0), (0.0, 2.0), (0.0, -2.0)] {
                                f.draw(&r, &v.gl, &name, tx + dx, ty2 + dy, 36, [0.0, 0.0, 0.0, 0.4]);
                            }
                            f.draw(&r, &v.gl, &name, tx, ty2, 36, theme.c1);
                        }
                    }
                }
            }
            Screen::Quick { items, selected, .. } => {
                r.rect(&v.gl, 0.0, 0.0, sw as f32, sh as f32, [0.0, 0.0, 0.0, 0.75]);
                if let Some(f) = font.as_mut() {
                    let count = items.len();
                    let total_h = count as f32 * ROW_H;
                    let top = (sh as f32 - total_h) / 2.0;
                    let cx = sw as f32 / 2.0;
                    for (i, it) in items.iter().enumerate() {
                        let y = top + i as f32 * ROW_H;
                        let lh = f.line_height(LIST_FONT);
                        let pill_y = y + (ROW_H - PILL_H as f32) / 2.0;
                        let text_y = pill_y + (PILL_H as f32 - lh) / 2.0;
                        let base_c = if i == *selected { theme.c1 } else { theme.c4 };
                        if let Some(val) = it.value.as_deref() {
                            // label (row color) + accent-highlighted value,
                            // the pair centered together
                            let bw = f.measure(&v.gl, &it.label, LIST_FONT);
                            let gw = f.measure(&v.gl, "  ", LIST_FONT);
                            let vw = f.measure(&v.gl, val, LIST_FONT);
                            let x0 = cx - (bw + gw + vw) / 2.0;
                            f.draw(&r, &v.gl, &it.label, x0, text_y, LIST_FONT, base_c);
                            f.draw(&r, &v.gl, val, x0 + bw + gw, text_y, LIST_FONT, theme.c2);
                        } else {
                            let tw = f.measure(&v.gl, &it.label, LIST_FONT);
                            f.draw(&r, &v.gl, &it.label, cx - tw / 2.0, text_y, LIST_FONT, base_c);
                        }
                    }
                }
            }
            Screen::Switcher { entries, idx, .. } => {
                if let Some(ent) = entries.get(*idx) {
                    let tex = switcher_art.entry(*idx).or_insert_with(|| {
                        switcher_art_path(&cfg, &sd, &ent.rom)
                            .and_then(|p| kui_gfx::load_png(&v.gl, &p).ok())
                    });
                    if let Some(t) = tex {
                        let max_w = sw as f32 * 0.55;
                        let max_h = sh as f32 * 0.58;
                        let s = (max_w / t.w as f32).min(max_h / t.h as f32);
                        let (dw, dh) = (t.w as f32 * s, t.h as f32 * s);
                        r.draw(
                            &v.gl,
                            t,
                            (sw as f32 - dw) / 2.0,
                            sh as f32 * 0.12 + (max_h - dh) / 2.0,
                            dw,
                            dh,
                            kui_gfx::WHITE,
                        );
                    }
                    if let Some(f) = font.as_mut() {
                        let label =
                            format!("{}   {} / {}", ent.alias, *idx + 1, entries.len());
                        let shown = f.fit(&v.gl, &label, 28, sw as f32 * 0.85);
                        let tw = f.measure(&v.gl, &shown, 28);
                        text_smoked(
                            f,
                            &r,
                            &v.gl,
                            &shown,
                            (sw as f32 - tw) / 2.0,
                            sh as f32 * 0.78,
                            28,
                            [1.0, 1.0, 1.0, 1.0],
                        );
                    }
                }
            }
            Screen::Dude => {
                if let (Some(f), Some(d)) = (font.as_mut(), dude_state.as_ref()) {
                    // display-only level-100 preview (touch /tmp/kui_dude100)
                    let preview100 = std::path::Path::new("/tmp/kui_dude100").exists();
                    let rank =
                        if preview100 { "The Grand Master Dude" } else { d.rank_title() };
                    // left panel: rank + menu
                    f.draw(&r, &v.gl, rank, 32.0, 28.0, 24, theme.c1);
                    let menu_top = 110.0;
                    for (i, label) in d.menu().iter().enumerate() {
                        let y = menu_top + i as f32 * 56.0;
                        let lh = f.line_height(24);
                        let pill_y = y + (56.0 - PILL_H as f32) / 2.0;
                        let text_y = pill_y + (PILL_H as f32 - lh) / 2.0;
                        if i == dude_menu {
                            let sel_c = theme.c1;
                            f.draw(&r, &v.gl, label, 48.0, text_y, 24, sel_c);
                        } else {
                            f.draw(&r, &v.gl, label, 48.0, text_y, 24, theme.c4);
                        }
                    }
                    // right panel: face, badges, mood
                    let rx = 380.0;
                    let rw = sw as f32 - rx - 32.0;
                    let face = d.face();
                    let fw = f.measure(&v.gl, face, 140);
                    f.draw(&r, &v.gl, face, rx + (rw - fw) / 2.0, 48.0, 140, theme.c1);
                    if preview100 || d.has_crown() {
                        // gold pixel crown above the face (no font dependency)
                        let (cs, cg) = (12.0, 2.0);
                        let cw = 5.0 * cs + 4.0 * cg;
                        let cx0 = rx + (rw - cw) / 2.0;
                        let gold = [1.0, 0.84, 0.0, 1.0];
                        // sits just above the head, not at the panel top
                        let cy = 26.0;
                        for col in 0..5 {
                            let x = cx0 + col as f32 * (cs + cg);
                            if col % 2 == 0 {
                                r.rect(&v.gl, x, cy, cs, cs, gold);
                            }
                            r.rect(&v.gl, x, cy + cs + cg, cs, cs, gold);
                            r.rect(&v.gl, x, cy + 2.0 * (cs + cg), cs, cs, gold);
                        }
                    }
                    // medal(s) sit LEFT of the mood/level/humor line
                    let stars = if preview100 {
                        "***".to_string()
                    } else {
                        "*".repeat(d.stars().chars().count())
                    };
                    let mood = if preview100 {
                        let m = d.mood_line();
                        let parts: Vec<&str> = m.split(" | ").collect();
                        if parts.len() == 3 {
                            format!("{} | Lv.100 | {}", parts[0], parts[2])
                        } else {
                            m
                        }
                    } else {
                        d.mood_line()
                    };
                    let mw = f.measure(&v.gl, &mood, 24);
                    let sw2 =
                        if stars.is_empty() { 0.0 } else { f.measure(&v.gl, &stars, 24) + 14.0 };
                    let mx = rx + (rw - (sw2 + mw)) / 2.0;
                    if !stars.is_empty() {
                        f.draw(&r, &v.gl, &stars, mx, 238.0, 24, theme.c2);
                    }
                    f.draw(&r, &v.gl, &mood, mx + sw2, 238.0, 24, theme.c1);
                    // XP bar toward next level, in the gap under the mood line
                    let (xp_in, xp_span) = d.xp_progress();
                    let xp_label = format!("{} / {} XP", xp_in, xp_span);
                    let lw = f.measure(&v.gl, &xp_label, 16);
                    let row_w = rw * 0.7;
                    let bar_w = (row_w - lw - 12.0).max(60.0);
                    let bar_x = rx + (rw - (bar_w + 12.0 + lw)) / 2.0;
                    r.rect(&v.gl, bar_x, 282.0, bar_w, 8.0, [0.22, 0.22, 0.22, 1.0]);
                    let frac = if preview100 {
                        1.0
                    } else if xp_span > 0 {
                        (xp_in as f32 / xp_span as f32).clamp(0.0, 1.0)
                    } else {
                        1.0
                    };
                    if frac > 0.0 {
                        r.rect(&v.gl, bar_x, 282.0, bar_w * frac, 8.0, theme.c2);
                    }
                    f.draw(&r, &v.gl, &xp_label, bar_x + bar_w + 12.0, 278.0, 16, theme.c4);
                    // content
                    let cy0 = 308.0;
                    let item = d.menu()[dude_menu];
                    let mut simple_lines: Option<Vec<String>> = None;
                    match item {
                        "Talk" => {
                            simple_lines = Some(wrap_text(f, &v.gl, &dude_text, 24, rw));
                        }
                        "Dude Quests" => {
                            let qs = d.dude_quests();
                            if last_marquee_sel != dude_sel {
                                last_marquee_sel = dude_sel;
                                marquee_start = now;
                            }
                            for (i, q) in qs.iter().take(8).enumerate() {
                                let sel = i == dude_sel;
                                let row = format!(
                                    "{} {} +{}xp",
                                    if sel { ">" } else { "  " },
                                    q.text,
                                    q.xp
                                );
                                let y = cy0 + i as f32 * 44.0;
                                let col = if sel { theme.c1 } else { theme.c4 };
                                let full_w = f.measure(&v.gl, &row, 24);
                                if sel && full_w > rw {
                                    // long titles roll, list-style
                                    let overflow = full_w - rw;
                                    let (speed, pause) = (60.0, 1.0);
                                    let cycle = pause + overflow / speed + pause;
                                    let t = (now - marquee_start).as_secs_f32() % cycle;
                                    let off = ((t - pause).max(0.0) * speed).min(overflow);
                                    r.scissor(&v.gl, rx, y, rw, 40.0);
                                    f.draw(&r, &v.gl, &row, rx - off, y, 24, col);
                                    r.scissor_off(&v.gl);
                                } else {
                                    let shown = f.fit(&v.gl, &row, 24, rw);
                                    f.draw(&r, &v.gl, &shown, rx, y, 24, col);
                                }
                            }
                            if !qs.is_empty() {
                                let ft = format!(
                                    "< Quest {}/{} >  A to accept",
                                    dude_sel + 1,
                                    qs.len()
                                );
                                let fw2 = f.measure(&v.gl, &ft, 22);
                                f.draw(
                                    &r,
                                    &v.gl,
                                    &ft,
                                    rx + (rw - fw2) / 2.0,
                                    sh as f32 - 96.0,
                                    22,
                                    theme.c6,
                                );
                            }
                        }
                        "Daily Quests" => {
                            let mut l = d.daily_lines();
                            l.push(String::new());
                            l.push("Refreshes every day.".to_string());
                            simple_lines = Some(l);
                        }
                        "Weekly Challenge" => {
                            let mut l = d.weekly_lines();
                            l.push(String::new());
                            l.push("Refreshes every Monday.".to_string());
                            simple_lines = Some(l);
                        }
                        "Completed" => simple_lines = Some(d.completed_lines()),
                        "Stats" => simple_lines = Some(d.stats_lines()),
                        "Achievements" => {
                            let pages = d.achievement_pages();
                            let all: Vec<&(bool, String, String)> =
                                pages.iter().flatten().collect();
                            let n_pages = all.len().div_ceil(12).max(1);
                            for (i, (done, name, desc)) in
                                all.iter().skip(dude_sel * 12).take(12).enumerate()
                            {
                                let col = i / 6;
                                let row = i % 6;
                                let x = rx + col as f32 * (rw / 2.0);
                                let y = cy0 + row as f32 * 62.0;
                                let mark = if *done { "[*] " } else { "[ ] " };
                                let nm =
                                    f.fit(&v.gl, &format!("{mark}{name}"), 24, rw / 2.0 - 16.0);
                                f.draw(
                                    &r,
                                    &v.gl,
                                    &nm,
                                    x,
                                    y,
                                    24,
                                    if *done { theme.c1 } else { theme.c4 },
                                );
                                let dsc = f.fit(&v.gl, desc, 18, rw / 2.0 - 16.0);
                                f.draw(&r, &v.gl, &dsc, x + 30.0, y + 28.0, 18, theme.c6);
                            }
                            let pg = format!("< Page {}/{} >", dude_sel + 1, n_pages);
                            let pw2 = f.measure(&v.gl, &pg, 20);
                            f.draw(
                                &r,
                                &v.gl,
                                &pg,
                                rx + (rw - pw2) / 2.0,
                                sh as f32 - 84.0,
                                20,
                                theme.c6,
                            );
                        }
                        "Play History" => {
                            let lines = d.history_lines();
                            let n_pages = lines.len().div_ceil(4).max(1);
                            for (i, entry) in
                                lines.iter().skip(dude_sel * 4).take(4).enumerate()
                            {
                                let y = cy0 + (i * 2) as f32 * 40.0;
                                let mut parts = entry.splitn(2, '\n');
                                if let Some(name) = parts.next() {
                                    let nm = f.fit(&v.gl, name, 24, rw);
                                    f.draw(&r, &v.gl, &nm, rx, y, 24, theme.c4);
                                }
                                if let Some(meta) = parts.next() {
                                    f.draw(&r, &v.gl, meta, rx, y + 40.0, 18, theme.c6);
                                }
                            }
                            let pg = format!("< Page {}/{} >", dude_sel + 1, n_pages);
                            let pw2 = f.measure(&v.gl, &pg, 20);
                            f.draw(
                                &r,
                                &v.gl,
                                &pg,
                                rx + (rw - pw2) / 2.0,
                                sh as f32 - 84.0,
                                20,
                                theme.c6,
                            );
                        }
                        "Streaks" => {
                            let (cells, footer) = d.streak_grid();
                            let (cell, gap) = (36.0, 10.0);
                            let grid_w = 7.0 * cell + 6.0 * gap;
                            let grid_h = 4.0 * cell + 3.0 * gap;
                            // center the whole block in the content area
                            let block_h = 30.0 + 14.0 + grid_h + 18.0 + 22.0;
                            let avail = (sh as f32 - 96.0) - cy0;
                            let y0 = cy0 + (avail - block_h).max(0.0) / 2.0;
                            let gx = rx + (rw - grid_w) / 2.0;
                            let hd = "Last 28 days:";
                            let hw = f.measure(&v.gl, hd, 24);
                            f.draw(&r, &v.gl, hd, rx + (rw - hw) / 2.0, y0, 24, theme.c4);
                            for (i, played) in cells.iter().enumerate() {
                                let x = gx + (i % 7) as f32 * (cell + gap);
                                let y = y0 + 44.0 + (i / 7) as f32 * (cell + gap);
                                let col = if *played {
                                    theme.c1
                                } else {
                                    [0.22, 0.22, 0.22, 1.0]
                                };
                                r.rect(&v.gl, x, y, cell, cell, col);
                            }
                            let fw2 = f.measure(&v.gl, &footer, 22);
                            f.draw(
                                &r,
                                &v.gl,
                                &footer,
                                rx + (rw - fw2) / 2.0,
                                y0 + 44.0 + grid_h + 18.0,
                                22,
                                theme.c4,
                            );
                        }
                        "Reset Progress" => {
                            let msg = if dude_armed {
                                "A to CONFIRM reset (all progress lost!)"
                            } else {
                                "A to reset progress"
                            };
                            let col = if dude_armed { theme.c2 } else { theme.c4 };
                            simple_lines = None;
                            let mw2 = f.measure(&v.gl, msg, 24);
                            f.draw(&r, &v.gl, msg, rx + (rw - mw2) / 2.0, cy0 + 60.0, 24, col);
                        }
                        _ => {}
                    }
                    if let Some(lines) = simple_lines {
                        for (i, line) in lines.iter().take(9).enumerate() {
                            let shown = f.fit(&v.gl, line, 24, rw);
                            // completed quests glow in the theme color
                            let col =
                                if line.starts_with("[*]") { theme.c1 } else { theme.c4 };
                            f.draw(&r, &v.gl, &shown, rx, cy0 + i as f32 * 40.0, 24, col);
                        }
                    }
                }
            }
            Screen::List { kind, rows, selected, scroll, show_art, .. } => {
                // Backgrounds: game/collection lists use their folder bg;
                // the Covers-mode root uses the global bg; Lists mode is
                // plain black everywhere.
                // Backgrounds are a ROOT-only affair (Arjun: game lists stay
                // clean). Explicit bg.png covers the screen; otherwise the
                // selected tile's carousel panel shows aspect-fit on the
                // right — the full art, never stretched.
                let root_panel = matches!(kind, ListKind::Root) && ui_mode != UiMode::Lists;
                if root_panel {
                    if let Some(Art::Ready(t)) = fbg.get(&ROOT_FBG) {
                        draw_cover(&r, &v.gl, t, sw as f32, sh as f32, 0.55);
                    } else if let Some(RowAction::OpenTile(i)) =
                        rows.get(*selected).map(|row| &row.action)
                        && let Some(Art::Ready(t)) = bg.get(i)
                    {
                        let scale = sh as f32 / t.h as f32;
                        let w = t.w as f32 * scale;
                        r.draw(&v.gl, t, sw as f32 - w, 0.0, w, sh as f32, WHITE);
                    }
                }
                let visible = visible_rows();
                let list_x = 32.0;
                let top_off = 16.0;
                // Lists mode is text-only: no box-art panel, even for
                // collections (whose detail sets show_art).
                let show_art = *show_art && ui_mode != UiMode::Lists;
                let list_w = if show_art {
                    sw as f32 * 0.55 - list_x
                } else if root_panel {
                    sw as f32 * 0.62 - list_x
                } else {
                    sw as f32 - 2.0 * list_x
                };
                let top = top_off;
                for row_i in 0..visible.min(rows.len()) {
                    let idx = scroll + row_i;
                    if idx >= rows.len() {
                        break;
                    }
                    let y = top + row_i as f32 * ROW_H;
                    let is_sel = idx == *selected;
                    // pinned rows: the "> " marker draws separately in c2 and
                    // stays put while long names marquee behind it
                    let (pinned, name): (bool, &str) =
                        match rows[idx].label.strip_prefix("> ") {
                            Some(rest) => (true, rest),
                            None => (false, rows[idx].label.as_str()),
                        };
                    if let Some(f) = font.as_mut() {
                        let pre_w = if pinned { f.measure(&v.gl, "> ", LIST_FONT) } else { 0.0 };
                        let text_max = list_w - 48.0 - pre_w;
                        let text_x = list_x + 24.0 + pre_w;
                        let lh = f.line_height(LIST_FONT);
                        let pill_y = y + (ROW_H - PILL_H as f32) / 2.0;
                        let text_y = pill_y + (PILL_H as f32 - lh) / 2.0;
                        if is_sel {
                            if last_marquee_sel != idx {
                                last_marquee_sel = idx;
                                marquee_start = now;
                            }
                            let full_w = f.measure(&v.gl, name, LIST_FONT);
                            if full_w > text_max {
                                let overflow = full_w - text_max;
                                let speed = 60.0;
                                let pause = 1.0;
                                let roll = overflow / speed;
                                let cycle = pause + roll + pause;
                                let t = (now - marquee_start).as_secs_f32() % cycle;
                                let off = if t < pause {
                                    0.0
                                } else if t < pause + roll {
                                    (t - pause) * speed
                                } else {
                                    overflow
                                };
                                let sel_c =
                                    theme.c1;
                                if pinned {
                                    f.draw(&r, &v.gl, "> ", list_x + 24.0, text_y, LIST_FONT, theme.c2);
                                }
                                r.scissor(&v.gl, text_x, y, text_max, ROW_H);
                                f.draw(&r, &v.gl, name, text_x - off, text_y, LIST_FONT, sel_c);
                                r.scissor_off(&v.gl);
                            } else {
                                let sel_c =
                                    theme.c1;
                                if pinned {
                                    f.draw(&r, &v.gl, "> ", list_x + 24.0, text_y, LIST_FONT, theme.c2);
                                }
                                f.draw(&r, &v.gl, name, text_x, text_y, LIST_FONT, sel_c);
                            }
                        } else {
                            let shown = f.fit(&v.gl, name, LIST_FONT, text_max);
                            if pinned {
                                f.draw(&r, &v.gl, "> ", list_x + 24.0, text_y, LIST_FONT, theme.c2);
                            }
                            f.draw(&r, &v.gl, &shown, text_x, text_y, LIST_FONT, theme.c4);
                        }
                    }
                }
                if show_art {
                    let show_footer = ui_mode == UiMode::Carousel;
                    let area_x = sw as f32 * 0.58;
                    let area_w = sw as f32 - area_x - 32.0;
                    let area_h = sh as f32 - 64.0;
                    let footer = match boxart.get(selected) {
                        Some(Art::Ready(t)) => {
                            let scale = (area_w / t.w as f32).min(area_h / t.h as f32).min(1.5);
                            let (dw, dh) = (t.w as f32 * scale, t.h as f32 * scale);
                            let (dx, dy) = (area_x + (area_w - dw) / 2.0, 32.0 + (area_h - dh) / 2.0);
                            r.draw(&v.gl, t, dx, dy, dw, dh, WHITE);
                            Some((dy + dh + 14.0, dw, dx + dw / 2.0))
                        }
                        Some(Art::Missing) => {
                            Some((32.0 + area_h / 2.0, area_w, area_x + area_w / 2.0))
                        }
                        _ => None,
                    };
                    if let (true, Some((footer_y, footer_w, footer_cx)), Some(f), Some(Some(line))) =
                        (show_footer, footer, font.as_mut(), infos.get(selected))
                    {
                        let col = [1.0, 1.0, 1.0, 0.82];
                        let tw = f.measure(&v.gl, line, META_FONT);
                        if tw <= footer_w {
                            f.draw(&r, &v.gl, line, footer_cx - tw / 2.0, footer_y, META_FONT, col);
                        } else {
                            let overflow = tw - footer_w;
                            let speed = 60.0;
                            let pause = 1.0;
                            let roll = overflow / speed;
                            let cycle = pause + roll + pause;
                            let t2 = (now - marquee_start).as_secs_f32() % cycle;
                            let off = if t2 < pause {
                                0.0
                            } else if t2 < pause + roll {
                                (t2 - pause) * speed
                            } else {
                                overflow
                            };
                            let fx = footer_cx - footer_w / 2.0;
                            r.scissor(&v.gl, fx, footer_y, footer_w, 32.0);
                            f.draw(&r, &v.gl, line, fx - off, footer_y, META_FONT, col);
                            r.scissor_off(&v.gl);
                        }
                    }
                }
            }
        }

        // ---- chrome: title pill, tag pill, tray, button hints ----
        status.refresh(on_device);
        if let Some(f) = font.as_mut() {
            let cy = 16.0;
            let cfont: u32 = 24;
            // bare tray (top-right): wifi/bt (when up) + battery gauge,
            // identifiers tinted with the theme main color, no pill
            let mut tray_w = 0.0;
            if let Some(sheet) = &assets_tex {
                let icon = 26.0;
                let batt_w = 34.0;
                let batt_h = 20.0;
                let gap = 14.0;
                let row_h = icon;
                let mut ix = sw as f32 - 16.0 - batt_w;
                let by = cy + (row_h - batt_h) / 2.0;
                let bx = ix; // fill anchors here even after the percent text shifts ix
                tex_smoked(&r, &v.gl, sheet, ix, by, batt_w, batt_h, asset_uv(A_BATTERY), theme.c1);
                if cfg.get_or("ui.battery_percent", "off") == "on"
                    && let Some(pct) = status.batt
                {
                    let txt = format!("{pct}%");
                    let pfont: u32 = 20;
                    let tw = f.measure(&v.gl, &txt, pfont);
                    let tlh = f.line_height(pfont);
                    ix -= tw + 10.0;
                    text_smoked(
                        f, &r, &v.gl, &txt, ix, cy + (row_h - tlh) / 2.0, pfont, theme.c1,
                    );
                }
                // charging shows the real level too (Arjun's call); the
                // bolt body is reserved for a confirmed full charge
                let charged = status.charging && status.batt.is_some_and(|p| p >= 100);
                let fill_frac = if charged {
                    None
                } else {
                    status.batt.map(|pct| (pct as f32 / 100.0).clamp(0.0, 1.0))
                };
                if let Some(frac) = fill_frac {
                    let full = A_BATTERY_FILL;
                    // the tip points left: anchor right, drain toward the tip
                    let fill_rect =
                        (full.0 + full.2 * (1.0 - frac), full.1, full.2 * frac, full.3);
                    r.draw_uv(
                        &v.gl,
                        sheet,
                        bx + 6.0 + 24.0 * (1.0 - frac),
                        by + 4.0,
                        24.0 * frac,
                        12.0,
                        asset_uv(fill_rect),
                        theme.c1,
                    );
                } else if charged {
                    // full body with bolt knockout
                    r.draw_uv(
                        &v.gl,
                        sheet,
                        bx + 2.0,
                        by,
                        32.0,
                        20.0,
                        asset_uv(A_BATTERY_BOLT),
                        theme.c1,
                    );
                }
                if status.charging && !charged {
                    // pixel bolt over the fill (crown-style): plugged in
                    // always shows a bolt, without hiding the real level
                    let (x0, y0) = (bx + 13.0, by + 4.0);
                    let bolt = [
                        (4.0, 0.0, 4.0, 3.0),
                        (2.0, 3.0, 4.0, 3.0),
                        (1.0, 5.0, 6.0, 2.0),
                        (3.0, 7.0, 4.0, 2.0),
                        (1.0, 9.0, 4.0, 3.0),
                    ];
                    for (dx, dy, w, h) in bolt {
                        r.rect(
                            &v.gl,
                            x0 + dx - 1.0,
                            y0 + dy - 1.0,
                            w + 2.0,
                            h + 2.0,
                            [0.0, 0.0, 0.0, 0.55],
                        );
                    }
                    for (dx, dy, w, h) in bolt {
                        r.rect(&v.gl, x0 + dx, y0 + dy, w, h, [1.0, 1.0, 1.0, 1.0]);
                    }
                }
                if status.bt {
                    ix -= icon + gap;
                    tex_smoked(&r, &v.gl, sheet, ix, cy, icon, icon, asset_uv(A_BLUETOOTH), theme.c1);
                }
                if status.wifi {
                    ix -= icon + gap;
                    tex_smoked(&r, &v.gl, sheet, ix, cy, icon, icon, asset_uv(A_WIFI), theme.c1);
                }
                tray_w = sw as f32 - 16.0 - ix + 18.0;
            }
            // list identifiers, bare: tag (theme c1) right-aligned before the
            // tray; title (white) top-left for lists without a tag.
            // The switcher shows the focused game's platform the same way.
            let chrome_tag: Option<String> = match &screen {
                // collections span platforms — show the highlighted game's
                // code, updating as you scroll (as the switcher does).
                Screen::List {
                    kind: ListKind::Collection(_) | ListKind::SmartCollection,
                    rows,
                    selected,
                    ..
                } => rows.get(*selected).and_then(|row| match &row.action {
                    RowAction::Launch(p) => sd.tag_of_rom(p),
                    _ => None,
                }),
                Screen::List { tag, .. } => tag.clone(),
                Screen::Switcher { entries, idx, .. } => {
                    entries.get(*idx).and_then(|e| sd.tag_of_rom(&e.rom))
                }
                _ => None,
            };
            if let Some(tag) = &chrome_tag {
                // The green platform-code identifier shows in every mode;
                // the platform-name title never does (the code IS the
                // context). Titles only for tag-less lists.
                let tlh = f.line_height(cfont);
                let text_cy = cy + (26.0 - tlh) / 2.0; // centered on the icon row
                // clearly separated from the tray: different things
                let tw = f.measure(&v.gl, tag, cfont);
                let x = sw as f32 - 16.0 - tray_w - 36.0 - tw;
                text_smoked(f, &r, &v.gl, tag, x, text_cy, cfont, theme.c1);
            }
            // button hints (bottom-right): bare — green letter (or the
            // TrimUI hexagon icon for MENU) + white label, no pills.
            // The root list is a root: B does nothing there, MENU does.
            // bottom description (left side) for the hub screens
            let desc: Option<&str> = match &screen {
                Screen::Quick { items, selected, .. } => {
                    items.get(*selected).map(|i| i.desc)
                }
                Screen::HubIndex { selected } => {
                    hub_pages.get(*selected).map(|p| p.desc)
                }
                Screen::HubPage { page, selected } => hub_pages
                    .get(*page)
                    .and_then(|p| p.items.get(*selected))
                    .map(|i| i.desc),
                Screen::BootLogo { .. } => Some("Shown at power on. Applies to the boot partition."),
                Screen::Themes { .. } => Some("Applying restarts kUI to reload the art."),
                Screen::InputTest => Some("Hold MENU to exit."),
                Screen::PortForge { pane } => Some(
                    match pane.rows.get(pane.selected) {
                        Some(fr) if fr.is_package => "Port Forge package — A installs it into Ports.",
                        Some(fr) if fr.is_game => "RPG Maker game found — A installs it as a port.",
                        _ => "Open your extracted game folder, or a Port Forge Web package.",
                    },
                ),
                Screen::PortForgeRun { .. } => None,
                // busy note during an async paste/delete; otherwise the
                // button hints already say Pane / Menu
                Screen::Files { .. } => {
                    if files_busy.load(std::sync::atomic::Ordering::Relaxed) > 0 {
                        files_verb.lock().ok().map(|v| *v)
                    } else {
                        None
                    }
                }
                Screen::LedEditor { row, profile, .. } => Some(match row {
                    0 if *profile == 0 => "The everyday look. Applied now and at boot.",
                    0 => "Applied on its event once the daemon lands.",
                    2 => "All 8 hardware effects.",
                    3 => "Press A to open the color picker.",
                    _ => "",
                }),
                _ => None,
            };
            if let Some(d) = desc
                && !d.is_empty()
            {
                let dfont: u32 = 20;
                let dlh = f.line_height(dfont);
                let dy = sh as f32 - 16.0 - dlh;
                let max_w = sw as f32 * 0.55;
                let full_w = f.measure(&v.gl, d, dfont);
                if full_w > max_w {
                    let off = roll_offset(&mut roll_state, "desc", d, full_w - max_w, now);
                    r.scissor(&v.gl, 16.0, dy - 4.0, max_w, dlh + 8.0);
                    text_smoked(f, &r, &v.gl, d, 16.0 - off, dy, dfont, theme.c6);
                    r.scissor_off(&v.gl);
                } else {
                    text_smoked(f, &r, &v.gl, d, 16.0, dy, dfont, theme.c6);
                }
            }
            let wipe_hint = if wiping.load(std::sync::atomic::Ordering::Relaxed) > 0 {
                "Wiping…"
            } else if wipe_armed.is_some() {
                "Sure?"
            } else {
                "Wipe"
            };
            let _ = &wipe_hint;
            let hints: &[(&str, &str)] = match &screen {
                Screen::Carousel => &[
                    ("MENU", "Menu"),
                    ("SELECT", "Switcher"),
                    ("START", "The Dude"),
                    ("A", "Open"),
                ],
                Screen::List { kind: ListKind::Root, .. } => &[
                    ("MENU", "Menu"),
                    ("SELECT", "Switcher"),
                    ("START", "The Dude"),
                    ("A", "Open"),
                ],
                Screen::List { kind: ListKind::Platform(_), rows, selected, .. } => {
                    match rows.get(*selected) {
                        // the Random row: not pinnable, not wipeable
                        Some(Row { action: RowAction::LaunchRandom, .. }) => {
                            &[("A", "Open"), ("B", "Back")]
                        }
                        Some(row) if row.label.starts_with("> ") => {
                            &[("Y", ""), ("X", "Unpin"), ("A", "Open"), ("B", "Back")]
                        }
                        _ => &[("Y", ""), ("X", "Pin"), ("A", "Open"), ("B", "Back")],
                    }
                }
                Screen::List { kind: ListKind::Recents, .. } => {
                    &[("Y", ""), ("A", "Open"), ("B", "Back")]
                }
                Screen::List { kind: ListKind::CollectionsIndex, .. } => {
                    &[("Y", ""), ("X", "New"), ("A", "Open"), ("B", "Back")]
                }
                Screen::List { kind: ListKind::Collection(_), .. } => {
                    &[("Y", ""), ("X", "Add"), ("A", "Open"), ("B", "Back")]
                }
                Screen::List { kind: ListKind::SmartCollection, .. } => {
                    &[("A", "Open"), ("B", "Back")]
                }
                Screen::List { kind: ListKind::PickPlatform(_) | ListKind::PickGame(_), .. } => {
                    &[("A", "Pick"), ("B", "Back")]
                }
                Screen::Quick { .. } => &[("A", "Select"), ("B", "Close")],
                Screen::Dude => &[("</>", "Browse"), ("A", "Okay"), ("B", "Back")],
                Screen::Switcher { .. } => {
                    &[("</>", "Browse"), ("Y", "Remove"), ("A", "Resume"), ("B", "Close")]
                }
                Screen::HubIndex { .. } => &[("A", "Open"), ("B", "Back")],
                Screen::HubPage { .. } => &[("</>", "Change"), ("A", "Edit"), ("B", "Back")],
                Screen::ColorPick { .. } => {
                    &[("</>", "Fine"), ("L/R", "Coarse"), ("A", "Apply"), ("B", "Cancel")]
                }
                Screen::LedEditor { .. } => &[("</>", "Change"), ("A", "Edit"), ("B", "Back")],
                Screen::CoreList { .. } => &[("A", "Open"), ("B", "Back")],
                Screen::Wifi { .. } => {
                    &[("Y", "Forget"), ("X", "Rescan"), ("A", "Connect"), ("B", "Back")]
                }
                Screen::Bt { .. } => {
                    &[("Y", "Forget"), ("X", "Rescan"), ("A", "Pair"), ("B", "Back")]
                }
                Screen::List { kind: ListKind::Paks, .. } => &[("A", "Open"), ("B", "Back")],
                Screen::GameTime { .. } => &[("</>", "Jump"), ("B", "Back")],
                Screen::InputTest => &[("MENU", "Hold to exit")],
                Screen::PortForge { pane } => match pane.rows.get(pane.selected) {
                    Some(fr) if fr.is_package || fr.is_game => &[("A", "Install"), ("B", "Back")],
                    Some(_) => &[("A", "Open"), ("B", "Back")],
                    None => &[("B", "Back")],
                },
                Screen::PortForgeRun { .. } => {
                    let busy = store_msg
                        .lock()
                        .map(|m| !m.is_empty() && !m.starts_with("Done") && !m.contains("Failed"))
                        .unwrap_or(false);
                    if busy {
                        &[]
                    } else if forge_del.lock().map(|d| d.is_some()).unwrap_or(false) {
                        &[("Y", "Delete original"), ("B", "Keep")]
                    } else {
                        &[("B", "Back")]
                    }
                }
                Screen::Files { menu: Some(_), .. } => &[("A", "Select"), ("B", "Close")],
                Screen::Files { panes, active, .. } => {
                    let pane = &panes[*active];
                    if pane.rows.get(pane.selected).is_some_and(|fr| fr.is_dir) {
                        &[("</>", "Pane"), ("START", "Menu"), ("A", "Open"), ("B", "Back")]
                    } else {
                        &[("</>", "Pane"), ("START", "Menu"), ("B", "Back")]
                    }
                }
                Screen::Battery { .. } => &[("</>", "Zoom"), ("B", "Back")],
                Screen::ScraperPlatforms { .. } => &[("A", "Open"), ("B", "Back")],
                Screen::CheatPlatforms { .. } | Screen::PrefetchPlatforms { .. } => {
                    &[("A", "Download"), ("B", "Back")]
                }
                Screen::PakCats { .. } => &[("A", "Open"), ("B", "Back")],
                Screen::PakDek { paks, selected, .. } => {
                    match paks.get(*selected).map(|p| {
                        let inst = kui_store::installed_version(p);
                        (inst.is_some(), inst.as_deref() != Some(p.version.as_str()))
                    }) {
                        Some((true, true)) => {
                            &[("Y", "Remove"), ("A", "Update"), ("B", "Back")]
                        }
                        Some((true, false)) => &[("Y", "Remove"), ("B", "Back")],
                        _ => &[("A", "Install"), ("B", "Back")],
                    }
                }
                Screen::PortCats { .. } => &[("A", "Open"), ("B", "Back")],
                Screen::Ports { ports, selected, .. } => {
                    match ports
                        .get(*selected)
                        .map(|p| kui_store::ports::installed(&sd.root, p))
                    {
                        Some(true) => &[("Y", "Remove"), ("B", "Back")],
                        Some(false) => &[("A", "Install"), ("B", "Back")],
                        None => &[("B", "Back")],
                    }
                }
                Screen::Updater { .. } => &[("A", "Install"), ("B", "Back")],
                Screen::ScraperMenu { .. } => &[("A", "Run"), ("B", "Back")],
                Screen::ScraperRun { job, .. } => {
                    if job.progress().finished {
                        &[("B", "Back")]
                    } else {
                        &[("B", "Cancel")]
                    }
                }
                Screen::CoreOpts { .. } => {
                    &[("</>", "Change"), ("X", "Clear"), ("A", "Bind"), ("B", "Back")]
                }
                Screen::Osk { .. } => {
                    &[("START", "OK"), ("X", "Delete"), ("A", "Type"), ("B", "Cancel")]
                }
                Screen::BootLogo { .. } => &[("</>", "Browse"), ("A", "Apply"), ("B", "Back")],
                Screen::Themes { .. } => {
                    &[("</>", "Browse"), ("X", "Reset"), ("A", "Apply"), ("B", "Back")]
                }
            };
            let hfont: u32 = 22;
            let hlh = f.line_height(hfont);
            let icon = 26.0;
            let hy = sh as f32 - 16.0 - hlh;
            let mut x = sw as f32 - 16.0;
            for (btn, label) in hints.iter().rev() {
                let label: &str = if *btn == "Y" && label.is_empty() { wipe_hint } else { label };
                let lw = f.measure(&v.gl, label, hfont);
                x -= lw;
                text_smoked(f, &r, &v.gl, label, x, hy, hfont, theme.c6);
                let icon_tex = match *btn {
                    "MENU" => menu_icon.as_ref(),
                    "SELECT" => select_icon.as_ref(),
                    "START" => start_icon.as_ref(),
                    _ => None,
                };
                if let Some(t) = icon_tex {
                    x -= icon + 10.0;
                    let iy = hy + hlh / 2.0 - icon / 2.0;
                    tex_smoked(&r, &v.gl, t, x, iy, icon, icon, [0.0, 0.0, 1.0, 1.0], theme.c1);
                } else {
                    let bw = f.measure(&v.gl, btn, hfont);
                    x -= bw + 10.0;
                    text_smoked(f, &r, &v.gl, btn, x, hy, hfont, theme.c1);
                }
                x -= 36.0;
            }
        }

        // volume/brightness OSD: a notification pill with a fill bar.
        // kuid owns the apply — re-read its live value every frame so
        // held-key repeats track; fall back to cfg before kuid has written
        if let Some((is_bright, until)) = osd {
            if Instant::now() > until {
                osd = None;
            } else {
                let (live, key, dflt) = if is_bright {
                    ("/tmp/kui/bright", "display.brightness", 90)
                } else {
                    ("/tmp/kui/vol", "audio.volume", 40)
                };
                let val = std::fs::read_to_string(live)
                    .ok()
                    .and_then(|s| s.trim().parse::<i32>().ok())
                    .unwrap_or_else(|| cfg.get_i32(key, dflt))
                    .clamp(0, 100);
                let ow = 360.0;
                let oh = PILL_H as f32;
                let ox = (sw as f32 - ow) / 2.0;
                let oy = sh as f32 - 110.0;
                pill.draw(&r, &v.gl, ox, oy, ow, theme.c2);
                if let Some(sheet) = &assets_tex {
                    let uv = if is_bright { asset_uv(A_BRIGHTNESS) } else { asset_uv(A_VOLUME) };
                    r.draw_uv(&v.gl, sheet, ox + 20.0, oy + (oh - 26.0) / 2.0, 26.0, 26.0, uv, [0.0, 0.0, 0.0, 1.0]);
                }
                let bar_x = ox + 64.0;
                let bar_w = ow - 64.0 - 24.0;
                let bar_y = oy + oh / 2.0 - 5.0;
                r.rect(&v.gl, bar_x, bar_y, bar_w, 10.0, [0.0, 0.0, 0.0, 0.15]);
                r.rect(&v.gl, bar_x, bar_y, bar_w * (val as f32 / 100.0), 10.0, theme.c1);
            }
        }

        // launch-failure toast: the frontend's two-pill notification look
        if let Some((msg, until)) = &toast {
            if Instant::now() > *until {
                toast = None;
            } else if let Some(f) = font.as_mut() {
                let tw = f.measure(&v.gl, msg, 24);
                let inner_w = tw + 40.0;
                let outer_w = inner_w + 16.0;
                let ox = (sw as f32 - outer_w) / 2.0;
                let oy = sh as f32 - 180.0;
                pill.draw(&r, &v.gl, ox, oy, outer_w, theme.c1);
                pill_inner.draw(&r, &v.gl, ox + 8.0, oy + 8.0, inner_w, theme.c2);
                let lh = f.line_height(24);
                f.draw(
                    &r,
                    &v.gl,
                    msg,
                    ox + 8.0 + (inner_w - tw) / 2.0,
                    oy + 8.0 + (36.0 - lh) / 2.0,
                    24,
                    cfg.get_color("theme.color8", 0x000000),
                );
            }
        }

        unsafe { v.gl.flush() };
        v.present();
        if first_frame_at.is_none() {
            let up = std::fs::read_to_string("/proc/uptime")
                .ok()
                .and_then(|s| s.split_whitespace().next().map(str::to_string))
                .unwrap_or_default();
            println!("first frame at {up}s uptime");
            first_frame_at = Some(());
        }
    }
}

fn root_screen(
    mode: UiMode,
    platforms: &[sd::PlatformEntry],
    tiles: &[Tile],
    at: usize,
) -> Screen {
    if mode == UiMode::Carousel {
        Screen::Carousel
    } else {
        open_root_list(platforms, tiles, at)
    }
}

/// Lists/Covers root: the tiles as a plain vertical list, cursor on `at`
/// (coming back from a platform lands on that platform, not the top).
fn open_root_list(platforms: &[sd::PlatformEntry], tiles: &[Tile], at: usize) -> Screen {
    let sel = at.min(tiles.len().saturating_sub(1));
    let visible = visible_rows();
    let scroll = sel
        .saturating_sub(5)
        .min(tiles.len().saturating_sub(visible));
    Screen::List {
        kind: ListKind::Root,
        rows: tiles
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let (_, name) = t.art_key(platforms);
                Row { label: name, action: RowAction::OpenTile(i) }
            })
            .collect(),
        selected: sel,
        scroll,
        show_art: false,
        tag: None,
    }
}

/// Mode-aware tile opener: Lists mode strips boxart from game lists;
/// restores this session's remembered cursor for the tile.
#[allow(clippy::too_many_arguments)]
fn open_tile_mode(
    sd: &Sd,
    loader: &Loader,
    fbg: &mut HashMap<usize, Art>,
    boxart: &mut HashMap<usize, Art>,
    infos: &mut HashMap<usize, Option<String>>,
    platforms: &[sd::PlatformEntry],
    tiles: &[Tile],
    tile: usize,
    mode: UiMode,
    remember: &HashMap<usize, (usize, usize)>,
) -> Option<Screen> {
    let mut sc = open_tile(sd, loader, fbg, boxart, infos, platforms, tiles, tile)?;
    if let Screen::List { show_art, selected, scroll, rows, .. } = &mut sc {
        if mode == UiMode::Lists {
            *show_art = false;
        }
        if let Some((rsel, rscr)) = remember.get(&tile)
            && !rows.is_empty()
        {
            *selected = (*rsel).min(rows.len() - 1);
            *scroll = (*rscr).min(rows.len().saturating_sub(1));
            if *selected < *scroll {
                *scroll = *selected;
            }
        }
    }
    Some(sc)
}

/// Open a carousel tile: platform/recents/collections list or the Dude.
#[allow(clippy::too_many_arguments)]
fn open_tile(
    sd: &Sd,
    loader: &Loader,
    fbg: &mut HashMap<usize, Art>,
    boxart: &mut HashMap<usize, Art>,
    infos: &mut HashMap<usize, Option<String>>,
    platforms: &[sd::PlatformEntry],
    tiles: &[Tile],
    tile: usize,
) -> Option<Screen> {
    boxart.clear();
    infos.clear();
    match &tiles[tile] {
        Tile::Dude => Some(Screen::Dude),
        Tile::Platform(i) => {
            let p = &platforms[*i];
            fbg.entry(tile).or_insert_with(|| match sd.folder_bg(p) {
                Some(path) => {
                    loader.request(art::key(K_FBG, tile), path);
                    Art::Pending
                }
                None => Art::Missing,
            });
            Some(Screen::List {
                kind: ListKind::Platform(*i),
                rows: {
                    let mut rows: Vec<Row> = p
                        .roms
                        .iter()
                        .map(|rom| {
                            let abs = p.dir.join(rom);
                            let pinned = sd.is_pinned(&abs);
                            let name = clean_name(rom);
                            Row {
                                label: if pinned { format!("> {name}") } else { name },
                                action: RowAction::Launch(abs),
                            }
                        })
                        .collect();
                    rows.sort_by_key(|row| {
                        let pinned = row.label.starts_with("> ");
                        (!pinned, row.label.to_lowercase())
                    });
                    if !rows.is_empty() {
                        rows.insert(
                            0,
                            Row {
                                label: "> Random".into(),
                                action: RowAction::LaunchRandom,
                            },
                        );
                    }
                    rows
                },
                selected: 0,
                scroll: 0,
                show_art: true,
                tag: Some(p.tag.clone()),
            })
        }
    }
}

fn open_collection_detail(sd: &Sd, col: &std::path::Path) -> Screen {
    let games = sd.collection_games(col);
    Screen::List {
        kind: ListKind::Collection(col.to_path_buf()),
        rows: games
            .into_iter()
            .map(|g| Row { label: stem_of(&g), action: RowAction::Launch(g) })
            .collect(),
        selected: 0,
        scroll: 0,
        show_art: true,
        tag: None,
    }
}

fn open_pick_platform(platforms: &[sd::PlatformEntry], col: &std::path::Path) -> Screen {
    Screen::List {
        kind: ListKind::PickPlatform(col.to_path_buf()),
        rows: platforms
            .iter()
            .enumerate()
            .map(|(i, p)| Row {
                label: p.display.clone(),
                action: RowAction::PickPlatform(i),
            })
            .collect(),
        selected: 0,
        scroll: 0,
        show_art: false,
        tag: None,
    }
}

/// Built-in "smart" collections: a display name and the accent-folded,
/// lowercase aliases that match a game when they occur in its (folded)
/// filename. Shown only when the library has a match and the user hasn't
/// dismissed it. Aliases stay specific (a distinctive word, or a multi-word
/// phrase) so they don't sweep in unrelated titles.
const SMART_COLLECTIONS: &[(&str, &[&str])] = &[
    ("Mario", &["mario"]),
    ("Pokémon", &["pokemon"]),
    ("Zelda", &["zelda"]),
    ("Mega Man", &["mega man", "megaman", "rockman"]),
    ("Metroid", &["metroid"]),
    ("Kirby", &["kirby"]),
    ("Donkey Kong", &["donkey kong"]),
    ("Yoshi", &["yoshi"]),
    ("Wario", &["wario"]),
    ("Fire Emblem", &["fire emblem"]),
    ("Final Fantasy", &["final fantasy"]),
    ("Dragon Quest", &["dragon quest", "dragon warrior"]),
    ("Castlevania", &["castlevania"]),
    ("Bomberman", &["bomberman"]),
    ("Sonic", &["sonic"]),
    ("Street Fighter", &["street fighter"]),
    ("Mortal Kombat", &["mortal kombat"]),
    ("Contra", &["contra"]),
    ("Metal Slug", &["metal slug"]),
    ("Double Dragon", &["double dragon"]),
    ("TMNT", &["teenage mutant", "ninja turtles", "tmnt"]),
    ("Tetris", &["tetris"]),
    ("Pac-Man", &["pac-man", "pacman", "pac man"]),
    ("Medabots", &["medarot", "medabots"]),
    ("Digimon", &["digimon"]),
    ("Dragon Ball", &["dragon ball", "dragonball"]),
    ("Yu-Gi-Oh!", &["yu-gi-oh", "yugioh", "yu gi oh"]),
    ("Naruto", &["naruto"]),
    ("One Piece", &["one piece"]),
    ("Crash Bandicoot", &["crash bandicoot"]),
    ("Spyro", &["spyro"]),
    ("Rayman", &["rayman"]),
    ("Star Wars", &["star wars"]),
    ("LEGO", &["lego"]),
    ("Spider-Man", &["spider-man", "spiderman", "spider man"]),
    ("Batman", &["batman"]),
    ("Harry Potter", &["harry potter"]),
    ("Advance Wars", &["advance wars"]),
    ("Golden Sun", &["golden sun"]),
    ("Metal Gear", &["metal gear"]),
    ("Star Fox", &["star fox", "starwing"]),
    ("Kid Icarus", &["kid icarus"]),
];

/// Lowercase and strip common Latin accents, so "Pokémon" and "Pokemon"
/// both fold to the same string for matching.
fn fold_name(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| match c {
            'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'ô' | 'ö' | 'õ' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            'ñ' => 'n',
            other => other,
        })
        .collect()
}

/// A collection's art key: accent-folded, lowercased, alphanumerics only —
/// "Pokémon" -> "pokemon", "Yu-Gi-Oh!" -> "yugioh", "Mega Man" -> "megaman".
fn collection_slug(name: &str) -> String {
    fold_name(name).chars().filter(|c| c.is_ascii_alphanumeric()).collect()
}

/// The art key for a collections-index row, if it is a collection.
fn collection_key(action: &RowAction) -> Option<String> {
    match action {
        RowAction::OpenSmartCollection(name) => Some(collection_slug(name)),
        RowAction::OpenCollection(path) => {
            path.file_stem().map(|s| collection_slug(&s.to_string_lossy()))
        }
        _ => None,
    }
}

/// Every ROM in the library as (folded filename, absolute path).
fn smart_library(platforms: &[sd::PlatformEntry]) -> Vec<(String, PathBuf)> {
    platforms
        .iter()
        .flat_map(|p| p.roms.iter().map(move |r| (fold_name(r), p.dir.join(r))))
        .collect()
}

/// ROMs whose folded name contains any of `aliases`.
fn smart_matches(library: &[(String, PathBuf)], aliases: &[&str]) -> Vec<PathBuf> {
    library
        .iter()
        .filter(|(name, _)| aliases.iter().any(|a| name.contains(a)))
        .map(|(_, p)| p.clone())
        .collect()
}

fn open_smart_collection_detail(platforms: &[sd::PlatformEntry], key: &str) -> Screen {
    let aliases = SMART_COLLECTIONS
        .iter()
        .find(|(name, _)| *name == key)
        .map(|(_, a)| *a)
        .unwrap_or(&[]);
    let library = smart_library(platforms);
    let mut games = smart_matches(&library, aliases);
    games.sort_by_key(|p| stem_of(p).to_lowercase());
    Screen::List {
        kind: ListKind::SmartCollection,
        rows: games
            .into_iter()
            .map(|g| Row { label: stem_of(&g), action: RowAction::Launch(g) })
            .collect(),
        selected: 0,
        scroll: 0,
        show_art: true,
        tag: None,
    }
}

fn open_collections_index(
    sd: &Sd,
    boxart: &mut HashMap<usize, Art>,
    infos: &mut HashMap<usize, Option<String>>,
    platforms: &[sd::PlatformEntry],
) -> Screen {
    boxart.clear();
    infos.clear();
    // user collections + built-in franchise collections, gathered together
    // then sorted alphabetically. Built-ins show only when the library has
    // a match and the user hasn't wiped them.
    let mut entries: Vec<(String, Row)> = sd
        .collections()
        .into_iter()
        .map(|(name, path)| {
            let count = sd.collection_games(&path).len();
            (
                name.to_lowercase(),
                Row {
                    label: format!("{name}  ({count})"),
                    action: RowAction::OpenCollection(path),
                },
            )
        })
        .collect();
    let dismissed = sd.smart_dismissed();
    let library = smart_library(platforms);
    for (name, aliases) in SMART_COLLECTIONS {
        if dismissed.contains(*name) {
            continue;
        }
        let count = smart_matches(&library, aliases).len();
        if count == 0 {
            continue;
        }
        entries.push((
            name.to_lowercase(),
            Row {
                label: format!("{name}  ({count})"),
                action: RowAction::OpenSmartCollection(name.to_string()),
            },
        ));
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    let mut rows: Vec<Row> = entries.into_iter().map(|(_, row)| row).collect();
    let paks = installed_paks(sd);
    if !paks.is_empty() {
        rows.push(Row {
            label: format!("Paks  ({})", paks.len()),
            action: RowAction::OpenPaks,
        });
    }
    rows.push(Row { label: "+ New Collection".into(), action: RowAction::NewCollection });
    Screen::List {
        kind: ListKind::CollectionsIndex,
        rows,
        selected: 0,
        scroll: 0,
        show_art: true,
        tag: None,
    }
}

/// Outcome of a launch attempt: Fail carries a user-facing message the
/// caller shows as a toast (failures used to die silently on stderr).
enum LaunchResult {
    Started(i32),
    Fail(String),
    NoOp,
}

/// Time-seeded pick for the Random row; plenty for "surprise me".
fn rand_below(len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    (n % len as u128) as usize
}

/// Write /tmp/next + recents and return the exit code (device); log-only on desktop.
fn launch_rom(sd: &Sd, cfg: &kui_config::Config, rom: &PathBuf, label: &str, on_device: bool) -> LaunchResult {
    let Some(tag) = sd.tag_of_rom(rom) else {
        eprintln!("no platform tag for {rom:?}");
        return LaunchResult::Fail("Can't launch: no platform (TAG) folder".into());
    };
    // native frontend is THE frontend; paks only serve platforms
    // without a libretro core (standalone emulators)
    let native_core = resolve_core(cfg, sd, &tag)
        .map(|stem| cores_dir(sd).join(format!("{stem}_libretro.so")))
        .filter(|p| p.is_file());
    let cmd = if rom.extension().is_some_and(|e| e == "sh") {
        // Ports: launcher scripts run under our bash with the ports
        // control-layer env; scripts find the control folder via
        // $XDG_DATA_HOME/PortMaster
        if !sd.root.join("Data/PortMaster/control.txt").is_file() {
            return LaunchResult::Fail("Ports support not installed".into());
        }
        // Run the port through kui_portrun: it launches the .sh in its own
        // session and watches the pad for MENU+START, SIGKILLing the port
        // group on the chord — a reliable quit even for ports that never
        // start gptokeyb (native-input games) or whose gptokeyb kill
        // misfires. Fall back to a bare bash launch if the runner isn't
        // deployed yet, so ports still launch on older cards.
        let env = "XDG_DATA_HOME=/mnt/SDCARD/Data HOME=/mnt/SDCARD/Data/home";
        let bash = shell_quote("/mnt/SDCARD/Data/PortMaster/bash");
        let port = shell_quote(&rom.display().to_string());
        let runner = sd.root.join("Data/PortMaster/kui_portrun");
        if runner.is_file() {
            format!("{env} {} {bash} {port}", shell_quote(&runner.display().to_string()))
        } else {
            format!("{env} {bash} {port}")
        }
    } else if let Some(core) = native_core {
        format!(
            "{} {} {}",
            shell_quote("/mnt/SDCARD/kui-frontend"),
            shell_quote(&core.display().to_string()),
            shell_quote(&rom.display().to_string())
        )
    } else if let Some(script) = sd.emu_launch(&tag) {
        format!(
            "{} {}",
            shell_quote(&script.display().to_string()),
            shell_quote(&rom.display().to_string())
        )
    } else {
        eprintln!("no core or emu pak for tag {tag}");
        return LaunchResult::Fail(format!("No emulator installed for {tag}"));
    };
    println!("launching: {cmd}");
    if on_device {
        run_hooks(
            "pre-launch.d",
            &[
                ("HOOK_PHASE", "pre".to_string()),
                ("HOOK_TYPE", "rom".to_string()),
                ("HOOK_CMD", cmd.clone()),
                ("HOOK_ROM_PATH", rom.display().to_string()),
                (
                    "HOOK_LAST",
                    std::fs::read_to_string("/tmp/kui_last.txt").unwrap_or_default(),
                ),
            ],
        );
        let _ = std::fs::write("/tmp/kui_last.txt", rom.display().to_string());
        let _ = std::fs::write(
            sd.root.join(".userdata/shared/kui/last.txt"),
            rom.display().to_string(),
        );
        sd.add_recent(rom, label);
        // session start stamp; consumed (with duration) at next launcher boot
        if let Ok(epoch) = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            && let Ok(rel) = rom.strip_prefix(&sd.root)
        {
            let _ = std::fs::write(
                "/tmp/kui_session",
                format!("{}\t/{}\t{}", epoch.as_secs(), rel.display(), label),
            );
        }
        match std::fs::File::create("/tmp/next").and_then(|mut f| f.write_all(cmd.as_bytes())) {
            Ok(()) => LaunchResult::Started(0),
            Err(e) => {
                eprintln!("writing /tmp/next failed: {e}");
                LaunchResult::Fail(format!("Launch failed: {e}"))
            }
        }
    } else {
        LaunchResult::NoOp
    }
}

/// Draw a texture covering the whole screen (center crop, no distortion).
fn draw_cover(
    r: &Renderer,
    gl: &glow::Context,
    t: &kui_gfx::Texture,
    sw: f32,
    sh: f32,
    dim: f32,
) {
    let scale = (sw / t.w as f32).max(sh / t.h as f32);
    let crop_u = (sw / scale) / t.w as f32;
    let crop_v = (sh / scale) / t.h as f32;
    let uv = [(1.0 - crop_u) / 2.0, (1.0 - crop_v) / 2.0, crop_u, crop_v];
    r.draw_uv(gl, t, 0.0, 0.0, sw, sh, uv, WHITE);
    r.rect(gl, 0.0, 0.0, sw, sh, [0.0, 0.0, 0.0, dim]);
}

/// Platform lists have no title pill (the tag pill carries the context),
/// so their rows start at the top and one more row fits.
/// Thin smoky black contour for chrome-scale elements (proportional to
/// the logo halo: rings at 1.2px @ 40% and 2.4px @ 14%).
const SMOKE_RINGS: [(f32, f32); 2] = [(2.4, 0.14), (1.2, 0.40)];

fn smoke_offsets(rad: f32) -> [(f32, f32); 8] {
    let d = rad * 0.7;
    [(rad, 0.0), (-rad, 0.0), (0.0, rad), (0.0, -rad), (d, d), (d, -d), (-d, d), (-d, -d)]
}

#[allow(clippy::too_many_arguments)]
fn tex_smoked(
    r: &Renderer,
    gl: &glow::Context,
    tex: &kui_gfx::Texture,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    uv: [f32; 4],
    color: [f32; 4],
) {
    for (rad, a) in SMOKE_RINGS {
        for (dx, dy) in smoke_offsets(rad) {
            r.draw_uv(gl, tex, x + dx, y + dy, w, h, uv, [0.0, 0.0, 0.0, a]);
        }
    }
    r.draw_uv(gl, tex, x, y, w, h, uv, color);
}

#[allow(clippy::too_many_arguments)]
fn text_smoked(
    f: &mut Font,
    r: &Renderer,
    gl: &glow::Context,
    text: &str,
    x: f32,
    y: f32,
    size: u32,
    color: [f32; 4],
) {
    for (rad, a) in SMOKE_RINGS {
        for (dx, dy) in smoke_offsets(rad) {
            f.draw(r, gl, text, x + dx, y + dy, size, [0.0, 0.0, 0.0, a]);
        }
    }
    f.draw(r, gl, text, x, y, size, color);
}

/// All lists start at the top: 11 rows.
fn now_hint() -> Instant {
    Instant::now()
}

fn visible_rows() -> usize {
    11
}

// Pak Dek rows are 82 px (name + description) vs the standard 64, so
// fewer fit; input clamp and renderer must agree on this count.
fn pakdek_visible_rows() -> usize {
    7
}

fn stem_of(p: &std::path::Path) -> String {
    p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Single-quote a string for `eval`, escaping embedded quotes.
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Display name for a ROM file: extension stripped. PICO-8 carts use
/// the double extension .p8.png, so a leftover .p8 goes too.
fn clean_name(rom: &str) -> String {
    let s = rom.rsplit_once('.').map(|(s, _)| s).unwrap_or(rom);
    let lower = s.to_ascii_lowercase();
    let s = if let Some(pre) = lower.strip_suffix(".p8") { &s[..pre.len()] } else { s };
    s.to_string()
}

fn wrap(base: usize, delta: i32, n: usize) -> usize {
    if n == 0 {
        return 0;
    }
    (base as i32 + delta).rem_euclid(n as i32) as usize
}

/// Hub page position by title, for back-navigation cursor memory.
/// Tester grid, 4 columns x 5 rows; the first four slots are the d-pad
/// (fed from the repeat-state snapshots, not button events).
const INPUT_LABELS: [&str; 20] = [
    "UP", "DOWN", "LEFT", "RIGHT", "A", "B", "X", "Y", "L1", "R1", "L2", "R2",
    "F1", "F2", "SELECT", "START", "MENU", "POWER", "VOL-", "VOL+",
];

fn input_slot(b: Button) -> Option<usize> {
    Some(match b {
        Button::Up => 0,
        Button::Down => 1,
        Button::Left => 2,
        Button::Right => 3,
        Button::A => 4,
        Button::B => 5,
        Button::X => 6,
        Button::Y => 7,
        Button::L1 => 8,
        Button::R1 => 9,
        Button::L2 => 10,
        Button::R2 => 11,
        Button::Fn1 => 12,
        Button::Fn2 => 13,
        Button::Select => 14,
        Button::Start => 15,
        Button::Menu => 16,
        Button::Power => 17,
        Button::VolDown => 18,
        Button::VolUp => 19,
        _ => return None,
    })
}

struct FileRow {
    name: String,
    path: PathBuf,
    is_dir: bool,
    /// Right-column text: entry count for folders, size for files.
    info: String,
    /// Port Forge only: this folder is directly an RPG Maker game. Computed
    /// once at row-build time (dir_rows), so hint/desc/render never touch
    /// the filesystem per frame. Always false for the Files browser.
    is_game: bool,
    /// Port Forge only: this folder is a Port Forge Web package (has a
    /// `portforge.json`). Selecting it installs (moves into place) rather than
    /// forges. Computed once in dir_rows, like `is_game`.
    is_package: bool,
}

fn file_rows(dir: &Path) -> Vec<FileRow> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let path = e.path();
            let is_dir = path.is_dir();
            let info = if is_dir {
                match std::fs::read_dir(&path) {
                    Ok(r2) => {
                        let n = r2.count();
                        if n == 1 { "1 item".to_string() } else { format!("{n} items") }
                    }
                    Err(_) => String::new(),
                }
            } else {
                e.metadata().map(|m| fmt_size(m.len())).unwrap_or_default()
            };
            out.push(FileRow { name, path, is_dir, info, is_game: false, is_package: false });
        }
    }
    out.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    out
}

fn fmt_size(b: u64) -> String {
    if b >= 1 << 30 {
        format!("{:.1} GB", b as f64 / (1u64 << 30) as f64)
    } else if b >= 1 << 20 {
        format!("{:.1} MB", b as f64 / (1u64 << 20) as f64)
    } else if b >= 1024 {
        format!("{:.0} KB", b as f64 / 1024.0)
    } else {
        format!("{b} B")
    }
}

fn file_actions(clip: &Option<(PathBuf, bool)>) -> Vec<&'static str> {
    let mut v = vec!["Copy", "Cut"];
    if clip.is_some() {
        v.push("Paste");
    }
    v.extend(["Rename", "New Folder", "Delete", "Quit"]);
    v
}

fn copy_recursive(src: &Path, dest: &Path) -> std::io::Result<()> {
    if src.is_dir() {
        std::fs::create_dir_all(dest)?;
        for e in std::fs::read_dir(src)?.flatten() {
            copy_recursive(&e.path(), &dest.join(e.file_name()))?;
        }
    } else {
        std::fs::copy(src, dest)?;
    }
    Ok(())
}

fn file_paste(src: &Path, dir: &Path, cut: bool) -> std::io::Result<()> {
    let name = src
        .file_name()
        .ok_or_else(|| std::io::Error::other("no file name"))?;
    let mut dest = dir.join(name);
    if cut && dest == src {
        return Ok(());
    }
    let mut n = 1u32;
    while dest.exists() {
        n += 1;
        dest = dir.join(format!("{} ({n})", name.to_string_lossy()));
    }
    if cut {
        if std::fs::rename(src, &dest).is_err() {
            copy_recursive(src, &dest)?;
            if src.is_dir() {
                std::fs::remove_dir_all(src)?;
            } else {
                std::fs::remove_file(src)?;
            }
        }
    } else {
        copy_recursive(src, &dest)?;
    }
    Ok(())
}

struct FilePane {
    dir: PathBuf,
    rows: Vec<FileRow>,
    selected: usize,
    scroll: usize,
}

fn file_pane(dir: PathBuf) -> FilePane {
    FilePane { rows: file_rows(&dir), dir, selected: 0, scroll: 0 }
}

/// Directory-only rows. Port Forge is a folder picker now (on-device zip
/// extraction is slow, so only pre-extracted game folders are accepted) —
/// hiding files keeps the browse simple.
fn dir_rows(dir: &Path) -> Vec<FileRow> {
    let mut r = file_rows(dir);
    r.retain(|f| f.is_dir);
    for f in &mut r {
        // a package wins over game detection (it has no root Game.ini anyway)
        f.is_package = portforge::is_port_package(&f.path);
        f.is_game = !f.is_package && portforge::is_rmxp_game(&f.path);
    }
    r
}

fn dir_pane(dir: PathBuf) -> FilePane {
    FilePane { rows: dir_rows(&dir), dir, selected: 0, scroll: 0 }
}

fn files_reopen(dirs: &[PathBuf; 2], active: usize) -> Screen {
    Screen::Files {
        panes: [file_pane(dirs[0].clone()), file_pane(dirs[1].clone())],
        active,
        menu: None,
        armed_delete: false,
    }
}

/// Queue every carousel panel + logo for decode, radiating out from the
/// landing tile so the visible window dresses first.
fn request_carousel_art(
    loader: &Loader,
    bg: &mut HashMap<usize, Art>,
    logo: &mut HashMap<usize, Art>,
    sd: &Sd,
    tiles: &[Tile],
    platforms: &[sd::PlatformEntry],
    land_tile: usize,
) {
    let n_t = tiles.len();
    let mut order: Vec<usize> = (0..n_t).collect();
    order.sort_by_key(|&t| {
        let d = (t as i32 - land_tile as i32).abs();
        d.min(n_t as i32 - d)
    });
    for t in order {
        let tl = &tiles[t];
        let (bg_p, logo_p) = match tl {
            Tile::Platform(i) => (sd.carousel_bg(&platforms[*i]), sd.carousel_logo(&platforms[*i])),
            _ => {
                let (key, _) = tl.art_key(platforms);
                (sd.carousel_bg_key(&key), sd.carousel_logo_key(&key))
            }
        };
        bg.insert(t, match bg_p {
            Some(p) => {
                loader.request(art::key(K_BG, t), p);
                Art::Pending
            }
            None => Art::Missing,
        });
        logo.insert(t, match logo_p {
            Some(p) => {
                loader.request(art::key(K_LOGO, t), p);
                Art::Pending
            }
            None => Art::Missing,
        });
    }
}

/// Ports category index: Installed, All Ports, then genres by size.
/// Category index for the ports browser. `rtr` scopes it to ready-to-play
/// ports: the top level lists Installed / All Ports / Ready to Play then
/// every genre; the Ready-to-Play sub-level lists the same genres over
/// the ready-to-play ports only.
fn port_categories(
    cat: &kui_store::ports::Catalog,
    sd_root: &Path,
    rtr: bool,
) -> Vec<(String, usize)> {
    let pool: Vec<&kui_store::ports::PortEntry> =
        cat.ports.iter().filter(|p| !rtr || p.rtr).collect();
    let mut out = Vec::new();
    if rtr {
        if !pool.is_empty() {
            out.push(("All Ready to Play".to_string(), pool.len()));
        }
    } else {
        let inst = pool
            .iter()
            .filter(|p| kui_store::ports::installed(sd_root, p))
            .count();
        if inst > 0 {
            out.push(("Installed".to_string(), inst));
        }
        if !pool.is_empty() {
            out.push(("All Ports".to_string(), pool.len()));
            let n = pool.iter().filter(|p| p.rtr).count();
            if n > 0 {
                out.push(("Ready to Play".to_string(), n));
            }
        }
    }
    let mut names: Vec<String> =
        pool.iter().flat_map(|p| p.genres.clone()).collect();
    names.sort();
    names.dedup();
    let mut genres: Vec<(String, usize)> = names
        .into_iter()
        .map(|g| {
            let n = pool.iter().filter(|p| p.genres.contains(&g)).count();
            (genre_label(&g), n)
        })
        .collect();
    genres.sort_by(|a, b| a.0.cmp(&b.0));
    out.extend(genres);
    out
}

fn genre_label(g: &str) -> String {
    match g {
        "fps" => "FPS".to_string(),
        "rpg" => "RPG".to_string(),
        _ => g
            .split(' ')
            .map(|w| {
                let mut c = w.chars();
                match c.next() {
                    Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                    None => String::new(),
                }
            })
            .collect::<Vec<_>>()
            .join(" "),
    }
}

/// Drop platforms nothing can launch: no native core, no emu pak, and
/// (for .sh ports platforms) no ports control layer on the card.
fn retain_launchable(
    platforms: &mut Vec<sd::PlatformEntry>,
    sd: &Sd,
    cfg: &kui_config::Config,
) {
    let ports_ready = sd.root.join("Data/PortMaster/control.txt").is_file();
    platforms.retain(|p| {
        let ok = sd.emu_launch(&p.tag).is_some()
            || resolve_core(cfg, sd, &p.tag)
                .map(|stem| cores_dir(sd).join(format!("{stem}_libretro.so")).is_file())
                .unwrap_or(false)
            || (ports_ready && p.roms.iter().any(|r| r.ends_with(".sh")));
        if !ok {
            println!("hidden (no emu pak): {} ({})", p.display, p.tag);
        }
        ok
    });
}

fn hub_pos(pages: &[hub::Page], title: &str) -> usize {
    pages.iter().position(|p| p.title == title).unwrap_or(0)
}

/// Built-in tag -> libretro core map: the classic tg5040 set. Override
/// per tag with `fe.<TAG>.core`; a pak's EMU_EXE line is the last resort
/// for platforms not listed here.
const CORE_TABLE: &[(&str, &str)] = &[
    ("32X", "picodrive"),
    ("3DO", "opera"),
    ("A2600", "stella2014"),
    ("A7800", "prosystem"),
    ("A800", "atari800"),
    ("ARDUBOY", "arduous"),
    ("C128", "vice_x128"),
    ("C64", "vice_x64"),
    ("C64GS", "vice_x64"),
    ("CD32", "puae2021"),
    ("CHANF", "freechaf"),
    ("COLECO", "gearcoleco"),
    ("CPC", "cap32"),
    ("DOS", "dosbox_pure"),
    ("F8", "fake08"),
    ("FBN", "fbneo"),
    ("FC", "fceumm"),
    ("FDS", "fceumm"),
    ("GB", "gambatte"),
    ("GBA", "gpsp"),
    ("GBC", "gambatte"),
    ("GBH", "mgba"),
    ("GG", "picodrive"),
    ("GX4000", "cap32"),
    ("INTV", "freeintv"),
    ("JAGUAR", "virtualjaguar"),
    ("JAGUARCD", "virtualjaguar"),
    ("LYNX", "handy"),
    ("MD", "picodrive"),
    ("MEGADUCK", "sameduck"),
    ("MGBA", "mgba"),
    ("MSX", "bluemsx"),
    ("NEOCD", "neocd"),
    ("NGP", "race"),
    ("NGPC", "race"),
    ("O2", "o2em"),
    ("P8", "fake08"),
    ("PCE", "mednafen_supergrafx"),
    ("PCECD", "mednafen_supergrafx"),
    ("PET", "vice_xpet"),
    ("PKM", "pokemini"),
    ("PLUS4", "vice_xplus4"),
    ("PRBOOM", "prboom"),
    ("PS", "pcsx_rearmed"),
    ("PUAE", "puae2021"),
    ("SEGACD", "picodrive"),
    ("SFC", "snes9x"),
    ("SG1000", "picodrive"),
    ("SGB", "mgba"),
    ("SGFX", "mednafen_supergrafx"),
    ("SMS", "picodrive"),
    ("SUPA", "mednafen_supafaust"),
    ("SV", "potator"),
    ("UZEBOX", "uzem"),
    ("VB", "mednafen_vb"),
    ("VECTREX", "vecx"),
    ("VIC", "vice_xvic"),
    ("WS", "mednafen_wswan"),
    ("WSC", "mednafen_wswan"),
    ("ZXS", "fuse"),
];

fn cores_dir(sd: &sd::Sd) -> PathBuf {
    std::env::var("CORES_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| sd.root.join(".system/tg5040/cores"))
}

fn resolve_core(cfg: &kui_config::Config, sd: &sd::Sd, tag: &str) -> Option<String> {
    if let Some(v) = cfg.get(&format!("fe.{tag}.core")) {
        return Some(v.to_string());
    }
    if let Some((_, c)) = CORE_TABLE.iter().find(|(t, _)| *t == tag) {
        return Some((*c).to_string());
    }
    let script = sd.emu_launch(tag)?;
    std::fs::read_to_string(script).ok()?.lines().find_map(|l| {
        l.trim()
            .strip_prefix("EMU_EXE=")
            .map(|v| v.trim().trim_matches('"').to_string())
    })
}

/// Art for the switcher: the selected slot's state preview, else boxart.
fn switcher_art_path(
    cfg: &kui_config::Config,
    sd: &Sd,
    rom: &std::path::Path,
) -> Option<PathBuf> {
    let tag = rom
        .parent()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .and_then(|n| {
            let open = n.rfind('(')?;
            let close = n.rfind(')')?;
            (close > open).then(|| n[open + 1..close].to_string())
        });
    if let Some(tag) = tag
        && let Some(core) = resolve_core(cfg, sd, &tag)
    {
        let stem =
            rom.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let dir = sd.root.join(".userdata/shared").join(format!("{tag}-{core}"));
        // the last thing on screen that session beats everything
        let session = dir.join(format!("{stem}.session.png"));
        if session.is_file() {
            return Some(session);
        }
        let slot = std::fs::read_to_string(dir.join(format!("{stem}.slot")))
            .ok()
            .and_then(|s| s.trim().parse::<usize>().ok())
            .unwrap_or(0);
        let preview = dir.join(format!("{stem}.state{slot}.png"));
        if preview.is_file() {
            return Some(preview);
        }
    }
    sd.boxart_for(rom)
}

/// Greedy word wrap by measured width.
fn wrap_text(
    f: &mut kui_gfx::text::Font,
    gl: &glow::Context,
    text: &str,
    size: u32,
    maxw: f32,
) -> Vec<String> {
    let mut out = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let cand = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
        if f.measure(gl, &cand, size) <= maxw || line.is_empty() {
            line = cand;
        } else {
            out.push(std::mem::take(&mut line));
            line = word.to_string();
        }
    }
    if !line.is_empty() {
        out.push(line);
    }
    out
}

/// `wpa_cli -p /etc/wifi/sockets -i wlan0 <args>`, captured stdout (empty on failure).
fn wifi_cli(args: &str) -> String {
    std::process::Command::new("sh")
        .args(["-c", &format!("wpa_cli -p /etc/wifi/sockets -i wlan0 {args} 2>/dev/null")])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

/// Current connection as (ssid, ip), if associated.
fn wifi_status() -> Option<(String, String)> {
    let out = wifi_cli("status");
    let mut ssid = None;
    let mut ip = None;
    for line in out.lines() {
        if let Some(v) = line.strip_prefix("ssid=") {
            ssid = Some(v.to_string());
        }
        if let Some(v) = line.strip_prefix("ip_address=") {
            ip = Some(v.to_string());
        }
        if line.starts_with("wpa_state=") && !line.contains("COMPLETED") {
            return None;
        }
    }
    Some((ssid?, ip.unwrap_or_else(|| "no ip".into())))
}

/// Scan results merged with the saved-network list, strongest first.
fn wifi_scan_collect() -> Vec<WifiNet> {
    let saved: Vec<(i32, String, bool)> = wifi_cli("list_networks")
        .lines()
        .skip(1)
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            Some((f.first()?.parse().ok()?, (*f.get(1)?).to_string(), l.contains("[CURRENT]")))
        })
        .collect();
    let mut nets: Vec<WifiNet> = Vec::new();
    for l in wifi_cli("scan_results").lines().skip(1) {
        let f: Vec<&str> = l.split('\t').collect();
        let (Some(sig), Some(flags), Some(ssid)) = (f.get(2), f.get(3), f.get(4)) else {
            continue;
        };
        if ssid.is_empty() {
            continue;
        }
        let signal: i32 = sig.parse().unwrap_or(-100);
        if let Some(existing) = nets.iter_mut().find(|n| n.ssid == *ssid) {
            existing.signal = existing.signal.max(signal);
            continue;
        }
        let known = saved.iter().find(|(_, s, _)| s == ssid);
        nets.push(WifiNet {
            ssid: (*ssid).to_string(),
            signal,
            secured: flags.contains("WPA") || flags.contains("WEP"),
            saved: known.map(|(id, _, _)| *id),
            current: known.is_some_and(|(_, _, c)| *c),
        });
    }
    nets.sort_by(|a, b| b.current.cmp(&a.current).then(b.signal.cmp(&a.signal)));
    nets
}

/// Join a network in the background; env vars dodge shell quoting.
fn wifi_connect_spawn(ssid: &str, psk: Option<&str>) {
    let script = if psk.is_some() {
        r#"ID=$(wpa_cli -p /etc/wifi/sockets -i wlan0 add_network | tail -1)
wpa_cli -p /etc/wifi/sockets -i wlan0 set_network "$ID" ssid ""$S""
wpa_cli -p /etc/wifi/sockets -i wlan0 set_network "$ID" psk ""$P""
wpa_cli -p /etc/wifi/sockets -i wlan0 enable_network "$ID"
wpa_cli -p /etc/wifi/sockets -i wlan0 select_network "$ID"
wpa_cli -p /etc/wifi/sockets -i wlan0 save_config"#
    } else {
        r#"ID=$(wpa_cli -p /etc/wifi/sockets -i wlan0 add_network | tail -1)
wpa_cli -p /etc/wifi/sockets -i wlan0 set_network "$ID" ssid ""$S""
wpa_cli -p /etc/wifi/sockets -i wlan0 set_network "$ID" key_mgmt NONE
wpa_cli -p /etc/wifi/sockets -i wlan0 enable_network "$ID"
wpa_cli -p /etc/wifi/sockets -i wlan0 select_network "$ID"
wpa_cli -p /etc/wifi/sockets -i wlan0 save_config"#
    };
    let _ = std::process::Command::new("sh")
        .args(["-c", &format!("({script}) >/dev/null 2>&1 &")])
        .env("S", ssid)
        .env("P", psk.unwrap_or(""))
        .spawn();
}

/// Known and freshly-scanned bluetooth devices; nameless ones skipped.
fn bt_collect() -> Vec<BtDev> {
    let run = |args: &str| {
        std::process::Command::new("sh")
            .args(["-c", &format!("bluetoothctl {args} 2>/dev/null")])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default()
    };
    let paired: Vec<String> = run("paired-devices")
        .lines()
        .filter_map(|l| l.split_whitespace().nth(1).map(str::to_string))
        .collect();
    let mut devs: Vec<BtDev> = Vec::new();
    for l in run("devices").lines() {
        let mut it = l.split_whitespace();
        let (Some(_), Some(mac)) = (it.next(), it.next()) else {
            continue;
        };
        let name = it.collect::<Vec<_>>().join(" ");
        // a MAC echoed as a name means the device never told us its name
        if name.is_empty() || name.replace('-', ":") == mac {
            continue;
        }
        let is_paired = paired.iter().any(|p| p == mac);
        let connected = is_paired
            && run(&format!("info {mac}")).lines().any(|l| l.trim() == "Connected: yes");
        devs.push(BtDev { mac: mac.to_string(), name, paired: is_paired, connected });
    }
    devs.sort_by(|a, b| {
        b.connected.cmp(&a.connected).then(b.paired.cmp(&a.paired)).then(a.name.cmp(&b.name))
    });
    devs
}

/// Per-game playlog aggregation, most-played first, plus the totals line.
fn gametime_rows(sd: &Sd) -> (Vec<(String, String)>, String) {
    let fmt = |secs: u64| {
        if secs >= 3600 {
            format!("{}h{:02}m", secs / 3600, (secs % 3600) / 60)
        } else {
            format!("{}m", (secs / 60).max(1))
        }
    };
    let mut games: HashMap<String, (u64, u64)> = HashMap::new(); // alias -> (secs, plays)
    let mut total_secs = 0u64;
    let mut total_plays = 0u64;
    if let Ok(text) = std::fs::read_to_string(sd.root.join(".userdata/shared/kui/playlog.txt"))
    {
        for l in text.lines() {
            let f: Vec<&str> = l.split('\t').collect();
            let (Some(secs), Some(alias)) = (f.get(1), f.get(3)) else {
                continue;
            };
            let secs: u64 = secs.parse().unwrap_or(0);
            let e = games.entry((*alias).to_string()).or_default();
            e.0 += secs;
            e.1 += 1;
            total_secs += secs;
            total_plays += 1;
        }
    }
    let n_games = games.len();
    let mut rows: Vec<(String, (u64, u64))> = games.into_iter().collect();
    rows.sort_by_key(|r| std::cmp::Reverse(r.1.0));
    let rows = rows
        .into_iter()
        .map(|(name, (secs, plays))| {
            let avg = fmt(secs / plays.max(1));
            (name, format!("{} · {} plays · avg {}", fmt(secs), plays, avg))
        })
        .collect();
    let header = format!(
        "Total: {} · {} sessions · {} games",
        fmt(total_secs),
        total_plays,
        n_games
    );
    (rows, header)
}

/// Battery samples (epoch, pct, charging) from kuid's minute log.
fn battlog_read(sd: &Sd) -> Vec<(u64, i32, bool)> {
    std::fs::read_to_string(sd.root.join(".userdata/shared/kui/battlog.txt"))
        .map(|text| {
            text.lines()
                .filter_map(|l| {
                    let f: Vec<&str> = l.split('\t').collect();
                    Some((
                        f.first()?.parse().ok()?,
                        f.get(1)?.parse().ok()?,
                        *f.get(2)? == "1",
                    ))
                })
                .collect()
        })
        .unwrap_or_default()
}

const SCRAPER_ACTIONS: [&str; 6] = [
    "Download Missing",
    "Download Images Only",
    "Download Metadata Only",
    "Download All (refresh)",
    "Patch Images (optimize size)",
    "Delete Artwork & Metadata",
];

/// Minimal percent-encoding (RFC 3986 unreserved kept).
fn urlenc(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Pak contract: run every .sh in the hook dir — alphabetical, each in
/// its own quiet subshell, failures ignored, launches never cancelled.
fn run_hooks(dir: &str, envs: &[(&str, String)]) {
    let base = format!("/mnt/SDCARD/.userdata/tg5040/.hooks/{dir}");
    let Ok(rd) = std::fs::read_dir(&base) else {
        return;
    };
    let mut scripts: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().map(|e| e == "sh").unwrap_or(false))
        .collect();
    scripts.sort();
    for sc in scripts {
        let mut cmd = std::process::Command::new("sh");
        cmd.args(["-c", &format!("( . {} ) >/dev/null 2>&1", shell_quote(&sc.display().to_string()))]);
        for (k, v) in envs {
            cmd.env(k, v);
        }
        let _ = cmd.status();
    }
}

/// Resolve a theme.font value to a font path: legacy "0"/"1" map to the
/// built-ins, anything else is a file stem under .system/res.
fn resolve_font(res: &std::path::Path, value: &str) -> std::path::PathBuf {
    match value {
        "1" => res.join("font1.ttf"),
        "" | "0" => res.join("font2.ttf"),
        stem => {
            let ttf = res.join(format!("{stem}.ttf"));
            if ttf.is_file() { ttf } else { res.join(format!("{stem}.otf")) }
        }
    }
}

/// Selectable fonts: the two built-ins only — OG (BPreplay) and Next
/// (Rounded M+). Dropped-in ttf/otf are intentionally not offered.
fn font_options(_sd: &Sd) -> Vec<(String, String)> {
    vec![("0".to_string(), "OG".to_string()), ("1".to_string(), "Next".to_string())]
}

/// (config key, label, built-in default) — mirrors the frontend table.
const LSC: [(&str, &str, &str); 11] = [
    ("save", "Save State", "menu+r1"),
    ("load", "Load State", "menu+l1"),
    ("reset", "Reset", "menu+b"),
    ("savequit", "Save & Quit", "menu+start"),
    ("switcher", "Game Switcher", "menu+select"),
    ("toggleff", "Toggle FF", "f2"),
    ("holdff", "Hold FF", "none"),
    ("cyclescale", "Cycle Scaling", "none"),
    ("cycleeffect", "Cycle Effect", "none"),
    ("screenshot", "Screenshot", "menu+x"),
    ("turboall", "Turbo All", "f1"),
];

fn lbutton_name(b: Button) -> &'static str {
    match b {
        Button::A => "a",
        Button::B => "b",
        Button::X => "x",
        Button::Y => "y",
        Button::L1 => "l1",
        Button::R1 => "r1",
        Button::L2 => "l2",
        Button::R2 => "r2",
        Button::Fn1 => "f1",
        Button::Fn2 => "f2",
        Button::Select => "select",
        Button::Start => "start",
        _ => "none",
    }
}

fn lshortcut_disp(v: &str) -> String {
    match v {
        "none" | "" => "None".into(),
        _ => v.to_uppercase(),
    }
}

/// Installed pak tools: any *.pak folder with a launch.sh under the
/// user pak roots. (name, launch.sh path), sorted.
fn installed_paks(sd: &Sd) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    for root in [sd.root.join("Tools/tg5040"), sd.root.join("Emus/tg5040")] {
        let Ok(rd) = std::fs::read_dir(&root) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            let is_pak = p.is_dir()
                && p.extension().map(|x| x == "pak").unwrap_or(false);
            if !is_pak {
                continue;
            }
            let script = p.join("launch.sh");
            if !script.is_file() {
                continue;
            }
            let name = p
                .file_stem()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            out.push((name, script));
        }
    }
    out.sort_by_key(|a| a.0.to_lowercase());
    out
}

/// Rolling offset for overflowing single-line text: pause, roll, pause.
/// Keyed per display slot; the clock resets whenever the text changes.
fn roll_offset(
    state: &mut HashMap<String, (String, Instant)>,
    slot: &str,
    text: &str,
    overflow: f32,
    now: Instant,
) -> f32 {
    let e = state
        .entry(slot.to_string())
        .or_insert_with(|| (text.to_string(), now));
    if e.0 != text {
        *e = (text.to_string(), now);
    }
    let (speed, pause) = (60.0, 1.0);
    let cycle = pause + overflow / speed + pause;
    let t = (now - e.1).as_secs_f32() % cycle;
    ((t - pause).max(0.0) * speed).min(overflow)
}

/// Draw a single-line text that rolls when it overflows `max_w`.
#[allow(clippy::too_many_arguments)]
fn draw_roll(
    f: &mut Font,
    r: &kui_gfx::Renderer,
    gl: &glow::Context,
    state: &mut HashMap<String, (String, Instant)>,
    slot: &str,
    text: &str,
    x: f32,
    y_text: f32,
    y_clip: f32,
    h_clip: f32,
    size: u32,
    color: [f32; 4],
    max_w: f32,
    now: Instant,
) {
    let full = f.measure(gl, text, size);
    if full > max_w {
        let off = roll_offset(state, slot, text, full - max_w, now);
        r.scissor(gl, x, y_clip, max_w, h_clip);
        f.draw(r, gl, text, x - off, y_text, size, color);
        r.scissor_off(gl);
    } else {
        f.draw(r, gl, text, x, y_text, size, color);
    }
}

/// Category rows for the pak store: Installed first when non-empty,
/// then every category with (count, update-count).
fn pak_categories(all: &[kui_store::Pak]) -> Vec<(String, usize, usize)> {
    let mut out: Vec<(String, usize, usize)> = Vec::new();
    let installed: Vec<&kui_store::Pak> =
        all.iter().filter(|p| kui_store::installed_version(p).is_some()).collect();
    if !installed.is_empty() {
        let upd = installed
            .iter()
            .filter(|p| kui_store::installed_version(p).as_deref() != Some(p.version.as_str()))
            .count();
        out.push(("Installed".into(), installed.len(), upd));
    }
    let mut names: Vec<String> = all.iter().flat_map(|p| p.categories.clone()).collect();
    names.sort();
    names.dedup();
    for c in names {
        let members: Vec<&kui_store::Pak> =
            all.iter().filter(|p| p.categories.contains(&c)).collect();
        let upd = members
            .iter()
            .filter(|p| {
                kui_store::installed_version(p)
                    .map(|v| v != p.version)
                    .unwrap_or(false)
            })
            .count();
        out.push((c, members.len(), upd));
    }
    out
}

/// The Control Panel's grouped index. Headers are render-only.
enum HubRow {
    Header(&'static str),
    Page(usize),
}

const HUB_GROUPS: [(&str, &[&str]); 4] = [
    ("LOOK", &["Appearance", "Themes", "Boot Logo", "LED Control", "Scraper"]),
    ("PLAY", &["In-Game", "Core Options", "FN Switch", "Game Tracker", "Ports", "Port Forge"]),
    (
        "DEVICE",
        &["Display", "Connectivity", "Input", "Files", "Date & Time", "Battery", "System"],
    ),
    ("KUI", &["PakDek", "Updater", "Developer", "About"]),
];

fn build_hub_rows(pages: &[hub::Page]) -> Vec<HubRow> {
    let mut rows = Vec::new();
    let mut used = vec![false; pages.len()];
    for (name, members) in HUB_GROUPS {
        rows.push(HubRow::Header(name));
        for m in members {
            if let Some(i) = pages.iter().position(|p| p.title == *m) {
                rows.push(HubRow::Page(i));
                used[i] = true;
            }
        }
    }
    // anything unlisted still shows (safety for future pages)
    for (i, u) in used.iter().enumerate() {
        if !u {
            rows.push(HubRow::Page(i));
        }
    }
    rows
}
