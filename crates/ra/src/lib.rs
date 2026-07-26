//! kui-ra — RetroAchievements foundation for kUI.
//!
//! Safe wrapper around the official rcheevos library's modern `rc_client`
//! API. The C library is vendored at `crates/ra/vendor/rcheevos` and is MIT
//! licensed (Copyright (c) 2018 RetroAchievements.org; see
//! `vendor/rcheevos/LICENSE` for the full text: permission to use, copy,
//! modify, merge, publish, distribute, sublicense and/or sell, provided the
//! copyright notice and permission notice are included in all copies).
//!
//! Model: single-threaded. The frontend owns an [`RaClient`], calls
//! [`RaClient::do_frame`] once per emulated frame (or [`RaClient::idle`]
//! while paused) and drains unlock events with [`RaClient::drain_events`].
//! HTTP is performed synchronously inside the rc_client server-call hook by
//! shelling out to `curl -s`, so the `begin_*` async entry points complete
//! by the time they return.

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

pub mod offline;
pub use offline::{
    DEFAULT_OFFLINE_ROOT, flush_unlock_queue, hash_rom, prefetch_game, set_offline_root,
};

// ---------------------------------------------------------------------------
// FFI: opaque types and the C structs we actually touch (layouts mirror the
// vendored headers in vendor/rcheevos/include).
// ---------------------------------------------------------------------------

#[repr(C)]
struct RcClient {
    _private: [u8; 0],
}

/// First three fields of rc_api_request_t (rc_api_request.h). The struct
/// also carries an internal rc_buffer_t we never touch; we only ever hold a
/// pointer to a C-owned instance and read these leading fields, so the
/// trailing storage may be omitted from the declaration.
#[repr(C)]
struct RcApiRequest {
    url: *const c_char,
    post_data: *const c_char,
    content_type: *const c_char,
    // rc_buffer_t buffer; -- never accessed from Rust
}

/// rc_api_server_response_t (rc_api_request.h) — constructed by us, so this
/// must be the complete layout (it is: three fields).
#[repr(C)]
struct RcApiServerResponse {
    body: *const c_char,
    body_length: usize,
    http_status_code: c_int,
}

/// rc_client_user_t (rc_client.h).
#[repr(C)]
struct RcClientUser {
    display_name: *const c_char,
    username: *const c_char,
    token: *const c_char,
    score: u32,
    score_softcore: u32,
    num_unread_messages: u32,
    avatar_url: *const c_char,
}

/// rc_client_achievement_t (rc_client.h).
#[repr(C)]
struct RcClientAchievement {
    title: *const c_char,
    description: *const c_char,
    badge_name: [c_char; 8],
    measured_progress: [c_char; 24],
    measured_percent: f32,
    id: u32,
    points: u32,
    unlock_time: i64, // time_t on 64-bit linux (both our targets)
    state: u8,
    category: u8,
    bucket: u8,
    unlocked: u8,
    rarity: f32,
    rarity_hardcore: f32,
    achievement_type: u8,
    badge_url: *const c_char,
    badge_locked_url: *const c_char,
}

/// rc_client_achievement_bucket_t (rc_client.h).
#[repr(C)]
struct RcClientAchievementBucket {
    achievements: *const *const RcClientAchievement,
    num_achievements: u32,
    label: *const c_char,
    subset_id: u32,
    bucket_type: u8,
}

/// rc_client_achievement_list_t (rc_client.h).
#[repr(C)]
struct RcClientAchievementList {
    buckets: *const RcClientAchievementBucket,
    num_buckets: u32,
}

/// rc_client_server_error_t (rc_client.h).
#[repr(C)]
struct RcClientServerError {
    error_message: *const c_char,
    api: *const c_char,
    result: c_int,
    related_id: u32,
}

/// rc_client_event_t (rc_client.h).
#[repr(C)]
struct RcClientEvent {
    event_type: u32,
    achievement: *mut RcClientAchievement,
    leaderboard: *mut c_void,
    leaderboard_tracker: *mut c_void,
    leaderboard_scoreboard: *mut c_void,
    server_error: *mut RcClientServerError,
    subset: *mut c_void,
}

