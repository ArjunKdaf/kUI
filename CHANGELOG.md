# Changelog

## Unreleased

### Fixed

- **Pak Dek lists scroll with the cursor again.** Pak rows are taller
  than standard list rows, so only 7 fit on screen — but the scroll
  logic assumed 11. The highlight could walk below the visible area,
  and in categories with 8-10 paks (Media) the view never scrolled at
  all, leaving the last rows unreachable. Input and renderer now share
  one row count. Reported by pawndev.

## 0.27k (2026-07-31)

### New

- **Power profiles actually manage heat now.** All three profiles run the
  schedutil governor (idle always clocks down) and differ by frequency
  ceiling: Performance is uncapped, Auto caps at the 1.4GHz step,
  Powersave at the 1.0GHz step. One global setting (Control Panel →
  System) governs every game alike — fast-forward speed scales with the
  ceiling too, by design. Powersave is plenty for GB/GBC/GBA-class cores
  and keeps a long session dramatically cooler — an uncapped Hoenn hack
  session hit 79°C and wedged a CPU core; the same game under Powersave
  holds 42°C.
- **Save format is one choice now.** "Save format" offers RetroArch
  (.srm), RetroArch compressed (.srm in RA's rzip container, save states
  included), or minarch (.sav, always raw — minarch never compressed).
  Compressed saves move freely between kUI and RetroArch devices, and
  reading is always transparent: raw or rzip, any of them loads
  regardless of the setting, so switching never strands a save.
- **RetroAchievements hardcore, audit-ready.** Hardcore mode (config key
  `ra.hardcore`, default off; the Control Panel toggle stays hidden until
  RetroAchievements approves kUI as a client) implements RA's published
  hardcore compliance rules: save states can be created but never loaded
  (auto-resume boots fresh, Load hidden from the menu), cheats are never
  applied, and rc_client reset requests restart the game. Achievement
  runtime now rides beside every save state (`.rap` sidecar) so softcore
  state loads restore progress correctly, RA disconnect/reconnect and
  mastery events surface as toasts, and kUI identifies itself with its
  own stable user agent (`kUI/<version> (TrimUI; Linux) rc_client/…`).
  In hardcore, save-and-quit (menu+Start) simply quits: the in-game save
  is the only continuation point, so no resume state or slot is written
  (the battery save always flushes on exit). Core options RA forbids in
  hardcore (rcheevos' rc_libretro tables: layer hiding, PAL forcing,
  underclocking, built-in cheats, UNIBIOS, …) are dropped at launch and
  blocked in the in-game editor ("Blocked in hardcore") — the same rules
  RetroArch enforces, ported table-for-table with tests. Until RetroAchievements
  approves kUI as a client, the server records hardcore unlocks as
  softcore — the behavior ships now so the approval clock and audit have
  something real to look at.
- **Cheat downloads, RetroArch-style.** The in-game Cheats menu is now
  always available (except in RA hardcore); when a game has no cheat
  file yet, a "Download cheats" entry fetches it from the libretro cheat
  database — the same files RetroArch's online updater ships — matched
  by name with a preference for USA/World dumps. Needs WiFi; the file
  lands in `Cheats/<TAG>/<rom>.cht` so it loads automatically from then
  on. Control Panel gains **Download Cheats** next to RA PreFetch: pick
  All Platforms or a single one and it fetches cheat files for every
  game you have (skipping ones already on card), with live progress and
  A-to-cancel. RA PreFetch gained the same all-or-one platform picker.
- **Hold-to-scroll everywhere.** The in-game menus (Core Options, cheats,
  shortcuts, and every other list) now auto-repeat while up/down is held,
  with the same tuning as the launcher lists — one shared implementation
  drives both.
- **Game sessions get a log.** Emulator output now lands in
  `logs/kui-frontend.txt` (kept across reboots, pruned at 512K, with
  session start/exit markers). Previously it was discarded, which made
  in-game crashes impossible to diagnose after the fact.
- **Thermal telemetry.** kuid's minute-by-minute battery log
  (`.userdata/shared/kui/battlog.txt`) now also records CPU temperature,
  current CPU frequency, and the kernel's thermal-throttle step —
  the evidence trail for heat-related crash hunting.

- **Core license inventory.** `LICENSES/CORES.md` lists every shipped
  libretro core with its upstream repository and license (verified against
  libretro docs and each upstream), including a note on the four
  non-commercial cores (fbneo, opera, picodrive, snes9x). Ships on the card
  with the other license notices.

- **Default collections.** kUI ships built-in franchise collections —
  Mario, Pokémon, Zelda, Mega Man, Castlevania, Final Fantasy, Sonic,
  Dragon Quest, Metroid, Kirby, and ~30 more — that appear on their own the
  moment your library has games matching them (matched by name, accent- and
  case-insensitive: "Pokémon" and "Pokemon" both count, "Rockman" counts as
  Mega Man, "Dragon Warrior" as Dragon Quest), and stay hidden otherwise.
  They sit alongside your own collections. Don't want one? Wipe it (Y twice)
  and it's gone for good — the dismissal is remembered across updates, so it
  never comes back.
