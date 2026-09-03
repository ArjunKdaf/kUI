//! libretro host: dlopen a core, wire the callback surface, run frames.
//! Hand-written FFI against the public libretro API (a stable C ABI).
//! Single core per process by design — the callback state is process-global.

use std::ffi::{CStr, CString, c_char, c_uint, c_void};
use std::path::Path;

// ---- API types ----

#[repr(C)]
pub struct GameGeometry {
    pub base_width: c_uint,
    pub base_height: c_uint,
    pub max_width: c_uint,
    pub max_height: c_uint,
    pub aspect_ratio: f32,
}

#[repr(C)]
pub struct SystemTiming {
    pub fps: f64,
    pub sample_rate: f64,
}

#[repr(C)]
pub struct SystemAvInfo {
    pub geometry: GameGeometry,
    pub timing: SystemTiming,
}

#[repr(C)]
struct SystemInfo {
    library_name: *const c_char,
    library_version: *const c_char,
    valid_extensions: *const c_char,
    need_fullpath: bool,
    block_extract: bool,
}

#[repr(C)]
struct GameInfo {
    path: *const c_char,
    data: *const c_void,
    size: usize,
    meta: *const c_char,
}

#[repr(C)]
struct Variable {
    key: *const c_char,
    value: *const c_char,
}

// environment commands we honor
const ENV_GET_CAN_DUPE: c_uint = 3;
const ENV_GET_SYSTEM_DIRECTORY: c_uint = 9;
const ENV_SET_PIXEL_FORMAT: c_uint = 10;
const ENV_SET_DISK_CONTROL_INTERFACE: c_uint = 13;
const ENV_GET_VARIABLE: c_uint = 15;
const ENV_SET_VARIABLES: c_uint = 16;
const ENV_GET_VARIABLE_UPDATE: c_uint = 17;
const ENV_SET_SUPPORT_NO_GAME: c_uint = 18;
const ENV_SET_FRAME_TIME_CALLBACK: c_uint = 21;
const ENV_SET_AUDIO_CALLBACK: c_uint = 22;
const ENV_GET_LOG_INTERFACE: c_uint = 27;
const ENV_GET_SAVE_DIRECTORY: c_uint = 31;
const ENV_SET_SYSTEM_AV_INFO: c_uint = 32;
const ENV_SET_CONTROLLER_INFO: c_uint = 35;
const ENV_SET_MEMORY_MAPS: c_uint = 36;
const ENV_SET_GEOMETRY: c_uint = 37;
const ENV_SET_SUPPORT_ACHIEVEMENTS: c_uint = 42;

pub const JOYPAD_B: u32 = 0;
pub const JOYPAD_Y: u32 = 1;
pub const JOYPAD_SELECT: u32 = 2;
pub const JOYPAD_START: u32 = 3;
pub const JOYPAD_UP: u32 = 4;
pub const JOYPAD_DOWN: u32 = 5;
pub const JOYPAD_LEFT: u32 = 6;
pub const JOYPAD_RIGHT: u32 = 7;
pub const JOYPAD_A: u32 = 8;
pub const JOYPAD_X: u32 = 9;
pub const JOYPAD_L: u32 = 10;
pub const JOYPAD_R: u32 = 11;
pub const JOYPAD_L2: u32 = 12;
pub const JOYPAD_R2: u32 = 13;

#[derive(Clone, Copy, PartialEq)]
pub enum PixelFormat {
    Xrgb1555,
    Xrgb8888,
    Rgb565,
}

// ---- process-global host state (single-core, single-thread model) ----

/// One core option definition captured from SET_VARIABLES.
pub struct VarDef {
    pub key: String,
    pub desc: String,
    pub choices: Vec<String>,
}

pub struct HostState {
    /// core option values by key (from the config store); GET_VARIABLE
    /// serves from here, missing keys fall back to the first choice.
    pub options: std::collections::HashMap<String, CString>,
    /// First-listed choice per option — the answer when nothing is set,
    /// matching classic frontend behavior (cores' internal fallbacks differ).
    pub option_defaults: std::collections::HashMap<String, CString>,
    pub disk_ctrl: Option<DiskControl>,
    pub mem_maps: Vec<MemDesc>,
    pub mem_fns: Option<(
        unsafe extern "C" fn(c_uint) -> *mut c_void,
        unsafe extern "C" fn(c_uint) -> usize,
    )>,
    pub var_defs: Vec<VarDef>,
    pub vars_dirty: bool,
    /// dpad drives the left analog stick (stick-needing games on a
    /// stickless device); the digital dpad reads empty while set
    pub dpad_as_stick: bool,
    pub video: Vec<u8>, // RGBA8
    pub video_w: u32,
    pub video_h: u32,
    pub video_dirty: bool,
    pub audio: Vec<i16>, // interleaved stereo
    pub pad: u32,        // bitmask by JOYPAD_*
    pub pixel_format: PixelFormat,
    pub av_dirty: bool,
    pub system_dir: CString,
    pub save_dir: CString,
    /// Frame-time callback (SET_FRAME_TIME_CALLBACK). Some cores have NO
    /// internal clock and advance time only when we call this — see
    /// [`Core::run_frame`].
    pub frame_time_cb: Option<unsafe extern "C" fn(i64)>,
    /// Microseconds per frame at the core's target fps, from the same struct.
    pub frame_time_ref: i64,
    /// When we last ticked the core's clock.
    pub frame_time_last: Option<std::time::Instant>,
    /// Audio callback (SET_AUDIO_CALLBACK). Cores using this interface emit
    /// NOTHING from retro_run — audio exists only if we call this each frame.
    pub audio_cb: Option<unsafe extern "C" fn()>,
    /// Its companion enable/disable hook; the core stays silent until told true.
    pub audio_set_state: Option<unsafe extern "C" fn(bool)>,
}

