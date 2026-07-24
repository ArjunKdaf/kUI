//! TrimUI Hammer / Brick class (Allwinner A133P, "tg5040").
//!
//! Hardware facts, device-verified 2026-07-18 via the M0 input echo on the
//! Hammer (renderer: PowerVR Rogue GE8300, OpenGL ES 3.2, 1024x768@60):
//!
//! - Pad enumerates as an Xbox-360-style SDL joystick.
//! - Face/system buttons are joystick buttons (indices below).
//! - D-pad arrives as SDL hat events.
//! - L2/R2 are full-swing analog axes (2 / 5).
//! - Power is a keyboard key (scancode 102); volume keys are joy buttons
//!   14 (+) / 13 (-) on Brick-class devices.
//! - No analog sticks on the Hammer; F1/F2 front keys report as 9/10.

use crate::{Button, ButtonState, InputEvent};

/// Allwinner disp2 ioctls (numbers from the behavioral spec).
const DISP_LCD_SET_BRIGHTNESS: libc::c_ulong = 0x102;
const DISP_LCD_SET_GAMMA_TABLE: libc::c_ulong = 0x10b;
const DISP_LCD_GAMMA_CORRECTION_ENABLE: libc::c_ulong = 0x10c;
const DISP_LCD_GAMMA_CORRECTION_DISABLE: libc::c_ulong = 0x10d;

/// Run a shell fragment, ignoring the result (device glue style).
fn sh(cmd: &str) {
    let _ = std::process::Command::new("sh").args(["-c", cmd]).status();
}

/// Panel backlight via /dev/disp ioctl; 0 = off, 1..=255 raw level.
pub fn set_raw_brightness(val: u32) {
    use std::os::fd::AsRawFd;
    if let Ok(f) = std::fs::OpenOptions::new().read(true).write(true).open("/dev/disp") {
        let param: [libc::c_ulong; 4] = [0, val as libc::c_ulong, 0, 0];
        unsafe {
            libc::ioctl(f.as_raw_fd(), DISP_LCD_SET_BRIGHTNESS, param.as_ptr());
        }
    }
}

/// Brick-class brightness curve (0-10 anchor points from the C settings
/// lib), interpolated over the 0-100 settings scale.
pub fn brightness_raw(value_0_100: i32) -> u32 {
    const CURVE: [u32; 11] = [1, 8, 16, 32, 48, 72, 96, 128, 160, 192, 255];
    let v = value_0_100.clamp(0, 100) as usize;
    let lo = v / 10;
    if lo >= 10 {
        return CURVE[10];
    }
    let frac = (v % 10) as u32;
    CURVE[lo] + (CURVE[lo + 1] - CURVE[lo]) * frac / 10
}

fn sysfs_write(path: &str, val: i32) {
    let _ = std::fs::write(path, val.to_string());
}

/// Color temperature: settings 0..=40, neutral 20 (raw -200..=200).
pub fn set_colortemp(value: i32) {
    sysfs_write(
        "/sys/class/disp/disp/attr/color_temperature",
        value.clamp(0, 40) * 10 - 200,
    );
}

/// Contrast: settings -4..=5, neutral 0 (raw 10..=100).
pub fn set_contrast(value: i32) {
    sysfs_write(
        "/sys/class/disp/disp/attr/enhance_contrast",
        50 + value.clamp(-4, 5) * 10,
    );
}

/// Saturation: settings -5..=5, neutral 0 (raw 0..=100).
pub fn set_saturation(value: i32) {
    sysfs_write(
        "/sys/class/disp/disp/attr/enhance_saturation",
        50 + value.clamp(-5, 5) * 10,
    );
}

/// Exposure: settings -4..=5, neutral 0 (raw 10..=100).
pub fn set_exposure(value: i32) {
    sysfs_write(
        "/sys/class/disp/disp/attr/enhance_bright",
        50 + value.clamp(-4, 5) * 10,
    );
}

