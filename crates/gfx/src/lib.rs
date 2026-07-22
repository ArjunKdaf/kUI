//! GLES2 renderer + UI kit for kUI.
//!
//! Immediate-mode textured quads: a unit-quad VBO, per-draw uniforms for
//! destination rect and tint. A handful of draws per frame — no batching
//! needed yet. No animation system (SPEC decision: no animation options).

use std::path::Path;

use glow::HasContext as _;

pub mod text;

const VERT: &str = r#"
attribute vec2 a_unit;
uniform vec4 u_dst;    // x, y, w, h in pixels (top-left origin)
uniform vec4 u_uv;     // sub-rect of the texture to sample
uniform vec2 u_screen; // screen size in pixels
varying vec2 v_uv;
void main() {
    v_uv = u_uv.xy + a_unit * u_uv.zw;
    vec2 px = u_dst.xy + a_unit * u_dst.zw;
    vec2 ndc = px / u_screen * 2.0 - 1.0;
    gl_Position = vec4(ndc.x, -ndc.y, 0.0, 1.0);
}
"#;

const FRAG: &str = r#"
precision mediump float;
uniform sampler2D u_tex;
uniform vec4 u_tint;
uniform highp vec2 u_texsize;
uniform highp float u_sharp;
varying vec2 v_uv;
void main() {
    highp vec2 uv = v_uv;
    if (u_sharp > 1.0) {
        // sharp-bilinear: crisp texels, blended only at block seams
        highp vec2 p = uv * u_texsize - 0.5;
        highp vec2 i = floor(p);
        highp vec2 f = clamp((p - i - 0.5) * u_sharp + 0.5, 0.0, 1.0);
        uv = (i + 0.5 + f) / u_texsize;
    }
    gl_FragColor = texture2D(u_tex, uv) * u_tint;
}
"#;

pub const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

#[derive(Clone, Copy)]
pub struct Texture {
    id: glow::Texture,
    pub w: u32,
    pub h: u32,
}

pub struct Renderer {
    program: glow::Program,
    quad_vbo: Option<glow::Buffer>,
    u_dst: glow::UniformLocation,
    u_uv: glow::UniformLocation,
    u_screen: glow::UniformLocation,
    u_tint: glow::UniformLocation,
    u_texsize: glow::UniformLocation,
    u_sharp: glow::UniformLocation,
    white: Texture,
    screen: (u32, u32),
}

impl Renderer {
    pub fn new(gl: &glow::Context) -> Result<Self, String> {
        let program = unsafe {
            let program = gl.create_program()?;
            for (kind, src) in [(glow::VERTEX_SHADER, VERT), (glow::FRAGMENT_SHADER, FRAG)] {
                let sh = gl.create_shader(kind)?;
                gl.shader_source(sh, src);
                gl.compile_shader(sh);
                if !gl.get_shader_compile_status(sh) {
                    return Err(gl.get_shader_info_log(sh));
                }
                gl.attach_shader(program, sh);
            }
            gl.link_program(program);
            if !gl.get_program_link_status(program) {
                return Err(gl.get_program_info_log(program));
            }
            program
        };

        let quad_vbo;
        unsafe {
            let vbo = gl.create_buffer()?;
            quad_vbo = Some(vbo);
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            let unit: [f32; 8] = [0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 1.0];
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, as_bytes(&unit), glow::STATIC_DRAW);
            gl.use_program(Some(program));
            let a_unit = gl
                .get_attrib_location(program, "a_unit")
                .ok_or("no a_unit")?;
            gl.vertex_attrib_pointer_f32(a_unit, 2, glow::FLOAT, false, 8, 0);
            gl.enable_vertex_attrib_array(a_unit);

            gl.enable(glow::BLEND);
            gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
        }

