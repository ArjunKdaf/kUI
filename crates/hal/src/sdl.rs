//! SDL2-backed platform glue: window/GLES2 context, input, vsync present.
//!
//! One code path for both the device (fullscreen, TrimUI's SDL2 2.26 fork)
//! and the desktop dev window. Raw DRM/evdev is a later optimization behind
//! the same [`crate::Platform`] trait if benchmarks demand it.

use glow::HasContext as _;

use crate::tg5040;
use crate::{Button, ButtonState, InputEvent};

/// Hammer panel.
pub const DEVICE_WIDTH: u32 = 1024;
pub const DEVICE_HEIGHT: u32 = 768;

pub struct SdlVideo {
    pub sdl: sdl2::Sdl,
    pub window: sdl2::video::Window,
    /// Kept alive for the lifetime of the GL context.
    _gl_ctx: sdl2::video::GLContext,
    pub gl: glow::Context,
    pub events: sdl2::EventPump,
}

impl SdlVideo {
    /// Create the window and a GLES2 context with vsync.
    ///
    /// `fullscreen` is true on the device; false gives a desktop dev window.
    pub fn new(title: &str, fullscreen: bool) -> Result<Self, String> {
        let sdl = sdl2::init()?;
        let video = sdl.video()?;

        let attr = video.gl_attr();
        attr.set_context_profile(sdl2::video::GLProfile::GLES);
        attr.set_context_version(2, 0);
        attr.set_double_buffer(true);

        let mut wb = video.window(title, DEVICE_WIDTH, DEVICE_HEIGHT);
        wb.opengl();
        if fullscreen {
            wb.fullscreen();
        }
        let window = wb.build().map_err(|e| e.to_string())?;

        let gl_ctx = window.gl_create_context()?;
        window.gl_make_current(&gl_ctx)?;
        // vsync — present() blocks on the panel refresh
        video.gl_set_swap_interval(sdl2::video::SwapInterval::VSync)?;

        let gl = unsafe {
            glow::Context::from_loader_function(|s| {
                video.gl_get_proc_address(s) as *const _
            })
        };

        let events = sdl.event_pump()?;

        Ok(Self { sdl, window, _gl_ctx: gl_ctx, gl, events })
    }

    pub fn drawable_size(&self) -> (u32, u32) {
        self.window.drawable_size()
    }

    pub fn present(&self) {
        self.window.gl_swap_window();
    }

    /// Deep sleep; see [`deep_sleep`]. Method form for exclusive owners.
    pub fn deep_sleep(&mut self, cfg: &kui_config::Config) {
        deep_sleep(&mut self.events, cfg);
    }

    /// GL renderer identity, for the M0 log (proves HW acceleration).
    pub fn renderer_info(&self) -> String {
        unsafe {
            format!(
                "{} | {} | GLSL {}",
                self.gl.get_parameter_string(glow::RENDERER),
                self.gl.get_parameter_string(glow::VERSION),
                self.gl.get_parameter_string(glow::SHADING_LANGUAGE_VERSION),
            )
        }
    }
}

/// Deep sleep with the device-proven preparation sequence: LED sleep
/// profile, helper daemons paused, sync, backlight off, settle, then
/// suspend-to-RAM with retries. Power button wakes; restores LEDs,
/// panel (brick raw-8 kick + brightness) and daemons on exit.
/// Free function over the event pump so callers can hold GL borrows.
pub fn deep_sleep(events: &mut sdl2::EventPump, cfg: &kui_config::Config) {
        use std::time::{Duration, Instant};
        run_sleep_hooks("pre-sleep.d");
        // Radio down before we freeze: the XR819 firmware dies on resume
        // and takes wpa_supplicant with it. See tg5040::wifi_suspend.
        tg5040::wifi_suspend(cfg);
        // Legacy helper daemons only; harmless no-ops once they are gone.
        // kuid needs no STOP/CONT: its key loop has a >1s tick-gap suspend
        // guard that discards stale events and held-key state on resume.
        let _ = std::process::Command::new("sh")
            .args(["-c", "killall -STOP keymon.elf batmon.elf audiomon.elf 2>/dev/null; sync"])
            .status();
        tg5040::leds::apply_profile(cfg, 4);
        tg5040::set_raw_brightness(0);

    std::thread::sleep(Duration::from_millis(300));
    for _ in events.poll_iter() {}

    'sleep: for _ in 0..5 {
        if tg5040::suspend_to_ram().is_ok() {
            break 'sleep;
        }
        let retry_at = Instant::now();
        while retry_at.elapsed() < Duration::from_secs(2) {
            for ev in events.poll_iter() {
                    if let Some(InputEvent::Button(Button::Power, ButtonState::Pressed)) =
                        tg5040::map_event(&ev)
                    {
                        break 'sleep;
                    }
                }
                std::thread::sleep(Duration::from_millis(200));
            }
        }

        tg5040::leds::apply_profile(cfg, 0);
        tg5040::set_raw_brightness(8);
        tg5040::set_raw_brightness(tg5040::brightness_raw(
            cfg.get_i32("display.brightness", 90),
        ));
        let _ = std::process::Command::new("sh")
            .args(["-c", "killall -CONT keymon.elf batmon.elf audiomon.elf 2>/dev/null"])
            .status();
        // Radio back up before the hooks run, so a pak that wants the
        // network on wake finds it coming up rather than dead.
        tg5040::wifi_resume(cfg);
    run_sleep_hooks("post-resume.d");
    for _ in events.poll_iter() {}
}

/// Pak contract: run every .sh in the hook dir (sorted, isolated, quiet).
fn run_sleep_hooks(dir: &str) {
    let base = format!("/mnt/SDCARD/.userdata/tg5040/.hooks/{dir}");
    let Ok(rd) = std::fs::read_dir(&base) else {
        return;
    };
    let mut scripts: Vec<_> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().map(|e| e == "sh").unwrap_or(false))
        .collect();
    scripts.sort();
    for s in scripts {
        let _ = std::process::Command::new("sh")
            .args(["-c", &format!("( . {} ) >/dev/null 2>&1", shell_word(&s))])
            .status();
    }
}

fn shell_word(p: &std::path::Path) -> String {
    format!("'{}'", p.display().to_string().replace('\'', "'\\''"))
}
