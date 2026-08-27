//! SVG rasterization via resvg/usvg/tiny-skia.
//!
//! All rendering preserves aspect ratio and fits the drawing inside the target box (transparent
//! letterbox padding), matching `rsvg-convert --keep-aspect-ratio`.
//!
//! resvg is built without its `text` feature, so text never reaches this module:
//! [`crate::svg_text::strip_text`] removes it from the document first.

use anyhow::{Context, Result, ensure};
use resvg::{tiny_skia, usvg};

/// Parse SVG bytes into a usvg tree.
pub fn load_tree_from_data(data: &[u8]) -> Result<usvg::Tree> {
	let opt = usvg::Options::default();
	usvg::Tree::from_data(data, &opt).context("parsing SVG")
}

/// Render the tree into a `target_w` x `target_h` pixmap, scaling to fit inside while preserving
/// aspect ratio and centering the result.
// Icon sides are small enough to be exact in an f32, and the pixmap allocation below is what would
// fail on an absurd one.
#[allow(clippy::cast_precision_loss)]
pub fn render_size(tree: &usvg::Tree, target_w: u32, target_h: u32) -> Result<tiny_skia::Pixmap> {
	let target_w = target_w.max(1);
	let target_h = target_h.max(1);

	let size = tree.size();
	let (w, h) = (size.width(), size.height());
	let scale = (target_w as f32 / w).min(target_h as f32 / h);
	let scaled_w = w * scale;
	let scaled_h = h * scale;
	let tx = (target_w as f32 - scaled_w) / 2.0;
	let ty = (target_h as f32 - scaled_h) / 2.0;

	let transform = tiny_skia::Transform::from_scale(scale, scale).post_translate(tx, ty);

	let mut pixmap = tiny_skia::Pixmap::new(target_w, target_h)
		.context("allocating pixmap (size too large?)")?;
	resvg::render(tree, transform, &mut pixmap.as_mut());
	Ok(pixmap)
}

/// Render the tree at a uniform scale `k` into a pixmap sized to the scaled intrinsic size, with no
/// translation. Device pixel (x, y) therefore maps to tree coordinate (x/k, y/k) exactly, used for
/// geometric measurement.
// Rounding the scaled size up to whole pixels is the point of the cast; `max` keeps it positive,
// and `as` saturates rather than wrapping.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub fn render_scale(tree: &usvg::Tree, k: f32) -> Result<tiny_skia::Pixmap> {
	let size = tree.size();
	let w = (size.width() * k).ceil().max(1.0) as u32;
	let h = (size.height() * k).ceil().max(1.0) as u32;
	let mut pixmap = tiny_skia::Pixmap::new(w, h).context("allocating pixmap")?;
	resvg::render(
		tree,
		tiny_skia::Transform::from_scale(k, k),
		&mut pixmap.as_mut(),
	);
	Ok(pixmap)
}

/// Encode a pixmap to PNG bytes.
pub fn encode_png(pixmap: &tiny_skia::Pixmap) -> Result<Vec<u8>> {
	pixmap.encode_png().context("encoding PNG")
}

/// Root-mean-square difference between two renders, normalized to 0..1. Identical pixmaps give 0.
///
/// The comparison is over premultiplied RGBA bytes.
// The byte count is at most 4 * 2^32, well inside what an f64 counts exactly.
#[allow(clippy::cast_precision_loss)]
pub fn rmse(a: &tiny_skia::Pixmap, b: &tiny_skia::Pixmap) -> Result<f64> {
	ensure!(
		a.width() == b.width() && a.height() == b.height(),
		"comparing a {}x{} render against a {}x{} one",
		a.width(),
		a.height(),
		b.width(),
		b.height()
	);

	let (a, b) = (a.data(), b.data());
	let sum: f64 = a
		.iter()
		.zip(b)
		.map(|(x, y)| {
			let d = f64::from(*x) - f64::from(*y);
			d * d
		})
		.sum();
	Ok((sum / a.len() as f64).sqrt() / 255.0)
}

#[cfg(test)]
mod tests {
	use super::*;

	fn filled(w: u32, h: u32, color: tiny_skia::Color) -> tiny_skia::Pixmap {
		let mut p = tiny_skia::Pixmap::new(w, h).unwrap();
		p.fill(color);
		p
	}

	// The identity case is exactly 0, not approximately.
	#[allow(clippy::float_cmp)]
	#[test]
	fn rmse_of_a_pixmap_against_itself_is_zero() {
		let p = filled(8, 8, tiny_skia::Color::from_rgba8(12, 34, 56, 255));
		assert_eq!(rmse(&p, &p).unwrap(), 0.0);
	}

	/// Opaque white against a transparent pixmap differs by the full range on every byte.
	#[allow(clippy::float_cmp)]
	#[test]
	fn rmse_of_opposite_pixmaps_is_one() {
		let white = filled(8, 8, tiny_skia::Color::from_rgba8(255, 255, 255, 255));
		let clear = filled(8, 8, tiny_skia::Color::TRANSPARENT);
		assert_eq!(rmse(&white, &clear).unwrap(), 1.0);
	}

	#[test]
	fn rmse_rejects_mismatched_sizes() {
		let a = filled(8, 8, tiny_skia::Color::TRANSPARENT);
		let b = filled(8, 4, tiny_skia::Color::TRANSPARENT);
		assert!(rmse(&a, &b).is_err());
	}
}
