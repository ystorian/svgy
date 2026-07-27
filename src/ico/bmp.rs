//! 256-color BMP icon entries, for legacy ICO consumers.
//!
//! Windows picks the entry whose declared depth best matches the display, so a session capped at
//! 256 colors (old `mstsc`, or the "Limit maximum color depth" policy) wants an entry declaring 8
//! bits per pixel; PNG entries declare 32. Pre-Vista shells cannot read PNG entries at all, so
//! these also keep the two smallest sizes usable there.
//!
//! The layout is the classic one: a bottom-up 8bpp DIB whose height is doubled to cover the 1-bit
//! AND mask that follows the color data.

use resvg::tiny_skia;
use std::collections::HashMap;

/// A set of distinct colors and how many pixels each covers, as median cut
/// subdivides it.
type ColorBox = Vec<([u8; 3], u32)>;

const HEADER_LEN: u32 = 40;
/// A full table is written even when fewer colors are used: this entry exists for old readers, so
/// it is not the place to be clever about `biClrUsed`.
const PALETTE_LEN: usize = 256;
/// The mask is 1 bit, so alpha has to collapse to opaque or fully transparent.
const ALPHA_THRESHOLD: u8 = 128;

/// Encode a pixmap as the payload of an 8bpp BMP icon entry.
pub fn encode_bmp_8bpp(pixmap: &tiny_skia::Pixmap) -> Vec<u8> {
	let (w, h) = (pixmap.width() as usize, pixmap.height() as usize);
	let mut rgb: Vec<[u8; 3]> = Vec::with_capacity(w * h);
	let mut opaque: Vec<bool> = Vec::with_capacity(w * h);
	for px in pixmap.pixels() {
		let c = px.demultiply();
		rgb.push([c.red(), c.green(), c.blue()]);
		opaque.push(c.alpha() >= ALPHA_THRESHOLD);
	}

	let palette = quantize(&rgb, &opaque);
	let indices: Vec<u8> = rgb
		.iter()
		.zip(&opaque)
		.map(|(c, &o)| if o { nearest(&palette, *c) } else { 0 })
		.collect();

	// Both bitmaps are stored bottom-up with rows padded to 4 bytes.
	let color_stride = w.div_ceil(4) * 4;
	let mask_stride = w.div_ceil(8).div_ceil(4) * 4;
	let mut data = Vec::with_capacity(
		HEADER_LEN as usize + 4 * PALETTE_LEN + h * (color_stride + mask_stride),
	);

	// BITMAPINFOHEADER. The doubled height is what marks this as an icon DIB.
	let (w32, h32) = (
		i32::try_from(w).expect("icon width is at most 256"),
		i32::try_from(h).expect("icon height is at most 256"),
	);
	data.extend_from_slice(&HEADER_LEN.to_le_bytes());
	data.extend_from_slice(&w32.to_le_bytes());
	data.extend_from_slice(&(2 * h32).to_le_bytes());
	data.extend_from_slice(&1u16.to_le_bytes()); // planes
	data.extend_from_slice(&8u16.to_le_bytes()); // bits per pixel
	data.extend_from_slice(&0u32.to_le_bytes()); // compression: BI_RGB
	data.extend_from_slice(&0u32.to_le_bytes()); // image size, 0 for BI_RGB
	data.extend_from_slice(&0i32.to_le_bytes()); // horizontal pixels per meter
	data.extend_from_slice(&0i32.to_le_bytes()); // vertical pixels per meter
	data.extend_from_slice(&0u32.to_le_bytes()); // colors used: all of them
	data.extend_from_slice(&0u32.to_le_bytes()); // colors important: all of them

	// Color table, BGRA with a zero alpha byte.
	for c in &palette {
		data.extend_from_slice(&[c[2], c[1], c[0], 0]);
	}
	for _ in palette.len()..PALETTE_LEN {
		data.extend_from_slice(&[0, 0, 0, 0]);
	}

	// Color data: one byte per pixel, bottom row first.
	for y in (0..h).rev() {
		let row = &indices[y * w..(y + 1) * w];
		data.extend_from_slice(row);
		data.extend(std::iter::repeat_n(0u8, color_stride - w));
	}

	// AND mask: a set bit means transparent. Leftmost pixel is the high bit.
	for y in (0..h).rev() {
		let start = data.len();
		for chunk in opaque[y * w..(y + 1) * w].chunks(8) {
			let mut byte = 0u8;
			for (bit, &o) in chunk.iter().enumerate() {
				if !o {
					byte |= 0x80 >> bit;
				}
			}
			data.push(byte);
		}
		data.extend(std::iter::repeat_n(0u8, mask_stride - (data.len() - start)));
	}

	data
}

