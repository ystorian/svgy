// src/svg_gradient.rs

//! Fold a `gradientTransform` into the coordinates of its gradient.
//!
//! A gradient is defined by points: the two ends of a `<linearGradient>`, the circle of a
//! `<radialGradient>`. Moving those points is the same as transforming the gradient, when the
//! coordinates can express the result.
//!
//! The color of a linear gradient is an affine function of the point, and an affine function stays
//! affine through an affine map. A transformed linear gradient is therefore again a plain linear
//! gradient, and every invertible transform folds into its two ends.
//!
//! A radial gradient also carries a circle. Only a translation, a rotation and a uniform scale keep
//! a circle a circle. A skew or an axis-dependent scale makes an ellipse, which `r` cannot express,
//! and the transform is then kept.
//!
//! This module also resolves the reference that lets one gradient borrow the stops of another.

use std::collections::{HashMap, HashSet};
use std::ops::Range;

use anyhow::{Context as _, Result, anyhow};
use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};
use quick_xml::reader::Reader;
use quick_xml::writer::Writer;

use crate::numeric::{fmt_num, parse_num_list, split_num_unit};

/// How close two matrix entries must be to count as equal.
const EPS: f64 = 1e-9;

/// How many rounds of inlining to run. A chain of references settles one hop per round.
const MAX_ROUNDS: usize = 8;

/// An affine `x' = xx*x + xy*y + x0`, `y' = yx*x + yy*y + y0`.
#[derive(Clone, Copy)]
struct Matrix {
	/// Weight of `x` in `x'`.
	xx: f64,
	/// Weight of `x` in `y'`.
	yx: f64,
	/// Weight of `y` in `x'`.
	xy: f64,
	/// Weight of `y` in `y'`.
	yy: f64,
	/// Offset of `x'`.
	x0: f64,
	/// Offset of `y'`.
	y0: f64,
}

impl Matrix {
	/// The matrix that changes nothing.
	const IDENTITY: Self = Self {
		xx: 1.0,
		yx: 0.0,
		xy: 0.0,
		yy: 1.0,
		x0: 0.0,
		y0: 0.0,
	};

	/// `self` applied after `other`.
	fn then(self, other: Self) -> Self {
		return Self {
			xx: self.xx * other.xx + self.xy * other.yx,
			yx: self.yx * other.xx + self.yy * other.yx,
			xy: self.xx * other.xy + self.xy * other.yy,
			yy: self.yx * other.xy + self.yy * other.yy,
			x0: self.xx * other.x0 + self.xy * other.y0 + self.x0,
			y0: self.yx * other.x0 + self.yy * other.y0 + self.y0,
		};
	}

	/// Map one point.
	fn point(self, x: f64, y: f64) -> (f64, f64) {
		return (
			self.xx * x + self.xy * y + self.x0,
			self.yx * x + self.yy * y + self.y0,
		);
	}

	/// The inverse, or `None` when the matrix flattens the plane onto a line.
	fn invert(self) -> Option<Self> {
		let det = self.xx * self.yy - self.yx * self.xy;
		if det.abs() < EPS {
			return None;
		}
		return Some(Self {
			xx: self.yy / det,
			yx: -self.yx / det,
			xy: -self.xy / det,
			yy: self.xx / det,
			x0: (self.xy * self.y0 - self.yy * self.x0) / det,
			y0: (self.yx * self.x0 - self.xx * self.y0) / det,
		});
	}

	/// The transposed linear part applied to a vector. The translation takes no part, since a
	/// direction has no position.
	fn covector(self, x: f64, y: f64) -> (f64, f64) {
		return (self.xx * x + self.yx * y, self.xy * x + self.yy * y);
	}

	/// The factor a similarity scales every length by, or `None` when it is not a similarity.
	fn similarity_scale(self) -> Option<f64> {
		let rotation = (self.xx - self.yy).abs() < EPS && (self.yx + self.xy).abs() < EPS;
		let reflection = (self.xx + self.yy).abs() < EPS && (self.yx - self.xy).abs() < EPS;
		if !rotation && !reflection {
			return None;
		}
		let scale = self.xx.hypot(self.yx);
		return (scale > EPS).then_some(scale);
	}
}

/// Fold every foldable `gradientTransform` in `src` into its gradient.
pub fn flatten(src: &str) -> Result<String> {
	let mut reader = Reader::from_str(src);
	let mut writer = Writer::new(Vec::new());

	loop {
		match reader.read_event().context("parsing SVG")? {
			Event::Eof => break,
			Event::Start(element) => writer.write_event(Event::Start(fold(&element)?))?,
			Event::Empty(element) => writer.write_event(Event::Empty(fold(&element)?))?,
			other => writer.write_event(other)?,
		}
	}

	return Ok(String::from_utf8(writer.into_inner())?);
}

