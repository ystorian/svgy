// src/ico/bmp.rs

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
use std::iter::repeat_n;

/// A set of distinct colors and how many pixels each covers, as median cut
/// subdivides it.
type ColorBox = Vec<([u8; 3], u32)>;

/// Size of the `BITMAPINFOHEADER` in bytes.
const HEADER_LEN: u32 = 40;
/// A full table is written even when fewer colors are used: this entry exists for old readers, so
/// it is not the place to be clever about `biClrUsed`.
const PALETTE_LEN: usize = 256;
/// The mask is 1 bit, so alpha has to collapse to opaque or fully transparent.
const ALPHA_THRESHOLD: u8 = 128;

/// Encode a pixmap as the payload of an 8bpp BMP icon entry.
pub fn encode_bmp_8bpp(pixmap: &tiny_skia::Pixmap) -> Vec<u8> {
	let (w, height) = (pixmap.width() as usize, pixmap.height() as usize);
	let mut rgb: Vec<[u8; 3]> = Vec::with_capacity(w * height);
	let mut opaque: Vec<bool> = Vec::with_capacity(w * height);
	for px in pixmap.pixels() {
		let color = px.demultiply();
		rgb.push([color.red(), color.green(), color.blue()]);
		opaque.push(color.alpha() >= ALPHA_THRESHOLD);
	}

	let palette = quantize(&rgb, &opaque);
	let indices: Vec<u8> = rgb
		.iter()
		.zip(&opaque)
		.map(|(color, &is_opaque)| {
			if is_opaque {
				return nearest(&palette, *color);
			}
			return 0;
		})
		.collect();

	// Both bitmaps are stored bottom-up with rows padded to 4 bytes.
	let color_stride = w.div_ceil(4) * 4;
	let mask_stride = w.div_ceil(8).div_ceil(4) * 4;
	let mut data = Vec::with_capacity(
		HEADER_LEN as usize + 4 * PALETTE_LEN + height * (color_stride + mask_stride),
	);

	// BITMAPINFOHEADER. The doubled height is what marks this as an icon DIB.
	let (w32, h32) = (
		i32::try_from(w).expect("icon width is at most 256"),
		i32::try_from(height).expect("icon height is at most 256"),
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
	for color in &palette {
		data.extend_from_slice(&[color[2], color[1], color[0], 0]);
	}
	for _ in palette.len()..PALETTE_LEN {
		data.extend_from_slice(&[0, 0, 0, 0]);
	}

	// Color data: one byte per pixel, bottom row first.
	for y in (0..height).rev() {
		let row = &indices[y * w..(y + 1) * w];
		data.extend_from_slice(row);
		data.extend(repeat_n(0u8, color_stride - w));
	}

	// AND mask: a set bit means transparent. Leftmost pixel is the high bit.
	for y in (0..height).rev() {
		let start = data.len();
		for chunk in opaque[y * w..(y + 1) * w].chunks(8) {
			let mut byte = 0u8;
			for (bit, &is_opaque) in chunk.iter().enumerate() {
				if !is_opaque {
					byte |= 0x80 >> bit;
				}
			}
			data.push(byte);
		}
		data.extend(repeat_n(0u8, mask_stride - (data.len() - start)));
	}

	return data;
}

/// Build a palette of at most [`PALETTE_LEN`] colors covering the opaque pixels, exactly when they
/// are few enough and by median cut when they are not.
fn quantize(rgb: &[[u8; 3]], opaque: &[bool]) -> Vec<[u8; 3]> {
	let mut counts: HashMap<[u8; 3], u32> = HashMap::new();
	for (color, &is_opaque) in rgb.iter().zip(opaque) {
		if is_opaque {
			*counts.entry(*color).or_default() += 1;
		}
	}
	let mut colors: ColorBox = counts.into_iter().collect();
	// Sorted so the palette does not depend on hash iteration order.
	colors.sort_unstable();

	if colors.is_empty() {
		return vec![[0, 0, 0]];
	}
	if colors.len() <= PALETTE_LEN {
		return colors.into_iter().map(|(color, _)| return color).collect();
	}

	let mut boxes = vec![colors];
	while boxes.len() < PALETTE_LEN {
		// Split whichever box still spans the widest range on any one channel.
		let Some(i) = boxes
			.iter()
			.enumerate()
			.filter(|(_, color_box)| return color_box.len() > 1)
			.max_by_key(|(_, color_box)| return widest_channel(color_box).1)
			.map(|(i, _)| return i)
		else {
			break;
		};
		let (channel, _) = widest_channel(&boxes[i]);
		let (lo, hi) = split_at_median(boxes.swap_remove(i), channel);
		boxes.push(lo);
		boxes.push(hi);
	}
	return boxes.iter().map(average).collect();
}

/// The channel with the largest spread in a box, and that spread.
fn widest_channel(color_box: &ColorBox) -> (usize, u8) {
	return (0..3)
		.map(|ch| {
			let min = color_box
				.iter()
				.map(|(color, _)| return color[ch])
				.min()
				.unwrap_or(0);
			let max = color_box
				.iter()
				.map(|(color, _)| return color[ch])
				.max()
				.unwrap_or(0);
			return (ch, max - min);
		})
		.max_by_key(|&(_, range)| return range)
		.unwrap_or((0, 0));
}

/// Split a box at the point where half its pixels lie on either side.
fn split_at_median(mut color_box: ColorBox, channel: usize) -> (ColorBox, ColorBox) {
	color_box.sort_unstable_by_key(|(color, _)| return color[channel]);
	let half = color_box
		.iter()
		.map(|(_, n)| return u64::from(*n))
		.sum::<u64>()
		/ 2;
	let mut running = 0u64;
	let mut at = 0;
	for (i, (_, n)) in color_box.iter().enumerate() {
		running += u64::from(*n);
		if running > half {
			at = i;
			break;
		}
	}
	// Both halves must be non-empty or the loop above would never terminate.
	let at = at.clamp(1, color_box.len() - 1);
	let hi = color_box.split_off(at);
	return (color_box, hi);
}

/// Pixel-count-weighted mean color of a box.
fn average(color_box: &ColorBox) -> [u8; 3] {
	let total: u64 = color_box.iter().map(|(_, n)| return u64::from(*n)).sum();
	if total == 0 {
		return [0, 0, 0];
	}
	let mut out = [0u8; 3];
	for (ch, slot) in out.iter_mut().enumerate() {
		let sum: u64 = color_box
			.iter()
			.map(|(color, n)| return u64::from(color[ch]) * u64::from(*n))
			.sum();
		*slot = u8::try_from(sum / total).expect("a mean of bytes is a byte");
	}
	return out;
}

/// Index of the palette entry closest to `color` by squared distance.
fn nearest(palette: &[[u8; 3]], color: [u8; 3]) -> u8 {
	let mut best = (0usize, i32::MAX);
	for (i, entry) in palette.iter().enumerate() {
		let dist: i32 = (0..3)
			.map(|ch| {
				let delta = i32::from(entry[ch]) - i32::from(color[ch]);
				return delta * delta;
			})
			.sum();
		if dist < best.1 {
			best = (i, dist);
		}
	}
	return u8::try_from(best.0).expect("palette holds at most 256 entries");
}