/// `struct retro_audio_callback` from libretro.h.
#[repr(C)]
struct AudioCallback {
    callback: Option<unsafe extern "C" fn()>,
    set_state: Option<unsafe extern "C" fn(bool)>,
}

/// `struct retro_frame_time_callback` from libretro.h.
#[repr(C)]
struct FrameTimeCallback {
    callback: Option<unsafe extern "C" fn(i64)>,
    reference: i64,
}

static mut HOST: Option<HostState> = None;

#[allow(static_mut_refs)]
fn host() -> &'static mut HostState {
    unsafe { HOST.as_mut().expect("host not initialized") }
}

// ---- callbacks handed to the core ----

unsafe extern "C" fn cb_environment(cmd: c_uint, data: *mut c_void) -> bool {
    if std::env::var_os("KUI_TRACE").is_some() {
        eprintln!("env cmd {}", cmd);
    }
    match cmd {
        ENV_GET_CAN_DUPE => {
            unsafe { *(data as *mut bool) = true };
            true
        }
        ENV_SET_PIXEL_FORMAT => {
            let f = unsafe { *(data as *const c_uint) };
            host().pixel_format = match f {
                1 => PixelFormat::Xrgb8888,
                2 => PixelFormat::Rgb565,
                _ => PixelFormat::Xrgb1555,
            };
            true
        }
        ENV_GET_SYSTEM_DIRECTORY => {
            unsafe { *(data as *mut *const c_char) = host().system_dir.as_ptr() };
            true
        }
        ENV_GET_SAVE_DIRECTORY => {
            unsafe { *(data as *mut *const c_char) = host().save_dir.as_ptr() };
            true
        }
        ENV_SET_VARIABLES => {
            // capture definitions: value = "Description; opt1|opt2|..."
            let mut v = data as *const Variable;
            let defs = &mut host().var_defs;
            defs.clear();
            unsafe {
                while !(*v).key.is_null() {
                    let key = CStr::from_ptr((*v).key).to_string_lossy().into_owned();
                    let raw = if (*v).value.is_null() {
                        String::new()
                    } else {
                        CStr::from_ptr((*v).value).to_string_lossy().into_owned()
                    };
                    let (desc, opts) = raw.split_once(';').unwrap_or(("", ""));
                    let choices: Vec<String> =
                        opts.trim().split('|').map(|s| s.to_string()).collect();
                    if let Some(first) = choices.first()
                        && let Ok(c) = CString::new(first.as_str())
                    {
                        host().option_defaults.insert(key.clone(), c);
                    }
                    defs.push(VarDef { key, desc: desc.trim().to_string(), choices });
                    v = v.add(1);
                }
            }
            true
        }
        ENV_SET_SYSTEM_AV_INFO | ENV_SET_GEOMETRY => {
            // geometry/timing changes: note dirty; frontend re-reads av_info
            host().av_dirty = true;
            true
        }
        ENV_SET_CONTROLLER_INFO | ENV_SET_SUPPORT_ACHIEVEMENTS => true,
        ENV_SET_MEMORY_MAPS => {
            let map = data as *const RetroMemoryMap;
            let mut out = Vec::new();
            unsafe {
                let n = (*map).num_descriptors as usize;
                for i in 0..n {
                    let d = &*(*map).descriptors.add(i);
                    if d.ptr.is_null() || d.len == 0 {
                        continue;
                    }
                    out.push(MemDesc {
                        ptr: d.ptr as usize,
                        offset: d.offset,
                        start: d.start,
                        select: d.select,
                        len: d.len,
                    });
                }
            }
            host().mem_maps = out;
            true
        }
        ENV_SET_DISK_CONTROL_INTERFACE => {
            host().disk_ctrl =
                Some(unsafe { (*(data as *const DiskControl)).clone() });
            true
        }
        ENV_SET_FRAME_TIME_CALLBACK => {
            // Not optional for every core: EasyRPG's LibretroClock keeps its
            // whole notion of time in a counter this callback increments, so
            // refusing here froze the clock at zero -- the game loaded, drew
            // blank frames forever and produced no audio, with no error.
            let ft = data as *const FrameTimeCallback;
            if ft.is_null() {
                return false;
            }
            unsafe {
                host().frame_time_cb = (*ft).callback;
                host().frame_time_ref = (*ft).reference;
            }
            host().frame_time_last = None;
            true
        }
        ENV_SET_AUDIO_CALLBACK => {
            // Also not optional for every core. EasyRPG's libretro backend
            // emits NO audio from retro_run at all -- its only path out is
            // this callback, decoding exactly one frame per call. Refusing
            // here meant permanent silence with no warning of any kind.
            // The core also re-registers a NULL pair on unload, so a null
            // callback must clear our stored pointers, not be called.
            let ac = data as *const AudioCallback;
            if ac.is_null() {
                return false;
            }
            unsafe {
                host().audio_cb = (*ac).callback;
                host().audio_set_state = (*ac).set_state;
                // cores start muted until the frontend says otherwise
                if let (Some(set), Some(_)) = (host().audio_set_state, host().audio_cb) {
                    set(true);
                }
            }
            true
        }
        ENV_GET_LOG_INTERFACE => {
            // must be filled: handy (and friends) read the struct without
            // checking our return value and call whatever is in it
            unsafe { *(data as *mut *const c_void) = kui_core_log_shim as *const c_void };
            true
        }
        ENV_GET_VARIABLE => {
            let var = data as *mut Variable;
            let key = unsafe {
                if (*var).key.is_null() {
                    return false;
                }
                CStr::from_ptr((*var).key).to_string_lossy().into_owned()
            };
            if let Some(v) = host().options.get(&key) {
                unsafe { (*var).value = v.as_ptr() };
                true
            } else if let Some(v) = host().option_defaults.get(&key) {
                unsafe { (*var).value = v.as_ptr() };
                true
            } else {
                unsafe { (*var).value = std::ptr::null() };
                false
            }
        }
        ENV_GET_VARIABLE_UPDATE => {
            let dirty = host().vars_dirty;
            host().vars_dirty = false;
            unsafe { *(data as *mut bool) = dirty };
            true
        }
        // contentless cores (ports ship these) announce they run without
        // a game; acknowledging lets retro_load_game(NULL) succeed
        ENV_SET_SUPPORT_NO_GAME => true,
        _ => false,
    }
}