        let u_dst = unsafe { gl.get_uniform_location(program, "u_dst").ok_or("u_dst")? };
        let u_uv = unsafe { gl.get_uniform_location(program, "u_uv").ok_or("u_uv")? };
        let u_screen = unsafe { gl.get_uniform_location(program, "u_screen").ok_or("u_screen")? };
        let u_tint = unsafe { gl.get_uniform_location(program, "u_tint").ok_or("u_tint")? };
        let u_texsize =
            unsafe { gl.get_uniform_location(program, "u_texsize").ok_or("u_texsize")? };
        let u_sharp = unsafe { gl.get_uniform_location(program, "u_sharp").ok_or("u_sharp")? };

        let white = upload_rgba(gl, 1, 1, &[255, 255, 255, 255])?;

        Ok(Self {
            program,
            quad_vbo,
            u_dst,
            u_uv,
            u_screen,
            u_tint,
            u_texsize,
            u_sharp,
            white,
            screen: (0, 0),
        })
    }

    pub fn screen(&self) -> (u32, u32) {
        self.screen
    }

    /// Restore the quad VBO/attribute state after a foreign program drew
    /// (game shaders bind their own buffers and attributes).
    pub fn rebind_quad(&self, gl: &glow::Context) {
        unsafe {
            gl.use_program(Some(self.program));
            gl.bind_buffer(glow::ARRAY_BUFFER, self.quad_vbo);
            if let Some(a_unit) = gl.get_attrib_location(self.program, "a_unit") {
                gl.vertex_attrib_pointer_f32(a_unit, 2, glow::FLOAT, false, 8, 0);
                gl.enable_vertex_attrib_array(a_unit);
            }
        }
    }

    /// Sharp-bilinear factor for subsequent draws: `sharp` is the integer
    /// prescale (0 or 1 disables), `tw`/`th` the source texture size.
    pub fn set_sharp(&self, gl: &glow::Context, sharp: f32, tw: f32, th: f32) {
        unsafe {
            gl.use_program(Some(self.program));
            gl.uniform_1_f32(Some(&self.u_sharp), sharp);
            gl.uniform_2_f32(Some(&self.u_texsize), tw, th);
        }
    }

    pub fn begin_frame(&mut self, gl: &glow::Context, w: u32, h: u32, clear: [f32; 3]) {
        self.screen = (w, h);
        unsafe {
            gl.viewport(0, 0, w as i32, h as i32);
            gl.clear_color(clear[0], clear[1], clear[2], 1.0);
            gl.clear(glow::COLOR_BUFFER_BIT);
            gl.use_program(Some(self.program));
            gl.uniform_2_f32(Some(&self.u_screen), w as f32, h as f32);
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &self,
        gl: &glow::Context,
        tex: &Texture,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        tint: [f32; 4],
    ) {
        self.draw_uv(gl, tex, x, y, w, h, [0.0, 0.0, 1.0, 1.0], tint);
    }

    /// Draw a sub-rect of a texture (uv in 0..1 space): glyphs, 9-slices.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_uv(
        &self,
        gl: &glow::Context,
        tex: &Texture,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        uv: [f32; 4],
        tint: [f32; 4],
    ) {
        unsafe {
            gl.bind_texture(glow::TEXTURE_2D, Some(tex.id));
            gl.uniform_4_f32(Some(&self.u_dst), x, y, w, h);
            gl.uniform_4_f32(Some(&self.u_uv), uv[0], uv[1], uv[2], uv[3]);
            gl.uniform_4_f32(Some(&self.u_tint), tint[0], tint[1], tint[2], tint[3]);
            gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
        }
    }

    /// Solid-color rectangle.
    pub fn rect(&self, gl: &glow::Context, x: f32, y: f32, w: f32, h: f32, tint: [f32; 4]) {
        let white = self.white;
        self.draw(gl, &white, x, y, w, h, tint);
    }

    pub fn drop_texture(&self, gl: &glow::Context, tex: Texture) {
        unsafe { gl.delete_texture(tex.id) }
    }

    /// Clip subsequent draws to a rect (top-left origin coords).
    pub fn scissor(&self, gl: &glow::Context, x: f32, y: f32, w: f32, h: f32) {
        let (_, sh) = self.screen;
        unsafe {
            gl.enable(glow::SCISSOR_TEST);
            gl.scissor(
                x as i32,
                (sh as f32 - y - h) as i32,
                w as i32,
                h as i32,
            );
        }
    }

    pub fn scissor_off(&self, gl: &glow::Context) {
        unsafe { gl.disable(glow::SCISSOR_TEST) }
    }
}

