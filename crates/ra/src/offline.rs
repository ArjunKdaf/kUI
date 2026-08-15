//! Offline support for kui-ra.
//!
//! Three pieces:
//!
//! 1. A transparent response cache in the HTTP transport
//!    ([`cached_transport`], installed by `RaClient::new`). Successful
//!    responses to the session-bootstrap requests (`login2`, `gameid`,
//!    `patch`, `achievementsets`, `startsession`) are written to
//!    `<offline root>/cache/<reqtype>_<stable-id>.json`; on a network
//!    failure the cached body is replayed as a synthetic 200 so login and
//!    game load keep working without connectivity.
//! 2. An offline unlock queue (`<offline root>/queue.txt`): when an
//!    `awardachievement` POST fails at the network level its body is
//!    appended to the queue and a synthetic success is returned so play
//!    continues. [`flush_unlock_queue`] replays the queue when back online.
//! 3. Standalone helpers: [`hash_rom`] (rc_hash) and [`prefetch_game`]
//!    (populate the cache for a ROM ahead of time).
//!
//! The default offline root is [`DEFAULT_OFFLINE_ROOT`]; tests (or the
//! frontend) can redirect it with [`set_offline_root`].

use std::ffi::{CString, c_char, c_int, c_void};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::{HttpRequest, HttpResponse, curl_http};

/// Where cache/ and queue.txt live on the device.
pub const DEFAULT_OFFLINE_ROOT: &str = "/mnt/SDCARD/.userdata/shared/.ra/offline";

const DOREQUEST_URL: &str = "https://retroachievements.org/dorequest.php";
const FORM_CONTENT_TYPE: &str = "application/x-www-form-urlencoded";
/// Matches vendor/rcheevos/src/rc_version.h (only used for the `l=` param).
const RCHEEVOS_VERSION_STRING: &str = "12.3.0";

// ---------------------------------------------------------------------------
// Offline root (overridable for tests)
// ---------------------------------------------------------------------------

static OFFLINE_ROOT: Mutex<Option<PathBuf>> = Mutex::new(None);

fn lock_root() -> std::sync::MutexGuard<'static, Option<PathBuf>> {
    match OFFLINE_ROOT.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// Override the offline root directory (cache + unlock queue). `None`
/// restores the device default, [`DEFAULT_OFFLINE_ROOT`]. Intended for
/// tests and desktop runs.
pub fn set_offline_root(root: Option<&Path>) {
    *lock_root() = root.map(Path::to_path_buf);
}

fn offline_root() -> PathBuf {
    lock_root()
        .clone()
        .unwrap_or_else(|| PathBuf::from(DEFAULT_OFFLINE_ROOT))
}

fn cache_dir() -> PathBuf {
    offline_root().join("cache")
}

fn queue_path() -> PathBuf {
    offline_root().join("queue.txt")
}

// ---------------------------------------------------------------------------
// Form (application/x-www-form-urlencoded) helpers
// ---------------------------------------------------------------------------

/// Percent-encode for a form body: unreserved characters pass through,
/// everything else becomes %XX per UTF-8 byte.
pub(crate) fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => {
                out.push('%');
                out.push(hex_digit(b >> 4));
                out.push(hex_digit(b & 0xf));
            }
        }
    }
    out
}

fn hex_digit(n: u8) -> char {
    match n {
        0..=9 => (b'0' + n) as char,
        _ => (b'A' + n - 10) as char,
    }
}

