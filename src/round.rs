// src/round.rs

//! `round`: scale + center an SVG's content so its actual drawn shape fits inside the circle
//! inscribed in the viewBox, keeping the viewBox unchanged.
//!
//! The shape's *minimum enclosing circle* is measured by rasterizing and running Welzl's algorithm
//! over the opaque-pixel hull. The uniform scale + translation that maps that circle onto the
//! inscribed circle is then baked into every coordinate (no transforms added; element structure
//! preserved).

use anyhow::{Context as _, Result, anyhow, bail};
use quick_xml::events::{BytesStart, Event};
use quick_xml::reader::Reader;
use quick_xml::writer::Writer;
use resvg::usvg;

use crate::affine;
use crate::cli::DEFAULT_ROUND_PADDING;
use crate::numeric::{parse_num_list, parse_px, split_num_unit};
use crate::render;

/// Pixmap max dimension used for shape measurement.
const MEASURE_PX: f64 = 1024.0;

/// Largest move, as a fraction of the longest canvas side, that leaves the artwork as it is.
///
/// The measure works on pixels, so artwork that already fits comes back a pixel off.
const STILL: f64 = 0.002;

/// Pixmap side used to measure how much of the canvas a shape covers.
const COVERAGE_PX: u32 = 64;

/// Fraction of the canvas a background covers.
///
/// A rounded square tile covers about 95%, and a disc that touches every edge covers 78.5%.
const BACKGROUND_COVERAGE: f64 = 0.9;

/// A rounded document, plus what it took to get there.
pub struct Round {
	/// The rewritten SVG.
	pub svg: String,
	/// The padding fraction actually applied.
	pub padding: f64,
	/// True when `padding` was svgy's choice rather than the caller's, which happens when the
	/// source has a full-canvas background.
	pub auto_padding: bool,
	/// Uniform scale applied to the artwork.
	pub scale: f64,
	/// Radius of the circle the artwork was fitted into.
	pub radius: f64,
}

/// Fit `src`'s shape into the circle inscribed in its viewBox, leaving a `padding` fraction of the
/// radius empty. `None` picks the default: an inset when the source has a full-canvas background,
/// flush otherwise.
pub fn round_str(src: &str, padding: Option<f64>) -> Result<Round> {
	let (min_x, min_y, width, height) = parse_root_box(src)?;

	// Measure only the foreground: strip full-canvas background fills first.
	let (foreground, has_background) = strip_background(src, min_x, min_y, width, height)?;
	let tree = render::load_tree_from_data(foreground.as_bytes())?;
	let (cx, cy, shape_radius) = shape_enclosing_circle(&tree, min_x, min_y, width, height)?;
	if shape_radius <= 1e-9 {
		bail!("content has no measurable extent");
	}

	// Artwork sitting on a background reads better inset from the rim, so that is the default
	// there; artwork alone fills the circle.
	let auto_padding = padding.is_none();
	let padding = padding.unwrap_or(if has_background {
		DEFAULT_ROUND_PADDING
	} else {
		0.0
	});
	if !(0.0..1.0).contains(&padding) {
		bail!("--padding must be in 0.0..1.0");
	}

	let radius = (width.min(height) / 2.0) * (1.0 - padding);
	let (ox, oy) = (min_x + width / 2.0, min_y + height / 2.0);
	let scale = radius / shape_radius;
	let tx = ox - scale * cx;
	let ty = oy - scale * cy;

	// Artwork that already fits keeps its numbers, and the circles that snap to the canvas.
	let moved = [(min_x, min_y), (min_x + width, min_y + height)]
		.iter()
		.flat_map(|&(x, y)| {
			return [
				((scale - 1.0) * x + tx).abs(),
				((scale - 1.0) * y + ty).abs(),
			];
		})
		.fold(0.0, f64::max);
	if moved <= width.abs().max(height.abs()) * STILL {
		return Ok(Round {
			svg: src.to_owned(),
			padding,
			auto_padding,
			scale: 1.0,
			radius,
		});
	}

	return Ok(Round {
		svg: bake_affine(src, scale, tx, ty, (min_x, min_y, width, height))?,
		padding,
		auto_padding,
		scale,
		radius,
	});
}

// --- shape measurement -----------------------------------------------------