/// Sound cards from /proc/asound/cards as (index, id) pairs.
pub fn sound_cards() -> Vec<(i32, String)> {
    let mut cards = Vec::new();
    if let Ok(text) = std::fs::read_to_string("/proc/asound/cards") {
        for line in text.lines() {
            // " 0 [audiocodec     ]: audiocodec - audiocodec"
            if let Some((num, rest)) = line.split_once('[')
                && let Ok(n) = num.trim().parse::<i32>()
                && let Some((id, _)) = rest.split_once(']')
            {
                cards.push((n, id.trim().to_string()));
            }
        }
    }
    cards
}

/// The internal codec's ALSA card index (spec: card named "audiocodec"
/// in /proc/asound/cards, else card 0).
fn audiocodec_card() -> i32 {
    sound_cards().into_iter().find(|(_, id)| id == "audiocodec").map(|(n, _)| n).unwrap_or(0)
}

/// Complete speaker/jack volume application on the internal codec:
/// - "digital volume" is INVERTED (0 = loudest): set to 100 - pct
/// - "DAC volume" companion: raw 160 while audible, 0 at zero
/// - /sys/class/speaker/mute = 1 at zero volume, else 0
///
/// Card is selected explicitly so a BT/USB .asoundrc default never
/// hijacks the controls.
pub fn set_volume_full(pct: i32) {
    let pct = pct.clamp(0, 100);
    let card = audiocodec_card();
    let inv = 100 - pct;
    let dac = if pct > 0 { 160 } else { 0 };
    sh(&format!(
        "amixer -c {card} sset 'digital volume' {inv}% >/dev/null 2>&1; \
         amixer -c {card} sset 'DAC volume' {dac} >/dev/null 2>&1"
    ));
    let _ = std::fs::write("/sys/class/speaker/mute", if pct == 0 { "1" } else { "0" });
}

/// Speaker volume 0..=100% (kept for existing callers).
pub fn set_volume_percent(pct: i32) {
    set_volume_full(pct);
}

/// One-time ALSA normalization at daemon startup: a bare `amixer` run
/// warms the alsa state, then Headphone 0, digital volume 0 (inverted:
/// full output) and DAC Swap Off (fixes swapped L/R channels).
pub fn audio_init_once() {
    sh("amixer >/dev/null 2>&1; \
        amixer sset 'Headphone' 0 >/dev/null 2>&1; \
        amixer sset 'digital volume' 0 >/dev/null 2>&1; \
        amixer sset 'DAC Swap' Off >/dev/null 2>&1");
}

fn srgb_to_linear(c: f64) -> f64 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn linear_to_srgb(l: f64) -> f64 {
    if l <= 0.0031308 { l * 12.92 } else { 1.055 * l.powf(1.0 / 2.4) - 0.055 }
}

/// Display calibration (white point) gamma LUT. Gains are 0..=200 with
/// 100 neutral. Each of the 256 entries is (r<<16)|(g<<8)|b where each
/// channel scales the sRGB-linearized ramp by gain/100.
///
/// Disable path loads the identity LUT first, then turns correction off.
pub fn apply_displaycal(enabled: bool, r: u32, g: u32, b: u32) {
    use std::os::fd::AsRawFd;
    let Ok(f) = std::fs::OpenOptions::new().read(true).write(true).open("/dev/disp") else {
        return;
    };
    let fd = f.as_raw_fd();
    let gains: [u32; 3] = if enabled { [r, g, b] } else { [100, 100, 100] };
    let mut lut = [0u32; 256];
    for (i, entry) in lut.iter_mut().enumerate() {
        let lin = srgb_to_linear(i as f64 / 255.0);
        let ch = |gain: u32| -> u32 {
            (linear_to_srgb(lin * gain as f64 / 100.0).clamp(0.0, 1.0) * 255.0).round() as u32
        };
        *entry = (ch(gains[0]) << 16) | (ch(gains[1]) << 8) | ch(gains[2]);
    }
    // ulong[4]: {screen 0, table pointer, byte size 1024, 0}
    let arg: [libc::c_ulong; 4] =
        [0, lut.as_ptr() as usize as libc::c_ulong, 1024, 0];
    unsafe {
        libc::ioctl(fd, DISP_LCD_SET_GAMMA_TABLE, arg.as_ptr());
    }
    let toggle: [libc::c_ulong; 4] = [0, 0, 0, 0];
    let cmd = if enabled {
        DISP_LCD_GAMMA_CORRECTION_ENABLE
    } else {
        DISP_LCD_GAMMA_CORRECTION_DISABLE
    };
    unsafe {
        libc::ioctl(fd, cmd, toggle.as_ptr());
    }
}

