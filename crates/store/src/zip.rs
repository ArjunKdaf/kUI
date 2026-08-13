//! Zip extraction (full tree) with miniz_oxide — the device's only unzip.
//!
//! Central-directory walk (EOCD scan), stored + deflate methods, directory
//! entries, path traversal protection, atomic per-file writes. Unix modes
//! from external attributes are preserved (launch.sh must stay executable).

use std::fs;
use std::path::{Path, PathBuf};

const EOCD_SIG: [u8; 4] = [0x50, 0x4B, 0x05, 0x06];
const CDIR_SIG: [u8; 4] = [0x50, 0x4B, 0x01, 0x02];
const LOCAL_SIG: [u8; 4] = [0x50, 0x4B, 0x03, 0x04];

struct Entry {
    name: String,
    method: u16,
    comp_size: u64,
    local_off: u64,
    unix_mode: u32, // 0 = unset
    is_dir: bool,
}

/// Extract every entry of `zip_path` into `dest` (created if missing).
pub fn unzip(zip_path: &Path, dest: &Path) -> Result<(), String> {
    extract(zip_path, dest, false)
}

/// Like [`unzip`], but if all entries share a single top-level directory
/// (`kUI-<ver>/...`), that component is stripped.
pub fn unzip_strip_toplevel(zip_path: &Path, dest: &Path) -> Result<(), String> {
    extract(zip_path, dest, true)
}

fn extract(zip_path: &Path, dest: &Path, strip_top: bool) -> Result<(), String> {
    let data = fs::read(zip_path).map_err(|e| format!("read {}: {e}", zip_path.display()))?;
    let entries = parse_central_directory(&data)?;
    let prefix = if strip_top { common_toplevel(&entries) } else { None };

    fs::create_dir_all(dest).map_err(|e| format!("mkdir {}: {e}", dest.display()))?;

    for e in &entries {
        let mut name = e.name.trim_start_matches('/');
        if let Some(pre) = &prefix {
            name = match name.strip_prefix(pre.as_str()) {
                Some(rest) => rest,
                None => continue, // the top-level dir entry itself
            };
        }
        if name.is_empty() {
            continue;
        }
        let rel = sanitize(name)?;
        let target = dest.join(&rel);
        if e.is_dir {
            fs::create_dir_all(&target).map_err(|e| format!("mkdir {}: {e}", target.display()))?;
            continue;
        }
        let bytes = entry_data(&data, e)?;
        let mode = if e.unix_mode & 0o777 != 0 {
            e.unix_mode & 0o7777
        } else if name.ends_with(".sh") {
            0o755 // zips built without unix attrs: keep scripts runnable
        } else {
            0
        };
        write_atomic(&target, &bytes, mode)?;
    }
    Ok(())
}

fn parse_central_directory(data: &[u8]) -> Result<Vec<Entry>, String> {
    if data.len() < 22 {
        return Err("not a zip: too small".into());
    }
    // EOCD scan: the record is 22 bytes plus an up-to-64KiB comment.
    let scan_floor = data.len().saturating_sub(22 + 65535);
    let mut eocd = None;
    for i in (scan_floor..=data.len() - 22).rev() {
        if data[i..i + 4] == EOCD_SIG {
            eocd = Some(i);
            break;
        }
    }
    let e = eocd.ok_or("not a zip: end-of-central-directory record not found")?;
    let total = rd_u16(data, e + 10) as usize;
    let cd_off = rd_u32(data, e + 16) as usize;

    let mut entries = Vec::with_capacity(total);
    let mut p = cd_off;
    for n in 0..total {
        if p + 46 > data.len() || data[p..p + 4] != CDIR_SIG {
            return Err(format!("bad central directory entry #{n}"));
        }
        let version_made_by = rd_u16(data, p + 4);
        let method = rd_u16(data, p + 10);
        let comp_size = rd_u32(data, p + 20) as u64;
        let name_len = rd_u16(data, p + 28) as usize;
        let extra_len = rd_u16(data, p + 30) as usize;
        let comment_len = rd_u16(data, p + 32) as usize;
        let external = rd_u32(data, p + 38);
        let local_off = rd_u32(data, p + 42) as u64;
        if p + 46 + name_len > data.len() {
            return Err(format!("truncated name in central directory entry #{n}"));
        }
        let name = String::from_utf8_lossy(&data[p + 46..p + 46 + name_len]).replace('\\', "/");
        // High byte of version-made-by 3 = UNIX; mode lives in the high 16 bits.
        let unix_mode = if version_made_by >> 8 == 3 { external >> 16 } else { 0 };
        let is_dir = name.ends_with('/');
        entries.push(Entry { name, method, comp_size, local_off, unix_mode, is_dir });
        p += 46 + name_len + extra_len + comment_len;
    }
    Ok(entries)
}