/// Build a palette of at most [`PALETTE_LEN`] colors covering the opaque pixels, exactly when they
/// are few enough and by median cut when they are not.
fn quantize(rgb: &[[u8; 3]], opaque: &[bool]) -> Vec<[u8; 3]> {
	let mut counts: HashMap<[u8; 3], u32> = HashMap::new();
	for (c, &o) in rgb.iter().zip(opaque) {
		if o {
			*counts.entry(*c).or_default() += 1;
		}
	}
	let mut colors: ColorBox = counts.into_iter().collect();
	// Sorted so the palette does not depend on hash iteration order.
	colors.sort_unstable();

	if colors.is_empty() {
		return vec![[0, 0, 0]];
	}
	if colors.len() <= PALETTE_LEN {
		return colors.into_iter().map(|(c, _)| c).collect();
	}

	let mut boxes = vec![colors];
	while boxes.len() < PALETTE_LEN {
		// Split whichever box still spans the widest range on any one channel.
		let Some(i) = boxes
			.iter()
			.enumerate()
			.filter(|(_, b)| b.len() > 1)
			.max_by_key(|(_, b)| widest_channel(b).1)
			.map(|(i, _)| i)
		else {
			break;
		};
		let (channel, _) = widest_channel(&boxes[i]);
		let (lo, hi) = split_at_median(boxes.swap_remove(i), channel);
		boxes.push(lo);
		boxes.push(hi);
	}
	boxes.iter().map(average).collect()
}

/// The channel with the largest spread in a box, and that spread.
fn widest_channel(b: &ColorBox) -> (usize, u8) {
	(0..3)
		.map(|ch| {
			let min = b.iter().map(|(c, _)| c[ch]).min().unwrap_or(0);
			let max = b.iter().map(|(c, _)| c[ch]).max().unwrap_or(0);
			(ch, max - min)
		})
		.max_by_key(|&(_, range)| range)
		.unwrap_or((0, 0))
}

/// Split a box at the point where half its pixels lie on either side.
fn split_at_median(mut b: ColorBox, channel: usize) -> (ColorBox, ColorBox) {
	b.sort_unstable_by_key(|(c, _)| c[channel]);
	let half = b.iter().map(|(_, n)| u64::from(*n)).sum::<u64>() / 2;
	let mut running = 0u64;
	let mut at = 0;
	for (i, (_, n)) in b.iter().enumerate() {
		running += u64::from(*n);
		if running > half {
			at = i;
			break;
		}
	}
	// Both halves must be non-empty or the loop above would never terminate.
	let at = at.clamp(1, b.len() - 1);
	let hi = b.split_off(at);
	(b, hi)
}

/// Pixel-count-weighted mean color of a box.
fn average(b: &ColorBox) -> [u8; 3] {
	let total: u64 = b.iter().map(|(_, n)| u64::from(*n)).sum();
	if total == 0 {
		return [0, 0, 0];
	}
	let mut out = [0u8; 3];
	for (ch, slot) in out.iter_mut().enumerate() {
		let sum: u64 = b
			.iter()
			.map(|(c, n)| u64::from(c[ch]) * u64::from(*n))
			.sum();
		*slot = u8::try_from(sum / total).expect("a mean of bytes is a byte");
	}
	out
}

/// Index of the palette entry closest to `c` by squared distance.
fn nearest(palette: &[[u8; 3]], c: [u8; 3]) -> u8 {
	let mut best = (0usize, i32::MAX);
	for (i, p) in palette.iter().enumerate() {
		let d: i32 = (0..3)
			.map(|ch| {
				let delta = i32::from(p[ch]) - i32::from(c[ch]);
				delta * delta
			})
			.sum();
		if d < best.1 {
			best = (i, d);
		}
	}
	u8::try_from(best.0).expect("palette holds at most 256 entries")
}
