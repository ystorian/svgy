// src/svg_stroke.rs

//! Join neighbouring outlines of the same style into one path.
//!
//! `merge_paths` of oxvg joins two paths only when their shapes do not overlap, since a fill can
//! change where shapes overlap. An outline with no fill and an opaque stroke paints the same pixels
//! whether its parts are drawn one by one or in one go, so an overlap does no harm.
//!
//! A dash pattern, a marker or a stroke with opacity is drawn per path, so any of them, on the
//! element or inherited, keeps the paths apart.

use anyhow::{Context as _, Result, anyhow};
use quick_xml::events::{BytesStart, BytesText, Event};
use quick_xml::reader::Reader;
use quick_xml::writer::Writer;

/// Attributes that draw each path on its own, or that tie a path to others.
const BLOCKERS: [&str; 12] = [
	"id",
	"style",
	"class",
	"opacity",
	"filter",
	"mask",
	"stroke-opacity",
	"stroke-dasharray",
	"marker-start",
	"marker-mid",
	"marker-end",
	"marker",
];

/// Join each run of neighbouring outline paths of `src` that share every attribute but `d`.
///
/// `dynamic` says the document has a `<style>` or a `<script>`, which can tell the paths apart.
pub fn merge_strokes(src: &str, dynamic: bool) -> Result<String> {
	if dynamic {
		return Ok(src.to_owned());
	}
	let mut reader = Reader::from_str(src);
	let mut writer = Writer::new(Vec::new());
	// One entry per open element: whether it, or an element above it, sets a blocker.
	let mut blocked = Vec::new();
	// An outline, and the blank text after it, waiting for its neighbour.
	let mut pending: Option<(Outline, Vec<BytesText<'static>>)> = None;

	loop {
		let event = reader.read_event().context("parsing SVG")?;
		if let Some((first, blanks)) = pending.as_mut() {
			match &event {
				Event::Text(text) if text.as_ref().trim().is_empty() => {
					blanks.push(text.clone().into_owned());
					continue;
				}
				Event::Empty(elem) if !blocked.last().copied().unwrap_or(false) => {
					if let Some(next) = Outline::read(elem)?
						&& next.rest == first.rest
					{
						first.join(&next.data);
						blanks.clear();
						continue;
					}
				}
				_ => {}
			}
			if let Some((first, blanks)) = pending.take() {
				writer.write_event(Event::Empty(first.write()))?;
				for blank in blanks {
					writer.write_event(Event::Text(blank))?;
				}
			}
		}
		match event {
			Event::Eof => break,
			Event::Start(elem) => {
				let inherited = blocked.last().copied().unwrap_or(false);
				blocked.push(inherited || has_blocker(&elem)?);
				writer.write_event(Event::Start(elem))?;
			}
			Event::End(elem) => {
				blocked.pop();
				writer.write_event(Event::End(elem))?;
			}
			Event::Empty(elem) => {
				let outline = if blocked.last().copied().unwrap_or(false) {
					None
				} else {
					Outline::read(&elem)?
				};
				match outline {
					Some(outline) => pending = Some((outline, Vec::new())),
					None => writer.write_event(Event::Empty(elem))?,
				}
			}
			other => writer.write_event(other)?,
		}
	}
	return Ok(String::from_utf8(writer.into_inner())?);
}

/// Whether `elem` sets an attribute that its children inherit and that draws paths one by one.
fn has_blocker(elem: &BytesStart) -> Result<bool> {
	for attr in elem.attributes() {
		let attr = attr.map_err(|err| anyhow!("attribute: {err}"))?;
		let key = attr.key.into_inner();
		if key == "style"
			|| key == "stroke-opacity"
			|| key == "stroke-dasharray"
			|| key.starts_with("marker")
		{
			return Ok(true);
		}
	}
	return Ok(false);
}

/// A `<path>` with no fill and an opaque, solid stroke.
struct Outline {
	/// Every attribute but `d`, in document order.
	rest: Vec<(String, String)>,
	/// The path data.
	data: String,
}

impl Outline {
	/// The outline of `elem`, if it is one.
	fn read(elem: &BytesStart) -> Result<Option<Self>> {
		if elem.name().into_inner() != "path" {
			return Ok(None);
		}
		let mut rest = Vec::new();
		let mut data = None;
		for attr in elem.attributes() {
			let attr = attr.map_err(|err| anyhow!("attribute: {err}"))?;
			let key = attr.key.into_inner().to_owned();
			let val = attr
				.normalized_value(quick_xml::XmlVersion::Implicit1_0)?
				.into_owned();
			if BLOCKERS.contains(&key.as_str()) {
				return Ok(None);
			}
			if key == "d" {
				data = Some(val);
			} else {
				rest.push((key, val));
			}
		}
		let get = |name: &str| {
			return rest
				.iter()
				.find(|(key, _)| return key == name)
				.map(|(_, val)| return val.trim());
		};
		// A paint server sized to the bounding box changes with the shape.
		let solid = get("stroke").is_some_and(|stroke| {
			return stroke != "none" && !stroke.starts_with("url(");
		});
		if get("fill") != Some("none") || !solid {
			return Ok(None);
		}
		return Ok(data.map(|data| return Self { rest, data }));
	}

	/// Add the subpaths of `data` after these.
	fn join(&mut self, data: &str) {
		let data = data.trim_start();
		// A leading `m` is absolute, but not once it follows other subpaths.
		let data = match data.strip_prefix('m') {
			Some(tail) => format!("M{tail}"),
			None => data.to_owned(),
		};
		self.data.push_str(&data);
	}

	/// The path as a start tag.
	fn write(self) -> BytesStart<'static> {
		let mut out = BytesStart::new("path");
		for (key, val) in &self.rest {
			out.push_attribute((key.as_str(), val.as_str()));
		}
		out.push_attribute(("d", self.data.as_str()));
		return out;
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn overlapping_outlines_merge() {
		let src = r##"<svg xmlns="http://www.w3.org/2000/svg">
	<path fill="none" stroke="#000" d="M0 0h10v10H0Z"/>
	<path fill="none" stroke="#000" d="m5 5h10v10H5Z"/>
</svg>"##;
		let out = merge_strokes(src, false).unwrap();
		assert_eq!(
			out,
			r##"<svg xmlns="http://www.w3.org/2000/svg">
	<path fill="none" stroke="#000" d="M0 0h10v10H0ZM5 5h10v10H5Z"/>
</svg>"##
		);
	}

	/// A fill makes the overlap matter.
	#[test]
	fn filled_paths_are_kept() {
		let src = r##"<svg xmlns="http://www.w3.org/2000/svg"><path fill="#f00" stroke="#000" d="M0 0h9"/><path fill="#f00" stroke="#000" d="M5 5h9"/></svg>"##;
		assert_eq!(merge_strokes(src, false).unwrap(), src);
	}

	#[test]
	fn another_stroke_is_kept() {
		let src = r##"<svg xmlns="http://www.w3.org/2000/svg"><path fill="none" stroke="#000" d="M0 0h9"/><path fill="none" stroke="#000" stroke-width="2" d="M5 5h9"/></svg>"##;
		assert_eq!(merge_strokes(src, false).unwrap(), src);
	}

	/// A see-through stroke paints the overlap twice.
	#[test]
	fn an_inherited_stroke_opacity_is_kept() {
		let src = r##"<svg xmlns="http://www.w3.org/2000/svg"><g stroke-opacity=".5"><path fill="none" stroke="#000" d="M0 0h9"/><path fill="none" stroke="#000" d="M5 5h9"/></g></svg>"##;
		assert_eq!(merge_strokes(src, false).unwrap(), src);
	}

	#[test]
	fn a_dash_pattern_is_kept() {
		let src = r##"<svg xmlns="http://www.w3.org/2000/svg"><path fill="none" stroke="#000" stroke-dasharray="2" d="M0 0h9"/><path fill="none" stroke="#000" stroke-dasharray="2" d="M5 5h9"/></svg>"##;
		assert_eq!(merge_strokes(src, false).unwrap(), src);
	}
}
