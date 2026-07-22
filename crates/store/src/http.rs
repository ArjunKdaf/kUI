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
pub(crate) fn download(url: &str, dest: &Path, headers: &[&str]) -> Result<(), String> {
    let tmp = PathBuf::from(format!("{}.part", dest.display()));
    let status = base_cmd(headers)
        .arg("-o")
        .arg(&tmp)
        .arg(url)
        .status()
        .map_err(|e| format!("failed to run curl: {e}"))?;
    if !status.success() {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("download failed ({status}) for {url}"));
    }
    std::fs::rename(&tmp, dest)
        .map_err(|e| format!("rename {} -> {}: {e}", tmp.display(), dest.display()))
}