type RcClientReadMemoryFunc =
    unsafe extern "C" fn(address: u32, buffer: *mut u8, num_bytes: u32, client: *mut RcClient) -> u32;
type RcClientServerCallback =
    unsafe extern "C" fn(server_response: *const RcApiServerResponse, callback_data: *mut c_void);
type RcClientServerCall = unsafe extern "C" fn(
    request: *const RcApiRequest,
    callback: Option<RcClientServerCallback>,
    callback_data: *mut c_void,
    client: *mut RcClient,
);
type RcClientCallback = unsafe extern "C" fn(
    result: c_int,
    error_message: *const c_char,
    client: *mut RcClient,
    userdata: *mut c_void,
);
type RcClientEventHandler =
    unsafe extern "C" fn(event: *const RcClientEvent, client: *mut RcClient);

// Constants from the vendored headers.
const RC_OK: c_int = 0;
/// rc_api_request.h: network-level failure the client may retry.
const RC_API_SERVER_RESPONSE_RETRYABLE_CLIENT_ERROR: c_int = -2;
const RC_CLIENT_ACHIEVEMENT_CATEGORY_CORE: c_int = 1 << 0;
const RC_CLIENT_ACHIEVEMENT_LIST_GROUPING_LOCK_STATE: c_int = 0;
const RC_CONSOLE_UNKNOWN: u32 = 0;

const RC_CLIENT_EVENT_ACHIEVEMENT_TRIGGERED: u32 = 1;
const RC_CLIENT_EVENT_RESET: u32 = 14;
const RC_CLIENT_EVENT_GAME_COMPLETED: u32 = 15;
const RC_CLIENT_EVENT_SERVER_ERROR: u32 = 16;
const RC_CLIENT_EVENT_DISCONNECTED: u32 = 17;
const RC_CLIENT_EVENT_RECONNECTED: u32 = 18;

unsafe extern "C" {
    fn rc_client_create(
        read_memory_function: Option<RcClientReadMemoryFunc>,
        server_call_function: Option<RcClientServerCall>,
    ) -> *mut RcClient;
    fn rc_client_destroy(client: *mut RcClient);
    fn rc_client_set_userdata(client: *mut RcClient, userdata: *mut c_void);
    fn rc_client_get_userdata(client: *const RcClient) -> *mut c_void;
    fn rc_client_set_event_handler(client: *mut RcClient, handler: Option<RcClientEventHandler>);
    fn rc_client_set_hardcore_enabled(client: *mut RcClient, enabled: c_int);
    fn rc_client_get_hardcore_enabled(client: *const RcClient) -> c_int;
    fn rc_client_begin_login_with_password(
        client: *mut RcClient,
        username: *const c_char,
        password: *const c_char,
        callback: Option<RcClientCallback>,
        callback_userdata: *mut c_void,
    ) -> *mut c_void;
    fn rc_client_begin_login_with_token(
        client: *mut RcClient,
        username: *const c_char,
        token: *const c_char,
        callback: Option<RcClientCallback>,
        callback_userdata: *mut c_void,
    ) -> *mut c_void;
    fn rc_client_logout(client: *mut RcClient);
    fn rc_client_get_user_info(client: *const RcClient) -> *const RcClientUser;
    fn rc_client_begin_identify_and_load_game(
        client: *mut RcClient,
        console_id: u32,
        file_path: *const c_char,
        data: *const u8,
        data_size: usize,
        callback: Option<RcClientCallback>,
        callback_userdata: *mut c_void,
    ) -> *mut c_void;
    fn rc_client_unload_game(client: *mut RcClient);
    fn rc_client_is_game_loaded(client: *const RcClient) -> c_int;
    fn rc_client_do_frame(client: *mut RcClient);
    fn rc_client_idle(client: *mut RcClient);
    fn rc_client_create_achievement_list(
        client: *mut RcClient,
        category: c_int,
        grouping: c_int,
    ) -> *mut RcClientAchievementList;
    fn rc_client_destroy_achievement_list(list: *mut RcClientAchievementList);
    fn rc_client_reset(client: *mut RcClient);
    fn rc_client_progress_size(client: *mut RcClient) -> usize;
    fn rc_client_serialize_progress_sized(
        client: *mut RcClient,
        buffer: *mut u8,
        buffer_size: usize,
    ) -> c_int;
    fn rc_client_deserialize_progress_sized(
        client: *mut RcClient,
        serialized: *const u8,
        serialized_size: usize,
    ) -> c_int;
    fn rc_client_get_user_agent_clause(
        client: *mut RcClient,
        buffer: *mut c_char,
        buffer_size: usize,
    ) -> usize;
}