// The variadic function cores are actually handed (csrc/log_shim.c). Stable
// Rust can declare a variadic extern fn but not define one, so the shim does
// the vsnprintf and calls kui_core_log_line below with a finished string.
unsafe extern "C" {
    fn kui_core_log_shim(level: c_uint, fmt: *const c_char, ...);
}

/// Levels per libretro.h: 0 DEBUG, 1 INFO, 2 WARN, 3 ERROR.
const LOG_WARN: c_uint = 2;

/// Core log sink, called by the shim with the arguments already expanded.
///
/// Warnings and errors ALWAYS print: a core explaining why it refused a game
/// is the single most useful thing on the way to a black screen, and dropping
/// it unless someone happened to set KUI_TRACE cost a long debugging session.
/// Debug/info stay behind KUI_TRACE so normal play is quiet.
///
/// # Safety
/// `msg` must be NUL-terminated and valid for the duration of the call. The
/// only caller is the shim, which passes its own stack buffer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kui_core_log_line(level: c_uint, msg: *const c_char) {
    if msg.is_null() {
        return;
    }
    if level < LOG_WARN && std::env::var_os("KUI_TRACE").is_none() {
        return;
    }
    let s = unsafe { CStr::from_ptr(msg) }.to_string_lossy();
    let tag = match level {
        0 => "debug",
        1 => "info",
        2 => "warn",
        _ => "error",
    };
    // cores are inconsistent about trailing newlines
    eprintln!("core[{tag}] {}", s.trim_end_matches('\n'));
}

#[repr(C)]
struct RetroMemoryDescriptor {
    flags: u64,
    ptr: *mut c_void,
    offset: usize,
    start: usize,
    select: usize,
    disconnect: usize,
    len: usize,
    addrspace: *const c_char,
}
#[repr(C)]
struct RetroMemoryMap {
    descriptors: *const RetroMemoryDescriptor,
    num_descriptors: c_uint,
}
/// A captured memory-map descriptor (ptr stored as usize).
#[derive(Clone, Copy)]
pub struct MemDesc {
    pub ptr: usize,
    pub offset: usize,
    pub start: usize,
    pub select: usize,
    pub len: usize,
}

/// retro_disk_control_callback — the classic (v0) interface.
#[repr(C)]
#[derive(Clone)]
pub struct DiskControl {
    pub set_eject_state: unsafe extern "C" fn(bool) -> bool,
    pub get_eject_state: unsafe extern "C" fn() -> bool,
    pub get_image_index: unsafe extern "C" fn() -> c_uint,
    pub set_image_index: unsafe extern "C" fn(c_uint) -> bool,
    pub get_num_images: unsafe extern "C" fn() -> c_uint,
    pub replace_image_index: *const c_void,
    pub add_image_index: *const c_void,
}

unsafe extern "C" fn cb_video_refresh(
    data: *const c_void,
    width: c_uint,
    height: c_uint,
    pitch: usize,
) {
    if data.is_null() {
        return; // duped frame
    }
    let h = host();
    let (w, ht) = (width as usize, height as usize);
    h.video.resize(w * ht * 4, 0);
    h.video_w = width;
    h.video_h = height;
    let src = data as *const u8;
    match h.pixel_format {
        PixelFormat::Xrgb8888 => {
            for y in 0..ht {
                let row = unsafe { std::slice::from_raw_parts(src.add(y * pitch), w * 4) };
                for x in 0..w {
                    let o = (y * w + x) * 4;
                    h.video[o] = row[x * 4 + 2];
                    h.video[o + 1] = row[x * 4 + 1];
                    h.video[o + 2] = row[x * 4];
                    h.video[o + 3] = 255;
                }
            }
        }
        PixelFormat::Rgb565 => {
            for y in 0..ht {
                let row =
                    unsafe { std::slice::from_raw_parts(src.add(y * pitch) as *const u16, w) };
                for (x, &p) in row.iter().enumerate() {
                    let o = (y * w + x) * 4;
                    h.video[o] = (((p >> 11) & 0x1F) as u8) << 3;
                    h.video[o + 1] = (((p >> 5) & 0x3F) as u8) << 2;
                    h.video[o + 2] = ((p & 0x1F) as u8) << 3;
                    h.video[o + 3] = 255;
                }
            }
        }
        PixelFormat::Xrgb1555 => {
            for y in 0..ht {
                let row =
                    unsafe { std::slice::from_raw_parts(src.add(y * pitch) as *const u16, w) };
                for (x, &p) in row.iter().enumerate() {
                    let o = (y * w + x) * 4;
                    h.video[o] = (((p >> 10) & 0x1F) as u8) << 3;
                    h.video[o + 1] = (((p >> 5) & 0x1F) as u8) << 3;
                    h.video[o + 2] = ((p & 0x1F) as u8) << 3;
                    h.video[o + 3] = 255;
                }
            }
        }
    }
    h.video_dirty = true;
}

