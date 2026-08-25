//! PNG optimization via oxipng.

use crate::cli::Cli;
use anyhow::{Context, Result};

/// Preset used by default.
const PRESET: u8 = 4;

/// How much effort to spend on a PNG.
#[derive(Clone, Copy)]
pub struct Opts {
	/// Whether to run oxipng, cleared by `--no-optimize`.
	pub enabled: bool,
	/// Whether to compress with Zopfli, set by `--zopfli`.
	pub zopfli: bool,
}

impl Opts {
	pub fn from_cli(cli: &Cli) -> Self {
		Self {
			enabled: !cli.no_optimize,
			zopfli: cli.zopfli,
		}
	}
}

/// Optimize PNG bytes with oxipng when enabled, otherwise return them as-is.
pub fn maybe_optimize(png: Vec<u8>, opts: Opts) -> Result<Vec<u8>> {
	if !opts.enabled {
		return Ok(png);
	}
	let mut options = oxipng::Options::from_preset(PRESET);
	options.optimize_alpha = true;
	if opts.zopfli {
		options.deflater = oxipng::Deflater::Zopfli(oxipng::ZopfliOptions::default());
	}
	oxipng::optimize_from_memory(&png, &options).context("oxipng optimization")
}