/// Minimum enclosing circle of the drawn shape, in userspace coordinates.
// The measurement raster is a few thousand pixels a side: exact in both f32 and f64, so narrowing
// the scale and widening the pixel indices are both faithful.
#[expect(
	clippy::cast_possible_truncation,
	clippy::cast_precision_loss,
	reason = "raster sizes are exact in f32"
)]
fn shape_enclosing_circle(
	tree: &usvg::Tree,
	min_x: f64,
	min_y: f64,
	width: f64,
	height: f64,
) -> Result<(f64, f64, f64)> {
	let size = tree.size();
	let (sw, sh) = (f64::from(size.width()), f64::from(size.height()));
	let zoom = MEASURE_PX / sw.max(sh);
	let pixmap = render::render_scale(tree, zoom as f32)?;
	let (pw, ph) = (pixmap.width() as usize, pixmap.height() as usize);
	let pixels = pixmap.pixels();

	// Per-row min/max opaque x captures every convex-hull vertex of the shape.
	let mut pts: Vec<(f64, f64)> = Vec::new();
	for y in 0..ph {
		let row = y * pw;
		let mut lo: Option<usize> = None;
		let mut hi = 0usize;
		for x in 0..pw {
			if pixels[row + x].alpha() > 0 {
				if lo.is_none() {
					lo = Some(x);
				}
				hi = x;
			}
		}
		if let Some(left) = lo {
			for &dx in &[left, hi] {
				// device -> tree (/zoom) -> userspace (viewBox mapping).
				let ux = min_x + ((dx as f64 + 0.5) / zoom) * (width / sw);
				let uy = min_y + ((y as f64 + 0.5) / zoom) * (height / sh);
				pts.push((ux, uy));
			}
		}
	}

	if pts.is_empty() {
		bail!("no foreground content to fit (only a full-canvas background?)");
	}
	let hull = convex_hull(pts);
	let circle = min_enclosing_circle(&hull);
	return Ok((circle.cx, circle.cy, circle.radius));
}

/// A circle in userspace coordinates.
#[derive(Clone, Copy)]
struct Circle {
	/// Center x.
	cx: f64,
	/// Center y.
	cy: f64,
	/// Radius.
	radius: f64,
}

/// Convex hull of the points.
fn convex_hull(mut points: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
	points.sort_by(|left, right| {
		return left
			.0
			.partial_cmp(&right.0)
			.unwrap()
			.then(left.1.partial_cmp(&right.1).unwrap());
	});
	points.dedup();
	let n = points.len();
	if n <= 2 {
		return points;
	}
	let cross = |origin: (f64, f64), first: (f64, f64), second: (f64, f64)| {
		return (first.0 - origin.0) * (second.1 - origin.1)
			- (first.1 - origin.1) * (second.0 - origin.0);
	};

	let mut lower: Vec<(f64, f64)> = Vec::new();
	for &pt in &points {
		while lower.len() >= 2 && cross(lower[lower.len() - 2], lower[lower.len() - 1], pt) <= 0.0 {
			lower.pop();
		}
		lower.push(pt);
	}
	let mut upper: Vec<(f64, f64)> = Vec::new();
	for &pt in points.iter().rev() {
		while upper.len() >= 2 && cross(upper[upper.len() - 2], upper[upper.len() - 1], pt) <= 0.0 {
			upper.pop();
		}
		upper.push(pt);
	}
	lower.pop();
	upper.pop();
	lower.extend(upper);
	return lower;
}

/// Welzl's minimum enclosing circle over a (small) hull point set.
fn min_enclosing_circle(pts: &[(f64, f64)]) -> Circle {
	return welzl(pts, &mut Vec::new());
}

/// Welzl's minimum enclosing circle, recursive step.
fn welzl(points: &[(f64, f64)], boundary: &mut Vec<(f64, f64)>) -> Circle {
	if points.is_empty() || boundary.len() == 3 {
		return trivial(boundary);
	}
	let idx = points.len() - 1;
	let point = points[idx];
	let circle = welzl(&points[..idx], boundary);
	if in_circle(&circle, point) {
		return circle;
	}
	boundary.push(point);
	let circle = welzl(&points[..idx], boundary);
	boundary.pop();
	return circle;
}

/// Whether the point is inside, with tolerance.
fn in_circle(circle: &Circle, point: (f64, f64)) -> bool {
	let dx = point.0 - circle.cx;
	let dy = point.1 - circle.cy;
	return (dx * dx + dy * dy).sqrt() <= circle.radius + 1e-7;
}

