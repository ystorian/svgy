// src/affine.rs

//! Bake an affine `A(p) = s*p + t` into SVG coordinates, with no `transform` added.
//!
//! Both coordinate-baking commands share this: `svg` resize scales and centers the artwork on the
//! output canvas, and `round` fits it inside the inscribed circle. Position components get
//! `s*n + t`, lengths and relative offsets get `s*n`, and existing transforms are conjugated so the
//! document renders the same.

use anyhow::{Context as _, Result, anyhow};
use quick_xml::events::{BytesStart, Event};
use quick_xml::reader::Reader;
use std::collections::HashSet;

use crate::numeric::{affine_length, fmt_num, parse_num_list, scale_len_list, scale_length};

/// Ids that a `<use>` instantiates, `<use href="#logo">` -> `logo`.
pub fn use_targets(src: &str) -> Result<HashSet<String>> {
	let mut reader = Reader::from_str(src);
	let mut ids = HashSet::new();
	loop {
		let element = match reader.read_event().context("parsing SVG")? {
			Event::Eof => break,
			Event::Start(element) | Event::Empty(element) => element,
			_ => continue,
		};
		if element.local_name().into_inner() != "use" {
			continue;
		}
		for key in ["href", "xlink:href"] {
			if let Some(href) = attr_value(&element, key)?
				&& let Some(id) = href.trim().strip_prefix('#')
			{
				ids.insert(id.to_owned());
			}
		}
	}
	return Ok(ids);
}

/// A `<symbol>`, or anything a `<use>` refers to: its coordinates are relative to where the
/// instance is placed, so a translation must not reach them.
pub fn is_template(element: &BytesStart, templates: &HashSet<String>) -> Result<bool> {
	if element.local_name().into_inner() == "symbol" {
		return Ok(true);
	}
	return Ok(attr_value(element, "id")?.is_some_and(|id| return templates.contains(&id)));
}

/// The normalized value of one attribute, if present.
pub fn attr_value(element: &BytesStart, key: &str) -> Result<Option<String>> {
	for attr in element.attributes() {
		let attr = attr.map_err(|err| anyhow!("attribute: {err}"))?;
		if attr.key.as_ref() == key {
			return Ok(Some(
				attr.normalized_value(quick_xml::XmlVersion::Implicit1_0)?
					.into_owned(),
			));
		}
	}
	return Ok(None);
}

/// Apply the affine to one attribute. Position components get `s*n + t`; lengths and relative
/// offsets get `s*n`.
pub fn attr(key: &str, val: &str, scale: f64, tx: f64, ty: f64) -> String {
	match key {
		"transform" | "gradientTransform" | "patternTransform" => {
			return transform(val, scale, tx, ty);
		}
		"d" => return path_data(val, scale, tx, ty),
		"points" => return points(val, scale, tx, ty),
		"stroke-dasharray" | "dx" | "dy" => return scale_len_list(val, scale),
		"x" | "cx" | "x1" | "x2" | "fx" => return affine_length(val, scale, tx),
		"y" | "cy" | "y1" | "y2" | "fy" => return affine_length(val, scale, ty),
		"width" | "height" | "r" | "rx" | "ry" | "stroke-width" | "stroke-dashoffset"
		| "font-size" => return scale_length(val, scale),
		_ => return val.to_owned(),
	}
}

/// Position attributes that default to `0` when absent, per element. A translation has to be
/// written out for them: an omitted `x` is not the same as `x="0"` once the artwork moves.
pub fn implicit_origins(local: &str) -> &'static [&'static str] {
	match local {
		"rect" | "image" | "use" | "svg" | "foreignObject" => return &["x", "y"],
		"circle" | "ellipse" => return &["cx", "cy"],
		"line" => return &["x1", "y1", "x2", "y2"],
		_ => return &[],
	}
}

/// The translation an [`implicit_origins`] attribute carries.
pub fn origin_value(key: &str, tx: f64, ty: f64) -> String {
	match key {
		"y" | "cy" | "y1" | "y2" => return fmt_num(ty),
		_ => return fmt_num(tx),
	}
}

/// Map each `x y` pair of a `points` list.
fn points(val: &str, scale: f64, tx: f64, ty: f64) -> String {
	return parse_num_list(val)
		.chunks(2)
		.map(|pair| {
			if pair.len() == 2 {
				return format!(
					"{} {}",
					fmt_num(scale * pair[0] + tx),
					fmt_num(scale * pair[1] + ty)
				);
			}
			return fmt_num(scale * pair[0] + tx);
		})
		.collect::<Vec<_>>()
		.join(" ");
}

