//! SVG optimization via oxvg.

use anyhow::{Context, Result, anyhow};
use oxvg_ast::{
	parse::roxmltree::{ParsingOptions, parse_with_options},
	serialize::{Indent, Node as _, Options},
	visitor::Info,
};
use oxvg_optimiser::{
	CleanupNumericValues, ConvertPathData, ConvertShapeToPath, ConvertStyleToAttrs,
	ConvertTransform, Jobs, RemoveAttrs, RemoveDimensions, RemoveXlink,
};
use oxvg_path::geometry::Tolerance;
use quick_xml::events::Event;
use quick_xml::reader::Reader;

use crate::cli::{Cli, DEFAULT_PRECISION_THRESHOLD};
use crate::render;

/// Candidates for the search, ascending: the lowest one under the budget wins.
const PRECISION_RANGE: std::ops::RangeInclusive<i32> = 0..=5;

/// The lowest candidate, which every mix starts from.
const PRECISION_FLOOR: i32 = *PRECISION_RANGE.start();

/// The precision used when the search is off, and when no candidate fits. This is the default of
/// oxvg itself.
const DEFAULT_PRECISION: i32 = 3;

/// The square that candidates are rendered on to be compared.
const CANVAS: u32 = 1024;

/// How many times the job list runs over one document.
///
/// The jobs run once each, in a fixed order, and the ones that read the numbers run before the ones
/// that round them. A merge that rounding makes possible therefore needs another run. Every test
/// file settles after the second run, and the cap stops a document that never settles.
const MAX_PASSES: usize = 3;

/// How to round the numbers in the optimized SVG.
#[derive(Clone, Copy)]
pub enum Precision {
	/// The fixed default of oxvg.
	Default,
	/// The lowest precision whose render stays within `threshold` of the source.
	Adaptive { threshold: f64 },
}

/// How much effort to spend on an SVG.
#[derive(Clone, Copy)]
pub struct Opts {
	/// Whether to run oxvg, cleared by `--no-optimize`.
	pub enabled: bool,
	/// `--no-resize`: the root width and height are the source's own and are kept.
	pub keep_dimensions: bool,
	/// Which precision to write the numbers at.
	pub precision: Precision,
}

impl Opts {
	pub fn from_cli(cli: &Cli) -> Self {
		let threshold = if cli.no_precision {
			None
		} else {
			Some(cli.precision.unwrap_or(DEFAULT_PRECISION_THRESHOLD))
		};
		Self {
			enabled: !cli.no_optimize,
			keep_dimensions: cli.no_resize,
			precision: threshold.map_or(Precision::Default, |threshold| Precision::Adaptive {
				threshold,
			}),
		}
	}
}

/// An optimized SVG, and what the precision search settled on.
pub struct Optimized {
	pub svg: String,
	/// `None` when the search did not run.
	pub precision: Option<i32>,
	/// Rendered difference against the source, 0..1. `None` as above.
	pub difference: Option<f64>,
	/// How many elements a mix wrote above the lowest precision. `None` when no mix was written.
	pub upgrades: Option<usize>,
}

/// Optimize `src` with oxvg when enabled, otherwise return it unchanged.
pub fn maybe_optimize(src: &str, opts: Opts) -> Result<Optimized> {
	if !opts.enabled {
		return Ok(Optimized {
			svg: src.to_string(),
			precision: None,
			difference: None,
			upgrades: None,
		});
	}
	let dynamic = is_dynamic(src)?;
	match opts.precision {
		Precision::Default => Ok(Optimized {
			svg: optimize_at(src, &jobs(opts.keep_dimensions, None, dynamic), dynamic)?,
			precision: None,
			difference: None,
			upgrades: None,
		}),
		Precision::Adaptive { threshold } => search(src, opts.keep_dimensions, threshold, dynamic),
	}
}

