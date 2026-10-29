// src/icns/mod.rs

//! Apple Icon Image Format (.icns) writer.
//!
//! Ported from sveinbjornt/createicns: 16x16 and 32x32 icons are stored as `ic04`/`ic05` (ARGB with
//! per-channel `PackBits` RLE); every larger size embeds its PNG bytes verbatim. Because we render
//! each size ourselves, the small icons get ARGB directly from the tiny-skia pixmap (no PNG decode
//! needed).

mod packbits;

use anyhow::{Context as _, Result};
use packbits::rle_encode_channel;
use resvg::{tiny_skia, usvg};
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::fs::File;
use std::io::Write as _;
use std::path::Path;

use crate::{optimize, render};

/// File signature at the start of an icns file.
const MAGIC: &[u8; 4] = b"icns";
/// Prefix of an ARGB-RLE chunk payload.
const ARGB_MAGIC: &[u8; 4] = b"ARGB";

/// One chunk of the icns file: `OSType`, the pixel side length to render, and whether it is stored
/// as ARGB-RLE (small icons) or embedded PNG.
struct IconSpec {
	/// Four-byte chunk type.
	ostype: &'static [u8; 4],
	/// Side length in pixels.
	px: u32,
	/// Store as ARGB-RLE, else as PNG.
	argb: bool,
}

/// The standard "complete" set, mirroring createicns' `kIconTypes` plus the non-retina ARGB
/// entries. Ordering does not matter to icns readers.
const SPECS: &[IconSpec] = &[
	IconSpec {
		ostype: b"ic04",
		px: 16,
		argb: true,
	}, // 16x16
	IconSpec {
		ostype: b"ic05",
		px: 32,
		argb: true,
	}, // 32x32
	IconSpec {
		ostype: b"ic11",
		px: 32,
		argb: false,
	}, // 16x16@2x
	IconSpec {
		ostype: b"icp6",
		px: 48,
		argb: false,
	}, // 48x48
	IconSpec {
		ostype: b"ic12",
		px: 64,
		argb: false,
	}, // 32x32@2x
	IconSpec {
		ostype: b"ic07",
		px: 128,
		argb: false,
	}, // 128x128
	IconSpec {
		ostype: b"ic13",
		px: 256,
		argb: false,
	}, // 128x128@2x
	IconSpec {
		ostype: b"ic08",
		px: 256,
		argb: false,
	}, // 256x256
	IconSpec {
		ostype: b"ic14",
		px: 512,
		argb: false,
	}, // 256x256@2x
	IconSpec {
		ostype: b"ic09",
		px: 512,
		argb: false,
	}, // 512x512
	IconSpec {
		ostype: b"ic10",
		px: 1024,
		argb: false,
	}, // 512x512@2x
];

/// Render every standard size and assemble a `.icns` file.
pub fn write_icns(tree: &usvg::Tree, opt: optimize::Opts, out: &Path) -> Result<()> {
	// Eleven OSTypes cover eight distinct sizes, so both the render and (where the format matches)
	// the encode are shared. 32 x 32 is rendered once but encoded twice: `ic05` wants ARGB-RLE,
	// `ic11` wants PNG.
	let mut pixmap_cache: HashMap<u32, tiny_skia::Pixmap> = HashMap::new();
	let mut png_cache: HashMap<u32, Vec<u8>> = HashMap::new();
	let mut chunks: Vec<u8> = Vec::new();

	for spec in SPECS {
		let pixmap = match pixmap_cache.entry(spec.px) {
			Entry::Occupied(entry) => entry.into_mut(),
			Entry::Vacant(entry) => entry.insert(render::render_size(tree, spec.px, spec.px)?),
		};
		if spec.argb {
			let data = encode_argb_chunk(pixmap);
			write_chunk(&mut chunks, *spec.ostype, &data);
		} else {
			// Sizes that appear under more than one OSType are encoded once.
			let png = if let Some(cached) = png_cache.get(&spec.px) {
				cached.clone()
			} else {
				let raw = render::encode_png(pixmap)?;
				let optimized = optimize::maybe_optimize(raw, opt)?;
				png_cache.insert(spec.px, optimized.clone());
				optimized
			};
			write_chunk(&mut chunks, *spec.ostype, &png);
		}
	}

	// File = "icns" magic + total size (incl. these 8 bytes) + chunks.
	let total = u32::try_from(8 + chunks.len()).context("icns file exceeds 4 GiB")?;
	let mut file = File::create(out).with_context(|| format!("creating {}", out.display()))?;
	file.write_all(MAGIC)?;
	file.write_all(&total.to_be_bytes())?;
	file.write_all(&chunks)?;
	return Ok(());
}

/// Append `OSType + big-endian (len + 8) + data` to the chunk buffer.
fn write_chunk(buf: &mut Vec<u8>, ostype: [u8; 4], data: &[u8]) {
	let size = u32::try_from(8 + data.len()).expect("icns chunk exceeds 4 GiB");
	buf.extend_from_slice(&ostype);
	buf.extend_from_slice(&size.to_be_bytes());
	buf.extend_from_slice(data);
}

/// Build the payload of an ARGB chunk: `'ARGB'` magic followed by the A, R, G, B channels, each
/// PackBits-RLE encoded. Alpha is straight (non-premultiplied).
fn encode_argb_chunk(pixmap: &tiny_skia::Pixmap) -> Vec<u8> {
	let count = (pixmap.width() * pixmap.height()) as usize;
	let (mut alpha, mut red, mut green, mut blue) = (
		Vec::with_capacity(count),
		Vec::with_capacity(count),
		Vec::with_capacity(count),
		Vec::with_capacity(count),
	);
	for px in pixmap.pixels() {
		let color = px.demultiply();
		alpha.push(color.alpha());
		red.push(color.red());
		green.push(color.green());
		blue.push(color.blue());
	}

	let mut data = Vec::new();
	data.extend_from_slice(ARGB_MAGIC);
	data.extend_from_slice(&rle_encode_channel(&alpha));
	data.extend_from_slice(&rle_encode_channel(&red));
	data.extend_from_slice(&rle_encode_channel(&green));
	data.extend_from_slice(&rle_encode_channel(&blue));
	return data;
}
