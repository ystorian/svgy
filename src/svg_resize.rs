//! Resize an SVG by baking a uniform scale into the document.
//!
//! The output (a) rewrites the root `viewBox`, (b) drops the root `width`/`height`, and (c) uses
//! **no** `transform` to scale, the factor is multiplied into every coordinate/length in place,
//! preserving the original element structure. A uniform scale is origin-independent and commutes
//! with rotation, so this renders identically to the source while changing only its intrinsic
//! (viewBox) size.
//!
//! The canvas is padded to the requested size unless [`Canvas::Tight`] is asked for. The artwork is
//! centered in it by [`crate::affine`], and the `viewBox` always starts at `0 0`.

use anyhow::{Context, Result, anyhow, bail};
use quick_xml::events::{BytesStart, Event};
use quick_xml::reader::Reader;
use quick_xml::writer::Writer;
use std::collections::HashSet;

use crate::affine;
use crate::cli::Target;
use crate::numeric::{fmt_num, parse_num_list, parse_px};

struct State {
	seen_root: bool,
	scale: f64,
	/// Translation that centers the scaled artwork on the canvas.
	tx: f64,
	ty: f64,
	new_viewbox: String,
	add_viewbox: bool,
}

/// How much canvas the output keeps around the artwork.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Canvas {
	/// Pad the canvas to the requested size and center the artwork (the default).
	Padded,
	/// Fit the canvas to the artwork.
	Tight,
}

/// Resize `src` by `target`, returning the rewritten document.
pub fn transform_svg(src: &str, target: Target, canvas: Canvas) -> Result<String> {
	let mut reader = Reader::from_str(src);
	let mut writer = Writer::new(Vec::new());
	let mut state = State {
		seen_root: false,
		scale: 1.0,
		tx: 0.0,
		ty: 0.0,
		new_viewbox: String::new(),
		add_viewbox: false,
	};

	let templates = use_targets(src)?;
	// Depth inside a template: it is placed by the `<use>` that carries the translation, so its own
	// coordinates are only scaled.
	let mut template = 0usize;

	loop {
		match reader.read_event().context("parsing SVG")? {
			Event::Eof => break,
			Event::Start(e) => {
				if template > 0 {
					template += 1;
				} else if state.seen_root && is_template(&e, &templates)? {
					template = 1;
				}
				let out = process_element(&e, &mut state, target, canvas, template == 0)?;
				writer.write_event(Event::Start(out))?;
			}
			Event::End(e) => {
				template = template.saturating_sub(1);
				writer.write_event(Event::End(e))?;
			}
			Event::Empty(e) => {
				let inside = template > 0 || (state.seen_root && is_template(&e, &templates)?);
				let out = process_element(&e, &mut state, target, canvas, !inside)?;
				writer.write_event(Event::Empty(out))?;
			}
			other => writer.write_event(other)?,
		}
	}

	Ok(String::from_utf8(writer.into_inner())?)
}

/// Ids that a `<use>` instantiates, `<use href="#logo">` -> `logo`.
fn use_targets(src: &str) -> Result<HashSet<String>> {
	let mut reader = Reader::from_str(src);
	let mut ids = HashSet::new();
	loop {
		let e = match reader.read_event().context("parsing SVG")? {
			Event::Eof => break,
			Event::Start(e) | Event::Empty(e) => e,
			_ => continue,
		};
		if e.local_name().into_inner() != "use" {
			continue;
		}
		for key in ["href", "xlink:href"] {
			if let Some(href) = attr_value(&e, key)?
				&& let Some(id) = href.trim().strip_prefix('#')
			{
				ids.insert(id.to_string());
			}
		}
	}
	Ok(ids)
}

/// A `<symbol>`, or anything a `<use>` refers to: its coordinates are relative to where the
/// instance is placed, so a translation must not reach them.
fn is_template(e: &BytesStart, templates: &HashSet<String>) -> Result<bool> {
	if e.local_name().into_inner() == "symbol" {
		return Ok(true);
	}
	Ok(attr_value(e, "id")?.is_some_and(|id| templates.contains(&id)))
}

