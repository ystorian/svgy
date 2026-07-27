//! Resize an SVG by baking a uniform scale into the document.
//!
//! The output (a) rewrites the root `viewBox`, (b) drops the root `width`/`height`, and (c) uses
//! **no** `transform` to scale, the factor is multiplied into every coordinate/length in place,
//! preserving the original element structure. A uniform scale is origin-independent and commutes
//! with rotation, so this renders identically to the source while changing only its intrinsic
//! (viewBox) size.

use anyhow::{Context, Result, anyhow, bail};
use quick_xml::events::{BytesStart, Event};
use quick_xml::reader::Reader;
use quick_xml::writer::Writer;

use crate::cli::Target;
use crate::numeric::{
	fmt_num, parse_num_list, parse_px, scale_len_list, scale_length, scale_num_list,
};

struct State {
	seen_root: bool,
	scale: f64,
	new_viewbox: String,
	add_viewbox: bool,
}

/// Resize `src` by `target`, returning the rewritten document.
pub fn transform_svg(src: &str, target: Target) -> Result<String> {
	let mut reader = Reader::from_str(src);
	let mut writer = Writer::new(Vec::new());
	let mut state = State {
		seen_root: false,
		scale: 1.0,
		new_viewbox: String::new(),
		add_viewbox: false,
	};

	loop {
		match reader.read_event().context("parsing SVG")? {
			Event::Eof => break,
			Event::Start(e) => {
				let out = process_element(&e, &mut state, target)?;
				writer.write_event(Event::Start(out))?;
			}
			Event::Empty(e) => {
				let out = process_element(&e, &mut state, target)?;
				writer.write_event(Event::Empty(out))?;
			}
			other => writer.write_event(other)?,
		}
	}

	Ok(String::from_utf8(writer.into_inner())?)
}

/// Rebuild one element with its coordinates scaled.
fn process_element(
	e: &BytesStart,
	state: &mut State,
	target: Target,
) -> Result<BytesStart<'static>> {
	let local = e.local_name();
	let local = std::str::from_utf8(local.as_ref())?.to_string();

	let is_root = !state.seen_root;
	if is_root {
		state.seen_root = true;
		if local != "svg" {
			bail!("root element is <{local}>, expected <svg>");
		}
		let (min_x, min_y, w, h, has_vb) = read_source_box(e)?;
		state.scale = compute_scale(target, w, h);
		let s = state.scale;
		state.new_viewbox = format!(
			"{} {} {} {}",
			fmt_num(min_x * s),
			fmt_num(min_y * s),
			fmt_num(w * s),
			fmt_num(h * s)
		);
		state.add_viewbox = !has_vb;
	}
	let s = state.scale;

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

	let name = std::str::from_utf8(e.name().as_ref())?.to_string();
	let mut out = BytesStart::new(name);

	for attr in e.attributes() {
		let attr = attr.map_err(|err| anyhow!("attribute: {err}"))?;
		let key = std::str::from_utf8(attr.key.as_ref())?.to_string();
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
			scale_attr(&key, &val, s)
		} else {
			val
		};
		out.push_attribute((key.as_str(), new_val.as_str()));
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

fn compute_scale(target: Target, w0: f64, h0: f64) -> f64 {
	match target {
		Target::Square(n) => f64::from(n) / w0.max(h0),
		Target::Width(w) => f64::from(w) / w0,
		Target::Height(h) => f64::from(h) / h0,
		Target::Both(w, h) => (f64::from(w) / w0).min(f64::from(h) / h0),
	}
}

/// Dispatch scaling based on the attribute name.
fn scale_attr(key: &str, val: &str, s: f64) -> String {
	match key {
		"transform" | "gradientTransform" | "patternTransform" => scale_transform(val, s),
		"d" => scale_path_data(val, s),
		"points" => scale_num_list(val, s),
		"stroke-dasharray" => scale_len_list(val, s),
		"x" | "y" | "width" | "height" | "cx" | "cy" | "r" | "rx" | "ry" | "x1" | "y1" | "x2"
		| "y2" | "dx" | "dy" | "fx" | "fy" | "stroke-width" | "stroke-dashoffset" | "font-size" => {
			scale_length(val, s)
		}
		_ => val.to_string(),
	}
}

// --- transform / path scaling ---------------------------------------------

/// Scale the translation parts of a `transform` list without adding transforms. For a uniform scale
/// S: `translate(tx,ty)` -> both scale; `matrix(a b c d e f)` -> only e,f scale;
/// `rotate(a[,cx,cy])` -> center scales, angle does not; `scale/skewX/skewY` -> unchanged.
fn scale_transform(v: &str, s: f64) -> String {
	let mut out = String::new();
	let mut rest = v;
	loop {
		let trimmed = rest.trim_start_matches([' ', ',', '\t', '\n', '\r']);
		if trimmed.is_empty() {
			break;
		}
		let Some(open) = trimmed.find('(') else {
			out.push_str(trimmed);
			break;
		};
		let Some(close) = trimmed[open + 1..].find(')') else {
			out.push_str(trimmed);
			break;
		};
		let name = trimmed[..open].trim();
		let args = parse_num_list(&trimmed[open + 1..open + 1 + close]);
		let scaled: Vec<String> = match name {
			"translate" => args.iter().map(|a| fmt_num(a * s)).collect(),
			"matrix" => args
				.iter()
				.enumerate()
				.map(|(i, a)| if i >= 4 { fmt_num(a * s) } else { fmt_num(*a) })
				.collect(),
			"rotate" => args
				.iter()
				.enumerate()
				.map(|(i, a)| if i == 0 { fmt_num(*a) } else { fmt_num(a * s) })
				.collect(),
			_ => args.iter().map(|a| fmt_num(*a)).collect(),
		};
		if !out.is_empty() {
			out.push(' ');
		}
		out.push_str(name);
		out.push('(');
		out.push_str(&scaled.join(" "));
		out.push(')');
		rest = &trimmed[open + 1 + close + 1..];
	}
	out
}

