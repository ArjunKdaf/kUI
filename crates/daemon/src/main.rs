//! kuid: the kUI system daemon — the ONE background process.
//!
//! Jobs:
//! - LED event engine: battery/charger/session transitions -> profiles.
//! - Battery: minute sampler to battlog, /tmp/percBat on change.
//! - Radio boot enforcement (wifi/bt per kui.cfg).
//! - Global keys: VOL+/- with MENU (brightness) / SELECT (colortemp)
//!   chords, 300ms/100ms repeat, 60Hz evdev loop with suspend guard.
//! - FN slider cascade (overrides + trimui_inputd flag files + rumble).
//! - Headphone jack volume-slot switching.
//! - BT/USB audio routing (.asoundrc + sink + volume re-apply).
//! - Boot apply: audio init, volume, panel values, DisplayCal, FN.
//!
//! UIs never apply hardware state for these; they render from /tmp/kui/.

use std::collections::BTreeMap;
use std::io::Read as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::time::{Duration, Instant};

const SHARED_DIR: &str = "/mnt/SDCARD/.userdata/shared";
const USERDATA_DIR: &str = "/mnt/SDCARD/.userdata/tg5040";
const TMP_DIR: &str = "/tmp/kui";
const PERC_BAT: &str = "/tmp/percBat";
const INPUTD_DIR: &str = "/tmp/trimui_inputd";

const BATT: &str = "/sys/class/power_supply/axp2202-battery/capacity";
const STATUS: &str = "/sys/class/power_supply/axp2202-battery/status";

/// "Unchanged" sentinel for fn.* override values.
const NO_CHANGE: i32 = -69;

// profile indices per hal::tg5040::leds::PROFILES
const P_DEFAULT: usize = 0;
const P_LOWBAT: usize = 1;
const P_CRITICAL: usize = 2;
const P_CHARGING: usize = 3;
const P_SLEEP: usize = 4;
const P_GAMING: usize = 5;

static RUNNING: AtomicBool = AtomicBool::new(true);
/// 0 default / 1 bluetooth / 2 usbdac (routing thread writes).
static SINK: AtomicI32 = AtomicI32::new(0);
/// ALSA card index of the USB DAC while SINK == 2.
static USB_CARD: AtomicI32 = AtomicI32::new(0);
/// Headphone jack plugged (key thread writes from EV_SW 2).
static JACK: AtomicBool = AtomicBool::new(false);
/// Routing thread asks the key thread to re-apply volume on its slot.
static REAPPLY: AtomicBool = AtomicBool::new(false);
/// FN slider position, published by input_thread for the LED engine.
static FN_ON: AtomicBool = AtomicBool::new(false);
/// FN flipped: LED engine must re-evaluate now, not at the 30s reassert.
static LED_SYNC: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(_sig: libc::c_int) {
    RUNNING.store(false, Ordering::SeqCst);
}

fn load_cfg() -> kui_config::Config {
    kui_config::Config::load(Path::new(SHARED_DIR))
}

fn sh(cmd: &str) {
    let _ = std::process::Command::new("sh").args(["-c", cmd]).status();
}

