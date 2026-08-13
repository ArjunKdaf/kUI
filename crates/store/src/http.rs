//! HTTP via curl, matching the rest of the project: `-sfL --max-time 120`,
//! the SD-card CA bundle when present (else `-k`), atomic downloads.

use std::path::{Path, PathBuf};
use std::process::Command;

const CA_BUNDLE: &str = "/mnt/SDCARD/.system/res/cacert.pem";

fn curl_bin() -> &'static str {
    if Path::new("/usr/bin/curl").exists() {
        "/usr/bin/curl"
    } else {
        "curl"
    }
}

fn base_cmd(headers: &[&str]) -> Command {
    let mut cmd = Command::new(curl_bin());
    cmd.arg("-sfL").arg("--max-time").arg("120");
    if Path::new(CA_BUNDLE).is_file() {
        cmd.arg("--cacert").arg(CA_BUNDLE);
    } else {
        cmd.arg("-k");
    }
    for h in headers {
        cmd.arg("-H").arg(h);
    }
    cmd
}

/// Fetch a URL and return the body as text.
pub(crate) fn fetch_text(url: &str, headers: &[&str]) -> Result<String, String> {
    let out = base_cmd(headers)
        .arg(url)
        .output()
        .map_err(|e| format!("failed to run curl: {e}"))?;
    if !out.status.success() {
        return Err(format!("curl failed ({}) for {url}", out.status));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Download a URL to `dest` atomically (curl to `<dest>.part`, then rename).
/// No wall-clock timeout — ports run to gigabytes — only stall detection:
/// abort when under 1 KiB/s for 30s.
pub(crate) fn download(url: &str, dest: &Path, headers: &[&str]) -> Result<(), String> {
    let status = download_cmd(url, dest, headers)
        .status()
        .map_err(|e| format!("failed to run curl: {e}"))?;
    let tmp = PathBuf::from(format!("{}.part", dest.display()));
    if !status.success() {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("download failed ({status}) for {url}"));
    }
    std::fs::rename(&tmp, dest)
        .map_err(|e| format!("rename {} -> {}: {e}", tmp.display(), dest.display()))
}

/// The curl invocation `download` runs, for callers that want to spawn it
/// and watch `<dest>.part` grow (progress display).
pub(crate) fn download_cmd(url: &str, dest: &Path, headers: &[&str]) -> Command {
    let tmp = format!("{}.part", dest.display());
    let mut cmd = Command::new(curl_bin());
    cmd.arg("-sfL").arg("--speed-limit").arg("1024").arg("--speed-time").arg("30");
    if Path::new(CA_BUNDLE).is_file() {
        cmd.arg("--cacert").arg(CA_BUNDLE);
    } else {
        cmd.arg("-k");
    }
    for h in headers {
        cmd.arg("-H").arg(h);
    }
    cmd.arg("-o").arg(tmp).arg(url);
    cmd
}
