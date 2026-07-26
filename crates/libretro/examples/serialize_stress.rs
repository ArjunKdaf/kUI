//! Headless save-state stress harness (PicoDrive/Asterix freeze hunt,
//! 2026-07-26). Boots a core+rom with no video/audio device, then:
//!   Phase A — serialize repeatedly while running; detect the emulated
//!             machine wedging (framebuffer static despite input).
//!   Phase B — serialize once while healthy, keep running, then load the
//!             snapshot back; detect machine reset / wedge after load.
//!
//! Usage: serialize_stress <core.so> <rom> <out_dir> [saves] [state]
//! With [state]: skips the stress phases and instead loads the given
//! state file after warmup (foreign-state compatibility check).
//! Exit codes: 0 = no defect observed, 2 = wedge in phase A,
//!             3 = bad restore in phase B, 4 = both, 5 = foreign state
//!             load failed/dead.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use kui_libretro as lr;

fn fb_hash(core: &lr::Core, last: &mut u64) -> u64 {
    if let Some((_, _, px)) = core.take_video() {
        let mut h = DefaultHasher::new();
        px.hash(&mut h);
        *last = h.finish();
    }
    *last
}

/// Run n frames with light input mashing (Start pulses + d-pad wiggles,
/// so title screens advance and attract modes stay busy); return the
/// number of distinct framebuffer hashes seen.
fn run_frames(core: &lr::Core, n: usize, mash: bool, last: &mut u64) -> usize {
    let mut seen = std::collections::HashSet::new();
    for i in 0..n {
        let pad = if !mash {
            0
        } else {
            match (i / 30) % 4 {
                0 => 1 << 3,           // START
                1 => 1 << 8,           // A/button1
                2 => 1 << 6,           // right
                _ => 0,
            }
        };
        core.set_pad(pad);
        core.run_frame();
        let _ = core.take_audio();
        seen.insert(fb_hash(core, last));
    }
    seen.len()
}

fn dump_ppm(core: &lr::Core, last: &mut u64, path: &Path) {
    // force one more frame so a fresh buffer is available
    core.run_frame();
    let _ = core.take_audio();
    if let Some((w, h, px)) = core.take_video() {
        let mut out = format!("P6\n{w} {h}\n255\n").into_bytes();
        for c in px.chunks(4) {
            out.extend_from_slice(&c[..3]);
        }
        let _ = std::fs::write(path, out);
    } else {
        let mut hh = DefaultHasher::new();
        last.hash(&mut hh);
        let _ = std::fs::write(path, b"no frame available");
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 {
        eprintln!("usage: {} <core.so> <rom> <out_dir> [saves]", args[0]);
        std::process::exit(1);
    }
    let out = PathBuf::from(&args[3]);
    let _ = std::fs::create_dir_all(&out);
    let saves: usize = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(30);

    let core = lr::Core::load(
        Path::new(&args[1]),
        Path::new(&args[2]),
        &out.join("system"),
        &out.join("saves"),
        Vec::<(String, String)>::new(),
    )
    .expect("core load");
    println!("core: {} | {}x{} @{:.2}fps", core.name, core.av_info.geometry.base_width,
        core.av_info.geometry.base_height, core.av_info.timing.fps);

    let mut last = 0u64;
    let mut failures = 0;

    // boot + get into motion
    let distinct = run_frames(&core, 900, true, &mut last);
    println!("warmup: {distinct} distinct frames over 900");
    dump_ppm(&core, &mut last, &out.join("warmup.ppm"));

    // foreign-state mode: load the provided state, judge, exit
    if let Some(state_path) = args.get(5) {
        let bytes = std::fs::read(state_path).expect("read state");
        let loaded = core.load_state(&bytes);
        let d1 = run_frames(&core, 30, false, &mut last);
        dump_ppm(&core, &mut last, &out.join("foreign-load-30f.ppm"));
        let d2 = run_frames(&core, 300, true, &mut last);
        dump_ppm(&core, &mut last, &out.join("foreign-load-330f.ppm"));
        println!("FOREIGN STATE: load_ok={loaded} distinct_30f={d1} distinct_300f={d2}");
        std::process::exit(if loaded && d2 > 1 { 0 } else { 5 });
    }

    // ---- Phase A: repeated serialize while running -----------------
    let mut wedged_at = None;
    for i in 1..=saves {
        let snap = core.save_state();
        let ok = snap.is_some();
        let distinct = run_frames(&core, 120, true, &mut last);
        println!("save #{i}: serialize_ok={ok} distinct_frames_after={distinct}");
        if distinct <= 1 {
            // one more chance: some scenes idle briefly
            let retry = run_frames(&core, 240, true, &mut last);
            if retry <= 1 {
                println!("PHASE A: machine wedged after save #{i}");
                dump_ppm(&core, &mut last, &out.join(format!("wedged-after-{i}.ppm")));
                if let Some(s) = &snap {
                    let _ = std::fs::write(out.join(format!("wedged-{i}.state")), s);
                }
                wedged_at = Some(i);
                break;
            }
        }
    }
    if wedged_at.is_some() {
        failures |= 2;
    } else {
        println!("PHASE A: no wedge after {saves} saves");
    }

    // ---- Phase B: save healthy, run, load back ---------------------
    core.reset();
    let _ = run_frames(&core, 900, true, &mut last);
    let snap = core.save_state().expect("phase B serialize");
    let _ = std::fs::write(out.join("healthy.state"), &snap);
    dump_ppm(&core, &mut last, &out.join("healthy-at-save.ppm"));
    let _ = run_frames(&core, 300, true, &mut last);
    let loaded = core.load_state(&snap);
    // give it a beat, then judge
    let d1 = run_frames(&core, 30, false, &mut last);
    dump_ppm(&core, &mut last, &out.join("after-load-30f.ppm"));
    let d2 = run_frames(&core, 300, true, &mut last);
    dump_ppm(&core, &mut last, &out.join("after-load-330f.ppm"));
    println!(
        "PHASE B: load_ok={loaded} distinct_30f={d1} distinct_300f={d2} \
         (compare after-load-30f.ppm vs healthy-at-save.ppm: mismatch = reset/corrupt)"
    );
    if !loaded || d2 <= 1 {
        println!("PHASE B: restore failed or machine dead after load");
        failures |= 4;
    }

    std::process::exit(if failures == 0 { 0 } else { failures.min(4) });
}
