//! SVG rasterization via resvg/usvg/tiny-skia.
//!
//! All rendering preserves aspect ratio and fits the drawing inside the target box (transparent
//! letterbox padding), matching `rsvg-convert --keep-aspect-ratio`.
//!
//! resvg is built without its `text` feature, so text never reaches this module:
//! [`crate::svg_text::strip_text`] removes it from the document first.

use anyhow::{Context, Result};
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