fn sh_out(cmd: &str) -> String {
    std::process::Command::new("sh")
        .args(["-c", cmd])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

fn read_i32(path: &str) -> Option<i32> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

/// Live-state file under /tmp/kui/ (ascii, write + flush).
fn write_state(name: &str, val: impl std::fmt::Display) {
    let _ = std::fs::create_dir_all(TMP_DIR);
    if let Ok(mut f) = std::fs::File::create(format!("{TMP_DIR}/{name}")) {
        let _ = write!(f, "{val}");
        let _ = f.flush();
    }
}

// ---------------------------------------------------------------- settings

/// Snapshot of every kui.cfg value the key/FN engine applies.
#[derive(Clone, Copy)]
struct Vals {
    vol_spk: i32,    // percent, speaker slot
    vol_hp: i32,     // percent, headphone/BT/USB slot
    bright: i32,     // percent
    colortemp: i32,  // 0..40, neutral 20
    contrast: i32,   // -4..5
    saturation: i32, // -5..5
    exposure: i32,   // -4..5
    // fn.* overrides in each setting's UI scale, NO_CHANGE = untouched
    fn_vol: i32,    // 0..20 (x5)
    fn_bright: i32, // 0..10 (x10)
    fn_ct: i32,     // 0..40
    fn_con: i32,
    fn_sat: i32,
    fn_exp: i32,
    cal_enabled: bool,
    cal_r: i32,
    cal_g: i32,
    cal_b: i32,
}

impl Vals {
    fn from_cfg(cfg: &kui_config::Config) -> Self {
        Vals {
            vol_spk: cfg.get_i32("audio.volume", 40).clamp(0, 100),
            vol_hp: cfg.get_i32("audio.volume_hp", 40).clamp(0, 100),
            bright: cfg.get_i32("display.brightness", 90).clamp(0, 100),
            colortemp: cfg.get_i32("display.colortemp", 20).clamp(0, 40),
            contrast: cfg.get_i32("display.contrast", 0).clamp(-4, 5),
            saturation: cfg.get_i32("display.saturation", 0).clamp(-5, 5),
            exposure: cfg.get_i32("display.exposure", 0).clamp(-4, 5),
            // FN defaults: stealth switch — muted, screen dimmed to
            // minimum (and LEDs dark, consumed by the LED engine);
            // color settings Unchanged
            fn_vol: cfg.get_i32("fn.volume", 0),
            fn_bright: cfg.get_i32("fn.brightness", 0),
            fn_ct: cfg.get_i32("fn.colortemp", NO_CHANGE),
            fn_con: cfg.get_i32("fn.contrast", NO_CHANGE),
            fn_sat: cfg.get_i32("fn.saturation", NO_CHANGE),
            fn_exp: cfg.get_i32("fn.exposure", NO_CHANGE),
            // Brick panel calibration defaults when unset
            cal_enabled: cfg.get_or("cal.enabled", "on") == "on",
            cal_r: cfg.get_i32("cal.gain.r", 100).clamp(0, 200),
            cal_g: cfg.get_i32("cal.gain.g", 92).clamp(0, 200),
            cal_b: cfg.get_i32("cal.gain.b", 58).clamp(0, 200),
        }
    }

    fn effective_volume(&self, fn_on: bool) -> i32 {
        if fn_on && self.fn_vol != NO_CHANGE {
            return (self.fn_vol * 5).clamp(0, 100);
        }
        if hp_slot_active() { self.vol_hp } else { self.vol_spk }
    }

    fn effective_bright(&self, fn_on: bool) -> i32 {
        if fn_on && self.fn_bright != NO_CHANGE {
            (self.fn_bright * 10).clamp(0, 100)
        } else {
            self.bright
        }
    }

    fn effective_colortemp(&self, fn_on: bool) -> i32 {
        if fn_on && self.fn_ct != NO_CHANGE { self.fn_ct.clamp(0, 40) } else { self.colortemp }
    }
}

/// Headphone slot is active when the jack is in OR a non-default sink is.
fn hp_slot_active() -> bool {
    JACK.load(Ordering::SeqCst) || SINK.load(Ordering::SeqCst) != 0
}

// ---------------------------------------------------------------- apply

/// Route the effective volume to whatever owns audio right now.
fn apply_volume_hw(pct: i32) {
    let pct = pct.clamp(0, 100);
    match SINK.load(Ordering::SeqCst) {
        // BT: first simple ctl containing "A2DP" (ctl.!default is
        // bluealsa via our .asoundrc), mapped scale
        1 => sh(&format!(
            "c=$(amixer scontrols 2>/dev/null | sed -n \"s/.*'\\(.*A2DP.*\\)'.*/\\1/p\" | head -n1); \
             [ -n \"$c\" ] && amixer sset \"$c\" -M {pct}% >/dev/null 2>&1"
        )),
        // USB DAC: first (PCM|Playback)*[Vv]olume ctl on the DAC card,
        // linear min..max mapping (plain % = raw range)
        2 => {
            let card = USB_CARD.load(Ordering::SeqCst).max(0);
            sh(&format!(
                "n=$(amixer -c {card} controls 2>/dev/null | sed -n \"s/.*name='\\([^']*\\)'.*/\\1/p\" | grep -E '(PCM|Playback).*[Vv]olume' | head -n1); \
                 [ -n \"$n\" ] && amixer -c {card} cset \"name=$n\" {pct}% >/dev/null 2>&1"
            ));
        }
        _ => kui_hal::tg5040::set_volume_full(pct),
    }
    write_state("vol", pct);
}

fn apply_volume(v: &Vals, fn_on: bool) {
    apply_volume_hw(v.effective_volume(fn_on));
}

fn apply_brightness(v: &Vals, fn_on: bool) {
    let pct = v.effective_bright(fn_on);
    kui_hal::tg5040::set_raw_brightness(kui_hal::tg5040::brightness_raw(pct));
    write_state("bright", pct);
}

fn apply_colortemp(v: &Vals, fn_on: bool) {
    kui_hal::tg5040::set_colortemp(v.effective_colortemp(fn_on));
}

/// (config key, stock trimui_inputd flag file) — note L1/R1 map to the
/// unsuffixed turbo_l / turbo_r names.
const TURBO_FLAGS: [(&str, &str); 8] = [
    ("fn.turbo.a", "turbo_a"),
    ("fn.turbo.b", "turbo_b"),
    ("fn.turbo.x", "turbo_x"),
    ("fn.turbo.y", "turbo_y"),
    ("fn.turbo.l1", "turbo_l"),
    ("fn.turbo.l2", "turbo_l2"),
    ("fn.turbo.r1", "turbo_r"),
    ("fn.turbo.r2", "turbo_r2"),
];

/// Empty flag files consumed by the STOCK trimui_inputd: exists = on.
/// FN off removes everything.
fn apply_fn_flags(cfg: &kui_config::Config, fn_on: bool) {
    let _ = std::fs::create_dir_all(INPUTD_DIR);
    let set = |file: &str, on: bool| {
        let p = format!("{INPUTD_DIR}/{file}");
        if on {
            let _ = std::fs::write(&p, "");
        } else {
            let _ = std::fs::remove_file(&p);
        }
    };
    for (key, file) in TURBO_FLAGS {
        set(file, fn_on && cfg.get_or(key, "off") == "on");
    }
    set("input_no_dpad", fn_on && cfg.get_or("fn.dpad_disable", "off") == "on");
    set("input_dpad_to_joystick", fn_on && cfg.get_or("fn.joystick", "off") == "on");
}

/// The full FN/boot cascade: every applied setting with FN substitution,
/// then DisplayCal, then the inputd flag files.
fn apply_cascade(cfg: &kui_config::Config, v: &Vals, fn_on: bool) {
    apply_volume(v, fn_on);
    apply_brightness(v, fn_on);
    apply_colortemp(v, fn_on);
    kui_hal::tg5040::set_contrast(if fn_on && v.fn_con != NO_CHANGE {
        v.fn_con
    } else {
        v.contrast
    });
    kui_hal::tg5040::set_saturation(if fn_on && v.fn_sat != NO_CHANGE {
        v.fn_sat
    } else {
        v.saturation
    });
    kui_hal::tg5040::set_exposure(if fn_on && v.fn_exp != NO_CHANGE {
        v.fn_exp
    } else {
        v.exposure
    });
    kui_hal::tg5040::apply_displaycal(
        v.cal_enabled,
        v.cal_r as u32,
        v.cal_g as u32,
        v.cal_b as u32,
    );
    apply_fn_flags(cfg, fn_on);
}

// ---------------------------------------------------------------- keys

const EV_KEY: u16 = 1;
const EV_SW: u16 = 5;
const KEY_VOL_DOWN: u16 = 114;
const KEY_VOL_UP: u16 = 115;
const BTN_SELECT: u16 = 314; // colortemp chord
const BTN_MODE: u16 = 316; // MENU, brightness chord (wins over SELECT)
const SW_JACK: u16 = 2;

/// struct input_event on 64-bit: timeval (16) + type u16 + code u16 + value i32.
const EV_SIZE: usize = 24;

/// Drain one nonblocking evdev fd into (type, code, value) triples.
fn drain_events(f: &mut std::fs::File, out: &mut Vec<(u16, u16, i32)>) {
    let mut buf = [0u8; EV_SIZE * 64];
    loop {
        match f.read(&mut buf) {
            Ok(n) if n >= EV_SIZE => {
                for ev in buf[..n - n % EV_SIZE].chunks_exact(EV_SIZE) {
                    out.push((
                        u16::from_ne_bytes([ev[16], ev[17]]),
                        u16::from_ne_bytes([ev[18], ev[19]]),
                        i32::from_ne_bytes([ev[20], ev[21], ev[22], ev[23]]),
                    ));
                }
            }
            _ => break,
        }
    }
}

/// Load-modify-save so we never clobber keys other writers (settings
/// pages) changed since our snapshot.
fn flush_dirty(dirty: &mut BTreeMap<&'static str, i32>) {
    if dirty.is_empty() {
        return;
    }
    let mut cfg = load_cfg();
    for (k, v) in dirty.iter() {
        cfg.set(k, v);
    }
    let _ = cfg.save();
    dirty.clear();
}

/// One key action: MENU held = brightness, SELECT held = colortemp,
/// neither = volume on the active slot. Applies + marks dirty.
fn fire(
    code: u16,
    menu: bool,
    select: bool,
    v: &mut Vals,
    fn_on: bool,
    dirty: &mut BTreeMap<&'static str, i32>,
) {
    let dir: i32 = if code == KEY_VOL_UP { 1 } else { -1 };
    if menu {
        v.bright = (v.bright + dir * 10).clamp(0, 100);
        apply_brightness(v, fn_on);
        dirty.insert("display.brightness", v.bright);
    } else if select {
        v.colortemp = (v.colortemp + dir).clamp(0, 40);
        apply_colortemp(v, fn_on);
        dirty.insert("display.colortemp", v.colortemp);
    } else {
        let hp = hp_slot_active();
        let cur = if hp { v.vol_hp } else { v.vol_spk };
        let next = (cur + dir * 5).clamp(0, 100);
        if hp {
            v.vol_hp = next;
            dirty.insert("audio.volume_hp", next);
        } else {
            v.vol_spk = next;
            dirty.insert("audio.volume", next);
        }
        apply_volume(v, fn_on);
    }
}

/// The 60Hz global input engine: volume/brightness/colortemp keys with
/// press + 300ms/100ms repeat, jack slot switching, FN slider cascade
/// (200ms subpoll), config persistence debounced 1s.
fn input_thread() {
    // ---- boot apply -----------------------------------------------
    kui_hal::tg5040::audio_init_once();
    let (fn0, jack0) = kui_hal::tg5040::switch_states();
    JACK.store(jack0.unwrap_or(false), Ordering::SeqCst);
    let mut fn_on = fn0.unwrap_or(false);
    FN_ON.store(fn_on, Ordering::SeqCst);
    // the LED loop may have won the race with its first apply
    LED_SYNC.store(true, Ordering::SeqCst);
    let boot_cfg = load_cfg();
    let mut v = Vals::from_cfg(&boot_cfg);
    write_state("fn", if fn_on { "1" } else { "0" });
    write_state("jack", if JACK.load(Ordering::SeqCst) { "1" } else { "0" });
    write_state("sink", 0);
    // covers both slider positions: fn_on substitutes the overrides
    apply_cascade(&boot_cfg, &v, fn_on);
    drop(boot_cfg);
    println!("kuid: boot apply done (fn={} jack={})", fn_on, JACK.load(Ordering::SeqCst));

    // ---- evdev ----------------------------------------------------
    use std::os::unix::fs::OpenOptionsExt as _;
    let mut devs: Vec<std::fs::File> = (0..5)
        .filter_map(|i| {
            std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NONBLOCK)
                .open(format!("/dev/input/event{i}"))
                .ok()
        })
        .collect();
    println!("kuid: key loop on {} input devices", devs.len());

    let mut menu_down = false;
    let mut select_down = false;
    let mut rep_up: Option<Instant> = None; // next scheduled fire while held
    let mut rep_down: Option<Instant> = None;
    let mut dirty: BTreeMap<&'static str, i32> = BTreeMap::new();
    let mut last_change: Option<Instant> = None;
    let mut last_tick = Instant::now();
    let mut ticks: u64 = 0;
    let mut events: Vec<(u16, u16, i32)> = Vec::new();

    while RUNNING.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_micros(16_666));
        ticks += 1;
        let now = Instant::now();
        let stale = now.duration_since(last_tick) > Duration::from_millis(1000);
        last_tick = now;

        events.clear();
        for f in &mut devs {
            drain_events(f, &mut events);
        }

        // Suspend guard: a >1s tick gap means we slept — the queued
        // events are ghosts. Drop them and every held/repeat state.
        if stale {
            menu_down = false;
            select_down = false;
            rep_up = None;
            rep_down = None;
            continue;
        }

        for &(ty, code, value) in &events {
            match (ty, code) {
                (EV_KEY, BTN_MODE) if value != 2 => menu_down = value != 0,
                (EV_KEY, BTN_SELECT) if value != 2 => select_down = value != 0,
                (EV_KEY, KEY_VOL_UP) | (EV_KEY, KEY_VOL_DOWN) => {
                    let rep = if code == KEY_VOL_UP { &mut rep_up } else { &mut rep_down };
                    match value {
                        1 => {
                            // fresh press: sync with on-disk cfg unless we
                            // hold newer unsaved values ourselves
                            if dirty.is_empty() {
                                v = Vals::from_cfg(&load_cfg());
                            }
                            fire(code, menu_down, select_down, &mut v, fn_on, &mut dirty);
                            last_change = Some(now);
                            *rep = Some(now + Duration::from_millis(300));
                        }
                        0 => *rep = None,
                        _ => {} // kernel autorepeat: our own timing rules
                    }
                }
                (EV_SW, SW_JACK) => {
                    let plugged = value != 0;
                    JACK.store(plugged, Ordering::SeqCst);
                    write_state("jack", if plugged { "1" } else { "0" });
                    flush_dirty(&mut dirty);
                    v = Vals::from_cfg(&load_cfg());
                    apply_volume(&v, fn_on); // slot switched; levels only
                    println!("kuid: jack {}", if plugged { "in" } else { "out" });
                }
                _ => {}
            }
        }

        // repeats: first at +300ms, then every +100ms from scheduled time
        for (code, rep) in [(KEY_VOL_UP, &mut rep_up), (KEY_VOL_DOWN, &mut rep_down)] {
            if let Some(t) = *rep
                && now >= t
            {
                fire(code, menu_down, select_down, &mut v, fn_on, &mut dirty);
                last_change = Some(now);
                *rep = Some(t + Duration::from_millis(100));
            }
        }

        // FN slider subpoll every 12 ticks (~200ms)
        if ticks.is_multiple_of(12)
            && let (Some(f), _) = kui_hal::tg5040::switch_states()
            && f != fn_on
        {
            fn_on = f;
            FN_ON.store(f, Ordering::SeqCst);
            LED_SYNC.store(true, Ordering::SeqCst);
            write_state("fn", if f { "1" } else { "0" });
            if f {
                // double-pulse on the way IN only; off-thread so the
                // 300ms of sleeps never stall key timing
                std::thread::spawn(kui_hal::tg5040::rumble_pulse);
            }
            flush_dirty(&mut dirty);
            let cfg = load_cfg();
            v = Vals::from_cfg(&cfg);
            apply_cascade(&cfg, &v, fn_on);
            println!("kuid: fn {}", if f { "on" } else { "off" });
        }

        // routing thread changed the sink: re-apply on the active slot
        if REAPPLY.swap(false, Ordering::SeqCst) {
            flush_dirty(&mut dirty);
            v = Vals::from_cfg(&load_cfg());
            apply_volume(&v, fn_on);
        }

        // debounced persistence: save 1s after the last key change
        if let Some(t) = last_change
            && now.duration_since(t) >= Duration::from_secs(1)
        {
            flush_dirty(&mut dirty);
            last_change = None;
        }
    }
    flush_dirty(&mut dirty);
}