/// Smallest circle through 0 to 3 boundary points.
fn trivial(boundary: &[(f64, f64)]) -> Circle {
	match boundary.len() {
		0 => {
			return Circle {
				cx: 0.0,
				cy: 0.0,
				radius: 0.0,
			};
		}
		1 => {
			return Circle {
				cx: boundary[0].0,
				cy: boundary[0].1,
				radius: 0.0,
			};
		}
		2 => return circle_from_2(boundary[0], boundary[1]),
		_ => {
			if let Some(circle) = circumcircle(boundary[0], boundary[1], boundary[2])
				&& encloses(&circle, boundary)
			{
				return circle;
			}
			// Degenerate/near-collinear: smallest pair-diameter circle covering all.
			return [(0, 1), (0, 2), (1, 2)]
				.iter()
				.map(|&(i, j)| return circle_from_2(boundary[i], boundary[j]))
				.filter(|circle| return encloses(circle, boundary))
				.min_by(|left, right| return left.radius.partial_cmp(&right.radius).unwrap())
				.unwrap_or(Circle {
					cx: 0.0,
					cy: 0.0,
					radius: f64::MAX,
				});
		}
	}
}

/// Whether the circle holds all points.
fn encloses(circle: &Circle, pts: &[(f64, f64)]) -> bool {
	return pts.iter().all(|&point| return in_circle(circle, point));
}

/// Circle with the two points as diameter.
fn circle_from_2(first: (f64, f64), second: (f64, f64)) -> Circle {
	let cx = f64::midpoint(first.0, second.0);
	let cy = f64::midpoint(first.1, second.1);
	let radius = ((first.0 - second.0).powi(2) + (first.1 - second.1).powi(2)).sqrt() / 2.0;
	return Circle { cx, cy, radius };
}

/// Circle through three points, if not collinear.
fn circumcircle(first: (f64, f64), second: (f64, f64), third: (f64, f64)) -> Option<Circle> {
	let det = 2.0
		* (first.0 * (second.1 - third.1)
			+ second.0 * (third.1 - first.1)
			+ third.0 * (first.1 - second.1));
	if det.abs() < 1e-12 {
		return None;
	}
	let (ax2, ay2) = (first.0 * first.0 + first.1 * first.1, 0.0);
	let _ = ay2;
	let bx2 = second.0 * second.0 + second.1 * second.1;
	let cx2 = third.0 * third.0 + third.1 * third.1;
	let ux =
		(ax2 * (second.1 - third.1) + bx2 * (third.1 - first.1) + cx2 * (first.1 - second.1)) / det;
	let uy =
		(ax2 * (third.0 - second.0) + bx2 * (first.0 - third.0) + cx2 * (second.0 - first.0)) / det;
	let radius = ((first.0 - ux).powi(2) + (first.1 - uy).powi(2)).sqrt();
	return Some(Circle {
		cx: ux,
		cy: uy,
		radius,
	});
}

// --- affine coordinate baking ----------------------------------------------

/// Apply the affine to each element's coordinates.
fn bake_affine(
	src: &str,
	scale: f64,
	tx: f64,
	ty: f64,
	bx: (f64, f64, f64, f64),
) -> Result<String> {
	let mut reader = Reader::from_str(src);
	let mut writer = Writer::new(Vec::new());
	let mut seen_root = false;
	// Depth counter while inside a background subtree, which is copied verbatim.
	let mut asis = 0usize;
	let templates = affine::use_targets(src)?;
	// Depth inside a template: its `<use>` carries the translation.
	let mut template = 0usize;

	loop {
		match reader.read_event().context("parsing SVG")? {
			Event::Eof => break,
			Event::Start(tag) => {
				if asis > 0 {
					asis += 1;
					writer.write_event(Event::Start(tag))?;
				} else if !seen_root {
					seen_root = true;
					writer.write_event(Event::Start(tag))?; // canvas/viewBox unchanged
				} else if is_background(&tag, bx)? {
					asis = 1;
					writer.write_event(Event::Start(tag))?;
				} else {
					if template > 0 {
						template += 1;
					} else if affine::is_template(&tag, &templates)? {
						template = 1;
					}
					let out = process_element(&tag, scale, tx, ty, template == 0)?;
					writer.write_event(Event::Start(out))?;
				}
			}
			Event::End(tag) => {
				if asis > 0 {
					asis -= 1;
				} else {
					template = template.saturating_sub(1);
				}
				writer.write_event(Event::End(tag))?;
			}
			Event::Empty(tag) => {
				if asis > 0 || !seen_root {
					seen_root = true;
					writer.write_event(Event::Empty(tag))?;
				} else if is_background(&tag, bx)? {
					writer.write_event(Event::Empty(tag))?; // kept as-is
				} else {
					let inside = template > 0 || affine::is_template(&tag, &templates)?;
					let out = process_element(&tag, scale, tx, ty, !inside)?;
					writer.write_event(Event::Empty(out))?;
				}
			}
			other => writer.write_event(other)?,
		}
	}
	return Ok(String::from_utf8(writer.into_inner())?);
}

