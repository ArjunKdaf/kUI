//! The Dude — gamification system (logic + state only).
//!
//! Port of KdafUI's `thedude_screen.c` (written by the project owner). No
//! rendering here: the launcher renders from the view methods and calls the
//! mutating methods on input.
//!
//! Data sources differ from the C original:
//! - Play data comes from the flat playlog at `<shared>/kui/playlog.txt`
//!   (one `epoch\tsecs\t/Roms/rel/path\tAlias` line per finished session)
//!   instead of game_logs.sqlite.
//! - The legacy `last_plays`/`last_time`/`last_unique` counters in dude.txt
//!   are treated as an imported baseline: combined totals = baseline +
//!   playlog aggregates, so the C's delta-since-last-visit logic keeps
//!   working across the sqlite -> playlog migration. The baseline is
//!   persisted as extra `baseline_*` keys (the C reader ignores unknown
//!   keys, so the file stays legacy-compatible).
//! - RetroAchievements live data is unavailable; `last_ra_unlocks` is kept
//!   as a stored number and shown in Stats only when > 0.
#![allow(dead_code)]

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

const MIN_PLAY_SECS: i64 = 300; // 5 minutes
const DAILY_QUEST_COUNT: usize = 6;
const WEEKLY_COUNT: usize = 3;
const STREAK_DAYS: usize = 28;
const ACH_COUNT: usize = 48;
const ACH_PER_PAGE: usize = 16;

// =============================================
// Small deterministic RNG (std only; replaces C rand())
// =============================================

struct Lcg(u64);

impl Lcg {
    fn from_time() -> Self {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9E3779B97F4A7C15);
        Lcg(n ^ 0xD1B54A32D192ED03)
    }
    fn seeded(seed: u64) -> Self {
        Lcg(seed.wrapping_mul(0x9E3779B97F4A7C15) ^ 0x5851F42D4C957F2D)
    }
    fn next(&mut self) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) as u32
    }
    fn below(&mut self, n: u32) -> u32 {
        if n == 0 { 0 } else { self.next() % n }
    }
}

// =============================================
// Local time (std only). The C used localtime(); we get the UTC offset once
// from `date +%z` (works with busybox, respects TZ) and fall back to UTC.
// =============================================

fn now_epoch() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn tz_offset_secs() -> i64 {
    let Ok(out) = std::process::Command::new("date").arg("+%z").output() else {
        return 0;
    };
    if !out.status.success() {
        return 0;
    }
    let s = String::from_utf8_lossy(&out.stdout);
    let s = s.trim();
    // "+0530" / "-0800"
    if s.len() == 5 {
        let sign = match &s[..1] {
            "+" => 1,
            "-" => -1,
            _ => return 0,
        };
        let (Ok(h), Ok(m)) = (s[1..3].parse::<i64>(), s[3..5].parse::<i64>()) else {
            return 0;
        };
        return sign * (h * 3600 + m * 60);
    }
    0
}

#[derive(Clone, Copy)]
struct Civil {
    year: i64,
    month: u32, // 1-12
    day: u32,   // 1-31
    hour: u32,
    min: u32,
    sec: u32,
    wday: u32, // 0 = Sunday (like tm_wday)
    yday: u32, // 0-based
}

/// Days since 1970-01-01 for a civil date (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let mp = if m > 2 { m - 3 } else { m + 9 } as i64;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn civil_from_epoch(epoch: i64, off: i64) -> Civil {
    let local = epoch + off;
    let days = local.div_euclid(86400);
    let secs = local.rem_euclid(86400);
    let (year, month, day) = civil_from_days(days);
    let yday = (days - days_from_civil(year, 1, 1)) as u32;
    Civil {
        year,
        month,
        day,
        hour: (secs / 3600) as u32,
        min: (secs / 60 % 60) as u32,
        sec: (secs % 60) as u32,
        wday: (days + 4).rem_euclid(7) as u32, // 1970-01-01 was a Thursday
        yday,
    }
}

fn yyyymmdd(c: &Civil) -> String {
    format!("{:04}{:02}{:02}", c.year, c.month, c.day)
}

/// strftime %W: week of year, Monday as first day, 00-53.
fn week_number(c: &Civil) -> i64 {
    let mwday = (c.wday + 6) % 7; // Monday = 0
    ((c.yday + 7 - mwday) / 7) as i64
}

// =============================================
// Definition tables (verbatim from the C)
// =============================================

// text, xp, target type (0 = plays, 1 = minutes), target value
const QUEST_DEFS: [(&str, i64, u8, i64); 10] = [
    // Tier 1
    ("Play any game (5+ min)", 20, 0, 1),
    ("Play for 10 minutes", 25, 1, 10),
    // Tier 2
    ("Play 2 different games", 50, 0, 2),
    ("Play for 20 minutes", 40, 1, 20),
    // Tier 3
    ("Play 3 different games", 75, 0, 3),
    ("Play for 30 minutes", 60, 1, 30),
    // Tier 4
    ("Play 5 different games", 120, 0, 5),
    ("Play for 1 hour", 100, 1, 60),
    // Tier 5
    ("Try a new game", 80, 0, 1),
    ("Play for 45 minutes", 80, 1, 45),
];

// text, xp, type (0=unique games, 1=total minutes, 2=sessions, 3=platforms, 4=streak), need
const WEEKLY_DEFS: [(&str, i64, i64, i64); 12] = [
    ("Play 5 different games", 200, 0, 5),
    ("Play for 2 hours total", 200, 1, 120),
    ("Complete 10 play sessions", 250, 2, 10),
    ("Try 3 different platforms", 300, 3, 3),
    ("Maintain a 5-day streak", 250, 4, 5),
    ("Play 10 different games", 400, 0, 10),
    ("Play for 5 hours total", 400, 1, 300),
    ("Complete 20 play sessions", 500, 2, 20),
    ("Try 5 different platforms", 500, 3, 5),
    ("Maintain a 7-day streak", 400, 4, 7),
    ("Play for 3 hours total", 300, 1, 180),
    ("Play 7 different games", 300, 0, 7),
];

const MENU: [&str; 10] = [
    "Talk",
    "Dude Quests",
    "Daily Quests",
    "Weekly Challenge",
    "Completed",
    "Achievements",
    "Stats",
    "Play History",
    "Streaks",
    "Reset Progress",
];

// =============================================
// Data types
// =============================================

struct Session {
    epoch: i64,
    dur: i64, // seconds
    rel: String,
    alias: String,
}

struct Game {
    rel: String,
    name: String,
    platform: String,
    play_count: i64,
    play_time: i64, // seconds
    last_played: i64,
}

struct DailyQuest {
    text: String,
    xp: i64,
    done: bool,
    def_idx: usize, // index into QUEST_DEFS; usize::MAX if unknown
}

#[derive(Default, Clone)]
struct Weekly {
    text: String,
    xp: i64,
    target: i64,
    progress: i64,
    done: bool,
    ty: i64,
}

pub struct QuestView {
    pub text: String,
    pub xp: i64,
    pub rom: Option<PathBuf>,
    pub hidden: bool,
}

struct DudeQuest {
    view: QuestView,
    rel: String,
}

struct State {
    xp: i64,
    level: i64,
    humor: i64,
    total_quests: i64,
    total_achievements: i64,
    last_plays: i64,
    last_time: i64,
    last_unique: i64,
    streak: i64,
    best_streak: i64,
    last_ra_unlocks: i64,
    last_date: String,
    reset_at: i64,
    // Playlog-era additions (see module docs).
    baseline_plays: i64,
    baseline_time: i64,
    baseline_unique: i64,
}

impl Default for State {
    fn default() -> Self {
        State {
            xp: 0,
            level: 1,
            humor: 50,
            total_quests: 0,
            total_achievements: 0,
            last_plays: 0,
            last_time: 0,
            last_unique: 0,
            streak: 0,
            best_streak: 0,
            last_ra_unlocks: 0,
            last_date: String::new(),
            reset_at: 0,
            baseline_plays: 0,
            baseline_time: 0,
            baseline_unique: 0,
        }
    }
}

// =============================================
// The Dude
// =============================================

pub struct Dude {
    shared: PathBuf,
    roms_root: PathBuf,
    /// Playable games (rel paths) from the launcher's scan — already filtered
    /// to platforms that have both ROMs and an emulator. The Dude picks its
    /// quest games from exactly this list, so it never suggests a game the
    /// launcher can't show or launch.
    library: Vec<String>,
    tz_off: i64,
    rng: Lcg,

    st: State,
    saved_ach: [bool; ACH_COUNT],

    sessions: Vec<Session>,         // full playlog, epoch ascending
    games: Vec<Game>,               // aggregated, last_played DESC, >= 5 min, rom exists
    daily: Vec<DailyQuest>,         // 6 per day
    daily_date: String,
    weekly: Vec<Weekly>,            // 3 per week
    weekly_week: i64,
    dude_quests: Vec<DudeQuest>,
    quest_views: Vec<QuestView>,    // parallel copy exposed to the caller
    achievements: Vec<(bool, &'static str, &'static str)>,

    delta_plays: i64,
    delta_time_min: i64,
    delta_unique: i64,

    greeting: String,
}

impl Dude {
    // ----- paths -----

    fn dir(&self) -> PathBuf {
        self.shared.join(".thedude")
    }
    fn data_file(&self) -> PathBuf {
        self.dir().join("dude.txt")
    }
    fn ach_file(&self) -> PathBuf {
        self.dir().join("achievements.txt")
    }
    fn daily_file(&self) -> PathBuf {
        self.dir().join("daily.txt")
    }
    fn weekly_file(&self) -> PathBuf {
        self.dir().join("weekly.txt")
    }
    fn pending_file(&self) -> PathBuf {
        self.dir().join("pending_quest.txt")
    }
    fn playlog_file(&self) -> PathBuf {
        self.shared.join("kui/playlog.txt")
    }

    // ----- open -----

    /// Load state, ingest the playlog, refresh daily/weekly/achievements/
    /// streak, credit any pending quest and session XP, generate dude quests
    /// and the entry greeting.
    pub fn open(shared: &Path, roms_root: &Path, library: Vec<String>) -> Dude {
        let mut d = Dude {
            shared: shared.to_path_buf(),
            roms_root: roms_root.to_path_buf(),
            library,
            tz_off: tz_offset_secs(),
            rng: Lcg::from_time(),
            st: State::default(),
            saved_ach: [false; ACH_COUNT],
            sessions: Vec::new(),
            games: Vec::new(),
            daily: Vec::new(),
            daily_date: String::new(),
            weekly: vec![Weekly::default(); WEEKLY_COUNT],
            weekly_week: 0,
            dude_quests: Vec::new(),
            quest_views: Vec::new(),
            achievements: Vec::new(),
            delta_plays: 0,
            delta_time_min: 0,
            delta_unique: 0,
            greeting: String::new(),
        };

        d.load_state();
        d.load_sessions();
        d.build_games();
        let quest_xp = d.resolve_pending_quest();
        d.load_saved_achievements();
        let earned = d.calculate_xp();
        d.generate_daily_quests();
        d.generate_weekly();
        d.check_achievements();
        d.generate_dude_quests();

        d.greeting = d.dialogue_greeting();
        if quest_xp > 0 {
            d.greeting.push_str(&format!("\nQuest complete! +{quest_xp}xp!"));
        }
        if earned > 0 {
            d.greeting.push_str(&format!("\n+{earned}xp earned!"));
        }
        d
    }

    // ----- state file I/O (legacy dude.txt format) -----