unsafe extern "C" fn cb_audio_sample(left: i16, right: i16) {
    let h = host();
    h.audio.push(left);
    h.audio.push(right);
}

unsafe extern "C" fn cb_audio_sample_batch(data: *const i16, frames: usize) -> usize {
    let h = host();
    let s = unsafe { std::slice::from_raw_parts(data, frames * 2) };
    h.audio.extend_from_slice(s);
    frames
}

unsafe extern "C" fn cb_input_poll() {}

unsafe extern "C" fn cb_input_state(port: c_uint, device: c_uint, index: c_uint, id: c_uint) -> i16 {
    if port != 0 {
        return 0;
    }
    let h = host();
    match device {
        // RETRO_DEVICE_JOYPAD
        1 if id < 16 => {
            // stick mode: the dpad belongs to the left analog, so the
            // digital dpad reads empty (both at once confuses cores)
            if h.dpad_as_stick && (4..=7).contains(&id) {
                return 0;
            }
            ((h.pad >> id) & 1) as i16
        }
        // RETRO_DEVICE_ANALOG: index 0 = left stick, id 0/1 = X/Y.
        // The device has no physical sticks; in stick mode the dpad
        // drives the left stick at full deflection (PS1-class games).
        5 if h.dpad_as_stick && index == 0 => {
            let b = |i: u32| ((h.pad >> i) & 1) as i16;
            match id {
                0 => (b(7) - b(6)) * 0x7FFF, // right - left
                1 => (b(5) - b(4)) * 0x7FFF, // down - up
                _ => 0,
            }
        }
        _ => 0,
    }
}

/// Enumerate a core's option definitions without loading a game:
/// dlopen, set_environment (captures SET_VARIABLES), init, deinit.
/// kUI option defaults where a core's first-listed choice is wrong for
/// this device. Precedence everywhere: cfg core.*/game.* value > this
/// table > the core's first-listed choice.
pub fn kui_option_defaults(core_stem: &str) -> &'static [(&'static str, &'static str)] {
    match core_stem {
        // SGB border art shrinks the game inside our bezels (Arjun call)
        "mgba" => &[("mgba_sgb_borders", "OFF")],
        _ => &[],
    }
}

/// Single-key lookup into [`kui_option_defaults`] for the options UIs.
pub fn kui_option_default(core_stem: &str, key: &str) -> Option<&'static str> {
    kui_option_defaults(core_stem).iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
}

pub fn enumerate_options(core_path: &Path, system_dir: &Path) -> Result<Vec<VarDef>, String> {
    unsafe {
        HOST = Some(HostState {
            options: std::collections::HashMap::new(),
            option_defaults: std::collections::HashMap::new(),
            disk_ctrl: None,
            mem_maps: Vec::new(),
            mem_fns: None,
            var_defs: Vec::new(),
            vars_dirty: false,
            dpad_as_stick: false,
            video: Vec::new(),
            video_w: 0,
            video_h: 0,
            video_dirty: false,
            audio: Vec::new(),
            pad: 0,
            pixel_format: PixelFormat::Xrgb1555,
            av_dirty: false,
            // the real BIOS dir, never /tmp: pcsx_rearmed scans system_dir
            // during retro_init and open()s every entry — a FIFO in the dir
            // (/tmp/.mtp_fifo) blocks that open forever
            system_dir: CString::new(system_dir.display().to_string()).unwrap(),
            save_dir: CString::new("/tmp").unwrap(),
            frame_time_cb: None,
            frame_time_ref: 0,
            frame_time_last: None,
            audio_cb: None,
            audio_set_state: None,
        });
    }
    let lib = unsafe { libloading::Library::new(core_path) }.map_err(|e| e.to_string())?;
    unsafe {
        let set_environment: libloading::Symbol<
            unsafe extern "C" fn(unsafe extern "C" fn(c_uint, *mut c_void) -> bool),
        > = lib.get(b"retro_set_environment").map_err(|e| e.to_string())?;
        set_environment(cb_environment);
        // full callback set: some cores touch these even during init
        let sv: libloading::Symbol<
            unsafe extern "C" fn(unsafe extern "C" fn(*const c_void, c_uint, c_uint, usize)),
        > = lib.get(b"retro_set_video_refresh").map_err(|e| e.to_string())?;
        sv(cb_video_refresh);
        let sa: libloading::Symbol<unsafe extern "C" fn(unsafe extern "C" fn(i16, i16))> =
            lib.get(b"retro_set_audio_sample").map_err(|e| e.to_string())?;
        sa(cb_audio_sample);
        let sab: libloading::Symbol<
            unsafe extern "C" fn(unsafe extern "C" fn(*const i16, usize) -> usize),
        > = lib.get(b"retro_set_audio_sample_batch").map_err(|e| e.to_string())?;
        sab(cb_audio_sample_batch);
        let sip: libloading::Symbol<unsafe extern "C" fn(unsafe extern "C" fn())> =
            lib.get(b"retro_set_input_poll").map_err(|e| e.to_string())?;
        sip(cb_input_poll);
        let sis: libloading::Symbol<
            unsafe extern "C" fn(unsafe extern "C" fn(c_uint, c_uint, c_uint, c_uint) -> i16),
        > = lib.get(b"retro_set_input_state").map_err(|e| e.to_string())?;
        sis(cb_input_state);
        let init: libloading::Symbol<unsafe extern "C" fn()> =
            lib.get(b"retro_init").map_err(|e| e.to_string())?;
        init();
        let deinit: libloading::Symbol<unsafe extern "C" fn()> =
            lib.get(b"retro_deinit").map_err(|e| e.to_string())?;
        deinit();
    }
    let defs = std::mem::take(&mut host().var_defs);
    unsafe { HOST = None };
    Ok(defs)
}

