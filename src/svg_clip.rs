// src/svg_clip.rs

//! Drop a `clip-path` that covers the whole canvas.
//!
//! This runs on the source, before the resize. The resize pads the canvas, and after that the clip
//! is no longer the same size as the canvas. Once the `clip-path` attributes go, oxvg removes the
//! `<clipPath>` and its `<defs>` because nothing uses them.

use anyhow::{Context as _, Result, anyhow};
use quick_xml::events::{BytesStart, Event};
use quick_xml::reader::Reader;
use quick_xml::writer::Writer;
use std::collections::{HashMap, HashSet};

use crate::affine::attr_value;
use crate::numeric::{parse_num_list, parse_px};

/// A box as `(min_x, min_y, max_x, max_y)`.
type Bounds = (f64, f64, f64, f64);

/// Elements that start a new coordinate system for their content.
const NEW_SPACE: [&str; 6] = ["svg", "symbol", "pattern", "marker", "mask", "clipPath"];

/// Remove every `clip-path` of `src` whose clip covers the canvas.
///
/// `dynamic` says the document has a `<style>` or a `<script>`, which can move an element or its
/// clip. The document then stays as it is.
pub fn strip_canvas_clips(src: &str, dynamic: bool) -> Result<String> {
	if dynamic {
		return Ok(src.to_owned());
	}
	let Some(canvas) = canvas(src)? else {
		return Ok(src.to_owned());
	};
	let full = full_clips(src, canvas)?;
	if full.is_empty() {
		return Ok(src.to_owned());
	}

	let mut reader = Reader::from_str(src);
	let mut writer = Writer::new(Vec::new());
	// One entry per open element: whether its content is in the canvas space.
	let mut plain = Vec::new();

	loop {
		match reader.read_event().context("parsing SVG")? {
			Event::Eof => break,
			Event::Start(elem) => {
				let here = in_canvas_space(&elem, &plain)?;
				let out = unclipped(&elem, here, &full)?;
				// The root is the canvas, not a new space.
				plain.push(here && (plain.is_empty() || !NEW_SPACE.contains(&local(&elem))));
				writer.write_event(Event::Start(out))?;
			}
			Event::End(elem) => {
				plain.pop();
				writer.write_event(Event::End(elem))?;
			}
			Event::Empty(elem) => {
				let here = in_canvas_space(&elem, &plain)?;
				writer.write_event(Event::Empty(unclipped(&elem, here, &full)?))?;
			}
			other => writer.write_event(other)?,
		}
	}
	return Ok(String::from_utf8(writer.into_inner())?);
}

/// Local name of the element.
fn local<'a>(elem: &'a BytesStart) -> &'a str {
	return elem.local_name().into_inner();
}

/// Whether `elem` draws in the canvas space: no transform on it or above it.
///
/// The root itself is the canvas, so it opens the first entry.
fn in_canvas_space(elem: &BytesStart, plain: &[bool]) -> Result<bool> {
	let parent = plain.last().copied().unwrap_or(true);
	return Ok(parent && attr_value(elem, "transform")?.is_none());
}

/// Rebuild `elem` without a `clip-path` that hides nothing.
fn unclipped(
	elem: &BytesStart,
	plain: bool,
	full: &HashSet<String>,
) -> Result<BytesStart<'static>> {
	let mut out = BytesStart::new(elem.name().into_inner().to_owned());
	for attr in elem.attributes() {
		let attr = attr.map_err(|err| anyhow!("attribute: {err}"))?;
		let key = attr.key.into_inner();
		let val = attr
			.normalized_value(quick_xml::XmlVersion::Implicit1_0)?
			.into_owned();
		if plain && key == "clip-path" && clip_id(&val).is_some_and(|id| full.contains(id)) {
			continue;
		}
		out.push_attribute((key, val.as_str()));
	}
	return Ok(out);
}

/// `url(#a)` -> `a`.
fn clip_id(val: &str) -> Option<&str> {
	let inner = val.trim().strip_prefix("url(")?.strip_suffix(')')?.trim();
	let inner = inner.trim_matches(|ch| return ch == '"' || ch == '\'');
	return inner.strip_prefix('#');
}

