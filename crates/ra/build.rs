//! Builds the vendored rcheevos C library (vendor/rcheevos, MIT licensed)
//! into a static archive without the `cc` crate: the C compiler is invoked
//! directly via std::process::Command.
//!
//! Compiler selection:
//! - TARGET == aarch64-unknown-linux-gnu -> the device cross-toolchain at
//!   <workspace>/toolchain/bin/aarch64-kui-linux-gnu-gcc (same toolchain
//!   .cargo/config.toml already uses as the linker), matching `-ar` for the
//!   archive step.
//! - anything else -> plain `cc` / `ar` from PATH.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

/// rcheevos translation units needed for the modern rc_client integration:
/// rc_client itself, the rc_api layer it talks to the server with, the
/// rcheevos trigger/runtime core, and rc_hash (with its bundled md5/aes)
/// for game identification.  Paths are relative to vendor/rcheevos/src.
///
/// Deliberately excluded: rc_client_external.c / rc_client_raintegration.c
/// (RAIntegration DLL support, Windows-only), rc_libretro.c (libretro memory
/// mapping helpers, not needed yet), rcheevos/rc_validate.c (dev validator).
const SOURCES: &[&str] = &[
    "rc_client.c",
    "rc_compat.c",
    "rc_util.c",
    "rc_version.c",
    "rapi/rc_api_common.c",
    "rapi/rc_api_editor.c",
    "rapi/rc_api_info.c",
    "rapi/rc_api_runtime.c",
    "rapi/rc_api_user.c",
    "rcheevos/alloc.c",
    "rcheevos/condition.c",
    "rcheevos/condset.c",
    "rcheevos/consoleinfo.c",
    "rcheevos/format.c",
    "rcheevos/lboard.c",
    "rcheevos/memref.c",
    "rcheevos/operand.c",
    "rcheevos/richpresence.c",
    "rcheevos/runtime.c",
    "rcheevos/runtime_progress.c",
    "rcheevos/trigger.c",
    "rcheevos/value.c",
    "rhash/aes.c",
    "rhash/cdreader.c",
    "rhash/hash.c",
    "rhash/hash_disc.c",
    "rhash/hash_encrypted.c",
    "rhash/hash_rom.c",
    "rhash/hash_zip.c",
    "rhash/md5.c",
];

fn run(cmd: &mut Command, what: &str) {
    let status = cmd
        .status()
        .unwrap_or_else(|e| panic!("failed to spawn {what} ({:?}): {e}", cmd.get_program()));
    if !status.success() {
        panic!("{what} failed with {status}: {cmd:?}");
    }
}

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let target = env::var("TARGET").unwrap_or_default();

    let vendor = manifest_dir.join("vendor/rcheevos");
    let src = vendor.join("src");
    let include = vendor.join("include");

    println!("cargo:rerun-if-changed={}", vendor.display());

    // Pick compiler + archiver.
    let (cc, ar) = if target == "aarch64-unknown-linux-gnu" {
        // Workspace-root toolchain (crates/ra/../../toolchain), with the
        // known absolute location as a fallback.
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

    let mut objects: Vec<PathBuf> = Vec::new();
    for rel in SOURCES {
        let c_file = src.join(rel);
        if !c_file.exists() {
            panic!("vendored source missing: {}", c_file.display());
        }
        // Flatten "rapi/rc_api_user.c" -> "rapi_rc_api_user.o" to avoid
        // collisions between subdirectories.
        let obj = out_dir.join(format!("{}.o", rel.replace(['/', '\\'], "_")));
        let mut cmd = Command::new(&cc);
        cmd.arg("-c")
            .arg(&c_file)
            .arg("-o")
            .arg(&obj)
            .arg("-O2")
            .arg("-fPIC")
            .arg("-fvisibility=hidden")
            .arg("-DRC_DISABLE_LUA")
            .arg("-DRC_CLIENT_SUPPORTS_HASH")
            .arg("-I")
            .arg(&include)
            .arg("-I")
            .arg(&src);
        run(&mut cmd, &format!("compile {rel}"));
        objects.push(obj);
    }

    let archive = out_dir.join("libkui_rcheevos.a");
    // Remove a stale archive so `ar` doesn't append to it.
    let _ = std::fs::remove_file(&archive);
    let mut cmd = Command::new(&ar);
    cmd.arg("crs").arg(&archive);
    for obj in &objects {
        cmd.arg(obj);
    }
    run(&mut cmd, "archive libkui_rcheevos.a");

    let _ = Path::new(&archive); // built above
    println!("cargo:rustc-link-search=native={}", out_dir.display());
    println!("cargo:rustc-link-lib=static=kui_rcheevos");
}