/// Decode a PNG file and upload it as a GL texture (RGBA8 or RGB8 sources).
pub fn load_png(gl: &glow::Context, path: &Path) -> Result<Texture, String> {
    let (w, h, rgba) = decode_png(path)?;
    upload_rgba(gl, w, h, &rgba)
}

/// Upload raw RGBA8 pixels as a texture (used by async loaders).
pub fn texture_from_rgba(gl: &glow::Context, w: u32, h: u32, rgba: &[u8]) -> Result<Texture, String> {
    upload_rgba(gl, w, h, rgba)
}

/// Decode an in-memory PNG (embedded assets) to RGBA8.
pub fn decode_png_bytes(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    decode_from(png::Decoder::new(std::io::Cursor::new(bytes)), "<embedded>")
}

/// Thread-safe PNG decode to RGBA8 — no GL, callable from worker threads.
pub fn decode_png(path: &Path) -> Result<(u32, u32, Vec<u8>), String> {
    let file = std::fs::File::open(path).map_err(|e| format!("{path:?}: {e}"))?;
    decode_from(
        png::Decoder::new(std::io::BufReader::new(file)),
        &path.display().to_string(),
    )
}

fn decode_from<R: std::io::BufRead + std::io::Seek>(
    mut decoder: png::Decoder<R>,
    what: &str,
) -> Result<(u32, u32, Vec<u8>), String> {
    // Scraper boxart arrives as 8-bit palette PNGs; EXPAND normalizes
    // palette->RGB(A) and STRIP_16 folds 16-bit sources to 8-bit.
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|e| format!("{what}: {e}"))?;
    let mut buf = vec![0u8; reader.output_buffer_size().ok_or("png too large")?];
    let info = reader.next_frame(&mut buf).map_err(|e| format!("{what}: {e}"))?;
    buf.truncate(info.buffer_size());

    let rgba = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => {
            let mut out = Vec::with_capacity(buf.len() / 3 * 4);
            for px in buf.chunks_exact(3) {
                out.extend_from_slice(px);
                out.push(255);
            }
            out
        }
        png::ColorType::GrayscaleAlpha => {
            let mut out = Vec::with_capacity(buf.len() * 2);
            for px in buf.chunks_exact(2) {
                out.extend_from_slice(&[px[0], px[0], px[0], px[1]]);
            }
            out
        }
        png::ColorType::Grayscale => {
            let mut out = Vec::with_capacity(buf.len() * 4);
            for &g in &buf {
                out.extend_from_slice(&[g, g, g, 255]);
            }
            out
        }
        // Indexed cannot reach here: EXPAND converts it to Rgb/Rgba.
        other => return Err(format!("{what}: unsupported png color type {other:?}")),
    };
    Ok((info.width, info.height, rgba))
}

pub fn upload_sub_rgba(
    gl: &glow::Context,
    tex: &Texture,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    rgba: &[u8],
) {
    unsafe {
        gl.bind_texture(glow::TEXTURE_2D, Some(tex.id));
        gl.tex_sub_image_2d(
            glow::TEXTURE_2D,
            0,
            x as i32,
            y as i32,
            w as i32,
            h as i32,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(Some(rgba)),
        );
    }
}

pub(crate) fn upload_rgba_pub(gl: &glow::Context, w: u32, h: u32, rgba: &[u8]) -> Result<Texture, String> {
    upload_rgba(gl, w, h, rgba)
}

/// Make a texture tile (repeat wrap) — for patterned overlays.
pub fn set_texture_wrap_repeat(gl: &glow::Context, tex: &Texture) {
    unsafe {
        gl.bind_texture(glow::TEXTURE_2D, Some(tex.id));
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, glow::REPEAT as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, glow::REPEAT as i32);
    }
}