// ---- the core ----

pub struct Core {
    _lib: libloading::Library,
    run: unsafe extern "C" fn(),
    unload_game: unsafe extern "C" fn(),
    cheat_reset: unsafe extern "C" fn(),
    cheat_set: unsafe extern "C" fn(c_uint, bool, *const c_char),
    deinit: unsafe extern "C" fn(),
    reset: unsafe extern "C" fn(),
    get_av: unsafe extern "C" fn(*mut SystemAvInfo),
    serialize_size: unsafe extern "C" fn() -> usize,
    serialize: unsafe extern "C" fn(*mut c_void, usize) -> bool,
    unserialize: unsafe extern "C" fn(*const c_void, usize) -> bool,
    get_memory_data: unsafe extern "C" fn(c_uint) -> *mut c_void,
    get_memory_size: unsafe extern "C" fn(c_uint) -> usize,
    pub av_info: SystemAvInfo,
    pub name: String,
}

const MEMORY_SAVE_RAM: c_uint = 0;
const MEMORY_RTC: c_uint = 2;

impl Core {
    /// Whether a core natively accepts .zip content (per valid_extensions).
/// get_system_info is callable without init, so this is a cheap peek.
pub fn core_supports_zip(core_path: &Path) -> bool {
    let Ok(lib) = (unsafe { libloading::Library::new(core_path) }) else {
        return false;
    };
    let mut sysinfo = SystemInfo {
        library_name: std::ptr::null(),
        library_version: std::ptr::null(),
        valid_extensions: std::ptr::null(),
        need_fullpath: false,
        block_extract: false,
    };
    unsafe {
        let Ok(get_sys) = lib
            .get::<unsafe extern "C" fn(*mut SystemInfo)>(b"retro_get_system_info")
        else {
            return false;
        };
        get_sys(&mut sysinfo);
        if sysinfo.valid_extensions.is_null() {
            return false;
        }
        CStr::from_ptr(sysinfo.valid_extensions)
            .to_string_lossy()
            .split('|')
            .any(|e| e.eq_ignore_ascii_case("zip"))
    }
}

/// Load a core, init callbacks, load the ROM. `system_dir` = BIOS dir.
    /// `rom_path: None` = contentless launch (retro_load_game(NULL));
    /// only meaningful for cores that support no-game.
    pub fn load(
        core_path: &Path,
        rom_path: Option<&Path>,
        system_dir: &Path,
        save_dir: &Path,
        options: impl IntoIterator<Item = (String, String)>,
    ) -> Result<Self, String> {
        // options must be servable BEFORE retro_load_game below:
        // restart-gated options (SGB borders, BIOS toggles, ...) are read
        // during load and would otherwise silently get the first choice
        let opt_map: std::collections::HashMap<String, CString> = options
            .into_iter()
            .filter_map(|(k, v)| CString::new(v).ok().map(|c| (k, c)))
            .collect();
        unsafe {
            HOST = Some(HostState {
                options: opt_map,
                option_defaults: std::collections::HashMap::new(),
                disk_ctrl: None,
                mem_maps: Vec::new(),
                mem_fns: None,
                var_defs: Vec::new(),
                vars_dirty: false,
                dpad_as_stick: false,
                video: Vec::new(),
                video_w: 0,
                video_h: 0,
                video_dirty: false,
                audio: Vec::new(),
                pad: 0,
                pixel_format: PixelFormat::Xrgb1555,
                av_dirty: false,
                system_dir: CString::new(system_dir.to_string_lossy().as_bytes())
                    .map_err(|e| e.to_string())?,
                save_dir: CString::new(save_dir.to_string_lossy().as_bytes())
                    .map_err(|e| e.to_string())?,
                frame_time_cb: None,
                frame_time_ref: 0,
                frame_time_last: None,
                audio_cb: None,
                audio_set_state: None,
            });
        }

        let lib = unsafe { libloading::Library::new(core_path) }.map_err(|e| e.to_string())?;
        macro_rules! sym {
            ($name:literal, $ty:ty) => {
                *unsafe { lib.get::<$ty>($name.as_bytes()) }.map_err(|e| e.to_string())?
            };
        }

        let set_environment: unsafe extern "C" fn(unsafe extern "C" fn(c_uint, *mut c_void) -> bool) =
            sym!("retro_set_environment", _);
        let set_video: unsafe extern "C" fn(unsafe extern "C" fn(*const c_void, c_uint, c_uint, usize)) =
            sym!("retro_set_video_refresh", _);
        let set_audio: unsafe extern "C" fn(unsafe extern "C" fn(i16, i16)) =
            sym!("retro_set_audio_sample", _);
        let set_audio_batch: unsafe extern "C" fn(unsafe extern "C" fn(*const i16, usize) -> usize) =
            sym!("retro_set_audio_sample_batch", _);
        let set_input_poll: unsafe extern "C" fn(unsafe extern "C" fn()) =
            sym!("retro_set_input_poll", _);
        let set_input_state: unsafe extern "C" fn(
            unsafe extern "C" fn(c_uint, c_uint, c_uint, c_uint) -> i16,
        ) = sym!("retro_set_input_state", _);
        let init: unsafe extern "C" fn() = sym!("retro_init", _);
        let load_game: unsafe extern "C" fn(*const GameInfo) -> bool = sym!("retro_load_game", _);
        let get_av: unsafe extern "C" fn(*mut SystemAvInfo) = sym!("retro_get_system_av_info", _);
        let get_sys: unsafe extern "C" fn(*mut SystemInfo) = sym!("retro_get_system_info", _);
        let run: unsafe extern "C" fn() = sym!("retro_run", _);
        let reset: unsafe extern "C" fn() = sym!("retro_reset", _);
        let unload_game: unsafe extern "C" fn() = sym!("retro_unload_game", _);
        let cheat_reset: unsafe extern "C" fn() = sym!("retro_cheat_reset", _);
        let cheat_set: unsafe extern "C" fn(c_uint, bool, *const c_char) =
            sym!("retro_cheat_set", _);
        let deinit: unsafe extern "C" fn() = sym!("retro_deinit", _);
        let serialize_size: unsafe extern "C" fn() -> usize = sym!("retro_serialize_size", _);
        let serialize: unsafe extern "C" fn(*mut c_void, usize) -> bool = sym!("retro_serialize", _);
        let unserialize: unsafe extern "C" fn(*const c_void, usize) -> bool =
            sym!("retro_unserialize", _);
        let get_memory_data: unsafe extern "C" fn(c_uint) -> *mut c_void =
            sym!("retro_get_memory_data", _);
        let get_memory_size: unsafe extern "C" fn(c_uint) -> usize =
            sym!("retro_get_memory_size", _);

        unsafe {
            set_environment(cb_environment);
            set_video(cb_video_refresh);
            set_audio(cb_audio_sample);
            set_audio_batch(cb_audio_sample_batch);
            set_input_poll(cb_input_poll);
            set_input_state(cb_input_state);
            init();
        }

        // system info: honor need_fullpath
        let mut sysinfo = SystemInfo {
            library_name: std::ptr::null(),
            library_version: std::ptr::null(),
            valid_extensions: std::ptr::null(),
            need_fullpath: false,
            block_extract: false,
        };
        unsafe { get_sys(&mut sysinfo) };
        let name = unsafe {
            if sysinfo.library_name.is_null() {
                "core".into()
            } else {
                CStr::from_ptr(sysinfo.library_name).to_string_lossy().into_owned()
            }
        };

        let trace = std::env::var_os("KUI_TRACE").is_some();
        let ok = match rom_path {
            Some(rom_path) => {
                let cpath = CString::new(rom_path.to_string_lossy().as_bytes())
                    .map_err(|e| e.to_string())?;
                let data = if sysinfo.need_fullpath {
                    Vec::new()
                } else {
                    std::fs::read(rom_path).map_err(|e| e.to_string())?
                };
                let gi = GameInfo {
                    path: cpath.as_ptr(),
                    data: if data.is_empty() {
                        std::ptr::null()
                    } else {
                        data.as_ptr() as *const c_void
                    },
                    size: data.len(),
                    meta: std::ptr::null(),
                };
                if trace {
                    eprintln!(
                        "load_game: {} bytes, need_fullpath={}, path={:?}",
                        data.len(),
                        sysinfo.need_fullpath,
                        rom_path
                    );
                }
                unsafe { load_game(&gi) }
            }
            None => {
                if trace {
                    eprintln!("load_game: contentless (NULL)");
                }
                unsafe { load_game(std::ptr::null()) }
            }
        };
        if trace {
            eprintln!("load_game returned {ok}");
        }
        if !ok {
            return Err("core rejected the ROM".into());
        }

        host().mem_fns = Some((get_memory_data, get_memory_size));

        let mut av = SystemAvInfo {
            geometry: GameGeometry {
                base_width: 0,
                base_height: 0,
                max_width: 0,
                max_height: 0,
                aspect_ratio: 0.0,
            },
            timing: SystemTiming { fps: 60.0, sample_rate: 44100.0 },
        };
        unsafe { get_av(&mut av) };

        Ok(Self {
            _lib: lib,
            run,
            reset,
            get_av,
            unload_game,
            cheat_reset,
            cheat_set,
            deinit,
            serialize_size,
            serialize,
            unserialize,
            get_memory_data,
            get_memory_size,
            av_info: av,
            name,
        })
    }

