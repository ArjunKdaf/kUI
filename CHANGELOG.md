# Changelog

## 0.27k (unreleased)

### New

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

- **Default LEDs are white.** The Top and Trigger (L/R) lights now default
  to white instead of green on the Default profile. Other profiles (charging
  green, low-battery amber, critical red, sleep dark) are unchanged, and any
  color you've set yourself still wins.

### Bugfixes

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