/// Rewrite one gradient with its transform folded in, or return it unchanged.
fn fold(element: &BytesStart) -> Result<BytesStart<'static>> {
	let local = element.local_name().into_inner().to_owned();
	let radial = match local.as_str() {
		"linearGradient" => false,
		"radialGradient" => true,
		_ => return Ok(copy(element)),
	};

	let Some(matrix) =
		attr(element, "gradientTransform")?.and_then(|value| return parse_transform(&value))
	else {
		return Ok(copy(element));
	};

	// Percentages resolve against a viewport the fold cannot see, and an absent coordinate is a
	// percentage by default, so every coordinate has to be a plain number.
	let names: &[&str] = if radial {
		&["cx", "cy", "r"]
	} else {
		&["x1", "y1", "x2", "y2"]
	};
	let mut values = Vec::new();
	for name in names {
		match attr(element, name)?.as_deref().and_then(number) {
			Some(value) => values.push(value),
			None => return Ok(copy(element)),
		}
	}
	let mut folded: Vec<(&str, String)> = Vec::new();
	if radial {
		let Some(scale) = matrix.similarity_scale() else {
			return Ok(copy(element));
		};
		// `fx`/`fy` default to the center, which the fold moves with it.
		let focus = match (attr(element, "fx")?, attr(element, "fy")?) {
			(None, None) => None,
			(fx, fy) => match (
				fx.as_deref().and_then(number),
				fy.as_deref().and_then(number),
			) {
				(Some(fx), Some(fy)) => Some((fx, fy)),
				_ => return Ok(copy(element)),
			},
		};

		let (cx, cy) = matrix.point(values[0], values[1]);
		folded.push(("cx", fmt_num(cx)));
		folded.push(("cy", fmt_num(cy)));
		folded.push(("r", fmt_num(values[2] * scale)));
		if let Some((fx, fy)) = focus {
			let (fx, fy) = matrix.point(fx, fy);
			folded.push(("fx", fmt_num(fx)));
			folded.push(("fy", fmt_num(fy)));
		}
	} else {
		let ends = linear_ends(matrix, (values[0], values[1]), (values[2], values[3]));
		let Some(((x1, y1), (x2, y2))) = ends else {
			return Ok(copy(element));
		};
		folded.push(("x1", fmt_num(x1)));
		folded.push(("y1", fmt_num(y1)));
		folded.push(("x2", fmt_num(x2)));
		folded.push(("y2", fmt_num(y2)));
	}

	let mut out = BytesStart::new(element.name().into_inner().to_owned());
	for attribute in element.attributes() {
		let attribute = attribute.map_err(|err| anyhow!("attribute: {err}"))?;
		let key = attribute.key.into_inner();
		if key == "gradientTransform" {
			continue;
		}
		match folded.iter().find(|(name, _)| return *name == key) {
			Some((_, val)) => out.push_attribute((key, val.as_str())),
			None => out.push_attribute(attribute),
		}
	}
	return Ok(out);
}

/// The two ends a linear gradient has after `matrix` is folded into it, `None` when the fold is not
/// possible.
///
/// The color of a point `p` is `p * w + constant`, where `w` points along the gradient and its
/// length is one full color band. The transform moves that direction with its transposed inverse.
/// The first end keeps its own mapped position, since the transform maps a point to the point that
/// carries the same color.
fn linear_ends(matrix: Matrix, p1: (f64, f64), p2: (f64, f64)) -> Option<((f64, f64), (f64, f64))> {
	let inverse = matrix.invert()?;
	let delta = (p2.0 - p1.0, p2.1 - p1.1);
	let square = delta.0 * delta.0 + delta.1 * delta.1;
	if square == 0.0 {
		return None;
	}

	let (wx, wy) = inverse.covector(delta.0 / square, delta.1 / square);
	let w_square = wx * wx + wy * wy;
	if w_square == 0.0 {
		return None;
	}

	let start = matrix.point(p1.0, p1.1);
	let end = (start.0 + wx / w_square, start.1 + wy / w_square);
	// A near-degenerate transform gives numbers too large to write, and the transform then stays.
	let finite = [start.0, start.1, end.0, end.1]
		.iter()
		.all(|n| return n.is_finite());
	return finite.then_some((start, end));
}