    pub fn run_frame(&self) {
        // Tick the core's clock BEFORE running the frame. Cores that keep
        // their own time ignore this; cores with an external clock (EasyRPG)
        // do not advance at all without it.
        let h = host();
        if let Some(cb) = h.frame_time_cb {
            // reference = one frame at the core's target fps; fall back to
            // ~60fps if a core hands us something useless
            let refr = if h.frame_time_ref > 0 { h.frame_time_ref } else { 16_667 };
            let usec = match h.frame_time_last {
                None => refr,
                // Clamp the measured delta: after a suspend/resume the wall
                // clock can have jumped by hours, and handing a core that as
                // one frame would fast-forward the game.
                Some(t) => (t.elapsed().as_micros() as i64).clamp(1, refr.saturating_mul(4)),
            };
            h.frame_time_last = Some(std::time::Instant::now());
            unsafe { cb(usec) };
        }
        unsafe { (self.run)() };
        // Pull one frame of audio from cores that use the audio-callback
        // interface. They push nothing during retro_run, so without this
        // they are simply silent.
        if let Some(cb) = h.audio_cb {
            unsafe { cb() };
        }
    }

    pub fn reset(&self) {
        unsafe { (self.reset)() };
    }

    /// Refresh av_info if the core changed it mid-game.
    pub fn refresh_av(&mut self) -> bool {
        if host().av_dirty {
            host().av_dirty = false;
            unsafe { (self.get_av)(&mut self.av_info) };
            true
        } else {
            false
        }
    }

