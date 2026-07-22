//! Multi-call pak-compat shim (busybox-style): behaves as `nextval`,
//! `syncsettings.elf`, or `gametimectl.elf` depending on argv[0]. These
//! legacy helper CLIs are gone from kUI's own stack, but third-party
//! paks still invoke them by name — the cleanup migration installs this
//! binary under each of those names so those paks keep working.
//!
//! Scope is exactly what the pak audit found in use: nextval reads
//! settings, syncsettings re-applies hardware, gametimectl is a no-op
//! (the launcher tracks play time itself now).

use std::path::Path;

const SHARED_DIR: &str = "/mnt/SDCARD/.userdata/shared";

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let name = Path::new(&args[0])
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();
    let code = match name.as_str() {
        "nextval" => nextval(args.get(1).map(String::as_str)),
        "syncsettings" => syncsettings(),
        // playtime is tracked natively; accept every verb, do nothing
        "gametimectl" => 0,
        other => {
            eprintln!("kui-shim: unknown applet '{other}'");
            2
        }
    };
    std::process::exit(code);
}

fn cfg() -> kui_config::Config {
    kui_config::Config::load(Path::new(SHARED_DIR))
}

/// `nextval <key>` — emit `{"<key>": <value>}` (or `{}`) for the legacy
/// settings key a pak asks for, served from kUI's kui.cfg. Paks that use
/// this (theme readers) fall back to a default when they get `{}`, so an
/// unmapped key is safe.
fn nextval(key: Option<&str>) -> i32 {
    let Some(key) = key else {
        // no arg: legacy dumps everything; nobody parses that, emit {}
        println!("{{}}");
        return 0;
    };
    let c = cfg();
    // colorN -> theme.colorN, re-adding the 0x the legacy format carried
    if let Some(n) = key.strip_prefix("color")
        && n.chars().all(|ch| ch.is_ascii_digit())
        && !n.is_empty()
    {
        match c.get(&format!("theme.color{n}")) {
            Some(hex) => println!("{{\"{key}\": 0x{hex}}}"),
            None => println!("{{}}"),
        }
        return 0;
    }
    // on/off booleans the legacy store expressed as 1/0
    let bool_key = match key {
        "wifi" => Some("radio.wifi"),
        "bluetooth" => Some("radio.bluetooth"),
        "muteLeds" => Some("fn.leds"),
        _ => None,
    };
    if let Some(ck) = bool_key {
        let v = if c.get_or(ck, "off") == "on" { 1 } else { 0 };
        println!("{{\"{key}\": {v}}}");
        return 0;
    }
    // font/fontStyle and anything else: pass through kui.cfg if present
    let mapped = match key {
        "font" => "theme.font",
        "fontStyle" => "theme.font_style",
        other => other,
    };
    match c.get(mapped) {
        Some(v) => println!("{{\"{key}\": {v}}}"),
        None => println!("{{}}"),
    }
    0
}

/// Push the stored display/audio settings back to hardware — what paks
/// called after they poked a mixer or the panel. kuid normally owns
/// this; run standalone here so a pak invocation works while kuid idles.
fn syncsettings() -> i32 {
    let c = cfg();
    kui_hal::tg5040::set_volume_full(c.get_i32("audio.volume", 40));
    let b = c.get_i32("display.brightness", 90);
    kui_hal::tg5040::set_raw_brightness(kui_hal::tg5040::brightness_raw(b));
    kui_hal::tg5040::set_colortemp(c.get_i32("display.colortemp", 20));
    kui_hal::tg5040::set_contrast(c.get_i32("display.contrast", 0));
    kui_hal::tg5040::set_saturation(c.get_i32("display.saturation", 0));
    kui_hal::tg5040::set_exposure(c.get_i32("display.exposure", 0));
    0
}
