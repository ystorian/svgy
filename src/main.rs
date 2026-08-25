#![deny(unsafe_code)]
#![warn(clippy::pedantic)]
// Geometry reads better with the names the maths uses: a circle is (cx, cy, r), a triangle is (a,
// b, c), and spelling those out obscures more than it helps.
#![allow(clippy::many_single_char_names, clippy::similar_names)]

mod cli;
mod icns;
mod ico;
mod numeric;
mod optimize;
mod render;
mod round;
mod svg_resize;
mod svg_text;

use anyhow::{Context, Result};
use clap::Parser;
use cli::{Cli, ROUND_SUFFIX};
use resvg::usvg;
use std::path::{Path, PathBuf};

fn main() {
	if let Err(e) = run() {
		eprintln!("Error: {e:#}");
		std::process::exit(1);
	}
}

fn run() -> Result<()> {
	let cli = Cli::parse();

	let src = std::fs::read_to_string(&cli.input)
		.with_context(|| format!("reading {}", cli.input.display()))?;

	// Text cannot be rendered, so it is removed before anything else looks at the
	// document: every target then agrees on what the artwork is.
	let (src, had_text) = svg_text::strip_text(&src)?;
	if had_text {
		eprintln!(
			"Warning: <text> is not supported and was removed. Convert it to paths first \
			 (Inkscape: Path > Object to Path)."
		);
	}

	// Resize first: every other target is derived from the resized document, so
	// `--size` reaches the SVG, the PNG and the rounded output alike.
	let resized = if cli.no_resize {
		src
	} else {
		svg_resize::transform_svg(&src, cli.size.target_or_default())?
	};

	if let Some(dest) = cli.svg_target() {
		let out = resolve_output(&cli, dest.as_deref(), &cli.suffix, "svg")?;
		write(&out, resized.as_bytes())?;
	}

	// One parse shared by every raster target.
	if cli.png.is_some() || cli.ico.is_some() || cli.icns.is_some() {
		let tree = render::load_tree_from_data(resized.as_bytes())?;
		let opt = optimize::Opts::from_cli(&cli);

		if let Some(dest) = &cli.png {
			let out = resolve_output(&cli, dest.as_deref(), &cli.suffix, "png")?;
			write_png(&tree, &out, opt)?;
		}
		if let Some(dest) = &cli.ico {
			let out = resolve_output(&cli, dest.as_deref(), &cli.suffix, "ico")?;
			let entries = ico::write_ico(&tree, opt, !cli.no_legacy, &out)?;
			println!(
				"Wrote {} ({} sizes, {entries} entries)",
				out.display(),
				ico::SIZES.len()
			);
		}
		if let Some(dest) = &cli.icns {
			let out = resolve_output(&cli, dest.as_deref(), &cli.suffix, "icns")?;
			icns::write_icns(&tree, opt, &out)?;
			println!("Wrote {}", out.display());
		}
	}

	// Rounding runs last, inside whatever canvas the resize established.
	if let Some(dest) = &cli.round {
		let out = resolve_output(&cli, dest.as_deref(), ROUND_SUFFIX, "svg")?;
		let r = round::round_str(&resized, cli.padding)?;
		write(&out, r.svg.as_bytes())?;
		let chosen = if r.auto_padding { ", chosen" } else { "" };
		println!(
			"  scaled content x{:.4} into r={} circle (padding {}{chosen})",
			r.scale,
			numeric::fmt_num(r.radius),
			numeric::fmt_num(r.padding),
		);
	}

	Ok(())
}

fn cmd_png_size(tree: &usvg::Tree) -> (u32, u32) {
	// Rounding to whole pixels is the point of the cast, and `max(1.0)` keeps the value positive
	// before it is taken.
	#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
	let round = |v: f32| f64::from(v).round().max(1.0) as u32;
	let size = tree.size();
	(round(size.width()), round(size.height()))
}

fn write_png(tree: &usvg::Tree, out: &Path, opt: optimize::Opts) -> Result<()> {
	// The tree already carries the requested size, so the PNG is its intrinsic size: no letterbox,
	// and `--size` means the same thing it does for the SVG.
	let (w, h) = cmd_png_size(tree);
	let pixmap = render::render_size(tree, w, h)?;
	let png = render::encode_png(&pixmap)?;
	let png = optimize::maybe_optimize(png, opt)?;
	std::fs::write(out, &png).with_context(|| format!("writing {}", out.display()))?;
	println!("Wrote {} ({w}x{h}, {} bytes)", out.display(), png.len());
	Ok(())
}