/// Rewrite the SVG with full-canvas background fills removed, for measurement. The flag reports
/// whether anything was actually removed.
fn strip_background(
	src: &str,
	min_x: f64,
	min_y: f64,
	width: f64,
	height: f64,
) -> Result<(String, bool)> {
	let bx = (min_x, min_y, width, height);
	let mut reader = Reader::from_str(src);
	let mut writer = Writer::new(Vec::new());
	let mut seen_root = false;
	let mut skip = 0usize;
	let mut stripped = false;

	loop {
		match reader.read_event().context("parsing SVG")? {
			Event::Eof => break,
			Event::Start(tag) => {
				if skip > 0 {
					skip += 1;
				} else if !seen_root {
					seen_root = true;
					writer.write_event(Event::Start(tag))?;
				} else if is_background(&tag, bx)? {
					skip = 1; // drop this element and its subtree
					stripped = true;
				} else {
					writer.write_event(Event::Start(tag))?;
				}
			}
			Event::End(tag) => {
				if skip > 0 {
					skip -= 1;
				} else {
					writer.write_event(Event::End(tag))?;
				}
			}
			Event::Empty(tag) => {
				if skip > 0 {
					// nested empty inside a skipped subtree: drop
				} else if !seen_root {
					seen_root = true;
					writer.write_event(Event::Empty(tag))?;
				} else if is_background(&tag, bx)? {
					stripped = true; // dropped
				} else {
					writer.write_event(Event::Empty(tag))?;
				}
			}
			other => {
				if skip == 0 {
					writer.write_event(other)?;
				}
			}
		}
	}
	return Ok((String::from_utf8(writer.into_inner())?, stripped));
}

/// A drawable, visibly-filled shape whose bounding box covers the whole viewBox.
fn is_background(tag: &BytesStart, bx: (f64, f64, f64, f64)) -> Result<bool> {
	let (min_x, min_y, width, height) = bx;
	let local = tag.local_name().into_inner().to_owned();

	let Some((left, top, right, bottom)) = element_bbox(&local, tag)? else {
		return Ok(false);
	};
	let filled = affine::attr_value(tag, "fill")?.is_none_or(|fill| return fill.trim() != "none");
	if !filled {
		return Ok(false);
	}

	let tol_x = width.abs() * 0.005 + 1e-6;
	let tol_y = height.abs() * 0.005 + 1e-6;
	let spans = left <= min_x + tol_x
		&& top <= min_y + tol_y
		&& right >= min_x + width - tol_x
		&& bottom >= min_y + height - tol_y;
	// A disc spans the canvas but is the frame, not a background.
	return Ok(spans && coverage(tag, bx)? >= BACKGROUND_COVERAGE);
}

