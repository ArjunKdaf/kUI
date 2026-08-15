//! RA hardcore: core options that RetroAchievements forbids.
//!
//! Data and matching semantics are a direct port of rcheevos'
//! `rc_libretro.c` (vendored at `crates/ra/vendor/rcheevos`, MIT) —
//! the same tables RetroArch enforces, so kUI blocks exactly what an
//! RA audit expects. Re-sync `TABLE` when the vendored rcheevos moves.
//!
//! Value-pattern language (from rc_libretro.c): a leading `,` starts a
//! CSV of tokens; a leading `!` inverts (explicitly allowed); a leading
//! `<` compares numerically; `?`/`*` wildcard, case-insensitive.

/// (setting key pattern, forbidden value pattern) — flattened across
/// cores; setting keys are globally unique per core family.
const TABLE: &[(&str, &str)] = &[
    // Beetle PSX (+HW)
    ("beetle_psx_cpu_freq_scale", "<100"),
    ("beetle_psx_hw_cpu_freq_scale", "<100"),
    // bsnes-mercury
    ("bsnes_region", "pal"),
    // cap32
    ("cap32_autorun", "disabled"),
    // dolphin
    ("dolphin_cheats_enabled", "enabled"),
    // DOSBox-pure
    ("dosbox_pure_strict_mode", "false"),
    // DuckStation
    ("duckstation_CDROM.LoadImagePatches", "true"),
    // ecwolf
    ("ecwolf-invulnerability", "enabled"),
    // FinalBurn Neo
    ("fbneo-allow-patched-romsets", "enabled"),
    ("fbneo-cheat-*", "!,Disabled,0 - Disabled"),
    ("fbneo-cpu-speed-adjust", "??%"),
    ("fbneo-dipswitch-*", "Universe BIOS*"),
    ("fbneo-neogeo-mode", "UNIBIOS"),
    // FCEUmm
    ("fceumm_game_genie", "!disabled"),
    ("fceumm_region", ",PAL,Dendy"),
    // Flycast
    ("reicast_sh4clock", "<200"),
    // Genesis Plus GX (+Wide)
    ("genesis_plus_gx_lock_on", ",action replay (pro),game genie"),
    ("genesis_plus_gx_region_detect", "pal"),
    ("genesis_plus_gx_wide_lock_on", ",action replay (pro),game genie"),
    ("genesis_plus_gx_wide_region_detect", "pal"),
    // Mesen / Mesen-S
    ("mesen_region", ",PAL,Dendy"),
    ("mesen-s_region", "PAL"),
    // NeoCD
    ("neocd_bios", "uni-bios*"),
    // PCSX-ReARMed
    ("pcsx_rearmed_psxclock", ",!auto,<55"),
    ("pcsx_rearmed_region", "pal"),
    // PicoDrive
    ("picodrive_region", ",Europe,Japan PAL"),
    // PPSSPP
    ("ppsspp_cheats", "enabled"),
    // QUASI88
    ("q88_cpu_clock", ",1,2"),
    // SMS Plus GX
    ("smsplus_region", "pal"),
    // Snes9x
    ("snes9x_gfx_clip", "disabled"),
    ("snes9x_gfx_transp", "disabled"),
    ("snes9x_layer_*", "disabled"),
    ("snes9x_region", "pal"),
    // SwanStation
    ("swanstation_CPU_Overclock", "<100"),
    // VICE x64
    ("vice_autostart", "disabled"),
    ("vice_reset", "!autostart"),
    // Virtual Jaguar
    ("virtualjaguar_pal", "enabled"),
];

/// Would RA hardcore forbid `key = value`? False for unknown keys.
pub fn disallowed(key: &str, value: &str) -> bool {
    TABLE
        .iter()
        .any(|(k, v)| eq_nocase_wildcard(key, k) && match_value(value, v))
}

/// rc_libretro_string_equal_nocase_wildcard: case-insensitive compare,
/// `?` matches any one char, `*` matches the rest from first mismatch.
fn eq_nocase_wildcard(test: &str, pat: &str) -> bool {
    let mut p = pat.bytes();
    for c1 in test.bytes() {
        match p.next() {
            Some(c2) => {
                if !c1.eq_ignore_ascii_case(&c2) && c2 != b'?' {
                    return c2 == b'*';
                }
            }
            None => return false,
        }
    }
    p.next().is_none()
}

/// C atoi: leading (signed) integer, 0 when none.
fn atoi(s: &str) -> i64 {
    let t = s.trim_start();
    let (neg, digits) = match t.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, t.strip_prefix('+').unwrap_or(t)),
    };
    let n: i64 = digits
        .bytes()
        .take_while(u8::is_ascii_digit)
        .fold(0i64, |a, d| a.saturating_mul(10).saturating_add((d - b'0') as i64));
    if neg { -n } else { n }
}

/// rc_libretro_match_token. `Some(r)` = token matched with verdict `r`
/// (true = forbidden, false = explicitly allowed); `None` = no match.
fn match_token(val: &str, token: &str) -> Option<bool> {
    if let Some(rest) = token.strip_prefix('!')
        && match_token(val, rest).is_some()
    {
        return Some(false);
    }
    if let Some(num) = token.strip_prefix('<')
        && atoi(val) < atoi(num)
    {
        return Some(true);
    }
    if val == token || eq_nocase_wildcard(val, token) {
        return Some(true);
    }
    None
}

/// rc_libretro_match_value: CSV / inverted / single-token forms.
fn match_value(val: &str, pattern: &str) -> bool {
    if let Some(rest) = pattern.strip_prefix(',') {
        for tok in rest.split(',') {
            if let Some(r) = match_token(val, tok) {
                return r;
            }
        }
        return false;
    }
    if let Some(rest) = pattern.strip_prefix('!') {
        return !match_value(val, rest);
    }
    match_token(val, pattern).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snes9x_rules() {
        assert!(disallowed("snes9x_layer_1", "disabled"));
        assert!(disallowed("SNES9X_LAYER_5", "Disabled"));
        assert!(!disallowed("snes9x_layer_1", "enabled"));
        assert!(disallowed("snes9x_region", "PAL"));
        assert!(!disallowed("snes9x_region", "auto"));
    }

    #[test]
    fn numeric_and_csv() {
        assert!(disallowed("beetle_psx_cpu_freq_scale", "50%"));
        assert!(!disallowed("beetle_psx_cpu_freq_scale", "100%"));
        assert!(disallowed("pcsx_rearmed_psxclock", "54"));
        assert!(!disallowed("pcsx_rearmed_psxclock", "auto"));
        assert!(disallowed("fceumm_region", "Dendy"));
        assert!(!disallowed("fceumm_region", "NTSC"));
    }

    #[test]
    fn inverted_and_wildcard() {
        assert!(disallowed("fceumm_game_genie", "enabled"));
        assert!(!disallowed("fceumm_game_genie", "disabled"));
        assert!(disallowed("fbneo-cheat-42", "Enabled"));
        assert!(!disallowed("fbneo-cheat-42", "Disabled"));
        assert!(disallowed("fbneo-cpu-speed-adjust", "50%"));
        assert!(!disallowed("fbneo-cpu-speed-adjust", "100%"));
        assert!(disallowed("neocd_bios", "Uni-BIOS 4.0"));
        assert!(!disallowed("unknown_option", "whatever"));
    }
}
