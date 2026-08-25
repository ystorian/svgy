//! Command-line interface (clap).

use clap::{Args, Parser};
use std::path::PathBuf;

/// Default longest-side length when no sizing parameter is given.
pub const DEFAULT_SIZE: u32 = 1024;

/// Suffix used for output names when no destination is given.
pub const DEFAULT_SUFFIX: &str = "svgy";

/// Suffix used for `--round`, so it never collides with `--svg`.
pub const ROUND_SUFFIX: &str = "round";

/// Padding applied to `--round` when the flag is given without a value, and
/// when the source turns out to have a full-canvas background.
pub const DEFAULT_ROUND_PADDING: f64 = 0.1;

#[derive(Parser)]
#[command(
	name = "svgy",
	version,
	about = "Convert an SVG to PNG / ICO / ICNS, or resize an SVG"
)]
// `Option<Option<PathBuf>>` is how clap spells a flag with an optional value, and all three cases
// are meaningful here: absent, present bare (derive the name), present with a path.
#[allow(clippy::option_option)]
// The flags are independent switches on one command line.
#[allow(clippy::struct_excessive_bools)]
pub struct Cli {
	/// Input SVG file
	pub input: PathBuf,

	/// Write an SVG (the default when no other target is given)
	#[arg(long, value_name = "PATH", num_args = 0..=1, require_equals = true)]
	pub svg: Option<Option<PathBuf>>,
	/// Write a PNG
	#[arg(long, value_name = "PATH", num_args = 0..=1, require_equals = true)]
	pub png: Option<Option<PathBuf>>,
	/// Write a Windows icon
	#[arg(long, value_name = "PATH", num_args = 0..=1, require_equals = true)]
	pub ico: Option<Option<PathBuf>>,
	/// Write a macOS icon
	#[arg(long, value_name = "PATH", num_args = 0..=1, require_equals = true)]
	pub icns: Option<Option<PathBuf>>,
	/// Write an SVG whose artwork is fitted inside the inscribed circle
	#[arg(long, value_name = "PATH", num_args = 0..=1, require_equals = true)]
	pub round: Option<Option<PathBuf>>,

	#[command(flatten)]
	pub size: SizeOpts,

	/// Fraction of the inscribed circle's radius left empty (0.0..1.0); --round only
	#[arg(
		long,
		value_name = "FRACTION",
		num_args = 0..=1,
		require_equals = true,
		default_missing_value = "0.1"
	)]
	pub padding: Option<f64>,

	/// Suffix used when a destination is not specified
	#[arg(long, default_value = DEFAULT_SUFFIX)]
	pub suffix: String,

	/// Keep the original dimensions
	#[arg(long = "no-resize")]
	pub no_resize: bool,

	/// Skip oxipng optimization
	#[arg(long = "no-optimize")]
	pub no_optimize: bool,

	/// Compress PNGs with Zopfli: 1~5% smaller, 100x times slower
	#[arg(long)]
	pub zopfli: bool,

	/// Skip the 256-color BMP entries for 16x16 and 32x32
	#[arg(long = "no-legacy-ico")]
	pub no_legacy: bool,
}

impl Cli {
	/// `true` when no output target was requested, in which case `--svg` is implied. Any explicit
	/// target turns that default off.
	pub fn no_target(&self) -> bool {
		self.svg.is_none()
			&& self.png.is_none()
			&& self.ico.is_none()
			&& self.icns.is_none()
			&& self.round.is_none()
	}

	/// Whether an SVG output is wanted, and where it goes.
	pub fn svg_target(&self) -> Option<&Option<PathBuf>> {
		if self.no_target() {
			// The implicit default: `svgy file.svg` -> `file.svgy.svg`.
			return Some(&None);
		}
		self.svg.as_ref()
	}
}