/// Fraction of the canvas that `e`, drawn alone and opaque, covers.
///
/// The paint is replaced by black: a gradient it names is not in the small document.
// The pixel counts are at most `COVERAGE_PX` squared, which is exact in an f64.
#[expect(clippy::cast_precision_loss, reason = "pixel counts are exact in f64")]
fn coverage(tag: &BytesStart, bx: (f64, f64, f64, f64)) -> Result<f64> {
	let (min_x, min_y, width, height) = bx;
	let mut shape = BytesStart::new(tag.name().into_inner().to_owned());
	for attr in tag.attributes() {
		let attr = attr.map_err(|err| anyhow!("attribute: {err}"))?;
		if !matches!(
			attr.key.into_inner(),
			"fill" | "fill-opacity" | "opacity" | "style" | "stroke" | "filter" | "mask"
		) {
			shape.push_attribute(attr);
		}
	}
	shape.push_attribute(("fill", "#000"));

	let mut writer = Writer::new(Vec::new());
	writer.write_event(Event::Empty(shape))?;
	let shape = String::from_utf8(writer.into_inner())?;
	let doc = format!(
		r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="{min_x} {min_y} {width} {height}" width="{COVERAGE_PX}" height="{COVERAGE_PX}" preserveAspectRatio="none">{shape}</svg>"#
	);

	let tree = render::load_tree_from_data(doc.as_bytes())?;
	let pixmap = render::render_scale(&tree, 1.0)?;
	let pixels = pixmap.pixels();
	let opaque = pixels
		.iter()
		.filter(|pixel| return pixel.alpha() > 0)
		.count();
	return Ok(opaque as f64 / pixels.len() as f64);
}

/// Local (transform-ignoring) bounding box of a shape from its own attributes.
fn element_bbox(local: &str, tag: &BytesStart) -> Result<Option<(f64, f64, f64, f64)>> {
	let num = |key: &str| -> Result<Option<f64>> {
		return Ok(affine::attr_value(tag, key)?
			.and_then(|value| return split_num_unit(&value).map(|(n, _)| return n)));
	};
	let bbox = match local {
		"rect" => {
			let x = num("x")?.unwrap_or(0.0);
			let y = num("y")?.unwrap_or(0.0);
			match (num("width")?, num("height")?) {
				(Some(width), Some(height)) => Some((x, y, x + width, y + height)),
				_ => None,
			}
		}
		"circle" => {
			let cx = num("cx")?.unwrap_or(0.0);
			let cy = num("cy")?.unwrap_or(0.0);
			num("r")?.map(|radius| return (cx - radius, cy - radius, cx + radius, cy + radius))
		}
		"ellipse" => {
			let cx = num("cx")?.unwrap_or(0.0);
			let cy = num("cy")?.unwrap_or(0.0);
			match (num("rx")?, num("ry")?) {
				(Some(rx), Some(ry)) => Some((cx - rx, cy - ry, cx + rx, cy + ry)),
				_ => None,
			}
		}
		"polygon" | "polyline" => {
			affine::attr_value(tag, "points")?.and_then(|points| return points_bbox(&points))
		}
		"path" => affine::attr_value(tag, "d")?.and_then(|data| return path_bbox(&data)),
		_ => None,
	};
	return Ok(bbox);
}

/// Bounding box of a `points` list.
fn points_bbox(values: &str) -> Option<(f64, f64, f64, f64)> {
	let nums = parse_num_list(values);
	let mut bb: Option<(f64, f64, f64, f64)> = None;
	for pair in nums.chunks(2) {
		if pair.len() == 2 {
			extend(&mut bb, pair[0], pair[1]);
		}
	}
	return bb;
}

