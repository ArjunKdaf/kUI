//! FN Switch page helpers: pure value math and display text for the
//! fn.* config keys (CONTRACT). Values persist in kui.cfg only; kuid
//! applies them when the slider toggles.

/// "Unchanged" sentinel for the numeric overrides (CONTRACT: -69).
pub const NO_CHANGE: i32 = -69;

/// Config keys for the plain numeric FN rows.
pub const FN_NUM: [&str; 6] = [
    "fn.volume",
    "fn.brightness",
    "fn.colortemp",
    "fn.contrast",
    "fn.saturation",
    "fn.exposure",
];

/// Config keys for the turbo toggles (on/off).
pub const FN_TURBO: [&str; 8] = [
    "fn.turbo.a",
    "fn.turbo.b",
    "fn.turbo.x",
    "fn.turbo.y",
    "fn.turbo.l1",
    "fn.turbo.l2",
    "fn.turbo.r1",
    "fn.turbo.r2",
];

/// kUI default per numeric FN key. Arjun's stealth-switch defaults: FN
/// mutes and dims the screen to minimum; color settings stay untouched.
pub fn fn_num_default(key: &str) -> i32 {
    match key {
        "fn.volume" | "fn.brightness" => 0,
        _ => NO_CHANGE,
    }
}

/// kUI default for a toggle FN key ("on"/"off"): LEDs go dark by default.
pub fn fn_toggle_default(key: &str) -> &'static str {
    if key == "fn.leds" { "on" } else { "off" }
}

/// Value bounds per numeric row: (min, max) — NO_CHANGE sits below min.
pub fn fn_num_range(key: &str) -> (i32, i32) {
    match key {
        "fn.volume" => (0, 20),
        "fn.brightness" => (0, 10),
        "fn.colortemp" => (0, 40),
        "fn.contrast" => (-4, 5),
        "fn.saturation" => (-5, 5),
        "fn.exposure" => (-4, 5),
        _ => (0, 0),
    }
}

/// Display text for a numeric FN value.
pub fn fn_num_display(key: &str, v: i32) -> String {
    if v == NO_CHANGE {
        return "Unchanged".into();
    }
    match key {
        "fn.volume" => {
            if v == 0 {
                "Muted".into()
            } else {
                format!("{}%", v * 5)
            }
        }
        _ => v.to_string(),
    }
}
