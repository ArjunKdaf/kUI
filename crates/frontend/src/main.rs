//! The kUI frontend: a libretro host process, argv-compatible:
//! `<self> <core.so> <rom>`.
//!
//! Feature set: aspect/native/fullscreen scaling with per-platform bezel
//! overlays, in-game menu (Continue / Save / Load / Reset / Quit) with
//! 8 save-state slots and previews, DRC-adjusted audio, SRAM handling,
//! deep sleep on the power button.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use kui_gfx::{Renderer, Texture, WHITE};
use kui_hal::sdl::SdlVideo;
use kui_hal::tg5040;
use kui_hal::{Button, ButtonState, InputEvent, Repeat};
use kui_libretro as lr;

mod cheatdl;
mod hardcore;

const OUT_RATE: i32 = 48000;
const SLOTS: usize = 8;
const PILL_H: u32 = 52;
const ROW_H: f32 = 64.0;
const MENU_FONT: u32 = 30;

/// Sprite rects on the shared asset sheet (1x coords; sheet is @2x, 256px)
/// — same rects the launcher uses, so the OSD icons match exactly.
const A_BATTERY: (f32, f32, f32, f32) = (47.0, 51.0, 17.0, 10.0);
const A_BATTERY_FILL: (f32, f32, f32, f32) = (81.0, 33.0, 12.0, 6.0);
const A_BATTERY_BOLT: (f32, f32, f32, f32) = (91.0, 51.0, 16.0, 10.0);
const A_WIFI: (f32, f32, f32, f32) = (1.0, 104.0, 12.0, 12.0);
const A_BLUETOOTH: (f32, f32, f32, f32) = (53.0, 104.0, 12.0, 12.0);
const A_BRIGHTNESS: (f32, f32, f32, f32) = (1.0, 85.0, 19.0, 19.0);
const A_VOLUME: (f32, f32, f32, f32) = (21.0, 85.0, 19.0, 19.0);
const SHEET: f32 = 256.0;

fn asset_uv(rect: (f32, f32, f32, f32)) -> [f32; 4] {
    [
        rect.0 * 2.0 / SHEET,
        rect.1 * 2.0 / SHEET,
        rect.2 * 2.0 / SHEET,
        rect.3 * 2.0 / SHEET,
    ]
}

struct Resampler {
    base: f64,
    ratio: f64,
    phase: f64,
    last: (i16, i16),
}

impl Resampler {
    fn new(in_rate: f64) -> Self {
        let base = in_rate / OUT_RATE as f64;
        Self { base, ratio: base, phase: 0.0, last: (0, 0) }
    }

    /// Dynamic rate control: nudge the ratio by queue deviation (±0.5% max).
    fn tune(&mut self, queued_frames: f64, target_frames: f64) {
        let dev = ((queued_frames - target_frames) / target_frames).clamp(-1.0, 1.0);
        self.ratio = self.base * (1.0 + 0.005 * dev);
    }

    fn push(&mut self, input: &[i16], out: &mut Vec<i16>) {
        let frames = input.len() / 2;
        if frames == 0 {
            return;
        }
        let mut pos = self.phase;
        while (pos as usize) < frames {
            let i = pos as usize;
            let frac = pos - i as f64;
            let (l0, r0) =
                if i == 0 { self.last } else { (input[(i - 1) * 2], input[(i - 1) * 2 + 1]) };
            let (l1, r1) = (input[i * 2], input[i * 2 + 1]);
            out.push((l0 as f64 + (l1 - l0) as f64 * frac) as i16);
            out.push((r0 as f64 + (r1 - r0) as f64 * frac) as i16);
            pos += self.ratio;
        }
        self.phase = pos - frames as f64;
        self.last = (input[(frames - 1) * 2], input[(frames - 1) * 2 + 1]);
    }
}

