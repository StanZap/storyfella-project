//! Embeds the app icon into the Windows executable.
//!
//! The icon is resource 1: Explorer shows the lowest-numbered icon for an
//! `.exe`, and GPUI loads resource 1 for its window icon. The `.ico` wraps the
//! packaging PNG unchanged (PNG entries are valid from Windows Vista on), so the
//! PNG stays the only source. Other targets build nothing here.

use std::path::PathBuf;
use std::{env, fs};

const ICON: &str = "packaging/studio.png";

fn main() {
    println!("cargo:rerun-if-changed={ICON}");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let png = fs::read(ICON).unwrap_or_else(|error| panic!("reading {ICON}: {error}"));
    let ico = ico_from_png(&png).unwrap_or_else(|error| panic!("{ICON}: {error}"));
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    let ico_path = out.join("app.ico");
    fs::write(&ico_path, ico).expect("writing app.ico");
    let rc_path = out.join("app.rc");
    let quoted = ico_path.display().to_string().replace('\\', "\\\\");
    fs::write(&rc_path, format!("1 ICON \"{quoted}\"\n")).expect("writing app.rc");
    embed_resource::compile(&rc_path, embed_resource::NONE)
        .manifest_optional()
        .expect("compiling the icon resource");
}

/// A single-image `.ico` holding `png` as-is.
fn ico_from_png(png: &[u8]) -> Result<Vec<u8>, String> {
    const SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
    if png.len() < 24 || !png.starts_with(SIGNATURE) || &png[12..16] != b"IHDR" {
        return Err("not a PNG".into());
    }
    let width = u32::from_be_bytes(png[16..20].try_into().expect("four bytes"));
    let height = u32::from_be_bytes(png[20..24].try_into().expect("four bytes"));
    if width != height || !(1..=256).contains(&width) {
        return Err(format!(
            "an icon must be square and at most 256px, not {width}x{height}"
        ));
    }
    let size = u32::try_from(png.len()).map_err(|_| "the PNG is too large".to_string())?;
    // 256 is stored as 0 in the one-byte dimension fields.
    let dimension = (width % 256) as u8;
    let mut ico = Vec::with_capacity(22 + png.len());
    ico.extend_from_slice(&[0, 0, 1, 0, 1, 0]);
    ico.extend_from_slice(&[dimension, dimension, 0, 0]);
    ico.extend_from_slice(&1u16.to_le_bytes());
    ico.extend_from_slice(&32u16.to_le_bytes());
    ico.extend_from_slice(&size.to_le_bytes());
    ico.extend_from_slice(&22u32.to_le_bytes());
    ico.extend_from_slice(png);
    Ok(ico)
}