/// Switch a texture between crisp (nearest) and smooth (linear) sampling.
pub fn set_texture_filter(gl: &glow::Context, tex: &Texture, nearest: bool) {
    let f = if nearest { glow::NEAREST } else { glow::LINEAR } as i32;
    unsafe {
        gl.bind_texture(glow::TEXTURE_2D, Some(tex.id));
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, f);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, f);
    }
}

/// Decode a simple uncompressed 24/32bpp BMP (boot logos) to RGBA8.
pub fn decode_bmp(path: &Path) -> Result<(u32, u32, Vec<u8>), String> {
    let d = std::fs::read(path).map_err(|e| format!("{path:?}: {e}"))?;
    if d.len() < 54 || &d[0..2] != b"BM" {
        return Err(format!("{path:?}: not a BMP"));
    }
    let off = u32::from_le_bytes([d[10], d[11], d[12], d[13]]) as usize;
    let w = i32::from_le_bytes([d[18], d[19], d[20], d[21]]);
    let h_raw = i32::from_le_bytes([d[22], d[23], d[24], d[25]]);
    let bpp = u16::from_le_bytes([d[28], d[29]]) as usize;
    if bpp != 24 && bpp != 32 {
        return Err(format!("{path:?}: unsupported bpp {bpp}"));
    }
    let (w, h, flip) = (w as usize, h_raw.unsigned_abs() as usize, h_raw > 0);
    let stride = (w * bpp / 8).div_ceil(4) * 4;
    let mut out = vec![0u8; w * h * 4];
    for y in 0..h {
        let src_y = if flip { h - 1 - y } else { y };
        let row = off + src_y * stride;
        for x in 0..w {
            let p = row + x * bpp / 8;
            if p + bpp / 8 > d.len() {
                return Err(format!("{path:?}: truncated"));
            }
            let o = (y * w + x) * 4;
            out[o] = d[p + 2];
            out[o + 1] = d[p + 1];
            out[o + 2] = d[p];
            out[o + 3] = 255;
        }
    }
    Ok((w as u32, h as u32, out))
}

fn upload_rgba(gl: &glow::Context, w: u32, h: u32, rgba: &[u8]) -> Result<Texture, String> {
    unsafe {
        let id = gl.create_texture()?;
        gl.bind_texture(glow::TEXTURE_2D, Some(id));
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::LINEAR as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE as i32);
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::RGBA as i32,
            w as i32,
            h as i32,
            0,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(Some(rgba)),
        );
        Ok(Texture { id, w, h })
    }
}

/// Encode RGBA8 pixels as a PNG file (state previews).
pub fn encode_png(path: &Path, w: u32, h: u32, rgba: &[u8]) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut writer = enc.write_header().map_err(|e| e.to_string())?;
    writer.write_image_data(rgba).map_err(|e| e.to_string())
}

fn as_bytes(v: &[f32]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(v.as_ptr().cast(), std::mem::size_of_val(v)) }
}

// ---------------------------------------------------------------------------
// Game shaders: single-pass libretro GLSL format (#ifdef VERTEX/FRAGMENT,
// VertexCoord/TexCoord attributes, MVPMatrix/Texture/*Size uniforms).
// ---------------------------------------------------------------------------

pub struct GameShader {
    program: glow::Program,
    vbo: glow::Buffer,
    a_vertex: u32,
    a_tex: u32,
    u_mvp: Option<glow::UniformLocation>,
    u_tex: Option<glow::UniformLocation>,
    u_input: Option<glow::UniformLocation>,
    u_texsz: Option<glow::UniformLocation>,
    u_output: Option<glow::UniformLocation>,
    u_frame: Option<glow::UniformLocation>,
}