/// FN-slider rumble double-pulse: motor voltage 1500000, then gpio227
/// 1/0/1/0 at 100ms steps. Self-sufficient: exports the gpio and sets
/// its direction if the boot chain hasn't yet. Blocks ~300ms.
pub fn rumble_pulse() {
    let _ = std::fs::write("/sys/class/motor/voltage", "1500000");
    let gpio = "/sys/class/gpio/gpio227";
    if !std::path::Path::new(&format!("{gpio}/value")).exists() {
        let _ = std::fs::write("/sys/class/gpio/export", "227");
        let _ = std::fs::write(format!("{gpio}/direction"), "out");
    }
    for (i, v) in ["1", "0", "1", "0"].iter().enumerate() {
        if i > 0 {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        let _ = std::fs::write(format!("{gpio}/value"), v);
    }
}

/// RGB LED driver (/sys/class/led_anim). Channels: f1, f2, m (logo),
/// lr (both triggers), l, r. Hardware effects 0..=7:
/// disable, linear, breath, sniff, static, blink1, blink2, blink3.
pub mod leds {
    pub const CHANNELS: &[&str] = &["f1", "f2", "m", "lr"];
    pub const EFFECTS: &[(u8, &str)] = &[
        (0, "Off"),
        (1, "Linear"),
        (2, "Breathe"),
        (3, "Sniff"),
        (4, "Static"),
        (5, "Blink 1"),
        (6, "Blink 2"),
        (7, "Blink 3"),
    ];

    fn w(file: String, val: String) {
        let _ = std::fs::write(format!("/sys/class/led_anim/{file}"), val);
    }

    pub fn set_effect(ch: &str, effect: u8) {
        w(format!("effect_{ch}"), effect.to_string());
    }

    pub fn set_color(ch: &str, rgb: u32) {
        w(format!("effect_rgb_hex_{ch}"), format!("{rgb:06X}"));
    }

    pub fn set_duration(ch: &str, ms: i32) {
        w(format!("effect_duration_{ch}"), ms.to_string());
    }

    pub fn set_cycles(ch: &str, cycles: i32) {
        w(format!("effect_cycles_{ch}"), cycles.to_string());
    }

    /// Brightness groups: m -> max_scale, lr -> max_scale_lr,
    /// f1/f2 -> max_scale_f1f2. 0..=255 hardware scale.
    pub fn set_brightness(ch: &str, scale: i32) {
        let file = match ch {
            "lr" | "l" | "r" => "max_scale_lr",
            "f1" | "f2" => "max_scale_f1f2",
            _ => "max_scale",
        };
        w(file.to_string(), scale.to_string());
    }

    /// LED profiles as stored in the config (`led.<profile>.<light>.*`),
    /// shared by the editor (launcher) and the event engine (kuid).
    pub const PROFILES: &[(&str, &str)] = &[
        ("default", "Default"),
        ("lowbat", "Low Battery"),
        ("critical", "Critical Battery"),
        // key stays 'charging' so saved led.charging.* settings survive
        ("charging", "USB"),
        ("sleep", "Sleep"),
        ("gaming", "Gaming"),
    ];
    pub const LIGHTS: &[(&str, &str)] =
        &[("f1", "F1 key"), ("f2", "F2 key"), ("m", "Top"), ("lr", "Triggers")];

    pub fn profile_key(profile: usize, light: usize, field: &str) -> String {
        format!("led.{}.{}.{field}", PROFILES[profile].0, LIGHTS[light].0)
    }

    /// Per-profile defaults matching the classic kUI look: Default is white
    /// on Top/Triggers only; Low Battery amber 6; Critical red 3;
    /// Charging green 6; Sleep dark.
    pub fn profile_default(profile: usize, light: usize, field: &str) -> i64 {
        let name = PROFILES.get(profile).map(|(n, _)| *n).unwrap_or("default");
        match field {
            "effect" => 4, // static
            "duration" => 1000,
            "color" => match name {
                // amber, not lemon: RGB emitters render FFFF00 too green
                "lowbat" => 0xFFA000,
                "critical" => 0xFF0000,
                "default" => 0xFFFFFF, // Top + Triggers white by default
                _ => 0x00FF55,
            },
            "brightness" => match name {
                "default" => {
                    if LIGHTS[light].0 == "m" || LIGHTS[light].0 == "lr" { 6 } else { 0 }
                }
                // gaming: lights low by default — the game is the show
                "gaming" => 2,
                "lowbat" | "charging" => 6,
                "critical" => 3,
                _ => 0, // sleep
            },
            _ => 0,
        }
    }

    pub fn profile_get(
        cfg: &kui_config::Config,
        profile: usize,
        light: usize,
        field: &str,
    ) -> i64 {
        let key = profile_key(profile, light, field);
        cfg.get(&key)
            .and_then(|v| {
                if field == "color" {
                    u32::from_str_radix(v.trim_start_matches("0x"), 16)
                        .ok()
                        .map(|c| c as i64)
                } else {
                    v.parse().ok()
                }
            })
            .unwrap_or_else(|| profile_default(profile, light, field))
    }

    pub fn apply_light(cfg: &kui_config::Config, profile: usize, light: usize) {
        let ch = LIGHTS[light].0;
        // effect LAST: the driver latches color/duration on the effect write
        set_color(ch, profile_get(cfg, profile, light, "color") as u32);
        set_duration(ch, profile_get(cfg, profile, light, "duration") as i32);
        set_cycles(ch, -1);
        set_brightness(ch, profile_get(cfg, profile, light, "brightness") as i32);
        set_effect(ch, profile_get(cfg, profile, light, "effect") as u8);
    }

    pub fn apply_profile(cfg: &kui_config::Config, profile: usize) {
        for l in 0..LIGHTS.len() {
            apply_light(cfg, profile, l);
        }
    }
}

/// One suspend-to-RAM attempt; blocks until resume. Ok(()) means we went
/// down and came back. Err = the kernel refused (retry after a beat).
pub fn suspend_to_ram() -> std::io::Result<()> {
    std::fs::write("/sys/power/state", "mem")
}

/// Joystick button indices (Brick-class).
pub mod joy {
    pub const B: u8 = 0;
    pub const A: u8 = 1;
    pub const Y: u8 = 2;
    pub const X: u8 = 3;
    pub const L1: u8 = 4; // measured
    pub const R1: u8 = 5; // measured
    pub const SELECT: u8 = 6;
    pub const START: u8 = 7;
    pub const MENU: u8 = 8;
    pub const F1: u8 = 9;
    pub const F2: u8 = 10;
    pub const VOL_MINUS: u8 = 13; // measured
    pub const VOL_PLUS: u8 = 14; // measured
}

/// Analog axes.
pub mod axis {
    pub const L2: u8 = 2; // measured, full-swing on press
    pub const R2: u8 = 5; // measured, full-swing on press
}

/// Keyboard scancodes.
pub mod key {
    pub const POWER: i32 = 102; // measured (SDLK 1073741926)
}

const TRIGGER_THRESHOLD: i16 = 0;

/// Map a raw SDL event to an [`InputEvent`], if it concerns us.
pub fn map_event(ev: &sdl2::event::Event) -> Option<InputEvent> {
    use sdl2::event::Event;

    let btn = |b: Button, down: bool| {
        Some(InputEvent::Button(
            b,
            if down { ButtonState::Pressed } else { ButtonState::Released },
        ))
    };

    match ev {
        Event::JoyButtonDown { button_idx, .. } => joy_button(*button_idx).and_then(|b| btn(b, true)),
        Event::JoyButtonUp { button_idx, .. } => joy_button(*button_idx).and_then(|b| btn(b, false)),
        Event::JoyHatMotion { state, .. } => {
            use sdl2::joystick::HatState as H;
            let (up, down, left, right) = match state {
                H::Up => (true, false, false, false),
                H::Down => (false, true, false, false),
                H::Left => (false, false, true, false),
                H::Right => (false, false, false, true),
                H::LeftUp => (true, false, true, false),
                H::LeftDown => (false, true, true, false),
                H::RightUp => (true, false, false, true),
                H::RightDown => (false, true, false, true),
                H::Centered => (false, false, false, false),
            };
            Some(InputEvent::Dpad { up, down, left, right })
        }
        Event::JoyAxisMotion { axis_idx, value, .. } => match *axis_idx {
            a if a == axis::L2 => btn(Button::L2, *value > TRIGGER_THRESHOLD),
            a if a == axis::R2 => btn(Button::R2, *value > TRIGGER_THRESHOLD),
            _ => None,
        },
        Event::KeyDown { scancode: Some(sc), .. } if *sc as i32 == key::POWER => {
            btn(Button::Power, true)
        }
        Event::KeyUp { scancode: Some(sc), .. } if *sc as i32 == key::POWER => {
            btn(Button::Power, false)
        }
        _ => None,
    }
}

fn joy_button(idx: u8) -> Option<Button> {
    Some(match idx {
        joy::B => Button::B,
        joy::A => Button::A,
        joy::Y => Button::Y,
        joy::X => Button::X,
        joy::L1 => Button::L1,
        joy::R1 => Button::R1,
        joy::SELECT => Button::Select,
        joy::START => Button::Start,
        joy::MENU => Button::Menu,
        joy::F1 => Button::Fn1,
        joy::F2 => Button::Fn2,
        joy::VOL_PLUS => Button::VolUp,
        joy::VOL_MINUS => Button::VolDown,
        _ => return None,
    })
}

/// Live EV_SW switch states, read straight from evdev (SDL never
/// delivers switch events). Returns (fn_slider, headphone_jack);
/// None per slot if its device is missing.
///
/// On this hardware the FN slider is switch code 1 on the gamepad
/// device ("TRIMUI Player1") and the jack is SW_HEADPHONE_INSERT
/// (code 2) on the audiocodec device.
pub fn switch_states() -> (Option<bool>, Option<bool>) {
    const EVIOCGSW: libc::c_ulong = 0x8008451b; // _IOR('E', 0x1b, u64)
    const EVIOCGNAME: libc::c_ulong = 0x80404506; // _IOR('E', 0x06, [u8; 64])
    let mut fn_slider = None;
    let mut jack = None;
    for i in 0..8 {
        let Ok(f) = std::fs::File::open(format!("/dev/input/event{i}")) else {
            continue;
        };
        let fd = std::os::fd::AsRawFd::as_raw_fd(&f);
        let mut name = [0u8; 64];
        if unsafe { libc::ioctl(fd, EVIOCGNAME, name.as_mut_ptr()) } < 0 {
            continue;
        }
        let end = name.iter().position(|&b| b == 0).unwrap_or(name.len());
        let name = String::from_utf8_lossy(&name[..end]);
        let mut bits: u64 = 0;
        if unsafe { libc::ioctl(fd, EVIOCGSW, &mut bits) } < 0 {
            continue;
        }
        if name.contains("TRIMUI") {
            fn_slider = Some(bits & (1 << 1) != 0);
        } else if name.contains("Jack") {
            jack = Some(bits & (1 << 2) != 0);
        }
    }
    (fn_slider, jack)
}