fn entry_data(data: &[u8], e: &Entry) -> Result<Vec<u8>, String> {
    let lo = e.local_off as usize;
    if lo + 30 > data.len() || data[lo..lo + 4] != LOCAL_SIG {
        return Err(format!("bad local header for {}", e.name));
    }
    let name_len = rd_u16(data, lo + 26) as usize;
    let extra_len = rd_u16(data, lo + 28) as usize;
    let start = lo + 30 + name_len + extra_len;
    let end = start
        .checked_add(e.comp_size as usize)
        .ok_or_else(|| format!("overflow in {}", e.name))?;
    if end > data.len() {
        return Err(format!("truncated data for {}", e.name));
    }
    let comp = &data[start..end];
    match e.method {
        0 => Ok(comp.to_vec()), // stored
        8 => miniz_oxide::inflate::decompress_to_vec(comp)
            .map_err(|err| format!("deflate error in {}: {:?}", e.name, err.status)),
        m => Err(format!("unsupported compression method {m} in {}", e.name)),
    }
}

/// Reject path traversal; build a safe relative path. ".." only counts
/// as a whole component — filenames like "xmaskye jr..xml" (real, in the
/// xye port) are legitimate.
fn sanitize(name: &str) -> Result<PathBuf, String> {
    let mut out = PathBuf::new();
    for comp in name.split('/') {
        if comp.is_empty() || comp == "." {
            continue;
        }
        if comp == ".." || comp.contains('\\') {
            return Err(format!("zip entry rejected (path traversal): {name}"));
        }
        out.push(comp);
    }
    if out.as_os_str().is_empty() {
        return Err(format!("zip entry rejected (empty path): {name}"));
    }
    Ok(out)
}

/// If every entry lives under one shared top-level directory, return that
/// prefix including its trailing '/'.
fn common_toplevel(entries: &[Entry]) -> Option<String> {
    let mut top: Option<&str> = None;
    for e in entries {
        let n = e.name.trim_start_matches('/');
        if n.is_empty() {
            continue;
        }
        let first = match n.split_once('/') {
            Some((first, _)) => first,
            None => return None, // a file at the root: nothing to strip
        };
        match top {
            None => top = Some(first),
            Some(t) if t == first => {}
            Some(_) => return None,
        }
    }
    top.map(|t| format!("{t}/"))
}

fn write_atomic(dest: &Path, bytes: &[u8], mode: u32) -> Result<(), String> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    }
    let tmp = PathBuf::from(format!("{}.kui-part", dest.display()));
    fs::write(&tmp, bytes).map_err(|e| format!("write {}: {e}", tmp.display()))?;
    #[cfg(unix)]
    if mode != 0 {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&tmp, fs::Permissions::from_mode(mode))
            .map_err(|e| format!("chmod {}: {e}", tmp.display()))?;
    }
    fs::rename(&tmp, dest)
        .map_err(|e| format!("rename {} -> {}: {e}", tmp.display(), dest.display()))
}

fn rd_u16(b: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([b[off], b[off + 1]])
}

