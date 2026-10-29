// src/svg_circle.rs

//! Write a `<path>` made of circles as a `<circle>`, or as a path of arcs.

use std::f64::consts::{PI, TAU};
use std::mem;

use anyhow::{Context as _, Result, anyhow};
use quick_xml::events::{BytesStart, Event};
use quick_xml::reader::Reader;
use quick_xml::writer::Writer;
use svgtypes::{SimplePathSegment, SimplifyingPathParser};

use crate::numeric::fmt_num;
use crate::round::parse_root_box;
use crate::svg_id::is_named;

/// Largest distance from the circle, as a fraction of the longest canvas side.
///
/// 0.002 is two pixels at 1024 pixels.
const TOLERANCE: f64 = 0.002;

/// Points taken along each segment of the path.
const SAMPLES: usize = 16;

/// Notches per canvas side: 16 gives 64, 128 and 512 on a 1024 canvas.
const NOTCHES: f64 = 16.0;

/// The canvas, which decides the notches and the tolerance.
#[derive(Debug, Clone, Copy)]
struct Canvas {
	/// Left edge.
	min_x: f64,
	/// Top edge.
	min_y: f64,
	/// Width in user units.
	width: f64,
	/// Height in user units.
	height: f64,
	/// Largest distance from a circle, in user units.
	tolerance: f64,
}

impl Canvas {
	/// Canvas from a `(min_x, min_y, width, height)` box.
	fn new((min_x, min_y, width, height): (f64, f64, f64, f64)) -> Self {
		return Self {
			min_x,
			min_y,
			width,
			height,
			tolerance: width.abs().max(height.abs()) * TOLERANCE,
		};
	}

	/// `value` moved to the nearest notch of `origin + k * side / NOTCHES`, when it is that close.
	fn snap(&self, value: f64, origin: f64, side: f64) -> f64 {
		let step = side.abs() / NOTCHES;
		if step <= 0.0 {
			return value;
		}
		let notch = origin + ((value - origin) / step).round() * step;
		if (value - notch).abs() <= self.tolerance {
			return notch;
		}
		return value;
	}
}

/// One subpath, fitted.
#[derive(Debug, Clone, Copy)]
struct Circle {
	/// Center x.
	cx: f64,
	/// Center y.
	cy: f64,
	/// Radius.
	radius: f64,
	/// The direction on screen, which decides a hole under `nonzero`.
	clockwise: bool,
}

/// Replace every `<path>` of `src` that is made of circles.
///
/// `dynamic` says the document has a `<style>` or a `<script>`, which can select a `path` by its
/// name. The document then stays as it is.
pub fn to_circles(src: &str, dynamic: bool) -> Result<String> {
	if dynamic {
		return Ok(src.to_owned());
	}
	let Ok(root) = parse_root_box(src) else {
		return Ok(src.to_owned());
	};
	let canvas = Canvas::new(root);

	let mut reader = Reader::from_str(src);
	let mut writer = Writer::new(Vec::new());
	// One entry per open element: whether it or a parent draws an outline.
	let mut outlined = vec![false];

	loop {
		match reader.read_event().context("parsing SVG")? {
			Event::Eof => break,
			Event::Start(elem) => {
				let parent = *outlined.last().unwrap_or(&false);
				outlined.push(parent || has_outline(&elem)?);
				writer.write_event(Event::Start(elem))?;
			}
			Event::End(elem) => {
				outlined.pop();
				writer.write_event(Event::End(elem))?;
			}
			Event::Empty(elem) => {
				let parent = *outlined.last().unwrap_or(&false);
				let circles = if elem.local_name().into_inner() == "path" && !parent {
					rewritten(&elem, src, canvas)?
				} else {
					None
				};
				writer.write_event(Event::Empty(circles.unwrap_or(elem)))?;
			}
			other => writer.write_event(other)?,
		}
	}

	return Ok(String::from_utf8(writer.into_inner())?);
}

/// Whether `elem` sets a stroke or a marker, which follow the shape of the path.
fn has_outline(elem: &BytesStart) -> Result<bool> {
	for attr in elem.attributes() {
		let attr = attr.map_err(|err| anyhow!("attribute: {err}"))?;
		let key = attr.key.into_inner();
		let val = attr.normalized_value(quick_xml::XmlVersion::Implicit1_0)?;
		let outline = match key {
			"stroke" => val.trim() != "none",
			"style" => val.contains("stroke") || val.contains("marker"),
			_ => key.starts_with("marker"),
		};
		if outline {
			return Ok(true);
		}
	}
	return Ok(false);
}

