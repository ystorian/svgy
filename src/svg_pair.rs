// src/svg_pair.rs

//! Merge a filled shape with the stroked copy that follows it.

use anyhow::{Context as _, Result, anyhow};
use quick_xml::events::{BytesStart, BytesText, Event};
use quick_xml::reader::Reader;
use quick_xml::writer::Writer;

/// Attributes that paint the two elements as one group, or that tie an element to others.
const BLOCKERS: [&str; 10] = [
	"id",
	"style",
	"class",
	"opacity",
	"filter",
	"mask",
	"paint-order",
	"marker-start",
	"marker-mid",
	"marker-end",
];

/// Merge each filled shape of `src` with the stroked copy right after it.
///
/// `dynamic` says the document has a `<style>` or a `<script>`, which can tell the two apart.
pub fn merge_pairs(src: &str, dynamic: bool) -> Result<String> {
	if dynamic {
		return Ok(src.to_owned());
	}
	let mut reader = Reader::from_str(src);
	let mut writer = Writer::new(Vec::new());
	// One entry per open element: whether it, or an element above it, sets a stroke.
	let mut stroked = Vec::new();
	// A filled shape, and the blank text after it, waiting for its copy.
	let mut pending: Option<(BytesStart<'static>, Vec<BytesText<'static>>)> = None;

	loop {
		let event = reader.read_event().context("parsing SVG")?;
		if let Some((first, blanks)) = pending.as_mut() {
			match &event {
				Event::Text(text) if text.as_ref().trim().is_empty() => {
					blanks.push(text.clone().into_owned());
					continue;
				}
				Event::Empty(second) => {
					if let Some(merged) = merged(first, second)? {
						writer.write_event(Event::Empty(merged))?;
						pending = None;
						continue;
					}
				}
				_ => {}
			}
			if let Some((first, blanks)) = pending.take() {
				writer.write_event(Event::Empty(first))?;
				for blank in blanks {
					writer.write_event(Event::Text(blank))?;
				}
			}
		}
		match event {
			Event::Eof => break,
			Event::Start(elem) => {
				let inherited = stroked.last().copied().unwrap_or(false);
				stroked.push(inherited || sets_stroke(&elem)?);
				writer.write_event(Event::Start(elem))?;
			}
			Event::End(elem) => {
				stroked.pop();
				writer.write_event(Event::End(elem))?;
			}
			Event::Empty(elem) => {
				let inherited = stroked.last().copied().unwrap_or(false);
				if is_fill_only(&elem, inherited)? {
					pending = Some((elem.into_owned(), Vec::new()));
				} else {
					writer.write_event(Event::Empty(elem))?;
				}
			}
			other => writer.write_event(other)?,
		}
	}
	return Ok(String::from_utf8(writer.into_inner())?);
}

/// Name and value of every attribute of `elem`.
fn attrs(elem: &BytesStart) -> Result<Vec<(String, String)>> {
	let mut out = Vec::new();
	for attr in elem.attributes() {
		let attr = attr.map_err(|err| anyhow!("attribute: {err}"))?;
		let val = attr
			.normalized_value(quick_xml::XmlVersion::Implicit1_0)?
			.into_owned();
		out.push((attr.key.into_inner().to_owned(), val));
	}
	return Ok(out);
}

/// Whether the key is a stroke property.
fn is_stroke_key(key: &str) -> bool {
	return key == "stroke" || key.starts_with("stroke-");
}

/// Whether the key is a fill property.
fn is_fill_key(key: &str) -> bool {
	return key == "fill" || key.starts_with("fill-");
}

/// Whether `elem` sets a stroke that its children inherit.
fn sets_stroke(elem: &BytesStart) -> Result<bool> {
	return Ok(attrs(elem)?.iter().any(|(key, val)| {
		return (key == "stroke" && val.trim() != "none")
			|| (key == "style" && val.contains("stroke"));
	}));
}

/// A shape with a fill and no stroke, own or inherited.
fn is_fill_only(elem: &BytesStart, inherited: bool) -> Result<bool> {
	let attrs = attrs(elem)?;
	if attrs
		.iter()
		.any(|(key, _)| return BLOCKERS.contains(&key.as_str()))
	{
		return Ok(false);
	}
	let get = |name: &str| {
		return attrs
			.iter()
			.find(|(key, _)| return key == name)
			.map(|(_, val)| return val.trim());
	};
	if get("fill") == Some("none") {
		return Ok(false);
	}
	return Ok(match get("stroke") {
		Some(stroke) => stroke == "none",
		None => !inherited,
	});
}