/// Conservative path bounding box (curve control points bound the curve).
fn path_bbox(data: &str) -> Option<(f64, f64, f64, f64)> {
	use svgtypes::{PathParser, PathSegment};

	let mut bb: Option<(f64, f64, f64, f64)> = None;
	let mut cur = (0.0f64, 0.0f64);
	let mut start = (0.0f64, 0.0f64);
	let abs = |from: (f64, f64), x: f64, y: f64, is_abs: bool| {
		if is_abs {
			return (x, y);
		}
		return (from.0 + x, from.1 + y);
	};

	for seg in PathParser::from(data) {
		let seg = seg.ok()?;
		match seg {
			PathSegment::MoveTo {
				abs: absolute,
				x,
				y,
			} => {
				cur = abs(cur, x, y, absolute);
				start = cur;
				extend(&mut bb, cur.0, cur.1);
			}
			// Segments for which only the endpoint is tracked. A smooth quadratic's reflected
			// control point and an arc's bulge are not accounted for, exactly as before these arms
			// were merged.
			PathSegment::LineTo {
				abs: absolute,
				x,
				y,
			}
			| PathSegment::SmoothQuadratic {
				abs: absolute,
				x,
				y,
			}
			| PathSegment::EllipticalArc {
				abs: absolute,
				x,
				y,
				..
			} => {
				cur = abs(cur, x, y, absolute);
				extend(&mut bb, cur.0, cur.1);
			}
			PathSegment::HorizontalLineTo { abs: absolute, x } => {
				cur.0 = if absolute { x } else { cur.0 + x };
				extend(&mut bb, cur.0, cur.1);
			}
			PathSegment::VerticalLineTo { abs: absolute, y } => {
				cur.1 = if absolute { y } else { cur.1 + y };
				extend(&mut bb, cur.0, cur.1);
			}
			PathSegment::CurveTo {
				abs: absolute,
				x1,
				y1,
				x2,
				y2,
				x,
				y,
			} => {
				let p1 = abs(cur, x1, y1, absolute);
				let p2 = abs(cur, x2, y2, absolute);
				let end = abs(cur, x, y, absolute);
				extend(&mut bb, p1.0, p1.1);
				extend(&mut bb, p2.0, p2.1);
				extend(&mut bb, end.0, end.1);
				cur = end;
			}
			PathSegment::SmoothCurveTo {
				abs: absolute,
				x2,
				y2,
				x,
				y,
			} => {
				let p2 = abs(cur, x2, y2, absolute);
				let end = abs(cur, x, y, absolute);
				extend(&mut bb, p2.0, p2.1);
				extend(&mut bb, end.0, end.1);
				cur = end;
			}
			PathSegment::Quadratic {
				abs: absolute,
				x1,
				y1,
				x,
				y,
			} => {
				let p1 = abs(cur, x1, y1, absolute);
				let end = abs(cur, x, y, absolute);
				extend(&mut bb, p1.0, p1.1);
				extend(&mut bb, end.0, end.1);
				cur = end;
			}
			PathSegment::ClosePath { .. } => cur = start,
		}
	}
	return bb;
}

/// Grow the bounding box to hold a point.
fn extend(bb: &mut Option<(f64, f64, f64, f64)>, x: f64, y: f64) {
	match bb {
		None => *bb = Some((x, y, x, y)),
		Some((left, top, right, bottom)) => {
			*left = left.min(x);
			*top = top.min(y);
			*right = right.max(x);
			*bottom = bottom.max(y);
		}
	}
}

/// Copy an element with its coordinates transformed.
fn process_element(
	tag: &BytesStart,
	scale: f64,
	tx: f64,
	ty: f64,
	translate: bool,
) -> Result<BytesStart<'static>> {
	let (tx, ty) = if translate { (tx, ty) } else { (0.0, 0.0) };
	let local = tag.local_name().into_inner().to_owned();

	// Gradient/pattern coords are only user-space (scalable) under userSpaceOnUse.
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
		affine::attr_value(tag, key)?.as_deref() == Some("userSpaceOnUse")
	} else {
		true
	};

	let name = tag.name().into_inner().to_owned();
	let mut out = BytesStart::new(name);
	let mut seen_keys = Vec::new();
	for attr in tag.attributes() {
		let attr = attr.map_err(|err| anyhow!("attribute: {err}"))?;
		let key = attr.key.into_inner().to_owned();
		let val = attr
			.normalized_value(quick_xml::XmlVersion::Implicit1_0)?
			.into_owned();
		let new_val = if do_scale {
			affine::attr(&key, &val, scale, tx, ty)
		} else {
			val
		};
		seen_keys.push(key.clone());
		out.push_attribute((key.as_str(), new_val.as_str()));
	}

	// An omitted `x` means 0, which the translation moves.
	if do_scale && (tx != 0.0 || ty != 0.0) {
		for key in affine::implicit_origins(&local) {
			if !seen_keys.iter().any(|seen| return seen == key) {
				let val = affine::origin_value(key, tx, ty);
				out.push_attribute((*key, val.as_str()));
			}
		}
	}
	return Ok(out);
}

// --- root parsing ----------------------------------------------------------

/// Read the root `<svg>` coordinate box: `(min_x, min_y, w, h)` from `viewBox`, else from
/// `width`/`height`.
pub fn parse_root_box(src: &str) -> Result<(f64, f64, f64, f64)> {
	let mut reader = Reader::from_str(src);
	loop {
		match reader.read_event().context("parsing SVG")? {
			Event::Eof => bail!("no root <svg> element found"),
			Event::Start(tag) | Event::Empty(tag) => return root_box(&tag),
			_ => {}
		}
	}
}