/// `elem` rewritten from its circles, or `None` when it is not made of circles.
///
/// One circle becomes a `<circle>`. More circles stay a `<path>`, with arcs in place of `d`.
fn rewritten(elem: &BytesStart, src: &str, canvas: Canvas) -> Result<Option<BytesStart<'static>>> {
	let mut path = None;
	for attr in elem.attributes() {
		let attr = attr.map_err(|err| anyhow!("attribute: {err}"))?;
		let val = attr.normalized_value(quick_xml::XmlVersion::Implicit1_0)?;
		match attr.key.into_inner() {
			"d" => path = Some(val.into_owned()),
			// A length along the path depends on where it starts.
			"pathLength" => return Ok(None),
			// A `<textPath>` or an `<mpath>` follows the path from its start.
			"id" if is_named(src, &val)? => return Ok(None),
			_ => {}
		}
	}
	if has_outline(elem)? {
		return Ok(None);
	}
	let Some(circles) = path.as_deref().and_then(|data| return fit(data, canvas)) else {
		return Ok(None);
	};

	let mut out = BytesStart::new(if circles.len() == 1 { "circle" } else { "path" });
	for attr in elem.attributes() {
		let attr = attr.map_err(|err| anyhow!("attribute: {err}"))?;
		let key = attr.key.into_inner();
		if key != "d" {
			let val = attr.normalized_value(quick_xml::XmlVersion::Implicit1_0)?;
			out.push_attribute((key, val.as_ref()));
		} else if let [circle] = circles[..] {
			out.push_attribute(("cx", fmt_num(circle.cx).as_str()));
			out.push_attribute(("cy", fmt_num(circle.cy).as_str()));
			out.push_attribute(("r", fmt_num(circle.radius).as_str()));
		} else {
			out.push_attribute(("d", arcs(&circles).as_str()));
		}
	}
	return Ok(Some(out));
}

/// Each circle as two half-circle arcs, from its top and in its own direction.
fn arcs(circles: &[Circle]) -> String {
	use std::fmt::Write as _;

	let mut path = String::new();
	for circle in circles {
		let (x, radius) = (fmt_num(circle.cx), fmt_num(circle.radius));
		let top = fmt_num(circle.cy - circle.radius);
		let bottom = fmt_num(circle.cy + circle.radius);
		let sweep = u8::from(circle.clockwise);
		// A `String` never fails to take a write.
		let _ = write!(
			path,
			"M{x} {top}A{radius} {radius} 0 0 {sweep} {x} {bottom}A{radius} {radius} 0 0 {sweep} {x} {top}Z"
		);
	}
	return path;
}

/// The circles that `path` draws on `canvas`, one per subpath, if every subpath is one.
fn fit(path: &str, canvas: Canvas) -> Option<Vec<Circle>> {
	return sample(path)?
		.iter()
		.map(|points| return fit_one(points, canvas))
		.collect();
}

/// The circle that one subpath draws on `canvas`, if any, snapped to its notches when it can be.
fn fit_one(points: &[(f64, f64)], canvas: Canvas) -> Option<Circle> {
	let (cx, cy, radius) = least_squares(points)?;
	if radius < canvas.tolerance * 10.0 {
		return None;
	}
	// One turn: a spiral or an arc that comes back fails here.
	let turns = winding(points, cx, cy) / TAU;
	if (turns.abs() - 1.0).abs() >= 0.01 {
		return None;
	}

	let (sx, sy) = (
		canvas.snap(cx, canvas.min_x, canvas.width),
		canvas.snap(cy, canvas.min_y, canvas.height),
	);
	let sr = canvas.snap(radius, 0.0, canvas.width.abs().min(canvas.height.abs()));
	let near = |(cx, cy, radius): (f64, f64, f64)| {
		return points
			.iter()
			.all(|&(x, y)| return ((x - cx).hypot(y - cy) - radius).abs() <= canvas.tolerance);
	};
	// The most snapped circle that the path stays close to.
	let (cx, cy, radius) = [(sx, sy, sr), (sx, sy, radius), (cx, cy, radius)]
		.into_iter()
		.find(|&candidate| return near(candidate))?;
	// The y axis points down, so a positive turn is clockwise on screen.
	return Some(Circle {
		cx,
		cy,
		radius,
		clockwise: turns > 0.0,
	});
}

