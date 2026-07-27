//! PNG optimization via oxipng.

use anyhow::{Context, Result};

/// Above this many pixels per side, Zopfli's runtime stops being worth the
/// handful of bytes it saves and preset 6's libdeflater takes over.
const ZOPFLI_MAX_PX: u32 = 512;

/// Optimize PNG bytes with oxipng when `enabled`, otherwise return them as-is.
///
/// Icons are small and written once, so we spend the time: preset 6 tries every
/// filter, and up to [`ZOPFLI_MAX_PX`] Zopfli re-deflates the result better than
/// libdeflater. `px` is the side length of the image the PNG was rendered at.
pub fn maybe_optimize(png: Vec<u8>, enabled: bool, px: u32) -> Result<Vec<u8>> {
    if !enabled {
        return Ok(png);
    }
    // Preset 6 already deflates with libdeflater at its highest level, so the
    // large-image path is simply the preset left alone.
    let mut opts = oxipng::Options::from_preset(6);
    if px <= ZOPFLI_MAX_PX {
        opts.deflater = oxipng::Deflater::Zopfli(oxipng::ZopfliOptions::default());
    }
    oxipng::optimize_from_memory(&png, &opts).context("oxipng optimization")
}
