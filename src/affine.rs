//! Bake an affine `A(p) = s*p + t` into SVG coordinates, with no `transform` added.
//!
//! Both coordinate-baking commands share this: `svg` resize scales and centers the artwork on the
//! output canvas, and `round` fits it inside the inscribed circle. Position components get
//! `s*n + t`, lengths and relative offsets get `s*n`, and existing transforms are conjugated so the
//! document renders the same.

use crate::numeric::{affine_length, fmt_num, parse_num_list, scale_len_list, scale_length};

/// Apply the affine to one attribute. Position components get `s*n + t`; lengths and relative
/// offsets get `s*n`.
pub fn attr(key: &str, val: &str, s: f64, tx: f64, ty: f64) -> String {
	match key {
		"transform" | "gradientTransform" | "patternTransform" => transform(val, s, tx, ty),
		"d" => path_data(val, s, tx, ty),
		"points" => points(val, s, tx, ty),
		"stroke-dasharray" | "dx" | "dy" => scale_len_list(val, s),
		"x" | "cx" | "x1" | "x2" | "fx" => affine_length(val, s, tx),
		"y" | "cy" | "y1" | "y2" | "fy" => affine_length(val, s, ty),
		"width" | "height" | "r" | "rx" | "ry" | "stroke-width" | "stroke-dashoffset"
		| "font-size" => scale_length(val, s),
		_ => val.to_string(),
	}
}

/// Position attributes that default to `0` when absent, per element. A translation has to be
/// written out for them: an omitted `x` is not the same as `x="0"` once the artwork moves.
pub fn implicit_origins(local: &str) -> &'static [&'static str] {
	match local {
		"rect" | "image" | "use" | "svg" | "foreignObject" => &["x", "y"],
		"circle" | "ellipse" => &["cx", "cy"],
		"line" => &["x1", "y1", "x2", "y2"],
		_ => &[],
	}
}

/// The translation an [`implicit_origins`] attribute carries.
pub fn origin_value(key: &str, tx: f64, ty: f64) -> String {
	match key {
		"y" | "cy" | "y1" | "y2" => fmt_num(ty),
		_ => fmt_num(tx),
	}
}

fn points(v: &str, s: f64, tx: f64, ty: f64) -> String {
	parse_num_list(v)
		.chunks(2)
		.map(|c| {
			if c.len() == 2 {
				format!("{} {}", fmt_num(s * c[0] + tx), fmt_num(s * c[1] + ty))
			} else {
				fmt_num(s * c[0] + tx)
			}
		})
		.collect::<Vec<_>>()
		.join(" ")
}

/// Conjugate a `transform` list by the affine `A(p) = s*p + t`: each primitive becomes `A T A^-1`.
/// translate/rotate keep their kind; scale/skew/matrix become an equivalent `matrix(...)` (their
/// fixed point moves under translation).
fn transform(v: &str, s: f64, tx: f64, ty: f64) -> String {
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
		if !out.is_empty() {
			out.push(' ');
		}
		out.push_str(&conjugate(name, &args, s, tx, ty));
		rest = &trimmed[open + 1 + close + 1..];
	}
	out
}

fn conjugate(name: &str, args: &[f64], s: f64, tx: f64, ty: f64) -> String {
	let g = |i: usize, d: f64| args.get(i).copied().unwrap_or(d);
	match name {
		"translate" => {
			let (dx, dy) = (g(0, 0.0), g(1, 0.0));
			format!("translate({} {})", fmt_num(s * dx), fmt_num(s * dy))
		}
		"rotate" => {
			// center is a position -> full affine (default origin -> t).
			let (a, cx, cy) = (g(0, 0.0), g(1, 0.0), g(2, 0.0));
			format!(
				"rotate({} {} {})",
				fmt_num(a),
				fmt_num(s * cx + tx),
				fmt_num(s * cy + ty)
			)
		}
		"scale" => {
			let a = g(0, 1.0);
			let b = g(1, a);
			matrix_str(a, 0.0, 0.0, b, (1.0 - a) * tx, (1.0 - b) * ty)
		}
		"skewX" => {
			let c = (g(0, 0.0).to_radians()).tan();
			matrix_str(1.0, 0.0, c, 1.0, -c * ty, 0.0)
		}
		"skewY" => {
			let b = (g(0, 0.0).to_radians()).tan();
			matrix_str(1.0, b, 0.0, 1.0, 0.0, -b * tx)
		}
		"matrix" => {
			let (a, b, c, d, e, f) = (
				g(0, 1.0),
				g(1, 0.0),
				g(2, 0.0),
				g(3, 1.0),
				g(4, 0.0),
				g(5, 0.0),
			);
			let e2 = s * e + (1.0 - a) * tx - c * ty;
			let f2 = s * f - b * tx + (1.0 - d) * ty;
			matrix_str(a, b, c, d, e2, f2)
		}
		_ => {
			let joined = args
				.iter()
				.map(|a| fmt_num(*a))
				.collect::<Vec<_>>()
				.join(" ");
			format!("{name}({joined})")
		}
	}
}

fn matrix_str(a: f64, b: f64, c: f64, d: f64, e: f64, f: f64) -> String {
	format!(
		"matrix({} {} {} {} {} {})",
		fmt_num(a),
		fmt_num(b),
		fmt_num(c),
		fmt_num(d),
		fmt_num(e),
		fmt_num(f)
	)
}

/// Apply the affine to path `d`: absolute coords get `s*p + t`, relative deltas get `s*p`; arc
/// rx/ry scale, rotation and flags are preserved.
fn path_data(d: &str, s: f64, tx: f64, ty: f64) -> String {
	use svgtypes::{PathParser, PathSegment};

	// Absolute point -> affine; relative point -> scale-only.
	let pt = |x: f64, y: f64, abs: bool| {
		if abs {
			(s * x + tx, s * y + ty)
		} else {
			(s * x, s * y)
		}
	};

	let mut out = String::new();
	// A leading `m` is absolute.
	let mut first = true;
	for seg in PathParser::from(d) {
		let Ok(seg) = seg else {
			return d.to_string();
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
				let x = if abs { s * x + tx } else { s * x };
				cmd(&mut out, 'H', abs);
				out.push_str(&fmt_num(x));
			}
			PathSegment::VerticalLineTo { abs, y } => {
				let y = if abs { s * y + ty } else { s * y };
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
				out.push_str(&nums(&[rx * s, ry * s, x_axis_rotation]));
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
	out.trim_end().to_string()
}

fn cmd(out: &mut String, upper: char, abs: bool) {
	out.push(if abs {
		upper
	} else {
		upper.to_ascii_lowercase()
	});
	out.push(' ');
}

fn two(a: f64, b: f64) -> String {
	nums(&[a, b])
}

/// Space-separated `fmt_num` of each value.
fn nums(vs: &[f64]) -> String {
	vs.iter().map(|v| fmt_num(*v)).collect::<Vec<_>>().join(" ")
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