fn hex_value(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// Decode one form-encoded value (`+` -> space, `%XX` -> byte). Malformed
/// escapes pass through literally; the result is lossily UTF-8 decoded.
fn form_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                match (hex_value(bytes[i + 1]), hex_value(bytes[i + 2])) {
                    (Some(hi), Some(lo)) => {
                        out.push((hi << 4) | lo);
                        i += 3;
                    }
                    _ => {
                        out.push(b'%');
                        i += 1;
                    }
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Parse `k=v&k2=v2` pairs (form-decoded). Pairs without `=` are skipped.
fn form_params(body: &str) -> Vec<(String, String)> {
    body.split('&')
        .filter_map(|pair| {
            let (k, v) = pair.split_once('=')?;
            Some((form_decode(k), form_decode(v)))
        })
        .collect()
}

fn param<'a>(params: &'a [(String, String)], key: &str) -> Option<&'a str> {
    params
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

/// The `r=` request type of a dorequest POST body, if present.
fn request_type(post_body: &str) -> Option<String> {
    let params = form_params(post_body);
    param(&params, "r").map(str::to_owned)
}

// ---------------------------------------------------------------------------
// Cache keys
// ---------------------------------------------------------------------------

/// Keep cache file names filesystem-safe: RA usernames / hashes / ids are
/// alphanumeric already, anything else becomes '_'. Capped at 120 chars.
fn sanitize_id(id: &str) -> String {
    id.chars()
        .take(120)
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Cache file name (`<reqtype>_<stable-id>.json`) for a dorequest POST
/// body, or `None` when the request type is not cacheable.
///
/// Cacheable types and their stable ids:
/// - `login2`          -> `u` (username)
/// - `gameid`          -> `m` (ROM hash)
/// - `patch`           -> `g` (game id)
/// - `achievementsets` -> `g` if present, else `m` (this vendored
///   rc_client loads game data via achievementsets, usually by hash)
/// - `startsession`    -> `g` (game id)
///
/// `awardachievement` and `ping` are deliberately never cacheable.
fn cache_key(post_body: &str) -> Option<String> {
    let params = form_params(post_body);
    let rtype = param(&params, "r")?;
    let id = match rtype {
        "login2" => param(&params, "u")?,
        "gameid" => param(&params, "m")?,
        "patch" => param(&params, "g")?,
        "achievementsets" => param(&params, "g").or_else(|| param(&params, "m"))?,
        "startsession" => param(&params, "g")?,
        _ => return None,
    };
    let id = sanitize_id(id);
    if id.is_empty() {
        return None;
    }
    Some(format!("{rtype}_{id}.json"))
}

// ---------------------------------------------------------------------------
// Cache file I/O
// ---------------------------------------------------------------------------

fn write_cache(key: &str, body: &[u8]) {
    let dir = cache_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let path = dir.join(key);
    let tmp = dir.join(format!("{key}.tmp"));
    if std::fs::write(&tmp, body).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

fn read_cache(key: &str) -> Option<Vec<u8>> {
    let body = std::fs::read(cache_dir().join(key)).ok()?;
    if body.is_empty() { None } else { Some(body) }
}

// ---------------------------------------------------------------------------
// Response inspection (tiny, format-tolerant JSON probes; the real parsing
// is done by rc_api on the C side)
// ---------------------------------------------------------------------------

/// True when the body contains `"Success"` with the value `true`.
fn body_reports_success(body: &[u8]) -> bool {
    let text = String::from_utf8_lossy(body);
    match json_value_start(&text, "Success") {
        Some(rest) => rest.trim_start().starts_with("true"),
        None => false,
    }
}

/// True when the body contains a `"Success"` key at all (any value).
fn body_has_success_key(body: &[u8]) -> bool {
    String::from_utf8_lossy(body).contains("\"Success\"")
}

/// Slice of `text` starting just after `"key"` and its `:`.
fn json_value_start<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    let needle = format!("\"{key}\"");
    let idx = text.find(&needle)?;
    let rest = text[idx + needle.len()..].trim_start();
    rest.strip_prefix(':')
}

/// First `"key": <unsigned number>` in the body.
fn json_uint(text: &str, key: &str) -> Option<u64> {
    let rest = json_value_start(text, key)?.trim_start();
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// First `"key": "string"` in the body (handles `\"` and `\\` escapes).
fn json_string(text: &str, key: &str) -> Option<String> {
    let rest = json_value_start(text, key)?.trim_start();
    let rest = rest.strip_prefix('"')?;
    let mut out = String::new();
    let mut chars = rest.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(out),
            '\\' => match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some(other) => out.push(other), // covers \" \\ \/ well enough
                None => return Some(out),
            },
            other => out.push(other),
        }
    }
    Some(out)
}

