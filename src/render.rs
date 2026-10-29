// src/render.rs

//! SVG rasterization via resvg/usvg/tiny-skia.
//!
//! All rendering preserves aspect ratio and fits the drawing inside the target box (transparent
//! letterbox padding), matching `rsvg-convert --keep-aspect-ratio`.
//!
//! resvg is built without its `text` feature, so text never reaches this module:
//! [`crate::svg_text::strip_text`] removes it from the document first.

use anyhow::{Context as _, Result, ensure};
use resvg::{tiny_skia, usvg};

/// Parse SVG bytes into a usvg tree.
pub fn load_tree_from_data(data: &[u8]) -> Result<usvg::Tree> {
	let opt = usvg::Options::default();
	return usvg::Tree::from_data(data, &opt).context("parsing SVG");
}

/// Render the tree into a `target_w` x `target_h` pixmap, scaling to fit inside while preserving
/// aspect ratio and centering the result.
// Icon sides are small enough to be exact in an f32, and the pixmap allocation below is what would
// fail on an absurd one.
#[expect(clippy::cast_precision_loss, reason = "icon sides are exact in f32")]
pub fn render_size(tree: &usvg::Tree, target_w: u32, target_h: u32) -> Result<tiny_skia::Pixmap> {
	let target_w = target_w.max(1);
	let target_h = target_h.max(1);

	let size = tree.size();
	let (width, height) = (size.width(), size.height());
	let scale = (target_w as f32 / width).min(target_h as f32 / height);
	let scaled_w = width * scale;
	let scaled_h = height * scale;
	let tx = (target_w as f32 - scaled_w) / 2.0;
	let ty = (target_h as f32 - scaled_h) / 2.0;

	let transform = tiny_skia::Transform::from_scale(scale, scale).post_translate(tx, ty);

	let mut pixmap = tiny_skia::Pixmap::new(target_w, target_h)
		.context("allocating pixmap (size too large?)")?;
	resvg::render(tree, transform, &mut pixmap.as_mut());
	return Ok(pixmap);
}

/// Render the tree at a uniform `scale` into a pixmap sized to the scaled intrinsic size, with no
/// translation. Device pixel (x, y) therefore maps to tree coordinate (x/scale, y/scale) exactly, used for
/// geometric measurement.
// Rounding the scaled size up to whole pixels is the point of the cast; `max` keeps it positive,
// and `as` saturates rather than wrapping.
#[expect(
	clippy::cast_possible_truncation,
	clippy::cast_sign_loss,
	reason = "round up to whole pixels"
)]
pub fn render_scale(tree: &usvg::Tree, scale: f32) -> Result<tiny_skia::Pixmap> {
	let size = tree.size();
	let w = (size.width() * scale).ceil().max(1.0) as u32;
	let height = (size.height() * scale).ceil().max(1.0) as u32;
	let mut pixmap = tiny_skia::Pixmap::new(w, height).context("allocating pixmap")?;
	resvg::render(
		tree,
		tiny_skia::Transform::from_scale(scale, scale),
		&mut pixmap.as_mut(),
	);
	return Ok(pixmap);
}

/// Encode a pixmap to PNG bytes.
pub fn encode_png(pixmap: &tiny_skia::Pixmap) -> Result<Vec<u8>> {
	return pixmap.encode_png().context("encoding PNG");
}

/// Root-mean-square difference between two renders, normalized to 0..1. Identical pixmaps give 0.
///
/// The comparison is over premultiplied RGBA bytes.
// The byte count is at most 4 * 2^32, well inside what an f64 counts exactly.
#[expect(clippy::cast_precision_loss, reason = "byte counts are exact in f64")]
pub fn rmse(left: &tiny_skia::Pixmap, right: &tiny_skia::Pixmap) -> Result<f64> {
	ensure!(
		left.width() == right.width() && left.height() == right.height(),
		"comparing a {}x{} render against a {}x{} one",
		left.width(),
		left.height(),
		right.width(),
		right.height()
	);

	let (left, right) = (left.data(), right.data());
	let sum: f64 = left
		.iter()
		.zip(right)
		.map(|(x, y)| {
			let diff = f64::from(*x) - f64::from(*y);
			return diff * diff;
		})
		.sum();
	return Ok((sum / left.len() as f64).sqrt() / 255.0);
}

#[cfg(test)]
mod tests {
	use super::*;

	fn filled(w: u32, height: u32, color: tiny_skia::Color) -> tiny_skia::Pixmap {
		let mut pixmap = tiny_skia::Pixmap::new(w, height).unwrap();
		pixmap.fill(color);
		return pixmap;
	}

	// The identity case is exactly 0, not approximately.
	#[test]
	fn rmse_of_a_pixmap_against_itself_is_zero() {
		let pixmap = filled(8, 8, tiny_skia::Color::from_rgba8(12, 34, 56, 255));
		assert_eq!(rmse(&pixmap, &pixmap).unwrap(), 0.0);
	}

	/// Opaque white against a transparent pixmap differs by the full range on every byte.
	#[test]
	fn rmse_of_opposite_pixmaps_is_one() {
		let white = filled(8, 8, tiny_skia::Color::from_rgba8(255, 255, 255, 255));
		let clear = filled(8, 8, tiny_skia::Color::TRANSPARENT);
		assert_eq!(rmse(&white, &clear).unwrap(), 1.0);
	}

	#[test]
	fn rmse_rejects_mismatched_sizes() {
		let square = filled(8, 8, tiny_skia::Color::TRANSPARENT);
		let wide = filled(8, 4, tiny_skia::Color::TRANSPARENT);
		assert!(rmse(&square, &wide).is_err());
	}
}
