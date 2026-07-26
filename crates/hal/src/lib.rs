//! Platform abstraction. One trait, one impl per device.
//!
//! Adding a device (Brick Pro U, ...) means implementing [`Platform`] in a new
//! module — nothing outside this crate changes.

pub mod sdl;
pub mod tg5040;

#[cfg(feature = "desktop")]
pub mod desktop;

/// Battery snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatteryStatus {
    /// 0..=100
    pub percent: u8,
    pub charging: bool,
}

/// Physical buttons, named for position (Nintendo layout on the Hammer).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Button {
    Up,
    Down,
    Left,
    Right,
    A,
    B,
    X,
    Y,
    L1,
    R1,
    L2,
    R2,
    L3,
    R3,
    Start,
    Select,
    Menu,
    Power,
    VolUp,
    VolDown,
    Fn1,
    Fn2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonState {
    Pressed,
    Released,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputEvent {
    Button(Button, ButtonState),
    /// D-pad state snapshot (the tg5040 d-pad is an SDL hat).
    Dpad { up: bool, down: bool, left: bool, right: bool },
    /// FN/mute hardware switch.
    FnSwitch(bool),
    /// Headphone jack plug state.
    Jack(bool),
}

/// One addressable RGB light channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

/// CPU scaling policy (SPEC: Power Profile — Auto / Performance / Powersave).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Governor {
    #[default]
    Auto,
    Performance,
    Powersave,
}

/// The device. Everything the OS needs from hardware, and nothing else.
///
/// Contract notes:
/// - `display_*` owns the GLES2 context and vsync (SPEC: GPU rendering).
/// - `sleep` must be deep sleep (SPEC: instant wake, near-zero draw).
/// - `poweroff` must run the safe-shutdown path (PMIC soft-disconnect).
pub trait Platform {
    // -- identity --
    fn device_name(&self) -> &str;

    // -- display --
    fn display_size(&self) -> (u32, u32);
    /// Swap buffers; blocks on vsync.
    fn display_present(&mut self);
    fn set_backlight(&mut self, level: u8);

    // -- input --
    /// Non-blocking drain of pending events.
    fn poll_input(&mut self, out: &mut Vec<InputEvent>);

    // -- audio --
    /// Push interleaved stereo i16 frames; returns frames accepted.
    fn audio_write(&mut self, frames: &[i16]) -> usize;
    fn audio_set_volume(&mut self, level: u8);

    // -- power --
    fn battery(&self) -> BatteryStatus;
    fn set_governor(&mut self, gov: Governor);
    fn sleep(&mut self);
    fn poweroff(&mut self) -> !;
    fn reboot(&mut self) -> !;

    // -- leds --
    fn led_count(&self) -> usize;
    fn set_led(&mut self, index: usize, color: Rgb, brightness: u8);

    // -- rtc --
    fn set_datetime(&mut self, unix_seconds: i64);

    // -- rumble --
    fn rumble(&mut self, strength: u8);
}

/// Hold-to-scroll repeat with hat-bounce debounce, device-tuned on the
/// Hammer: instant first step, 260ms delay, 85ms rate, 20ms bounce window
/// (80ms swallowed deliberate fast taps). Shared by the launcher and the
/// frontend so every list in the OS scrolls the same way.
pub struct Repeat {
    pub held: [[bool; 2]; 3],
    dir: i32,
    prev_dir: i32,
    since: std::time::Instant,
    last: std::time::Instant,
    released_at: std::time::Instant,
}

impl Repeat {
    const DELAY: std::time::Duration = std::time::Duration::from_millis(260);
    const RATE: std::time::Duration = std::time::Duration::from_millis(85);
    const BOUNCE: std::time::Duration = std::time::Duration::from_millis(20);

    pub fn new() -> Self {
        let now = std::time::Instant::now();
        Self {
            held: [[false; 2]; 3],
            dir: 0,
            prev_dir: 0,
            since: now,
            last: now,
            released_at: now - Self::BOUNCE,
        }
    }

    pub fn clear(&mut self) {
        self.held = [[false; 2]; 3];
        self.dir = 0;
    }

    /// True while the user is holding a direction (repeat active).
    pub fn holding(&self) -> bool {
        self.dir != 0
    }

    /// Call once per frame after feeding `held`; returns -1/0/+1 steps.
    pub fn step(&mut self, now: std::time::Instant) -> i32 {
        let want: i32 = i32::from(self.held.iter().any(|h| h[1]))
            - i32::from(self.held.iter().any(|h| h[0]));
        let mut step = 0;
        if want != self.dir {
            if want == 0 {
                self.prev_dir = self.dir;
                self.released_at = now;
                self.dir = 0;
            } else if self.dir == 0
                && want == self.prev_dir
                && now - self.released_at < Self::BOUNCE
            {
                self.dir = want; // bounce: resume silently
            } else {
                self.dir = want;
                step = want;
                self.since = now;
                self.last = now;
            }
        }
        if self.dir != 0 && now - self.since > Self::DELAY && now - self.last >= Self::RATE {
            step = self.dir;
            self.last = now;
        }
        step
    }
}

impl Default for Repeat {
    fn default() -> Self {
        Self::new()
    }
}