// ---------------------------------------------------------------------------
// Unlock queue
// ---------------------------------------------------------------------------

/// Append an awardachievement POST body to the offline queue (one line per
/// unlock, deduplicated, single atomic append write).
fn queue_unlock(post_body: &str) {
    let line = post_body.trim();
    if line.is_empty() {
        return;
    }
    let path = queue_path();
    if let Some(dir) = path.parent()
        && std::fs::create_dir_all(dir).is_err() {
            return;
        }
    if let Ok(existing) = std::fs::read_to_string(&path)
        && existing.lines().any(|l| l.trim() == line) {
            return; // already queued
        }
    use std::io::Write as _;
    if let Ok(mut f) = std::fs::OpenOptions::new().append(true).create(true).open(&path) {
        let _ = f.write_all(format!("{line}\n").as_bytes());
    }
}

/// Replay the offline unlock queue against the server.
///
/// Each queued line is POSTed directly (same curl transport rc_client
/// uses). A line is consumed when the server answers HTTP 200 with a
/// `"Success"` key in the body — that covers both fresh unlocks
/// (`"Success":true`) and already-earned rejections (`"Success":false`,
/// "User already has..."). Genuine network failures keep the line queued
/// for next time.
///
/// Returns `(sent, remaining)`: lines consumed this call, lines still
/// queued afterwards.
pub fn flush_unlock_queue() -> (usize, usize) {
    let path = queue_path();
    let Ok(content) = std::fs::read_to_string(&path) else {
        return (0, 0);
    };
    let mut sent = 0usize;
    let mut kept: Vec<String> = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let req = HttpRequest {
            url: DOREQUEST_URL.to_string(),
            post_data: Some(line.to_string()),
            content_type: Some(FORM_CONTENT_TYPE.to_string()),
        };
        match curl_http(&req) {
            Ok(resp) if resp.status == 200 && body_has_success_key(&resp.body) => sent += 1,
            _ => kept.push(line.to_string()),
        }
    }
    let remaining = kept.len();
    if kept.is_empty() {
        let _ = std::fs::remove_file(&path);
    } else {
        // Atomic rewrite: tmp + rename.
        let tmp = path.with_extension("txt.tmp");
        let mut joined = kept.join("\n");
        joined.push('\n');
        if std::fs::write(&tmp, joined).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }
    (sent, remaining)
}

// ---------------------------------------------------------------------------
// The cached transport (installed as RaClient's HTTP hook)
// ---------------------------------------------------------------------------

/// `curl_http` wrapped with the offline cache and unlock queue. This is the
/// transport `RaClient` installs; [`prefetch_game`] routes through it too
/// so prefetched responses land in the same cache.
pub(crate) fn cached_transport(req: &HttpRequest) -> Result<HttpResponse, String> {
    let post = req.post_data.as_deref().unwrap_or("");
    let key = cache_key(post);

    let net = curl_http(req);
    let net_failed = match &net {
        Err(_) => true,
        // An empty body / nonsense status from a captive portal or dead
        // link is a network failure for our purposes.
        Ok(resp) => resp.status < 100 || resp.body.is_empty(),
    };

    if !net_failed {
        if let Ok(resp) = &net
            && resp.status == 200 && body_reports_success(&resp.body)
                && let Some(key) = &key {
                    write_cache(key, &resp.body);
                }
        return net;
    }

    // Network-level failure from here on.
    if request_type(post).as_deref() == Some("awardachievement") {
        // Queue the unlock and pretend it succeeded so rc_client does not
        // retry-loop; flush_unlock_queue() delivers it later.
        queue_unlock(post);
        let params = form_params(post);
        let ach_id = param(&params, "a")
            .and_then(|a| a.parse::<u64>().ok())
            .unwrap_or(0);
        let body = format!(
            "{{\"Success\":true,\"Score\":0,\"SoftcoreScore\":0,\"AchievementID\":{ach_id}}}"
        );
        return Ok(HttpResponse {
            status: 200,
            body: body.into_bytes(),
        });
    }

    if let Some(key) = &key
        && let Some(body) = read_cache(key) {
            return Ok(HttpResponse { status: 200, body });
        }

    // No cache to fall back on: preserve the retryable-error behavior.
    match net {
        Err(e) => Err(e),
        Ok(resp) => Err(format!(
            "unusable response (status {}, {} bytes) for {}",
            resp.status,
            resp.body.len(),
            req.url
        )),
    }
}