// ---------------------------------------------------------------- routing

/// First connected BT device advertising the A2DP sink UUID.
fn detect_bt_a2dp() -> Option<String> {
    let list = sh_out("bluetoothctl devices Connected 2>/dev/null || bluetoothctl devices 2>/dev/null");
    for line in list.lines() {
        let mut it = line.split_whitespace();
        if it.next() != Some("Device") {
            continue;
        }
        let Some(mac) = it.next() else { continue };
        if !mac.contains(':') {
            continue;
        }
        let info = sh_out(&format!("bluetoothctl info {mac} 2>/dev/null"));
        if info.contains("Connected: yes") && info.contains("0000110b-") {
            return Some(mac.to_string());
        }
    }
    None
}

/// First non-audiocodec ALSA card = a USB DAC.
fn detect_usb_dac() -> Option<i32> {
    kui_hal::tg5040::sound_cards()
        .into_iter()
        .find(|(_, id)| id != "audiocodec")
        .map(|(n, _)| n)
}

/// Durable write for .asoundrc: fsync the file AND its directory.
fn write_sync(path: &Path, content: &str) {
    if let Ok(mut f) = std::fs::File::create(path) {
        let _ = f.write_all(content.as_bytes());
        let _ = f.sync_all();
    }
    if let Some(dir) = path.parent()
        && let Ok(d) = std::fs::File::open(dir)
    {
        let _ = d.sync_all();
    }
}