impl GameShader {
    pub fn load(gl: &glow::Context, source: &str) -> Result<GameShader, String> {
        let compile = |kind: u32, define: &str| -> Result<glow::Shader, String> {
            unsafe {
                let sh = gl.create_shader(kind)?;
                let src = format!(
                    "#define {define}\n#define PARAMETER_UNIFORM\nprecision mediump float;\n{source}"
                );
                gl.shader_source(sh, &src);
                gl.compile_shader(sh);
                if !gl.get_shader_compile_status(sh) {
                    return Err(gl.get_shader_info_log(sh));
                }
                Ok(sh)
            }
        };
        unsafe {
            let program = gl.create_program()?;
            gl.attach_shader(program, compile(glow::VERTEX_SHADER, "VERTEX")?);
            gl.attach_shader(program, compile(glow::FRAGMENT_SHADER, "FRAGMENT")?);
            gl.link_program(program);
            if !gl.get_program_link_status(program) {
                return Err(gl.get_program_info_log(program));
            }
            let vbo = gl.create_buffer()?;
            let a_vertex = gl
                .get_attrib_location(program, "VertexCoord")
                .ok_or("no VertexCoord attribute")?;
            let a_tex = gl.get_attrib_location(program, "TexCoord").ok_or("no TexCoord")?;
            Ok(GameShader {
                program,
                vbo,
                a_vertex,
                a_tex,
                u_mvp: gl.get_uniform_location(program, "MVPMatrix"),
                u_tex: gl.get_uniform_location(program, "Texture"),
                u_input: gl.get_uniform_location(program, "InputSize"),
                u_texsz: gl.get_uniform_location(program, "TextureSize"),
                u_output: gl.get_uniform_location(program, "OutputSize"),
                u_frame: gl.get_uniform_location(program, "FrameCount"),
            })
        }
    }

    /// Draw `tex` into the pixel rect (screen coords, top-left origin).
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &self,
        gl: &glow::Context,
        tex: &Texture,
        screen: (u32, u32),
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        frame: u32,
    ) {
        let (sw, sh) = (screen.0 as f32, screen.1 as f32);
        // pixel rect -> clip space (y flipped)
        let x0 = x / sw * 2.0 - 1.0;
        let x1 = (x + w) / sw * 2.0 - 1.0;
        let y0 = 1.0 - y / sh * 2.0;
        let y1 = 1.0 - (y + h) / sh * 2.0;
        let verts: [f32; 16] = [
            x0, y0, 0.0, 0.0, //
            x1, y0, 1.0, 0.0, //
            x0, y1, 0.0, 1.0, //
            x1, y1, 1.0, 1.0,
        ];
        unsafe {
            gl.use_program(Some(self.program));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, as_bytes(&verts), glow::STREAM_DRAW);
            gl.vertex_attrib_pointer_f32(self.a_vertex, 2, glow::FLOAT, false, 16, 0);
            gl.enable_vertex_attrib_array(self.a_vertex);
            gl.vertex_attrib_pointer_f32(self.a_tex, 2, glow::FLOAT, false, 16, 8);
            gl.enable_vertex_attrib_array(self.a_tex);
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(tex.id));
            if let Some(u) = &self.u_mvp {
                const IDENT: [f32; 16] = [
                    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0,
                    1.0,
                ];
                gl.uniform_matrix_4_f32_slice(Some(u), false, &IDENT);
            }
            if let Some(u) = &self.u_tex {
                gl.uniform_1_i32(Some(u), 0);
            }
            let (tw, th) = (tex.w as f32, tex.h as f32);
            if let Some(u) = &self.u_input {
                gl.uniform_2_f32(Some(u), tw, th);
            }
            if let Some(u) = &self.u_texsz {
                gl.uniform_2_f32(Some(u), tw, th);
            }
            if let Some(u) = &self.u_output {
                gl.uniform_2_f32(Some(u), w, h);
            }
            if let Some(u) = &self.u_frame {
                gl.uniform_1_i32(Some(u), frame as i32);
            }
            gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
        }
    }

    pub fn destroy(self, gl: &glow::Context) {
        unsafe {
            gl.delete_program(self.program);
            gl.delete_buffer(self.vbo);
        }
    }
}
