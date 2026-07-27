//! Windows .ico writer (multi-size).
//!
//! The container is written by hand rather than through the `ico` crate: that
//! crate only encodes from raw RGBA and picks BMP or its own deflate per entry,
//! so there is no way to hand it PNG bytes we already ran through oxipng, nor to
//! ask it for a 256-color entry. The format is simple enough that writing it
//! ourselves is the shorter path.

mod bmp;

use anyhow::{Context, Result};
use resvg::usvg;
use std::io::Write;
use std::path::Path;

use crate::{optimize, render};

/// Icon (not cursor) resource.
const RES_TYPE_ICON: u16 = 1;
/// Bytes per ICONDIRENTRY.
const ENTRY_LEN: usize = 16;

/// The standard "complete" set, fixed on purpose: callers do not choose sizes.
/// 256 is the largest an `.ico` entry can describe (a width byte of 0).
pub const SIZES: &[u32] = &[16, 32, 48, 64, 128, 256];

/// Sizes that additionally get a 256-color BMP entry. Only the two smallest are
/// worth it: a 256-color session is by definition an old one, where these are
/// the sizes actually drawn, and the color table costs a flat 1 KiB per entry.
const LEGACY_SIZES: &[u32] = &[16, 32];

/// One image in the file, already encoded.
struct Entry {
    px: u32,
    /// Declared bits per pixel, which is what Windows matches against the
    /// display depth when choosing between two entries of the same size.
    bits: u16,
    data: Vec<u8>,
}

/// Render every standard size and assemble an `.ico` file. With `legacy`, the
/// sizes in [`LEGACY_SIZES`] get a second, 256-color entry. Returns the number
/// of entries written.
pub fn write_ico(
    tree: &usvg::Tree,
    optimize_png: bool,
    legacy: bool,
    out: &Path,
) -> Result<usize> {
    let mut entries: Vec<Entry> = Vec::with_capacity(SIZES.len() + LEGACY_SIZES.len());
    for &px in SIZES {
        let pixmap = render::render_size(tree, px, px)?;
        let raw = render::encode_png(&pixmap)?;
        // The PNG often ends up as an 8-bit palette, but it is a full-color
        // entry as far as the directory is concerned: it keeps its alpha.
        entries.push(Entry {
            px,
            bits: 32,
            data: optimize::maybe_optimize(raw, optimize_png, px)?,
        });
        // Listed after the PNG of the same size, so a reader that grabs the
        // first entry it can use still gets the good one.
        if legacy && LEGACY_SIZES.contains(&px) {
            entries.push(Entry { px, bits: 8, data: bmp::encode_bmp_8bpp(&pixmap) });
        }
    }

    let mut buf: Vec<u8> = Vec::new();

    // ICONDIR: reserved, resource type, image count.
    buf.extend_from_slice(&0u16.to_le_bytes());
    buf.extend_from_slice(&RES_TYPE_ICON.to_le_bytes());
    let count = u16::try_from(entries.len()).expect("entry count is a handful");
    buf.extend_from_slice(&count.to_le_bytes());

    // Image data follows the directory, in the same order as the entries.
    let mut offset =
        u32::try_from(6 + ENTRY_LEN * entries.len()).expect("directory is a few hundred bytes");
    for e in &entries {
        // A dimension of 256 is stored as 0; anything larger cannot be encoded.
        let dim = if e.px == 256 {
            0u8
        } else {
            u8::try_from(e.px).expect("ICO sizes are at most 256")
        };
        buf.push(dim); // width
        buf.push(dim); // height
        buf.push(0); // palette size: 0 means 256, or "not a palette entry"
        buf.push(0); // reserved
        buf.extend_from_slice(&1u16.to_le_bytes()); // color planes
        // For PNG entries the depth is advisory -- readers take the real one
        // from the IHDR -- but it is what entry selection compares against.
        buf.extend_from_slice(&e.bits.to_le_bytes());
        let len = u32::try_from(e.data.len()).context("ICO entry exceeds 4 GiB")?;
        buf.extend_from_slice(&len.to_le_bytes());
        buf.extend_from_slice(&offset.to_le_bytes());
        offset += len;
    }

    for e in &entries {
        buf.extend_from_slice(&e.data);
    }

    let mut file = std::fs::File::create(out)
        .with_context(|| format!("creating {}", out.display()))?;
    file.write_all(&buf)
        .with_context(|| format!("writing {}", out.display()))?;
    Ok(entries.len())
}
