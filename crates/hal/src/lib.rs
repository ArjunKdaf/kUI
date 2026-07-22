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