/// `first` with the stroke of `second`, when `second` is its stroked copy.
fn merged(first: &BytesStart, second: &BytesStart) -> Result<Option<BytesStart<'static>>> {
	if first.name() != second.name() {
		return Ok(None);
	}
	let second_attrs = attrs(second)?;
	if second_attrs
		.iter()
		.any(|(key, _)| return BLOCKERS.contains(&key.as_str()))
	{
		return Ok(None);
	}
	let get = |name: &str| {
		return second_attrs
			.iter()
			.find(|(key, _)| return key == name)
			.map(|(_, val)| return val.trim());
	};
	if get("fill") != Some("none") || get("stroke").is_none_or(|stroke| return stroke == "none") {
		return Ok(None);
	}

	// Geometry, transform and every other attribute must match.
	let first_attrs = attrs(first)?;
	let rest = |list: &[(String, String)]| {
		let mut rest: Vec<_> = list
			.iter()
			.filter(|(key, _)| return !is_fill_key(key) && !is_stroke_key(key))
			.cloned()
			.collect();
		rest.sort();
		return rest;
	};
	if rest(&first_attrs) != rest(&second_attrs) {
		return Ok(None);
	}

	let mut out = BytesStart::new(first.name().into_inner().to_owned());
	for (key, val) in first_attrs
		.iter()
		.filter(|(key, _)| return !is_stroke_key(key))
	{
		out.push_attribute((key.as_str(), val.as_str()));
	}
	for (key, val) in second_attrs
		.iter()
		.filter(|(key, _)| return is_stroke_key(key))
	{
		out.push_attribute((key.as_str(), val.as_str()));
	}
	return Ok(Some(out));
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_rect_and_its_outline_merge() {
		let src = r##"<svg xmlns="http://www.w3.org/2000/svg"><rect x="1" fill="#f00" width="5" height="2"/>
<rect x="1" fill="none" stroke="#000" stroke-width="3" width="5" height="2"/></svg>"##;
		let out = merge_pairs(src, false).unwrap();
		assert_eq!(
			out,
			r##"<svg xmlns="http://www.w3.org/2000/svg"><rect x="1" fill="#f00" width="5" height="2" stroke="#000" stroke-width="3"/></svg>"##
		);
	}

	#[test]
	fn other_geometry_is_kept() {
		let src = r##"<svg xmlns="http://www.w3.org/2000/svg"><rect fill="#f00" width="5" height="2"/><rect fill="none" stroke="#000" width="5" height="3"/></svg>"##;
		assert_eq!(merge_pairs(src, false).unwrap(), src);
	}

	/// An inherited stroke is wider than the copy can hide.
	#[test]
	fn an_inherited_stroke_is_kept() {
		let src = r##"<svg xmlns="http://www.w3.org/2000/svg"><g stroke="#00f" stroke-width="9"><rect fill="#f00" width="5" height="2"/><rect fill="none" stroke="#000" width="5" height="2"/></g></svg>"##;
		assert_eq!(merge_pairs(src, false).unwrap(), src);
	}

	/// Opacity on both paints the overlap twice, and once after a merge.
	#[test]
	fn an_opacity_is_kept() {
		let src = r##"<svg xmlns="http://www.w3.org/2000/svg"><rect fill="#f00" opacity=".5" width="5" height="2"/><rect fill="none" stroke="#000" opacity=".5" width="5" height="2"/></svg>"##;
		assert_eq!(merge_pairs(src, false).unwrap(), src);
	}

	#[test]
	fn an_element_between_is_kept() {
		let src = r##"<svg xmlns="http://www.w3.org/2000/svg"><rect fill="#f00" width="5" height="2"/><circle r="1"/><rect fill="none" stroke="#000" width="5" height="2"/></svg>"##;
		assert_eq!(merge_pairs(src, false).unwrap(), src);
	}

	/// The stroke must come after the fill, as the paint order draws it.
	#[test]
	fn a_stroke_before_the_fill_is_kept() {
		let src = r##"<svg xmlns="http://www.w3.org/2000/svg"><rect fill="none" stroke="#000" width="5" height="2"/><rect fill="#f00" width="5" height="2"/></svg>"##;
		assert_eq!(merge_pairs(src, false).unwrap(), src);
	}
}