/// The visible area of the root, or `None` when it shows more than its `viewBox`.
fn canvas(src: &str) -> Result<Option<Bounds>> {
	let mut reader = Reader::from_str(src);
	let root = loop {
		match reader.read_event().context("parsing SVG")? {
			Event::Eof => return Ok(None),
			Event::Start(elem) | Event::Empty(elem) => break elem.into_owned(),
			_ => {}
		}
	};
	let size = match (attr_value(&root, "width")?, attr_value(&root, "height")?) {
		(Some(width), Some(height)) => match (parse_px(&width), parse_px(&height)) {
			(Ok(width), Ok(height)) => Some((width, height)),
			_ => return Ok(None),
		},
		(None, None) => None,
		_ => return Ok(None),
	};
	let Some(view_box) = attr_value(&root, "viewBox")? else {
		return Ok(size.map(|(width, height)| return (0.0, 0.0, width, height)));
	};
	let nums = parse_num_list(&view_box);
	let &[min_x, min_y, width, height] = nums.as_slice() else {
		return Ok(None);
	};
	// A viewport of another shape shows space around the `viewBox`.
	if let Some((view_w, view_h)) = size
		&& (view_w * height - view_h * width).abs() > 1e-6 * view_w * height
	{
		return Ok(None);
	}
	return Ok(Some((min_x, min_y, min_x + width, min_y + height)));
}

/// Ids of the `<clipPath>` elements whose single shape covers `canvas`.
fn full_clips(src: &str, canvas: Bounds) -> Result<HashSet<String>> {
	let mut reader = Reader::from_str(src);
	let mut rects: HashMap<String, Bounds> = HashMap::new();
	// Each candidate clip, with what it holds: a box, or `None` for anything else.
	let mut clips: Vec<(String, Vec<Option<Shape>>)> = Vec::new();
	// How deep inside the open clip the reader is.
	let mut open: Option<usize> = None;

	loop {
		match reader.read_event().context("parsing SVG")? {
			Event::Eof => break,
			Event::Start(elem) => {
				if let Some(depth) = open.as_mut() {
					// A shape with children is not a box.
					if *depth == 0
						&& let Some(clip) = clips.last_mut()
					{
						clip.1.push(None);
					}
					*depth += 1;
				} else if let Some(id) = candidate_clip(&elem)? {
					clips.push((id, Vec::new()));
					open = Some(0);
				}
			}
			Event::End(_) => match open {
				Some(0) => open = None,
				Some(depth) => open = Some(depth - 1),
				None => {}
			},
			Event::Empty(elem) => {
				if let Some(id) = attr_value(&elem, "id")?
					&& let Some(bounds) = rect_bounds(&elem)?
				{
					rects.insert(id, bounds);
				}
				if open == Some(0)
					&& let Some(clip) = clips.last_mut()
				{
					clip.1.push(shape(&elem)?);
				}
			}
			_ => {}
		}
	}

	let eps = 1e-6 * (canvas.2 - canvas.0).abs().max((canvas.3 - canvas.1).abs());
	let covers = |bounds: Bounds| {
		return bounds.0 <= canvas.0 + eps
			&& bounds.1 <= canvas.1 + eps
			&& bounds.2 >= canvas.2 - eps
			&& bounds.3 >= canvas.3 - eps;
	};
	let mut full = HashSet::new();
	for (id, shapes) in clips {
		let [Some(shape)] = shapes.as_slice() else {
			continue;
		};
		let bounds = match shape {
			Shape::Rect(bounds) => Some(*bounds),
			Shape::Use { href, dx, dy } => rects
				.get(href)
				.map(|rect| return (rect.0 + dx, rect.1 + dy, rect.2 + dx, rect.3 + dy)),
		};
		if bounds.is_some_and(covers) {
			full.insert(id);
		}
	}
	return Ok(full);
}

/// The id of a `<clipPath>` in the space of the element it clips.
fn candidate_clip(elem: &BytesStart) -> Result<Option<String>> {
	if local(elem) != "clipPath"
		|| attr_value(elem, "transform")?.is_some()
		|| attr_value(elem, "clip-path")?.is_some()
		|| attr_value(elem, "clipPathUnits")?.is_some_and(|units| return units != "userSpaceOnUse")
	{
		return Ok(None);
	}
	return attr_value(elem, "id");
}