/// Target size, preserving aspect ratio and fitting inside the box.
#[derive(Clone, Copy)]
pub enum Target {
	/// Longest side becomes N.
	Square(u32),
	/// Fixed width, height derived from aspect ratio.
	Width(u32),
	/// Fixed height, width derived from aspect ratio.
	Height(u32),
	/// Fit inside W x H.
	Both(u32, u32),
}

#[derive(Args, Clone, Copy)]
pub struct SizeOpts {
	/// Longest side in pixels; takes precedence over --width/--height
	#[arg(short = 's', long = "size")]
	pub size: Option<u32>,
	/// Target width in pixels
	#[arg(long)]
	pub width: Option<u32>,
	/// Target height in pixels
	#[arg(long)]
	pub height: Option<u32>,
}

impl SizeOpts {
	/// Resolve to a [`Target`], or `None` if no size was given.
	pub fn target(&self) -> Option<Target> {
		match (self.size, self.width, self.height) {
			(Some(s), _, _) => Some(Target::Square(s)),
			(None, Some(w), Some(h)) => Some(Target::Both(w, h)),
			(None, Some(w), None) => Some(Target::Width(w)),
			(None, None, Some(h)) => Some(Target::Height(h)),
			(None, None, None) => None,
		}
	}

	/// The target to resize by, falling back to [`DEFAULT_SIZE`].
	pub fn target_or_default(&self) -> Target {
		self.target().unwrap_or(Target::Square(DEFAULT_SIZE))
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use clap::Parser;
	use std::path::Path;

	fn parse(args: &[&str]) -> Cli {
		Cli::try_parse_from(args).expect("parse")
	}

	#[test]
	fn bare_input_implies_svg() {
		let cli = parse(&["svgy", "logo.svg"]);
		assert!(cli.no_target());
		assert!(matches!(cli.svg_target(), Some(None)));
	}

	#[test]
	fn explicit_target_drops_the_implicit_svg() {
		let cli = parse(&["svgy", "logo.svg", "--png"]);
		assert!(!cli.no_target());
		assert!(cli.svg_target().is_none());
		assert!(matches!(cli.png, Some(None)));
	}

	#[test]
	fn svg_can_be_kept_alongside_another_target() {
		let cli = parse(&["svgy", "logo.svg", "--png", "--svg"]);
		assert!(matches!(cli.svg_target(), Some(None)));
		assert!(matches!(cli.png, Some(None)));
	}

	#[test]
	fn targets_take_an_optional_path() {
		let cli = parse(&["svgy", "logo.svg", "--icns=macos/logo.icns", "--ico"]);
		assert_eq!(
			cli.icns.as_ref().unwrap().as_deref(),
			Some(Path::new("macos/logo.icns"))
		);
		assert!(matches!(cli.ico, Some(None)));
	}

	#[test]
	fn bare_padding_is_the_default_fraction() {
		let cli = parse(&["svgy", "logo.svg", "--round", "--padding"]);
		assert_eq!(cli.padding, Some(DEFAULT_ROUND_PADDING));
		let cli = parse(&["svgy", "logo.svg", "--round", "--padding=0.25"]);
		assert_eq!(cli.padding, Some(0.25));
		let cli = parse(&["svgy", "logo.svg", "--round"]);
		assert_eq!(cli.padding, None);
	}

	/// Zopfli is opt-in.
	#[test]
	fn zopfli_is_off_by_default() {
		let cli = parse(&["svgy", "logo.svg", "--png"]);
		assert!(!cli.zopfli);
		let cli = parse(&["svgy", "logo.svg", "--png", "--zopfli"]);
		assert!(cli.zopfli);
	}

	#[test]
	fn size_defaults_to_the_documented_value() {
		let cli = parse(&["svgy", "logo.svg"]);
		assert!(matches!(
			cli.size.target_or_default(),
			Target::Square(DEFAULT_SIZE)
		));
		let cli = parse(&["svgy", "logo.svg", "--width=200"]);
		assert!(matches!(cli.size.target_or_default(), Target::Width(200)));
	}
}