/// The coordinates that only a `<linearGradient>` reads.
const LINEAR_GEOMETRY: &[&str] = &["x1", "y1", "x2", "y2"];

/// The coordinates that only a `<radialGradient>` reads.
const RADIAL_GEOMETRY: &[&str] = &["cx", "cy", "r", "fx", "fy", "fr"];

/// A gradient, and what it lends or borrows.
struct Gradient {
	/// Its own `id`.
	id: String,
	/// The `id` that its own `href` names.
	borrows: Option<String>,
	/// Whether it has `<stop>` children.
	has_stops: bool,
	/// Where its children are in the source.
	body: Range<usize>,
	/// Its start tag, to read the attributes from.
	tag: BytesStart<'static>,
}

/// Write a gradient that only lends its stops into the gradient that borrows them.
///
/// A gradient inherits the stops and every attribute of the gradient its `href` names, so the pair
/// is one gradient written twice. The pair becomes one element, which saves the two tags and the
/// reference.
///
/// The move is only safe when nothing else names the lender: a second gradient would need the stops
/// copied, and a `fill` would lose its paint. A `<style>` or a `<script>` names an `id` in a way that
/// reading the attributes cannot follow, and such a document is left alone.
///
/// The two gradients need not be of the same kind, which editors often write. A gradient only
/// inherits the attributes that its own kind reads, so the move drops the coordinates of the other
/// kind, which the borrower would ignore anyway.
pub fn inline(src: &str, dynamic: bool) -> Result<String> {
	if dynamic {
		return Ok(src.to_owned());
	}
	let mut out: Option<String> = None;
	for _ in 0..MAX_ROUNDS {
		match inline_once(out.as_deref().unwrap_or(src))? {
			Some(next) => out = Some(next),
			None => break,
		}
	}
	return Ok(out.unwrap_or_else(|| return src.to_owned()));
}

/// One round: every pair that stands on its own is written as one gradient. `None` when there is no
/// such pair.
fn inline_once(src: &str) -> Result<Option<String>> {
	let (gradients, refs) = scan(src)?;
	let mut pairs: Vec<(usize, usize)> = Vec::new();

	for (lender, gradient) in gradients.iter().enumerate() {
		// A lender holds the stops itself, and it holds them for one gradient alone.
		if gradient.borrows.is_some() || !gradient.has_stops || refs.get(&gradient.id) != Some(&1) {
			continue;
		}
		let mut borrowers = gradients.iter().enumerate().filter(|(_, candidate)| {
			return candidate.borrows.as_deref() == Some(gradient.id.as_str());
		});
		let Some((borrower, borrowing)) = borrowers.next() else {
			continue;
		};
		// A borrower that holds stops of its own borrows nothing.
		if borrowers.next().is_none() && !borrowing.has_stops && borrower != lender {
			pairs.push((lender, borrower));
		}
	}

	if pairs.is_empty() {
		return Ok(None);
	}
	return Ok(Some(rewrite(src, &gradients, &pairs)?));
}

/// Every gradient in `src`, and how many times each `id` is named.
fn scan(src: &str) -> Result<(Vec<Gradient>, HashMap<String, usize>)> {
	let mut reader = Reader::from_str(src);
	let mut gradients: Vec<Gradient> = Vec::new();
	let mut refs: HashMap<String, usize> = HashMap::new();
	// The gradient the events belong to, and the depth its own tag sits at.
	let mut open: Option<(usize, usize)> = None;
	let mut depth: usize = 0;

	loop {
		let before = usize::try_from(reader.buffer_position())?;
		let event = reader.read_event().context("parsing SVG")?;
		let after = usize::try_from(reader.buffer_position())?;

		let (element, opens) = match event {
			Event::Eof => break,
			Event::Start(element) => (element, true),
			Event::Empty(element) => (element, false),
			Event::End(_) => {
				if let Some((index, at)) = open
					&& at == depth
				{
					gradients[index].body.end = before;
					open = None;
				}
				depth = depth.saturating_sub(1);
				continue;
			}
			_ => continue,
		};

		// An empty element opens and closes at once, and it sits as deep as one that opens.
		let own = depth + 1;
		if opens {
			depth = own;
		}
		add_refs(&element, &mut refs)?;
		if let Some((index, at)) = open
			&& at + 1 == own
			&& element.local_name().into_inner() == "stop"
		{
			gradients[index].has_stops = true;
		}

		if !matches!(
			element.local_name().into_inner(),
			"linearGradient" | "radialGradient"
		) {
			continue;
		}
		let Some(id) = attr(&element, "id")? else {
			continue;
		};
		gradients.push(Gradient {
			id,
			borrows: reference(&element)?,
			has_stops: false,
			body: after..after,
			tag: element.to_owned().into_owned(),
		});
		if opens {
			open = Some((gradients.len() - 1, own));
		}
	}

	return Ok((gradients, refs));
}