/// Whether `src` has a `<style>` or a `<script>`, which decide what an element looks like in a way
/// that reading the attributes cannot follow.
///
/// A `style` attribute beats a rule of a stylesheet, and a presentation attribute loses to one, so
/// `convert_style_to_attrs` is unsafe here. A script names an element by its `id`, so the dead root
/// `id` stays as well.
fn is_dynamic(src: &str) -> Result<bool> {
	let mut reader = Reader::from_str(src);
	loop {
		match reader.read_event().context("parsing SVG")? {
			Event::Eof => return Ok(false),
			Event::Start(e) | Event::Empty(e)
				if matches!(e.local_name().into_inner(), "style" | "script") =>
			{
				return Ok(true);
			}
			_ => {}
		}
	}
}

/// The lowest precision whose render stays within `threshold` of `src`.
///
/// The error grows as the precision drops, so the first candidate that fits is the smallest one
/// that fits and the scan can stop there. Every candidate is kept, since a mix of them can be
/// smaller than the one that won.
fn search(src: &str, keep_dimensions: bool, threshold: f64, dynamic: bool) -> Result<Optimized> {
	// A source that cannot be rendered would fail every other target too, so this one is fatal.
	let reference = rasterize(src).context("rendering the source to compare against")?;

	let mut fallback: Option<String> = None;
	let mut best: Option<(i32, f64)> = None;
	let mut previous: Option<String> = None;
	// Every candidate, lowest precision first, for the mix to draw from.
	let mut built: Vec<String> = Vec::new();

	for p in PRECISION_RANGE {
		let svg = optimize_at(src, &jobs(keep_dimensions, Some(p), dynamic), dynamic)?;
		if p == DEFAULT_PRECISION {
			fallback = Some(svg.clone());
		}
		// Identical bytes cannot render differently, so the previous verdict still stands.
		let repeat = previous.as_deref() == Some(svg.as_str());
		built.push(svg.clone());
		if repeat {
			continue;
		}

		// A candidate rounded past what the renderer accepts is rejected, not fatal.
		if let Ok(candidate) = rasterize(&svg) {
			let difference = render::rmse(&reference, &candidate)?;
			if difference <= threshold {
				return refine(svg, p, difference, &built, &reference, threshold);
			}
			if best.is_none_or(|(_, b)| difference < b) {
				best = Some((p, difference));
			}
		}
		previous = Some(svg);
	}

	match best {
		Some((p, difference)) => eprintln!(
			"Warning: no precision met the {:.2}% budget (best {:.2}% at precision {p}), using \
			 precision {DEFAULT_PRECISION}.",
			threshold * 100.0,
			difference * 100.0
		),
		None => eprintln!(
			"Warning: no precision produced a document that renders, using precision \
			 {DEFAULT_PRECISION}."
		),
	}

	// What is written is what was written before the search existed.
	match fallback {
		Some(svg) => Ok(Optimized {
			svg,
			precision: Some(DEFAULT_PRECISION),
			difference: None,
			upgrades: None,
		}),
		None => Ok(Optimized {
			svg: optimize_at(src, &jobs(keep_dimensions, None, dynamic), dynamic)?,
			precision: None,
			difference: None,
			upgrades: None,
		}),
	}
}

/// The winner of the search, or a mix of the candidates when that one is smaller.
///
/// One precision for the whole document gives every element the decimals that the worst element
/// needs. A mix writes each element at its own precision, which spends the budget rather than
/// undershooting it.
fn refine(
	winner: String,
	precision: i32,
	difference: f64,
	built: &[String],
	reference: &resvg::tiny_skia::Pixmap,
	threshold: f64,
) -> Result<Optimized> {
	let kept = |svg: String, upgrades| Optimized {
		svg,
		precision: Some(precision),
		difference: Some(difference),
		upgrades,
	};
	// Nothing sits below the lowest precision, and a mix can only reach what the lowest one saves.
	let room = built.first().is_some_and(|base| base.len() < winner.len());
	if precision == PRECISION_FLOOR || !room {
		return Ok(kept(winner, None));
	}

	let mut measure = |svg: &str| -> Result<Option<f64>> {
		match rasterize(svg) {
			Ok(pixmap) => Ok(Some(render::rmse(reference, &pixmap)?)),
			// A mix that does not render is no candidate, and the next try may still be one.
			Err(_) => Ok(None),
		}
	};
	match crate::svg_mix::mix(built, threshold, &mut measure)? {
		Some(mixed) if mixed.svg.len() < winner.len() => Ok(Optimized {
			svg: mixed.svg,
			precision: Some(precision),
			difference: Some(mixed.difference),
			upgrades: Some(mixed.upgrades),
		}),
		_ => Ok(kept(winner, None)),
	}
}