/// audiomon replacement: every 2s look for a BT A2DP device or a USB
/// DAC, own $USERDATA/.asoundrc + /tmp/kui/sink, and have the key
/// thread re-apply volume on the now-active slot. BT outranks USB.
fn routing_thread() {
    let asoundrc = PathBuf::from(USERDATA_DIR).join(".asoundrc");
    // start fresh: stale routing from a previous boot must not linger
    let _ = std::fs::remove_file(&asoundrc);
    let mut cur: i32 = 0;
    while RUNNING.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_secs(2));
        let bt = detect_bt_a2dp();
        let usb = if bt.is_none() { detect_usb_dac() } else { None };
        let want = if bt.is_some() {
            1
        } else if usb.is_some() {
            2
        } else {
            0
        };
        if want == cur {
            continue;
        }
        match want {
            1 => {
                let mac = bt.unwrap();
                write_sync(
                    &asoundrc,
                    &format!(
                        "defaults.bluealsa.device \"{mac}\"\n\
                         pcm.!default {{ type plug slave.pcm {{ type bluealsa device \"{mac}\" profile \"a2dp\" delay 0 }} }}\n\
                         ctl.!default {{ type bluealsa }}\n"
                    ),
                );
            }
            2 => {
                let card = usb.unwrap();
                USB_CARD.store(card, Ordering::SeqCst);
                write_sync(
                    &asoundrc,
                    &format!(
                        "pcm.!default {{ type hw card {card} }}\n\
                         ctl.!default {{ type hw card {card} }}\n"
                    ),
                );
            }
            _ => {
                let _ = std::fs::remove_file(&asoundrc);
                if let Ok(d) = std::fs::File::open(USERDATA_DIR) {
                    let _ = d.sync_all();
                }
            }
        }
        SINK.store(want, Ordering::SeqCst);
        write_state("sink", want);
        REAPPLY.store(true, Ordering::SeqCst); // key thread re-applies volume
        println!("kuid: audio sink -> {want}");
        cur = want;
    }
}