/// Points along each subpath of `path`, each ending where it starts.
fn sample(path: &str) -> Option<Vec<Vec<(f64, f64)>>> {
	let mut subpaths: Vec<Vec<(f64, f64)>> = Vec::new();
	let mut points: Vec<(f64, f64)> = Vec::new();
	let mut start = (0.0, 0.0);
	let mut cur = (0.0, 0.0);

	// A subpath is filled as if closed, so an open one closes here too.
	let mut close = |points: &mut Vec<(f64, f64)>, cur: (f64, f64), start: (f64, f64)| {
		if points.len() > 1 {
			points.extend(along(|ratio| return lerp(cur, start, ratio)));
			subpaths.push(mem::take(points));
		}
		points.clear();
	};

	for segment in SimplifyingPathParser::from(path) {
		let segment = segment.ok()?;
		if points.is_empty() && !matches!(segment, SimplePathSegment::MoveTo { .. }) {
			// A segment after `Z` starts where the subpath started.
			points.push(cur);
			start = cur;
		}
		match segment {
			SimplePathSegment::MoveTo { x, y } => {
				close(&mut points, cur, start);
				start = (x, y);
				points.push(start);
				cur = start;
			}
			SimplePathSegment::LineTo { x, y } => {
				points.extend(along(|ratio| return lerp(cur, (x, y), ratio)));
				cur = (x, y);
			}
			SimplePathSegment::Quadratic { x1, y1, x, y } => {
				let (control, end) = ((x1, y1), (x, y));
				points.extend(along(|ratio| {
					return lerp(lerp(cur, control, ratio), lerp(control, end, ratio), ratio);
				}));
				cur = end;
			}
			SimplePathSegment::CurveTo {
				x1,
				y1,
				x2,
				y2,
				x,
				y,
			} => {
				let (c1, c2, end) = ((x1, y1), (x2, y2), (x, y));
				points.extend(along(|ratio| {
					let first = lerp(cur, c1, ratio);
					let second = lerp(c1, c2, ratio);
					let third = lerp(c2, end, ratio);
					return lerp(
						lerp(first, second, ratio),
						lerp(second, third, ratio),
						ratio,
					);
				}));
				cur = end;
			}
			SimplePathSegment::ClosePath => {
				close(&mut points, cur, start);
				cur = start;
			}
		}
	}
	close(&mut points, cur, start);
	return (!subpaths.is_empty()).then_some(subpaths);
}

/// `SAMPLES` points of one segment, without its start.
#[expect(clippy::cast_precision_loss, reason = "sample counts are small")]
fn along(at: impl Fn(f64) -> (f64, f64)) -> impl Iterator<Item = (f64, f64)> {
	return (1..=SAMPLES).map(move |i| return at(i as f64 / SAMPLES as f64));
}

/// Linear interpolation between two points.
fn lerp(from: (f64, f64), to: (f64, f64), ratio: f64) -> (f64, f64) {
	return (
		from.0 + (to.0 - from.0) * ratio,
		from.1 + (to.1 - from.1) * ratio,
	);
}

/// Kasa fit: the circle `x² + y² + Dx + Ey + F = 0` closest to the points.
#[expect(clippy::cast_precision_loss, reason = "point counts are small")]
fn least_squares(points: &[(f64, f64)]) -> Option<(f64, f64, f64)> {
	// Shift to the mean, which keeps the sums small and the solve stable.
	let n = points.len() as f64;
	let mx = points.iter().map(|point| return point.0).sum::<f64>() / n;
	let my = points.iter().map(|point| return point.1).sum::<f64>() / n;

	let (mut sxx, mut sxy, mut syy, mut sxz, mut syz) = (0.0, 0.0, 0.0, 0.0, 0.0);
	let mut sz = 0.0;
	for &(x, y) in points {
		let (x, y) = (x - mx, y - my);
		let z = x * x + y * y;
		sxx += x * x;
		sxy += x * y;
		syy += y * y;
		sxz += x * z;
		syz += y * z;
		sz += z;
	}
	// With centred points, D and E solve a 2x2 system, and F is the mean of -z.
	let det = sxx * syy - sxy * sxy;
	if det.abs() < f64::EPSILON {
		return None;
	}
	let coef_d = -(sxz * syy - syz * sxy) / det;
	let coef_e = -(sxx * syz - sxy * sxz) / det;
	let coef_f = -sz / n;
	let r2 = (coef_d * coef_d + coef_e * coef_e) / 4.0 - coef_f;
	return (r2 > 0.0).then(|| return (mx - coef_d / 2.0, my - coef_e / 2.0, r2.sqrt()));
}