/// Rasterize `svg` onto the comparison canvas.
fn rasterize(svg: &str) -> Result<resvg::tiny_skia::Pixmap> {
	let tree = render::load_tree_from_data(svg.as_bytes())?;
	render::render_size(&tree, CANVAS, CANVAS)
}

/// Run `jobs` over `src` and serialize the result.
///
/// Five cleanups frame the oxvg jobs. The dead root `id`, a lent gradient, a `gradientTransform` and
/// the opacity decimals go first, since the jobs read all four. The namespace declarations that
/// nothing uses go last, since the serializer repeats the ones it cannot place.
fn optimize_at(src: &str, jobs: &Jobs, dynamic: bool) -> Result<String> {
	// The root `id` goes first: it stops `remove_useless_stroke_and_fill` on the whole document.
	let named = crate::svg_id::strip_dead_root_id(src, dynamic)?;
	// A gradient that only lends its stops is written into the one that borrows them, before the fold
	// reads the transform that the borrower inherits.
	let lent = crate::svg_gradient::inline(&named, dynamic)?;
	// Gradients are folded next, so the numbers the fold produces are rounded with the rest.
	let folded = crate::svg_gradient::flatten(&lent)?;
	// Opacities are rounded before the jobs, since the serializer of the styles keeps what it reads.
	let short = crate::svg_style::round_opacities(&folded)?;
	crate::svg_namespace::strip_unused(&run_passes(&short, jobs)?)
}

/// Run the job list until it stops paying.
///
/// A run reads what the run before it wrote, so a rounded number can now merge with its neighbour.
/// A run that writes no fewer bytes is dropped: the fixed point is reached, or the jobs are trading a
/// shape for a longer one.
fn run_passes(src: &str, jobs: &Jobs) -> Result<String> {
	let mut best = run_jobs(src, jobs)?;
	for _ in 1..MAX_PASSES {
		let next = run_jobs(&best, jobs)?;
		if next.len() >= best.len() {
			break;
		}
		best = next;
	}
	Ok(best)
}

fn run_jobs(src: &str, jobs: &Jobs) -> Result<String> {
	// The document lives in an arena that ends with the closure, so the string is all that leaves.
	parse_with_options(
		src,
		// A DOCTYPE must parse before `remove_doctype` can strip it.
		ParsingOptions {
			allow_dtd: true,
			..ParsingOptions::default()
		},
		|dom, allocator| -> Result<String> {
			jobs.run(dom, &Info::new(allocator))
				// `JobsError` borrows the arena, so only its message survives.
				.map_err(|e| anyhow!("{e}"))
				.context("oxvg optimization")?;
			// `minify` shortens attribute values (`10px` -> `10`) and is separate from `indent`.
			dom.serialize_with_options(Options {
				indent: Indent::Tabs,
				..Options::default()
			})
			.context("serializing the optimized SVG")
		},
	)
	.context("parsing SVG")?
}