// ---------------------------------------------------------------------------
// Public data types
// ---------------------------------------------------------------------------

/// Snapshot of one achievement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RaAchievement {
    pub id: u32,
    pub title: String,
    pub description: String,
    pub points: u32,
    pub unlocked: bool,
    pub badge_url: Option<String>,
}

/// Events captured from rc_client, drained by the caller each frame.
#[derive(Debug, Clone)]
pub enum RaEvent {
    /// The player earned an achievement.
    AchievementUnlocked(RaAchievement),
    /// All core achievements for the game have been earned.
    GameCompleted,
    /// rc_client asks the emulated system to be reset (e.g. hardcore toggle).
    Reset,
    /// An API call failed and will not be retried.
    ServerError { api: String, message: String },
    /// An unlock could not be delivered and is pending retry.
    Disconnected,
    /// All pending unlocks were delivered after a disconnect.
    Reconnected,
    /// Any other rc_client event, identified by its RC_CLIENT_EVENT_* value.
    Other(u32),
}

/// Signature of the guest-memory reader the frontend registers:
/// `(address, destination buffer) -> bytes actually read`.
pub type ReadMemoryFn = dyn FnMut(u32, &mut [u8]) -> u32 + 'static;
type HttpFn = dyn FnMut(&HttpRequest) -> Result<HttpResponse, String> + 'static;

// ---------------------------------------------------------------------------
// Internal shared state, recovered inside the C callbacks via the rc_client
// userdata pointer.
// ---------------------------------------------------------------------------

struct HttpRequest {
    url: String,
    post_data: Option<String>,
    content_type: Option<String>,
}

struct HttpResponse {
    status: c_int,
    body: Vec<u8>,
}

struct ClientState {
    read_memory: Box<ReadMemoryFn>,
    http: Box<HttpFn>,
    events: Vec<RaEvent>,
    /// Result slot for the current synchronous begin_* call.
    pending: Option<Result<(), String>>,
}

// ---------------------------------------------------------------------------
// extern "C" shims. Each recovers the ClientState through the rc_client
// userdata pointer and is wrapped in catch_unwind so a Rust panic can never
// unwind across the C frames.
// ---------------------------------------------------------------------------

unsafe fn state_from_client<'a>(client: *const RcClient) -> Option<&'a mut ClientState> {
    if client.is_null() {
        return None;
    }
    let ptr = unsafe { rc_client_get_userdata(client) } as *mut ClientState;
    if ptr.is_null() { None } else { Some(unsafe { &mut *ptr }) }
}

unsafe extern "C" fn read_memory_shim(
    address: u32,
    buffer: *mut u8,
    num_bytes: u32,
    client: *mut RcClient,
) -> u32 {
    catch_unwind(AssertUnwindSafe(|| {
        let Some(state) = (unsafe { state_from_client(client) }) else {
            return 0;
        };
        if buffer.is_null() || num_bytes == 0 {
            return 0;
        }
        let slice = unsafe { std::slice::from_raw_parts_mut(buffer, num_bytes as usize) };
        (state.read_memory)(address, slice)
    }))
    .unwrap_or(0)
}