/// A clip shape this pass can measure.
enum Shape {
	/// A `<rect>` and its bounds.
	Rect(Bounds),
	/// A `<use>` of a rect, moved by `dx`, `dy`.
	Use {
		/// The `id` of the rect.
		href: String,
		/// Move along x.
		dx: f64,
		/// Move along y.
		dy: f64,
	},
}

/// The clip shape of `elem`, if measurable.
fn shape(elem: &BytesStart) -> Result<Option<Shape>> {
	match local(elem) {
		"rect" => return Ok(rect_bounds(elem)?.map(Shape::Rect)),
		"use" if attr_value(elem, "transform")?.is_none() => {
			let href = match attr_value(elem, "href")? {
				Some(href) => Some(href),
				None => attr_value(elem, "xlink:href")?,
			};
			let Some(href) =
				href.and_then(|href| return href.trim().strip_prefix('#').map(str::to_owned))
			else {
				return Ok(None);
			};
			let (Some(dx), Some(dy)) = (length(elem, "x")?, length(elem, "y")?) else {
				return Ok(None);
			};
			return Ok(Some(Shape::Use { href, dx, dy }));
		}
		_ => return Ok(None),
	}
}

/// The box of a plain `<rect>`: no transform, no rounded corners.
fn rect_bounds(elem: &BytesStart) -> Result<Option<Bounds>> {
	if local(elem) != "rect" || attr_value(elem, "transform")?.is_some() {
		return Ok(None);
	}
	for key in ["rx", "ry"] {
		if length(elem, key)?.is_none_or(|radius| return radius != 0.0) {
			return Ok(None);
		}
	}
	let (Some(x), Some(y)) = (length(elem, "x")?, length(elem, "y")?) else {
		return Ok(None);
	};
	let (Some(Some(width)), Some(Some(height))) = (
		attr_value(elem, "width")?.map(|val| return parse_px(&val).ok()),
		attr_value(elem, "height")?.map(|val| return parse_px(&val).ok()),
	) else {
		return Ok(None);
	};
	return Ok(Some((x, y, x + width, y + height)));
}

/// A length that defaults to `0`, or `None` when it is not in user units.
fn length(elem: &BytesStart, key: &str) -> Result<Option<f64>> {
	match attr_value(elem, key)? {
		None => return Ok(Some(0.0)),
		Some(val) => return Ok(parse_px(&val).ok()),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// The clip Illustrator writes for the artboard.
	#[test]
	fn an_artboard_clip_is_dropped() {
		let src = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="20px" height="10px" viewBox="-5 0 20 10"><defs><rect id="r" x="-5" y="0" width="20" height="10"/></defs><clipPath id="c"><use xlink:href="#r" overflow="visible"/></clipPath><g><path clip-path="url(#c)" d="M0 0h5"/></g></svg>"##;
		let out = strip_canvas_clips(src, false).unwrap();
		assert!(!out.contains("clip-path="), "{out}");
	}

	#[test]
	fn a_smaller_clip_is_kept() {
		let src = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 10"><clipPath id="c"><rect width="19" height="10"/></clipPath><path clip-path="url(#c)" d="M0 0h5"/></svg>"#;
		let out = strip_canvas_clips(src, false).unwrap();
		assert_eq!(out, src);
	}

	/// A transform moves the clip with the element.
	#[test]
	fn a_moved_element_keeps_its_clip() {
		let src = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 10"><clipPath id="c"><rect width="20" height="10"/></clipPath><g transform="translate(5 0)"><path clip-path="url(#c)" d="M0 0h5"/></g></svg>"#;
		let out = strip_canvas_clips(src, false).unwrap();
		assert_eq!(out, src);
	}

	/// A wide viewport shows space beside the `viewBox`, which the clip hides.
	#[test]
	fn a_wider_viewport_keeps_the_clip() {
		let src = r#"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="10" viewBox="0 0 20 10"><clipPath id="c"><rect width="20" height="10"/></clipPath><path clip-path="url(#c)" d="M0 0h5"/></svg>"#;
		let out = strip_canvas_clips(src, false).unwrap();
		assert_eq!(out, src);
	}

	#[test]
	fn a_clip_of_two_shapes_is_kept() {
		let src = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 10"><clipPath id="c"><rect width="20" height="10"/><rect width="1" height="1"/></clipPath><path clip-path="url(#c)" d="M0 0h5"/></svg>"#;
		let out = strip_canvas_clips(src, false).unwrap();
		assert_eq!(out, src);
	}
}