/// Conjugate a `transform` list by the affine `A(p) = s*p + t`: each primitive becomes `A T A^-1`.
/// translate/rotate keep their kind; scale/skew/matrix become an equivalent `matrix(...)` (their
/// fixed point moves under translation).
fn transform(val: &str, scale: f64, tx: f64, ty: f64) -> String {
	let mut out = String::new();
	let mut rest = val;
	loop {
		let trimmed = rest.trim_start_matches([' ', ',', '\t', '\n', '\r']);
		if trimmed.is_empty() {
			break;
		}
		let Some((name, after)) = trimmed.split_once('(') else {
			out.push_str(trimmed);
			break;
		};
		let Some((inner, tail)) = after.split_once(')') else {
			out.push_str(trimmed);
			break;
		};
		let name = name.trim();
		let args = parse_num_list(inner);
		if !out.is_empty() {
			out.push(' ');
		}
		out.push_str(&conjugate(name, &args, scale, tx, ty));
		rest = tail;
	}
	return out;
}

/// Conjugate one transform primitive by the affine.
fn conjugate(name: &str, args: &[f64], scale: f64, tx: f64, ty: f64) -> String {
	let arg = |i: usize, default: f64| return args.get(i).copied().unwrap_or(default);
	match name {
		"translate" => {
			let (dx, dy) = (arg(0, 0.0), arg(1, 0.0));
			return format!("translate({} {})", fmt_num(scale * dx), fmt_num(scale * dy));
		}
		"rotate" => {
			// center is a position -> full affine (default origin -> t).
			let (angle, cx, cy) = (arg(0, 0.0), arg(1, 0.0), arg(2, 0.0));
			return format!(
				"rotate({} {} {})",
				fmt_num(angle),
				fmt_num(scale * cx + tx),
				fmt_num(scale * cy + ty)
			);
		}
		"scale" => {
			let sx = arg(0, 1.0);
			let sy = arg(1, sx);
			return matrix_str([sx, 0.0, 0.0, sy, (1.0 - sx) * tx, (1.0 - sy) * ty]);
		}
		"skewX" => {
			let tan = (arg(0, 0.0).to_radians()).tan();
			return matrix_str([1.0, 0.0, tan, 1.0, -tan * ty, 0.0]);
		}
		"skewY" => {
			let tan = (arg(0, 0.0).to_radians()).tan();
			return matrix_str([1.0, tan, 0.0, 1.0, 0.0, -tan * tx]);
		}
		"matrix" => {
			let (ma, mb, mc, md, me, mf) = (
				arg(0, 1.0),
				arg(1, 0.0),
				arg(2, 0.0),
				arg(3, 1.0),
				arg(4, 0.0),
				arg(5, 0.0),
			);
			let me2 = scale * me + (1.0 - ma) * tx - mc * ty;
			let mf2 = scale * mf - mb * tx + (1.0 - md) * ty;
			return matrix_str([ma, mb, mc, md, me2, mf2]);
		}
		_ => {
			let joined = args
				.iter()
				.map(|val| return fmt_num(*val))
				.collect::<Vec<_>>()
				.join(" ");
			return format!("{name}({joined})");
		}
	}
}

/// Format six values as `matrix(...)`.
fn matrix_str(values: [f64; 6]) -> String {
	return format!("matrix({})", nums(&values));
}