// ---------------------------------------------------------------------------
// rc_hash FFI (iterator API from the vendored rc_hash.h)
// ---------------------------------------------------------------------------

/// rc_hash_callbacks_t: verbose/error message callbacks, a 5-function
/// filereader, a 5-function cdreader and 2 encryption callbacks (the
/// build defines neither RC_HASH_NO_DISC nor RC_HASH_NO_ENCRYPTED).
#[repr(C)]
struct RcHashCallbacks {
    verbose_message: *mut c_void,
    error_message: *mut c_void,
    filereader: [*mut c_void; 5],
    cdreader: [*mut c_void; 5],
    encryption: [*mut c_void; 2],
}

/// rc_hash_iterator_t (rc_hash.h). `_reserved` is a safety margin: the C
/// side memsets sizeof(its own struct) in rc_hash_reset_iterator, so keep
/// spare room in case a future vendored header grows the layout.
#[repr(C)]
struct RcHashIterator {
    buffer: *const u8,
    buffer_size: usize,
    consoles: [u8; 12],
    index: c_int,
    path: *const c_char,
    userdata: *mut c_void,
    callbacks: RcHashCallbacks,
    _reserved: [u8; 64],
}

unsafe extern "C" {
    fn rc_hash_initialize_iterator(
        iterator: *mut RcHashIterator,
        path: *const c_char,
        buffer: *const u8,
        buffer_size: usize,
    );
    fn rc_hash_destroy_iterator(iterator: *mut RcHashIterator);
    fn rc_hash_iterate(hash: *mut c_char, iterator: *mut RcHashIterator) -> c_int;
}

/// Hash a ROM file with rc_hash, console unknown: the iterator tries the
/// consoles matching the file extension and returns the first hash that
/// can be generated (32 lowercase hex chars), or `None` on failure.
pub fn hash_rom(path: &Path) -> Option<String> {
    let path_str = path.to_str()?;
    let cpath = CString::new(path_str).ok()?;
    let mut iterator: RcHashIterator = unsafe { std::mem::zeroed() };
    let mut hash = [0u8; 33];
    let generated = unsafe {
        rc_hash_initialize_iterator(&mut iterator, cpath.as_ptr(), std::ptr::null(), 0);
        let r = rc_hash_iterate(hash.as_mut_ptr() as *mut c_char, &mut iterator);
        rc_hash_destroy_iterator(&mut iterator);
        r
    };
    if generated == 0 {
        return None;
    }
    let len = hash.iter().position(|&b| b == 0).unwrap_or(32).min(32);
    let s = String::from_utf8_lossy(&hash[..len]).into_owned();
    if s.is_empty() { None } else { Some(s) }
}

// ---------------------------------------------------------------------------
// Prefetch
// ---------------------------------------------------------------------------

fn do_request(post: String) -> Result<String, String> {
    let req = HttpRequest {
        url: DOREQUEST_URL.to_string(),
        post_data: Some(post),
        content_type: Some(FORM_CONTENT_TYPE.to_string()),
    };
    let resp = cached_transport(&req)?;
    if resp.status != 200 {
        return Err(format!("server returned HTTP {}", resp.status));
    }
    Ok(String::from_utf8_lossy(&resp.body).into_owned())
}

fn check_success(body: &str, what: &str) -> Result<(), String> {
    if body_reports_success(body.as_bytes()) {
        Ok(())
    } else {
        match json_string(body, "Error") {
            Some(err) if !err.is_empty() => Err(format!("{what}: {err}")),
            _ => Err(format!("{what}: server did not report success")),
        }
    }
}