fn write(out: &Path, bytes: &[u8]) -> Result<()> {
	std::fs::write(out, bytes).with_context(|| format!("writing {}", out.display()))?;
	println!("Wrote {} ({} bytes)", out.display(), bytes.len());
	Ok(())
}

/// Where a target writes: the explicit destination if given, else the source name with `suffix` and
/// `ext` beside it, `logo.svg` -> `logo.svgy.png`.
///
/// A destination may name a directory that does not exist yet, as the icons-into-subfolders example
/// does, so the parent is created here rather than failing partway through a multi-target run.
fn resolve_output(cli: &Cli, dest: Option<&Path>, suffix: &str, ext: &str) -> Result<PathBuf> {
	let out = match dest {
		Some(p) => p.to_path_buf(),
		None => default_name(&cli.input, suffix, ext),
	};
	if let Some(dir) = out.parent()
		&& !dir.as_os_str().is_empty()
	{
		std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
	}
	Ok(out)
}

fn default_name(input: &Path, suffix: &str, ext: &str) -> PathBuf {
	let stem = input.file_stem().and_then(|s| s.to_str()).unwrap_or("out");
	input.with_file_name(format!("{stem}.{suffix}.{ext}"))
}

#[cfg(test)]
mod tests {
	use super::*;
	use cli::DEFAULT_SUFFIX;

	#[test]
	fn default_names_carry_the_suffix() {
		let input = Path::new("art/logo.svg");
		assert_eq!(
			default_name(input, DEFAULT_SUFFIX, "png"),
			Path::new("art/logo.svgy.png")
		);
		assert_eq!(
			default_name(input, DEFAULT_SUFFIX, "svg"),
			Path::new("art/logo.svgy.svg")
		);
		assert_eq!(
			default_name(input, "v2", "icns"),
			Path::new("art/logo.v2.icns")
		);
	}

	/// `--svg` and `--round` both write an SVG, so `--round` uses its own suffix and the two never
	/// claim the same path.
	#[test]
	fn round_never_collides_with_svg() {
		let input = Path::new("logo.svg");
		let svg = default_name(input, DEFAULT_SUFFIX, "svg");
		let round = default_name(input, ROUND_SUFFIX, "svg");
		assert_eq!(svg, Path::new("logo.svgy.svg"));
		assert_eq!(round, Path::new("logo.round.svg"));
		assert_ne!(svg, round);
	}

	/// A custom `--suffix` moves the SVG but leaves `--round` where it is.
	#[test]
	fn custom_suffix_does_not_move_round() {
		let input = Path::new("logo.svg");
		assert_eq!(default_name(input, "v2", "svg"), Path::new("logo.v2.svg"));
		assert_eq!(
			default_name(input, ROUND_SUFFIX, "svg"),
			Path::new("logo.round.svg")
		);
	}

	/// `--size` fits the longest side for the SVG and the PNG alike: a 200x100 source becomes
	/// 1024x512, not a padded 1024x1024.
	#[test]
	fn size_fits_the_longest_side_for_both_outputs() {
		let src = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 200 100"><rect width="200" height="100" fill="#000"/></svg>"##;
		let resized = svg_resize::transform_svg(src, cli::Target::Square(1024)).unwrap();
		assert!(resized.contains(r#"viewBox="0 0 1024 512""#));

		let tree = render::load_tree_from_data(resized.as_bytes()).unwrap();
		assert_eq!(cmd_png_size(&tree), (1024, 512));
	}

	/// `--width`/`--height` together fit inside the box rather than letterboxing.
	#[test]
	fn width_and_height_fit_inside_the_box() {
		let src = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 200 100"><rect width="200" height="100" fill="#000"/></svg>"##;
		let resized = svg_resize::transform_svg(src, cli::Target::Both(300, 300)).unwrap();
		let tree = render::load_tree_from_data(resized.as_bytes()).unwrap();
		assert_eq!(cmd_png_size(&tree), (300, 150));
	}
}