/// Count every `id` that the attributes of `element` name.
fn add_refs(element: &BytesStart, refs: &mut HashMap<String, usize>) -> Result<()> {
	for attribute in element.attributes() {
		let attribute = attribute.map_err(|err| anyhow!("attribute: {err}"))?;
		let value = attribute
			.normalized_value(quick_xml::XmlVersion::Implicit1_0)?
			.into_owned();
		if is_href(attribute.key.into_inner())
			&& let Some(id) = value.strip_prefix('#')
		{
			*refs.entry(id.to_owned()).or_default() += 1;
			continue;
		}
		// A paint names its gradient as `url(#id)`, and a name given twice counts twice.
		let mut rest = value.as_str();
		while let Some((_, after)) = rest.split_once("url(") {
			let Some((inner, tail)) = after.split_once(')') else {
				break;
			};
			let named = inner.trim().trim_matches(['"', '\'']);
			if let Some(id) = named.strip_prefix('#') {
				*refs.entry(id.to_owned()).or_default() += 1;
			}
			rest = tail;
		}
	}
	return Ok(());
}

/// The `id` that the `href` of `element` names.
fn reference(element: &BytesStart) -> Result<Option<String>> {
	for key in ["href", "xlink:href"] {
		if let Some(value) = attr(element, key)?
			&& let Some(id) = value.strip_prefix('#')
		{
			return Ok(Some(id.to_owned()));
		}
	}
	return Ok(None);
}

/// Whether the attribute key is an `href`.
fn is_href(key: &str) -> bool {
	return key == "href" || key == "xlink:href";
}

/// Write `src` with every pair written as one gradient.
fn rewrite(src: &str, gradients: &[Gradient], pairs: &[(usize, usize)]) -> Result<String> {
	let lenders: HashSet<&str> = pairs
		.iter()
		.map(|&(lender, _)| return gradients[lender].id.as_str())
		.collect();
	let borrowers: HashMap<&str, usize> = pairs
		.iter()
		.map(|&(lender, borrower)| return (gradients[borrower].id.as_str(), lender))
		.collect();

	let mut reader = Reader::from_str(src);
	let mut writer = Writer::new(Vec::new());
	// The depth of the lender whose events are being dropped.
	let mut skip: Option<usize> = None;
	let mut depth: usize = 0;

	loop {
		match reader.read_event().context("parsing SVG")? {
			Event::Eof => break,
			Event::End(element) => {
				if skip == Some(depth) {
					skip = None;
					depth = depth.saturating_sub(1);
					continue;
				}
				depth = depth.saturating_sub(1);
				if skip.is_none() {
					writer.write_event(Event::End(element))?;
				}
			}
			Event::Start(element) => {
				depth += 1;
				if skip.is_some() {
					continue;
				}
				let id = attr(&element, "id")?.unwrap_or_default();
				if lenders.contains(id.as_str()) {
					skip = Some(depth);
					continue;
				}
				match borrowers.get(id.as_str()) {
					// The stops go in first, and the own children of the borrower follow.
					Some(&lender) => {
						let gradient = &gradients[lender];
						writer.write_event(Event::Start(merged(&element, &gradient.tag)?))?;
						writer.write_event(body(src, gradient))?;
					}
					None => writer.write_event(Event::Start(element))?,
				}
			}
			Event::Empty(element) => {
				if skip.is_some() {
					continue;
				}
				let id = attr(&element, "id")?.unwrap_or_default();
				// A lender holds stops, so it is never an empty element.
				match borrowers.get(id.as_str()) {
					Some(&lender) => {
						let gradient = &gradients[lender];
						let start = merged(&element, &gradient.tag)?;
						let name = start.name().into_inner().to_owned();
						writer.write_event(Event::Start(start))?;
						writer.write_event(body(src, gradient))?;
						writer.write_event(Event::End(BytesEnd::new(name)))?;
					}
					None => writer.write_event(Event::Empty(element))?,
				}
			}
			other => {
				if skip.is_none() {
					writer.write_event(other)?;
				}
			}
		}
	}

	return Ok(String::from_utf8(writer.into_inner())?);
}

/// The children of `gradient`, verbatim.
#[expect(clippy::string_slice, reason = "parser offsets are char boundaries")]
fn body<'a>(src: &'a str, gradient: &Gradient) -> Event<'a> {
	return Event::Text(BytesText::from_escaped(&src[gradient.body.clone()]));
}

