//! Shared UI helpers: input repeat and the 9-slice rounded pill.

use std::time::{Duration, Instant};

use kui_gfx::{Renderer, Texture};

/// Hold-to-scroll repeat with hat-bounce debounce, device-tuned on the
/// Hammer: instant first step, 260ms delay, 85ms rate, 20ms bounce window
/// (80ms swallowed deliberate fast taps).
pub struct Repeat {
    pub held: [[bool; 2]; 3],
    dir: i32,
    prev_dir: i32,
    since: Instant,
    last: Instant,
    released_at: Instant,
}

impl Repeat {
    const DELAY: Duration = Duration::from_millis(260);
    const RATE: Duration = Duration::from_millis(85);
    const BOUNCE: Duration = Duration::from_millis(20);

    pub fn new() -> Self {
        let now = Instant::now();
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
    pub fn step(&mut self, now: Instant) -> i32 {
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

/// 9-slice rounded pill (fixed radius 20 per SPEC: fixed art geometry).
pub struct Pill {
    tex: Texture,
    radius: f32,
    tex_w: f32,
    tex_h: f32,
}

impl Pill {
    pub const RADIUS: u32 = 20;

    pub fn new(gl: &glow::Context, height: u32) -> Result<Self, String> {
        let r = Self::RADIUS.min(height / 2);
        let w = r * 2 + 2;
        let h = height;
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                // distance-based alpha with 1px AA edge, corners at radius r
                let fx = x as f32 + 0.5;
                let fy = y as f32 + 0.5;
                let cx = fx.clamp(r as f32, w as f32 - r as f32);
                let cy = fy.clamp(r as f32, h as f32 - r as f32);
                let d = ((fx - cx).powi(2) + (fy - cy).powi(2)).sqrt();
                let a = (r as f32 - d + 0.5).clamp(0.0, 1.0);
                let i = ((y * w + x) * 4) as usize;
                rgba[i..i + 4].copy_from_slice(&[255, 255, 255, (a * 255.0) as u8]);
            }
        }
        let tex = kui_gfx::texture_from_rgba(gl, w, h, &rgba)?;
        Ok(Self { tex, radius: r as f32, tex_w: w as f32, tex_h: h as f32 })
    }

    /// Draw at any width; height is the pill's native height.
    pub fn draw(&self, r: &Renderer, gl: &glow::Context, x: f32, y: f32, w: f32, tint: [f32; 4]) {
        let rad = self.radius;
        let u = rad / self.tex_w;
        let mid_u = 2.0 / self.tex_w;
        let h = self.tex_h;
        r.draw_uv(gl, &self.tex, x, y, rad, h, [0.0, 0.0, u, 1.0], tint);
        r.draw_uv(gl, &self.tex, x + rad, y, w - rad * 2.0, h, [u, 0.0, mid_u, 1.0], tint);
        r.draw_uv(gl, &self.tex, x + w - rad, y, rad, h, [u + mid_u, 0.0, u, 1.0], tint);
    }
}