/// Scale all coordinates in path `d` data. Arc rx/ry/endpoint scale; the x-axis-rotation and the
/// large-arc/sweep flags do not. On any parse error the original string is returned unchanged.
fn scale_path_data(d: &str, s: f64) -> String {
	use svgtypes::{PathParser, PathSegment};

	let mut out = String::new();
	for seg in PathParser::from(d) {
		let Ok(seg) = seg else {
			return d.to_string();
		};
		match seg {
			PathSegment::MoveTo { abs, x, y } => {
				push_cmd(&mut out, 'M', abs);
				out.push_str(&pair(x * s, y * s));
			}
			PathSegment::LineTo { abs, x, y } => {
				push_cmd(&mut out, 'L', abs);
				out.push_str(&pair(x * s, y * s));
			}
			PathSegment::HorizontalLineTo { abs, x } => {
				push_cmd(&mut out, 'H', abs);
				out.push_str(&fmt_num(x * s));
			}
			PathSegment::VerticalLineTo { abs, y } => {
				push_cmd(&mut out, 'V', abs);
				out.push_str(&fmt_num(y * s));
			}
			PathSegment::CurveTo {
				abs,
				x1,
				y1,
				x2,
				y2,
				x,
				y,
			} => {
				push_cmd(&mut out, 'C', abs);
				out.push_str(&nums(&[x1 * s, y1 * s, x2 * s, y2 * s, x * s, y * s]));
			}
			PathSegment::SmoothCurveTo { abs, x2, y2, x, y } => {
				push_cmd(&mut out, 'S', abs);
				out.push_str(&nums(&[x2 * s, y2 * s, x * s, y * s]));
			}
			PathSegment::Quadratic { abs, x1, y1, x, y } => {
				push_cmd(&mut out, 'Q', abs);
				out.push_str(&nums(&[x1 * s, y1 * s, x * s, y * s]));
			}
			PathSegment::SmoothQuadratic { abs, x, y } => {
				push_cmd(&mut out, 'T', abs);
				out.push_str(&pair(x * s, y * s));
			}
			PathSegment::EllipticalArc {
				abs,
				rx,
				ry,
				x_axis_rotation,
				large_arc,
				sweep,
				x,
				y,
			} => {
				push_cmd(&mut out, 'A', abs);
				// The two flags are single digits, not numbers to format.
				out.push_str(&nums(&[rx * s, ry * s, x_axis_rotation]));
				out.push(' ');
				out.push(if large_arc { '1' } else { '0' });
				out.push(' ');
				out.push(if sweep { '1' } else { '0' });
				out.push(' ');
				out.push_str(&pair(x * s, y * s));
			}
			PathSegment::ClosePath { abs } => {
				push_cmd(&mut out, 'Z', abs);
			}
		}
		out.push(' ');
	}
	out.trim_end().to_string()
}

fn push_cmd(out: &mut String, upper: char, abs: bool) {
	out.push(if abs {
		upper
	} else {
		upper.to_ascii_lowercase()
	});
	out.push(' ');
}

fn pair(a: f64, b: f64) -> String {
	nums(&[a, b])
}

/// Space-separated `fmt_num` of each value.
fn nums(vs: &[f64]) -> String {
	vs.iter().map(|v| fmt_num(*v)).collect::<Vec<_>>().join(" ")
}

/// First matching attribute value, unescaped.
fn attr_value(e: &BytesStart, key: &str) -> Result<Option<String>> {
	for attr in e.attributes() {
		let attr = attr.map_err(|err| anyhow!("attribute: {err}"))?;
		if attr.key.as_ref() == key.as_bytes() {
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
	fn scales_length() {
		assert_eq!(scale_length("10", 2.0), "20");
		assert_eq!(scale_length("2.5px", 2.0), "5px");
		assert_eq!(scale_length("50%", 2.0), "50%");
		assert_eq!(scale_length("1em", 2.0), "1em");
	}

	#[test]
	fn scales_path_arc_keeps_flags() {
		// rx,ry and endpoint scale; rotation + flags stay.
		let out = scale_path_data("M0 0 A5 5 0 0 1 10 10", 2.0);
		assert_eq!(out, "M 0 0 A 10 10 0 0 1 20 20");
	}

	#[test]
	fn scales_transform_translate_and_matrix() {
		assert_eq!(scale_transform("translate(10 20)", 2.0), "translate(20 40)");
		assert_eq!(
			scale_transform("matrix(1 0 0 1 5 6)", 2.0),
			"matrix(1 0 0 1 10 12)"
		);
		assert_eq!(scale_transform("scale(3)", 2.0), "scale(3)");
		assert_eq!(scale_transform("rotate(45 4 4)", 2.0), "rotate(45 8 8)");
	}

	#[test]
	fn root_viewbox_rewritten_no_wh() {
		let src = r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24"><rect x="1" y="2" width="4" height="6"/></svg>"#;
		let out = transform_svg(src, Target::Square(48)).unwrap();
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
		let out = transform_svg(src, Target::Width(20)).unwrap();
		assert!(out.contains(r#"viewBox="0 0 20 40""#));
		assert!(!out.contains("width=\"10\""));
	}

	#[test]
	fn objectboundingbox_gradient_untouched() {
		let src = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><linearGradient x1="0" y1="0" x2="1" y2="1"/></svg>"#;
		let out = transform_svg(src, Target::Square(20)).unwrap();
		// default objectBoundingBox: gradient coords stay as 0..1 ratios
		assert!(out.contains(r#"x2="1""#));
	}
}
