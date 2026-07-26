//! The settings hub: native, in-process, schema-driven pages editing the
//! one config store. Tools live here too (SPEC: unified hub) — installed
//! pak listing arrives with the store work.

/// kUI display version (k-series). Single source: the VERSION file at
/// the repo root; the Cargo semver mirrors it as 0.9.x.
pub const KUI_VERSION: &str = include_str!("../../../VERSION");

/// A choice option: (stored value, display label).
pub type Choice = (&'static str, &'static str);

pub enum ItemKind {
    /// Cycle through fixed choices with left/right.
    Choice(&'static [Choice]),
    /// Integer range with left/right stepping.
    Range { min: i32, max: i32, step: i32 },
    /// Read-only info line.
    Info(String),
    /// Runtime-managed item (radios etc) — the main loop owns behavior.
    External,
    /// Press A to run (main owns the behavior, keyed by `key`).
    Action,
}

pub struct Item {
    pub label: &'static str,
    pub key: &'static str,
    pub desc: &'static str,
    pub kind: ItemKind,
}

pub struct Page {
    pub title: &'static str,
    pub desc: &'static str,
    pub items: Vec<Item>,
}

pub fn pages(device_name: &str, stock_ver: &str, busybox_ver: &str) -> Vec<Page> {
    vec![
        Page {
            title: "Appearance",
            desc: "How kUI looks",
            items: vec![
                Item {
                    label: "UI Mode",
                    key: "ui.mode",
                    desc: "Carousel: the full experience. Covers: lists with art. Lists: text only.",
                    kind: ItemKind::Choice(&[
                        ("carousel", "Carousel"),
                        ("covers", "Covers"),
                        ("lists", "Lists"),
                    ]),
                },
                Item {
                    label: "Font",
                    key: "theme.font",
                    desc: "The interface font. Applied immediately.",
                    kind: ItemKind::External,
                },
                Item {
                    label: "Font style",
                    key: "theme.font_style",
                    desc: "Applied immediately.",
                    kind: ItemKind::Choice(&[("normal", "Normal"), ("bold", "Bold")]),
                },
                Item {
                    label: "Main Color",
                    key: "theme.color1",
                    desc: "Identifiers, cursors, logos. Press A to edit.",
                    kind: ItemKind::External,
                },
                Item {
                    label: "Accent Color",
                    key: "theme.color2",
                    desc: "Values and the pin marker. Press A to edit.",
                    kind: ItemKind::External,
                },
                Item {
                    label: "List Text",
                    key: "theme.color4",
                    desc: "Unselected rows. Press A to edit.",
                    kind: ItemKind::External,
                },
                Item {
                    label: "Hint Color",
                    key: "theme.color6",
                    desc: "Bottom hints, descriptions, headers. Press A to edit.",
                    kind: ItemKind::External,
                },
                Item {
                    label: "Notification Text",
                    key: "theme.color8",
                    desc: "In-game notification text. Press A to edit.",
                    kind: ItemKind::External,
                },
                Item {
                    label: "Background",
                    key: "theme.color7",
                    desc: "Screen color when no art is shown. Press A to edit.",
                    kind: ItemKind::External,
                },
                Item {
                    label: "Battery Percentage",
                    key: "ui.battery_percent",
                    desc: "Show the number next to the battery icon.",
                    kind: ItemKind::Choice(&[("off", "Off"), ("on", "On")]),
                },
                Item {
                    label: "Reset to defaults",
                    key: "reset.theme",
                    desc: "Press A to restore the default look.",
                    kind: ItemKind::Action,
                },
            ],
        },
        Page {
            title: "Display",
            desc: "Panel settings",
            items: vec![
                Item {
                    label: "Brightness",
                    key: "display.brightness",
                    desc: "Applied live while you adjust.",
                    kind: ItemKind::Range { min: 0, max: 100, step: 5 },
                },
                Item {
                    label: "Color Temperature",
                    key: "display.colortemp",
                    desc: "20 is neutral. Lower is warmer, higher is cooler.",
                    kind: ItemKind::Range { min: 0, max: 40, step: 1 },
                },
                Item {
                    label: "Contrast",
                    key: "display.contrast",
                    desc: "0 is neutral.",
                    kind: ItemKind::Range { min: -4, max: 5, step: 1 },
                },
                Item {
                    label: "Saturation",
                    key: "display.saturation",
                    desc: "0 is neutral.",
                    kind: ItemKind::Range { min: -5, max: 5, step: 1 },
                },
                Item {
                    label: "Exposure",
                    key: "display.exposure",
                    desc: "0 is neutral.",
                    kind: ItemKind::Range { min: -4, max: 5, step: 1 },
                },
                Item {
                    label: "White point correction",
                    key: "cal.enabled",
                    desc: "Match the sRGB white point, costing some brightness.",
                    kind: ItemKind::External,
                },
                Item {
                    label: "Red gain",
                    key: "cal.gain.r",
                    desc: "White point red channel (0-200).",
                    kind: ItemKind::External,
                },
                Item {
                    label: "Green gain",
                    key: "cal.gain.g",
                    desc: "White point green channel (0-200).",
                    kind: ItemKind::External,
                },
                Item {
                    label: "Blue gain",
                    key: "cal.gain.b",
                    desc: "White point blue channel (0-200).",
                    kind: ItemKind::External,
                },
                Item {
                    label: "Reset to defaults",
                    key: "reset.display",
                    desc: "Press A to restore neutral panel settings.",
                    kind: ItemKind::Action,
                },
            ],
        },
        Page {
            title: "Connectivity",
            desc: "WiFi and Bluetooth",
            items: vec![
                Item {
                    label: "WiFi",
                    key: "radio.wifi",
                    desc: "Press A to scan and connect.",
                    kind: ItemKind::External,
                },
                Item {
                    label: "Bluetooth",
                    key: "radio.bluetooth",
                    desc: "Press A to scan and pair.",
                    kind: ItemKind::External,
                },
            ],
        },
        Page {
            title: "Boot Logo",
            desc: "The image shown at power on",
            items: vec![],
        },
        Page {
            title: "Themes",
            desc: "Artbook carousel variants",
            items: vec![],
        },
        Page {
            title: "FN Switch",
            desc: "What the FN slider does while toggled",
            items: vec![
                Item {
                    label: "Volume when toggled",
                    key: "fn.volume",
                    desc: "Speaker volume while the switch is on.",
                    kind: ItemKind::External,
                },
                Item {
                    label: "FN switch disables LED",
                    key: "fn.leds",
                    desc: "Lights go dark while toggled.",
                    kind: ItemKind::External,
                },
                Item {
                    label: "Brightness when toggled",
                    key: "fn.brightness",
                    desc: "0-10, or Unchanged.",
                    kind: ItemKind::External,
                },
                Item {
                    label: "Color temperature when toggled",
                    key: "fn.colortemp",
                    desc: "0-40, or Unchanged.",
                    kind: ItemKind::External,
                },
                Item {
                    label: "Contrast when toggled",
                    key: "fn.contrast",
                    desc: "-4 to 5, or Unchanged.",
                    kind: ItemKind::External,
                },
                Item {
                    label: "Saturation when toggled",
                    key: "fn.saturation",
                    desc: "-5 to 5, or Unchanged.",
                    kind: ItemKind::External,
                },
                Item {
                    label: "Exposure when toggled",
                    key: "fn.exposure",
                    desc: "-4 to 5, or Unchanged.",
                    kind: ItemKind::External,
                },
                Item {
                    label: "Turbo A",
                    key: "fn.turbo.a",
                    desc: "Autofire on A while toggled.",
                    kind: ItemKind::External,
                },
                Item {
                    label: "Turbo B",
                    key: "fn.turbo.b",
                    desc: "Autofire on B while toggled.",
                    kind: ItemKind::External,
                },
                Item {
                    label: "Turbo X",
                    key: "fn.turbo.x",
                    desc: "Autofire on X while toggled.",
                    kind: ItemKind::External,
                },
                Item {
                    label: "Turbo Y",
                    key: "fn.turbo.y",
                    desc: "Autofire on Y while toggled.",
                    kind: ItemKind::External,
                },
                Item {
                    label: "Turbo L1",
                    key: "fn.turbo.l1",
                    desc: "Autofire on L1 while toggled.",
                    kind: ItemKind::External,
                },
                Item {
                    label: "Turbo L2",
                    key: "fn.turbo.l2",
                    desc: "Autofire on L2 while toggled.",
                    kind: ItemKind::External,
                },
                Item {
                    label: "Turbo R1",
                    key: "fn.turbo.r1",
                    desc: "Autofire on R1 while toggled.",
                    kind: ItemKind::External,
                },
                Item {
                    label: "Turbo R2",
                    key: "fn.turbo.r2",
                    desc: "Autofire on R2 while toggled.",
                    kind: ItemKind::External,
                },
                Item {
                    label: "Dpad mode when toggled",
                    key: "fn.dpad",
                    desc: "Dpad, Joystick (dpad acts as stick), or Both.",
                    kind: ItemKind::External,
                },
                Item {
                    label: "Reset to defaults",
                    key: "reset.fn",
                    desc: "Everything back to Unchanged and Off.",
                    kind: ItemKind::Action,
                },
            ],
        },
        Page {
            title: "PakDek",
            desc: "Browse and install community paks",
            items: vec![],
        },
        Page {
            title: "Updater",
            desc: "kUI over-the-air updates",
            items: vec![],
        },
        Page {
            title: "Scraper",
            desc: "Boxart and metadata: download and patch",
            items: vec![],
        },
        Page {
            title: "Input",
            desc: "Button and switch tester",
            items: vec![],
        },
        Page {
            title: "Files",
            desc: "Browse the SD card",
            items: vec![],
        },
        Page {
            title: "In-Game",
            desc: "Notifications while playing",
            items: vec![
                Item {
                    label: "Notify on save",
                    key: "notify.save",
                    desc: "Toast when a state is saved.",
                    kind: ItemKind::Choice(&[("on", "On"), ("off", "Off")]),
                },
                Item {
                    label: "Notify on load",
                    key: "notify.load",
                    desc: "Toast when a state is loaded.",
                    kind: ItemKind::Choice(&[("on", "On"), ("off", "Off")]),
                },
                Item {
                    label: "Notify on screenshot",
                    key: "notify.screenshot",
                    desc: "Toast when a screenshot is taken.",
                    kind: ItemKind::Choice(&[("on", "On"), ("off", "Off")]),
                },
                Item {
                    label: "RetroAchievements",
                    key: "ra.enabled",
                    desc: "Earn achievements while playing (needs an account).",
                    kind: ItemKind::Choice(&[("off", "Off"), ("on", "On")]),
                },
                Item {
                    label: "RA Username",
                    key: "ra.user",
                    desc: "Press A to enter your RetroAchievements username.",
                    kind: ItemKind::External,
                },
                Item {
                    label: "RA Password",
                    key: "ra.pass",
                    desc: "Press A to enter the password. Stored on this card only.",
                    kind: ItemKind::External,
                },
                Item {
                    label: "Authenticate",
                    key: "ra.auth",
                    desc: "Press A to log in and store the session token (needs WiFi).",
                    kind: ItemKind::Action,
                },
                Item {
                    label: "RA PreFetch",
                    key: "ra.prefetch",
                    desc: "Press A to cache every game's achievement data for offline play. A again cancels.",
                    kind: ItemKind::Action,
                },
                // RA Hardcore stays UI-hidden until RetroAchievements
                // approves kUI as a client (unlocks are server-demoted to
                // softcore until then). The ra.hardcore config key is live
                // for testing; restore the Item here once approval lands.
                Item {
                    label: "Notification duration",
                    key: "notify.duration",
                    desc: "Seconds a toast stays on screen.",
                    kind: ItemKind::Range { min: 1, max: 5, step: 1 },
                },
            ],
        },
        Page {
            title: "Battery",
            desc: "Charge history and projection",
            items: vec![],
        },
        Page {
            title: "Game Tracker",
            desc: "Play activity per game",
            items: vec![],
        },
        Page {
            title: "Core Options",
            desc: "Default emulator settings per core",
            items: vec![],
        },
        Page {
            title: "LED Control",
            desc: "Lights, colors and effects",
            items: vec![],
        },
        Page {
            title: "System",
            desc: "Performance and power",
            items: vec![Item {
                label: "Volume",
                key: "audio.volume",
                desc: "Speaker volume.",
                kind: ItemKind::Range { min: 0, max: 100, step: 5 },
            },
            Item {
                label: "Headphone Volume",
                key: "audio.volume_hp",
                desc: "Level when headphones or Bluetooth audio are active.",
                kind: ItemKind::Range { min: 0, max: 100, step: 5 },
            },
            Item {
                label: "Auto Sleep",
                key: "power.auto_sleep_min",
                desc: "Deep sleep after this many idle minutes. 0 disables.",
                kind: ItemKind::Range { min: 0, max: 30, step: 1 },
            },
            Item {
                label: "Safe Poweroff",
                key: "power.safe_poweroff",
                desc: "Disconnect the battery cleanly at shutdown.",
                kind: ItemKind::Choice(&[("on", "On"), ("off", "Off")]),
            },
            Item {
                label: "Save format",
                key: "save.format",
                desc: "Battery-save file extension for new games.",
                kind: ItemKind::Choice(&[("srm", ".srm"), ("sav", ".sav")]),
            },
            Item {
                label: "Use extracted name",
                key: "save.extracted",
                desc: "Zipped games save under the inner rom's name.",
                kind: ItemKind::Choice(&[("off", "Off"), ("on", "On")]),
            },
            Item {
                label: "Save compression",
                key: "save.compress",
                desc: "Write battery saves rzip-compressed (RetroArch format).",
                kind: ItemKind::Choice(&[("off", "Off"), ("on", "On")]),
            },
            Item {
                label: "State compression",
                key: "state.compress",
                desc: "Write save states rzip-compressed (RetroArch format).",
                kind: ItemKind::Choice(&[("off", "Off"), ("on", "On")]),
            },
            Item {
                label: "Power Profile",
                key: "power.profile",
                desc: "Auto balances speed and battery. Performance pins max clocks.",
                kind: ItemKind::Choice(&[
                    ("auto", "Auto"),
                    ("performance", "Performance"),
                    ("powersave", "Powersave"),
                ]),
            },
            Item {
                label: "Reset to defaults",
                key: "reset.system",
                desc: "Press A to restore this page's defaults.",
                kind: ItemKind::Action,
            }],
        },
        Page {
            title: "Date & Time",
            desc: "Set the clock",
            items: vec![
                Item {
                    label: "Time zone",
                    key: "tz.utc",
                    desc: "UTC offset. Applies everywhere at next boot.",
                    kind: ItemKind::Range { min: -12, max: 14, step: 1 },
                },
                Item { label: "Year", key: "dt.year", desc: "", kind: ItemKind::External },
                Item { label: "Month", key: "dt.month", desc: "", kind: ItemKind::External },
                Item { label: "Day", key: "dt.day", desc: "", kind: ItemKind::External },
                Item { label: "Hour", key: "dt.hour", desc: "", kind: ItemKind::External },
                Item { label: "Minute", key: "dt.minute", desc: "", kind: ItemKind::External },
                Item {
                    label: "Sync from internet",
                    key: "dt.ntp",
                    desc: "Press A to set the clock from the network (needs WiFi).",
                    kind: ItemKind::Action,
                },
            ],
        },
        Page {
            title: "Developer",
            desc: "SSH and debugging",
            items: vec![
                Item {
                    label: "SSH",
                    key: "dev.ssh",
                    desc: "Start or stop the SSH server now.",
                    kind: ItemKind::External,
                },
                Item {
                    label: "SSH on boot",
                    key: "dev.ssh_on_boot",
                    desc: "Start the SSH server automatically at boot.",
                    kind: ItemKind::Choice(&[("off", "Off"), ("on", "On")]),
                },
                Item {
                    label: "Disable auto sleep",
                    key: "dev.no_sleep",
                    desc: "Idle no longer suspends. The power button still sleeps.",
                    kind: ItemKind::Choice(&[("off", "Off"), ("on", "On")]),
                },
                Item {
                    label: "Clean dot files",
                    key: "dev.clean_dots",
                    desc: "Press A to delete ._*, .DS_Store and Thumbs.db from the card.",
                    kind: ItemKind::Action,
                },
            ],
        },
        Page {
            title: "About",
            desc: "This device",
            items: vec![
                Item {
                    label: "kUI",
                    key: "",
                    desc: "",
                    kind: ItemKind::Info(KUI_VERSION.to_string()),
                },
                Item {
                    label: "Device",
                    key: "",
                    desc: "",
                    kind: ItemKind::Info(device_name.to_string()),
                },
                Item {
                    label: "Stock OS",
                    key: "",
                    desc: "",
                    kind: ItemKind::Info(stock_ver.to_string()),
                },
                Item {
                    label: "BusyBox",
                    key: "",
                    desc: "",
                    kind: ItemKind::Info(busybox_ver.to_string()),
                },
            ],
        },
    ]
}

/// Current display value for an item.
pub fn display_value(cfg: &kui_config::Config, item: &Item) -> String {
    match &item.kind {
        ItemKind::Info(v) => v.clone(),
        ItemKind::Choice(choices) => {
            let cur = cfg.get_or(item.key, choices[0].0);
            choices
                .iter()
                .find(|(v, _)| *v == cur)
                .map(|(_, d)| d.to_string())
                .unwrap_or_else(|| choices[0].1.to_string())
        }
        ItemKind::Range { min, .. } => cfg.get_i32(item.key, *min).to_string(),
        ItemKind::External | ItemKind::Action => String::new(),
    }
}

/// Step an item's value; returns true if it changed.
pub fn adjust(cfg: &mut kui_config::Config, item: &Item, dir: i32) -> bool {
    match &item.kind {
        ItemKind::Info(_) | ItemKind::External | ItemKind::Action => false,
        ItemKind::Choice(choices) => {
            let cur = cfg.get_or(item.key, choices[0].0).to_string();
            let idx = choices.iter().position(|(v, _)| *v == cur).unwrap_or(0);
            let next = (idx as i32 + dir).rem_euclid(choices.len() as i32) as usize;
            cfg.set(item.key, choices[next].0);
            let _ = cfg.save();
            true
        }
        ItemKind::Range { min, max, step } => {
            let cur = cfg.get_i32(item.key, *min);
            let next = (cur + dir * step).clamp(*min, *max);
            if next != cur {
                cfg.set(item.key, next);
                let _ = cfg.save();
                true
            } else {
                false
            }
        }
    }
}