/// Rebuild one element with its coordinates scaled.
fn process_element(
	e: &BytesStart,
	state: &mut State,
	target: Target,
	canvas: Canvas,
	translate: bool,
) -> Result<BytesStart<'static>> {
	let local = e.local_name().into_inner().to_string();

	let is_root = !state.seen_root;
	if is_root {
		state.seen_root = true;
		if local != "svg" {
			bail!("root element is <{local}>, expected <svg>");
		}
		let (min_x, min_y, w, h, has_vb) = read_source_box(e)?;
		state.scale = compute_scale(target, w, h);
		let s = state.scale;
		let (want_w, want_h) = target.canvas();
		let (vw, tx) = place_axis(min_x * s, w * s, want_w, canvas);
		let (vh, ty) = place_axis(min_y * s, h * s, want_h, canvas);
		state.tx = tx;
		state.ty = ty;
		state.new_viewbox = format!("0 0 {} {}", fmt_num(vw), fmt_num(vh));
		state.add_viewbox = !has_vb;
	}
	let s = state.scale;
	let (tx, ty) = if translate {
		(state.tx, state.ty)
	} else {
		(0.0, 0.0)
	};

	// Gradient/pattern coordinates are only in user space (and thus scalable) when
	// `*Units="userSpaceOnUse"`; the default objectBoundingBox uses 0..1 ratios that must be left
	// untouched (as must the matching *Transform).
	let is_grad = matches!(
		local.as_str(),
		"linearGradient" | "radialGradient" | "pattern"
	);
	let do_scale = if is_grad {
		let key = if local == "pattern" {
			"patternUnits"
		} else {
			"gradientUnits"
		};
		attr_value(e, key)?.as_deref() == Some("userSpaceOnUse")
	} else {
		true
	};

	let name = e.name().into_inner().to_string();
	let mut out = BytesStart::new(name);
	let mut seen_keys = Vec::new();

	for attr in e.attributes() {
		let attr = attr.map_err(|err| anyhow!("attribute: {err}"))?;
		let key = attr.key.into_inner().to_string();
		let val = attr
			.normalized_value(quick_xml::XmlVersion::Implicit1_0)?
			.into_owned();

		if is_root && (key == "width" || key == "height") {
			continue; // dropped: intrinsic size now comes from viewBox
		}
		if is_root && key == "viewBox" {
			out.push_attribute(("viewBox", state.new_viewbox.as_str()));
			continue;
		}

		let new_val = if do_scale {
			affine::attr(&key, &val, s, tx, ty)
		} else {
			val
		};
		seen_keys.push(key.clone());
		out.push_attribute((key.as_str(), new_val.as_str()));
	}

	// An omitted `x` means 0, which the translation moves like any other coordinate.
	if do_scale && !is_root && (tx != 0.0 || ty != 0.0) {
		for key in affine::implicit_origins(&local) {
			if !seen_keys.iter().any(|k| k == key) {
				let val = affine::origin_value(key, tx, ty);
				out.push_attribute((*key, val.as_str()));
			}
		}
	}

	if is_root && state.add_viewbox {
		out.push_attribute(("viewBox", state.new_viewbox.as_str()));
	}

	Ok(out)
}

/// Source coordinate box: `(min_x, min_y, w, h, had_viewBox)`.
fn read_source_box(e: &BytesStart) -> Result<(f64, f64, f64, f64, bool)> {
	if let Some(vb) = attr_value(e, "viewBox")? {
		let nums = parse_num_list(&vb);
		if nums.len() != 4 {
			bail!("viewBox must have 4 numbers, got {}", nums.len());
		}
		return Ok((nums[0], nums[1], nums[2], nums[3], true));
	}
	match (attr_value(e, "width")?, attr_value(e, "height")?) {
		(Some(w), Some(h)) => {
			let w = parse_px(&w).context("root width")?;
			let h = parse_px(&h).context("root height")?;
			Ok((0.0, 0.0, w, h, false))
		}
		_ => bail!("root <svg> has neither viewBox nor numeric width/height; cannot resize"),
	}
}

/// Tolerance for a scaled edge that is an integer except for floating-point error.
const EDGE_EPS: f64 = 1e-6;

/// Place one axis of the artwork on the output canvas, returning `(canvas length, translation)`.
///
/// The length is a whole number: the artwork rounded up, or `want` when the target fixes this axis
/// and asks for more. The artwork keeps its exact scale and is centered in whatever is left, so the
/// canvas never crops it. The translation is a whole number too whenever the slack allows one.
fn place_axis(start: f64, extent: f64, want: Option<f64>, canvas: Canvas) -> (f64, f64) {
	let mut len = (extent - EDGE_EPS).ceil().max(1.0);
	if canvas == Canvas::Padded
		&& let Some(want) = want
	{
		len = len.max(want);
	}

	let centered = (len - extent) / 2.0 - start;
	// The artwork stays inside the canvas while the translation is in this range.
	let (lo, hi) = (-start, len - extent - start);
	let rounded = centered.round();
	if rounded >= lo - EDGE_EPS && rounded <= hi + EDGE_EPS {
		return (len, rounded);
	}
	(len, centered)
}

fn compute_scale(target: Target, w0: f64, h0: f64) -> f64 {
	match target {
		Target::Square(n) => f64::from(n) / w0.max(h0),
		Target::Width(w) => f64::from(w) / w0,
		Target::Height(h) => f64::from(h) / h0,
		Target::Both(w, h) => (f64::from(w) / w0).min(f64::from(h) / h0),
	}
}