// ---------------------------------------------------------------- battery/leds

fn charging() -> bool {
    // USB presence, not battery state: "Full" at 100% is still plugged
    std::fs::read_to_string("/sys/class/power_supply/axp2202-usb/online")
        .map(|s| s.trim() == "1")
        .unwrap_or(false)
}

fn pick_profile() -> usize {
    // battery emergencies outrank everything; gameplay outranks the rest
    let batt = read_i32(BATT);
    if let Some(p) = batt
        && p <= 5
    {
        return P_CRITICAL;
    }
    if std::path::Path::new("/tmp/kui_session").exists() {
        return P_GAMING;
    }
    if charging() {
        return P_CHARGING;
    }
    match batt {
        Some(p) if p <= 10 => P_LOWBAT,
        _ => P_DEFAULT,
    }
}

/// /tmp/percBat: ascii int, no newline, flushed + synced; on change only.
fn write_percbat(pct: i32) {
    if let Ok(mut f) = std::fs::File::create(PERC_BAT) {
        let _ = write!(f, "{pct}");
        let _ = f.flush();
        let _ = f.sync_all();
    }
}

fn main() {
    let handler = on_signal as extern "C" fn(libc::c_int);
    unsafe {
        libc::signal(libc::SIGTERM, handler as libc::sighandler_t);
        libc::signal(libc::SIGINT, handler as libc::sighandler_t);
    }

    let shared = PathBuf::from(SHARED_DIR);
    let mut current = usize::MAX;
    let mut ticks: u32 = 0;
    println!("kuid {} up", env!("CARGO_PKG_VERSION"));
    let _ = std::fs::create_dir_all(TMP_DIR);

    let battlog = shared.join("kui/battlog.txt");
    // prune once per boot so the log never grows unbounded
    if let Ok(text) = std::fs::read_to_string(&battlog) {
        let lines: Vec<&str> = text.lines().collect();
        if lines.len() > 20000 {
            let _ = std::fs::write(&battlog, lines[lines.len() - 20000..].join("\n") + "\n");
        }
    }
    // boot radio state: off unless the user left it on
    {
        let cfg = kui_config::Config::load(&shared);
        for (key, script) in [
            ("radio.wifi", "/etc/wifi/wifi_init.sh"),
            ("radio.bluetooth", "/etc/bluetooth/bt_init.sh"),
        ] {
            let verb = if cfg.get_or(key, "off") == "on" { "start" } else { "stop" };
            let _ = std::process::Command::new("sh")
                .args(["-c", &format!("{script} {verb} >/dev/null 2>&1 &")])
                .status();
        }
    }

    // battery percent: once at startup, then on change; gone on exit
    let mut last_batt = read_i32(BATT);
    if let Some(p) = last_batt {
        write_percbat(p);
    }

    // the other two engines; they exit when RUNNING drops
    std::thread::spawn(input_thread);
    std::thread::spawn(routing_thread);

    while RUNNING.load(Ordering::SeqCst) {
        let want = pick_profile();
        // reassert every ~30s even without a transition: the launcher
        // touches the LEDs around sleep/editor use and we own the truth
        ticks += 1;
        if want != current || ticks.is_multiple_of(10) || LED_SYNC.swap(false, Ordering::SeqCst) {
            // reload config each transition: cheap, and picks up editor changes
            let cfg = kui_config::Config::load(&shared);
            // "FN switch disables LED": the user's explicit lights-off
            // outranks every profile, battery emergencies included
            let eff = if FN_ON.load(Ordering::SeqCst) && cfg.get_or("fn.leds", "on") == "on" {
                P_SLEEP
            } else {
                want
            };
            kui_hal::tg5040::leds::apply_profile(&cfg, eff);
            println!("kuid: profile -> {}", kui_hal::tg5040::leds::PROFILES[eff].0);
            // publish for the launcher (LED editor exit restores this)
            let _ = std::fs::write("/tmp/kui_profile", eff.to_string());
            current = want;
        }
        // /tmp/percBat on change — or when the file vanished: our
        // startup write can land on a tmpfs that stock init then mounts
        // over (batmon used to mask this by rewriting constantly)
        let batt = read_i32(BATT);
        if let Some(p) = batt
            && (batt != last_batt || !std::path::Path::new(PERC_BAT).exists())
        {
            write_percbat(p);
            last_batt = batt;
        }
        // battery sample once a minute: epoch, capacity %, charging flag
        if ticks.is_multiple_of(20)
            && let (Some(pct), Ok(now)) = (
                batt,
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH),
            )
        {
            let charging = std::fs::read_to_string(STATUS)
                .map(|s| s.trim() == "Charging")
                .unwrap_or(false);
            if let Some(dir) = battlog.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Ok(mut f) =
                std::fs::OpenOptions::new().append(true).create(true).open(&battlog)
            {
                let _ = writeln!(f, "{}\t{}\t{}", now.as_secs(), pct, charging as u8);
            }
        }
        // 3s cycle in short hops so SIGTERM lands within ~250ms
        for _ in 0..12 {
            if !RUNNING.load(Ordering::SeqCst) {
                break;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }

    // clean exit: consumers must not read a stale battery percent
    let _ = std::fs::remove_file(PERC_BAT);
    println!("kuid: clean exit");
}