/// Apply the affine to path `d`: absolute coords get `s*p + t`, relative deltas get `s*p`; arc
/// rx/ry scale, rotation and flags are preserved.
fn path_data(data: &str, scale: f64, tx: f64, ty: f64) -> String {
	use svgtypes::{PathParser, PathSegment};

	// Absolute point -> affine; relative point -> scale-only.
	let pt = |x: f64, y: f64, abs: bool| {
		if abs {
			return (scale * x + tx, scale * y + ty);
		}
		return (scale * x, scale * y);
	};

	let mut out = String::new();
	// A leading `m` is absolute.
	let mut first = true;
	for seg in PathParser::from(data) {
		let Ok(seg) = seg else {
			return data.to_owned();
		};
		match seg {
			PathSegment::MoveTo { abs, x, y } => {
				let absolute = abs || first;
				let (x, y) = pt(x, y, absolute);
				cmd(&mut out, 'M', absolute);
				out.push_str(&two(x, y));
			}
			PathSegment::LineTo { abs, x, y } => {
				let (x, y) = pt(x, y, abs);
				cmd(&mut out, 'L', abs);
				out.push_str(&two(x, y));
			}
			PathSegment::HorizontalLineTo { abs, x } => {
				let x = if abs { scale * x + tx } else { scale * x };
				cmd(&mut out, 'H', abs);
				out.push_str(&fmt_num(x));
			}
			PathSegment::VerticalLineTo { abs, y } => {
				let y = if abs { scale * y + ty } else { scale * y };
				cmd(&mut out, 'V', abs);
				out.push_str(&fmt_num(y));
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
				let (x1, y1) = pt(x1, y1, abs);
				let (x2, y2) = pt(x2, y2, abs);
				let (x, y) = pt(x, y, abs);
				cmd(&mut out, 'C', abs);
				out.push_str(&nums(&[x1, y1, x2, y2, x, y]));
			}
			PathSegment::SmoothCurveTo { abs, x2, y2, x, y } => {
				let (x2, y2) = pt(x2, y2, abs);
				let (x, y) = pt(x, y, abs);
				cmd(&mut out, 'S', abs);
				out.push_str(&nums(&[x2, y2, x, y]));
			}
			PathSegment::Quadratic { abs, x1, y1, x, y } => {
				let (x1, y1) = pt(x1, y1, abs);
				let (x, y) = pt(x, y, abs);
				cmd(&mut out, 'Q', abs);
				out.push_str(&nums(&[x1, y1, x, y]));
			}
			PathSegment::SmoothQuadratic { abs, x, y } => {
				let (x, y) = pt(x, y, abs);
				cmd(&mut out, 'T', abs);
				out.push_str(&two(x, y));
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
				let (x, y) = pt(x, y, abs);
				cmd(&mut out, 'A', abs);
				// The two flags are single digits, not numbers to format.
				out.push_str(&nums(&[rx * scale, ry * scale, x_axis_rotation]));
				out.push(' ');
				out.push(if large_arc { '1' } else { '0' });
				out.push(' ');
				out.push(if sweep { '1' } else { '0' });
				out.push(' ');
				out.push_str(&two(x, y));
			}
			PathSegment::ClosePath { abs } => cmd(&mut out, 'Z', abs),
		}
		first = false;
		out.push(' ');
	}
	return out.trim_end().to_owned();
}

/// Push a path command letter and a space.
fn cmd(out: &mut String, upper: char, abs: bool) {
	out.push(if abs {
		upper
	} else {
		upper.to_ascii_lowercase()
	});
	out.push(' ');
}

/// Format two values as `x y`.
fn two(first: f64, second: f64) -> String {
	return nums(&[first, second]);
}

/// Space-separated `fmt_num` of each value.
fn nums(values: &[f64]) -> String {
	return values
		.iter()
		.map(|val| return fmt_num(*val))
		.collect::<Vec<_>>()
		.join(" ");
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn path_absolute_and_relative() {
		// abs M gets scale+translate; rel l gets scale only.
		let out = path_data("M10 10 l5 0", 2.0, 3.0, 4.0);
		assert_eq!(out, "M 23 24 l 10 0");
	}

	/// A leading `m` is absolute, so it must take the translation.
	#[test]
	fn path_leading_relative_moveto_is_absolute() {
		let out = path_data("m10 10 l5 0", 2.0, 3.0, 4.0);
		assert_eq!(out, "M 23 24 l 10 0");
		let out = path_data("M0 0 m10 10", 2.0, 3.0, 4.0);
		assert_eq!(out, "M 3 4 m 20 20");
	}

	#[test]
	fn transform_translate_and_rotate() {
		assert_eq!(
			transform("translate(5 5)", 2.0, 3.0, 4.0),
			"translate(10 10)"
		);
		// rotate about origin -> rotate about t.
		assert_eq!(transform("rotate(90)", 2.0, 3.0, 4.0), "rotate(90 3 4)");
	}

	/// A pure scale leaves the shape of each primitive alone.
	#[test]
	fn scale_only_keeps_translate_and_rotate() {
		assert_eq!(
			transform("translate(10 20)", 2.0, 0.0, 0.0),
			"translate(20 40)"
		);
		assert_eq!(transform("rotate(45 4 4)", 2.0, 0.0, 0.0), "rotate(45 8 8)");
	}

	#[test]
	fn arc_keeps_its_flags() {
		// rx,ry and endpoint scale; rotation + flags stay.
		let out = path_data("M0 0 A5 5 0 0 1 10 10", 2.0, 0.0, 0.0);
		assert_eq!(out, "M 0 0 A 10 10 0 0 1 20 20");
	}
}