fn rd_u32(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "kui-store-test-{}-{}-{tag}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Build a zip in bytes by hand. `entries`: (name, data, method) where
    /// data is already in stored/deflated form per `method`; `uncomp` is the
    /// uncompressed size to record.
    fn build_zip(entries: &[(&str, Vec<u8>, u16, u32)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut central = Vec::new();
        for (name, data, method, uncomp) in entries {
            let local_off = out.len() as u32;
            // local file header
            out.extend_from_slice(&LOCAL_SIG);
            out.extend_from_slice(&20u16.to_le_bytes()); // version needed
            out.extend_from_slice(&0u16.to_le_bytes()); // flags
            out.extend_from_slice(&method.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes()); // mod time
            out.extend_from_slice(&0u16.to_le_bytes()); // mod date
            out.extend_from_slice(&0u32.to_le_bytes()); // crc32 (unchecked)
            out.extend_from_slice(&(data.len() as u32).to_le_bytes()); // comp size
            out.extend_from_slice(&uncomp.to_le_bytes()); // uncomp size
            out.extend_from_slice(&(name.len() as u16).to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes()); // extra len
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(data);
            // central directory entry
            central.extend_from_slice(&CDIR_SIG);
            central.extend_from_slice(&(3u16 << 8 | 20).to_le_bytes()); // made by: unix
            central.extend_from_slice(&20u16.to_le_bytes()); // version needed
            central.extend_from_slice(&0u16.to_le_bytes()); // flags
            central.extend_from_slice(&method.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes()); // time
            central.extend_from_slice(&0u16.to_le_bytes()); // date
            central.extend_from_slice(&0u32.to_le_bytes()); // crc32
            central.extend_from_slice(&(data.len() as u32).to_le_bytes());
            central.extend_from_slice(&uncomp.to_le_bytes());
            central.extend_from_slice(&(name.len() as u16).to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes()); // extra len
            central.extend_from_slice(&0u16.to_le_bytes()); // comment len
            central.extend_from_slice(&0u16.to_le_bytes()); // disk start
            central.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
            let mode: u32 = if name.ends_with(".sh") { 0o755 } else { 0o644 };
            central.extend_from_slice(&(mode << 16).to_le_bytes()); // external attrs
            central.extend_from_slice(&local_off.to_le_bytes());
            central.extend_from_slice(name.as_bytes());
        }
        let cd_off = out.len() as u32;
        let cd_size = central.len() as u32;
        out.extend_from_slice(&central);
        out.extend_from_slice(&EOCD_SIG);
        out.extend_from_slice(&0u16.to_le_bytes()); // disk
        out.extend_from_slice(&0u16.to_le_bytes()); // cd disk
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        out.extend_from_slice(&cd_size.to_le_bytes());
        out.extend_from_slice(&cd_off.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // comment len
        out
    }

    fn stored<'a>(name: &'a str, data: &[u8]) -> (&'a str, Vec<u8>, u16, u32) {
        (name, data.to_vec(), 0, data.len() as u32)
    }

    #[test]
    fn stored_tree() {
        let dir = scratch("stored");
        let zip = build_zip(&[
            stored("root.txt", b"hello root"),
            stored("sub/", b""),
            stored("sub/inner.txt", b"nested"),
            stored("sub/launch.sh", b"#!/bin/sh\necho hi\n"),
        ]);
        let zip_path = dir.join("t.zip");
        fs::write(&zip_path, &zip).unwrap();
        let out = dir.join("out");
        unzip(&zip_path, &out).unwrap();
        assert_eq!(fs::read(out.join("root.txt")).unwrap(), b"hello root");
        assert_eq!(fs::read(out.join("sub/inner.txt")).unwrap(), b"nested");
        assert!(out.join("sub").is_dir());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(out.join("sub/launch.sh")).unwrap().permissions().mode();
            assert_eq!(mode & 0o111, 0o111, "launch.sh must be executable");
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn deflate_roundtrip() {
        let dir = scratch("deflate");
        let payload = b"kui kui kui kui kui kui kui kui kui kui deflate me".repeat(20);
        let comp = miniz_oxide::deflate::compress_to_vec(&payload, 6);
        assert!(comp.len() < payload.len());
        let zip = build_zip(&[("data.bin", comp, 8, payload.len() as u32)]);
        let zip_path = dir.join("t.zip");
        fs::write(&zip_path, &zip).unwrap();
        let out = dir.join("out");
        unzip(&zip_path, &out).unwrap();
        assert_eq!(fs::read(out.join("data.bin")).unwrap(), payload);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_path_traversal() {
        let dir = scratch("traversal");
        let zip = build_zip(&[stored("../evil.txt", b"nope")]);
        let zip_path = dir.join("t.zip");
        fs::write(&zip_path, &zip).unwrap();
        let err = unzip(&zip_path, &dir.join("out")).unwrap_err();
        assert!(err.contains("path traversal"), "got: {err}");
        assert!(!dir.join("evil.txt").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn dotdot_inside_filename_is_fine() {
        // real case: the xye port ships "res/xmaskye jr..xml"
        let dir = scratch("dotdotname");
        let zip = build_zip(&[
            stored("Xye.sh", b"#!"),
            stored("xye/res/xmaskye jr..xml", b"ok"),
        ]);
        let zip_path = dir.join("t.zip");
        fs::write(&zip_path, &zip).unwrap();
        let out = dir.join("out");
        unzip(&zip_path, &out).unwrap();
        assert_eq!(fs::read(out.join("xye/res/xmaskye jr..xml")).unwrap(), b"ok");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn strips_single_toplevel_dir() {
        let dir = scratch("strip");
        let zip = build_zip(&[
            stored("kUI-1.2.3/", b""),
            stored("kUI-1.2.3/a.txt", b"aaa"),
            stored("kUI-1.2.3/sub/b.txt", b"bbb"),
        ]);
        let zip_path = dir.join("t.zip");
        fs::write(&zip_path, &zip).unwrap();
        let out = dir.join("out");
        unzip_strip_toplevel(&zip_path, &out).unwrap();
        assert_eq!(fs::read(out.join("a.txt")).unwrap(), b"aaa");
        assert_eq!(fs::read(out.join("sub/b.txt")).unwrap(), b"bbb");
        assert!(!out.join("kUI-1.2.3").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn no_strip_with_mixed_roots() {
        let dir = scratch("nostrip");
        let zip = build_zip(&[
            stored("top/a.txt", b"aaa"),
            stored("readme.txt", b"root file"),
        ]);
        let zip_path = dir.join("t.zip");
        fs::write(&zip_path, &zip).unwrap();
        let out = dir.join("out");
        unzip_strip_toplevel(&zip_path, &out).unwrap();
        assert_eq!(fs::read(out.join("top/a.txt")).unwrap(), b"aaa");
        assert_eq!(fs::read(out.join("readme.txt")).unwrap(), b"root file");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn not_a_zip() {
        let dir = scratch("notzip");
        let zip_path = dir.join("t.zip");
        fs::write(&zip_path, b"this is definitely not a zip archive, sorry").unwrap();
        assert!(unzip(&zip_path, &dir.join("out")).is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}