- **Collection background art.** The collections index shows each
  collection's own artwork panel on the right in Carousel and Covers views
  (Lists stays text-only), the way platform tiles do. Art resolves by an
  accent-folded slug key (`Pokémon` → `pokemon.png`): a user override in
  `.userdata/shared/kui/collections/` wins, otherwise the shipped default in
  `.system/res/collections/` is used — kept separate from the theme-managed
  carousel art. Drop a PNG in the userdata folder to replace any panel.

- **Gaming LEDs match the Default colors.** The Gaming profile's shipped
  color is now the same white as the Default profile (was green), still
  at its dimmer in-game brightness. Restyle either in the LED editor as
  before.
- **Default LEDs are white.** The Top and Trigger (L/R) lights now default
  to white instead of green on the Default profile. Other profiles (charging
  green, low-battery amber, critical red, sleep dark) are unchanged, and any
  color you've set yourself still wins.

### Bugfixes

- **SSH is off by default again, and boot is faster.** The stock rootfs was
  auto-starting sshd at boot (`rc.d/S50sshd`) and generating host keys on
  first boot — slow — regardless of the opt-in setting. kUI now neuters that
  hook, so SSH only runs when you enable it (Settings → Developer → SSH on
  boot). Shaves boot time, especially the first boot after a fresh install.
- **Lists view no longer shows a box-art panel** inside collections (or any
  list) — it's text-only now, as intended; Carousel and Covers keep the
  art.
- **A little more breathing room** between the WiFi/Bluetooth (quick menu)
  and Save/Load (in-game) labels and their highlighted values.

- **The Collections entry always shows in the quick menu.** It used to hide
  whenever you had no collections — but the quick menu is the only way in,
  so you could never reach the screen to make your first one. Always present
  now.

- **The Dude: quest games could silently fail to launch.** (Found via
  Castlevania: Order of Ecclesia and Gran Turismo, 2026-07-23.) Two fixes:
  - Platform tag resolution now walks up parent folders until it finds a
    `(TAG)`, so a ROM in a nested layout (a `disks/` subfolder, a per-game
    folder) still resolves to its platform and launches from anywhere in
    the UI — the switcher, Recents, the Dude.
  - The Dude now draws its quest pool from the launcher's own game library
    instead of a separate directory scanner. It offers exactly the games
    the launcher lists: only from platforms that have an emulator installed
    (so NDS games stop being offered when no NDS core is present), with the
    `.m3u` / disc-image handling inherited from the shared list. Deletes the
    Dude's bespoke scanner — less code, and the two lists can no longer
    drift.
- **Launch failures now show an error toast** (two-pill notification, e.g.
  "No emulator installed for NDS") instead of dying silently to stderr —
  from every launch surface: game lists, Recents, the switcher, and the
  Dude.
- **Quick menu WiFi / Bluetooth** now read e.g. "WiFi On" with the state
  highlighted in the accent color (was "WiFi: On"), and the toggle acts on
  that state rather than the label text — so flipping a radio works again.
- **In-game menu** shows the active save/load slot and multi-disc index as
  a highlighted value ("Save 3", "Disc 2"), with a "< / >" hint in the
  bottom bar, replacing the old "Save < 3 >" form.
- **WiFi / Bluetooth no longer come up at boot.** The stock rootfs
  auto-started wpa_supplicant (`rc.d/S96`, procd-respawned), lighting the
  radios for ~20 s before kUI could tear them down. That hook is now
  neutered once (the `/etc` overlay persists), so radios start only on
  request — kUI comes up dark.
- **PakDek could not fetch its pak list.** The storefront catalog
  (`storefront.json`) was missing from the repository after the rename;
  restored, so PakDek loads again.
- **PakDek now removes a pak's root leftovers on uninstall.** Some paks drop
  files at the SD-card root — config/settings JSON, a stray app binary,
  `LICENSE`/`README.md`, fonts — at install or first launch; removing the
  pak used to orphan them there. PakDek now snapshots the root when a pak is
  installed and, on removal, deletes exactly the entries that appeared
  afterward. It spares every kUI-owned path and every dotfile, and — when
  more than one pak is installed — anything another pak might own, so it
  only deletes what it can attribute unambiguously. Every deletion is
  logged to `.userdata/shared/pakdek/removals.log`. (Paks installed before
  this update carry no snapshot, so their existing strays are left as-is.)
- **Master System save states no longer corrupt the game.** (Found via
  Asterix and the Great Rescue, 2026-07-26: loading any state — including
  auto-resume — could garble the cartridge mapper and reset the game.)
  The shipped PicoDrive core was a 2024 build missing upstream's 2025 SMS
  state-loading fixes; this update bundles a fresh PicoDrive built from
  upstream master, plus kUI's own hardening for FM-sound state sizing
  (submitted upstream as picodrive #266). The one core rides along in the
  update payload — updates normally don't touch cores — so 0.09k cards
  get the fix without a reinstall.
- **Junk Roms folders are cleaned up.** A one-shot migration removes empty
  `Roms/` folders whose names carry stray color-code text (a card-curation
  script accident; e.g. a second "3DO (3DO)" folder wrapped in bracket
  gibberish). Only exact matches are touched and only if empty — a folder
  with anything in it is never deleted. Most cards have none of these and
  the migration does nothing.