fn tag_of(rom: &Path) -> String {
    rom.parent()
        .and_then(|d| d.file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .and_then(|name| {
            let open = name.rfind('(')?;
            let close = name.rfind(')')?;
            (close > open).then(|| name[open + 1..close].trim().to_uppercase())
        })
        .unwrap_or_default()
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Scaling {
    Native,
    Aspect,
    AspectScreen,
    Fullscreen,
    Cropped,
}

enum FeScreen {
    Game,
    Menu { sel: usize },
    Options { sel: usize, scroll: usize },
    Controls { sel: usize },
    OptionsMenu { sel: usize },
    /// (unlocked, "title (+pts)", description)
    Achievements { rows: Vec<(bool, String, String)>, scroll: usize },
    Cheats { sel: usize, scroll: usize },
    Shortcuts { sel: usize },
}

#[derive(Clone, Copy, PartialEq)]
enum Binding {
    Default,
    Btn(u32),
    Turbo(u32),
    None,
}

/// The remappable physical buttons, config-key order.
const PHYS: [(&str, &str); 8] = [
    ("a", "A"),
    ("b", "B"),
    ("x", "X"),
    ("y", "Y"),
    ("l1", "L1"),
    ("r1", "R1"),
    ("l2", "L2"),
    ("r2", "R2"),
];
/// Cycle order for a binding row.
const BIND_CHOICES: [(&str, &str); 14] = [
    ("", "Default"),
    ("a", "A"),
    ("b", "B"),
    ("x", "X"),
    ("y", "Y"),
    ("l", "L"),
    ("r", "R"),
    ("l2", "L2"),
    ("r2", "R2"),
    ("ta", "Turbo A"),
    ("tb", "Turbo B"),
    ("tx", "Turbo X"),
    ("ty", "Turbo Y"),
    ("none", "None"),
];

fn parse_binding(v: &str) -> Binding {
    match v {
        "a" => Binding::Btn(lr::JOYPAD_A),
        "b" => Binding::Btn(lr::JOYPAD_B),
        "x" => Binding::Btn(lr::JOYPAD_X),
        "y" => Binding::Btn(lr::JOYPAD_Y),
        "l" => Binding::Btn(lr::JOYPAD_L),
        "r" => Binding::Btn(lr::JOYPAD_R),
        "l2" => Binding::Btn(lr::JOYPAD_L2),
        "r2" => Binding::Btn(lr::JOYPAD_R2),
        "ta" => Binding::Turbo(lr::JOYPAD_A),
        "tb" => Binding::Turbo(lr::JOYPAD_B),
        "tx" => Binding::Turbo(lr::JOYPAD_X),
        "ty" => Binding::Turbo(lr::JOYPAD_Y),
        "none" => Binding::None,
        _ => Binding::Default,
    }
}

const MENU_FULL: [&str; 6] = ["Continue", "Save", "Load", "Options", "Reset", "Quit"];

/// (config key, label, default binding) — the old-kUI defaults.
const SHORTCUTS: [(&str, &str, &str); 11] = [
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

fn button_name(b: Button) -> &'static str {
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

fn parse_shortcut(v: &str) -> Option<(Button, bool)> {
    let (menu, name) = match v.strip_prefix("menu+") {
        Some(rest) => (true, rest),
        None => (false, v),
    };
    let b = match name {
        "a" => Button::A,
        "b" => Button::B,
        "x" => Button::X,
        "y" => Button::Y,
        "l1" => Button::L1,
        "r1" => Button::R1,
        "l2" => Button::L2,
        "r2" => Button::R2,
        "f1" | "fn1" => Button::Fn1,
        "f2" | "fn2" => Button::Fn2,
        "select" => Button::Select,
        "start" => Button::Start,
        _ => return None,
    };
    Some((b, menu))
}

fn shortcut_display(sb: Option<(Button, bool)>) -> String {
    match sb {
        None => "None".into(),
        Some((b, menu)) => {
            let n = button_name(b).to_uppercase();
            if menu { format!("MENU+{n}") } else { n }
        }
    }
}
const MENU_SIMPLE: [&str; 5] = ["Continue", "Save", "Load", "Reset", "Quit"];

fn main() {
    std::process::exit(run());
}

fn run() -> i32 {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: {} <core.so> <rom>", args[0]);
        return 2;
    }
    let core_path = PathBuf::from(&args[1]);
    let rom_path = PathBuf::from(&args[2]);
    let on_device = std::env::var("DEVICE").is_ok();
    let tag = tag_of(&rom_path);
    let sd_root = std::env::var("SDCARD_PATH").unwrap_or_else(|_| "/mnt/SDCARD".into());
    let cfg = kui_config::Config::load(&Path::new(&sd_root).join(".userdata/shared"));

    let bios_root = std::env::var("BIOS_PATH").unwrap_or_else(|_| format!("{sd_root}/Bios"));
    let saves_root = std::env::var("SAVES_PATH").unwrap_or_else(|_| format!("{sd_root}/Saves"));
    let system_dir = Path::new(&bios_root).join(&tag);
    let save_dir = Path::new(&saves_root).join(&tag);
    let _ = std::fs::create_dir_all(&save_dir);

    // zipped roms: cores that read zips natively (dosbox, fbneo — the
    // archive IS the content) get the file untouched; everyone else gets
    // the extracted rom. Save naming keeps following the zip itself.
    let is_zip = rom_path.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip"));
    let load_path = if is_zip && !lr::Core::core_supports_zip(&core_path) {
        match extract_zip(&rom_path) {
            Some(p) => p,
            None => {
                eprintln!("zip extract failed: {}", rom_path.display());
                return 1;
            }
        }
    } else {
        rom_path.clone()
    };
    let stem = rom_path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let core_stem_name = core_path
        .file_stem()
        .map(|s| s.to_string_lossy().replace("_libretro", ""))
        .unwrap_or_default();
    // RA hardcore (docs.retroachievements.org hardcore compliance): the
    // toggle governs the whole session — cheats never touch memory, save
    // states may be written but never loaded, and core options RA bans
    // (rc_libretro tables) are dropped before they reach the core.
    let ra_hardcore =
        cfg.get_or("ra.enabled", "off") == "on" && cfg.get_or("ra.hardcore", "off") == "on";
    // options: kUI defaults, then core-level, then per-game overrides —
    // handed to Core::load so restart-gated options (read during
    // retro_load_game) see the right values
    let core_prefix = format!("core.{core_stem_name}.");
    let game_prefix = format!("game.{tag}.{stem}.opt.");
    let mut opts: Vec<(String, String)> = lr::kui_option_defaults(&core_stem_name)
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    opts.extend(cfg.keys_with_prefix(&core_prefix).into_iter().filter_map(|k| {
        cfg.get(&k).map(|v| (k[core_prefix.len()..].to_string(), v.to_string()))
    }));
    opts.extend(cfg.keys_with_prefix(&game_prefix).into_iter().filter_map(|k| {
        cfg.get(&k).map(|v| (k[game_prefix.len()..].to_string(), v.to_string()))
    }));
    if ra_hardcore {
        opts.retain(|(k, v)| {
            let keep = !hardcore::disallowed(k, v);
            if !keep {
                println!("hardcore: dropped core option {k}={v}");
            }
            keep
        });
    }
    let mut core = match lr::Core::load(&core_path, &load_path, &system_dir, &save_dir, opts) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("core load failed: {e}");
            return 1;
        }
    };
    println!(
        "core {} | {}x{} @{:.2}fps | audio {}Hz",
        core.name,
        core.av_info.geometry.base_width,
        core.av_info.geometry.base_height,
        core.av_info.timing.fps,
        core.av_info.timing.sample_rate
    );
    let rom_file = rom_path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    // cheats: standard .cht at Cheats/<TAG>/<stem>.cht, enables persisted
    let cheat_path = Path::new(&sd_root).join("Cheats").join(&tag).join(format!("{stem}.cht"));
    // (desc, code, enabled)
    let mut cheats: Vec<(String, String, bool)> = Vec::new();
    if let Ok(text) = std::fs::read_to_string(&cheat_path) {
        cheats = parse_cht(&text, &cfg, &tag, &stem);
        if ra_hardcore {
            cheats.clear();
            println!("cheats skipped (RA hardcore)");
        } else {
            let applied: Vec<(bool, String)> =
                cheats.iter().map(|(_, c, on)| (*on, c.clone())).collect();
            core.apply_cheats(&applied);
        }
    }

    // RetroAchievements session (hardcore honored when the user opted in;
    // unlocks stay server-demoted to softcore until kUI's RA approval).
    // Token login + game identify are synchronous; only attempted online.
    let mut ra: Option<kui_ra::RaClient> = None;
    let mut ra_announce: Option<String> = None;
    if cfg.get_or("ra.enabled", "off") == "on" {
        let ra_user = cfg.get_or("ra.user", "").to_string();
        let ra_token = cfg.get_or("ra.token", "").to_string();
        // no wifi gate: the offline cache serves the session when the
        // network can't, and unlocks queue for the next online flush
        let wifi_up = std::process::Command::new("sh")
            .args(["-c", "pidof wpa_supplicant >/dev/null 2>&1"])
            .status()
            .map(|st| st.success())
            .unwrap_or(false);
        if wifi_up {
            // ship anything earned offline before the new session
            std::thread::spawn(|| {
                let (sent, left) = kui_ra::flush_unlock_queue();
                if sent > 0 || left > 0 {
                    eprintln!("ra queue: {sent} sent, {left} pending");
                }
            });
        }
        if !ra_user.is_empty() && !ra_token.is_empty() {
            match kui_ra::RaClient::new(|addr, buf| {
                let n = lr::read_memory_global(addr, buf);
                if n > 0 {
                    return n;
                }
                // no core memory map: rcheevos' console region tables
                match kui_ra::translate_fallback(addr) {
                    Some((mem_id, off)) => lr::read_memory_kind(mem_id, off, buf),
                    None => 0,
                }
            }) {
                Ok(mut c) => {
                    // unique stable identity, RA-required format:
                    // EmulatorName/version (platform) rc_client/version
                    kui_ra::set_user_agent(&format!(
                        "kUI/{} (TrimUI; Linux) {}",
                        include_str!("../../../VERSION").trim(),
                        c.user_agent_clause()
                    ));
                    if ra_hardcore {
                        c.set_hardcore_enabled(true);
                    }
                    match c.login_with_token(&ra_user, &ra_token) {
                        Ok(()) => match c.load_game(&load_path) {
                            Ok(()) if c.is_game_loaded() => {
                                kui_ra::build_fallback_map(c.game_console_id());
                                let (mut locked, mut unlocked) = (0, 0);
                                if let Ok(list) = c.achievement_list() {
                                    for a in &list {
                                        if a.unlocked {
                                            unlocked += 1;
                                        } else {
                                            locked += 1;
                                        }
                                    }
                                }
                                let mut probe = [0u8; 3];
                                let pr = {
                                    let n = lr::read_memory_global(0xC000, &mut probe);
                                    if n > 0 {
                                        n
                                    } else {
                                        match kui_ra::translate_fallback(0xC000) {
                                            Some((id, off)) => {
                                                lr::read_memory_kind(id, off, &mut probe)
                                            }
                                            None => 99,
                                        }
                                    }
                                };
                                println!(
                                    "ra: console {} — {} locked / {} unlocked, {} descriptors, probe C000 -> {}",
                                    c.game_console_id(),
                                    locked,
                                    unlocked,
                                    lr::mem_maps_len(),
                                    pr
                                );
                                if std::env::var_os("KUI_TRACE").is_some()
                                    && let Ok(list) = c.achievement_list()
                                {
                                    for a in list.iter().filter(|a| !a.unlocked) {
                                        println!(
                                            "ra locked: {} — {} ({}pts)",
                                            a.title, a.description, a.points
                                        );
                                    }
                                }
                                ra_announce = Some(if unlocked > 0 {
                                    format!("RA: {locked} to earn ({unlocked} done)")
                                } else {
                                    format!("RA: {locked} to earn")
                                });
                                ra = Some(c);
                            }
                            Ok(()) => eprintln!("ra: game not recognized"),
                            Err(e) => eprintln!("ra load: {e}"),
                        },
                        Err(e) => eprintln!("ra login: {e}"),
                    }
                }
                Err(e) => eprintln!("ra init: {e}"),
            }
        }
    }
    // the established on-card conventions: <stem>.srm, <romfile>.rtc,
    // states in .userdata/shared/<TAG>-<core>/<stem>.state{,N}
    let save_stem = if is_zip && cfg.get_or("save.extracted", "off") == "on" {
        load_path
            .file_stem()
            .map(|s2| s2.to_string_lossy().into_owned())
            .unwrap_or_else(|| stem.clone())
    } else {
        stem.clone()
    };
    // one save-format choice: RetroArch (.srm) raw or rzip-compressed, or
    // minarch (.sav, always raw — minarch never compressed). The choice
    // governs save states too; reads stay transparent either way.
    let (save_ext, save_compress) = match cfg.get_or("save.format", "srm") {
        "sav" => ("sav", false),
        "srmz" => ("srm", true),
        _ => ("srm", false),
    };
    let state_compress = save_compress;
    let sav_path = save_dir.join(format!("{save_stem}.{save_ext}"));
    let rtc_path = save_dir.join(format!("{rom_file}.rtc"));
    if let Ok(bytes) = std::fs::read(&sav_path) {
        let bytes = rzip_decompress(&bytes);
        core.load_sram(&bytes);
        println!("srm loaded ({} bytes)", bytes.len());
    }
    if let Ok(bytes) = std::fs::read(&rtc_path) {
        core.load_rtc(&rzip_decompress(&bytes));
    }
    // one global power profile (Control Panel), every game alike;
    // asserted here because session.sh forces the performance governor
    // around every launch
    let power_profile = cfg.get_or("power.profile", "auto").to_string();
    tg5040::apply_power_profile(&power_profile);
    println!("power profile: {power_profile}");
    let core_stem = core_path
        .file_stem()
        .map(|s| s.to_string_lossy().replace("_libretro", ""))
        .unwrap_or_default();
    let states_dir = Path::new(&sd_root)
        .join(".userdata/shared")
        .join(format!("{tag}-{core_stem}"));
    let _ = std::fs::create_dir_all(&states_dir);
    let state_path = |slot: usize| {
        if slot == 0 {
            states_dir.join(format!("{stem}.state"))
        } else {
            states_dir.join(format!("{stem}.state{slot}"))
        }
    };
    let preview_path = |slot: usize| states_dir.join(format!("{stem}.state{slot}.png"));
    // achievement runtime rides beside each state (rc_client integration
    // guide: serialize on state save, deserialize on load, reset if absent)
    let rap_path = |slot: usize| {
        let mut s = state_path(slot).into_os_string();
        s.push(".rap");
        std::path::PathBuf::from(s)
    };

    // the selected slot (1-8) is the resume point, persisted per game
    let slot_file = states_dir.join(format!("{stem}.slot"));
    let saved_slot: usize = std::fs::read_to_string(&slot_file)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .filter(|n: &usize| *n < SLOTS)
        .unwrap_or(0);
    // legacy migration: a one-time .state.auto beats an empty slot
    let auto_state = states_dir.join(format!("{stem}.state.auto"));
    let mut pending_resume = std::fs::read(&auto_state).ok().map(|b| rzip_decompress(&b));
    if pending_resume.is_some() {
        let _ = std::fs::remove_file(&auto_state);
    }

    let mut v = match SdlVideo::new("kUI", on_device) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("video init failed: {e}");
            return 1;
        }
    };
    let joy = v.sdl.joystick().ok();
    let _sticks: Vec<_> = joy
        .map(|j| (0..j.num_joysticks().unwrap_or(0)).filter_map(|i| j.open(i).ok()).collect())
        .unwrap_or_default();

    let audio = v.sdl.audio().expect("audio subsystem");
    let spec = sdl2::audio::AudioSpecDesired {
        freq: Some(OUT_RATE),
        channels: Some(2),
        samples: Some(1024),
    };
    let device: sdl2::audio::AudioQueue<i16> = audio.open_queue(None, &spec).expect("audio queue");
    device.resume();
    let mut resampler = Resampler::new(core.av_info.timing.sample_rate);
    let target_frames = (OUT_RATE / 10) as f64; // 100ms

    use glow::HasContext as _;
    let gl = &v.gl;
    let mut renderer = Renderer::new(gl).expect("renderer");
    // theme + font for the menu
    let theme_c1 = cfg.get_color("theme.color1", 0x00FF55);
    let theme_c2 = cfg.get_color("theme.color2", 0xB3FFB3);
    let theme_c4 = cfg.get_color("theme.color4", 0x505050);
    let theme_c6 = cfg.get_color("theme.color6", 0xFFFFFF);
    let theme_notif = cfg.get_color("theme.color8", 0x000000);
    let mut font = kui_gfx::text::Font::load(
        gl,
        &resolve_font(&Path::new(&sd_root).join(".system/res"), cfg.get_or("theme.font", "0")),
    )
    .ok();
    if let Some(f) = font.as_mut() {
        f.set_bold(cfg.get_or("theme.font_style", "normal") == "bold");
    }
    let pill = kui_frontend_pill(gl).expect("pill");
    let pill_inner = SimplePill::new(gl, 36).expect("pill");
    let assets_tex =
        kui_gfx::load_png(gl, &Path::new(&sd_root).join(".system/res/assets@2x.png")).ok();
    // 1x2 tile: clear row + dark row = CRT scanlines when tiled over the frame
    let scan_tex = kui_gfx::texture_from_rgba(gl, 1, 2, &[0, 0, 0, 0, 0, 0, 0, 96])
        .inspect(|t| kui_gfx::set_texture_wrap_repeat(gl, t))
        .ok();

    // scaling + bezel: Native.png implies integer scaling (the classic
    // handheld default); an explicit config key overrides.
    // GBH holds GB/GBC/GBA hacks with different screen sizes, so one
    // overlay can't fit: pick the sibling platform's art by extension
    // (GB/GBC hacks share 160x144; GB DMG art is the safe default).
    let overlay_tag = if tag == "GBH" {
        // load_path, not rom_path: a zipped hack reveals its real
        // extension only after extraction
        match load_path.extension().map(|e| e.to_ascii_lowercase()) {
            Some(e) if e == "gba" => "GBA",
            Some(e) if e == "gbc" => "GBC",
            _ => "GB",
        }
    } else {
        tag.as_str()
    };
    let overlays_dir = Path::new(&sd_root).join("Overlays").join(overlay_tag);
    let mut scaling = if overlays_dir.join("Native.png").is_file() {
        Scaling::Native
    } else {
        Scaling::Aspect
    };
    let scaling_key = format!("game.{tag}.{stem}.scaling");
    let tag_scaling_key = format!("fe.{tag}.scaling");
    match cfg.get_or(&tag_scaling_key, "") {
        "native" => scaling = Scaling::Native,
        "aspect" => scaling = Scaling::Aspect,
        "aspectscreen" => scaling = Scaling::AspectScreen,
        "fullscreen" => scaling = Scaling::Fullscreen,
        "cropped" => scaling = Scaling::Cropped,
        _ => {}
    }
    match cfg.get_or(&scaling_key, "") {
        "native" => scaling = Scaling::Native,
        "aspect" => scaling = Scaling::Aspect,
        "aspectscreen" => scaling = Scaling::AspectScreen,
        "fullscreen" => scaling = Scaling::Fullscreen,
        "cropped" => scaling = Scaling::Cropped,
        _ => {}
    }
    // screen effect: 0 none, 1 LCD grid (bezel variants), 2 scanlines
    let effect_key = format!("game.{tag}.{stem}.effect");
    let tag_effect: u8 = match cfg.get_or(&format!("fe.{tag}.effect"), "") {
        "grid" => 1,
        "line" => 2,
        _ => 0,
    };
    let mut effect_choice: usize = match cfg.get_or(&effect_key, "") {
        "off" => 1,
        "grid" => 2,
        "line" => 3,
        _ => 0,
    };
    let mut effect: u8 = match effect_choice {
        1 => 0,
        2 => 1,
        3 => 2,
        _ => tag_effect,
    };
    let mut bezel_tex: Option<Texture> = bezel_path(&overlays_dir, &scaling, effect == 1)
        .and_then(|p| kui_gfx::load_png(gl, &p).ok());

    // fast-forward speed: game override -> console default -> 4x
    const FF_CHOICES: [usize; 7] = [2, 3, 4, 5, 6, 7, 8];
    let ff_key = format!("game.{tag}.{stem}.ff");
    let tag_ff: Option<usize> = cfg.get_or(&format!("fe.{tag}.ff"), "").parse().ok();
    let mut ff_choice: usize = FF_CHOICES
        .iter()
        .position(|c| c.to_string() == cfg.get_or(&ff_key, ""))
        .map(|i| i + 1)
        .unwrap_or(0);
    let mut ff_speed: usize = match ff_choice {
        0 => tag_ff.unwrap_or(4),
        i => FF_CHOICES[i - 1],
    };

    // dpad mode: dpad+buttons (default) or stick+buttons — the dpad
    // drives the left analog for stick-needing games (game > console)
    let dpad_key = format!("game.{tag}.{stem}.dpad");
    let resolve_dpad_stick = |wcfg: &kui_config::Config| match wcfg.get_or(&dpad_key, "") {
        "stick" => true,
        "dpad" => false,
        _ => wcfg.get_or(&format!("fe.{tag}.dpad"), "") == "stick",
    };
    let mut dpad_stick = resolve_dpad_stick(&cfg);
    core.set_dpad_as_stick(dpad_stick);

    // controls: game override -> console default -> hardwired default
    let bind_key = |phys: &str| format!("game.{tag}.{stem}.btn.{phys}");
    let mut bindings: [Binding; 8] = [Binding::Default; 8];
    let mut bind_choice: [usize; 8] = [0; 8];
    for (i, (pk, _)) in PHYS.iter().enumerate() {
        let game_v = cfg.get_or(&bind_key(pk), "").to_string();
        if let Some(ci) =
            BIND_CHOICES.iter().position(|(v, _)| *v == game_v && !v.is_empty())
        {
            bind_choice[i] = ci;
            bindings[i] = parse_binding(&game_v);
        } else {
            let tag_v = cfg.get_or(&format!("fe.{tag}.btn.{pk}"), "");
            bindings[i] = parse_binding(tag_v);
        }
    }
    let mut turbo_held: [bool; 8] = [false; 8];
    let mut frame_no: u64 = 0;

    // display extras (per-game): sharpness override, offsets, debug hud
    let sharp_key = format!("game.{tag}.{stem}.sharp");
    let mut sharp_mode: usize = match cfg.get_or(&sharp_key, "") {
        "crisp" => 1,
        "smooth" => 2,
        _ => 0,
    };
    let offx_key = format!("game.{tag}.{stem}.offx");
    let offy_key = format!("game.{tag}.{stem}.offy");
    let mut off_x: i32 = cfg.get_i32(&offx_key, 0).clamp(-64, 64);
    let mut off_y: i32 = cfg.get_i32(&offy_key, 0).clamp(-64, 64);
    // game shader: files under /Shaders, game-level choice like effect
    let shaders_dir = Path::new(&sd_root).join("Shaders");
    let mut shader_names: Vec<String> = std::fs::read_dir(&shaders_dir)
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| {
                    let p = e.path();
                    (p.extension()? == "glsl")
                        .then(|| p.file_stem().map(|s2| s2.to_string_lossy().into_owned()))?
                })
                .collect()
        })
        .unwrap_or_default();
    shader_names.sort();
    let shader_key = format!("game.{tag}.{stem}.shader");
    let mut shader_choice: usize = shader_names
        .iter()
        .position(|n| n == cfg.get_or(&shader_key, ""))
        .map(|i| i + 1)
        .unwrap_or(0);
    let mut game_shader: Option<kui_gfx::GameShader> = None;
    let hud_key = format!("game.{tag}.{stem}.hud");
    let mut hud = cfg.get_or(&hud_key, "off") == "on";
    let mut fps_frames: u32 = 0;
    let mut fps_val: f32 = 0.0;
    let mut fps_t0 = Instant::now();

    let mut frame_tex: Option<Texture> = None;
    let mut tex_size = (0u32, 0u32);
    let mut last_frame: (u32, u32, Vec<u8>) = (0, 0, Vec::new());
    let mut screen = FeScreen::Game;
    let mut slot: usize = saved_slot;
    let mut slot_preview: Option<Texture> = None;
    let mut slot_preview_for: usize = usize::MAX;
    let mut pad: u32 = 0;
    let mut ff_on = false;
    let mut ff_toggled_at = Instant::now() - std::time::Duration::from_secs(1);
    let mut turbo_all = false;
    let mut turbo_toggled_at = Instant::now() - std::time::Duration::from_secs(1);
    let mut menu_held = false;
    let mut menu_combo = false;
    let mut menu_up_at: Option<Instant> = None;
    let mut save_quit = false;
    let mut out_buf: Vec<i16> = Vec::with_capacity(4096);
    let mut last_sram_flush = Instant::now();
    let simple = cfg.get_or("ui.simple", "off") == "on";
    let mut menu_items: Vec<&str> =
        if simple { MENU_SIMPLE.to_vec() } else { MENU_FULL.to_vec() };
    if ra_hardcore {
        // hardcore: the menu shows no state UI at all — the in-game save
        // is the continuation point (loading states is banned by RA rules)
        menu_items.retain(|m| *m != "Load" && *m != "Save");
    }
    // the Options submenu family (Control Panel visual language)
    let opt_items: Vec<&str> = {
        let mut v2 = vec!["Core Options", "Controls"];
        // always offered (cheats can be downloaded in-place); hardcore
        // sessions get no cheats surface at all per RA rules
        if !ra_hardcore {
            v2.push("Cheats");
        }
        if ra.is_some() {
            v2.push("Achievements");
        }
        v2.push("Shortcuts");
        v2
    };
    // shortcuts: the old-kUI defaults, globally bound (muscle memory
    // doesn't change per cartridge). fe.shortcut.<key> overrides.
    let mut sc_bind: Vec<Option<(Button, bool)>> = SHORTCUTS
        .iter()
        .map(|(k, _, d)| {
            let gv = cfg.get_or(&format!("game.{tag}.{stem}.shortcut.{k}"), "").to_string();
            let v = if !gv.is_empty() {
                gv
            } else {
                let tv = cfg.get_or(&format!("fe.{tag}.shortcut.{k}"), "").to_string();
                if !tv.is_empty() { tv } else { (*d).to_string() }
            };
            parse_shortcut(&v)
        })
        .collect();
    let mut sc_capture: Option<usize> = None;
    let notify_save = cfg.get_or("notify.save", "on") == "on";
    let notify_load = cfg.get_or("notify.load", "on") == "on";
    let notify_shot = cfg.get_or("notify.screenshot", "on") == "on";
    let toast_ms = (cfg.get_i32("notify.duration", 2).clamp(1, 5) as u128) * 1000;
    let mut toast: Option<(String, Instant)> = None;
    let mut ra_toast: Option<(String, Instant)> = None;
    // volume/brightness OSD: (showing brightness?, expiry). While a Vol
    // key is held the expiry keeps refreshing so the bar tracks kuid's
    // auto-repeat live.
    let mut osd: Option<(bool, Instant)> = None;
    // top-right tray state for menu screens (battery/radios), lazy 10s
    let mut tray_batt: Option<u8> = None;
    let mut tray_charging = false;
    let mut tray_wifi = false;
    let mut tray_bt = false;
    let mut tray_at: Option<Instant> = None;
    let show_batt_pct = cfg.get_or("ui.battery_percent", "off") == "on";
    let mut vol_hold: Option<bool> = None;
    let mut badge_tex: HashMap<usize, Option<Texture>> = HashMap::new();
    let mut roll_state: HashMap<String, (String, Instant)> = HashMap::new();
    if std::env::var_os("KUI_TRACE").is_some() {
        eprintln!("resume: slot {} path {:?}", slot, state_path(slot));
    }
    // resume from the selected slot; fall back to a legacy auto state.
    // Hardcore: states may exist but are never loaded — always boot fresh.
    if ra_hardcore {
        if pending_resume.take().is_some() || state_path(slot).exists() {
            toast = Some(("Hardcore: fresh start".into(), Instant::now()));
        }
    } else if let Ok(bytes) = std::fs::read(state_path(slot)) {
        let bytes = rzip_decompress(&bytes);
        if core.load_state(&bytes) {
            if let Some(c) = ra.as_mut() {
                let side = std::fs::read(rap_path(slot)).ok();
                c.deserialize_progress(side.as_deref());
            }
            toast = Some((format!("Resumed slot {}", slot + 1), Instant::now()));
        } else {
            eprintln!("state resume failed: {} bytes, slot {}", bytes.len(), slot);
            toast = Some(("State load failed".into(), Instant::now()));
        }
    } else if let Some(bytes) = pending_resume.take()
        && core.load_state(&bytes)
    {
        if let Some(c) = ra.as_mut() {
            c.deserialize_progress(None);
        }
        toast = Some(("Resumed".into(), Instant::now()));
    }
    // achievements announce themselves at launch, top-left channel
    if let Some(msg) = ra_announce.take() {
        ra_toast = Some((msg, Instant::now()));
    }

    // certification probe: run a moment, report a fresh state's size, exit
    if std::env::var_os("KUI_PROBE_STATE").is_some() {
        for _ in 0..30 {
            core.run_frame();
        }
        match core.save_state() {
            Some(b) => eprintln!("probe: serialize {} bytes", b.len()),
            None => eprintln!("probe: serialize failed"),
        }
        return 0;
    }

    // hold-to-scroll for every menu list, same tuning as the launcher
    let mut nav_rep = Repeat::new();
    // in-flight cheat download (Cheats screen row 0)
    let mut cheat_dl: Option<std::sync::mpsc::Receiver<Result<(usize, String), String>>> = None;
    'run: loop {
        let mut menu_pressed = false;
        let mut sleep_req = false;
        let mut confirm = false;
        let mut back = false;
        let mut up = false;
        let mut down = false;
        let mut left = false;
        let mut right = false;
        let mut clear_btn = false;
        for ev in v.events.poll_iter() {
            use sdl2::event::Event;
            if let Event::Quit { .. } = ev {
                break 'run;
            }
            if let Event::KeyDown { keycode: Some(sdl2::keyboard::Keycode::Escape), .. } = ev {
                break 'run;
            }
            match tg5040::map_event(&ev) {
                Some(InputEvent::Dpad { up: u, down: d, left: l, right: r }) => {
                    let set = |p: &mut u32, bit: u32, on: bool| {
                        if on {
                            *p |= 1 << bit;
                        } else {
                            *p &= !(1 << bit);
                        }
                    };
                    set(&mut pad, lr::JOYPAD_UP, u);
                    set(&mut pad, lr::JOYPAD_DOWN, d);
                    set(&mut pad, lr::JOYPAD_LEFT, l);
                    set(&mut pad, lr::JOYPAD_RIGHT, r);
                    // vertical nav goes through the repeat helper (below)
                    // so held up/down keeps scrolling like the launcher
                    nav_rep.held[0][0] = u;
                    nav_rep.held[0][1] = d;
                    if l {
                        left = true;
                    }
                    if r {
                        right = true;
                    }
                }
                Some(InputEvent::Button(b, st)) => {
                    let is_down = st == ButtonState::Pressed;
                    let mut sc_ate = false;
                    match b {
                        Button::Menu => {
                            if is_down {
                                if menu_up_at
                                    .map(|t| t.elapsed().as_millis() < 40)
                                    .unwrap_or(false)
                                {
                                    // contact bounce: still the same hold
                                    menu_up_at = None;
                                } else {
                                    menu_held = true;
                                    menu_combo = false;
                                }
                            } else {
                                // don't trust a release until it lasts
                                menu_up_at = Some(Instant::now());
                            }
                        }
                        Button::VolUp | Button::VolDown => {
                            // kuid applies the change (and repeats on
                            // hold); we only mirror it with an OSD bar
                            if is_down {
                                let is_bright = menu_held;
                                if menu_held {
                                    // MENU+VOL is brightness: the MENU
                                    // release must not open the menu
                                    menu_combo = true;
                                }
                                vol_hold = Some(is_bright);
                                osd = Some((is_bright, Instant::now()));
                            } else {
                                vol_hold = None;
                            }
                        }
                        Button::X
                            if is_down
                                && sc_capture.is_none()
                                && matches!(screen, FeScreen::Shortcuts { .. }) =>
                        {
                            clear_btn = true;
                        }
                        _ if sc_capture.is_some() && is_down && b != Button::Menu => {
                            // shortcut capture: this press becomes the binding
                            let i3 = sc_capture.take().unwrap();
                            if menu_held {
                                menu_combo = true;
                            }
                            let val = if menu_held {
                                format!("menu+{}", button_name(b))
                            } else {
                                button_name(b).to_string()
                            };
                            sc_bind[i3] = parse_shortcut(&val);
                            let shared = Path::new(&sd_root).join(".userdata/shared");
                            let mut wcfg = kui_config::Config::load(&shared);
                            wcfg.set(
                                &format!("game.{tag}.{stem}.shortcut.{}", SHORTCUTS[i3].0),
                                &val,
                            );
                            let _ = wcfg.save();
                        }
                        _ if is_down
                            && matches!(screen, FeScreen::Game)
                            && sc_bind.contains(&Some((b, menu_held))) =>
                        {
                            let i3 = sc_bind
                                .iter()
                                .position(|sb| *sb == Some((b, menu_held)))
                                .unwrap();
                            if menu_held {
                                menu_combo = true;
                            }
                            sc_ate = !menu_held;
                            match SHORTCUTS[i3].0 {
                                "savequit" => save_quit = true,
                                "switcher" => {
                                    let _ = std::fs::write("/tmp/kui_switcher", "");
                                    save_quit = true;
                                }
                                "save" => {
                                    if let Some(state) = core.save_state() {
                                        let ok =
                                            write_save(&state_path(slot), &state, state_compress)
                                                .is_ok();
                                        if ok {
                                            write_progress_sidecar(&mut ra, &rap_path(slot));
                                        }
                                        if ok && last_frame.0 > 0 {
                                            let _ = kui_gfx::encode_png(
                                                &preview_path(slot),
                                                last_frame.0,
                                                last_frame.1,
                                                &last_frame.2,
                                            );
                                        }
                                        if notify_save {
                                            toast = Some((
                                                format!("Saved to slot {}", slot + 1),
                                                Instant::now(),
                                            ));
                                        }
                                        slot_preview_for = usize::MAX;
                                    }
                                }
                                "load" => {
                                    if ra_hardcore {
                                        toast = Some((
                                            "Load disabled in hardcore".into(),
                                            Instant::now(),
                                        ));
                                    } else if let Ok(bytes) = std::fs::read(state_path(slot))
                                        && core.load_state(&rzip_decompress(&bytes))
                                    {
                                        if let Some(c) = ra.as_mut() {
                                            let side = std::fs::read(rap_path(slot)).ok();
                                            c.deserialize_progress(side.as_deref());
                                        }
                                        if notify_load {
                                            toast = Some((
                                                format!("Loaded slot {}", slot + 1),
                                                Instant::now(),
                                            ));
                                        }
                                    } else {
                                        toast = Some(("Empty slot".into(), Instant::now()));
                                    }
                                }
                                "reset" => {
                                    core.reset();
                                    toast = Some(("Reset".into(), Instant::now()));
                                }
                                "screenshot" => {
                                    if last_frame.0 > 0 {
                                        let dir = Path::new(&sd_root).join("Screenshots");
                                        let _ = std::fs::create_dir_all(&dir);
                                        let ts = std::time::SystemTime::now()
                                            .duration_since(std::time::UNIX_EPOCH)
                                            .map(|d| d.as_secs())
                                            .unwrap_or(0);
                                        let ok = kui_gfx::encode_png(
                                            &dir.join(format!("{stem}.{ts}.png")),
                                            last_frame.0,
                                            last_frame.1,
                                            &last_frame.2,
                                        )
                                        .is_ok();
                                        if notify_shot || !ok {
                                            toast = Some((
                                                if ok {
                                                    "Screenshot saved".to_string()
                                                } else {
                                                    "Screenshot failed".to_string()
                                                },
                                                Instant::now(),
                                            ));
                                        }
                                    }
                                }
                                "toggleff" => {
                                    if ff_toggled_at.elapsed().as_millis() >= 180 {
                                        ff_toggled_at = Instant::now();
                                        ff_on = !ff_on;
                                        toast = Some((
                                            if ff_on {
                                                format!("FF {ff_speed}x")
                                            } else {
                                                "FF off".to_string()
                                            },
                                            Instant::now(),
                                        ));
                                    }
                                }
                                "holdff" => ff_on = true,
                                "turboall" => {
                                    if turbo_toggled_at.elapsed().as_millis() >= 180 {
                                        turbo_toggled_at = Instant::now();
                                        turbo_all = !turbo_all;
                                        toast = Some((
                                            if turbo_all {
                                                "Turbo on".to_string()
                                            } else {
                                                "Turbo off".to_string()
                                            },
                                            Instant::now(),
                                        ));
                                    }
                                }
                                "cyclescale" => {
                                    const ORDER: [Scaling; 5] = [
                                        Scaling::Native,
                                        Scaling::Aspect,
                                        Scaling::AspectScreen,
                                        Scaling::Fullscreen,
                                        Scaling::Cropped,
                                    ];
                                    let cur = ORDER
                                        .iter()
                                        .position(|s2| *s2 == scaling)
                                        .unwrap_or(0);
                                    scaling = ORDER[(cur + 1) % ORDER.len()];
                                    let val = match scaling {
                                        Scaling::Native => "native",
                                        Scaling::Aspect => "aspect",
                                        Scaling::AspectScreen => "aspectscreen",
                                        Scaling::Fullscreen => "fullscreen",
                                        Scaling::Cropped => "cropped",
                                    };
                                    let shared =
                                        Path::new(&sd_root).join(".userdata/shared");
                                    let mut wcfg = kui_config::Config::load(&shared);
                                    wcfg.set(&scaling_key, val);
                                    let _ = wcfg.save();
                                    if let Some(t) = bezel_tex.take() {
                                        renderer.drop_texture(gl, t);
                                    }
                                    bezel_tex =
                                        bezel_path(&overlays_dir, &scaling, effect == 1)
                                            .and_then(|p| kui_gfx::load_png(gl, &p).ok());
                                    toast = Some((val.to_string(), Instant::now()));
                                }
                                "cycleeffect" => {
                                    effect_choice = (effect_choice + 1) % 4;
                                    effect = match effect_choice {
                                        1 => 0,
                                        2 => 1,
                                        3 => 2,
                                        _ => tag_effect,
                                    };
                                    let shared =
                                        Path::new(&sd_root).join(".userdata/shared");
                                    let mut wcfg = kui_config::Config::load(&shared);
                                    match effect_choice {
                                        1 => wcfg.set(&effect_key, "off"),
                                        2 => wcfg.set(&effect_key, "grid"),
                                        3 => wcfg.set(&effect_key, "line"),
                                        _ => wcfg.remove_prefix(&effect_key),
                                    }
                                    let _ = wcfg.save();
                                    if let Some(t) = bezel_tex.take() {
                                        renderer.drop_texture(gl, t);
                                    }
                                    bezel_tex =
                                        bezel_path(&overlays_dir, &scaling, effect == 1)
                                            .and_then(|p| kui_gfx::load_png(gl, &p).ok());
                                    toast = Some((
                                        ["Default", "Off", "LCD Grid", "Scanlines"]
                                            [effect_choice]
                                            .to_string(),
                                        Instant::now(),
                                    ));
                                }
                                _ => {}
                            }
                        }
                        _ if !is_down
                            && sc_bind[6].map(|(bb, _)| bb) == Some(b)
                            && ff_on
                            && sc_bind[6] == Some((b, false)) =>
                        {
                            // hold-FF released
                            ff_on = false;
                        }
                        Button::Power if is_down && on_device => sleep_req = true,
                        Button::A if is_down => confirm = true,
                        Button::B if is_down => back = true,
                        _ => {}
                    }
                    let phys_i = match b {
                        Button::A => Some(0),
                        Button::B if !menu_held => Some(1),
                        Button::X if !menu_held => Some(2),
                        Button::Y => Some(3),
                        Button::L1 if !menu_held => Some(4),
                        Button::R1 if !menu_held => Some(5),
                        Button::L2 => Some(6),
                        Button::R2 => Some(7),
                        _ => None,
                    };
                    const DEFAULT_BITS: [u32; 8] = [
                        lr::JOYPAD_A,
                        lr::JOYPAD_B,
                        lr::JOYPAD_X,
                        lr::JOYPAD_Y,
                        lr::JOYPAD_L,
                        lr::JOYPAD_R,
                        lr::JOYPAD_L2,
                        lr::JOYPAD_R2,
                    ];
                    if let Some(pi2) = phys_i.filter(|_| !sc_ate) {
                        match bindings[pi2] {
                            Binding::Default | Binding::Btn(_) => {
                                let bit = match bindings[pi2] {
                                    Binding::Btn(bit) => bit,
                                    _ => DEFAULT_BITS[pi2],
                                };
                                if is_down {
                                    pad |= 1 << bit;
                                } else {
                                    pad &= !(1 << bit);
                                }
                            }
                            Binding::Turbo(_) => turbo_held[pi2] = is_down,
                            Binding::None => {}
                        }
                    }
                    let bit = match b {
                        Button::Select if !menu_held => Some(lr::JOYPAD_SELECT),
                        Button::Start if !menu_held => Some(lr::JOYPAD_START),
                        _ => None,
                    };
                    if let Some(bit) = bit {
                        if is_down {
                            pad |= 1 << bit;
                        } else {
                            pad &= !(1 << bit);
                        }
                    }
                }
                _ => {}
            }
        }

        match nav_rep.step(Instant::now()) {
            s if s < 0 => up = true,
            s if s > 0 => down = true,
            _ => {}
        }
        if let Some(is_bright) = vol_hold {
            // key still held: keymon keeps stepping, keep the bar alive
            osd = Some((is_bright, Instant::now()));
        }
        if let Some(t) = menu_up_at
            && t.elapsed().as_millis() >= 40
        {
            menu_up_at = None;
            menu_held = false;
            if !menu_combo {
                menu_pressed = true;
            }
        }
        if save_quit {
            // hardcore: the in-game save is the only continuation point,
            // so menu+start just quits — no state, no resume slot (SRAM
            // still flushes on exit below)
            if !ra_hardcore {
                if let Some(state) = core.save_state() {
                    let ok = write_save(&state_path(slot), &state, state_compress).is_ok();
                    if ok {
                        write_progress_sidecar(&mut ra, &rap_path(slot));
                    }
                    if ok && last_frame.0 > 0 {
                        let _ = kui_gfx::encode_png(
                            &preview_path(slot),
                            last_frame.0,
                            last_frame.1,
                            &last_frame.2,
                        );
                    }
                }
                let _ = std::fs::write(&slot_file, slot.to_string());
            }
            break 'run;
        }
        if sleep_req {
            // insurance: a dead battery during sleep must not lose progress
            if let Some(sram) = core.sram() {
                let _ = write_save(&sav_path, sram, save_compress);
            }
            if let Some(rtc) = core.rtc() {
                let _ = std::fs::write(&rtc_path, rtc);
            }
            if let Some(state) = core.save_state() {
                let ok = write_save(&state_path(slot), &state, state_compress).is_ok();
                if ok {
                    write_progress_sidecar(&mut ra, &rap_path(slot));
                }
                if ok && last_frame.0 > 0
                {
                    let _ = kui_gfx::encode_png(
                        &preview_path(slot),
                        last_frame.0,
                        last_frame.1,
                        &last_frame.2,
                    );
                }
                let _ = std::fs::write(&slot_file, slot.to_string());
            }
            device.pause();
            kui_hal::sdl::deep_sleep(&mut v.events, &cfg);
            device.resume();
        }
        match &mut screen {
            FeScreen::Options { .. }
            | FeScreen::Controls { .. }
            | FeScreen::Achievements { .. }
            | FeScreen::Cheats { .. }
            | FeScreen::Shortcuts { .. }
            | FeScreen::OptionsMenu { .. } => {} // handled above
            FeScreen::Game => {
                if menu_pressed {
                    // entering the menu: flush SRAM, pause audio
                    if let Some(sram) = core.sram() {
                        let _ = write_save(&sav_path, sram, save_compress);
                    }
                    if let Some(rtc) = core.rtc() {
                        let _ = std::fs::write(&rtc_path, rtc);
                    }
                    device.pause();
                    screen = FeScreen::Menu { sel: 0 };
                } else {
                    frame_no += 1;
                    let mut live_pad = pad;
                    for (i2, held) in turbo_held.iter().enumerate() {
                        if *held
                            && let Binding::Turbo(bit) = bindings[i2]
                        {
                            // ~10Hz autofire: 3 frames on, 3 off
                            if (frame_no / 3).is_multiple_of(2) {
                                live_pad |= 1 << bit;
                            } else {
                                live_pad &= !(1 << bit);
                            }
                        }
                    }
                    if turbo_all {
                        const TMASK: u32 = (1 << lr::JOYPAD_A)
                            | (1 << lr::JOYPAD_B)
                            | (1 << lr::JOYPAD_X)
                            | (1 << lr::JOYPAD_Y)
                            | (1 << lr::JOYPAD_L)
                            | (1 << lr::JOYPAD_R)
                            | (1 << lr::JOYPAD_L2)
                            | (1 << lr::JOYPAD_R2);
                        if (frame_no / 3) % 2 == 1 {
                            live_pad &= !TMASK;
                        }
                    }
                    core.set_pad(live_pad);
                    if let Some(c) = ra.as_mut() {
                        c.do_frame();
                        for ev in c.drain_events() {
                            match ev {
                                kui_ra::RaEvent::AchievementUnlocked(a) => {
                                    ra_toast = Some((
                                        format!("{} (+{} pts)", a.title, a.points),
                                        Instant::now(),
                                    ));
                                    // The Dude counts these
                                    let p = Path::new(&sd_root)
                                        .join(".userdata/shared/kui/ra_unlocks.txt");
                                    let n3: u64 = std::fs::read_to_string(&p)
                                        .ok()
                                        .and_then(|s2| s2.trim().parse().ok())
                                        .unwrap_or(0);
                                    let _ = std::fs::write(&p, (n3 + 1).to_string());
                                }
                                kui_ra::RaEvent::GameCompleted => {
                                    ra_toast =
                                        Some(("Game mastered!".into(), Instant::now()));
                                }
                                kui_ra::RaEvent::Reset => {
                                    // rc_client demands a full reset (e.g.
                                    // switching into hardcore mid-session)
                                    core.reset();
                                    ra_toast =
                                        Some(("Game reset (RA)".into(), Instant::now()));
                                }
                                kui_ra::RaEvent::Disconnected => {
                                    ra_toast = Some((
                                        "RA offline — unlocks will queue".into(),
                                        Instant::now(),
                                    ));
                                }
                                kui_ra::RaEvent::Reconnected => {
                                    ra_toast =
                                        Some(("RA back online".into(), Instant::now()));
                                }
                                kui_ra::RaEvent::ServerError { api, message } => {
                                    eprintln!("ra server error: {api}: {message}");
                                }
                                kui_ra::RaEvent::Other(_) => {}
                            }
                        }
                    }
                    if ff_on {
                        // fast-forward: silent frames ahead of the shown one
                        for _ in 0..ff_speed.saturating_sub(1) {
                            core.run_frame();
                            let _ = core.take_audio();
                            let _ = core.take_video();
                        }
                    }
                    core.run_frame();
                    if core.refresh_av() {
                        resampler = Resampler::new(core.av_info.timing.sample_rate);
                    }

                    let samples = core.take_audio();
                    if !samples.is_empty() && !ff_on {
                        let queued = (device.size() / 4) as f64;
                        resampler.tune(queued, target_frames);
                        out_buf.clear();
                        resampler.push(&samples, &mut out_buf);
                        if device.size() < (OUT_RATE as u32) * 4 / 2 {
                            let _ = device.queue_audio(&out_buf);
                        }
                    }

                    if let Some((w, h, rgba)) = core.take_video() {
                        if tex_size != (w, h) {
                            if let Some(t) = frame_tex.take() {
                                renderer.drop_texture(gl, t);
                            }
                            frame_tex = kui_gfx::texture_from_rgba(gl, w, h, &rgba).ok();
                            tex_size = (w, h);
                        } else if let Some(t) = &frame_tex {
                            kui_gfx::upload_sub_rgba(gl, t, 0, 0, w, h, &rgba);
                        }
                        last_frame = (w, h, rgba);
                    }
                }
            }
            FeScreen::Menu { sel } => {
                if up {
                    *sel = (*sel + menu_items.len() - 1) % menu_items.len();
                }
                if down {
                    *sel = (*sel + 1) % menu_items.len();
                }
                if (left || right)
                    && matches!(menu_items[*sel], "Save" | "Load")
                {
                    slot = if left { (slot + SLOTS - 1) % SLOTS } else { (slot + 1) % SLOTS };
                    let _ = std::fs::write(&slot_file, slot.to_string());
                }
                if (left || right) && menu_items[*sel] == "Continue" {
                    // multi-disc: cycle and swap through the disk interface
                    let n = core.disc_count();
                    if n > 1 {
                        let cur = core.disc_index();
                        let next =
                            if left { (cur + n - 1) % n } else { (cur + 1) % n };
                        if core.disc_set(next) {
                            toast =
                                Some((format!("Disc {}", next + 1), Instant::now()));
                        }
                    }
                }
                if menu_pressed || back || (confirm && menu_items[*sel] == "Continue") {
                    device.resume();
                    screen = FeScreen::Game;
                } else if confirm {
                    match menu_items[*sel] {
                        "Save" => {
                            // save state + preview
                            if let Some(state) = core.save_state() {
                                let ok =
                                    write_save(&state_path(slot), &state, state_compress).is_ok();
                                if ok {
                                    write_progress_sidecar(&mut ra, &rap_path(slot));
                                }
                                if ok && last_frame.0 > 0 {
                                    let _ = kui_gfx::encode_png(
                                        &preview_path(slot),
                                        last_frame.0,
                                        last_frame.1,
                                        &last_frame.2,
                                    );
                                }
                                if notify_save {
                                    toast = Some((
                                        format!("Saved to slot {}", slot + 1),
                                        Instant::now(),
                                    ));
                                }
                                slot_preview_for = usize::MAX; // refresh preview
                            }
                        }
                        "Load" => {
                            if let Ok(bytes) = std::fs::read(state_path(slot)) {
                                if core.load_state(&rzip_decompress(&bytes)) {
                                    if let Some(c) = ra.as_mut() {
                                        let side = std::fs::read(rap_path(slot)).ok();
                                        c.deserialize_progress(side.as_deref());
                                    }
                                    if notify_load {
                                        toast = Some((
                                            format!("Loaded slot {}", slot + 1),
                                            Instant::now(),
                                        ));
                                    }
                                    device.resume();
                                    screen = FeScreen::Game;
                                }
                            } else {
                                toast = Some(("Empty slot".into(), Instant::now()));
                            }
                        }
                        "Options" => {
                            screen = FeScreen::OptionsMenu { sel: 0 };
                            // consume the press: the submenu handler runs
                            // later this same frame
                            confirm = false;
                        }
                        "Reset" => {
                            core.reset();
                            device.resume();
                            screen = FeScreen::Game;
                        }
                        "Quit" => {
                            if Path::new(&sd_root).join(".noui").is_file() {
                                let _ = std::fs::write("/tmp/poweroff", "");
                            }
                            break 'run;
                        }
                        _ => {}
                    }
                }
            }
        }

        if let FeScreen::Options { sel, scroll } = &mut screen {
            let defs = core.var_defs();
            // Scaling/Effect/FF up top; Sharpness/Offsets/HUD/Shader + Restore
            let n = defs.len() + 9;
            let mut reload_bezel = false;
            {
                if up {
                    *sel = (*sel + n - 1) % n;
                }
                if down {
                    *sel = (*sel + 1) % n;
                }
                let visible = 9usize;
                if *sel < *scroll {
                    *scroll = *sel;
                }
                if *sel >= *scroll + visible {
                    *scroll = *sel + 1 - visible;
                }
                if (left || right) && *sel == 0 {
                    const ORDER: [Scaling; 5] = [
                        Scaling::Native,
                        Scaling::Aspect,
                        Scaling::AspectScreen,
                        Scaling::Fullscreen,
                        Scaling::Cropped,
                    ];
                    let cur = ORDER.iter().position(|s2| *s2 == scaling).unwrap_or(0);
                    scaling =
                        ORDER[(cur + if left { ORDER.len() - 1 } else { 1 }) % ORDER.len()];
                    let val = match scaling {
                        Scaling::Native => "native",
                        Scaling::Aspect => "aspect",
                        Scaling::AspectScreen => "aspectscreen",
                        Scaling::Fullscreen => "fullscreen",
                        Scaling::Cropped => "cropped",
                    };
                    let shared = Path::new(&sd_root).join(".userdata/shared");
                    let mut wcfg = kui_config::Config::load(&shared);
                    wcfg.set(&scaling_key, val);
                    let _ = wcfg.save();
                    reload_bezel = true;
                }
                if (left || right) && *sel == 1 {
                    effect_choice = (effect_choice + if left { 3 } else { 1 }) % 4;
                    effect = match effect_choice {
                        1 => 0,
                        2 => 1,
                        3 => 2,
                        _ => tag_effect,
                    };
                    let shared = Path::new(&sd_root).join(".userdata/shared");
                    let mut wcfg = kui_config::Config::load(&shared);
                    match effect_choice {
                        1 => wcfg.set(&effect_key, "off"),
                        2 => wcfg.set(&effect_key, "grid"),
                        3 => wcfg.set(&effect_key, "line"),
                        _ => wcfg.remove_prefix(&effect_key),
                    }
                    let _ = wcfg.save();
                    reload_bezel = true;
                }
                if (left || right) && *sel == 2 {
                    let m = FF_CHOICES.len() + 1;
                    ff_choice = (ff_choice + if left { m - 1 } else { 1 }) % m;
                    ff_speed = match ff_choice {
                        0 => tag_ff.unwrap_or(4),
                        i => FF_CHOICES[i - 1],
                    };
                    let shared = Path::new(&sd_root).join(".userdata/shared");
                    let mut wcfg = kui_config::Config::load(&shared);
                    match ff_choice {
                        0 => wcfg.remove_prefix(&ff_key),
                        i => wcfg.set(&ff_key, FF_CHOICES[i - 1].to_string()),
                    }
                    let _ = wcfg.save();
                }
                let tail = defs.len() + 3;
                if (left || right) && *sel == tail {
                    sharp_mode = (sharp_mode + if left { 2 } else { 1 }) % 3;
                    let shared = Path::new(&sd_root).join(".userdata/shared");
                    let mut wcfg = kui_config::Config::load(&shared);
                    match sharp_mode {
                        1 => wcfg.set(&sharp_key, "crisp"),
                        2 => wcfg.set(&sharp_key, "smooth"),
                        _ => wcfg.remove_prefix(&sharp_key),
                    }
                    let _ = wcfg.save();
                }
                if (left || right) && (*sel == tail + 1 || *sel == tail + 2) {
                    let d = if left { -8 } else { 8 };
                    let shared = Path::new(&sd_root).join(".userdata/shared");
                    let mut wcfg = kui_config::Config::load(&shared);
                    if *sel == tail + 1 {
                        off_x = (off_x + d).clamp(-64, 64);
                        wcfg.set(&offx_key, off_x.to_string());
                    } else {
                        off_y = (off_y + d).clamp(-64, 64);
                        wcfg.set(&offy_key, off_y.to_string());
                    }
                    let _ = wcfg.save();
                }
                if (left || right) && *sel == tail + 4 {
                    let m = shader_names.len() + 1;
                    shader_choice =
                        (shader_choice + if left { m - 1 } else { 1 }) % m;
                    let shared = Path::new(&sd_root).join(".userdata/shared");
                    let mut wcfg = kui_config::Config::load(&shared);
                    if shader_choice == 0 {
                        wcfg.remove_prefix(&shader_key);
                    } else {
                        wcfg.set(&shader_key, &shader_names[shader_choice - 1]);
                    }
                    let _ = wcfg.save();
                    if let Some(gs) = game_shader.take() {
                        gs.destroy(gl);
                    }
                }
                if (left || right) && *sel == tail + 3 {
                    hud = !hud;
                    let shared = Path::new(&sd_root).join(".userdata/shared");
                    let mut wcfg = kui_config::Config::load(&shared);
                    wcfg.set(&hud_key, if hud { "on" } else { "off" });
                    let _ = wcfg.save();
                }
                if confirm && *sel == defs.len() + 8 {
                    // Restore defaults: game scope only — the console
                    // defaults (Core Options in the Control Panel) win again
                    let shared = Path::new(&sd_root).join(".userdata/shared");
                    let mut wcfg = kui_config::Config::load(&shared);
                    wcfg.remove_prefix(&game_prefix);
                    wcfg.remove_prefix(&scaling_key);
                    wcfg.remove_prefix(&effect_key);
                    wcfg.remove_prefix(&ff_key);
                    wcfg.remove_prefix(&sharp_key);
                    wcfg.remove_prefix(&offx_key);
                    wcfg.remove_prefix(&offy_key);
                    wcfg.remove_prefix(&hud_key);
                    wcfg.remove_prefix(&shader_key);
                    shader_choice = 0;
                    if let Some(gs) = game_shader.take() {
                        gs.destroy(gl);
                    }
                    sharp_mode = 0;
                    off_x = 0;
                    off_y = 0;
                    hud = false;
                    let _ = wcfg.save();
                    // live values re-derive from console scope
                    let console: Vec<(String, String)> = defs
                        .iter()
                        .filter_map(|d| {
                            let v = wcfg
                                .get(&format!("core.{core_stem_name}.{}", d.key))
                                .map(str::to_string)
                                .or_else(|| {
                                    lr::kui_option_default(&core_stem_name, &d.key)
                                        .map(str::to_string)
                                })
                                .or_else(|| d.choices.first().cloned())?;
                            Some((d.key.clone(), v))
                        })
                        .collect();
                    for (k, vv) in &console {
                        if ra_hardcore && hardcore::disallowed(k, vv) {
                            continue;
                        }
                        core.set_var(k, vv);
                    }
                    scaling = match wcfg.get_or(&tag_scaling_key, "") {
                        "native" => Scaling::Native,
                        "aspect" => Scaling::Aspect,
                        "aspectscreen" => Scaling::AspectScreen,
                        "fullscreen" => Scaling::Fullscreen,
                        "cropped" => Scaling::Cropped,
                        _ if overlays_dir.join("Native.png").is_file() => Scaling::Native,
                        _ => Scaling::Aspect,
                    };
                    effect = tag_effect;
                    effect_choice = 0;
                    ff_choice = 0;
                    ff_speed = tag_ff.unwrap_or(4);
                    reload_bezel = true;
                    toast = Some(("Game defaults restored".into(), Instant::now()));
                }
                if (left || right)
                    && *sel > 2
                    && let Some(def) = defs.get(*sel - 3)
                {
                    let cur = core.var_value(&def.key).unwrap_or_default();
                    let idx = def.choices.iter().position(|c| *c == cur).unwrap_or(0);
                    let ni = if left {
                        (idx + def.choices.len() - 1) % def.choices.len()
                    } else {
                        (idx + 1) % def.choices.len()
                    };
                    if let Some(nv) = def.choices.get(ni) {
                        if ra_hardcore && hardcore::disallowed(&def.key, nv) {
                            toast = Some(("Blocked in hardcore".into(), Instant::now()));
                        } else {
                            core.set_var(&def.key, nv);
                            let _ = &nv;
                            // persist per core
                            let ckey = format!("{game_prefix}{}", def.key);
                            let shared = Path::new(&sd_root).join(".userdata/shared");
                            let mut wcfg = kui_config::Config::load(&shared);
                            wcfg.set(&ckey, nv);
                            let _ = wcfg.save();
                        }
                    }
                }
            }
            if reload_bezel {
                if let Some(t) = bezel_tex.take() {
                    renderer.drop_texture(gl, t);
                }
                bezel_tex = bezel_path(&overlays_dir, &scaling, effect == 1)
                    .and_then(|p| kui_gfx::load_png(gl, &p).ok());
            }
            if back || menu_pressed {
                screen = FeScreen::OptionsMenu { sel: 0 };
                back = false;
                menu_pressed = false;
            }
        }

        if let FeScreen::Achievements { rows, scroll } = &mut screen {
            let per = 2usize; // rows scrolled per press
            if up {
                *scroll = scroll.saturating_sub(per);
            }
            if down && *scroll + 6 < rows.len() {
                *scroll += per;
            }
            if back || menu_pressed {
                let idx = opt_items
                    .iter()
                    .position(|m| *m == "Achievements")
                    .unwrap_or(0);
                screen = FeScreen::OptionsMenu { sel: idx };
                back = false;
                menu_pressed = false;
            }
        }
        if let FeScreen::Shortcuts { sel } = &mut screen {
            let n = SHORTCUTS.len();
            if up {
                *sel = (*sel + n - 1) % n;
            }
            if down {
                *sel = (*sel + 1) % n;
            }
            if confirm && sc_capture.is_none() {
                sc_capture = Some(*sel);
            }
            if clear_btn {
                sc_bind[*sel] = None;
                let shared = Path::new(&sd_root).join(".userdata/shared");
                let mut wcfg = kui_config::Config::load(&shared);
                wcfg.set(
                    &format!("game.{tag}.{stem}.shortcut.{}", SHORTCUTS[*sel].0),
                    "none",
                );
                let _ = wcfg.save();
            }
            if back || menu_pressed {
                sc_capture = None;
                let idx =
                    opt_items.iter().position(|m| *m == "Shortcuts").unwrap_or(0);
                screen = FeScreen::OptionsMenu { sel: idx };
                back = false;
                menu_pressed = false;
            }
        }
        if let FeScreen::Cheats { sel, scroll } = &mut screen {
            // row 0 = "Download cheats", cheat entries follow
            let n = cheats.len() + 1;
            if up {
                *sel = (*sel + n - 1) % n;
            }
            if down {
                *sel = (*sel + 1) % n;
            }
            let visible = 8usize;
            if *sel < *scroll {
                *scroll = *sel;
            }
            if *sel >= *scroll + visible {
                *scroll = *sel + 1 - visible;
            }
            if (left || right || confirm) && *sel > 0 {
                let ci = *sel - 1;
                cheats[ci].2 = !cheats[ci].2;
                let shared = Path::new(&sd_root).join(".userdata/shared");
                let mut wcfg = kui_config::Config::load(&shared);
                wcfg.set(
                    &format!("game.{tag}.{stem}.cheat.{ci}"),
                    if cheats[ci].2 { "on" } else { "off" },
                );
                let _ = wcfg.save();
                let applied: Vec<(bool, String)> =
                    cheats.iter().map(|(_, c, on)| (*on, c.clone())).collect();
                core.apply_cheats(&applied);
            } else if confirm && *sel == 0 && cheat_dl.is_none() {
                let wifi_up = std::process::Command::new("sh")
                    .args(["-c", "pidof wpa_supplicant >/dev/null 2>&1"])
                    .status()
                    .map(|st| st.success())
                    .unwrap_or(false);
                if !wifi_up {
                    toast = Some(("WiFi is off".into(), Instant::now()));
                } else {
                    let (tx, rx) = std::sync::mpsc::channel();
                    let (t2, s2, d2) = (tag.clone(), stem.clone(), cheat_path.clone());
                    std::thread::spawn(move || {
                        let _ = tx.send(cheatdl::download(&t2, &s2, &d2));
                    });
                    cheat_dl = Some(rx);
                }
            }
            if back || menu_pressed {
                let idx = opt_items.iter().position(|m| *m == "Cheats").unwrap_or(0);
                screen = FeScreen::OptionsMenu { sel: idx };
                back = false;
                menu_pressed = false;
            }
        }
        // finished cheat download: reload the list, report either way
        if let Some(rx) = &cheat_dl
            && let Ok(res) = rx.try_recv()
        {
            cheat_dl = None;
            match res {
                Ok((n2, name)) => {
                    if let Ok(text) = std::fs::read_to_string(&cheat_path) {
                        let shared = Path::new(&sd_root).join(".userdata/shared");
                        cheats =
                            parse_cht(&text, &kui_config::Config::load(&shared), &tag, &stem);
                    }
                    toast = Some((format!("{n2} cheats: {name}"), Instant::now()));
                }
                Err(e) => toast = Some((e, Instant::now())),
            }
        }
        if let FeScreen::OptionsMenu { sel } = &mut screen {
            let n = opt_items.len();
            if up {
                *sel = (*sel + n - 1) % n;
            }
            if down {
                *sel = (*sel + 1) % n;
            }
            if confirm {
                confirm = false; // children run later this frame
                match opt_items[*sel] {
                    "Core Options" => screen = FeScreen::Options { sel: 0, scroll: 0 },
                    "Controls" => screen = FeScreen::Controls { sel: 0 },
                    "Cheats" => screen = FeScreen::Cheats { sel: 0, scroll: 0 },
                    "Shortcuts" => screen = FeScreen::Shortcuts { sel: 0 },
                    "Achievements" => {
                        let rows = ra
                            .as_mut()
                            .and_then(|c| c.achievement_list().ok())
                            .map(|l| {
                                l.into_iter()
                                    .map(|a2| {
                                        let f2 = a2
                                            .badge_url
                                            .as_deref()
                                            .and_then(|u| {
                                                u.rsplit('/').next().map(str::to_string)
                                            })
                                            .unwrap_or_default();
                                        (
                                            a2.unlocked,
                                            format!(
                                                "{} (+{} pts)",
                                                a2.title, a2.points
                                            ),
                                            format!("{f2}\u{1}{}", a2.description),
                                        )
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();
                        screen = FeScreen::Achievements { rows, scroll: 0 };
                    }
                    _ => {}
                }
            } else if back || menu_pressed {
                let idx = menu_items.iter().position(|m| *m == "Options").unwrap_or(0);
                screen = FeScreen::Menu { sel: idx };
            }
        }
        if let FeScreen::Controls { sel } = &mut screen {
            // row 0 = Dpad mode, 1..=8 = the phys buttons, last = Restore
            let n = PHYS.len() + 2;
            if up {
                *sel = (*sel + n - 1) % n;
            }
            if down {
                *sel = (*sel + 1) % n;
            }
            if (left || right) && *sel == 0 {
                dpad_stick = !dpad_stick;
                core.set_dpad_as_stick(dpad_stick);
                let shared = Path::new(&sd_root).join(".userdata/shared");
                let mut wcfg = kui_config::Config::load(&shared);
                wcfg.set(&dpad_key, if dpad_stick { "stick" } else { "dpad" });
                let _ = wcfg.save();
            }
            if (left || right) && *sel >= 1 && *sel <= PHYS.len() {
                let bi = *sel - 1;
                let m = BIND_CHOICES.len();
                bind_choice[bi] = (bind_choice[bi] + if left { m - 1 } else { 1 }) % m;
                let (v, _) = BIND_CHOICES[bind_choice[bi]];
                let shared = Path::new(&sd_root).join(".userdata/shared");
                let mut wcfg = kui_config::Config::load(&shared);
                if v.is_empty() {
                    wcfg.remove_prefix(&bind_key(PHYS[bi].0));
                    let tag_v = wcfg
                        .get_or(&format!("fe.{tag}.btn.{}", PHYS[bi].0), "")
                        .to_string();
                    bindings[bi] = parse_binding(&tag_v);
                } else {
                    wcfg.set(&bind_key(PHYS[bi].0), v);
                    bindings[bi] = parse_binding(v);
                }
                let _ = wcfg.save();
            }
            if confirm && *sel == PHYS.len() + 1 {
                let shared = Path::new(&sd_root).join(".userdata/shared");
                let mut wcfg = kui_config::Config::load(&shared);
                for (pk, _) in PHYS.iter() {
                    wcfg.remove_prefix(&bind_key(pk));
                }
                wcfg.remove_prefix(&dpad_key);
                let _ = wcfg.save();
                for (i2, (pk, _)) in PHYS.iter().enumerate() {
                    bind_choice[i2] = 0;
                    let tag_v = wcfg.get_or(&format!("fe.{tag}.btn.{pk}"), "").to_string();
                    bindings[i2] = parse_binding(&tag_v);
                }
                dpad_stick = resolve_dpad_stick(&wcfg);
                core.set_dpad_as_stick(dpad_stick);
                toast = Some(("Controls restored".into(), Instant::now()));
            }
            if back || menu_pressed {
                let idx = opt_items.iter().position(|m| *m == "Controls").unwrap_or(0);
                screen = FeScreen::OptionsMenu { sel: idx };
            }
        }

        if !matches!(screen, FeScreen::Game)
            && let Some(c) = ra.as_mut()
        {
            c.idle();
        }

        // ---- render ----
        let (sw, sh) = v.drawable_size();
        renderer.begin_frame(gl, sw, sh, [0.0, 0.0, 0.0]);
        if let Some(t) = &frame_tex {
            // Native: pixel-perfect nearest. Other modes: sharp-bilinear —
            // crisp texels, blended only at block seams (the classic look)
            let native_now = match sharp_mode {
                1 => true,
                2 => false,
                _ => matches!(scaling, Scaling::Native | Scaling::Cropped),
            };
            kui_gfx::set_texture_filter(gl, t, native_now);
            let (gw, gh) = (tex_size.0 as f32, tex_size.1 as f32);
            let (dw, dh) = match scaling {
                Scaling::Fullscreen | Scaling::AspectScreen => (sw as f32, sh as f32),
                Scaling::Native => {
                    let s = (sw as f32 / gw).min(sh as f32 / gh).floor().max(1.0);
                    (gw * s, gh * s)
                }
                Scaling::Cropped => {
                    // next integer above the fit; overflow is cropped
                    let s = (sw as f32 / gw).min(sh as f32 / gh).ceil().max(1.0);
                    (gw * s, gh * s)
                }
                Scaling::Aspect => {
                    let aspect = if core.av_info.geometry.aspect_ratio > 0.0 {
                        core.av_info.geometry.aspect_ratio
                    } else {
                        gw / gh
                    };
                    let (mut w2, mut h2) = (sh as f32 * aspect, sh as f32);
                    if w2 > sw as f32 {
                        w2 = sw as f32;
                        h2 = w2 / aspect;
                    }
                    (w2, h2)
                }
            };
            let dim = if matches!(
                screen,
                FeScreen::Menu { .. }
                    | FeScreen::Options { .. }
                    | FeScreen::Controls { .. }
                    | FeScreen::Achievements { .. }
                    | FeScreen::Cheats { .. }
                    | FeScreen::Shortcuts { .. }
                    | FeScreen::OptionsMenu { .. }
            ) {
                // menu glass: darker, still translucent
                [0.10, 0.10, 0.10, 1.0]
            } else {
                WHITE
            };
            let sharp = if native_now || sharp_mode == 2 {
                0.0
            } else {
                (dh / gh).floor().max(1.0)
            };
            let (dx, dy) = (
                (sw as f32 - dw) / 2.0 + off_x as f32,
                (sh as f32 - dh) / 2.0 + off_y as f32,
            );
            // lazy-compile the selected shader; fall back on failure
            if shader_choice > 0 && game_shader.is_none() {
                let path = shaders_dir
                    .join(format!("{}.glsl", shader_names[shader_choice - 1]));
                match std::fs::read_to_string(&path)
                    .map_err(|e| e.to_string())
                    .and_then(|src| kui_gfx::GameShader::load(gl, &src))
                {
                    Ok(gs) => game_shader = Some(gs),
                    Err(e) => {
                        eprintln!("shader {}: {e}", path.display());
                        shader_choice = 0;
                    }
                }
            }
            if let (true, Some(gs)) = (shader_choice > 0, game_shader.as_ref()) {
                kui_gfx::set_texture_filter(gl, t, false);
                gs.draw(gl, t, (sw, sh), dx, dy, dw, dh, frame_no as u32);
                renderer.rebind_quad(gl);
            } else {
                renderer.set_sharp(gl, sharp, gw, gh);
                renderer.draw(gl, t, dx, dy, dw, dh, dim);
                renderer.set_sharp(gl, 0.0, 1.0, 1.0);
            }
            if let Some(bz) = &bezel_tex {
                renderer.draw(gl, bz, 0.0, 0.0, sw as f32, sh as f32, dim);
            }
            if effect == 2
                && let Some(st) = &scan_tex
            {
                // one dark line per game pixel row
                renderer.draw_uv(gl, st, dx, dy, dw, dh, [0.0, 0.0, 1.0, gh], dim);
            }
        }

        if let FeScreen::Options { sel, scroll } = &screen
            && let Some(f) = font.as_mut()
        {
            let defs = core.var_defs();
            {
                let total = defs.len() + 9;
                let visible = 9usize;
                let top = 40.0;
                for row in 0..visible.min(total) {
                    let idx = scroll + row;
                    if idx >= total {
                        break;
                    }
                    let (label, val) = if idx == 0 {
                        (
                            "Scaling".to_string(),
                            match scaling {
                                Scaling::Native => "Native".to_string(),
                                Scaling::Aspect => "Aspect".to_string(),
                                Scaling::AspectScreen => "Aspect Screen".to_string(),
                                Scaling::Fullscreen => "Fullscreen".to_string(),
                                Scaling::Cropped => "Cropped".to_string(),
                            },
                        )
                    } else if idx == 1 {
                        (
                            "Effect".to_string(),
                            match effect_choice {
                                1 => "Off".to_string(),
                                2 => "LCD Grid".to_string(),
                                3 => "Scanlines".to_string(),
                                _ => "Default".to_string(),
                            },
                        )
                    } else if idx == 2 {
                        (
                            "FF Speed".to_string(),
                            match ff_choice {
                                0 => "Default".to_string(),
                                i => format!("{}x", FF_CHOICES[i - 1]),
                            },
                        )
                    } else if idx == defs.len() + 3 {
                        (
                            "Sharpness".to_string(),
                            match sharp_mode {
                                1 => "Crisp".to_string(),
                                2 => "Smooth".to_string(),
                                _ => "Default".to_string(),
                            },
                        )
                    } else if idx == defs.len() + 4 {
                        ("Offset X".to_string(), off_x.to_string())
                    } else if idx == defs.len() + 5 {
                        ("Offset Y".to_string(), off_y.to_string())
                    } else if idx == defs.len() + 6 {
                        (
                            "Debug HUD".to_string(),
                            if hud { "On".to_string() } else { "Off".to_string() },
                        )
                    } else if idx == defs.len() + 7 {
                        (
                            "Shader".to_string(),
                            if shader_choice == 0 {
                                "Off".to_string()
                            } else {
                                shader_names[shader_choice - 1].clone()
                            },
                        )
                    } else if idx == defs.len() + 8 {
                        ("Restore defaults".to_string(), String::new())
                    } else {
                        let def = &defs[idx - 3];
                        (
                            if def.desc.is_empty() { def.key.clone() } else { def.desc.clone() },
                            core.var_value(&def.key).unwrap_or_default(),
                        )
                    };
                    let y = top + row as f32 * ROW_H;
                    let lh = f.line_height(26);
                    let pill_y = y + (ROW_H - PILL_H as f32) / 2.0;
                    let text_y = pill_y + (PILL_H as f32 - lh) / 2.0;
                    if idx == *sel {
                        let sel_c = theme_c1;
                        fe_draw_roll(
                            f, &renderer, gl, &mut roll_state, "options", &label, 72.0,
                            text_y, pill_y, PILL_H as f32, 26, sel_c,
                            sw as f32 * 0.55,
                        );
                        let vw = f.measure(gl, &val, 26);
                        f.draw(&renderer, gl, &val, sw as f32 - 72.0 - vw, text_y, 26, sel_c);
                    } else {
                        let shown = f.fit(gl, &label, 26, sw as f32 * 0.55);
                        f.draw(&renderer, gl, &shown, 72.0, text_y, 26, theme_c4);
                        let vw = f.measure(gl, &val, 26);
                        f.draw(&renderer, gl, &val, sw as f32 - 72.0 - vw, text_y, 26, theme_c4);
                    }
                }
            }
        }
        if let FeScreen::Achievements { rows, scroll } = &screen
            && let Some(f) = font.as_mut()
        {
            let top = 36.0;
            for (i, (unlocked, title, meta)) in
                rows.iter().skip(*scroll).take(7).enumerate()
            {
                let y = top + i as f32 * 96.0;
                let (bfile, desc) = meta.split_once('\u{1}').unwrap_or(("", meta));
                let bdir2 = Path::new(&sd_root).join(".userdata/shared/.ra/badges");
                // locked achievements prefer the _lock variant when present
                let bp = if !*unlocked
                    && let Some(stem2) = bfile.strip_suffix(".png")
                {
                    let lp = bdir2.join(format!("{stem2}_lock.png"));
                    if lp.is_file() { lp } else { bdir2.join(bfile) }
                } else {
                    bdir2.join(bfile)
                };
                let idx2 = *scroll + i;
                // load when the download lands; a miss is never cached
                if !badge_tex.contains_key(&idx2)
                    && bp.is_file()
                    && let Ok(t) = kui_gfx::load_png(gl, &bp)
                {
                    badge_tex.insert(idx2, Some(t));
                }
                if let Some(Some(t)) = badge_tex.get(&idx2) {
                    let tint = if *unlocked {
                        WHITE
                    } else {
                        [0.35, 0.35, 0.35, 1.0]
                    };
                    renderer.draw(gl, t, 40.0, y, 64.0, 64.0, tint);
                }
                let mark = if *unlocked { "[*] " } else { "[ ] " };
                let t2 = f.fit(gl, &format!("{mark}{title}"), 26, sw as f32 - 200.0);
                f.draw(
                    &renderer,
                    gl,
                    &t2,
                    120.0,
                    y,
                    26,
                    if *unlocked { theme_c1 } else { [1.0, 1.0, 1.0, 1.0] },
                );
                let d2 = f.fit(gl, desc, 20, sw as f32 - 220.0);
                f.draw(&renderer, gl, &d2, 120.0, y + 36.0, 20, theme_c4);
            }
            let done = rows.iter().filter(|r| r.0).count();
            let ftr = format!("{done} / {} unlocked", rows.len());
            let fw2 = f.measure(gl, &ftr, 22);
            f.draw(
                &renderer,
                gl,
                &ftr,
                (sw as f32 - fw2) / 2.0,
                sh as f32 - 44.0,
                22,
                theme_c4,
            );
        }
        if let FeScreen::Shortcuts { sel } = &screen
            && let Some(f) = font.as_mut()
        {
            let n = SHORTCUTS.len();
            let top = (sh as f32 - n as f32 * 58.0) / 2.0;
            for (i, (_, label, _)) in SHORTCUTS.iter().enumerate() {
                let y = top + i as f32 * 58.0;
                let lh = f.line_height(24);
                let pill_y = y + (58.0 - PILL_H as f32) / 2.0;
                let text_y = pill_y + (PILL_H as f32 - lh) / 2.0;
                let val = if sc_capture == Some(i) {
                    "Press a button...".to_string()
                } else {
                    shortcut_display(sc_bind[i])
                };
                let vw = f.measure(gl, &val, 24);
                if i == *sel {
                    let sel_c = theme_c1;
                    f.draw(&renderer, gl, label, 96.0, text_y, 24, sel_c);
                    f.draw(&renderer, gl, &val, sw as f32 - 96.0 - vw, text_y, 24, sel_c);
                } else {
                    f.draw(&renderer, gl, label, 96.0, text_y, 24, theme_c4);
                    f.draw(&renderer, gl, &val, sw as f32 - 96.0 - vw, text_y, 24, theme_c4);
                }
            }
        }
        if let FeScreen::Cheats { sel, scroll } = &screen
            && let Some(f) = font.as_mut()
        {
            let top = 40.0;
            let total = cheats.len() + 1;
            for i in *scroll..(*scroll + 8).min(total) {
                let row = i - *scroll;
                let y = top + row as f32 * ROW_H;
                let lh = f.line_height(26);
                let pill_y = y + (ROW_H - PILL_H as f32) / 2.0;
                let text_y = pill_y + (PILL_H as f32 - lh) / 2.0;
                let (desc, val) = if i == 0 {
                    let label = if cheat_dl.is_some() {
                        "Downloading..."
                    } else {
                        "Download cheats"
                    };
                    (label.to_string(), String::new())
                } else {
                    let (d, _, on) = &cheats[i - 1];
                    (d.clone(), (if *on { "On" } else { "Off" }).to_string())
                };
                let vw = f.measure(gl, &val, 26);
                if i == *sel {
                    let sel_c = theme_c1;
                    fe_draw_roll(
                        f, &renderer, gl, &mut roll_state, "cheats", &desc, 72.0, text_y,
                        pill_y, PILL_H as f32, 26, sel_c, sw as f32 * 0.62,
                    );
                    f.draw(&renderer, gl, &val, sw as f32 - 72.0 - vw, text_y, 26, sel_c);
                } else {
                    let shown = f.fit(gl, &desc, 26, sw as f32 * 0.62);
                    f.draw(&renderer, gl, &shown, 72.0, text_y, 26, theme_c4);
                    f.draw(&renderer, gl, &val, sw as f32 - 72.0 - vw, text_y, 26, theme_c4);
                }
            }
        }
        if let FeScreen::OptionsMenu { sel } = &screen
            && let Some(f) = font.as_mut()
        {
            f.draw(&renderer, gl, &stem, 24.0, 20.0, 24, [1.0, 1.0, 1.0, 1.0]);
            let top = 72.0;
            for (i, item) in opt_items.iter().enumerate() {
                let y = top + i as f32 * ROW_H;
                let lh = f.line_height(26);
                let pill_y = y + (ROW_H - PILL_H as f32) / 2.0;
                let text_y = pill_y + (PILL_H as f32 - lh) / 2.0;
                if i == *sel {
                    let sel_c = theme_c1;
                    f.draw(&renderer, gl, item, 72.0, text_y, 26, sel_c);
                } else {
                    f.draw(&renderer, gl, item, 72.0, text_y, 26, theme_c4);
                }
            }
        }
        if let FeScreen::Controls { sel } = &screen
            && let Some(f) = font.as_mut()
        {
            let n = PHYS.len() + 2;
            let top = (sh as f32 - n as f32 * ROW_H) / 2.0;
            for i2 in 0..n {
                let y = top + i2 as f32 * ROW_H;
                let lh = f.line_height(26);
                let pill_y = y + (ROW_H - PILL_H as f32) / 2.0;
                let text_y = pill_y + (PILL_H as f32 - lh) / 2.0;
                let (label, val) = if i2 == 0 {
                    (
                        "Dpad".to_string(),
                        if dpad_stick { "< Left Stick >" } else { "< Dpad >" }.to_string(),
                    )
                } else if i2 <= PHYS.len() {
                    (
                        PHYS[i2 - 1].1.to_string(),
                        BIND_CHOICES[bind_choice[i2 - 1]].1.to_string(),
                    )
                } else {
                    ("Restore defaults".to_string(), String::new())
                };
                let vw = f.measure(gl, &val, 26);
                if i2 == *sel {
                    let sel_c = theme_c1;
                    f.draw(&renderer, gl, &label, 128.0, text_y, 26, sel_c);
                    f.draw(&renderer, gl, &val, sw as f32 - 128.0 - vw, text_y, 26, sel_c);
                } else {
                    f.draw(&renderer, gl, &label, 128.0, text_y, 26, theme_c4);
                    f.draw(&renderer, gl, &val, sw as f32 - 128.0 - vw, text_y, 26, theme_c4);
                }
            }
        }
        if let FeScreen::Menu { sel } = &screen
            && let Some(f) = font.as_mut()
        {
            // chrome: rom name top-left (white), hints bottom (ids in c1)
            f.draw(&renderer, gl, &stem, 24.0, 20.0, 24, [1.0, 1.0, 1.0, 1.0]);
            {
                let mut hints: Vec<(&str, &str)> = Vec::new();
                if matches!(menu_items[*sel], "Save" | "Load") {
                    hints.push(("< / >", "Slot"));
                } else if menu_items[*sel] == "Continue" && core.disc_count() > 1 {
                    hints.push(("< / >", "Disc"));
                }
                hints.push(("A", "Okay"));
                hints.push(("B", "Back"));
                let size = 22u32;
                let mut wsum = 0.0;
                for (k, l) in &hints {
                    wsum += f.measure(gl, k, size) + 8.0 + f.measure(gl, l, size) + 28.0;
                }
                let mut hx = sw as f32 - 16.0 - (wsum - 28.0);
                let hy = sh as f32 - 44.0;
                for (k, l) in &hints {
                    f.draw(&renderer, gl, k, hx, hy, size, theme_c1);
                    hx += f.measure(gl, k, size) + 8.0;
                    f.draw(&renderer, gl, l, hx, hy, size, theme_c6);
                    hx += f.measure(gl, l, size) + 28.0;
                }
            }
            let top = (sh as f32 - menu_items.len() as f32 * ROW_H) / 2.0;
            for (i, item) in menu_items.iter().enumerate() {
                let y = top + i as f32 * ROW_H;
                let lh = f.line_height(MENU_FONT);
                let pill_y = y + (ROW_H - PILL_H as f32) / 2.0;
                let text_y = pill_y + (PILL_H as f32 - lh) / 2.0;
                let value: Option<String> = if matches!(*item, "Save" | "Load") {
                    Some(format!("{}", slot + 1))
                } else if *item == "Continue" && core.disc_count() > 1 {
                    Some(format!("Disc {}", core.disc_index() + 1))
                } else {
                    None
                };
                let base_c = if i == *sel { theme_c1 } else { theme_c4 };
                if let Some(val) = &value {
                    // label (row color) + accent-highlighted value
                    let base = format!("{item}  ");
                    let bw = f.measure(gl, &base, MENU_FONT);
                    f.draw(&renderer, gl, &base, 128.0, text_y, MENU_FONT, base_c);
                    f.draw(&renderer, gl, val, 128.0 + bw, text_y, MENU_FONT, theme_c2);
                } else {
                    f.draw(&renderer, gl, item, 128.0, text_y, MENU_FONT, base_c);
                }
            }
            // slot preview for Save/Load rows
            if matches!(menu_items[*sel], "Save" | "Load") {
                if slot_preview_for != slot {
                    if let Some(t) = slot_preview.take() {
                        renderer.drop_texture(gl, t);
                    }
                    slot_preview = kui_gfx::load_png(gl, &preview_path(slot)).ok();
                    slot_preview_for = slot;
                }
                if let Some(t) = &slot_preview {
                    let ph = sh as f32 * 0.35;
                    let pw2 = ph * (t.w as f32 / t.h as f32);
                    renderer.draw(
                        gl,
                        t,
                        sw as f32 - pw2 - 96.0,
                        (sh as f32 - ph) / 2.0,
                        pw2,
                        ph,
                        WHITE,
                    );
                    // 8-dot slot pagination under the preview
                    let rgba = |c: u32, a: f32| {
                        [
                            ((c >> 16) & 255) as f32 / 255.0,
                            ((c >> 8) & 255) as f32 / 255.0,
                            (c & 255) as f32 / 255.0,
                            a,
                        ]
                    };
                    let (dot, gap) = (8.0, 16.0);
                    let total = 8.0 * dot + 7.0 * gap;
                    let cx = sw as f32 - pw2 / 2.0 - 96.0;
                    let dy2 = (sh as f32 + ph) / 2.0 + 18.0;
                    for i in 0..8 {
                        let x = cx - total / 2.0 + i as f32 * (dot + gap);
                        let col = if i == slot { theme_c1 } else { rgba(0x808080, 0.6) };
                        renderer.rect(gl, x, dy2, dot, dot, col);
                    }
                } else if let Some(f2) = font.as_mut() {
                    // the truth comes from the STATE file, not the preview:
                    // legacy-made states have no PNG next to them
                    let msg = if state_path(slot).is_file() {
                        "Saved (no preview)"
                    } else {
                        "Empty slot"
                    };
                    let tw = f2.measure(gl, msg, 22);
                    f2.draw(
                        &renderer,
                        gl,
                        msg,
                        sw as f32 - 96.0 - tw,
                        sh as f32 / 2.0,
                        22,
                        [1.0, 1.0, 1.0, 0.7],
                    );
                }
            }
        }

        // toast notifications (pill doctrine)
        fps_frames += 1;
        if fps_t0.elapsed().as_millis() >= 1000 {
            fps_val = fps_frames as f32 * 1000.0 / fps_t0.elapsed().as_millis() as f32;
            fps_frames = 0;
            fps_t0 = Instant::now();
        }
        if hud
            && matches!(screen, FeScreen::Game)
            && let Some(f) = font.as_mut()
        {
            let line = format!(
                "{:.0}fps  {}x{}  {:?}",
                fps_val, tex_size.0, tex_size.1, scaling
            );
            let tw2 = f.measure(gl, &line, 18);
            f.draw(
                &renderer,
                gl,
                &line,
                sw as f32 - tw2 - 12.0,
                8.0,
                18,
                [1.0, 1.0, 1.0, 0.85],
            );
        }
        // two-pill toast: outer capsule in main, inner in accent, text in
        // the notification color (theme.color8, Appearance row)
        // ox < 0.0 means "center horizontally"
        let draw_toast = |f: &mut kui_gfx::text::Font, msg: &str, ox: f32, oy: f32| {
            let tw = f.measure(gl, msg, 24);
            let inner_w = tw + 40.0;
            let outer_w = inner_w + 16.0;
            let (ox, oy) = if ox < 0.0 { ((sw as f32 - outer_w) / 2.0, oy) } else { (ox, oy) };
            pill.draw(&renderer, gl, ox, oy, outer_w, theme_c1);
            pill_inner.draw(&renderer, gl, ox + 8.0, oy + 8.0, inner_w, theme_c2);
            let lh = f.line_height(24);
            f.draw(
                &renderer,
                gl,
                msg,
                ox + 8.0 + (inner_w - tw) / 2.0,
                oy + 8.0 + (36.0 - lh) / 2.0,
                24,
                theme_notif,
            );
        };
        // in-game menu chrome: tray + platform identifier (top-right),
        // mirroring the launcher's bare tray
        if !matches!(screen, FeScreen::Game)
            && let Some(f) = font.as_mut()
        {
            if tray_at.is_none_or(|t| t.elapsed().as_secs() >= 10) {
                tray_at = Some(Instant::now());
                tray_batt = std::fs::read_to_string("/tmp/percBat")
                    .ok()
                    .and_then(|s| s.trim().parse::<u8>().ok());
                tray_charging =
                    std::fs::read_to_string("/sys/class/power_supply/axp2202-usb/online")
                        .map(|s| s.trim() == "1")
                        .unwrap_or(false);
                let running = |name: &str| {
                    std::process::Command::new("sh")
                        .args(["-c", &format!("pidof {name} >/dev/null 2>&1")])
                        .status()
                        .map(|st| st.success())
                        .unwrap_or(false)
                };
                tray_wifi = running("wpa_supplicant");
                tray_bt = running("bluetoothd");
            }
            let cy = 16.0;
            let mut tray_w = 0.0;
            if let Some(sheet) = &assets_tex {
                let icon = 26.0;
                let batt_w = 34.0;
                let batt_h = 20.0;
                let gap = 14.0;
                let row_h = icon;
                let mut ix = sw as f32 - 16.0 - batt_w;
                let by = cy + (row_h - batt_h) / 2.0;
                let bx = ix;
                renderer.draw_uv(gl, sheet, ix, by, batt_w, batt_h, asset_uv(A_BATTERY), theme_c1);
                if show_batt_pct && let Some(pct) = tray_batt {
                    let txt = format!("{pct}%");
                    let pfont: u32 = 20;
                    let tw = f.measure(gl, &txt, pfont);
                    let tlh = f.line_height(pfont);
                    ix -= tw + 10.0;
                    f.draw(&renderer, gl, &txt, ix, cy + (row_h - tlh) / 2.0, pfont, theme_c1);
                }
                // charging shows the real level too (Arjun's call); the
                // bolt body is reserved for a confirmed full charge
                let charged = tray_charging && tray_batt.is_some_and(|p| p >= 100);
                let fill_frac = if charged {
                    None
                } else {
                    tray_batt.map(|pct| (pct as f32 / 100.0).clamp(0.0, 1.0))
                };
                if let Some(frac) = fill_frac {
                    let full = A_BATTERY_FILL;
                    let fill_rect =
                        (full.0 + full.2 * (1.0 - frac), full.1, full.2 * frac, full.3);
                    renderer.draw_uv(
                        gl,
                        sheet,
                        bx + 6.0 + 24.0 * (1.0 - frac),
                        by + 4.0,
                        24.0 * frac,
                        12.0,
                        asset_uv(fill_rect),
                        theme_c1,
                    );
                } else if charged {
                    // full body with bolt knockout
                    renderer.draw_uv(
                        gl, sheet, bx + 2.0, by, 32.0, 20.0, asset_uv(A_BATTERY_BOLT), theme_c1,
                    );
                }
                if tray_charging && !charged {
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
                        renderer.rect(
                            gl,
                            x0 + dx - 1.0,
                            y0 + dy - 1.0,
                            w + 2.0,
                            h + 2.0,
                            [0.0, 0.0, 0.0, 0.55],
                        );
                    }
                    for (dx, dy, w, h) in bolt {
                        renderer.rect(gl, x0 + dx, y0 + dy, w, h, [1.0, 1.0, 1.0, 1.0]);
                    }
                }
                if tray_bt {
                    ix -= icon + gap;
                    renderer.draw_uv(gl, sheet, ix, cy, icon, icon, asset_uv(A_BLUETOOTH), theme_c1);
                }
                if tray_wifi {
                    ix -= icon + gap;
                    renderer.draw_uv(gl, sheet, ix, cy, icon, icon, asset_uv(A_WIFI), theme_c1);
                }
                tray_w = sw as f32 - 16.0 - ix + 18.0;
            }
            let cfont: u32 = 24;
            let tlh = f.line_height(cfont);
            let text_cy = cy + (26.0 - tlh) / 2.0;
            let tw = f.measure(gl, &tag, cfont);
            let x = sw as f32 - 16.0 - tray_w - 36.0 - tw;
            f.draw(&renderer, gl, &tag, x, text_cy, cfont, theme_c1);
        }
        if let Some((msg, at)) = &ra_toast {
            if at.elapsed().as_millis() > toast_ms.max(2500) {
                ra_toast = None;
            } else if let Some(f) = font.as_mut() {
                draw_toast(f, msg, 16.0, 16.0);
            }
        }
        if let Some((msg, at)) = &toast {
            if at.elapsed().as_millis() > toast_ms {
                toast = None;
            } else if let Some(f) = font.as_mut() {
                draw_toast(f, msg, -1.0, sh as f32 - 110.0);
            }
        }
        // volume/brightness OSD: a notification pill with a fill bar —
        // the launcher's exact visual. kuid owns the value (live state in
        // /tmp/kui/); we poll it every frame so held-key repeats track live.
        if let Some((is_bright, at)) = osd {
            if at.elapsed().as_millis() > 1200 {
                osd = None;
            } else {
                let (path, key, def) = if is_bright {
                    ("/tmp/kui/bright", "display.brightness", 90)
                } else {
                    ("/tmp/kui/vol", "audio.volume", 40)
                };
                let val = std::fs::read_to_string(path)
                    .ok()
                    .and_then(|s| s.trim().parse::<i32>().ok())
                    .unwrap_or_else(|| cfg.get_i32(key, def))
                    .clamp(0, 100);
                let ow = 360.0;
                let oh = PILL_H as f32;
                let ox = (sw as f32 - ow) / 2.0;
                let oy = sh as f32 - 110.0;
                pill.draw(&renderer, gl, ox, oy, ow, theme_c2);
                if let Some(sheet) = &assets_tex {
                    let uv =
                        asset_uv(if is_bright { A_BRIGHTNESS } else { A_VOLUME });
                    renderer.draw_uv(
                        gl,
                        sheet,
                        ox + 20.0,
                        oy + (oh - 26.0) / 2.0,
                        26.0,
                        26.0,
                        uv,
                        [0.0, 0.0, 0.0, 1.0],
                    );
                }
                let bar_x = ox + 64.0;
                let bar_w = ow - 64.0 - 24.0;
                let bar_y = oy + oh / 2.0 - 5.0;
                renderer.rect(gl, bar_x, bar_y, bar_w, 10.0, [0.0, 0.0, 0.0, 0.15]);
                renderer.rect(gl, bar_x, bar_y, bar_w * (val as f32 / 100.0), 10.0, theme_c1);
            }
        }

        unsafe { gl.flush() };
        v.present();

        if matches!(screen, FeScreen::Game) && last_sram_flush.elapsed().as_secs() >= 30 {
            if let Some(sram) = core.sram() {
                let _ = write_save(&sav_path, sram, save_compress);
            }
            if let Some(rtc) = core.rtc() {
                let _ = std::fs::write(&rtc_path, rtc);
            }
            last_sram_flush = Instant::now();
        }
    }

    if let Some(sram) = core.sram() {
        let _ = write_save(&sav_path, sram, save_compress);
        println!("srm saved");
    }
    if let Some(rtc) = core.rtc() {
        let _ = std::fs::write(&rtc_path, rtc);
    }
    // last-session snapshot: what the game switcher shows for this game
    if last_frame.0 > 0 {
        let _ = kui_gfx::encode_png(
            &states_dir.join(format!("{stem}.session.png")),
            last_frame.0,
            last_frame.1,
            &last_frame.2,
        );
    }
    0
}

/// The 9-slice pill (duplicated minimally from the launcher's ui module —
/// a shared widget crate is due once a third consumer appears).
fn kui_frontend_pill(gl: &glow::Context) -> Result<SimplePill, String> {
    SimplePill::new(gl, PILL_H)
}

struct SimplePill {
    tex: Texture,
    radius: f32,
    tex_w: f32,
    tex_h: f32,
}

impl SimplePill {
    fn new(gl: &glow::Context, height: u32) -> Result<Self, String> {
        let r = 20u32.min(height / 2);
        let w = r * 2 + 2;
        let h = height;
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let fx = x as f32 + 0.5;
                let fy = y as f32 + 0.5;
                let cx = fx.clamp(r as f32, w as f32 - r as f32);
                let cy = fy.clamp(r as f32, h as f32 - r as f32);
                let d = ((fx - cx).powi(2) + (fy - cy).powi(2)).sqrt();
                let a = (r as f32 - d + 0.5).clamp(0.0, 1.0);
                let i = ((y * w + x) * 4) as usize;
                rgba[i..i + 4].copy_from_slice(&[255, 255, 255, (a * 255.0) as u8]);
            }
        }
        let tex = kui_gfx::texture_from_rgba(gl, w, h, &rgba)?;
        Ok(Self { tex, radius: r as f32, tex_w: w as f32, tex_h: h as f32 })
    }

    fn draw(&self, r: &Renderer, gl: &glow::Context, x: f32, y: f32, w: f32, tint: [f32; 4]) {
        let rad = self.radius;
        let u = rad / self.tex_w;
        let mid_u = 2.0 / self.tex_w;
        let h = self.tex_h;
        r.draw_uv(gl, &self.tex, x, y, rad, h, [0.0, 0.0, u, 1.0], tint);
        r.draw_uv(gl, &self.tex, x + rad, y, w - rad * 2.0, h, [u, 0.0, mid_u, 1.0], tint);
        r.draw_uv(gl, &self.tex, x + w - rad, y, rad, h, [u + mid_u, 0.0, u, 1.0], tint);
    }
}