unsafe extern "C" fn server_call_shim(
    request: *const RcApiRequest,
    callback: Option<RcClientServerCallback>,
    callback_data: *mut c_void,
    client: *mut RcClient,
) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        let Some(callback) = callback else { return };
        let respond = |status: c_int, body: &[u8]| {
            // Keep the body NUL-terminated: rc_api parsers treat it as a
            // C string as well as a (ptr, len) pair.
            let mut owned = body.to_vec();
            owned.push(0);
            let response = RcApiServerResponse {
                body: owned.as_ptr() as *const c_char,
                body_length: owned.len() - 1,
                http_status_code: status,
            };
            unsafe { callback(&response, callback_data) };
            // `owned` outlives the callback invocation, then drops here.
        };

        let (Some(state), false) = (unsafe { state_from_client(client) }, request.is_null()) else {
            respond(RC_API_SERVER_RESPONSE_RETRYABLE_CLIENT_ERROR, b"");
            return;
        };
        let req = unsafe {
            HttpRequest {
                url: cstr_to_string((*request).url),
                post_data: cstr_to_opt_string((*request).post_data),
                content_type: cstr_to_opt_string((*request).content_type),
            }
        };
        match (state.http)(&req) {
            Ok(resp) => respond(resp.status, &resp.body),
            Err(_) => respond(RC_API_SERVER_RESPONSE_RETRYABLE_CLIENT_ERROR, b""),
        }
    }));
}

unsafe extern "C" fn event_shim(event: *const RcClientEvent, client: *mut RcClient) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        let Some(state) = (unsafe { state_from_client(client) }) else {
            return;
        };
        if event.is_null() {
            return;
        }
        let event = unsafe { &*event };
        let ra_event = match event.event_type {
            RC_CLIENT_EVENT_ACHIEVEMENT_TRIGGERED => match unsafe { achievement_from_ptr(event.achievement) } {
                Some(a) => RaEvent::AchievementUnlocked(a),
                None => RaEvent::Other(event.event_type),
            },
            RC_CLIENT_EVENT_GAME_COMPLETED => RaEvent::GameCompleted,
            RC_CLIENT_EVENT_RESET => RaEvent::Reset,
            RC_CLIENT_EVENT_SERVER_ERROR => {
                let (api, message) = if event.server_error.is_null() {
                    (String::new(), String::new())
                } else {
                    unsafe {
                        (
                            cstr_to_string((*event.server_error).api),
                            cstr_to_string((*event.server_error).error_message),
                        )
                    }
                };
                RaEvent::ServerError { api, message }
            }
            RC_CLIENT_EVENT_DISCONNECTED => RaEvent::Disconnected,
            RC_CLIENT_EVENT_RECONNECTED => RaEvent::Reconnected,
            other => RaEvent::Other(other),
        };
        state.events.push(ra_event);
    }));
}

/// Completion callback for begin_login / begin_identify_and_load_game.
/// `userdata` is the ClientState pointer.
unsafe extern "C" fn completion_shim(
    result: c_int,
    error_message: *const c_char,
    _client: *mut RcClient,
    userdata: *mut c_void,
) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        let state = userdata as *mut ClientState;
        if state.is_null() {
            return;
        }
        let state = unsafe { &mut *state };
        state.pending = Some(if result == RC_OK {
            Ok(())
        } else {
            let msg = unsafe { cstr_to_string(error_message) };
            Err(if msg.is_empty() {
                format!("rcheevos error {result}")
            } else {
                format!("{msg} (rcheevos error {result})")
            })
        });
    }));
}

unsafe fn cstr_to_string(ptr: *const c_char) -> String {
    if ptr.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(ptr) }.to_string_lossy().into_owned()
    }
}

unsafe fn cstr_to_opt_string(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        None
    } else {
        Some(unsafe { cstr_to_string(ptr) })
    }
}

unsafe fn achievement_from_ptr(ptr: *const RcClientAchievement) -> Option<RaAchievement> {
    if ptr.is_null() {
        return None;
    }
    let a = unsafe { &*ptr };
    Some(RaAchievement {
        id: a.id,
        title: unsafe { cstr_to_string(a.title) },
        description: unsafe { cstr_to_string(a.description) },
        points: a.points,
        // RC_CLIENT_ACHIEVEMENT_UNLOCKED_NONE == 0; softcore/hardcore bits set otherwise.
        unlocked: a.unlocked != 0,
        badge_url: unsafe {
            if a.badge_url.is_null() {
                None
            } else {
                Some(cstr_to_string(a.badge_url)).filter(|s| !s.is_empty())
            }
        },
    })
}

