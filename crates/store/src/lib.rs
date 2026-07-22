//! kui-store: pak store (PakDek) and OTA updater engines for the kUI launcher.
//!
//! std only + miniz_oxide (deflate). HTTP is shelled out to curl, matching the
//! rest of the project. All downloads are atomic (tmp + rename).

pub mod json;
pub mod storefront;
pub mod updater;
pub mod zip;

mod http;

pub use json::Json;
pub use storefront::{Pak, PakKind, fetch_storefront, install_pak, installed_version, remove_pak};
pub use updater::{Release, apply_staged, download_and_stage, fetch_releases};