/// The bezel that dresses a scaling mode; the "LCD Grid" variant when the
/// screen effect asks for it and the art exists. Fullscreen has no border.
fn bezel_path(dir: &Path, scaling: &Scaling, grid: bool) -> Option<std::path::PathBuf> {
    let base = match scaling {
        Scaling::Native => "Native",
        Scaling::Aspect => "Aspect",
        Scaling::AspectScreen | Scaling::Fullscreen | Scaling::Cropped => return None,
    };
    if grid {
        let p = dir.join(format!("{base} - LCD Grid.png"));
        if p.is_file() {
            return Some(p);
        }
    }
    let p = dir.join(format!("{base}.png"));
    p.is_file().then_some(p)
}

/// Extract the largest entry of a zip into /tmp and return its path.
/// Stored and deflate entries only; anything else is not a rom zip.
fn extract_zip(zip_path: &Path) -> Option<std::path::PathBuf> {
    let b = std::fs::read(zip_path).ok()?;
    let u16at = |p: usize| -> Option<usize> {
        Some(u16::from_le_bytes(b.get(p..p + 2)?.try_into().ok()?) as usize)
    };
    let u32at = |p: usize| -> Option<usize> {
        Some(u32::from_le_bytes(b.get(p..p + 4)?.try_into().ok()?) as usize)
    };
    let eocd = b.windows(4).rposition(|w| w == b"PK\x05\x06")?;
    let count = u16at(eocd + 10)?;
    let mut p = u32at(eocd + 16)?;
    let mut best: Option<(usize, usize)> = None; // (central record, uncompressed size)
    for _ in 0..count {
        if b.get(p..p + 4)? != b"PK\x01\x02" {
            break;
        }
        let usz = u32at(p + 24)?;
        if best.is_none_or(|(_, s)| usz > s) {
            best = Some((p, usz));
        }
        p += 46 + u16at(p + 28)? + u16at(p + 30)? + u16at(p + 32)?;
    }
    let (rec, _) = best?;
    let method = u16at(rec + 10)?;
    let csz = u32at(rec + 20)?;
    let name = String::from_utf8_lossy(b.get(rec + 46..rec + 46 + u16at(rec + 28)?)?).into_owned();
    let lho = u32at(rec + 42)?;
    let data_at = lho + 30 + u16at(lho + 26)? + u16at(lho + 28)?;
    let data = b.get(data_at..data_at + csz)?;
    let out = match method {
        0 => data.to_vec(),
        8 => miniz_oxide::inflate::decompress_to_vec(data).ok()?,
        _ => return None,
    };
    let dir = Path::new("/tmp/kui-zip");
    std::fs::create_dir_all(dir).ok()?;
    let out_path = dir.join(Path::new(&name).file_name()?);
    std::fs::write(&out_path, out).ok()?;
    Some(out_path)
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

/// Roll overflowing single-line text: pause, roll, pause; clock resets
/// when the text changes.
#[allow(clippy::too_many_arguments)]
fn fe_draw_roll(
    f: &mut kui_gfx::text::Font,
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
) {
    let now = Instant::now();
    let full = f.measure(gl, text, size);
    if full <= max_w {
        f.draw(r, gl, text, x, y_text, size, color);
        return;
    }
    let e = state
        .entry(slot.to_string())
        .or_insert_with(|| (text.to_string(), now));
    if e.0 != text {
        *e = (text.to_string(), now);
    }
    let overflow = full - max_w;
    let (speed, pause) = (60.0, 1.0);
    let cycle = pause + overflow / speed + pause;
    let t = (now - e.1).as_secs_f32() % cycle;
    let off = ((t - pause).max(0.0) * speed).min(overflow);
    r.scissor(gl, x, y_clip, max_w, h_clip);
    f.draw(r, gl, text, x - off, y_text, size, color);
    r.scissor_off(gl);
}

// ---------------------------------------------------------------- rzip
// RetroArch's rzip container ("#RZIPv1#"): 8-byte magic, u32 LE chunk
// size, u64 LE total uncompressed size, then u32 LE compressed length +
// zlib stream per chunk. Reads are always transparent (raw or rzip, so
// cards move freely between kUI and RetroArch devices); writes are
// opt-in via save.compress / state.compress.
const RZIP_MAGIC: &[u8; 8] = b"#RZIPv1#";
const RZIP_CHUNK: usize = 128 * 1024;

fn rzip_compress(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(20 + data.len() / 2);
    out.extend_from_slice(RZIP_MAGIC);
    out.extend_from_slice(&(RZIP_CHUNK as u32).to_le_bytes());
    out.extend_from_slice(&(data.len() as u64).to_le_bytes());
    for chunk in data.chunks(RZIP_CHUNK) {
        let z = miniz_oxide::deflate::compress_to_vec_zlib(chunk, 6);
        out.extend_from_slice(&(z.len() as u32).to_le_bytes());
        out.extend_from_slice(&z);
    }
    out
}

/// Raw bytes pass through untouched; a malformed container falls back to
/// the original bytes so a truncated header can never eat a save.
fn rzip_decompress(data: &[u8]) -> Vec<u8> {
    let Some(rest) = data.strip_prefix(RZIP_MAGIC) else {
        return data.to_vec();
    };
    if rest.len() < 12 {
        return data.to_vec();
    }
    let total = u64::from_le_bytes(rest[4..12].try_into().unwrap()) as usize;
    let mut out = Vec::with_capacity(total.min(64 * 1024 * 1024));
    let mut p = &rest[12..];
    while !p.is_empty() {
        if p.len() < 4 {
            return data.to_vec();
        }
        let n = u32::from_le_bytes(p[..4].try_into().unwrap()) as usize;
        p = &p[4..];
        if p.len() < n {
            return data.to_vec();
        }
        match miniz_oxide::inflate::decompress_to_vec_zlib(&p[..n]) {
            Ok(mut c) => out.append(&mut c),
            Err(_) => return data.to_vec(),
        }
        p = &p[n..];
    }
    out
}

/// One save write, honouring the rzip toggle for this file class.
fn write_save(path: &std::path::Path, bytes: &[u8], compress: bool) -> std::io::Result<()> {
    if compress { std::fs::write(path, rzip_compress(bytes)) } else { std::fs::write(path, bytes) }
}

/// Parse a RetroArch-format .cht: (desc, code, enabled) per cheat, with
/// the per-game config enable overriding the file's own flag.
fn parse_cht(
    text: &str,
    cfg: &kui_config::Config,
    tag: &str,
    stem: &str,
) -> Vec<(String, String, bool)> {
    let val = |k: &str| -> Option<String> {
        text.lines().find_map(|l| {
            let (lk, lv) = l.split_once('=')?;
            (lk.trim() == k).then(|| lv.trim().trim_matches('"').to_string())
        })
    };
    let n: usize = val("cheats").and_then(|v| v.parse().ok()).unwrap_or(0);
    let mut out = Vec::new();
    for i in 0..n.min(200) {
        let desc = val(&format!("cheat{i}_desc")).unwrap_or_else(|| format!("Cheat {i}"));
        let Some(code) = val(&format!("cheat{i}_code")) else {
            continue;
        };
        let file_on = val(&format!("cheat{i}_enable")).map(|v| v == "true").unwrap_or(false);
        let on = match cfg.get_or(&format!("game.{tag}.{stem}.cheat.{i}"), "") {
            "on" => true,
            "off" => false,
            _ => file_on,
        };
        out.push((desc, code, on));
    }
    out
}

/// Snapshot the RA achievement runtime beside a freshly written state, or
/// clear a stale sidecar when there is nothing to snapshot.
fn write_progress_sidecar(ra: &mut Option<kui_ra::RaClient>, path: &std::path::Path) {
    if let Some(c) = ra.as_mut() {
        match c.serialize_progress() {
            Some(p) => {
                let _ = std::fs::write(path, p);
            }
            None => {
                let _ = std::fs::remove_file(path);
            }
        }
    }
}
