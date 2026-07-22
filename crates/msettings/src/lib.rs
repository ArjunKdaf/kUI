//! Clean-room `libmsettings.so` — the compat shim third-party paks link
//! against. The legacy library owned volume/brightness/colortemp via a
//! shared-memory settings struct; kUI owns them through kuid, kui.cfg,
//! and the `/tmp/kui` live-state files. This lib is a thin bridge: the
//! 16 C entry points paks actually import (established by ABI audit),
//! nothing more.
//!
//! Reads report kUI's live state (scaled to the legacy 0-20 / 0-10 / 0-40
//! ranges paks expect). Writes apply hardware exactly as kuid does and
//! keep kui.cfg + `/tmp/kui` coherent, so a value a pak changes survives
//! the pak exiting and the launcher's OSD shows the truth.
//!
//! No shared-memory handshake, no msettings.bin: no pak binary touches
//! the legacy struct directly (audit-confirmed) — only these calls.

use std::os::raw::c_int;
use std::path::Path;

const SHARED_DIR: &str = "/mnt/SDCARD/.userdata/shared";
const LIVE_DIR: &str = "/tmp/kui";

// ---- kUI state access -------------------------------------------------

fn cfg() -> kui_config::Config {
    kui_config::Config::load(Path::new(SHARED_DIR))
}

/// Read a `/tmp/kui/<name>` live value, else the cfg key, else default.
/// kuid writes live state every apply; it's the freshest source.
fn live_or_cfg(name: &str, key: &str, default: i32) -> i32 {
    if let Ok(s) = std::fs::read_to_string(format!("{LIVE_DIR}/{name}"))
        && let Ok(v) = s.trim().parse::<i32>()
    {
        return v;
    }
    cfg().get_i32(key, default)
}

fn write_live(name: &str, value: i32) {
    let _ = std::fs::create_dir_all(LIVE_DIR);
    let _ = std::fs::write(format!("{LIVE_DIR}/{name}"), value.to_string());
}

/// Persist a cfg key so the value outlives the pak and kuid restores it.
fn persist(key: &str, value: i32) {
    let mut c = cfg();
    c.set(key, value);
    let _ = c.save();
}

// round-tripping between kUI's 0-100 scale and the legacy pak scales
fn to_steps(pct: i32, step: i32) -> i32 {
    ((pct + step / 2) / step).clamp(0, 100 / step)
}

// ---- lifecycle (2 syms) ----------------------------------------------

/// Legacy: mmap the shared settings, reset mixers. kUI's settings are
/// already live under kuid; nothing to set up. Kept so paks that guard
/// on it succeed.
#[unsafe(no_mangle)]
pub extern "C" fn InitSettings() {}

#[unsafe(no_mangle)]
pub extern "C" fn QuitSettings() {}

/// Init never fails here (no shm to miss); paks guard launch on this.
#[unsafe(no_mangle)]
pub extern "C" fn InitializedSettings() -> c_int {
    1
}

// ---- getters ----------------------------------------------------------

/// Brightness, legacy 0-10 scale (kUI stores 0-100).
#[unsafe(no_mangle)]
pub extern "C" fn GetBrightness() -> c_int {
    to_steps(live_or_cfg("bright", "display.brightness", 90), 10)
}

/// Volume, legacy 0-20 scale (kUI stores 0-100).
#[unsafe(no_mangle)]
pub extern "C" fn GetVolume() -> c_int {
    to_steps(live_or_cfg("vol", "audio.volume", 40), 5)
}

/// Color temperature, 0-40 — same scale kUI uses.
#[unsafe(no_mangle)]
pub extern "C" fn GetColortemp() -> c_int {
    live_or_cfg("colortemp", "display.colortemp", 20).clamp(0, 40)
}

/// Mute flag: kUI has no separate mute — volume 0 is muted.
#[unsafe(no_mangle)]
pub extern "C" fn GetMute() -> c_int {
    (live_or_cfg("vol", "audio.volume", 40) == 0) as c_int
}

/// No HDMI on this hardware.
#[unsafe(no_mangle)]
pub extern "C" fn GetHDMI() -> c_int {
    0
}

/// Audio sink: 0 speaker, 1 bluetooth, 2 USB DAC (kuid publishes it).
#[unsafe(no_mangle)]
pub extern "C" fn GetAudioSink() -> c_int {
    live_or_cfg("sink", "audio.sink", 0).clamp(0, 2)
}

// The "muted <x>" overrides: kUI has no per-mute override, so report the
// legacy "no change" sentinels — paps treat these as "leave as-is".
const NO_CHANGE: c_int = -69;

#[unsafe(no_mangle)]
pub extern "C" fn GetMutedBrightness() -> c_int {
    NO_CHANGE
}

#[unsafe(no_mangle)]
pub extern "C" fn GetMutedColortemp() -> c_int {
    NO_CHANGE
}

#[unsafe(no_mangle)]
pub extern "C" fn GetMutedVolume() -> c_int {
    0 // 0 = fully mute on toggle, the legacy default
}

// ---- setters ----------------------------------------------------------

/// Volume, legacy 0-20. Applies hardware (sink-aware, via kuid's HAL),
/// updates live state + cfg so the value sticks and the OSD is honest.
#[unsafe(no_mangle)]
pub extern "C" fn SetVolume(value: c_int) {
    let pct = (value.clamp(0, 20) * 5).clamp(0, 100);
    SetRawVolume(pct);
    write_live("vol", pct);
    persist("audio.volume", pct);
}

/// Volume 0-100, hardware only, no persistence (legacy contract).
#[unsafe(no_mangle)]
pub extern "C" fn SetRawVolume(value: c_int) {
    kui_hal::tg5040::set_volume_full(value.clamp(0, 100));
}

/// Brightness, legacy 0-10. Hardware + live state + cfg.
#[unsafe(no_mangle)]
pub extern "C" fn SetBrightness(value: c_int) {
    let pct = (value.clamp(0, 10) * 10).clamp(0, 100);
    kui_hal::tg5040::set_raw_brightness(kui_hal::tg5040::brightness_raw(pct));
    write_live("bright", pct);
    persist("display.brightness", pct);
}

/// Brightness 0-255 raw, hardware only, no persistence (legacy contract).
#[unsafe(no_mangle)]
pub extern "C" fn SetRawBrightness(value: c_int) {
    kui_hal::tg5040::set_raw_brightness(value.clamp(0, 255) as u32);
}