    fn load_state(&mut self) {
        self.st = State::default();
        let Ok(text) = fs::read_to_string(self.data_file()) else {
            return;
        };
        let mut have_baseline = false;
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            let v = v.trim();
            let st = &mut self.st;
            match k {
                "xp" => st.xp = v.parse().unwrap_or(st.xp),
                "level" => st.level = v.parse().unwrap_or(st.level),
                "humor" => st.humor = v.parse().unwrap_or(st.humor),
                "total_quests" => st.total_quests = v.parse().unwrap_or(0),
                "total_achievements" => st.total_achievements = v.parse().unwrap_or(0),
                "last_plays" => st.last_plays = v.parse().unwrap_or(0),
                "last_time" => st.last_time = v.parse().unwrap_or(0),
                "last_unique" => st.last_unique = v.parse().unwrap_or(0),
                "streak" => st.streak = v.parse().unwrap_or(0),
                "best_streak" => st.best_streak = v.parse().unwrap_or(0),
                "last_ra_unlocks" => st.last_ra_unlocks = v.parse().unwrap_or(0),
                "last_date" => st.last_date = v.to_string(),
                "reset_at" => st.reset_at = v.parse().unwrap_or(0),
                "baseline_plays" => {
                    st.baseline_plays = v.parse().unwrap_or(0);
                    have_baseline = true;
                }
                "baseline_time" => st.baseline_time = v.parse().unwrap_or(0),
                "baseline_unique" => st.baseline_unique = v.parse().unwrap_or(0),
                _ => {}
            }
        }
        // First run after the sqlite -> playlog migration: the legacy last_*
        // totals ARE the baseline (the playlog held nothing when they were
        // last written by the C build).
        if !have_baseline {
            self.st.baseline_plays = self.st.last_plays;
            self.st.baseline_time = self.st.last_time;
            self.st.baseline_unique = self.st.last_unique;
        }
    }

    fn save_state(&self) {
        let _ = fs::create_dir_all(self.dir());
        let st = &self.st;
        let out = format!(
            "xp={}\nlevel={}\nhumor={}\ntotal_quests={}\ntotal_achievements={}\n\
             last_plays={}\nlast_time={}\nlast_unique={}\nstreak={}\nbest_streak={}\n\
             last_ra_unlocks={}\nlast_date={}\nreset_at={}\n\
             baseline_plays={}\nbaseline_time={}\nbaseline_unique={}\n",
            st.xp,
            st.level,
            st.humor,
            st.total_quests,
            st.total_achievements,
            st.last_plays,
            st.last_time,
            st.last_unique,
            st.streak,
            st.best_streak,
            st.last_ra_unlocks,
            st.last_date,
            st.reset_at,
            st.baseline_plays,
            st.baseline_time,
            st.baseline_unique,
        );
        let _ = fs::write(self.data_file(), out);
    }

    /// Persist dude.txt. (Achievements/daily/weekly files are written at
    /// their own mutation points, like the C.)
    pub fn save(&self) {
        self.save_state();
    }

    // ----- playlog ingest -----

    /// Normalize a rom path to a Roms-relative string ("GB (GB)/game.gb").
    fn norm_rel(&self, p: &str) -> String {
        let p = p.trim();
        if let Some(r) = p.strip_prefix("/Roms/") {
            return r.to_string();
        }
        if let Some(r) = p.strip_prefix("Roms/") {
            return r.to_string();
        }
        if let Ok(r) = Path::new(p).strip_prefix(&self.roms_root) {
            return r.to_string_lossy().into_owned();
        }
        p.trim_start_matches('/').to_string()
    }

    fn full_rom_path(&self, rel: &str) -> PathBuf {
        self.roms_root.join(rel)
    }

    fn load_sessions(&mut self) {
        self.sessions.clear();
        let Ok(text) = fs::read_to_string(self.playlog_file()) else {
            return;
        };
        for line in text.lines() {
            let mut it = line.splitn(4, '\t');
            let (Some(epoch), Some(dur), Some(path), Some(alias)) =
                (it.next(), it.next(), it.next(), it.next())
            else {
                continue;
            };
            let (Ok(epoch), Ok(dur)) = (epoch.trim().parse::<i64>(), dur.trim().parse::<i64>())
            else {
                continue;
            };
            let rel = self.norm_rel(path);
            self.sessions.push(Session {
                epoch,
                dur,
                rel,
                alias: alias.trim().to_string(),
            });
        }
        self.sessions.sort_by_key(|s| s.epoch);
    }

    /// Platform tag from a Roms-relative path: the "(TAG)" in the top-level
    /// folder name (whole folder name when untagged).
    fn platform_of(rel: &str) -> String {
        let folder = rel.split('/').next().unwrap_or("");
        if let Some(open) = folder.rfind('(')
            && let Some(close) = folder[open + 1..].find(')')
        {
            return folder[open + 1..open + 1 + close].to_string();
        }
        folder.to_string()
    }

    /// Aggregate per-game stats from the playlog. Mirrors the C read_games()
    /// query: sessions since reset_at, grouped by game, total >= 5 min,
    /// skipping games whose rom no longer exists, ordered last-played DESC.
    fn build_games(&mut self) {
        let mut agg: Vec<Game> = Vec::new();
        for s in &self.sessions {
            if s.epoch < self.st.reset_at {
                continue;
            }
            // identity = the alias field
            if let Some(g) = agg.iter_mut().find(|g| g.name == s.alias) {
                g.play_count += 1;
                g.play_time += s.dur;
                if s.epoch >= g.last_played {
                    g.last_played = s.epoch;
                    g.rel = s.rel.clone();
                }
            } else {
                agg.push(Game {
                    rel: s.rel.clone(),
                    name: s.alias.clone(),
                    platform: Self::platform_of(&s.rel),
                    play_count: 1,
                    play_time: s.dur,
                    last_played: s.epoch,
                });
            }
        }
        agg.retain(|g| g.play_time >= MIN_PLAY_SECS);
        // Keep only games still in the launcher's playable library (ROM
        // present AND an emulator for the platform) — so a Revisit/Marathon
        // quest never launches a dead path or an unplayable platform.
        if !self.library.is_empty() {
            let lib: HashSet<&String> = self.library.iter().collect();
            agg.retain(|g| lib.contains(&g.rel));
        }
        agg.sort_by_key(|g| std::cmp::Reverse(g.last_played));
        self.games = agg;
    }

    fn count_platforms(&self) -> i64 {
        let mut seen: Vec<&str> = Vec::new();
        for g in &self.games {
            if !seen.contains(&g.platform.as_str()) {
                seen.push(&g.platform);
            }
        }
        seen.len() as i64
    }

    // Combined totals (imported baseline + playlog aggregates).
    fn agg_plays(&self) -> i64 {
        self.games.iter().map(|g| g.play_count).sum()
    }
    fn agg_time(&self) -> i64 {
        self.games.iter().map(|g| g.play_time).sum()
    }
    fn combined_plays(&self) -> i64 {
        self.st.baseline_plays + self.agg_plays()
    }
    fn combined_time(&self) -> i64 {
        self.st.baseline_time + self.agg_time()
    }
    fn combined_unique(&self) -> i64 {
        self.st.baseline_unique + self.games.len() as i64
    }

    // ----- level & XP -----

    fn calc_level(&mut self) {
        let mut total = 0;
        self.st.level = 1;
        while self.st.level <= 100 {
            total += self.st.level * (self.st.level + 10);
            if self.st.xp < total {
                return;
            }
            self.st.level += 1;
        }
        self.st.level = 100;
    }

    fn xp_for_next(&self) -> i64 {
        (1..=self.st.level).map(|l| l * (l + 10)).sum()
    }

    fn xp_for_current(&self) -> i64 {
        (1..self.st.level).map(|l| l * (l + 10)).sum()
    }

    fn grant_xp(&mut self, amount: i64) {
        self.st.xp += amount;
        self.calc_level();
        self.save_state();
    }

    /// XP earned into the current level and the level's XP span — for the
    /// "%d / %d XP" progress bar.
    pub fn xp_progress(&self) -> (i64, i64) {
        let cur = self.xp_for_current();
        let next = self.xp_for_next();
        ((self.st.xp - cur).max(0), (next - cur).max(0))
    }

    // ----- streak -----

    fn update_streak(&mut self) {
        let now = now_epoch();
        let today = yyyymmdd(&civil_from_epoch(now, self.tz_off));
        if self.st.last_date == today {
            return; // already counted today
        }
        let yesterday = yyyymmdd(&civil_from_epoch(now - 86400, self.tz_off));
        if self.st.last_date == yesterday {
            self.st.streak += 1;
        } else {
            self.st.streak = 1;
        }
        if self.st.streak > self.st.best_streak {
            self.st.best_streak = self.st.streak;
        }
        self.st.last_date = today;
    }

    // ----- passive XP (delta since last visit) -----

    fn calculate_xp(&mut self) -> i64 {
        let total_plays = self.combined_plays();
        let total_time = self.combined_time();
        let unique = self.combined_unique();

        let dp = (total_plays - self.st.last_plays).max(0);
        let dt = (total_time - self.st.last_time).max(0);
        let du = (unique - self.st.last_unique).max(0);

        self.delta_plays = dp;
        self.delta_time_min = dt / 60;
        self.delta_unique = du;

        let earned = dp * 10 + du * 25 + dt / 60;

        // (RA unlock XP omitted: no live RA data in the playlog build.)
        if dp > 0 {
            self.st.humor += dp * 3 + du * 8;
            self.update_streak();
        } else {
            self.st.humor -= 5;
        }
        self.st.humor = self.st.humor.clamp(0, 100);

        self.st.xp += earned;
        self.st.last_plays = total_plays;
        self.st.last_time = total_time;
        self.st.last_unique = unique;
        self.calc_level();
        self.save_state();
        earned
    }

    // ----- mood, face, rank -----

    fn mood(&self) -> &'static str {
        match self.st.humor {
            95.. => "Legendary!",
            85.. => "Amazed",
            75.. => "Stoked!",
            65.. => "Happy",
            55.. => "Chill",
            45.. => "Vibin'",
            35.. => "Meh",
            25.. => "Bored",
            15.. => "Sleepy",
            5.. => "Sad",
            _ => "Bummed",
        }
    }

    pub fn face(&self) -> &'static str {
        match self.st.humor {
            95.. => "\\(^_^)/",
            85.. => "(*_*)",
            75.. => "d(^_^)b",
            65.. => "(^_^)",
            // the C's sunglasses face needs glyphs our fonts lack;
            // a relaxed wink keeps the Chill energy in pure ASCII
            55.. => "(-_o)",
            45.. => "(~_~)",
            35.. => "(-_-)",
            25.. => "(=_=)",
            15.. => "(._.)zZ",
            5.. => "(u_u)",
            _ => "(T_T)",
        }
    }

    pub fn rank_title(&self) -> &'static str {
        match self.st.level {
            90.. => "The Grand Master Dude",
            65.. => "The Master Dude",
            45.. => "The Knight Dude",
            25.. => "The Padawan Dude",
            10.. => "The Initiate Dude",
            _ => "The Youngling Dude",
        }
    }

    /// Rank star badge ("" below level 30).
    pub fn stars(&self) -> &'static str {
        match self.st.level {
            90.. => "\u{2605}\u{2605}\u{2605}",
            60.. => "\u{2605}\u{2605}",
            30.. => "\u{2605}",
            _ => "",
        }
    }

    /// Level 100 crown ("▲▲◆▲▲" in the C rendering).
    pub fn has_crown(&self) -> bool {
        self.st.level >= 100
    }

    pub fn mood_line(&self) -> String {
        format!("{} | Lv.{} | Humor:{}", self.mood(), self.st.level, self.st.humor)
    }

    pub fn menu(&self) -> &'static [&'static str] {
        &MENU
    }

    pub fn greeting(&self) -> String {
        self.greeting.clone()
    }

    // ----- dialogue -----

    fn dialogue_greeting(&self) -> String {
        let c = civil_from_epoch(now_epoch(), self.tz_off);
        let (hour, month, day) = (c.hour, c.month, c.day);

        let seasonal = if month == 12 && (24..=26).contains(&day) {
            Some("Merry Christmas! ")
        } else if month == 1 && day == 1 {
            Some("Happy New Year! ")
        } else if month == 10 && day == 31 {
            Some("Happy Halloween! ")
        } else if month == 12 && day == 31 {
            Some("Last day of the year! ")
        } else if month == 2 && day == 14 {
            Some("Happy Valentine's Day! ")
        } else {
            None
        };

        let timeword = match hour {
            0..4 => "Whoa, still up? Hardcore!",
            4..6 => "Early bird catches the game!",
            6..9 => "Good morning!",
            9..12 => "Morning vibes!",
            12..14 => "Lunchtime gaming?",
            14..17 => "Good afternoon!",
            17..20 => "Evening session!",
            20..23 => "Night owl mode!",
            _ => "Midnight gaming!",
        };

        let greeting = match seasonal {
            Some(s) => format!("{s}{timeword}"),
            None => timeword.to_string(),
        };

        match self.st.humor {
            90.. => format!("{greeting} The Dude is LEGENDARY today! Let's crush some games!"),
            70.. => format!("{greeting} Feeling great! What should we play?"),
            50.. => format!("{greeting} Hey there. Ready for some games?"),
            30.. => format!("{greeting} Been a while... The Dude misses gaming."),
            _ => format!("{greeting} Please play something... The Dude is fading..."),
        }
    }

    fn dialogue_game_comment(&mut self) -> String {
        // first max, like the C's strictly-greater scan
        let fav = self
            .games
            .iter()
            .fold(None::<&Game>, |best, g| match best {
                Some(b) if g.play_time > b.play_time => Some(g),
                None => Some(g),
                other => other,
            });
        let old = self.games.last();
        let platforms = self.count_platforms();
        let total_minutes: i64 = self.games.iter().map(|g| g.play_time / 60).sum();
        let game_count = self.games.len() as i64;
        let r = self.rng.below(16);

        match r {
            // NOTE: the C passed (name, minutes) to a "%d ... %s" format —
            // undefined behavior. Ported with the obviously intended order.
            0 if fav.is_some() => {
                let f = fav.unwrap();
                format!("You've spent {} minutes in {} alone! That's your #1 game and The Dude totally gets why. Some games just grab you and never let go. Keep at it!", f.play_time / 60, f.name)
            }
            1 if old.is_some() => {
                let o = old.unwrap();
                format!("Hey, remember {}? It's been sitting there lonely and forgotten. The Dude thinks it deserves another chance. Maybe you'll discover something you missed the first time around!", o.name)
            }
            2 if platforms > 1 => {
                format!("You've explored {platforms} different platforms! From 8-bit handhelds to 32-bit powerhouses, you're a true multi-system gamer. Each platform has its own soul and you're experiencing them all.")
            }
            3 if self.st.streak > 1 => {
                format!("{} days in a row! That's some serious dedication right there. Most people can't keep a streak going for more than a few days, but you're built different. Keep it alive!", self.st.streak)
            }
            4 if game_count > 0 => {
                format!("Your library has {} games with real playtime, adding up to {} hours of retro goodness! That's a lot of memories, boss fights, and pixel-perfect moments.", game_count, total_minutes / 60)
            }
            5 if fav.is_some_and(|f| f.play_count > 3) => {
                let f = fav.unwrap();
                format!("You've launched {} a total of {} times! That's what The Dude calls loyalty. Whether it's the gameplay, the music, or just the vibes, something keeps pulling you back.", f.name, f.play_count)
            }
            6 if game_count >= 5 => {
                format!("With {game_count} games in your played collection, you're building something real. Every game you try adds to your experience and makes you a better gamer. The Dude is genuinely proud.")
            }
            7 if platforms >= 3 => {
                format!("A true platform explorer with {platforms} systems under your belt! Have you tried the more obscure ones? WonderSwan, Neo Geo Pocket, or Vectrex have some incredible hidden gems.")
            }
            8 if total_minutes > 60 => {
                format!("You've logged {} hours of total gaming. Think about all the worlds you've visited, all the characters you've met, all the bosses you've defeated. That's a journey worth celebrating!", total_minutes / 60)
            }
            9 if fav.is_some() && total_minutes > 0 => {
                let f = fav.unwrap();
                let denom = if total_minutes > 0 { total_minutes } else { 1 };
                format!("{} takes up {}% of your total playtime! When you find a game that clicks, there's nothing else like it. The Dude knows that feeling well.", f.name, (f.play_time / 60) * 100 / denom)
            }
            10 if game_count >= 10 => {
                format!("Did you know you've played {game_count} unique games? That's more than most people play in a year. You're not just a gamer, you're a connoisseur of interactive entertainment!")
            }
            11 if old.is_some_and(|o| o.play_count == 1) => {
                let o = old.unwrap();
                format!("You only played {} once. First impressions aren't always right, you know. Some of the best games reveal their depth on the second or third playthrough.", o.name)
            }
            12 if self.st.streak == 1 => {
                "Day one of a new streak! Every long streak starts with a single day. The Dude believes this could be the beginning of something legendary. Let's see how far you can go!".to_string()
            }
            13 if platforms == 1 => {
                "You've been sticking to one platform so far. Nothing wrong with loyalty, but there's a whole universe of retro systems out there waiting to be explored. Branch out!".to_string()
            }
            14 if game_count >= 3 => {
                format!("The Dude's been watching your gaming habits and honestly? Pretty impressive. You've got {} games, {} platforms, and {} hours logged. That's a real gamer's resume right there.", game_count, platforms, total_minutes / 60)
            }
            _ => {
                "Every game has a story. Every play session is an adventure. Whether you're speedrunning classics or discovering hidden gems, The Dude is here for all of it. Keep playing!".to_string()
            }
        }
    }

    fn dialogue_level_milestone(&self) -> String {
        let l = self.st.level;
        match l {
            100.. => "Level 100! MAX LEVEL! You've reached the absolute pinnacle of The Dude's respect. You've played the games, earned the XP, and proven yourself a true legend. The Dude bows to you. \\(^_^)/".to_string(),
            75.. => format!("Level {l}! You're approaching legendary status and most gamers never make it this far. The crown is within reach. Keep pushing, keep playing, keep being awesome!"),
            50.. => format!("Level {l}! Halfway to the top! You've earned your stars and The Dude can see the dedication in every session. At this rate, legend status is just a matter of time."),
            25.. => format!("Level {l}! A quarter of the way to max and you've already proven you're not a casual. The sunglasses suit you. Keep grinding those quests and exploring new games!"),
            10.. => format!("Level {l}! Double digits, baby! You've left the newbie zone behind and The Dude is starting to take you seriously. Try completing daily quests for even more XP!"),
            5.. => format!("Level {l}! You're getting warmed up and The Dude likes what he sees. Pro tip: play different games and platforms to maximize your XP gains. Variety is the spice of gaming!"),
            _ => format!("Level {l}. Every legend starts somewhere, and this is your beginning. Play games for 5+ minutes to earn XP, complete daily quests, and unlock achievements. The Dude believes in you!"),
        }
    }

    fn dialogue_trivia(&mut self) -> String {
        TRIVIA[self.rng.below(TRIVIA.len() as u32) as usize].to_string()
    }

    fn dialogue_mood(&mut self) -> String {
        let r = self.rng.below(6) as usize;
        if self.st.humor >= 80 {
            const HAPPY: [&str; 6] = [
                "The Dude is absolutely vibing right now! Your gaming sessions have been fueling pure joy. Whatever you're playing, it's working. Don't stop!",
                "Life is good when you're gaming! The Dude hasn't felt this happy in ages. Your dedication to retro gaming is genuinely inspiring. Keep those good times rolling!",
                "You know what makes The Dude happy? A gamer who shows up consistently. And you've been doing exactly that. This partnership is going places!",
                "The Dude is on cloud nine! Between the games you've been playing and the quests you've been completing, everything just feels right. Let's keep this energy going!",
                "Mood check: AMAZING. The Dude is living his best life thanks to your gaming sessions. Fun fact: the more you play, the happier The Dude gets!",
                "The Dude is so happy right now that even his face has upgraded! Check out that expression. That's the face of a companion who truly appreciates great gaming.",
            ];
            HAPPY[r].to_string()
        } else if self.st.humor >= 50 {
            const OK: [&str; 6] = [
                "The Dude is doing alright! Not legendary, not terrible. A solid gaming session would bump things up though. What do you say, pick something and let's go?",
                "Vibes are decent today. The Dude has seen better days but also much worse. A good streak of gaming would turn things around. Every game counts!",
                "The Dude is chilling, but could use a pick-me-up. Maybe try a game you haven't played in a while? Nostalgia is a powerful mood booster for The Dude.",
                "Middle of the road right now. The Dude isn't complaining, but a few more gaming sessions would really help lift the mood. Quality time with retro games is the best medicine!",
                "Feeling okay! The Dude appreciates you checking in. Some regular gaming sessions and quest completions would get us back to peak happiness in no time.",
                "Not bad, not bad at all! The Dude is content but always striving for more. Play some games, complete some quests, and watch that humor meter climb!",
            ];
            OK[r].to_string()
        } else {
            const SAD: [&str; 6] = [
                "The Dude is feeling pretty low right now. It's been a while since a good gaming session and the boredom is real. Even a quick 10-minute session would help!",
                "Things aren't great, not gonna lie. The Dude needs some gaming fuel. Any game will do! Pick something, play for a bit, and watch the mood improve.",
                "The Dude is fading... humor is dropping and the only cure is retro gaming. Please, pick up that handheld and play something. Anything! The Dude needs you!",
                "Rock bottom vibes over here. But hey, every comeback story starts at the bottom. One game. That's all it takes to start turning things around. The Dude believes!",
                "It's been too quiet around here. The Dude misses the sound of chiptunes and the glow of pixels. Come on, let's fire up a classic and bring some life back!",
                "The Dude is lonely and bored. Remember when we used to play games together? Those were the days. We could have those days again if you just press Start...",
            ];
            SAD[r].to_string()
        }
    }

    /// Next Talk dialogue line. (The RA commentary variant is skipped: it
    /// needs live RA data, exactly as the C skipped it when unavailable.)
    pub fn talk(&mut self) -> String {
        // trivia weighted up from the C's uniform 1/5 — Arjun's call
        match self.rng.below(10) {
            0 | 1 => self.dialogue_game_comment(),
            2 => self.dialogue_level_milestone(),
            3..=6 => self.dialogue_trivia(),
            7 | 8 => self.dialogue_mood(),
            _ => {
                if self.st.humor >= 60 {
                    "The Dude abides. That's the mantra. No matter what happens out there in the real world, there's always a retro game waiting to take you somewhere magical. So what are we playing next?".to_string()
                } else {
                    "The Dude needs your help. Humor is low, energy is fading, and the only prescription is more gaming. It doesn't have to be a marathon, just pick something and play for a bit. Every minute counts!".to_string()
                }
            }
        }
    }

    // ----- achievements -----

    fn load_saved_achievements(&mut self) {
        self.saved_ach = [false; ACH_COUNT];
        let Ok(text) = fs::read_to_string(self.ach_file()) else {
            return;
        };
        for line in text.lines() {
            let Some((idx, val)) = line.split_once('=') else {
                continue;
            };
            // (The C loader had an `idx < 16` bounds bug that dropped saved
            // state for indices 16-47; fixed here so every earned
            // achievement stays earned across the playlog migration.)
            if let (Ok(i), Ok(v)) = (idx.trim().parse::<usize>(), val.trim().parse::<i64>())
                && i < ACH_COUNT
            {
                self.saved_ach[i] = v != 0;
            }
        }
    }

    fn save_achievements(&self) {
        let _ = fs::create_dir_all(self.dir());
        let mut out = String::new();
        for (i, (done, _, _)) in self.achievements.iter().enumerate() {
            out.push_str(&format!("{}={}\n", i, if *done { 1 } else { 0 }));
        }
        let _ = fs::write(self.ach_file(), out);
    }

    fn check_achievements(&mut self) {
        let c = civil_from_epoch(now_epoch(), self.tz_off);
        let hour = c.hour;
        let wday = c.wday;
        let platforms = self.count_platforms();
        // Combined totals keep threshold progress continuous across the
        // sqlite -> playlog migration; per-game checks use the playlog.
        let total_minutes = self.combined_time() / 60;
        let total_sessions = self.combined_plays();
        let game_count = self.combined_unique();
        let daily_done = self.daily.iter().filter(|q| q.done).count();
        let quest_count = self.daily.len();
        let any_time = |t: i64| self.games.iter().any(|g| g.play_time >= t);
        let any_count = |n: i64| self.games.iter().any(|g| g.play_count >= n);
        let st = &self.st;
        let saved = self.saved_ach;

        let mut a: Vec<(bool, &'static str, &'static str)> = Vec::with_capacity(ACH_COUNT);
        let ach = |list: &mut Vec<(bool, &'static str, &'static str)>,
                       name: &'static str,
                       desc: &'static str,
                       cond: bool| {
            let i = list.len();
            list.push((cond || (i < ACH_COUNT && saved[i]), name, desc));
        };

        // Page 1: Game milestones
        ach(&mut a, "First Steps", "Play your first game", game_count >= 1);
        ach(&mut a, "Casual Gamer", "Play 5 unique games", game_count >= 5);
        ach(&mut a, "Explorer", "Play 10 unique games", game_count >= 10);
        ach(&mut a, "Collector", "Play 50 unique games", game_count >= 50);
        ach(&mut a, "Game Hoarder", "Play 100 unique games", game_count >= 100);
        ach(&mut a, "Platform Hopper", "Try 5 platforms", platforms >= 5);
        ach(&mut a, "Genre Explorer", "Try 10 platforms", platforms >= 10);
        ach(&mut a, "Marathon", "2h in one session", any_time(7200));
        ach(&mut a, "Endurance", "5h in one session", any_time(18000));
        ach(&mut a, "Loyal Fan", "Launch one game 10 times", any_count(10));
        ach(&mut a, "Time Traveler", "50 hours total play", total_minutes >= 3000);
        ach(&mut a, "Time Lord", "100 hours total play", total_minutes >= 6000);
        ach(&mut a, "Night Owl", "Play after midnight", hour < 4);
        ach(&mut a, "Early Bird", "Play before 6am", (4..6).contains(&hour));
        ach(&mut a, "Replay King", "Launch one game 5x", any_count(5));
        ach(&mut a, "Speed Gamer", "Play 3 games today", self.delta_plays >= 3);

        // Page 2: Progression
        ach(&mut a, "Apprentice", "Reach level 5", st.level >= 5);
        ach(&mut a, "Rising Star", "Reach level 10", st.level >= 10);
        ach(&mut a, "Expert", "Reach level 25", st.level >= 25);
        ach(&mut a, "Veteran", "Reach level 50", st.level >= 50);
        ach(&mut a, "Grandmaster", "Reach level 75", st.level >= 75);
        ach(&mut a, "Legend", "Reach level 100", st.level >= 100);
        ach(&mut a, "XP Hunter", "Earn 1000 XP total", st.xp >= 1000);
        ach(&mut a, "XP Machine", "Earn 5000 XP total", st.xp >= 5000);
        ach(&mut a, "XP God", "Earn 10000 XP total", st.xp >= 10000);
        ach(&mut a, "Chill Dude", "Reach 55 humor", st.humor >= 55);
        ach(&mut a, "Stoked Dude", "Reach 75 humor", st.humor >= 75);
        ach(&mut a, "Happy Dude", "Reach 90 humor", st.humor >= 90);
        ach(&mut a, "Legendary Mood", "Reach 95 humor", st.humor >= 95);
        ach(&mut a, "Rock Bottom", "Humor drops to 5", st.humor <= 5);
        ach(&mut a, "Deep Diver", "10h on one game", any_time(36000));
        ach(&mut a, "Century Club", "100 total sessions", total_sessions >= 100);

        // Page 3: Streaks & quests
        ach(&mut a, "Getting Started", "3-day streak", st.best_streak >= 3);
        ach(&mut a, "Dedicated", "7-day streak", st.best_streak >= 7);
        ach(&mut a, "Habit Formed", "21-day streak", st.best_streak >= 21);
        ach(&mut a, "Committed", "30-day streak", st.best_streak >= 30);
        ach(&mut a, "Unstoppable", "60-day streak", st.best_streak >= 60);
        ach(&mut a, "Quest Beginner", "Complete 5 quests", st.total_quests >= 5);
        ach(&mut a, "Quest Master", "Complete 25 quests", st.total_quests >= 25);
        ach(&mut a, "Quest Legend", "Complete 100 quests", st.total_quests >= 100);
        ach(
            &mut a,
            "Completionist",
            "All dailies in a day",
            daily_done >= DAILY_QUEST_COUNT && quest_count > 0,
        );
        ach(&mut a, "Trophy Hunter", "Earn 10 achievements", st.total_achievements >= 10);
        ach(&mut a, "Trophy Master", "Earn 25 achievements", st.total_achievements >= 25);
        ach(&mut a, "Trophy Legend", "Earn 40 achievements", st.total_achievements >= 40);
        ach(&mut a, "Weekend Warrior", "Play on weekend", wday == 0 || wday == 6);
        ach(&mut a, "Lunchtime Hero", "Play between 12-14", (12..14).contains(&hour));
        ach(&mut a, "Iron Will", "90-day streak", st.best_streak >= 90);
        ach(&mut a, "The Grind", "500 hours total play", total_minutes >= 30000);

        self.achievements = a;
        self.st.total_achievements = self.achievements.iter().filter(|(d, _, _)| *d).count() as i64;
        self.save_achievements();
    }

    // ----- daily quests -----

    fn today_str(&self) -> String {
        yyyymmdd(&civil_from_epoch(now_epoch(), self.tz_off))
    }

    fn save_daily_quests(&self) {
        let _ = fs::create_dir_all(self.dir());
        let mut out = format!("date={}\n", self.daily_date);
        for (i, q) in self.daily.iter().enumerate() {
            let def = if q.def_idx < QUEST_DEFS.len() { q.def_idx as i64 } else { -1 };
            out.push_str(&format!(
                "q{}={}:{}:{}:{}\n",
                i,
                if q.done { 1 } else { 0 },
                q.xp,
                def,
                q.text
            ));
        }
        let _ = fs::write(self.daily_file(), out);
    }

    /// Returns true when today's saved quests were loaded.
    fn load_daily_quests(&mut self) -> bool {
        self.daily.clear();
        let Ok(text) = fs::read_to_string(self.daily_file()) else {
            return false;
        };
        let mut saved_date = String::new();
        for line in text.lines() {
            if let Some(d) = line.strip_prefix("date=") {
                saved_date = d.trim().to_string();
                continue;
            }
            if !line.starts_with('q') {
                continue;
            }
            let Some((_, rest)) = line.split_once('=') else {
                continue;
            };
            // format: done:xp:defidx:text
            let mut it = rest.splitn(4, ':');
            let (Some(done), Some(xp), Some(def), Some(txt)) =
                (it.next(), it.next(), it.next(), it.next())
            else {
                continue;
            };
            let (Ok(done), Ok(xp), Ok(def)) = (
                done.trim().parse::<i64>(),
                xp.trim().parse::<i64>(),
                def.trim().parse::<i64>(),
            ) else {
                continue;
            };
            if self.daily.len() >= 32 {
                break;
            }
            self.daily.push(DailyQuest {
                text: txt.to_string(),
                xp,
                done: done != 0,
                def_idx: if (0..QUEST_DEFS.len() as i64).contains(&def) {
                    def as usize
                } else {
                    usize::MAX
                },
            });
        }
        if saved_date != self.today_str() {
            return false; // new day, need new quests
        }
        self.daily_date = saved_date;
        true
    }

    fn generate_daily_quests(&mut self) {
        if self.load_daily_quests() {
            // Update progress for incomplete quests (never un-complete).
            let dp = self.delta_plays;
            let dt = self.delta_time_min;
            let mut counted = false;
            for q in &mut self.daily {
                if q.done || q.def_idx >= QUEST_DEFS.len() {
                    continue;
                }
                let (text, _, ty, need) = QUEST_DEFS[q.def_idx];
                let cur = if ty == 0 { dp } else { dt };
                if cur >= need {
                    // A daily crossing its target counts as a completed
                    // quest (fires once, on the not-done -> done edge).
                    q.done = true;
                    self.st.total_quests += 1;
                    counted = true;
                }
                q.text = format!("{} [{}/{}]", text, cur.min(need), need);
            }
            self.save_daily_quests();
            if counted {
                self.save_state();
            }
            return;
        }

        // New day — generate 6 quests, seeded by the date so the set is
        // stable within a day even if the file is regenerated.
        self.daily.clear();
        self.daily_date = self.today_str();
        let mut seed: u32 = 0;
        for b in self.daily_date.bytes() {
            seed = seed.wrapping_mul(31).wrapping_add(b as u32);
        }
        let mut rng = Lcg::seeded(seed as u64);

        let dp = self.delta_plays;
        let dt = self.delta_time_min;
        let mut used = [false; QUEST_DEFS.len()];

        for _ in 0..DAILY_QUEST_COUNT.min(QUEST_DEFS.len()) {
            let mut idx;
            let mut attempts = 0;
            loop {
                idx = rng.below(QUEST_DEFS.len() as u32) as usize;
                attempts += 1;
                if !used[idx] || attempts >= 50 {
                    break;
                }
            }
            used[idx] = true;
            let (text, xp, ty, need) = QUEST_DEFS[idx];
            let cur = if ty == 0 { dp } else { dt };
            self.daily.push(DailyQuest {
                text: format!("{} [{}/{}]", text, cur.min(need), need),
                xp,
                done: cur >= need,
                def_idx: idx,
            });
        }
        self.save_daily_quests();
    }

    // ----- weekly challenge -----

    fn load_weekly(&mut self) {
        self.weekly = vec![Weekly::default(); WEEKLY_COUNT];
        self.weekly_week = 0;
        let Ok(text) = fs::read_to_string(self.weekly_file()) else {
            return;
        };
        let mut idx: i64 = -1;
        for line in text.lines() {
            if let Some(w) = line.strip_prefix("week=") {
                self.weekly_week = w.trim().parse().unwrap_or(0);
            }
            if line.starts_with('[') {
                idx += 1;
                if idx >= WEEKLY_COUNT as i64 {
                    idx = -1;
                }
                continue;
            }
            if idx < 0 {
                continue;
            }
            let w = &mut self.weekly[idx as usize];
            if let Some(v) = line.strip_prefix("xp=") {
                w.xp = v.trim().parse().unwrap_or(0);
            } else if let Some(v) = line.strip_prefix("target=") {
                w.target = v.trim().parse().unwrap_or(0);
            } else if let Some(v) = line.strip_prefix("progress=") {
                w.progress = v.trim().parse().unwrap_or(0);
            } else if let Some(v) = line.strip_prefix("done=") {
                w.done = v.trim().parse::<i64>().unwrap_or(0) != 0;
            } else if let Some(v) = line.strip_prefix("type=") {
                w.ty = v.trim().parse().unwrap_or(0);
            } else if let Some(v) = line.strip_prefix("text=") {
                w.text = v.to_string();
            }
        }
    }

    fn save_weekly(&self) {
        let _ = fs::create_dir_all(self.dir());
        let mut out = format!("week={}\n", self.weekly_week);
        for (i, w) in self.weekly.iter().enumerate() {
            out.push_str(&format!(
                "[{}]\ntext={}\nxp={}\ntarget={}\nprogress={}\ndone={}\ntype={}\n",
                i,
                w.text,
                w.xp,
                w.target,
                w.progress,
                if w.done { 1 } else { 0 },
                w.ty
            ));
        }
        let _ = fs::write(self.weekly_file(), out);
    }

    fn generate_weekly(&mut self) {
        let cur_week = week_number(&civil_from_epoch(now_epoch(), self.tz_off));
        self.load_weekly();

        if self.weekly_week == cur_week && !self.weekly[0].text.is_empty() {
            // Same week — update progress for each challenge.
            let platforms = self.count_platforms();
            let mut grants = 0;
            for i in 0..WEEKLY_COUNT {
                if self.weekly[i].done {
                    continue;
                }
                let w = &self.weekly[i];
                let cur = match w.ty {
                    0 => self.delta_unique + w.progress,
                    1 => self.delta_time_min + w.progress,
                    2 => self.delta_plays + w.progress,
                    3 => platforms,
                    4 => self.st.streak,
                    _ => w.progress,
                };
                let w = &mut self.weekly[i];
                w.progress = cur;
                if w.progress >= w.target {
                    w.done = true;
                    grants += w.xp;
                    self.st.total_quests += 1;
                }
            }
            if grants > 0 {
                self.grant_xp(grants); // also saves state (total_quests bump)
            }
            self.save_weekly();
            return;
        }

        // New week — pick 3 different challenges, seeded by week number.
        // (Exact C selection arithmetic, kept for determinism.)
        self.weekly_week = cur_week;
        let mut seed: u32 = (cur_week as u32).wrapping_mul(7919);
        let mut used = [false; WEEKLY_DEFS.len()];
        for i in 0..WEEKLY_COUNT {
            let mut idx;
            loop {
                idx = (seed % WEEKLY_DEFS.len() as u32) as usize;
                seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
                if !used[idx] {
                    break;
                }
            }
            used[idx] = true;
            let (text, xp, ty, need) = WEEKLY_DEFS[idx];
            self.weekly[i] = Weekly {
                text: text.to_string(),
                xp,
                target: need,
                progress: 0,
                done: false,
                ty,
            };
        }
        self.save_weekly();
    }

    // ----- dude quests (Revisit / Marathon / Try / Mystery) -----

    fn quest_rel_picked(&self, rel: &str) -> bool {
        self.dude_quests.iter().any(|q| q.rel == rel)
    }

    fn dq_add(&mut self, prefix: &str, name: &str, rel: &str, xp: i64, hidden: bool) {
        if self.dude_quests.len() >= 8 {
            return;
        }
        self.dude_quests.push(DudeQuest {
            view: QuestView {
                text: format!("{prefix}: {name}"),
                xp,
                rom: Some(self.full_rom_path(rel)),
                hidden,
            },
            rel: rel.to_string(),
        });
    }

    /// Uniform random rom from the whole library via reservoir sampling.
    /// Mirrors the C scan: tagged platform folders only, depth <= 3, skip
    /// hidden entries (covers `.media`), junk extensions, cue/m3u data
    /// tracks; inside "(PORTS)" only top-level `.sh` launchers count.
    /// Reservoir-pick one game from the launcher's playable library (the same
    /// games the launcher lists), skipping already-quested and — when asked —
    /// already-played entries.
    fn dq_random_rom(&mut self, skip_played: bool, played: &HashSet<String>) -> Option<String> {
        let already: HashSet<String> = self.dude_quests.iter().map(|q| q.rel.clone()).collect();
        let mut count: u32 = 0;
        let mut pick: Option<String> = None;
        for i in 0..self.library.len() {
            let eligible = {
                let rel = &self.library[i];
                !already.contains(rel) && !(skip_played && played.contains(rel))
            };
            if !eligible {
                continue;
            }
            count += 1;
            if self.rng.below(count) == 0 {
                pick = Some(self.library[i].clone());
            }
        }
        pick
    }


    /// Generate 2 Revisit + 2 Marathon + 2 Try + 1 Mystery, with same-game
    /// collision avoidance; degrades gracefully with a small library.
    fn generate_dude_quests(&mut self) {
        self.dude_quests.clear();
        self.quest_views.clear();
        // No played games yet is fine: Try/Mystery only need the rom scan.
        let n = self.games.len();

        // REVISIT band: the least-recent half (games are last-played DESC).
        if n > 0 {
            let (band_start, band_end) = if n >= 4 { (n / 2, n - 1) } else { (0, n - 1) };
            let band_size = (band_end - band_start + 1) as u32;
            let mut picked = 0;
            for _ in 0..30 {
                if picked >= 2 {
                    break;
                }
                let i = band_start + self.rng.below(band_size) as usize;
                let (rel, name) = (self.games[i].rel.clone(), self.games[i].name.clone());
                if self.quest_rel_picked(&rel) {
                    continue;
                }
                self.dq_add("Revisit", &name, &rel, 40, false);
                picked += 1;
            }
        }

        // MARATHON band: the most-played half by accumulated play_time.
        if n > 0 {
            let mut idx: Vec<usize> = (0..n).collect();
            idx.sort_by(|&a, &b| self.games[b].play_time.cmp(&self.games[a].play_time));
            let top_half = if n >= 4 { n / 2 } else { n };
            let mut picked = 0;
            for _ in 0..30 {
                if picked >= 2 {
                    break;
                }
                let i = idx[self.rng.below(top_half as u32) as usize];
                let (rel, name) = (self.games[i].rel.clone(), self.games[i].name.clone());
                if self.quest_rel_picked(&rel) {
                    continue;
                }
                self.dq_add("Marathon", &name, &rel, 200, false);
                picked += 1;
            }
        }

        // 2 TRY: random UNPLAYED games from the full SD library (any game in
        // the playlog counts as tried). Falls back to played picks when the
        // whole library has been tried already.
        {
            let played: HashSet<String> = self.sessions.iter().map(|s| s.rel.clone()).collect();
            let mut try_picked = 0;
            while try_picked < 2 {
                let Some(fresh) = self.dq_random_rom(true, &played) else {
                    break; // library exhausted (or empty)
                };
                let display = get_display_name(&fresh);
                self.dq_add("Try", &display, &fresh, 50, false);
                try_picked += 1;
            }
            for _ in 0..30 {
                if try_picked >= 2 || n == 0 {
                    break;
                }
                let i = self.rng.below(n as u32) as usize;
                let (rel, name) = (self.games[i].rel.clone(), self.games[i].name.clone());
                if self.quest_rel_picked(&rel) {
                    continue;
                }
                self.dq_add("Try", &name, &rel, 50, false);
                try_picked += 1;
            }
        }

        // 1 MYSTERY: blind pick from the full library — title hidden, higher
        // reward for the leap of faith.
        {
            let empty = HashSet::new();
            if let Some(mystery) = self.dq_random_rom(false, &empty) {
                self.dq_add("Mystery", "???", &mystery, 75, true);
            }
        }

        self.quest_views = self
            .dude_quests
            .iter()
            .map(|q| QuestView {
                text: q.view.text.clone(),
                xp: q.view.xp,
                rom: q.view.rom.clone(),
                hidden: q.view.hidden,
            })
            .collect();
    }

    pub fn dude_quests(&self) -> &[QuestView] {
        &self.quest_views
    }

    /// Accept a dude quest: persists it as pending and returns the rom path
    /// to launch. Credited on a later `open()` once the game gained 5+
    /// minutes of play since acceptance.
    pub fn accept_quest(&mut self, idx: usize) -> Option<PathBuf> {
        if idx >= self.dude_quests.len() {
            return None;
        }
        // If a previous quest was completed but not yet resolved, flush it
        // first so accepting a new quest can't discard earned credit.
        self.resolve_pending_quest();
        let (rel, xp, rom) = {
            let q = &self.dude_quests[idx];
            (q.rel.clone(), q.view.xp, q.view.rom.clone())
        };
        self.save_pending_quest(&rel, xp);
        self.save_state();
        rom
    }

    // ----- pending quest -----

    fn save_pending_quest(&self, rel: &str, xp: i64) {
        let _ = fs::create_dir_all(self.dir());
        let _ = fs::write(
            self.pending_file(),
            format!("path={rel}\nxp={xp}\ntime={}\n", now_epoch()),
        );
    }

    /// Credit the pending quest if its game gained 5+ minutes of play since
    /// acceptance. Keeps the pending file until resolved or stale (>24h).
    fn resolve_pending_quest(&mut self) -> i64 {
        let Ok(text) = fs::read_to_string(self.pending_file()) else {
            return 0;
        };
        let mut game_path = String::new();
        let mut xp_reward: i64 = 0;
        let mut accepted_at: i64 = 0;
        for line in text.lines() {
            if let Some(v) = line.strip_prefix("path=") {
                game_path = v.trim().to_string();
            } else if let Some(v) = line.strip_prefix("xp=") {
                xp_reward = v.trim().parse().unwrap_or(0);
            } else if let Some(v) = line.strip_prefix("time=") {
                accepted_at = v.trim().parse().unwrap_or(0);
            }
        }
        if game_path.is_empty() || xp_reward <= 0 || accepted_at <= 0 {
            let _ = fs::remove_file(self.pending_file()); // malformed — discard
            return 0;
        }

        let target = self.norm_rel(&game_path);
        let played: i64 = self
            .sessions
            .iter()
            .filter(|s| s.rel == target && s.epoch >= accepted_at)
            .map(|s| s.dur)
            .sum();

        if played >= MIN_PLAY_SECS {
            self.grant_xp(xp_reward);
            self.st.total_quests += 1;
            self.save_state();
            let _ = fs::remove_file(self.pending_file()); // consume only on success
            return xp_reward;
        }

        // Not yet credited: keep the pending so a later visit can resolve
        // it; abandon only when stale so an unplayed quest doesn't linger.
        if now_epoch() - accepted_at > 24 * 60 * 60 {
            let _ = fs::remove_file(self.pending_file());
        }
        0
    }

    // ----- views -----

    pub fn daily_lines(&self) -> Vec<String> {
        if self.daily.is_empty() {
            return vec![
                "No daily quests available.".to_string(),
                "Play some games to unlock quests!".to_string(),
            ];
        }
        self.daily
            .iter()
            .map(|q| format!("{} {} +{}xp", if q.done { "[*]" } else { "[ ]" }, q.text, q.xp))
            .collect()
    }

    pub fn weekly_lines(&self) -> Vec<String> {
        self.weekly
            .iter()
            .filter(|w| !w.text.is_empty())
            .map(|w| format!("{} {} +{}xp", if w.done { "[*]" } else { "[ ]" }, w.text, w.xp))
            .collect()
    }

    pub fn completed_lines(&self) -> Vec<String> {
        let done_daily = self.daily.iter().filter(|q| q.done).count() as i64;
        let done_ach = self.achievements.iter().filter(|(d, _, _)| *d).count();
        vec![
            format!("Total quests completed: {}", self.st.total_quests),
            format!("Daily quests done today: {}/{}", done_daily, self.daily.len()),
            format!(
                "Dude quests accepted: {}",
                (self.st.total_quests - done_daily).max(0)
            ),
            format!("Achievements: {}/{}", done_ach, self.achievements.len()),
            format!("Best streak: {} days", self.st.best_streak),
            format!("Total XP earned: {}", self.st.xp),
        ]
    }

    /// Achievements paged 16 per page (3 pages), as (done, name, desc).
    pub fn achievement_pages(&self) -> Vec<Vec<(bool, String, String)>> {
        self.achievements
            .chunks(ACH_PER_PAGE)
            .map(|page| {
                page.iter()
                    .map(|(d, n, s)| (*d, n.to_string(), s.to_string()))
                    .collect()
            })
            .collect()
    }

    pub fn stats_lines(&self) -> Vec<String> {
        let (into, span) = self.xp_progress();
        let pct = if span > 0 { into * 100 / span } else { 100 };
        let mut out = vec![
            format!("Level {} ({}% to next)", self.st.level, pct),
            format!("XP: {} total", self.st.xp),
            format!("Humor: {}/100", self.st.humor),
            format!("Games: {} | Streak: {} days", self.combined_unique(), self.st.streak),
            format!(
                "Quests: {} | Achievements: {}/{}",
                self.st.total_quests,
                self.st.total_achievements,
                self.achievements.len()
            ),
        ];
        // Live RA data is unavailable in the playlog build; show the stored
        // unlock count when the user has any.
        let ra_new: i64 = fs::read_to_string(self.shared.join("kui/ra_unlocks.txt"))
            .ok()
            .and_then(|s2| s2.trim().parse().ok())
            .unwrap_or(0);
        let ra_total = self.st.last_ra_unlocks + ra_new;
        if ra_total > 0 {
            out.push(format!("RA: {ra_total} unlocks"));
        }
        out
    }

    /// Last 20 sessions of 1+ minute, newest first. Each entry is
    /// "Name\nYYYY-MM-DD HH:MM:SS - duration" (caller splits on '\n' for the
    /// two-row layout).
    pub fn history_lines(&self) -> Vec<String> {
        let mut out = Vec::new();
        for s in self.sessions.iter().rev() {
            if s.dur < 60 {
                continue;
            }
            let c = civil_from_epoch(s.epoch, self.tz_off);
            let date = format!(
                "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
                c.year, c.month, c.day, c.hour, c.min, c.sec
            );
            let minutes = s.dur / 60;
            let detail = if minutes >= 60 {
                format!("{} - {}h{:02}m", date, minutes / 60, minutes % 60)
            } else {
                format!("{} - {} min", date, minutes)
            };
            out.push(format!("{}\n{}", s.alias, detail));
            if out.len() >= 20 {
                break;
            }
        }
        out
    }

    /// 28-day activity cells (index 27 = today) and the stats footer.
    pub fn streak_grid(&self) -> ([bool; STREAK_DAYS], String) {
        let mut cells = [false; STREAK_DAYS];
        let today_idx = (now_epoch() + self.tz_off).div_euclid(86400);
        for s in &self.sessions {
            let idx = (s.epoch + self.tz_off).div_euclid(86400);
            let ago = today_idx - idx;
            if (0..STREAK_DAYS as i64).contains(&ago) {
                cells[STREAK_DAYS - 1 - ago as usize] = true;
            }
        }
        let played = cells.iter().filter(|c| **c).count();
        let line = format!(
            "Played {}/28 days | Streak: {} | Best: {}",
            played, self.st.streak, self.st.best_streak
        );
        (cells, line)
    }

    // ----- reset -----

    /// Wipe all Dude progress (two-press confirm is the caller's job).
    pub fn reset(&mut self) {
        for f in [
            "dude.txt",
            "quests.txt",
            "achievements.txt",
            "streak.txt",
            "weekly.txt",
            "pending_quest.txt",
            "daily.txt",
        ] {
            let _ = fs::remove_file(self.dir().join(f));
        }
        let reset_ts = now_epoch();
        self.st = State::default();
        self.st.reset_at = reset_ts;
        self.saved_ach = [false; ACH_COUNT];
        self.games.clear();
        self.daily.clear();
        self.daily_date.clear();
        self.weekly = vec![Weekly::default(); WEEKLY_COUNT];
        self.weekly_week = 0;
        self.dude_quests.clear();
        self.quest_views.clear();
        self.achievements.clear();
        self.delta_plays = 0;
        self.delta_time_min = 0;
        self.delta_unique = 0;
        self.save_state();
    }
}