    /// Dpad-as-left-stick mode for stick-needing games on a stickless
    /// device: digital dpad goes quiet, analog reads full deflection.
    pub fn set_dpad_as_stick(&self, on: bool) {
        host().dpad_as_stick = on;
    }

    pub fn set_pad(&self, bits: u32) {
        host().pad = bits;
    }

    /// Provide core option values (key -> value) before/after load.
    pub fn set_options(&self, opts: impl IntoIterator<Item = (String, String)>) {
        let map = &mut host().options;
        for (k, v) in opts {
            if let Ok(c) = CString::new(v) {
                map.insert(k, c);
            }
        }
    }

    /// Captured option definitions (empty for cores without options).
    pub fn var_defs(&self) -> &[VarDef] {
        &host().var_defs
    }

    /// Current effective value for an option key.
    pub fn var_value(&self, key: &str) -> Option<String> {
        host()
            .options
            .get(key)
            .map(|c| c.to_string_lossy().into_owned())
            .or_else(|| {
                host()
                    .var_defs
                    .iter()
                    .find(|d| d.key == key)
                    .and_then(|d| d.choices.first().cloned())
            })
    }

    /// Set an option value and flag the core to re-read.
    pub fn set_var(&self, key: &str, value: &str) {
        if let Ok(c) = CString::new(value) {
            host().options.insert(key.to_string(), c);
            host().vars_dirty = true;
        }
    }

    /// Take the frame produced this run (if any) as (w, h, rgba).
    pub fn take_video(&self) -> Option<(u32, u32, Vec<u8>)> {
        let h = host();
        if !h.video_dirty {
            return None;
        }
        h.video_dirty = false;
        Some((h.video_w, h.video_h, std::mem::take(&mut h.video)))
    }

    /// Drain queued audio (interleaved stereo i16).
    pub fn take_audio(&self) -> Vec<i16> {
        std::mem::take(&mut host().audio)
    }

    pub fn sram(&self) -> Option<&[u8]> {
        unsafe {
            let size = (self.get_memory_size)(MEMORY_SAVE_RAM);
            let data = (self.get_memory_data)(MEMORY_SAVE_RAM);
            if size == 0 || data.is_null() {
                None
            } else {
                Some(std::slice::from_raw_parts(data as *const u8, size))
            }
        }
    }

    pub fn rtc(&self) -> Option<&[u8]> {
        unsafe {
            let size = (self.get_memory_size)(MEMORY_RTC);
            let data = (self.get_memory_data)(MEMORY_RTC);
            if size == 0 || data.is_null() {
                None
            } else {
                Some(std::slice::from_raw_parts(data as *const u8, size))
            }
        }
    }