/// Total angle the points turn around `(cx, cy)`.
fn winding(points: &[(f64, f64)], cx: f64, cy: f64) -> f64 {
	let angles: Vec<f64> = points
		.iter()
		.map(|&(x, y)| return (y - cy).atan2(x - cx))
		.collect();
	return angles
		.windows(2)
		.map(|w| {
			let step = w[1] - w[0];
			// The step across the -pi to pi seam.
			if step > PI {
				return step - TAU;
			} else if step < -PI {
				return step + TAU;
			}
			return step;
		})
		.sum();
}

#[cfg(test)]
mod tests {
	use resvg::tiny_skia::Pixmap;

	use super::*;
	use crate::render::{load_tree_from_data, render_size, rmse};

	const ARCS: &str = "M512 0A512 512 0 0 1 1024 512A512 512 0 0 1 512 1024A512 512 0 0 1 0 512A512 512 0 0 1 512 0Z";

	fn svg(body: &str) -> String {
		return format!(
			r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024">{body}</svg>"#
		);
	}

	#[test]
	fn arcs_become_a_circle() {
		let src = svg(&format!(r##"<path fill="#fff" d="{ARCS}"/>"##));
		let out = to_circles(&src, false).unwrap();
		// The arcs are curves, which stray a little, and the notch puts them back.
		assert!(
			out.contains(r##"<circle fill="#fff" cx="512" cy="512" r="512"/>"##),
			"{out}"
		);
	}

	/// The badge of `road-closure.svg`, resized to 1024 and a little off its center.
	#[test]
	fn an_exported_badge_snaps_to_the_canvas() {
		let src = svg(
			r#"<path d="M 512.132518 0.557068 C 794.705318 0.557068 1023.801224 229.593751 1024 512.201788 V 512.936659 C 1023.807247 787.305412 807.869741 1011.178918 536.705506 1024 H 487.559529 C 217.450315 1011.230118 2.139266 789.0432 0.278614 516.135153 V 509.003294 C 2.195763 227.868311 230.626184 0.557276 512.132518 0.557068 Z"/>"#,
		);
		let out = to_circles(&src, false).unwrap();
		assert!(
			out.contains(r#"<circle cx="512" cy="512" r="512"/>"#),
			"{out}"
		);
	}

	/// A circle far from every notch keeps what it measures.
	#[test]
	fn a_circle_off_the_notches_keeps_its_values() {
		let src = svg(r#"<path d="M400 100A300 300 0 0 1 400 700A300 300 0 0 1 400 100Z"/>"#);
		let out = to_circles(&src, false).unwrap();
		let circle = out.split("<circle").nth(1).unwrap();
		for (name, want) in [("cx", 400.0), ("cy", 400.0), ("r", 300.0)] {
			let value: f64 = circle
				.split(&format!(r#" {name}=""#))
				.nth(1)
				.unwrap()
				.split('"')
				.next()
				.unwrap()
				.parse()
				.unwrap();
			assert!((value - want).abs() < 0.05, "{name} {value}: {out}");
		}
	}

	/// The ring of `road-closure.svg`: the outer circle snaps to the canvas, the inner one only
	/// to its center.
	#[test]
	fn a_ring_snaps_to_the_canvas() {
		let ring = "M 511.855435 1024 C 787.456 1024 1012.181835 806.116894 1023.277176 533.157647 V 490.842353 C 1012.181835 217.881901 787.468047 0 511.855435 0 C 236.241619 0 0 229.22752 0 512 C 0 794.771576 229.16608 1024 511.855435 1024 Z M 511.855435 101.411238 C 738.156424 101.411238 922.268612 285.599322 922.268612 512 C 922.268612 738.400376 738.156424 922.587859 511.855435 922.587859 C 285.554146 922.587859 101.440151 738.400376 101.440151 512 C 101.440151 285.599322 285.554146 101.411238 511.855435 101.411238 Z";
		let circles = fit(ring, Canvas::new((0.0, 0.0, 1024.0, 1024.0))).unwrap();
		let [outer, inner] = circles[..] else {
			panic!("two circles expected");
		};
		assert_eq!((outer.cx, outer.cy, outer.radius), (512.0, 512.0, 512.0));
		assert_eq!((inner.cx, inner.cy), (512.0, 512.0));
		assert!((inner.radius - 410.5).abs() < 0.1, "r {}", inner.radius);
	}

	/// The badge of `road-closure.svg`, with a short flat edge at the bottom.
	#[test]
	fn a_nearly_round_path_becomes_a_circle() {
		let src = svg(
			r##"<path fill="#fff" d="M512 1a511.2 511.2 0 0 1 511.1 510.9v.7a511.2 511.2 0 0 1-486.6 510.3h-49.1A511.2 511.2 0 0 1 .9 515.8v-7.1C2.8 228 230.9 1 512 1Z"/>"##,
		);
		let out = to_circles(&src, false).unwrap();
		assert!(out.contains("<circle"), "{out}");
		assert!(!out.contains(" d="), "{out}");
	}

	#[test]
	fn a_square_stays_a_path() {
		let src = svg(r#"<path d="M0 0h1024v1024H0Z"/>"#);
		let out = to_circles(&src, false).unwrap();
		assert_eq!(out, src);
	}

	/// Half a circle is close to the fitted circle, but does not go round it.
	#[test]
	fn a_half_circle_stays_a_path() {
		let src = svg(r#"<path d="M0 512A512 512 0 0 1 1024 512Z"/>"#);
		let out = to_circles(&src, false).unwrap();
		assert_eq!(out, src);
	}

	/// The ring of `road-closure.svg`: two circles, with flat edges where it was cut.
	const RING: &str = "M511.7 1022.9a511.2 511.2 0 0 0 510.6-490.1v-42.3A511.1 511.1 0 0 0 511.7.4C236.5.4.6 229.3.6 511.7s228.8 511.2 511.1 511.2ZM511.7 101.7c226 0 409.8 183.9 409.8 410s-183.8 410-409.8 410-409.8-183.9-409.8-410 183.8-410 409.8-410Z";

	/// Render one `path` filled black, with `rule` as the fill rule.
	fn render(path: &str, rule: &str) -> Pixmap {
		let src = svg(&format!(r#"<path fill-rule="{rule}" d="{path}"/>"#));
		let tree = load_tree_from_data(src.as_bytes()).unwrap();
		return render_size(&tree, 256, 256).unwrap();
	}

	/// The outer circle runs one way and the inner one the other way, which makes the hole.
	#[test]
	fn a_ring_becomes_arcs_with_its_hole() {
		let src = svg(&format!(r##"<path fill="#ffac7c" d="{RING}"/>"##));
		let out = to_circles(&src, false).unwrap();
		assert!(out.contains("<path"), "{out}");
		let path = out
			.split(r#"d=""#)
			.nth(1)
			.unwrap()
			.split('"')
			.next()
			.unwrap();
		assert_eq!(path.matches('A').count(), 4, "{path}");

		for rule in ["nonzero", "evenodd"] {
			let diff = rmse(&render(RING, rule), &render(path, rule)).unwrap();
			assert!(diff < 0.02, "{rule}: {diff}");
		}
	}

	/// Two circles that run the same way fill the hole under `nonzero`, and must still do so.
	#[test]
	fn the_direction_of_each_circle_is_kept() {
		let same = "M512 0A512 512 0 0 1 512 1024A512 512 0 0 1 512 0ZM512 100A412 412 0 0 1 512 924A412 412 0 0 1 512 100Z";
		let src = svg(&format!(r#"<path d="{same}"/>"#));
		let out = to_circles(&src, false).unwrap();
		let path = out
			.split(r#"d=""#)
			.nth(1)
			.unwrap()
			.split('"')
			.next()
			.unwrap();
		let diff = rmse(&render(same, "nonzero"), &render(path, "nonzero")).unwrap();
		assert!(diff < 0.02, "{diff}");
	}

	/// One subpath that is not a circle keeps the whole path.
	#[test]
	fn a_circle_with_a_square_stays_a_path() {
		let src = svg(&format!(r#"<path d="{ARCS}M400 400h200v200H400Z"/>"#));
		let out = to_circles(&src, false).unwrap();
		assert_eq!(out, src);
	}

	/// A stroke starts where the path starts, and a dash pattern shows it.
	#[test]
	fn a_stroke_keeps_the_path() {
		let own = svg(&format!(r#"<path stroke="red" d="{ARCS}"/>"#));
		assert_eq!(to_circles(&own, false).unwrap(), own);

		let inherited = svg(&format!(r#"<g stroke="red"><path d="{ARCS}"/></g>"#));
		assert_eq!(to_circles(&inherited, false).unwrap(), inherited);
	}

	#[test]
	fn a_referenced_path_stays_a_path() {
		let src = svg(&format!(
			r##"<path id="p" d="{ARCS}"/><text><textPath href="#p">A</textPath></text>"##
		));
		let out = to_circles(&src, false).unwrap();
		assert_eq!(out, src);
	}

	#[test]
	fn a_stylesheet_keeps_the_path() {
		let src = svg(&format!(
			r#"<style>path{{fill:red}}</style><path d="{ARCS}"/>"#
		));
		let out = to_circles(&src, true).unwrap();
		assert_eq!(out, src);
	}
}