/// The root `viewBox`, or its width and height.
fn root_box(tag: &BytesStart) -> Result<(f64, f64, f64, f64)> {
	let local = tag.local_name().into_inner();
	if local != "svg" {
		bail!("root element is <{local}>, expected <svg>");
	}
	if let Some(vb) = affine::attr_value(tag, "viewBox")? {
		let n = parse_num_list(&vb);
		if n.len() != 4 {
			bail!("viewBox must have 4 numbers, got {}", n.len());
		}
		return Ok((n[0], n[1], n[2], n[3]));
	}
	match (
		affine::attr_value(tag, "width")?,
		affine::attr_value(tag, "height")?,
	) {
		(Some(width), Some(height)) => {
			return Ok((0.0, 0.0, parse_px(&width)?, parse_px(&height)?));
		}
		_ => bail!("root <svg> has neither viewBox nor numeric width/height"),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn mec_of_square_corners() {
		// Unit square: MEC center (0.5,0.5), radius = sqrt(2)/2.
		let circle = min_enclosing_circle(&[(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]);
		assert!((circle.cx - 0.5).abs() < 1e-6);
		assert!((circle.cy - 0.5).abs() < 1e-6);
		assert!((circle.radius - 0.5_f64.sqrt()).abs() < 1e-6);
	}

	#[test]
	fn detects_full_canvas_background() {
		let bx = (0.0, 0.0, 1024.0, 1024.0);
		let bg = BytesStart::from_content(r##"path fill="#f60" d="M0 0h1024v1024H0z""##, 4);
		assert!(is_background(&bg, bx).unwrap());

		let fg = BytesStart::from_content(r#"rect x="70" y="10" width="10" height="10""#, 4);
		assert!(!is_background(&fg, bx).unwrap());

		let unfilled = BytesStart::from_content(r#"path fill="none" d="M0 0h1024v1024H0z""#, 4);
		assert!(!is_background(&unfilled, bx).unwrap());

		// A rounded tile still covers the canvas.
		let tile = BytesStart::from_content(
			r#"rect fill="url(#g)" width="1024" height="1024" rx="160""#,
			4,
		);
		assert!(is_background(&tile, bx).unwrap());
	}

	/// A disc that touches every edge is the frame of the artwork, not a background.
	#[test]
	fn a_full_canvas_disc_is_not_a_background() {
		let bx = (0.0, 0.0, 1024.0, 1024.0);
		let disc = BytesStart::from_content(r##"circle fill="#fff" cx="512" cy="512" r="512""##, 6);
		assert!(!is_background(&disc, bx).unwrap());
	}

	/// Artwork that is already round keeps its size.
	#[test]
	fn a_round_badge_is_unchanged() {
		let src = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><circle cx="50" cy="50" r="50" fill="#fff"/><rect x="40" y="40" width="20" height="20" fill="#000"/></svg>"##;
		let round = round_str(src, None).unwrap();
		assert!(round.padding.abs() < 1e-9, "padding {}", round.padding);
		assert!(
			(round.scale - 1.0).abs() < f64::EPSILON,
			"scale {}",
			round.scale
		);
		assert_eq!(round.svg, src);
	}

	#[test]
	fn strip_removes_background_keeps_foreground() {
		let src = r##"<svg viewBox="0 0 100 100"><rect width="100" height="100" fill="#000"/><rect x="70" y="10" width="10" height="10" fill="#fff"/></svg>"##;
		let (fg, stripped) = strip_background(src, 0.0, 0.0, 100.0, 100.0).unwrap();
		assert!(stripped);
		assert!(!fg.contains(r#"width="100""#));
		assert!(fg.contains(r#"x="70""#));
	}

	// The padding values are copied through, never computed, so comparing them exactly is the
	// assertion being made.
	#[test]
	#[expect(
		clippy::float_cmp_const,
		reason = "the values are copied, not computed"
	)]
	fn padding_default_follows_the_background() {
		let plain = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><rect x="70" y="10" width="10" height="10" fill="#fff"/></svg>"##;
		let on_bg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><rect width="100" height="100" fill="#000"/><rect x="70" y="10" width="10" height="10" fill="#fff"/></svg>"##;

		let round = round_str(plain, None).unwrap();
		assert!(round.auto_padding);
		assert_eq!(round.padding, 0.0);

		let round = round_str(on_bg, None).unwrap();
		assert!(round.auto_padding);
		assert_eq!(round.padding, DEFAULT_ROUND_PADDING);

		// An explicit value wins over both.
		let round = round_str(on_bg, Some(0.25)).unwrap();
		assert!(!round.auto_padding);
		assert_eq!(round.padding, 0.25);
	}

	#[test]
	fn padding_out_of_range_is_rejected() {
		let src = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><rect x="70" y="10" width="10" height="10" fill="#fff"/></svg>"##;
		assert!(round_str(src, Some(1.0)).is_err());
		assert!(round_str(src, Some(-0.1)).is_err());
	}

	#[test]
	fn foreground_fits_circle_background_kept() {
		let src = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><rect width="100" height="100" fill="#000"/><rect x="70" y="10" width="10" height="10" fill="#fff"/></svg>"##;
		let bx = (0.0, 0.0, 100.0, 100.0);

		// Measure foreground only.
		let (fg, _) = strip_background(src, 0.0, 0.0, 100.0, 100.0).unwrap();
		let tree = render::load_tree_from_data(fg.as_bytes()).unwrap();
		let (cx, cy, shape_radius) = shape_enclosing_circle(&tree, 0.0, 0.0, 100.0, 100.0).unwrap();

		let radius = 50.0;
		let scale = radius / shape_radius;
		let outs = bake_affine(src, scale, 50.0 - scale * cx, 50.0 - scale * cy, bx).unwrap();

		// Background left as-is.
		assert!(outs.contains(r##"fill="#000""##));
		assert!(outs.contains(r#"width="100" height="100""#));

		// Rounded foreground now fills the inscribed circle (center 50,50, r 50).
		let (fg2, _) = strip_background(&outs, 0.0, 0.0, 100.0, 100.0).unwrap();
		let tree2 = render::load_tree_from_data(fg2.as_bytes()).unwrap();
		let (cx2, cy2, r2) = shape_enclosing_circle(&tree2, 0.0, 0.0, 100.0, 100.0).unwrap();
		assert!((cx2 - 50.0).abs() < 1.0, "cx {cx2}");
		assert!((cy2 - 50.0).abs() < 1.0, "cy {cy2}");
		assert!((r2 - 50.0).abs() < 1.5, "r {r2}");
	}

	/// oxvg rounds coordinates, so the round fit must survive the optimizer within the same
	/// tolerance the test above uses. The precision search runs, as it does by default.
	#[test]
	fn round_output_survives_optimization() {
		use crate::svg_optimize::{Opts, Precision, maybe_optimize};

		let src = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><rect width="100" height="100" fill="#000"/><rect x="70" y="10" width="10" height="10" fill="#fff"/></svg>"##;

		let rounded = round_str(src, Some(0.0)).unwrap();
		let out = maybe_optimize(
			&rounded.svg,
			Opts {
				enabled: true,
				keep_dimensions: false,
				precision: Precision::Adaptive { threshold: 0.02 },
			},
		)
		.unwrap()
		.svg;

		let (fg, _) = strip_background(&out, 0.0, 0.0, 100.0, 100.0).unwrap();
		let tree = render::load_tree_from_data(fg.as_bytes()).unwrap();
		let (cx, cy, radius) = shape_enclosing_circle(&tree, 0.0, 0.0, 100.0, 100.0).unwrap();

		assert!((cx - 50.0).abs() < 1.0, "cx {cx}");
		assert!((cy - 50.0).abs() < 1.0, "cy {cy}");
		assert!((radius - 50.0).abs() < 1.5, "radius {radius}");
	}

	/// A template is placed by its `<use>`, so only the instance takes the translation.
	#[test]
	fn a_use_template_is_not_moved_twice() {
		let src = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><defs><path id="a" d="M10 10h20v20h-20Z"/></defs><use href="#a" x="10" y="10" fill="#000"/></svg>"##;
		let out = round_str(src, Some(0.0)).unwrap().svg;

		let tree = render::load_tree_from_data(out.as_bytes()).unwrap();
		let (cx, cy, _) = shape_enclosing_circle(&tree, 0.0, 0.0, 100.0, 100.0).unwrap();

		assert!((cx - 50.0).abs() < 1.0, "cx {cx}: {out}");
		assert!((cy - 50.0).abs() < 1.0, "cy {cy}: {out}");
	}
}