/// The default preset, plus the svgy settings.
///
/// A `precision` of `None` leaves every job at the default of oxvg. Otherwise one number drives the
/// four jobs that round numbers, and the rest of each job keeps its own defaults.
fn jobs(keep_dimensions: bool, precision: Option<i32>, dynamic: bool) -> Jobs {
	let mut jobs = Jobs {
		// Resizing already drops the root width/height, this covers `--no-resize`.
		remove_dimensions: Some(RemoveDimensions(!keep_dimensions)),
		// `xlink:href` becomes `href`, which lets the `xmlns:xlink` declaration go too.
		remove_xlink: Some(RemoveXlink::default()),
		// A presentation attribute is what `remove_unknowns_and_defaults` reads, so an editor's
		// `style="fill:red;fill-opacity:1"` loses the part that repeats the default. A stylesheet
		// makes the move unsafe, since an attribute loses to a rule that a style attribute wins.
		convert_style_to_attrs: (!dynamic).then(ConvertStyleToAttrs::default),
		// `<text>` is stripped up front, so `xml:space` has nothing left to act on. It does stop
		// the serializer from indenting the document, which is why it must go.
		remove_attrs: Some(RemoveAttrs {
			// The default separator is `:`, which a prefixed attribute name needs for itself.
			elem_separator: "|".to_string(),
			attrs: vec!["*|xml:space".to_string()],
			..RemoveAttrs::default()
		}),
		// A future `--keep-ids` sets `cleanup_ids` to `remove: false, minify: false`.
		..Jobs::default()
	};
	let Some(p) = precision else {
		return jobs;
	};

	jobs.cleanup_numeric_values = Some(CleanupNumericValues {
		// Above 5 this aborts the whole run rather than skipping the job.
		float_precision: u8::try_from(p.clamp(0, 5)).unwrap_or(5),
		..CleanupNumericValues::default()
	});
	jobs.convert_shape_to_path = Some(ConvertShapeToPath {
		float_precision: p,
		..ConvertShapeToPath::default()
	});
	jobs.convert_path_data = Some(ConvertPathData {
		// `positional` and `angular` are absolute distances, not digit counts, so they stay put.
		tolerance: Tolerance {
			precision: p,
			..Tolerance::default()
		},
		..ConvertPathData::default()
	});
	jobs.convert_transform = Some(ConvertTransform {
		// 0 rounds to integers rather than turning rounding off, so the floor is 1.
		float_precision: p.max(1),
		transform_precision: (p + 2).max(1),
		..ConvertTransform::default()
	});
	jobs
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Optimize at the fixed default, the behaviour from before the search existed.
	fn fixed(src: &str, keep_dimensions: bool) -> String {
		maybe_optimize(
			src,
			Opts {
				enabled: true,
				keep_dimensions,
				precision: Precision::Default,
			},
		)
		.unwrap()
		.svg
	}

	fn adaptive(src: &str, threshold: f64) -> Optimized {
		maybe_optimize(
			src,
			Opts {
				enabled: true,
				keep_dimensions: false,
				precision: Precision::Adaptive { threshold },
			},
		)
		.unwrap()
	}

	#[test]
	fn disabled_returns_the_input_verbatim() {
		let src = r#"<svg xmlns="http://www.w3.org/2000/svg"><!-- keep me --></svg>"#;
		let out = maybe_optimize(
			src,
			Opts {
				enabled: false,
				keep_dimensions: false,
				precision: Precision::Adaptive { threshold: 0.02 },
			},
		)
		.unwrap();
		assert_eq!(out.svg, src);
		assert!(out.precision.is_none());
	}

	/// Values are shortened, and `Indent::Tabs` keeps the structure readable.
	#[test]
	fn values_are_minified_and_elements_are_indented() {
		let src = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><rect x="1px" y="2px" width="4px" height="6px" fill="#ff0000"/></svg>"##;
		let out = fixed(src, false);
		// Units dropped, the colour written as short as it goes.
		assert!(!out.contains("px"), "{out}");
		assert!(out.contains(r#"fill="red""#), "{out}");
		// One element per line, indented with a tab.
		assert!(out.contains("\">\n\t<"), "{out}");
	}

	/// `xml:space` stops the serializer from indenting, so its removal is what makes tabs appear.
	#[test]
	fn xml_space_goes_and_the_document_is_still_indented() {
		let src = r#"<svg xmlns="http://www.w3.org/2000/svg" xml:space="preserve" viewBox="0 0 10 10"><rect width="10" height="10"/></svg>"#;
		let out = fixed(src, false);
		assert!(!out.contains("xml:space"), "{out}");
		assert!(out.contains("\">\n\t<"), "{out}");
	}

	/// `xlink:href` becomes `href`, which leaves the declaration unused and drops it.
	// Two gradients borrow the stops, so the reference stays for `removeXlink` to rewrite.
	#[test]
	fn xlink_is_replaced_by_the_native_reference() {
		let src = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" viewBox="0 0 10 10"><defs><linearGradient id="a" gradientUnits="userSpaceOnUse" x1="0" y1="0" x2="10" y2="10"><stop offset="0" stop-color="#f00"/><stop offset="1" stop-color="#00f"/></linearGradient><linearGradient id="b" xlink:href="#a" gradientUnits="userSpaceOnUse" x1="0" y1="0" x2="5" y2="5"/><linearGradient id="c" xlink:href="#a" gradientUnits="userSpaceOnUse" x1="0" y1="0" x2="8" y2="8"/></defs><rect width="10" height="10" fill="url(#b)"/><rect width="4" height="4" fill="url(#c)"/></svg>"##;
		let out = fixed(src, false);
		assert!(!out.contains("xlink"), "{out}");
		assert!(out.contains("href=\"#"), "{out}");
	}

	/// A gradient that lends its stops to one gradient alone becomes part of it.
	#[test]
	fn a_lent_gradient_is_written_into_its_borrower() {
		let src = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" viewBox="0 0 10 10"><defs><linearGradient id="a"><stop offset="0" stop-color="#f00"/><stop offset="1" stop-color="#00f"/></linearGradient><linearGradient id="b" xlink:href="#a" gradientUnits="userSpaceOnUse" x1="0" y1="0" x2="10" y2="10"/></defs><rect width="10" height="10" fill="url(#b)"/></svg>"##;
		let out = fixed(src, false);
		assert!(!out.contains("href"), "{out}");
		assert_eq!(out.matches("linearGradient").count(), 2, "{out}");
	}

	/// A style declaration becomes an attribute, and a default then goes with the rest.
	#[test]
	fn a_style_becomes_attributes_and_loses_the_defaults() {
		let src = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><path style="fill:#f00;fill-opacity:1;fill-rule:nonzero" d="M0 0h10v10H0Z"/></svg>"#;
		let out = fixed(src, false);
		assert!(!out.contains("style="), "{out}");
		assert!(out.contains(r#"fill="red""#), "{out}");
		// `fill-opacity:1` and `fill-rule:nonzero` are the defaults, so they carry nothing.
		assert!(!out.contains("fill-opacity"), "{out}");
		assert!(!out.contains("fill-rule"), "{out}");
	}

	/// A rule of a stylesheet beats a presentation attribute, and the source paints the square red.
	/// The move to attributes would make it blue, so the `style` attribute stays.
	#[test]
	fn a_stylesheet_keeps_the_style_attribute() {
		let src = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><style>path{fill:blue}</style><path style="fill:red" d="M0 0h10v10H0Z"/></svg>"#;
		let out = fixed(src, false);
		assert!(out.contains("style=\"fill:red\""), "{out}");
		assert!(!out.contains("#00f"), "{out}");
	}

	/// The segments are merged before the numbers are rounded, so a run of lines that rounding makes
	/// collinear is only merged by the run after it.
	#[test]
	fn a_second_pass_merges_what_rounding_made_collinear() {
		let src = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 40"><path fill="#000" d="M1 1l0.04 10l-0.04 10l0.03 10h5v-30Z"/></svg>"##;
		let all = jobs(false, Some(1), false);
		let one = run_jobs(src, &all).unwrap();
		let many = run_passes(src, &all).unwrap();
		// Three vertical lines that the first run leaves side by side.
		assert!(one.contains("v10 10 10"), "{one}");
		assert!(many.contains("v30"), "{many}");
	}

	/// A run that pays nothing is dropped, so the passes stop rather than trade bytes away.
	#[test]
	fn the_passes_never_write_more_than_the_first_one() {
		let src = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><path fill="#000" d="M12.3456 50.7891c4.1357-22.8642 31.9187-33.4501 49.2318-19.7734 12.6429 9.9871 14.3095 30.2286 3.4172 42.1129-11.7433 12.8118-34.8821 11.9042-45.9376-1.4287-4.2119-5.9218-7.1642-13.4419-6.7114-20.9108Z"/></svg>"##;
		let all = jobs(false, Some(1), false);
		let one = run_jobs(src, &all).unwrap();
		let many = run_passes(src, &all).unwrap();
		assert!(many.len() <= one.len(), "{many}");
	}

	/// Proves the default preset ran rather than an empty job list.
	#[test]
	fn comments_and_metadata_are_stripped() {
		let src = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><!-- a note --><metadata>x</metadata><rect width="10" height="10"/></svg>"#;
		let out = fixed(src, false);
		assert!(!out.contains("a note"), "{out}");
		assert!(!out.contains("metadata"), "{out}");
	}

	/// `removeViewBox` is not in the default preset.
	#[test]
	fn viewbox_survives_optimization() {
		let src = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 512"><rect width="1024" height="512"/></svg>"#;
		let out = fixed(src, false);
		assert!(out.contains(r#"viewBox="0 0 1024 512""#), "{out}");
	}

	#[test]
	fn dimensions_follow_the_resize_flag() {
		let src = r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10" viewBox="0 0 10 10"><rect width="10" height="10"/></svg>"#;

		let resized = fixed(src, false);
		assert!(!resized.contains(r#"width="10""#), "{resized}");

		let kept = fixed(src, true);
		assert!(kept.contains(r#"width="10""#), "{kept}");
		assert!(kept.contains(r#"height="10""#), "{kept}");
	}

	/// IDs are minified, so the reference and its target must still agree afterwards.
	#[test]
	fn referenced_ids_are_never_broken() {
		let src = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><defs><linearGradient id="the-gradient" gradientUnits="userSpaceOnUse" x1="0" y1="0" x2="10" y2="10"><stop offset="0" stop-color="#f00"/><stop offset="1" stop-color="#00f"/></linearGradient></defs><rect width="10" height="10" fill="url(#the-gradient)"/></svg>"##;
		let out = fixed(src, false);

		let start = out.find("url(#").expect("reference kept") + "url(#".len();
		let end = start + out[start..].find(')').expect("closed reference");
		let target = &out[start..end];
		assert!(out.contains(&format!(r#"id="{target}""#)), "{out}");
	}

	/// End to end: the optimized bytes still parse, and the size contract holds.
	// The viewBox carries these two values verbatim, so an exact comparison is the assertion.
	#[allow(clippy::float_cmp)]
	#[test]
	fn optimized_output_still_renders() {
		use crate::cli::Target;
		use crate::svg_resize::Canvas;

		let src = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 200 100"><rect width="200" height="100" fill="#000"/></svg>"##;
		let resized =
			crate::svg_resize::transform_svg(src, Target::Square(1024), Canvas::Tight).unwrap();
		let out = fixed(&resized, false);

		let tree = crate::render::load_tree_from_data(out.as_bytes()).unwrap();
		assert_eq!(tree.size().width(), 1024.0);
		assert_eq!(tree.size().height(), 512.0);
	}

	/// Artwork already on whole numbers loses nothing at the lowest precision, and a document at the
	/// lowest precision leaves nothing to mix.
	#[test]
	fn integer_artwork_selects_the_lowest_precision() {
		let src = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024"><path fill="#000" d="M64 64h512v256H64Z"/></svg>"##;
		let out = adaptive(src, 0.02);
		assert_eq!(out.precision, Some(0));
		assert_eq!(out.upgrades, None);
	}

	/// The big shape needs the decimals and the small ones do not, so only the big one gets them.
	#[test]
	fn a_mix_spends_the_decimals_where_they_show() {
		use std::fmt::Write as _;

		let big = "M112.37 512.61c0-221.44 179.55-401.02 400.99-401.02s401.02 179.58 401.02 401.02\
		           -179.58 400.99-401.02 400.99S112.37 734.05 112.37 512.61Z";
		let mut src = format!(
			r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024"><path fill="#0a7" d="{big}"/>"##
		);
		for i in 0..12 {
			let x = 100.0 + f64::from(i) * 61.7301;
			let y = 140.0 + f64::from(i) * 47.3129;
			write!(
				src,
				r##"<path fill="#333" d="M{x:.4} {y:.4}h9.3271v9.4173h-9.3271Z"/>"##
			)
			.unwrap();
		}
		src += "</svg>";

		let threshold = 0.005;
		let out = adaptive(&src, threshold);
		assert_eq!(out.upgrades, Some(1), "{}", out.svg);
		assert!(out.difference.unwrap() <= threshold);
		// The big shape keeps a decimal, and the small ones are written whole.
		assert!(out.svg.contains("M112.4 512.6"), "{}", out.svg);
		assert!(out.svg.contains("M100 140h9v9h-9Z"), "{}", out.svg);
		assert!(crate::render::load_tree_from_data(out.svg.as_bytes()).is_ok());
	}

	/// Curves need the decimals: a tight budget must push the search above the lowest precision.
	#[test]
	fn curved_artwork_needs_more_precision() {
		let src = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><path fill="#000" d="M12.3456 50.7891c4.1357-22.8642 31.9187-33.4501 49.2318-19.7734 12.6429 9.9871 14.3095 30.2286 3.4172 42.1129-11.7433 12.8118-34.8821 11.9042-45.9376-1.4287-4.2119-5.9218-7.1642-13.4419-6.7114-20.9108Z"/></svg>"##;
		let threshold = 0.002;
		let out = adaptive(src, threshold);
		let p = out.precision.expect("a precision was chosen");
		assert!(p > 0, "precision {p} should be above the lowest");
		assert!(out.difference.unwrap() <= threshold);
	}

	/// A budget nothing can meet falls back rather than failing. A difference is never negative, so
	/// this budget is out of reach whatever the artwork.
	#[test]
	fn an_unreachable_threshold_falls_back_to_the_default() {
		let src = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><path fill="#000" d="M12.3456 50.7891c4.1357-22.8642 31.9187-33.4501 49.2318-19.7734 12.6429 9.9871 14.3095 30.2286 3.4172 42.1129-11.7433 12.8118-34.8821 11.9042-45.9376-1.4287-4.2119-5.9218-7.1642-13.4419-6.7114-20.9108Z"/></svg>"##;
		let out = adaptive(src, -1.0);
		assert_eq!(out.precision, Some(DEFAULT_PRECISION));
		assert!(crate::render::load_tree_from_data(out.svg.as_bytes()).is_ok());
	}

	/// A budget of zero is met when the render is pixel-identical, which it can be.
	#[test]
	fn a_zero_threshold_is_met_by_an_exact_render() {
		let src = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024"><path fill="#000" d="M64 64h512v256H64Z"/></svg>"##;
		let out = adaptive(src, 0.0);
		assert_eq!(out.difference, Some(0.0));
	}

	/// Above 5 the optimizer aborts the run, so the mapping must clamp.
	#[test]
	fn precision_above_five_is_clamped() {
		let jobs = jobs(false, Some(9), false);
		assert_eq!(jobs.cleanup_numeric_values.unwrap().float_precision, 5);
	}
}
