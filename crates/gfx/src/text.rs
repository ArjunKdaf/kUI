//! Text rendering: fontdue rasterizer + a dynamic glyph atlas.
//!
//! Glyphs rasterize on first use (any Unicode char — game names have
//! accents) into a shelf-packed RGBA atlas; drawing is atlas sub-rect
//! quads through the normal renderer. Colors come from the tint.

use std::collections::HashMap;

use crate::{Renderer, Texture};

const ATLAS_SIZE: u32 = 1024;

struct Glyph {
    /// uv sub-rect in the atlas (normalized).
    uv: [f32; 4],
    w: f32,
    h: f32,
    /// Offset from pen position to glyph top-left.
    xmin: f32,
    ymin: f32,
    advance: f32,
}

pub struct Font {
    bold: bool,
    font: fontdue::Font,
    atlas: Texture,
    glyphs: HashMap<(char, u32), Glyph>,
    // shelf packer cursor
    cur_x: u32,
    cur_y: u32,
    row_h: u32,
}

impl Font {
    pub fn load(gl: &glow::Context, path: &std::path::Path) -> Result<Self, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{path:?}: {e}"))?;
        let font = fontdue::Font::from_bytes(bytes, fontdue::FontSettings::default())
            .map_err(|e| format!("{path:?}: {e}"))?;
        let blank = vec![0u8; (ATLAS_SIZE * ATLAS_SIZE * 4) as usize];
        let atlas = crate::upload_rgba_pub(gl, ATLAS_SIZE, ATLAS_SIZE, &blank)?;
        Ok(Self {
            bold: false, font, atlas, glyphs: HashMap::new(), cur_x: 0, cur_y: 0, row_h: 0 })
    }

    fn glyph(&mut self, gl: &glow::Context, c: char, size: u32) -> &Glyph {
        if !self.glyphs.contains_key(&(c, size)) {
            let (m, coverage) = self.font.rasterize(c, size as f32);
            let (gw, gh) = (m.width as u32, m.height as u32);

            // shelf-pack into the atlas
            if self.cur_x + gw + 1 > ATLAS_SIZE {
                self.cur_x = 0;
                self.cur_y += self.row_h + 1;
                self.row_h = 0;
            }
            if self.cur_y + gh + 1 > ATLAS_SIZE {
                // atlas full: wrap around (glyphs from the very start of the
                // session may corrupt — acceptable until we see it happen)
                self.cur_y = 0;
                self.row_h = 0;
                eprintln!("glyph atlas wrapped");
            }
            let (ax, ay) = (self.cur_x, self.cur_y);
            self.cur_x += gw + 1;
            self.row_h = self.row_h.max(gh);

            if gw > 0 && gh > 0 {
                let mut rgba = Vec::with_capacity(coverage.len() * 4);
                for a in coverage {
                    rgba.extend_from_slice(&[255, 255, 255, a]);
                }
                crate::upload_sub_rgba(gl, &self.atlas, ax, ay, gw, gh, &rgba);
            }

            let s = ATLAS_SIZE as f32;
            self.glyphs.insert(
                (c, size),
                Glyph {
                    uv: [ax as f32 / s, ay as f32 / s, gw as f32 / s, gh as f32 / s],
                    w: gw as f32,
                    h: gh as f32,
                    xmin: m.xmin as f32,
                    ymin: m.ymin as f32,
                    advance: m.advance_width,
                },
            );
        }
        &self.glyphs[&(c, size)]
    }

    /// Visual line height (ascent + |descent|) at `size` px.
    pub fn line_height(&self, size: u32) -> f32 {
        self.font
            .horizontal_line_metrics(size as f32)
            .map(|m| m.ascent - m.descent)
            .unwrap_or(size as f32)
    }

    /// Width of `text` at `size` px without drawing it.
    pub fn measure(&mut self, gl: &glow::Context, text: &str, size: u32) -> f32 {
        text.chars().map(|c| self.glyph(gl, c, size).advance).sum()
    }

    /// Draw text with its baseline-ish anchored so `y` is the TOP of the line.
    /// Returns the pen x after the last glyph.
    #[allow(clippy::too_many_arguments)]
    pub fn set_bold(&mut self, on: bool) {
        self.bold = on;
    }

    pub fn draw(
        &mut self,
        r: &Renderer,
        gl: &glow::Context,
        text: &str,
        x: f32,
        y: f32,
        size: u32,
        color: [f32; 4],
    ) -> f32 {
        let ascent = {
            let lm = self.font.horizontal_line_metrics(size as f32);
            lm.map(|m| m.ascent).unwrap_or(size as f32 * 0.8)
        };
        let mut pen = x;
        for c in text.chars() {
            let g = self.glyph(gl, c, size);
            let (gw, gh, xmin, ymin, adv, uv) = (g.w, g.h, g.xmin, g.ymin, g.advance, g.uv);
            if gw > 0.0 {
                let gx = pen + xmin;
                let gy = y + ascent - gh - ymin;
                let atlas = self.atlas;
                r.draw_uv(gl, &atlas, gx, gy, gw, gh, uv, color);
                if self.bold {
                    // synthetic bold: the classic 1px double-strike
                    r.draw_uv(gl, &atlas, gx + 1.0, gy, gw, gh, uv, color);
                }
            }
            pen += adv;
        }
        pen
    }

    /// Truncate with an ellipsis to fit `max_w`. Returns owned string.
    pub fn fit(&mut self, gl: &glow::Context, text: &str, size: u32, max_w: f32) -> String {
        if self.measure(gl, text, size) <= max_w {
            return text.to_string();
        }
        let ell = '\u{2026}';
        let ell_w = self.glyph(gl, ell, size).advance;
        let mut out = String::new();
        let mut w = 0.0;
        for c in text.chars() {
            let a = self.glyph(gl, c, size).advance;
            if w + a + ell_w > max_w {
                break;
            }
            w += a;
            out.push(c);
        }
        out.push(ell);
        out
    }
}
