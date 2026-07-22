//! M0 proof: GLES2 triangle + raw input echo on the Hammer.
//!
//! Prints every SDL event (the point: discover the device's real button
//! mapping for the tg5040 input table). Quit: Esc, controller Start+Select
//! held together, window close, or the K key.
//!
//! Run on desktop:  cargo run -p kui-hal --example hello
//! Run on device:   cross-build, push, run with the device's SDL2 on
//!                  LD_LIBRARY_PATH (see scripts/push-hello.sh).

use std::time::Instant;

use glow::HasContext as _;
use kui_hal::sdl::SdlVideo;
use sdl2::event::Event;

const VERT: &str = r#"
attribute vec2 pos;
attribute vec3 color;
varying vec3 v_color;
void main() {
    v_color = color;
    gl_Position = vec4(pos, 0.0, 1.0);
}
"#;

const FRAG: &str = r#"
precision mediump float;
varying vec3 v_color;
void main() {
    gl_FragColor = vec4(v_color, 1.0);
}
"#;

fn main() -> Result<(), String> {
    let fullscreen = std::env::var("DEVICE").is_ok();
    let mut v = SdlVideo::new("kUI M0", fullscreen)?;
    println!("renderer: {}", v.renderer_info());
    println!("drawable: {:?}", v.drawable_size());

    // Open every joystick so the device's pad reports events.
    let joy = v.sdl.joystick()?;
    let mut sticks = Vec::new();
    for i in 0..joy.num_joysticks().unwrap_or(0) {
        if let Ok(j) = joy.open(i) {
            println!("joystick {i}: {}", j.name());
            sticks.push(j);
        }
    }

    let gl = &v.gl;
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

    // interleaved x,y,r,g,b — kUI-green top vertex
    #[rustfmt::skip]
    let verts: [f32; 15] = [
         0.0,  0.6,  0.0, 1.0, 0.33,
        -0.6, -0.5,  1.0, 1.0, 1.0,
         0.6, -0.5,  0.0, 0.4, 0.2,
    ];
    unsafe {
        let vbo = gl.create_buffer()?;
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
        gl.buffer_data_u8_slice(
            glow::ARRAY_BUFFER,
            bytemuck_cast(&verts),
            glow::STATIC_DRAW,
        );
        gl.use_program(Some(program));
        let pos = gl.get_attrib_location(program, "pos").ok_or("no pos")?;
        let color = gl.get_attrib_location(program, "color").ok_or("no color")?;
        gl.vertex_attrib_pointer_f32(pos, 2, glow::FLOAT, false, 20, 0);
        gl.vertex_attrib_pointer_f32(color, 3, glow::FLOAT, false, 20, 8);
        gl.enable_vertex_attrib_array(pos);
        gl.enable_vertex_attrib_array(color);
    }

    let start = Instant::now();
    let mut frames = 0u32;
    let mut last_fps = Instant::now();
    let (mut held_start, mut held_select) = (false, false);

    'run: loop {
        for ev in v.events.poll_iter() {
            match &ev {
                Event::Quit { .. } => break 'run,
                Event::KeyDown { keycode: Some(k), .. } => {
                    println!("keydown: {k:?}");
                    use sdl2::keyboard::Keycode;
                    match *k {
                        Keycode::Escape | Keycode::K => break 'run,
                        Keycode::Return => held_start = true,
                        Keycode::RCtrl => held_select = true,
                        _ => {}
                    }
                }
                Event::KeyUp { keycode: Some(k), .. } => {
                    use sdl2::keyboard::Keycode;
                    match *k {
                        Keycode::Return => held_start = false,
                        Keycode::RCtrl => held_select = false,
                        _ => {}
                    }
                }
                Event::JoyButtonDown { button_idx, .. } => {
                    println!("joybutton down: {button_idx}");
                    // TrimUI: discover real indices via this echo.
                    match button_idx {
                        9 => held_start = true,
                        10 => held_select = true,
                        _ => {}
                    }
                }
                Event::JoyButtonUp { button_idx, .. } => match button_idx {
                    9 => held_start = false,
                    10 => held_select = false,
                    _ => {}
                },
                Event::JoyAxisMotion { axis_idx, value, .. } => {
                    if value.unsigned_abs() > 16000 {
                        println!("joyaxis {axis_idx}: {value}");
                    }
                }
                Event::JoyHatMotion { hat_idx, state, .. } => {
                    println!("joyhat {hat_idx}: {state:?}");
                }
                _ => {}
            }
            if held_start && held_select {
                break 'run;
            }
        }

        let t = start.elapsed().as_secs_f32();
        let (w, h) = v.drawable_size();
        unsafe {
            gl.viewport(0, 0, w as i32, h as i32);
            // slow background pulse so vsync pacing is visible on camera
            let p = (t * 0.5).sin() * 0.5 + 0.5;
            gl.clear_color(0.02, 0.05 + 0.08 * p, 0.03, 1.0);
            gl.clear(glow::COLOR_BUFFER_BIT);
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
        }
        v.present();

        frames += 1;
        if last_fps.elapsed().as_secs() >= 5 {
            println!("fps: {:.1}", frames as f32 / last_fps.elapsed().as_secs_f32());
            frames = 0;
            last_fps = Instant::now();
        }
    }

    println!("clean exit after {:.1}s", start.elapsed().as_secs_f32());
    Ok(())
}

/// f32 slice → bytes without a bytemuck dependency.
fn bytemuck_cast(v: &[f32]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(v.as_ptr().cast(), std::mem::size_of_val(v)) }
}
