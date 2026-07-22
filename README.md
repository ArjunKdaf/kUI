# kUI

A custom operating system for the TrimUI Brick, written from scratch in
Rust. No inherited code — one launcher, one game frontend, one small
daemon, all built for exactly this device.

## Why

- **Fast.** Boot adds ~1.2s over the kernel to a fully dressed carousel.
  Navigation, settings, and overlays are instant. 18–24 MB of RAM
  across two processes. The whole OS is a few megabytes on the card.
- **One of everything.** One settings surface (the Control Panel), one
  config file, one LED editor, one save-state universe per game.
- **Yours.** Theme colors drive the whole chrome. Artbook carousel.
  The Dude keeps your streaks honest.

## What's inside

- **Carousel launcher** — full-art platform carousel (Lists and Covers
  modes too), pins, collections, recents, per-list cursor memory,
  launch-origin memory (you return to where you launched from).
- **Native libretro frontend** — every core on the card certified:
  GB/GBC/GBA, SNES, Genesis/SMS/GG, TurboGrafx (+CD), Lynx, NGPC,
  WonderSwan, Amiga (+CD32), DOS, and more. Save states (8 slots with
  previews), auto-resume, bezels, five scaling modes, screen effects,
  GLSL shaders, cheats, per-game control remapping, turbo, rebindable
  shortcuts, fast-forward, multi-disc, screenshots.
- **RetroAchievements** — built on the official rcheevos library.
  Session announce, unlock toasts, badge art, in-game achievement
  browser. Softcore (hardcore pending upstream approval).
- **The Dude** — kUI's gamification heart: XP, quests (session, daily,
  weekly), 48 achievements, play streaks, retro trivia.
- **Game Switcher** — SELECT anywhere: your recent games as a carousel
  of last-session screenshots.
- **Control Panel** — every setting and tool in one place: appearance,
  display (incl. white point), connectivity (real WiFi/Bluetooth pages),
  LED profiles (incl. a Gaming profile), FN switch behavior, battery
  graph, game tracker, scraper & patcher, core options, time zone.
- **kuid** — a 1.8 MB daemon watching battery, LEDs, and radios.

## Install

See the release zip's README: copy three binaries and a boot hook onto
the SD card (the hook rides the card's existing boot chain). Fully
reversible; your games, saves, states, and art stay exactly where they
are.

## Compatibility

- kUI runs paks (SD layout, env, launch.sh, lifecycle hooks honored) —
  that contract is its one point of contact with other launchers. Zero
  paks shipped, none required.
- Save files, save states, box art, collections, and play history from
  earlier setups are read in place and carried forward.

## Building

Rust stable, cross-compiled with the tg5040 toolchain (see
`scripts/`). `nix-shell --run 'cargo build --release --target
aarch64-unknown-linux-gnu'`; `scripts/package.sh` produces the release
zip. Builds are deterministic.

## License

GPL-3.0. The Dude, the scraper, and the design are original works by
ArjunKdaf; rcheevos is vendored under its MIT license.
