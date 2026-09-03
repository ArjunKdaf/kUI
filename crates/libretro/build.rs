//! Builds the variadic log shim (csrc/log_shim.c) into a static archive
//! without the `cc` crate: the C compiler is invoked directly via
//! std::process::Command, mirroring crates/ra/build.rs.
//!
//! Compiler selection:
//! - TARGET == aarch64-unknown-linux-gnu -> the device cross-toolchain at
//!   <workspace>/toolchain/bin/aarch64-kui-linux-gnu-gcc (the same toolchain
//!   .cargo/config.toml already uses as the linker), with the matching -ar.
//! - anything else -> plain `cc` / `ar` from PATH, so desktop builds work.

use std::env;
use std::path::PathBuf;
use std::process::Command;

fn run(cmd: &mut Command, what: &str) {
    let status = cmd
        .status()
        .unwrap_or_else(|e| panic!("failed to spawn for {what}: {e}"));
    if !status.success() {
        panic!("{what} failed with {status}");
    }
}

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let target = env::var("TARGET").unwrap_or_default();

    let c_file = manifest_dir.join("csrc/log_shim.c");
    println!("cargo:rerun-if-changed={}", c_file.display());

    let (cc, ar) = if target == "aarch64-unknown-linux-gnu" {
        let tc_bin = manifest_dir
            .join("../../toolchain/bin")
            .canonicalize()
            .unwrap_or_else(|_| manifest_dir.join("../../toolchain/bin"));
        (
            tc_bin.join("aarch64-kui-linux-gnu-gcc"),
            tc_bin.join("aarch64-kui-linux-gnu-ar"),
        )
    } else {
        (PathBuf::from("cc"), PathBuf::from("ar"))
    };

    let obj = out_dir.join("log_shim.o");
    let mut cmd = Command::new(&cc);
    cmd.arg("-c")
        .arg(&c_file)
        .arg("-o")
        .arg(&obj)
        .arg("-O2")
        .arg("-fPIC")
        .arg("-Wall");
    run(&mut cmd, "compile log_shim.c");

    let archive = out_dir.join("libkui_logshim.a");
    // Remove a stale archive so `ar` doesn't append to it.
    let _ = std::fs::remove_file(&archive);
    let mut cmd = Command::new(&ar);
    cmd.arg("crs").arg(&archive).arg(&obj);
    run(&mut cmd, "archive libkui_logshim.a");

    println!("cargo:rustc-link-search=native={}", out_dir.display());
    println!("cargo:rustc-link-lib=static=kui_logshim");
}