/// The borrower, with the attributes it inherits written out and its reference gone.
fn merged(borrower: &BytesStart, lender: &BytesStart) -> Result<BytesStart<'static>> {
	let mut out = BytesStart::new(borrower.name().into_inner().to_owned());
	let mut own: Vec<String> = Vec::new();
	for attribute in borrower.attributes() {
		let attribute = attribute.map_err(|err| anyhow!("attribute: {err}"))?;
		if is_href(attribute.key.into_inner()) {
			continue;
		}
		own.push(attribute.key.into_inner().to_owned());
		out.push_attribute(attribute);
	}
	// A coordinate of the other kind of gradient is one the borrower does not read.
	let other = match borrower.local_name().into_inner() {
		"radialGradient" => LINEAR_GEOMETRY,
		_ => RADIAL_GEOMETRY,
	};
	// An attribute of the borrower beats the one it would inherit, and the `id` of the lender goes.
	for attribute in lender.attributes() {
		let attribute = attribute.map_err(|err| anyhow!("attribute: {err}"))?;
		let key = attribute.key.into_inner();
		if is_href(key)
			|| key == "id"
			|| own.iter().any(|name| return name == key)
			|| other.contains(&key)
		{
			continue;
		}
		out.push_attribute(attribute);
	}
	return Ok(out);
}

/// An owned copy of the element.
fn copy(element: &BytesStart) -> BytesStart<'static> {
	return element.to_owned().into_owned();
}

/// A length that is a plain number, so the fold can move it.
fn number(value: &str) -> Option<f64> {
	match split_num_unit(value) {
		Some((n, "")) => return Some(n),
		_ => return None,
	}
}

/// First matching attribute value, unescaped.
fn attr(element: &BytesStart, key: &str) -> Result<Option<String>> {
	for attribute in element.attributes() {
		let attribute = attribute.map_err(|err| anyhow!("attribute: {err}"))?;
		if attribute.key.as_ref() == key {
			return Ok(Some(
				attribute
					.normalized_value(quick_xml::XmlVersion::Implicit1_0)?
					.into_owned(),
			));
		}
	}
	return Ok(None);
}

/// Compose a `transform` list into one matrix. Anything unreadable gives `None`, which keeps the
/// attribute as it is.
fn parse_transform(value: &str) -> Option<Matrix> {
	let mut out = Matrix::IDENTITY;
	let mut rest = value;
	loop {
		let trimmed = rest.trim_start_matches([' ', ',', '\t', '\n', '\r']);
		if trimmed.is_empty() {
			return Some(out);
		}
		let (name, after) = trimmed.split_once('(')?;
		let (inner, tail) = after.split_once(')')?;
		let args = parse_num_list(inner);
		out = out.then(primitive(name.trim(), &args)?);
		rest = tail;
	}
}