// =============================================
// Display name: strip the file extension(s) and any trailing
// region/version tags in () or [] to show a clean title.
// =============================================

fn get_display_name(rel: &str) -> String {
    let base = rel.rsplit('/').next().unwrap_or(rel);
    let mut name = base.to_string();

    // remove extension(s), e.g. ".p8.png" — 1-4 letter extension plus dot
    while let Some(dot) = name.rfind('.') {
        let len = name.len() - dot;
        if len > 2 && len <= 5 {
            name.truncate(dot);
        } else {
            break;
        }
    }

    // remove trailing parens (round and square)
    let work = name.clone();
    loop {
        let pos = match (name.rfind('('), name.rfind('[')) {
            (Some(p), _) => Some(p),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        };
        match pos {
            Some(0) | None => break,
            Some(p) => name.truncate(p),
        }
    }
    if name.is_empty() {
        name = work;
    }
    name.trim_end().to_string()
}

// =============================================
// Trivia (verbatim from the C trivia[] array)
// =============================================

const TRIVIA: &[&str] = &[
    // === NES / Famicom (9) ===
    "The NES (Famicom) launched in Japan on July 15, 1983. It sold over 60 million units worldwide and single-handedly saved the gaming industry after the devastating 1983 crash.",
    "The NES had a lockout chip called the 10NES to prevent unlicensed games. Tengen (Atari's subsidiary) reverse-engineered it and got sued by Nintendo in a landmark legal battle.",
    "Super Mario Bros. was bundled with the NES and became the best-selling game for 24 years until Wii Sports took the crown. It defined what a platformer could be.",
    "The NES Zapper light gun worked by briefly flashing the screen black, then drawing white boxes where targets were. The gun detected the light to determine if you hit anything.",
    "Mega Man was almost called 'Rainbow Man' in the West. The original box art for Mega Man 1 is famously terrible and looks nothing like the actual character in the game.",
    "The Legend of Zelda was the first NES game to feature a battery-backed save system. Before that, players had to use passwords or start from scratch every time.",
    "Metroid on NES blew minds when players discovered Samus was a woman. The reveal at the ending was one of gaming's earliest and most iconic plot twists.",
    "Castlevania III on Famicom had an extra sound chip (VRC6) that made its music far superior to the US version. The Japanese soundtrack is considered one of the best on the system.",
    "Mike Tyson's Punch-Out!! was so difficult that beating Tyson became a badge of honor. When his license expired, Nintendo replaced him with 'Mr. Dream' in later prints.",
    // === SNES / Super Famicom (9) ===
    "The SNES sound chip (SPC700) was designed by Sony's Ken Kutaragi. This collaboration eventually led to the PlayStation when the SNES-CD partnership deal fell apart spectacularly.",
    "Chrono Trigger (1995) was created by a 'Dream Team' of Hironobu Sakaguchi (Final Fantasy), Yuji Horii (Dragon Quest), and Akira Toriyama (Dragon Ball). It's still considered one of the greatest RPGs.",
    "Star Fox used the Super FX chip, a coprocessor inside the cartridge, to render 3D polygons. The SNES wasn't designed for 3D, but Nintendo found a way around its limitations.",
    "The SNES could display up to 256 colors on screen from a palette of 32,768. Its Mode 7 effect allowed rotation and scaling of backgrounds, creating pseudo-3D effects in games like F-Zero.",
    "Super Mario World was the SNES launch title and introduced Yoshi. The game had 96 exits and secret paths that kept players exploring for months after release.",
    "EarthBound (Mother 2) was a commercial failure in the West but became a cult classic. Its quirky humor, modern setting, and emotional story influenced countless indie games.",
    "Donkey Kong Country used pre-rendered 3D models compressed into 2D sprites, looking far more advanced than anything else on the SNES. It was Rare's masterpiece.",
    "A Link to the Past established the Zelda formula that lasted for decades: parallel worlds, items that unlock new areas, and dungeons with unique themes and boss fights.",
    "The SNES controller introduced the diamond button layout (A/B/X/Y) and shoulder buttons (L/R). This design became the standard that virtually every controller still follows today.",
    // === Game Boy / GBC (9) ===
    "The original Game Boy had a green-tinted screen with no backlight. Despite this massive limitation, it outsold every color competitor thanks to battery life and its incredible game library.",
    "Tetris was created by Alexey Pajitnov, a Soviet engineer, in 1984. He didn't receive royalties until the fall of the USSR. Bundled with Game Boy, it became the system's killer app.",
    "Pokemon Red/Blue sold 31 million copies and spawned a franchise worth over $100 billion. Satoshi Tajiri was inspired by his childhood hobby of collecting insects in rural Japan.",
    "The Game Boy survived a bombing in the Gulf War. The charred unit, still functional and playing Tetris, is on display at the Nintendo World Store in New York City.",
    "The Game Boy Color was backward compatible with original Game Boy games. Some games like Links Awakening DX added color when played on GBC but still worked on the original.",
    "The Game Boy Camera was the smallest digital camera in the world when it launched in 1998, according to Guinness World Records. It could take 128x112 pixel photos.",
    "Game Boy's Z80-derived CPU ran at just 4.19 MHz, but efficient programming made games like Super Mario Land and Kirbys Dream Land feel smooth and responsive.",
    "The link cable for Game Boy was revolutionary. Pokemon trading and battling created a social gaming phenomenon that predated online multiplayer by over a decade.",
    "Wario Land: Super Mario Land 3 turned Mario's villain into the protagonist. Wario became so popular he got his own series of games spanning multiple Nintendo platforms.",
    // === GBA (9) ===
    "The Game Boy Advance was essentially a portable SNES. Its ARM7 CPU ran at 16.78 MHz and it could display 32,768 colors, bringing console-quality gaming to your pocket.",
    "The GBA had no built-in backlight, making it nearly impossible to play in dim conditions. The GBA SP fixed this with a front-lit (later backlit) screen and a clamshell design.",
    "Metroid Fusion and Metroid Zero Mission on GBA are considered some of the best 2D Metroid games. They refined the formula while adding new mechanics and stunning pixel art.",
    "Final Fantasy Tactics Advance brought deep tactical RPG gameplay to handheld for the first time. Its job system and law mechanics added unique strategic layers.",
    "The GBA could play original Game Boy and Game Boy Color games through backward compatibility. The system's cartridge slot was designed to accept all three cart sizes.",
    "Golden Sun on GBA featured pseudo-3D battle effects that seemed impossible for the hardware. Camelot used sprite scaling tricks to create impressive visual effects.",
    "Pokemon Ruby/Sapphire introduced abilities, natures, and double battles to the series. The GBA generation is when competitive Pokemon really took off.",
    "Advance Wars brought turn-based strategy to a wider audience and spawned a beloved franchise. Its simple rules but deep gameplay made it endlessly replayable.",
    "The GBA's sound system was limited compared to SNES but creative composers like Motoi Sakuraba (Golden Sun) and Hitoshi Sakimoto (FFTA) produced incredible soundtracks anyway.",
    // === Genesis / Mega Drive (9) ===
    "Sega's 'Genesis does what Nintendon't' campaign was one of gaming's most aggressive marketing moves. The console war between Sega and Nintendo defined the entire early 90s.",
    "Sonic the Hedgehog was specifically designed to be edgier and faster than Mario. His attitude and speed perfectly matched Sega's rebellious brand identity.",
    "The Genesis used a Motorola 68000 CPU, the same processor found in early Macintosh computers. This gave it a significant power advantage over the NES.",
    "Streets of Rage 2 is considered one of the greatest beat-em-ups ever made. Yuzo Koshiro composed its soundtrack using a custom music programming language he created.",
    "The Sega CD add-on brought FMV games and CD audio to the Genesis. While many FMV games were terrible, gems like Sonic CD and Lunar made the add-on worthwhile.",
    "Gunstar Heroes by Treasure featured some of the most impressive technical achievements on the Genesis, with massive sprites, rotation effects, and non-stop action.",
    "Phantasy Star IV on Genesis combined manga-style cutscenes with deep JRPG gameplay. The series started on Master System and is one of Sega's most beloved franchises.",
    "The Genesis had a rich library of sports games that defined the genre. EA's Madden and NHL series found their groove on the platform.",
    "Shining Force on Genesis helped popularize tactical RPGs outside Japan. The series combined Fire Emblem-style strategy with traditional JRPG town exploration.",
    // === Atari 2600 (6) ===
    "The Atari 2600 had just 128 bytes of RAM and no frame buffer. Developers had to draw each scanline in real-time, a technique called 'racing the beam.' The creativity required was incredible.",
    "E.T. for the Atari 2600 is often called the worst game ever made. Millions of unsold copies were famously buried in a New Mexico landfill and excavated in 2014.",
    "Pitfall! on the Atari 2600 is considered one of the first platforming games. David Crane programmed it in just 1,000 bytes of ROM and it sold over 4 million copies.",
    "The Atari 2600 was originally called the Atari VCS (Video Computer System). It launched in 1977 and remained in production until 1992, an impressive 15-year lifespan.",
    "Adventure on Atari 2600 contained the first known Easter egg in gaming: programmer Warren Robinett hid his name in a secret room because Atari didn't credit developers.",
    "River Raid by Carol Shaw (one of gaming's first female programmers) was so realistic that it was banned in West Germany for being a 'war preparation' game.",
    // === Master System (6) ===
    "The Sega Master System outsold the NES in Brazil and Europe but struggled in North America and Japan. In Brazil, it remained popular well into the 2000s through Tec Toy.",
    "Alex Kidd in Miracle World was built into later Master System consoles as a pack-in game. He was Sega's mascot before Sonic the Hedgehog took over.",
    "The Master System had superior hardware to the NES but a weaker game library. It could display more colors and had better sound, but third-party support was limited.",
    "Phantasy Star on Master System was one of the first console RPGs with a female protagonist. It featured first-person dungeon crawling and a sci-fi setting years ahead of its time.",
    "The Master System's light gun, the Light Phaser, was more advanced than the NES Zapper. Games like Safari Hunt and Gangster Town showcased its accuracy.",
    "Wonder Boy III: The Dragon's Trap on Master System recently got a beautiful remake. The original game's transformation mechanics were revolutionary for its time.",
    // === Game Gear (6) ===
    "The Game Gear used the same CPU as the Master System and could play SMS games with an adapter. It ate 6 AA batteries in about 3 hours, making it an expensive habit.",
    "The Game Gear had a full-color backlit screen in 1990, years before Nintendo offered anything similar. The display was impressive but crushed battery life.",
    "Sonic the Hedgehog on Game Gear was not a port of the Genesis version but an entirely different game. The handheld Sonic games had their own unique level designs.",
    "The Game Gear's TV Tuner accessory turned it into a portable television. It was one of the first handheld devices that could receive broadcast TV signals.",
    "Columns on Game Gear was Sega's answer to Tetris. The jewel-matching puzzle game was a pack-in title in some regions and became synonymous with the handheld.",
    "Despite selling 11 million units, the Game Gear is considered a commercial disappointment compared to the Game Boy's 118 million. Battery life was its biggest weakness.",
    // === TurboGrafx-16 / PC Engine (6) ===
    "The TurboGrafx-16 (PC Engine) was the first 16-bit console, beating the Genesis by two years in Japan. It was also the first console with a CD-ROM add-on.",
    "The PC Engine was tiny. The Japanese CoreGrafx model was roughly the size of a CD case. Despite its small size, it packed impressive hardware capabilities.",
    "Bonk's Adventure was the TurboGrafx-16's mascot game. The bald caveman who headbutted enemies became an icon of 90s gaming, though he never reached Mario or Sonic fame.",
    "The PC Engine's HuCard format used credit-card-sized cards instead of bulky cartridges. This unique format contributed to the console's compact design.",
    "R-Type on PC Engine is considered one of the best home ports of the arcade classic. The game was split across two HuCards due to its massive size.",
    "Ys Book I & II on the TurboGrafx-CD featured full voice acting and CD-quality music, something unheard of in 1989. It set the standard for CD-ROM RPGs.",
    // === Neo Geo (6) ===
    "Neo Geo AES games cost $200-$600 at launch because they were identical to the arcade versions. The console itself was $649 in 1990, making it the ultimate luxury gaming item.",
    "The Neo Geo MVS (arcade) and AES (home) used identical hardware. You were literally playing the exact same game at home that people paid quarters to play in arcades.",
    "Metal Slug on Neo Geo features some of the most detailed pixel art ever created. Each frame of animation was hand-drawn, giving the game its iconic fluid movement style.",
    "King of Fighters on Neo Geo pioneered the team-based fighting game format. The series ran annually from 1994 to 2003, each entry refining the 3-on-3 formula.",
    "The Neo Geo Pocket Color was SNK's handheld answer to the Game Boy Color. Despite excellent games like SNK vs. Capcom, it couldn't compete and was discontinued quickly.",
    "Samurai Shodown on Neo Geo introduced weapon-based combat to fighting games. Its emphasis on single powerful strikes over combos made it unique in the genre.",
    // === WonderSwan (3) ===
    "The WonderSwan was designed by Gunpei Yokoi after he left Nintendo. It could run on a single AA battery for 30+ hours and was huge in Japan, though never released in the West.",
    "The WonderSwan Color received exclusive Final Fantasy remakes (I, II, and IV) that drove sales in Japan. Square's support gave the handheld credibility against the Game Boy.",
    "The WonderSwan could be held both horizontally and vertically, making it ideal for different game genres. This dual-orientation design was innovative for its time.",
    // === Vectrex (3) ===
    "The Vectrex (1982) was the only home console with a built-in vector display. Every game came with a screen overlay for color since the display was monochrome.",
    "The Vectrex had a thriving homebrew scene that continues today. New games are still being developed for the system over 40 years after its release.",
    "Mine Storm, the built-in game on every Vectrex, was an Asteroids clone that showed off the sharp vector graphics perfectly. No cartridge needed to start playing.",
    // === Virtual Boy (3) ===
    "Nintendo's Virtual Boy used red LEDs because they were the cheapest. Gunpei Yokoi was pressured to release it early despite it being unfinished. It sold only 770,000 units.",
    "Despite its commercial failure, Virtual Boy had some genuinely good games. Virtual Boy Wario Land is considered one of the best platformers that most people have never played.",
    "The Virtual Boy's stereoscopic 3D was actually impressive technology for 1995. The problem wasn't the concept but the execution: red-only display, no portability, and headaches.",
    // === Atari Lynx (3) ===
    "The Atari Lynx (1989) was the first handheld with a color LCD and hardware sprite scaling. It was years ahead of its time but couldn't compete with the Game Boy's price and battery life.",
    "The Lynx could be flipped upside-down for left-handed players, swapping the D-pad and buttons. This accessibility feature was unique and thoughtful for its era.",
    "California Games on the Lynx was a standout title that showed off the handheld's color display and scaling capabilities with smooth surfing and BMX gameplay.",
    // === Atari 7800 (3) ===
    "The Atari 7800 was designed in 1984 but shelved for two years due to Atari's financial troubles. By the time it launched in 1986, the NES had already dominated the market.",
    "The 7800 was fully backward compatible with the Atari 2600, giving it instant access to a library of hundreds of games on day one.",
    "The 7800 had no dedicated sound chip and relied on the 2600's TIA chip for audio. This meant its sound was essentially stuck in 1977, despite improved graphics.",
    // === ColecoVision (3) ===
    "The ColecoVision launched in 1982 with a port of Donkey Kong that was the closest thing to arcade-perfect at home. It was the console's killer app and system seller.",
    "ColecoVision's Expansion Module #1 made it compatible with Atari 2600 games. Atari sued Coleco for this, leading to one of gaming's first major legal battles.",
    "The ColecoVision had impressive hardware for 1982 with 16 colors, 32 sprites, and a Texas Instruments CPU. It could have been huge if not for the 1983 crash.",
    // === Intellivision (3) ===
    "The Intellivision was Mattel's answer to the Atari 2600. It had superior graphics and was marketed as the 'intelligent television' with a focus on sports games.",
    "Intellivision's controller featured a 16-direction disc and a 12-button keypad with game-specific overlays. It was innovative but often criticized as uncomfortable.",
    "Utopia on Intellivision (1982) is considered one of the first simulation/god games ever made, predating SimCity by seven years. It let two players compete to build civilizations.",
    // === 3DO (3) ===
    "The 3DO was designed by Trip Hawkins, founder of EA. Multiple manufacturers built 3DO consoles, but at $699 it was far too expensive to compete with the PlayStation and Saturn.",
    "The 3DO had an unusual licensing model: any manufacturer could build a 3DO console. Panasonic, Goldstar, and Sanyo all produced their own versions of the hardware.",
    "Road Rash on 3DO is considered the definitive version of the motorcycle combat racing game. The full-motion video intros and CD-quality soundtrack made it a showcase title.",
    // === Jaguar (3) ===
    "Atari marketed the Jaguar as the first 64-bit console, though this claim was debatable. Its 'Tom' and 'Jerry' chips were powerful but extremely difficult to program for.",
    "Tempest 2000 on Jaguar by Jeff Minter is considered one of the greatest remakes ever made. Its psychedelic visuals and trance soundtrack defined a new genre of arcade shooters.",
    "The Jaguar's controller had a phone-style number pad and game-specific overlays, similar to the Intellivision. Despite its advanced hardware, the system only sold 250,000 units.",
    // === 32X (3) ===
    "The Sega 32X was an add-on that plugged into the Genesis cartridge slot to boost its power. It launched just months before the Saturn, confusing consumers and splitting Sega's market.",
    "Knuckles' Chaotix was a 32X exclusive featuring Sonic's rival Knuckles. Its rubber-band partner mechanic was unique but divisive among fans of the series.",
    "The 32X had dual SH-2 processors, the same chips used in the Saturn. Despite this power, its tiny library of about 40 games doomed it to commercial failure.",
    // === Sega CD (3) ===
    "The Sega CD added CD-ROM capabilities to the Genesis. While it brought FMV games and CD audio, its best titles were Sonic CD, Lunar, and Snatcher by Hideo Kojima.",
    "Sonic CD introduced Metal Sonic and Amy Rose to the franchise. Its time-travel mechanic let players alter levels between past, present, and future versions.",
    "The Sega CD's most infamous games were FMV titles like Night Trap and Sewer Shark. Despite being cheesy, Night Trap sparked Congressional hearings about violence in games.",
    // === Neo Geo Pocket (3) ===
    "The Neo Geo Pocket Color had one of the best microswitched joysticks ever put on a handheld. Its clicky directional input was perfect for fighting games like SNK vs. Capcom.",
    "SNK vs. Capcom: Match of the Millennium on NGPC is considered one of the best portable fighting games ever made, with a massive roster and deep gameplay systems.",
    "The Neo Geo Pocket Color could link with the Dreamcast for bonus content in some games. This cross-platform connectivity was ahead of its time for the late 90s.",
    // === Neo Geo CD (3) ===
    "The Neo Geo CD was SNK's affordable alternative to the expensive AES cartridge system. Games cost $50 instead of $200+, but load times were painfully long.",
    "The Neo Geo CD used a single-speed CD drive, making load times between rounds in fighting games sometimes exceed 30 seconds. SNK addressed this with the faster CDZ model.",
    "Despite the loading issues, the Neo Geo CD had exclusive content including arranged soundtracks and bonus features not found in the cartridge versions.",
    // === MSX (3) ===
    "The MSX was a standardized home computer platform popular in Japan, Europe, and South America. Konami developed Metal Gear and Castlevania games specifically for MSX.",
    "Metal Gear by Hideo Kojima debuted on MSX2 in 1987. The NES version was a different, inferior port that Kojima had no involvement with. The MSX original is the definitive version.",
    "The MSX standard was created by Microsoft and ASCII Corporation. Multiple manufacturers built MSX computers, similar to how the IBM PC standard worked.",
    // === Commodore 64 (6) ===
    "The Commodore 64 is the best-selling single computer model of all time with 12.5 to 17 million units sold. Its SID sound chip produced iconic music that spawned its own genre.",
    "The C64's SID chip, designed by Bob Yannes, was so musically capable that artists still compose SID music today. Yannes later founded Ensoniq, a professional synthesizer company.",
    "The C64 had a massive game library of over 10,000 titles. Classics like Impossible Mission, Maniac Mansion, and The Last Ninja defined home computer gaming in the 1980s.",
    "Loading games from cassette tape on the C64 could take 5-20 minutes. The disk drive was faster but cost almost as much as the computer itself.",
    "The C64 was so popular in Germany and Scandinavia that it created a distinct gaming culture. The demoscene, which pushed hardware to its limits with audiovisual demos, was born on the C64.",
    "Commodore sold the C64 in toy stores and department stores, not just computer shops. This retail strategy helped it reach a much wider audience than competing computers.",
    // === Amstrad CPC (3) ===
    "The Amstrad CPC was hugely popular in the UK, France, and Spain. It came bundled with a monitor, making it a complete ready-to-use system right out of the box.",
    "The CPC's hardware color palette included 27 colors, and it could display up to 16 on screen. Games like Gryzor and Rick Dangerous were CPC classics.",
    "Amstrad CPC games were distributed on cassette tapes and 3-inch floppy disks. The non-standard 3-inch disk format was unique to Amstrad and Hitachi computers.",
    // === DOS (3) ===
    "DOS gaming defined PC gaming from the mid-80s through the mid-90s. Classics like DOOM, Commander Keen, and Oregon Trail all ran on MS-DOS.",
    "DOOM (1993) by id Software revolutionized gaming with its fast 3D engine, network multiplayer, and modding support. It popularized the first-person shooter genre worldwide.",
    "LucasArts adventure games like Monkey Island, Day of the Tentacle, and Indiana Jones ran on DOS using the SCUMM engine. These point-and-click classics are still beloved today.",
    // === Amiga (3) ===
    "The Amiga was decades ahead of its time with preemptive multitasking, stereo sound, and a custom chipset for graphics. It was the multimedia computer before multimedia was a word.",
    "The Amiga demoscene pushed the hardware to impossible limits. Demos featured 3D graphics, real-time effects, and music that seemed to defy the hardware's specifications.",
    "Amiga CD32 was the first 32-bit CD-based console, launching in 1993. Despite innovative hardware, Commodore's bankruptcy killed it before it could gain momentum.",
    // === Channel F (3) ===
    "The Fairchild Channel F (1976) was the first home console to use interchangeable ROM cartridges. It predated the Atari 2600 by a full year.",
    "Channel F games used 'Videocarts' — bright yellow cartridges with built-in labels. The console's innovative design influenced every cartridge-based system that followed.",
    "The Channel F featured a unique joystick that could be pushed, pulled, twisted, and moved in eight directions. It was remarkably advanced for 1976.",
    // === Odyssey 2 (3) ===
    "The Magnavox Odyssey 2 (Philips Videopac in Europe) had a built-in keyboard, making it both a game console and a simple computer.",
    "Quest for the Rings on Odyssey 2 came with a board game component. Players combined the video game with physical game pieces for a hybrid experience.",
    "The Odyssey 2 had a distinctive synthesized voice capability. Games like K.C. Munchkin used speech synthesis, which was rare and impressive for early 80s hardware.",
    // === SG-1000 (3) ===
    "The SG-1000 was Sega's first home console, launching in Japan on the same day as the Famicom: July 15, 1983. It was completely overshadowed by Nintendo's machine.",
    "The SG-1000 evolved into the Mark III, which became the Master System in the West. Sega's console lineage started here before Genesis and Dreamcast.",
    "Girl's Garden on SG-1000 was designed by Yuji Naka, who would later create Sonic the Hedgehog and Nights into Dreams. Everyone starts somewhere!",
    // === Pokemon Mini (3) ===
    "The Pokemon Mini is the smallest cartridge-based game system ever made. It fits in the palm of your hand and was sold exclusively as a Pokemon-branded device.",
    "Despite its tiny size, the Pokemon Mini had a built-in accelerometer, rumble motor, and infrared communication. It was surprisingly feature-rich for such a small device.",
    "Only 10 games were officially released for the Pokemon Mini, but its homebrew scene has produced dozens more. The system has a dedicated fan community to this day.",
    // === PICO-8 (3) ===
    "PICO-8 is a fantasy console created by Lexaloffle Games. It has intentional limitations: 128x128 resolution, 16 colors, and 32KB cartridges, fostering creativity through constraint.",
    "Celeste, the acclaimed precision platformer, started as a PICO-8 game jam entry. The tiny original version captured the essence of what became a major indie hit.",
    "PICO-8 games are shared as PNG images with code embedded in the pixels. You can literally tweet a playable game as a small image file.",
    // === FDS (3) ===
    "The Famicom Disk System used rewritable disks that could be updated at kiosks in Japanese stores. You could buy a blank disk and write a new game onto it for less than a cartridge.",
    "The Legend of Zelda, Metroid, and Kid Icarus all debuted on the Famicom Disk System before being converted to NES cartridges for Western release.",
    "The FDS added an extra sound channel to the Famicom. Games like Zelda and Castlevania had enhanced music on the disk version compared to the cartridge ports.",
    // === Supervision (3) ===
    "The Watara Supervision was a budget Game Boy competitor from Hong Kong. Despite its low price, it couldn't match the Game Boy's game library or build quality.",
    "The Supervision had a TV link cable that could output games to a television, turning the handheld into a home console. This feature was unusual for portable systems.",
    "The Supervision's screen was slightly larger than the Game Boy's but suffered from heavy ghosting. Its library of about 65 games included many simple but addictive puzzlers.",
    // === Mega Duck (3) ===
    "The Mega Duck (Cougar Boy in some regions) was a budget handheld made by Welback Holdings. It was sold in European department stores as a cheap alternative to the Game Boy.",
    "Mega Duck cartridges are not compatible with Game Boy despite similar hardware. The system used its own proprietary cartridge format with a different pinout.",
    "The Mega Duck had about 30 games, mostly simple action and puzzle titles. Some were ports from other budget handhelds, repackaged for the Mega Duck market.",
    // === ZX Spectrum (9) ===
    "The ZX Spectrum was designed by Sinclair Research and launched in 1982 for just 125 pounds. Its rubber-key design became one of the most recognizable computers ever made.",
    "The Spectrum's attribute system gave each 8x8 pixel block only two colors, causing the infamous 'color clash' effect. Developers turned this limitation into an art form.",
    "Over 24,000 software titles were released for the ZX Spectrum, making it one of the most prolific platforms in computing history. Most were distributed on cassette tape.",
    "Loading a game on the Spectrum meant playing a cassette tape and waiting several minutes while the screen showed colorful stripes. The loading sounds became iconic.",
    "The Spectrum spawned the UK games industry. Studios like Ultimate Play the Game (later Rare), Ocean Software, and Codemasters all started making Spectrum games.",
    "Jet Set Willy and Manic Miner by Matthew Smith were among the first celebrity game developer stories. Smith became famous at 17 and then mysteriously disappeared from the industry.",
    "The Spectrum used a Z80 processor at 3.5MHz with just 48KB of RAM. Despite these specs, developers created remarkably sophisticated games including 3D titles like Elite.",
    "In the Soviet Union, the Spectrum was widely cloned as unlicensed machines like the Pentagon and Scorpion. These clones kept the platform alive well into the 1990s.",
    "The Spectrum's BASIC interpreter was built into ROM, letting users start programming the moment they turned it on. An entire generation learned to code on this machine.",
    // === Atari 800 (6) ===
    "The Atari 800 was one of the first home computers with custom graphics and sound chips. ANTIC and POKEY gave it capabilities that rivaled arcade machines of the era.",
    "Star Raiders on the Atari 800 was a groundbreaking first-person space combat game released in 1979. It featured a galactic map, shields, and warp drive years before Elite.",
    "The Atari 800's GTIA chip could display up to 256 colors, far more than contemporary computers. This made it a favorite for graphically intensive games and demos.",
    "Atari BASIC was included with the 800, but serious developers used assembly language. The machine's custom chips rewarded low-level programming with stunning visual effects.",
    "The Atari 800 series introduced the SIO (Serial I/O) interface, a daisy-chainable peripheral bus that was ahead of its time and inspired later serial bus designs.",
    "M.U.L.E. by Ozark Softscape, released in 1983 for the Atari 800, is considered one of the greatest multiplayer strategy games ever made. Four players competed to colonize a planet.",
    // === Arduboy (3) ===
    "The Arduboy is a credit-card sized handheld game console based on the Arduino platform. Its 128x64 pixel OLED screen and open-source design make it a favorite among indie developers.",
    "Arduboy games are limited to 28KB of flash memory and 2.5KB of RAM, forcing developers to be incredibly creative with optimization. Some have packed full RPGs into this tiny space.",
    "The Arduboy community has produced over 300 games, all free and open source. The constraints of the hardware have inspired a unique pixel art aesthetic in pure black and white.",
    // === Uzebox (3) ===
    "The Uzebox is a retro-minimalist open source game console created by Alec Bourque in 2008. It uses a single ATmega644 microcontroller with no GPU — all video is generated in software.",
    "Uzebox generates its video signal entirely through software interrupts, producing a 256x224 resolution with up to 256 colors. This was considered impossible on a single 8-bit chip.",
    "The Uzebox community has created over 100 games including impressive ports of classic titles. The entire system fits on a single circuit board smaller than a credit card.",
    // === FinalBurn / Arcade (3) ===
    "Arcade games in the 80s and 90s used custom hardware that home consoles couldn't match. The gap between arcade and home was a driving force behind console innovation.",
    "CPS-1 and CPS-2 arcade boards by Capcom powered classics like Street Fighter II, Marvel vs. Capcom, and Dungeons & Dragons. Their sprite work remains some of the finest ever created.",
    "Neo Geo MVS arcade cabinets could hold multiple games on one board, letting arcade operators swap titles easily. This multi-slot design was unique and cost-effective.",
];

