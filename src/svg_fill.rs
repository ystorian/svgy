// src/svg_fill.rs

//! Drop a `fill` that no shape inherits.
//!
//! An export from Figma writes `fill="none"` on the root, and then a `fill` on every shape. A shape
//! reads its own `fill` first, so the one on the root reaches nothing. oxvg removes a useless
//! `fill` from a shape, but not from a container that every shape overrides.
//!
//! The document is read as a tree to find the containers, and is then written back as it came,
//! without the `fill` of those containers.

use anyhow::{Context as _, Result, anyhow};
use quick_xml::events::{BytesStart, Event};
use quick_xml::reader::Reader;
use quick_xml::writer::Writer;
use resvg::usvg::roxmltree;

/// The elements that paint with `fill`. A `<use>` draws content that can inherit it.
const PAINTED: &[&str] = &[
	"path", "rect", "circle", "ellipse", "line", "polyline", "polygon", "text", "tspan",
	"textPath", "use",
];

/// Remove every `fill` of a container whose shapes all set their own.
///
/// `dynamic` says the document has a `<style>` or a `<script>`, which can set a `fill` that this
/// scan cannot see. The document then stays as it is.
pub fn strip_unused(src: &str, dynamic: bool) -> Result<String> {
	if dynamic {
		return Ok(src.to_owned());
	}
	let unused = unused_fills(src)?;
	if unused.is_empty() {
		return Ok(src.to_owned());
	}

	let mut reader = Reader::from_str(src);
	let mut writer = Writer::new(Vec::new());
	// Elements in document order, as the tree counts them.
	let mut index = 0;

	loop {
		match reader.read_event().context("parsing SVG")? {
			Event::Eof => break,
			Event::Start(element) => {
				let out = if unused.contains(&index) {
					without_fill(&element)?
				} else {
					element
				};
				index += 1;
				writer.write_event(Event::Start(out))?;
			}
			Event::Empty(element) => {
				let out = if unused.contains(&index) {
					without_fill(&element)?
				} else {
					element
				};
				index += 1;
				writer.write_event(Event::Empty(out))?;
			}
			other => writer.write_event(other)?,
		}
	}

	return Ok(String::from_utf8(writer.into_inner())?);
}

/// The document-order index of every container whose `fill` no shape inherits.
fn unused_fills(src: &str) -> Result<Vec<usize>> {
	let doc = roxmltree::Document::parse_with_options(
		src,
		roxmltree::ParsingOptions {
			allow_dtd: true,
			..roxmltree::ParsingOptions::default()
		},
	)
	.context("parsing SVG")?;

	let elements: Vec<_> = doc
		.descendants()
		.filter(roxmltree::Node::is_element)
		.collect();
	return Ok(elements
		.iter()
		.enumerate()
		.filter(|(_, node)| return !is_painted(node) && node.has_attribute("fill"))
		.filter(|(_, node)| {
			return node
				.descendants()
				.filter(is_painted)
				.all(|shape| return overridden(shape, **node));
		})
		.map(|(i, _)| return i)
		.collect());
}

/// Whether the node is a shape that paints.
fn is_painted(node: &roxmltree::Node) -> bool {
	return node.is_element() && PAINTED.contains(&node.tag_name().name());
}

/// Whether `shape`, or a parent below `container`, sets its own `fill`.
fn overridden(shape: roxmltree::Node, container: roxmltree::Node) -> bool {
	return shape
		.ancestors()
		.take_while(|node| return *node != container)
		.any(|node| return sets_fill(&node));
}

/// Whether `node` sets a `fill` of its own, as an attribute or in its `style`.
fn sets_fill(node: &roxmltree::Node) -> bool {
	let own = |value: &str| return value.trim() != "inherit";
	return node.attribute("fill").is_some_and(own)
		|| node.attribute("style").is_some_and(|style| {
			return style.split(';').any(|declaration| {
				return declaration.split_once(':').is_some_and(|(name, value)| {
					return name.trim().eq_ignore_ascii_case("fill") && own(value);
				});
			});
		});
}

/// Copy of the element without `fill`.
fn without_fill(element: &BytesStart) -> Result<BytesStart<'static>> {
	let mut out = BytesStart::new(element.name().into_inner().to_owned());

	for attr in element.attributes() {
		let attr = attr.map_err(|err| anyhow!("attribute: {err}"))?;
		let key = attr.key.into_inner();
		if key == "fill" {
			continue;
		}
		let val = attr
			.normalized_value(quick_xml::XmlVersion::Implicit1_0)?
			.into_owned();
		out.push_attribute((key, val.as_str()));
	}
	return Ok(out);
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_root_fill_every_shape_overrides_goes() {
		let src = r##"<svg xmlns="http://www.w3.org/2000/svg" fill="none"><path fill="#fff" d="M0 0"/><g><circle style="fill:red" r="1"/></g></svg>"##;
		let out = strip_unused(src, false).unwrap();
		assert!(
			out.starts_with(r#"<svg xmlns="http://www.w3.org/2000/svg">"#),
			"{out}"
		);
		assert!(out.contains(r##"fill="#fff""##), "{out}");
	}

	#[test]
	fn an_inherited_fill_stays() {
		let src = r##"<svg xmlns="http://www.w3.org/2000/svg" fill="none"><path fill="#fff" d="M0 0"/><path d="M1 1"/></svg>"##;
		let out = strip_unused(src, false).unwrap();
		assert_eq!(out, src);
	}

	/// A group that sets a `fill` covers every shape inside it.
	#[test]
	fn a_closer_group_overrides_the_root() {
		let src = r#"<svg xmlns="http://www.w3.org/2000/svg" fill="none"><g fill="red"><path d="M0 0"/></g></svg>"#;
		let out = strip_unused(src, false).unwrap();
		assert!(
			out.contains(r#"<svg xmlns="http://www.w3.org/2000/svg">"#),
			"{out}"
		);
		assert!(out.contains(r#"<g fill="red">"#), "{out}");
	}

	/// `inherit` reads the `fill` of the parent.
	#[test]
	fn an_inherit_keeps_the_parent_fill() {
		let src = r#"<svg xmlns="http://www.w3.org/2000/svg" fill="none"><path fill="inherit" d="M0 0"/></svg>"#;
		let out = strip_unused(src, false).unwrap();
		assert_eq!(out, src);
	}

	/// The content of a `<use>` inherits from the `<use>`.
	#[test]
	fn a_use_keeps_the_fill() {
		let src = r##"<svg xmlns="http://www.w3.org/2000/svg" fill="none"><defs><path id="p" fill="red" d="M0 0"/></defs><use href="#p"/></svg>"##;
		let out = strip_unused(src, false).unwrap();
		assert_eq!(out, src);
	}

	#[test]
	fn a_stylesheet_keeps_the_fill() {
		let src = r#"<svg xmlns="http://www.w3.org/2000/svg" fill="none"><style>path{fill:inherit}</style><path fill="red" d="M0 0"/></svg>"#;
		let out = strip_unused(src, true).unwrap();
		assert_eq!(out, src);
	}
}
