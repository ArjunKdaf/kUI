//! kui-power — the protected poweroff/reboot sequences (CONTRACT:
//! spec-services). The UI handles screen-off/mute before writing
//! /tmp/poweroff or /tmp/reboot; the session script then execs us.
//!
//! Power-off must not cut power with the SD card dirty: kill the card's
//! users, unmount it, then hard-off via the AXP2202 PMIC so no stock
//! code runs on the way down.

use std::ffi::CString;
use std::fs;
use std::thread::sleep;
use std::time::Duration;

const SDCARD: &str = "/mnt/SDCARD";
const SHARED_DIR: &str = "/mnt/SDCARD/.userdata/shared";
const I2C_DEV: &str = "/dev/i2c-6";
const AXP2202_ADDR: libc::c_ulong = 0x34;
const I2C_SLAVE: libc::c_ulong = 0x0703;

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_default();
    match mode.as_str() {
        "off" => poweroff(),
        "reboot" => reboot(),
        "sleep" => sleep_to_ram(),
        _ => {
            eprintln!("usage: kui-power off|reboot|sleep");
            std::process::exit(2);
        }
    }
}

/// Suspend-to-RAM and block until the power button wakes us. Compat
/// entry for paks that called the legacy `suspend` binary — a bare
/// `echo mem` freezes this hardware, so the backlight-off + sync + retry
/// prep matters. The launcher/frontend deep-sleep path is richer (it
/// drives the LEDs and event pump); this is the standalone minimum.
fn sleep_to_ram() {
    let cfg = kui_config::Config::load(std::path::Path::new(SHARED_DIR));
    // backlight off (DISP_LCD_SET_BRIGHTNESS raw 0) so a black screen
    // greets the sleeper, then flush before the kernel freezes us
    kui_hal::tg5040::set_raw_brightness(0);
    sync();
    sleep(Duration::from_millis(300));
    for _ in 0..5 {
        if kui_hal::tg5040::suspend_to_ram().is_ok() {
            break; // returns on wake
        }
        sleep(Duration::from_secs(2));
    }
    // wake: kick the panel and restore the stored brightness
    kui_hal::tg5040::set_raw_brightness(8);
    let pct = cfg.get_i32("display.brightness", 90);
    kui_hal::tg5040::set_raw_brightness(kui_hal::tg5040::brightness_raw(pct));
}

fn pids() -> Vec<i32> {
    fs::read_dir("/proc")
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| e.file_name().to_string_lossy().parse::<i32>().ok())
                .collect()
        })
        .unwrap_or_default()
}

/// Does this process hold an open fd under the SD card?
fn uses_sdcard(pid: i32) -> bool {
    let fd_dir = format!("/proc/{pid}/fd");
    fs::read_dir(fd_dir)
        .map(|rd| {
            rd.flatten().any(|e| {
                fs::read_link(e.path())
                    .map(|t| t.starts_with(SDCARD))
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

fn kill(pid: i32, sig: libc::c_int) {
    unsafe {
        libc::kill(pid, sig);
    }
}

fn kill_sdcard_users(sig: libc::c_int) {
    let me = std::process::id() as i32;
    for pid in pids() {
        if pid > 1 && pid != me && uses_sdcard(pid) {
            kill(pid, sig);
        }
    }
}

fn kill_everything(sig: libc::c_int) {
    let me = std::process::id() as i32;
    for pid in pids() {
        if pid > 1 && pid != me {
            kill(pid, sig);
        }
    }
}

fn sync() {
    unsafe { libc::sync() }
}

fn swapoff_all() {
    let Ok(text) = fs::read_to_string("/proc/swaps") else { return };
    for line in text.lines().skip(1) {
        if let Some(dev) = line.split_whitespace().next()
            && let Ok(c) = CString::new(dev)
        {
            unsafe {
                libc::swapoff(c.as_ptr());
            }
        }
    }
}

fn umount(path: &str, flags: libc::c_int) -> bool {
    let Ok(c) = CString::new(path) else { return false };
    unsafe { libc::umount2(c.as_ptr(), flags) == 0 }
}

fn mounted(path: &str) -> bool {
    fs::read_to_string("/proc/mounts")
        .map(|t| t.lines().any(|l| l.split_whitespace().nth(1) == Some(path)))
        .unwrap_or(false)
}

/// AXP2202 hard-off: mask every IRQ so nothing wakes us mid-sequence,
/// then the soft-off dance (reg 0x22 <- 0x0A, then 0x27 <- 0x01).
fn pmic_off() {
    let Ok(f) = fs::OpenOptions::new().read(true).write(true).open(I2C_DEV) else {
        return;
    };
    use std::os::unix::io::AsRawFd;
    let fd = f.as_raw_fd();
    if unsafe { libc::ioctl(fd, I2C_SLAVE, AXP2202_ADDR) } != 0 {
        return;
    }
    let wr = |reg: u8, val: u8| {
        let buf = [reg, val];
        unsafe { libc::write(fd, buf.as_ptr() as *const libc::c_void, 2) };
    };
    for reg in 0x40..=0x44u8 {
        wr(reg, 0x00);
    }
    for reg in 0x48..=0x4Cu8 {
        wr(reg, 0xFF);
    }
    wr(0x22, 0x0A);
    sleep(Duration::from_millis(50));
    wr(0x27, 0x01);
    sleep(Duration::from_secs(1));
}

fn poweroff() {
    // the card's users die first so the unmount below can succeed
    kill_sdcard_users(libc::SIGKILL);
    sync();
    swapoff_all();
    // stock bind-mounts a profile over /etc/profile; drop it
    umount("/etc/profile", libc::MNT_FORCE);
    for _ in 0..3 {
        if !mounted(SDCARD) {
            break;
        }
        if umount(SDCARD, libc::MNT_FORCE | libc::MNT_DETACH) {
            break;
        }
        sleep(Duration::from_millis(800));
        kill_sdcard_users(libc::SIGKILL);
        sync();
    }
    kill_everything(libc::SIGTERM);
    sleep(Duration::from_secs(2));
    kill_everything(libc::SIGKILL);
    sync();
    sleep(Duration::from_millis(500));
    pmic_off();
    // PMIC should have cut power already; belt and suspenders
    unsafe { libc::reboot(libc::RB_POWER_OFF) };
    let _ = std::process::Command::new("poweroff").status();
    loop {
        sleep(Duration::from_secs(1));
    }
}

fn reboot() {
    kill_everything(libc::SIGTERM);
    sleep(Duration::from_millis(500));
    kill_everything(libc::SIGKILL);
    sync();
    swapoff_all();
    umount("/etc/profile", libc::MNT_FORCE);
    umount(SDCARD, libc::MNT_DETACH);
    sync();
    unsafe { libc::reboot(libc::RB_AUTOBOOT) };
    let _ = std::process::Command::new("reboot").status();
    loop {
        sleep(Duration::from_secs(1));
    }
}