/// First matching attribute value, unescaped.
fn attr_value(e: &BytesStart, key: &str) -> Result<Option<String>> {
	for attr in e.attributes() {
		let attr = attr.map_err(|err| anyhow!("attribute: {err}"))?;
		if attr.key.as_ref() == key {
			return Ok(Some(
				attr.normalized_value(quick_xml::XmlVersion::Implicit1_0)?
					.into_owned(),
			));
		}
	}
	Ok(None)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn root_viewbox_rewritten_no_wh() {
		let src = r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24"><rect x="1" y="2" width="4" height="6"/></svg>"#;
		let out = transform_svg(src, Target::Square(48), Canvas::Padded).unwrap();
		assert!(out.contains(r#"viewBox="0 0 48 48""#));
		assert!(!out.contains("width=\"24\""));
		assert!(!out.contains("height=\"24\""));
		assert!(out.contains(r#"x="2""#));
		assert!(out.contains(r#"width="8""#));
		assert!(!out.contains("transform"));
	}

	#[test]
	fn derives_viewbox_when_absent() {
		let src = r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="20"><rect x="1" y="1" width="2" height="2"/></svg>"#;
		let out = transform_svg(src, Target::Width(20), Canvas::Padded).unwrap();
		assert!(out.contains(r#"viewBox="0 0 20 40""#));
		assert!(!out.contains("width=\"10\""));
	}

	/// 620x720 scaled to a 1024 longest side gives 881.777...; the viewBox is 882 wide.
	#[test]
	fn viewbox_is_rounded_up_to_whole_units() {
		let src = r#"<svg xmlns="http://www.w3.org/2000/svg" width="620" height="720"><rect x="1" y="1" width="2" height="2"/></svg>"#;
		let out = transform_svg(src, Target::Square(1024), Canvas::Tight).unwrap();
		assert!(out.contains(r#"viewBox="0 0 882 1024""#), "{out}");
	}

	/// The same source padded to the square: 1024 - 881.8 = 142.2 of slack, half of it on each side,
	/// and the coordinates carry the shift.
	#[test]
	fn padded_canvas_centers_the_artwork() {
		let src = r#"<svg xmlns="http://www.w3.org/2000/svg" width="620" height="720"><rect x="1" y="1" width="2" height="2"/></svg>"#;
		let out = transform_svg(src, Target::Square(1024), Canvas::Padded).unwrap();
		assert!(out.contains(r#"viewBox="0 0 1024 1024""#), "{out}");
		// x = 1 * 1024/720 + 71, y = 1 * 1024/720.
		assert!(out.contains(r#"x="72.422222""#), "{out}");
		assert!(out.contains(r#"y="1.422222""#), "{out}");
	}

	/// A source whose min is not at the origin lands on `0 0` all the same.
	#[test]
	fn a_shifted_viewbox_is_moved_to_the_origin() {
		let src = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="10 10 10 10"><rect x="10" y="10" width="10" height="10"/></svg>"#;
		let out = transform_svg(src, Target::Square(20), Canvas::Padded).unwrap();
		assert!(out.contains(r#"viewBox="0 0 20 20""#), "{out}");
		assert!(out.contains(r#"x="0""#), "{out}");
		assert!(out.contains(r#"width="20""#), "{out}");
	}

	#[test]
	fn place_axis_pads_only_when_asked() {
		// A free axis, and an axis the artwork already fills, both stay tight.
		assert_eq!(place_axis(0.0, 881.777, None, Canvas::Padded), (882.0, 0.0));
		assert_eq!(
			place_axis(0.0, 1024.0, Some(1024.0), Canvas::Padded),
			(1024.0, 0.0)
		);
		assert_eq!(
			place_axis(0.0, 881.777, Some(1024.0), Canvas::Tight),
			(882.0, 0.0)
		);
		assert_eq!(
			place_axis(0.0, 881.777, Some(1024.0), Canvas::Padded),
			(1024.0, 71.0)
		);
		// A source that starts away from the origin is moved back to it.
		assert_eq!(
			place_axis(10.0, 20.0, Some(20.0), Canvas::Padded),
			(20.0, -10.0)
		);
	}

	/// A template is placed by its `<use>`, so only the instance takes the translation.
	#[test]
	fn a_use_template_is_not_moved_twice() {
		let src = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 200 100"><defs><path id="a" d="M0 0h10"/></defs><use href="#a"/></svg>"##;
		let out = transform_svg(src, Target::Square(200), Canvas::Padded).unwrap();
		assert!(out.contains(r#"viewBox="0 0 200 200""#), "{out}");
		assert!(out.contains(r#"d="M 0 0 h 10""#), "{out}");
		assert!(out.contains(r#"y="50""#), "{out}");
	}

	#[test]
	fn objectboundingbox_gradient_untouched() {
		let src = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><linearGradient x1="0" y1="0" x2="1" y2="1"/></svg>"#;
		let out = transform_svg(src, Target::Square(20), Canvas::Padded).unwrap();
		// default objectBoundingBox: gradient coords stay as 0..1 ratios
		assert!(out.contains(r#"x2="1""#));
	}
}