// ---------------------------------------------------------------------------
// Default HTTP transport: shell out to `curl -s`.
// ---------------------------------------------------------------------------

static CURL_TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

fn curl_http(req: &HttpRequest) -> Result<HttpResponse, String> {
    // Body goes to a temp file (binary-safe); the HTTP status code is the
    // only thing curl writes to stdout.
    let tmp = std::env::temp_dir().join(format!(
        "kui-ra-{}-{}.http",
        std::process::id(),
        CURL_TMP_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let curl_bin = if std::path::Path::new("/usr/bin/curl").exists() {
        "/usr/bin/curl"
    } else {
        "curl"
    };
    let mut cmd = Command::new(curl_bin);
    cmd.arg("-s")
        .arg("--max-time")
        .arg("15")
        .arg("-A")
        .arg(user_agent());
    // the device ships no CA store; kUI carries a bundle on the card
    let ca = "/mnt/SDCARD/.system/res/cacert.pem";
    if std::path::Path::new(ca).is_file() {
        cmd.arg("--cacert").arg(ca);
    } else if curl_bin == "/usr/bin/curl" {
        cmd.arg("-k");
    }
    cmd
        .arg("-o")
        .arg(&tmp)
        .arg("-w")
        .arg("%{http_code}");
    if let Some(post) = &req.post_data {
        cmd.arg("-X").arg("POST").arg("--data-raw").arg(post);
        let content_type = req
            .content_type
            .as_deref()
            .unwrap_or("application/x-www-form-urlencoded");
        cmd.arg("-H").arg(format!("Content-Type: {content_type}"));
    }
    cmd.arg(&req.url);

    let output = cmd
        .output()
        .map_err(|e| format!("failed to run curl: {e}"))?;
    let body = std::fs::read(&tmp).unwrap_or_default();
    let _ = std::fs::remove_file(&tmp);
    if !output.status.success() {
        return Err(format!(
            "curl exited with {} for {}",
            output.status, req.url
        ));
    }
    let status: c_int = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .map_err(|_| format!("curl returned an unparsable status code for {}", req.url))?;
    Ok(HttpResponse { status, body })
}

// ---------------------------------------------------------------------------
// User agent — RA compliance requires a unique, stable
// `EmulatorName/vX.Y.Z (OS) core/vN` identity, not a library default.
// The frontend composes and installs it before the first request.
// ---------------------------------------------------------------------------

static USER_AGENT: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// Install the client identity used on every RA request. First caller
/// wins; call before login. See docs.retroachievements.org hardcore
/// compliance ("unique user agent").
pub fn set_user_agent(ua: &str) {
    let _ = USER_AGENT.set(ua.to_string());
}

fn user_agent() -> &'static str {
    USER_AGENT.get().map(String::as_str).unwrap_or("kUI/unknown (Linux)")
}

// ---------------------------------------------------------------------------
// RaClient — the safe wrapper
// ---------------------------------------------------------------------------

/// Safe, single-threaded wrapper around an `rc_client_t`.
///
/// Not `Send`/`Sync` (raw pointers): keep it on the thread that runs the
/// emulation loop and call [`RaClient::do_frame`] from there.
pub struct RaClient {
    client: *mut RcClient,
    state: *mut ClientState,
}

impl RaClient {
    /// Create a client. `read_memory` maps a RetroAchievements memory
    /// address into guest memory: fill `buffer` and return the number of
    /// bytes provided (0 if the address is invalid).
    ///
    /// Hardcore starts disabled; the frontend opts in per session via
    /// [`RaClient::set_hardcore_enabled`] before loading the game. Until
    /// kUI holds RA client approval the server demotes hardcore unlocks
    /// to softcore, but the gating behavior ships ready for audit.
    pub fn new(
        read_memory: impl FnMut(u32, &mut [u8]) -> u32 + 'static,
    ) -> Result<RaClient, String> {
        let state = Box::into_raw(Box::new(ClientState {
            read_memory: Box::new(read_memory),
            // curl wrapped with the offline response cache + unlock queue.
            http: Box::new(offline::cached_transport),
            events: Vec::new(),
            pending: None,
        }));
        let client = unsafe { rc_client_create(Some(read_memory_shim), Some(server_call_shim)) };
        if client.is_null() {
            // Reclaim the state box before bailing.
            drop(unsafe { Box::from_raw(state) });
            return Err("rc_client_create returned NULL".to_string());
        }
        unsafe {
            rc_client_set_userdata(client, state as *mut c_void);
            rc_client_set_event_handler(client, Some(event_shim));
            rc_client_set_hardcore_enabled(client, 0);
        }
        Ok(RaClient { client, state })
    }

    fn state_mut(&mut self) -> &mut ClientState {
        unsafe { &mut *self.state }
    }

    /// Take the completion result the shim stored during a begin_* call.
    fn take_pending(&mut self, what: &str) -> Result<(), String> {
        match self.state_mut().pending.take() {
            Some(result) => result,
            None => Err(format!("{what} did not complete (no callback fired)")),
        }
    }

    /// Log in with username + password. Synchronous (HTTP runs inline).
    /// On success the session token is available via [`RaClient::user_token`].
    pub fn login_with_password(&mut self, username: &str, password: &str) -> Result<(), String> {
        let user = CString::new(username).map_err(|_| "username contains NUL".to_string())?;
        let pass = CString::new(password).map_err(|_| "password contains NUL".to_string())?;
        self.state_mut().pending = None;
        unsafe {
            rc_client_begin_login_with_password(
                self.client,
                user.as_ptr(),
                pass.as_ptr(),
                Some(completion_shim),
                self.state as *mut c_void,
            );
        }
        self.take_pending("login")
    }

    /// Log in with username + a previously stored session token.
    pub fn login_with_token(&mut self, username: &str, token: &str) -> Result<(), String> {
        let user = CString::new(username).map_err(|_| "username contains NUL".to_string())?;
        let tok = CString::new(token).map_err(|_| "token contains NUL".to_string())?;
        self.state_mut().pending = None;
        unsafe {
            rc_client_begin_login_with_token(
                self.client,
                user.as_ptr(),
                tok.as_ptr(),
                Some(completion_shim),
                self.state as *mut c_void,
            );
        }
        self.take_pending("login")
    }

    /// Forget the current login.
    pub fn logout(&mut self) {
        unsafe { rc_client_logout(self.client) };
    }

    /// The session token for the logged-in user (persist it and reuse with
    /// [`RaClient::login_with_token`]). `None` when not logged in.
    pub fn user_token(&self) -> Option<String> {
        unsafe {
            let user = rc_client_get_user_info(self.client);
            if user.is_null() {
                return None;
            }
            let token = cstr_to_string((*user).token);
            if token.is_empty() { None } else { Some(token) }
        }
    }

    /// The display name of the logged-in user, if any.
    pub fn user_display_name(&self) -> Option<String> {
        unsafe {
            let user = rc_client_get_user_info(self.client);
            if user.is_null() {
                return None;
            }
            let name = cstr_to_string((*user).display_name);
            if name.is_empty() { None } else { Some(name) }
        }
    }

    /// Identify a ROM by path (rc_hash reads and hashes the file) and load
    /// its achievement set. Synchronous. Requires a prior login.
    pub fn load_game(&mut self, rom_path: &Path) -> Result<(), String> {
        let path_str = rom_path
            .to_str()
            .ok_or_else(|| "rom path is not valid UTF-8".to_string())?;
        let path = CString::new(path_str).map_err(|_| "rom path contains NUL".to_string())?;
        self.state_mut().pending = None;
        unsafe {
            rc_client_begin_identify_and_load_game(
                self.client,
                RC_CONSOLE_UNKNOWN,
                path.as_ptr(),
                std::ptr::null(),
                0,
                Some(completion_shim),
                self.state as *mut c_void,
            );
        }
        self.take_pending("load_game")
    }

    /// Unload the current game (flushes nothing; unlocks already sent).
    pub fn unload_game(&mut self) {
        unsafe { rc_client_unload_game(self.client) };
    }

    /// True once a game's achievement set is active.
    pub fn is_game_loaded(&self) -> bool {
        unsafe { rc_client_is_game_loaded(self.client) != 0 }
    }

    /// Process achievements for the current emulated frame. Call once per
    /// frame from the emulation loop.
    pub fn do_frame(&mut self) {
        unsafe { rc_client_do_frame(self.client) };
    }

    /// Keep the periodic queue moving while emulation is paused.
    pub fn idle(&mut self) {
        unsafe { rc_client_idle(self.client) };
    }

    /// Enable/disable hardcore mode. Disabled by default in kUI.
    pub fn set_hardcore_enabled(&mut self, enabled: bool) {
        unsafe { rc_client_set_hardcore_enabled(self.client, enabled as c_int) };
    }

    /// Ask the emulated system to reset. Required after a casual->hardcore
    /// switch (rc_client raises `RaEvent::Reset` for the same purpose).
    pub fn reset(&mut self) {
        unsafe { rc_client_reset(self.client) };
    }

    /// Snapshot the achievement runtime for embedding beside a save state.
    pub fn serialize_progress(&mut self) -> Option<Vec<u8>> {
        let size = unsafe { rc_client_progress_size(self.client) };
        if size == 0 {
            return None;
        }
        let mut buf = vec![0u8; size];
        let rc =
            unsafe { rc_client_serialize_progress_sized(self.client, buf.as_mut_ptr(), size) };
        (rc == RC_OK).then_some(buf)
    }

    /// Restore the achievement runtime captured with a save state. `None`
    /// resets the runtime to its initial state (the right call when a
    /// state has no progress sidecar).
    pub fn deserialize_progress(&mut self, data: Option<&[u8]>) -> bool {
        let rc = match data {
            Some(d) => unsafe {
                rc_client_deserialize_progress_sized(self.client, d.as_ptr(), d.len())
            },
            None => unsafe {
                rc_client_deserialize_progress_sized(self.client, std::ptr::null(), 0)
            },
        };
        rc == RC_OK
    }

    /// rcheevos' own User-Agent suffix, e.g. "rc_client/12.3.0".
    pub fn user_agent_clause(&self) -> String {
        let mut buf = [0u8; 64];
        let n = unsafe {
            rc_client_get_user_agent_clause(self.client, buf.as_mut_ptr() as *mut c_char, buf.len())
        };
        String::from_utf8_lossy(&buf[..n.min(buf.len() - 1)]).into_owned()
    }

    /// Whether hardcore mode is currently enabled.
    pub fn hardcore_enabled(&self) -> bool {
        unsafe { rc_client_get_hardcore_enabled(self.client) != 0 }
    }

    /// Snapshot of the core achievement list for the loaded game.
    pub fn achievement_list(&mut self) -> Result<Vec<RaAchievement>, String> {
        if !self.is_game_loaded() {
            return Err("no game loaded".to_string());
        }
        unsafe {
            let list = rc_client_create_achievement_list(
                self.client,
                RC_CLIENT_ACHIEVEMENT_CATEGORY_CORE,
                RC_CLIENT_ACHIEVEMENT_LIST_GROUPING_LOCK_STATE,
            );
            if list.is_null() {
                return Err("achievement list unavailable".to_string());
            }
            let mut out = Vec::new();
            let buckets = if (*list).buckets.is_null() || (*list).num_buckets == 0 {
                &[]
            } else {
                std::slice::from_raw_parts((*list).buckets, (*list).num_buckets as usize)
            };
            for bucket in buckets {
                if bucket.achievements.is_null() || bucket.num_achievements == 0 {
                    continue;
                }
                let achievements = std::slice::from_raw_parts(
                    bucket.achievements,
                    bucket.num_achievements as usize,
                );
                for &ach in achievements {
                    if let Some(a) = achievement_from_ptr(ach) {
                        out.push(a);
                    }
                }
            }
            rc_client_destroy_achievement_list(list);
            Ok(out)
        }
    }

    /// Drain all events captured since the last call (unlocks, completion,
    /// server errors, ...). Call after [`RaClient::do_frame`].
    pub fn drain_events(&mut self) -> Vec<RaEvent> {
        std::mem::take(&mut self.state_mut().events)
    }
}

impl Drop for RaClient {
    fn drop(&mut self) {
        unsafe {
            // Destroy the C client first: this aborts in-flight async
            // handles and stops all callbacks, after which the state box
            // can be reclaimed safely.
            rc_client_destroy(self.client);
            drop(Box::from_raw(self.state));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_and_destroy() {
        let mut client = RaClient::new(|_addr, _buf| 0).expect("client creation");
        assert!(!client.is_game_loaded());
        assert!(client.user_token().is_none());
        assert!(client.drain_events().is_empty());
        assert!(!client.hardcore_enabled());
        client.do_frame();
        client.idle();
        assert!(client.achievement_list().is_err());
    }
}

// ---------------------------------------------------------------------------
// Console-region fallback mapping: when a core registers no memory map,
// translate RA addresses via rcheevos' per-console region tables into
// (retro memory id, offset) — the same scheme rc_libretro uses.
// ---------------------------------------------------------------------------

#[repr(C)]
struct RcMemoryRegion {
    start_address: u32,
    end_address: u32,
    real_address: u32,
    kind: u8,
    description: *const c_char,
}
#[repr(C)]
struct RcMemoryRegions {
    region: *const RcMemoryRegion,
    num_regions: u32,
}
#[repr(C)]
struct RcClientGame {
    id: u32,
    console_id: u32,
    title: *const c_char,
    hash: *const c_char,
    badge_name: *const c_char,
}
unsafe extern "C" {
    fn rc_console_memory_regions(console_id: u32) -> *const RcMemoryRegions;
    fn rc_client_get_game_info(client: *const RcClient) -> *const RcClientGame;
}

/// (ra_start, ra_end, retro_mem_id, offset_within_that_memory)
static FALLBACK_MAP: std::sync::Mutex<Vec<(u32, u32, u32, usize)>> =
    std::sync::Mutex::new(Vec::new());

const RETRO_MEMORY_SAVE_RAM: u32 = 0;
const RETRO_MEMORY_SYSTEM_RAM: u32 = 2;

/// Build the fallback map for a console. RAM-type regions consume the
/// core's SYSTEM_RAM sequentially; save regions consume SAVE_RAM.
pub fn build_fallback_map(console_id: u32) {
    let mut out = Vec::new();
    let regions = unsafe { rc_console_memory_regions(console_id) };
    if !regions.is_null() {
        let (mut ram_off, mut save_off) = (0usize, 0usize);
        let n = unsafe { (*regions).num_regions } as usize;
        for i in 0..n {
            let r = unsafe { &*(*regions).region.add(i) };
            let len = (r.end_address - r.start_address + 1) as usize;
            match r.kind {
                0 | 5 => {
                    // system ram / virtual ram
                    out.push((r.start_address, r.end_address, RETRO_MEMORY_SYSTEM_RAM, ram_off));
                    ram_off += len;
                }
                1 => {
                    out.push((r.start_address, r.end_address, RETRO_MEMORY_SAVE_RAM, save_off));
                    save_off += len;
                }
                _ => {}
            }
        }
    }
    *FALLBACK_MAP.lock().unwrap() = out;
}

/// RA address -> (retro memory id, offset), if the console map covers it.
pub fn translate_fallback(addr: u32) -> Option<(u32, usize)> {
    for (start, end, id, off) in FALLBACK_MAP.lock().unwrap().iter() {
        if addr >= *start && addr <= *end {
            return Some((*id, off + (addr - start) as usize));
        }
    }
    None
}

impl RaClient {
    /// Console id of the loaded game (0 if none).
    pub fn game_console_id(&self) -> u32 {
        let info = unsafe { rc_client_get_game_info(self.client) };
        if info.is_null() { 0 } else { unsafe { (*info).console_id } }
    }
}