/// Matrix of one transform primitive.
fn primitive(name: &str, args: &[f64]) -> Option<Matrix> {
	let arg = |i: usize, default: f64| return args.get(i).copied().unwrap_or(default);
	let matrix = |xx, yx, xy, yy, x0, y0| {
		return Some(Matrix {
			xx,
			yx,
			xy,
			yy,
			x0,
			y0,
		});
	};
	match name {
		"translate" => return matrix(1.0, 0.0, 0.0, 1.0, arg(0, 0.0), arg(1, 0.0)),
		"scale" => {
			let sx = arg(0, 1.0);
			return matrix(sx, 0.0, 0.0, arg(1, sx), 0.0, 0.0);
		}
		"rotate" => {
			let (rad, cx, cy) = (arg(0, 0.0).to_radians(), arg(1, 0.0), arg(2, 0.0));
			let (sin, cos) = rad.sin_cos();
			// About a center: move it to the origin, rotate, move it back.
			return matrix(
				cos,
				sin,
				-sin,
				cos,
				cx - cx * cos + cy * sin,
				cy - cx * sin - cy * cos,
			);
		}
		"skewX" => return matrix(1.0, 0.0, arg(0, 0.0).to_radians().tan(), 1.0, 0.0, 0.0),
		"skewY" => return matrix(1.0, arg(0, 0.0).to_radians().tan(), 0.0, 1.0, 0.0, 0.0),
		"matrix" if args.len() == 6 => {
			return matrix(args[0], args[1], args[2], args[3], args[4], args[5]);
		}
		_ => return None,
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::render::{load_tree_from_data, render_size, rmse};

	#[test]
	fn a_translation_moves_the_ends() {
		let src = r#"<svg xmlns="http://www.w3.org/2000/svg"><linearGradient id="a" x1="10" y1="20" x2="30" y2="40" gradientTransform="translate(0 -25)" gradientUnits="userSpaceOnUse"/></svg>"#;
		let out = flatten(src).unwrap();
		assert!(!out.contains("gradientTransform"), "{out}");
		assert!(out.contains(r#"y1="-5""#), "{out}");
		assert!(out.contains(r#"y2="15""#), "{out}");
		assert!(out.contains(r#"x1="10""#), "{out}");
	}

	#[test]
	fn a_uniform_scale_moves_the_circle() {
		let src = r#"<svg xmlns="http://www.w3.org/2000/svg"><radialGradient id="a" cx="10" cy="10" r="4" gradientTransform="scale(2)"/></svg>"#;
		let out = flatten(src).unwrap();
		assert!(!out.contains("gradientTransform"), "{out}");
		assert!(out.contains(r#"cx="20""#), "{out}");
		assert!(out.contains(r#"r="8""#), "{out}");
	}

	/// A rotation about a center keeps the ends at the same distance.
	#[test]
	fn a_rotation_turns_the_ends() {
		let src = r#"<svg xmlns="http://www.w3.org/2000/svg"><linearGradient id="a" x1="0" y1="0" x2="10" y2="0" gradientTransform="rotate(90)"/></svg>"#;
		let out = flatten(src).unwrap();
		assert!(!out.contains("gradientTransform"), "{out}");
		assert!(out.contains(r#"x2="0""#), "{out}");
		assert!(out.contains(r#"y2="10""#), "{out}");
	}

	/// A skew tilts the color bands away from the ends, which the ends can still express.
	#[test]
	fn a_skew_folds_into_a_linear_gradient() {
		let src = r#"<svg xmlns="http://www.w3.org/2000/svg"><linearGradient id="a" x1="0" y1="0" x2="10" y2="0" gradientTransform="skewX(20)"/></svg>"#;
		let out = flatten(src).unwrap();
		assert!(!out.contains("gradientTransform"), "{out}");
		// `skewX(20)` maps the vertical band at `x` to a line that leans by 20 degrees.
		assert!(out.contains(r#"x2="8.830222""#), "{out}");
		assert!(out.contains(r#"y2="-3.213938""#), "{out}");
	}

	/// An axis-dependent scale is also only a change of the two ends.
	#[test]
	fn a_non_uniform_scale_folds_into_a_linear_gradient() {
		let src = r#"<svg xmlns="http://www.w3.org/2000/svg"><linearGradient id="a" x1="0" y1="0" x2="10" y2="0" gradientTransform="scale(2 3)"/></svg>"#;
		let out = flatten(src).unwrap();
		assert!(!out.contains("gradientTransform"), "{out}");
		// The bands stay vertical, and the scale on `x` alone sets their spacing.
		assert!(out.contains(r#"x2="20""#), "{out}");
		assert!(out.contains(r#"y2="0""#), "{out}");
	}

	/// The fold moves the ends, not the colors: a skewed gradient renders the same after it.
	#[test]
	fn a_folded_skew_renders_the_same() {
		let src = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 200 200"><defs><linearGradient id="a" gradientUnits="userSpaceOnUse" x1="20" y1="20" x2="180" y2="20" gradientTransform="matrix(1.3 .4 -.9 2.1 -30 -60)"><stop offset="0" stop-color="#f00"/><stop offset=".5" stop-color="#0f0"/><stop offset="1" stop-color="#00f"/></linearGradient></defs><rect width="200" height="200" fill="url(#a)"/></svg>"##;
		let out = flatten(src).unwrap();
		assert!(!out.contains("gradientTransform"), "{out}");

		let render = |svg: &str| {
			let tree = load_tree_from_data(svg.as_bytes()).unwrap();
			return render_size(&tree, 256, 256).unwrap();
		};
		let difference = rmse(&render(src), &render(&out)).unwrap();
		assert!(difference < 0.001, "difference {difference}");
	}

	/// A transform that flattens the plane leaves no direction to fold into.
	#[test]
	fn a_degenerate_transform_is_kept() {
		let src = r#"<svg xmlns="http://www.w3.org/2000/svg"><linearGradient id="a" x1="0" y1="0" x2="10" y2="0" gradientTransform="scale(0)"/></svg>"#;
		let out = flatten(src).unwrap();
		assert!(out.contains("gradientTransform"), "{out}");
	}

	/// An axis-dependent scale turns the circle into an ellipse, which `r` cannot say.
	#[test]
	fn a_non_uniform_scale_is_kept_on_a_radial_gradient() {
		let src = r#"<svg xmlns="http://www.w3.org/2000/svg"><radialGradient id="a" cx="10" cy="10" r="4" gradientTransform="scale(2 3)"/></svg>"#;
		let out = flatten(src).unwrap();
		assert!(out.contains("gradientTransform"), "{out}");
	}

	/// A skew makes an ellipse too.
	#[test]
	fn a_skew_is_kept_on_a_radial_gradient() {
		let src = r#"<svg xmlns="http://www.w3.org/2000/svg"><radialGradient id="a" cx="10" cy="10" r="4" gradientTransform="skewX(20)"/></svg>"#;
		let out = flatten(src).unwrap();
		assert!(out.contains("gradientTransform"), "{out}");
	}

	/// A percentage resolves against a viewport, so it stays where it is.
	#[test]
	fn a_percentage_coordinate_is_kept() {
		let src = r#"<svg xmlns="http://www.w3.org/2000/svg"><linearGradient id="a" x1="0%" y1="0" x2="100%" y2="0" gradientTransform="translate(5 0)"/></svg>"#;
		let out = flatten(src).unwrap();
		assert!(out.contains("gradientTransform"), "{out}");
	}

	/// A gradient that lends its stops to one gradient alone becomes part of it.
	#[test]
	fn a_lent_gradient_is_written_into_its_borrower() {
		let src = r##"<svg xmlns="http://www.w3.org/2000/svg"><defs><linearGradient id="a"><stop offset="0" stop-color="#f00"/><stop offset="1" stop-color="#00f"/></linearGradient><linearGradient xlink:href="#a" id="b" x1="0" y1="0" x2="10" y2="10" gradientUnits="userSpaceOnUse"/></defs><rect width="10" height="10" fill="url(#b)"/></svg>"##;
		let out = inline(src, false).unwrap();
		// One gradient is left, it keeps the `id` that the paint names, and it holds the stops.
		assert_eq!(out.matches("<linearGradient").count(), 1, "{out}");
		assert!(out.contains(r#"id="b""#), "{out}");
		assert!(out.contains(r##"stop-color="#f00""##), "{out}");
		assert!(!out.contains("href"), "{out}");
	}

	/// The borrower carries the attribute itself, so it wins over the one it would inherit.
	#[test]
	fn the_borrower_keeps_its_own_attributes() {
		let src = r##"<svg xmlns="http://www.w3.org/2000/svg"><defs><linearGradient id="a" x1="1" spreadMethod="pad"><stop offset="0" stop-color="#f00"/></linearGradient><linearGradient href="#a" id="b" x1="7"/></defs><rect fill="url(#b)"/></svg>"##;
		let out = inline(src, false).unwrap();
		assert!(out.contains(r#"x1="7""#), "{out}");
		assert!(!out.contains(r#"x1="1""#), "{out}");
		// What the borrower does not carry, it inherits, and the move writes it out.
		assert!(out.contains(r#"spreadMethod="pad""#), "{out}");
	}

	/// A chain settles one hop per round, from the end that holds the stops.
	#[test]
	fn a_chain_of_references_settles() {
		let src = r##"<svg xmlns="http://www.w3.org/2000/svg"><defs><linearGradient id="a"><stop offset="0" stop-color="#f00"/></linearGradient><linearGradient href="#a" id="b" spreadMethod="pad"/><linearGradient href="#b" id="c" x1="0" y1="0" x2="4" y2="4"/></defs><rect fill="url(#c)"/></svg>"##;
		let out = inline(src, false).unwrap();
		assert_eq!(out.matches("<linearGradient").count(), 1, "{out}");
		assert!(out.contains(r#"id="c""#), "{out}");
		assert!(out.contains(r#"spreadMethod="pad""#), "{out}");
		assert!(out.contains("stop-color"), "{out}");
	}

	/// Two borrowers would each need a copy of the stops, which is not always shorter.
	#[test]
	fn a_gradient_lent_twice_is_kept() {
		let src = r##"<svg xmlns="http://www.w3.org/2000/svg"><defs><linearGradient id="a"><stop offset="0" stop-color="#f00"/></linearGradient><linearGradient href="#a" id="b" x1="0"/><linearGradient href="#a" id="c" x1="4"/></defs><rect fill="url(#b)"/><rect fill="url(#c)"/></svg>"##;
		let out = inline(src, false).unwrap();
		assert_eq!(out.matches("<linearGradient").count(), 3, "{out}");
	}

	/// A paint that names the lender would lose its gradient.
	#[test]
	fn a_gradient_a_paint_names_is_kept() {
		let src = r##"<svg xmlns="http://www.w3.org/2000/svg"><defs><linearGradient id="a"><stop offset="0" stop-color="#f00"/></linearGradient><linearGradient href="#a" id="b" x1="0"/></defs><rect fill="url(#a)"/><rect fill="url(#b)"/></svg>"##;
		let out = inline(src, false).unwrap();
		assert_eq!(out.matches("<linearGradient").count(), 2, "{out}");
	}

	/// A gradient of another kind borrows the stops, and the coordinates it cannot read stay behind.
	#[test]
	fn a_gradient_of_another_kind_borrows_the_stops() {
		let src = r##"<svg xmlns="http://www.w3.org/2000/svg"><defs><linearGradient id="a" x1="0" y1="0" x2="9" y2="9" spreadMethod="pad"><stop offset="0" stop-color="#f00"/></linearGradient><radialGradient href="#a" id="b" cx="5" cy="5" r="5"/></defs><rect fill="url(#b)"/></svg>"##;
		let out = inline(src, false).unwrap();
		assert_eq!(out.matches("Gradient id=").count(), 1, "{out}");
		assert!(out.contains("<radialGradient"), "{out}");
		assert!(out.contains("stop-color"), "{out}");
		assert!(!out.contains("href"), "{out}");
		// A radial gradient reads no `x1`, and the attributes both kinds read come along.
		assert!(!out.contains("x1"), "{out}");
		assert!(out.contains(r#"spreadMethod="pad""#), "{out}");
	}

	/// The other direction: a linear gradient reads no `cx`.
	#[test]
	fn a_linear_gradient_leaves_the_circle_behind() {
		let src = r##"<svg xmlns="http://www.w3.org/2000/svg"><defs><radialGradient id="a" cx="5" cy="5" r="5" fx="5" fy="5"><stop offset="0" stop-color="#f00"/></radialGradient><linearGradient href="#a" id="b" x1="0" y1="0" x2="9" y2="9"/></defs><rect fill="url(#b)"/></svg>"##;
		let out = inline(src, false).unwrap();
		assert_eq!(out.matches("Gradient id=").count(), 1, "{out}");
		assert!(out.contains("stop-color"), "{out}");
		for name in ["cx", "cy", "fx", "fy", " r="] {
			assert!(!out.contains(name), "{name} in {out}");
		}
	}

	/// A stylesheet or a script can name an `id` in a way that the attributes do not show.
	#[test]
	fn a_dynamic_document_is_left_alone() {
		let src = r##"<svg xmlns="http://www.w3.org/2000/svg"><style>rect{fill:url(#b)}</style><defs><linearGradient id="a"><stop offset="0" stop-color="#f00"/></linearGradient><linearGradient href="#a" id="b" x1="0"/></defs><rect/></svg>"##;
		let out = inline(src, true).unwrap();
		assert_eq!(out, src);
	}

	/// The move writes out what the borrower inherited, so the paint stays the same.
	#[test]
	fn an_inlined_gradient_renders_the_same() {
		let src = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 200 200"><defs><linearGradient id="a"><stop offset="0" stop-color="#f00"/><stop offset=".5" stop-color="#0f0"/><stop offset="1" stop-color="#00f"/></linearGradient><linearGradient xlink:href="#a" id="b" gradientUnits="userSpaceOnUse" x1="20" y1="20" x2="180" y2="60" spreadMethod="reflect" xmlns:xlink="http://www.w3.org/1999/xlink"/></defs><rect width="200" height="200" fill="url(#b)"/></svg>"##;
		let out = inline(src, false).unwrap();
		assert_eq!(out.matches("<linearGradient").count(), 1, "{out}");

		let render = |svg: &str| {
			let tree = load_tree_from_data(svg.as_bytes()).unwrap();
			return render_size(&tree, 256, 256).unwrap();
		};
		let difference = rmse(&render(src), &render(&out)).unwrap();
		assert!(difference < 0.001, "difference {difference}");
	}

	/// An absent coordinate is a percentage by default, so it stays too.
	#[test]
	fn a_default_coordinate_is_kept() {
		let src = r#"<svg xmlns="http://www.w3.org/2000/svg"><linearGradient id="a" x1="0" y1="0" gradientTransform="translate(5 0)"/></svg>"#;
		let out = flatten(src).unwrap();
		assert!(out.contains("gradientTransform"), "{out}");
	}
}