    pub fn load_rtc(&self, bytes: &[u8]) {
        unsafe {
            let size = (self.get_memory_size)(MEMORY_RTC);
            let data = (self.get_memory_data)(MEMORY_RTC);
            if !data.is_null() && size > 0 {
                let n = size.min(bytes.len());
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), data as *mut u8, n);
            }
        }
    }

    pub fn load_sram(&self, bytes: &[u8]) {
        unsafe {
            let size = (self.get_memory_size)(MEMORY_SAVE_RAM);
            let data = (self.get_memory_data)(MEMORY_SAVE_RAM);
            if !data.is_null() && size > 0 {
                let n = size.min(bytes.len());
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), data as *mut u8, n);
            }
        }
    }

    pub fn save_state(&self) -> Option<Vec<u8>> {
        unsafe {
            let size = (self.serialize_size)();
            if size == 0 {
                return None;
            }
            let mut buf = vec![0u8; size];
            (self.serialize)(buf.as_mut_ptr() as *mut c_void, size).then_some(buf)
        }
    }

    /// Clear all cheats, then apply the enabled ones in file order.
    ///
    /// The index handed to `retro_cheat_set` counts only the cheats we
    /// actually send, not their position in the file. Cores assign slots
    /// sequentially as they receive them, and several then key the enable
    /// flag off that slot: PCSX-ReARMed appends via `AddCheat` (which
    /// leaves `Enabled = 0`) and only sets `Cheats[index].Enabled` when
    /// `index < NumCheats`. Passing a file index leaves every cheat past
    /// the first gap loaded but switched off.
    pub fn apply_cheats(&self, cheats: &[(bool, String)]) {
        unsafe {
            (self.cheat_reset)();
            let mut slot: c_uint = 0;
            for (on, code) in cheats.iter() {
                if *on && let Ok(c) = CString::new(code.as_str()) {
                    (self.cheat_set)(slot, true, c.as_ptr());
                    slot += 1;
                }
            }
        }
    }

    /// See [`read_memory_global`] — method form.
    /// Read emulated memory at a console address for achievement checks:
    /// walk the core's memory map, fall back to system RAM at offset 0.
    pub fn read_memory(&self, addr: u32, buf: &mut [u8]) -> u32 {
        let addr = addr as usize;
        for d in host().mem_maps.iter() {
            let hit = if d.select != 0 {
                (addr & d.select) == d.start
            } else {
                addr >= d.start && addr < d.start + d.len
            };
            if !hit {
                continue;
            }
            let idx = if d.select != 0 {
                (addr - d.start) % d.len.max(1)
            } else {
                addr - d.start
            };
            let avail = d.len.saturating_sub(idx);
            let n = buf.len().min(avail);
            unsafe {
                std::ptr::copy_nonoverlapping(
                    (d.ptr + d.offset + idx) as *const u8,
                    buf.as_mut_ptr(),
                    n,
                );
            }
            return n as u32;
        }
        // no maps: treat the address as an offset into system RAM
        const MEMORY_SYSTEM_RAM: c_uint = 2; // RETRO_MEMORY_SYSTEM_RAM
        let ram = unsafe { (self.get_memory_data)(MEMORY_SYSTEM_RAM) };
        let size = unsafe { (self.get_memory_size)(MEMORY_SYSTEM_RAM) };
        if ram.is_null() || addr >= size {
            return 0;
        }
        let n = buf.len().min(size - addr);
        unsafe {
            std::ptr::copy_nonoverlapping((ram as usize + addr) as *const u8, buf.as_mut_ptr(), n);
        }
        n as u32
    }

    /// Number of disc images the core reported (1 when no disk interface).
    pub fn disc_count(&self) -> u32 {
        host().disk_ctrl.as_ref().map(|d| unsafe { (d.get_num_images)() }).unwrap_or(0).max(1)
    }
    pub fn disc_index(&self) -> u32 {
        host().disk_ctrl.as_ref().map(|d| unsafe { (d.get_image_index)() }).unwrap_or(0)
    }
    /// Eject, swap, close — the physical ritual, minus the dust.
    pub fn disc_set(&mut self, idx: u32) -> bool {
        let Some(d) = host().disk_ctrl.clone() else {
            return false;
        };
        unsafe {
            (d.set_eject_state)(true);
            let ok = (d.set_image_index)(idx);
            (d.set_eject_state)(false);
            ok
        }
    }
    pub fn load_state(&self, buf: &[u8]) -> bool {
        unsafe { (self.unserialize)(buf.as_ptr() as *const c_void, buf.len()) }
    }
}

impl Drop for Core {
    fn drop(&mut self) {
        unsafe {
            (self.unload_game)();
            (self.deinit)();
        }
    }
}

/// Read emulated memory without a Core borrow (for 'static callbacks):
/// same memory-map walk, falling back to system RAM at offset 0.
pub fn read_memory_global(addr: u32, buf: &mut [u8]) -> u32 {
    let addr = addr as usize;
    for d in host().mem_maps.iter() {
        let hit = if d.select != 0 {
            (addr & d.select) == d.start
        } else {
            addr >= d.start && addr < d.start + d.len
        };
        if !hit {
            continue;
        }
        let idx = if d.select != 0 { (addr - d.start) % d.len.max(1) } else { addr - d.start };
        let avail = d.len.saturating_sub(idx);
        let n = buf.len().min(avail);
        unsafe {
            std::ptr::copy_nonoverlapping(
                (d.ptr + d.offset + idx) as *const u8,
                buf.as_mut_ptr(),
                n,
            );
        }
        return n as u32;
    }
    let Some((gmd, gms)) = host().mem_fns else {
        return 0;
    };
    const MEMORY_SYSTEM_RAM: c_uint = 2;
    let ram = unsafe { gmd(MEMORY_SYSTEM_RAM) };
    let size = unsafe { gms(MEMORY_SYSTEM_RAM) };
    if ram.is_null() || addr >= size {
        return 0;
    }
    let n = buf.len().min(size - addr);
    unsafe {
        std::ptr::copy_nonoverlapping((ram as usize + addr) as *const u8, buf.as_mut_ptr(), n);
    }
    n as u32
}

/// How many memory-map descriptors the core registered.
pub fn mem_maps_len() -> usize {
    host().mem_maps.len()
}

/// Read from a specific retro memory bank (for the console-region
/// fallback): id per RETRO_MEMORY_*, offset within that bank.
pub fn read_memory_kind(mem_id: u32, off: usize, buf: &mut [u8]) -> u32 {
    let Some((gmd, gms)) = host().mem_fns else {
        return 0;
    };
    let ram = unsafe { gmd(mem_id) };
    let size = unsafe { gms(mem_id) };
    if ram.is_null() || off >= size {
        return 0;
    }
    let n = buf.len().min(size - off);
    unsafe {
        std::ptr::copy_nonoverlapping((ram as usize + off) as *const u8, buf.as_mut_ptr(), n);
    }
    n as u32
}