/// Warm the offline cache for a ROM: hash it, resolve the game id, fetch
/// the achievement set data and start a session — all through the cached
/// transport, so a later fully-offline `RaClient` login + load succeeds.
///
/// Requests made (mirroring what the vendored rc_client sends on load):
/// `gameid` (by hash) -> `achievementsets` (by hash) -> `startsession`
/// (by game id). Badge images are not fetched; the frontend handles those.
///
/// Returns a short status like `"<GameTitle>: cached"`, or an error when
/// the ROM has no achievement set or a request fails with nothing cached.
pub fn prefetch_game(username: &str, token: &str, rom_path: &Path) -> Result<String, String> {
    let hash =
        hash_rom(rom_path).ok_or_else(|| format!("could not hash {}", rom_path.display()))?;
    let m = percent_encode(&hash);
    let u = percent_encode(username);
    let t = percent_encode(token);

    // 1) Resolve hash -> game id.
    let body = do_request(format!("r=gameid&m={m}"))?;
    check_success(&body, "gameid")?;
    let game_id = json_uint(&body, "GameID").unwrap_or(0);
    if game_id == 0 {
        return Err("no RetroAchievements set for this ROM".to_string());
    }

    // 2) Game data (achievement definitions + rich presence), by hash —
    //    the exact request rc_client caches against on a normal load.
    let sets = do_request(format!("r=achievementsets&u={u}&t={t}&m={m}"))?;
    check_success(&sets, "achievementsets")?;
    let title = json_string(&sets, "Title")
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("Game {game_id}"));

    // 3) Start a session (also caches current unlock state).
    let session = do_request(format!(
        "r=startsession&u={u}&t={t}&g={game_id}&h=0&m={m}&l={RCHEEVOS_VERSION_STRING}"
    ))?;
    check_success(&session, "startsession")?;

    Ok(format!("{title}: cached"))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_key_parsing() {
        // The session bootstrap set is keyed as documented.
        assert_eq!(
            cache_key("r=login2&u=testuser&p=testpwd"),
            Some("login2_testuser.json".to_string())
        );
        assert_eq!(
            cache_key("r=login2&u=testuser&t=SOMETOKEN"),
            Some("login2_testuser.json".to_string())
        );
        assert_eq!(
            cache_key("r=gameid&m=6a2305a2b6675a97ff792709be1ca857"),
            Some("gameid_6a2305a2b6675a97ff792709be1ca857.json".to_string())
        );
        assert_eq!(
            cache_key("r=patch&u=testuser&t=TOK&g=14402"),
            Some("patch_14402.json".to_string())
        );
        assert_eq!(
            cache_key("r=startsession&u=testuser&t=TOK&g=14402&h=0&m=abc&l=12.3.0"),
            Some("startsession_14402.json".to_string())
        );
        // achievementsets keys by g when present, else by m (the by-hash
        // load rc_client actually performs).
        assert_eq!(
            cache_key("r=achievementsets&u=testuser&t=TOK&m=6a2305a2b6675a97ff792709be1ca857"),
            Some("achievementsets_6a2305a2b6675a97ff792709be1ca857.json".to_string())
        );
        assert_eq!(
            cache_key("r=achievementsets&u=testuser&t=TOK&g=14402"),
            Some("achievementsets_14402.json".to_string())
        );

        // Never cached.
        assert_eq!(cache_key("r=awardachievement&u=testuser&t=TOK&a=1&h=0&v=sig"), None);
        assert_eq!(cache_key("r=ping&u=testuser&t=TOK&g=14402"), None);
        assert_eq!(cache_key("r=submitlbentry&u=testuser&t=TOK"), None);

        // Missing pieces / garbage.
        assert_eq!(cache_key(""), None);
        assert_eq!(cache_key("r=login2"), None);
        assert_eq!(cache_key("r=gameid&g=123"), None);
        assert_eq!(cache_key("no equals signs here"), None);

        // Parameter order does not matter; ids are decoded then sanitized.
        assert_eq!(
            cache_key("u=A%20B&r=login2"),
            Some("login2_A_B.json".to_string())
        );
    }

    #[test]
    fn form_helpers() {
        assert_eq!(percent_encode("abc-_.~123"), "abc-_.~123");
        assert_eq!(percent_encode("a b&c=d"), "a%20b%26c%3Dd");
        assert_eq!(form_decode("a%20b%26c"), "a b&c");
        assert_eq!(form_decode("a+b"), "a b");
        assert_eq!(form_decode("bad%2"), "bad%2");
        assert_eq!(form_decode("bad%zz"), "bad%zz");
        let roundtrip = "user name+&=%~";
        assert_eq!(form_decode(&percent_encode(roundtrip)), roundtrip);
        let params = form_params("r=gameid&m=abc&empty&x=1");
        assert_eq!(param(&params, "r"), Some("gameid"));
        assert_eq!(param(&params, "m"), Some("abc"));
        assert_eq!(param(&params, "x"), Some("1"));
        assert_eq!(param(&params, "empty"), None);
    }

    #[test]
    fn json_probes() {
        let body = r#"{"Success":true,"GameID":14402,"Title":"Some Game"}"#.as_bytes();
        assert!(body_reports_success(body));
        assert!(body_has_success_key(body));
        assert!(!body_reports_success(br#"{"Success":false,"Error":"nope"}"#));
        assert!(body_has_success_key(br#"{"Success":false}"#));
        assert!(!body_reports_success(b"<html>captive portal</html>"));
        let text = String::from_utf8_lossy(body);
        assert_eq!(json_uint(&text, "GameID"), Some(14402));
        assert_eq!(json_uint(&text, "Missing"), None);
        assert_eq!(
            json_string(r#"{"Title":"He said \"hi\"","X":1}"#, "Title"),
            Some("He said \"hi\"".to_string())
        );
        assert_eq!(
            json_string(r#"{ "Success" : true , "Error" : "Invalid user" }"#, "Error"),
            Some("Invalid user".to_string())
        );
    }

    /// Cache + queue file behavior, all under an overridden offline root
    /// (single test so the global override is not raced by another test).
    #[test]
    fn offline_files() {
        let root = std::env::temp_dir().join(format!("kui-ra-offline-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        set_offline_root(Some(&root));

        // Cache write is atomic-by-rename and read back verbatim.
        write_cache("gameid_abc.json", b"{\"Success\":true,\"GameID\":7}");
        assert_eq!(
            read_cache("gameid_abc.json"),
            Some(b"{\"Success\":true,\"GameID\":7}".to_vec())
        );
        assert_eq!(read_cache("gameid_missing.json"), None);
        assert!(root.join("cache/gameid_abc.json").is_file());

        // Queue appends one line per unlock and deduplicates.
        queue_unlock("r=awardachievement&u=A&t=T&a=1&h=0&v=sig1");
        queue_unlock("r=awardachievement&u=A&t=T&a=2&h=0&v=sig2");
        queue_unlock("r=awardachievement&u=A&t=T&a=1&h=0&v=sig1"); // dupe
        let content = std::fs::read_to_string(root.join("queue.txt")).unwrap_or_default();
        assert_eq!(content.lines().count(), 2);

        // Flushing a missing queue is a no-op that touches no network.
        let _ = std::fs::remove_file(root.join("queue.txt"));
        assert_eq!(flush_unlock_queue(), (0, 0));

        set_offline_root(None);
        let _ = std::fs::remove_dir_all(&root);
    }
}

#[cfg(test)]
mod hash_tests {
    use super::*;

    /// GB ROMs hash as md5 of the whole file; exercises the real rc_hash
    /// iterator FFI end to end.
    #[test]
    fn hash_rom_gb_whole_file_md5() {
        let path = std::env::temp_dir().join(format!("kui-ra-hash-test-{}.gb", std::process::id()));
        let _ = std::fs::write(&path, b"KUI-RA HASH SELFTEST ROM CONTENTS 0123456789");
        let hash = hash_rom(&path);
        let _ = std::fs::remove_file(&path);
        assert_eq!(hash.as_deref(), Some("5b56a8e7aabc69c382598748220f1218"));
        assert_eq!(hash_rom(Path::new("/nonexistent/kui-ra-missing.gb")), None);
    }
}
