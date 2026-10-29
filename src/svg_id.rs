// src/svg_id.rs

//! Drop the `id` of the root element when nothing names it.
//!
//! An editor writes an `id` on the `<svg>` element itself, and `cleanup_ids` keeps that one. It
//! costs more than its own bytes: `remove_useless_stroke_and_fill` stops at an element that carries
//! an `id`, and the root carries every other element, so one dead `id` switches the job off for the
//! whole document.

use anyhow::{Context as _, Result, anyhow};
use quick_xml::events::{BytesStart, Event};
use quick_xml::reader::Reader;
use quick_xml::writer::Writer;

/// Remove the `id` of the root element when the document does not name it.
///
/// `dynamic` says the document has a `<style>` or a `<script>`, which can name an `id` in a way
/// this scan cannot follow. The `id` then stays.
pub fn strip_dead_root_id(src: &str, dynamic: bool) -> Result<String> {
	if dynamic {
		return Ok(src.to_owned());
	}
	let Some(id) = root_id(src)? else {
		return Ok(src.to_owned());
	};
	if is_named(src, &id)? {
		return Ok(src.to_owned());
	}

	let mut reader = Reader::from_str(src);
	let mut writer = Writer::new(Vec::new());
	let mut root_seen = false;

	loop {
		let event = reader.read_event().context("parsing SVG")?;
		let is_root = !root_seen && matches!(event, Event::Start(_) | Event::Empty(_));
		root_seen |= is_root;

		match event {
			Event::Eof => break,
			Event::Start(tag) if is_root => writer.write_event(Event::Start(without_id(&tag)?))?,
			Event::Empty(tag) if is_root => writer.write_event(Event::Empty(without_id(&tag)?))?,
			other => writer.write_event(other)?,
		}
	}

	return Ok(String::from_utf8(writer.into_inner())?);
}

/// The `id` of the first element, which is the root.
fn root_id(src: &str) -> Result<Option<String>> {
	let mut reader = Reader::from_str(src);
	loop {
		let tag = match reader.read_event().context("parsing SVG")? {
			Event::Eof => return Ok(None),
			Event::Start(tag) | Event::Empty(tag) => tag,
			_ => continue,
		};
		for attr in tag.attributes() {
			let attr = attr.map_err(|err| anyhow!("attribute: {err}"))?;
			if attr.key.into_inner() == "id" {
				return Ok(Some(
					attr.normalized_value(quick_xml::XmlVersion::Implicit1_0)?
						.into_owned(),
				));
			}
		}
		return Ok(None);
	}
}

/// Whether an attribute or a text names `id`.
///
/// The `id` attributes are declarations, not uses. An attribute with a prefix belongs to an editor
/// (`inkscape:current-layer`) or declares a namespace, and only `xlink:href` can point to an
/// element. Text is read too, since a `<style>` can name an `id`.
pub fn is_named(src: &str, id: &str) -> Result<bool> {
	let mut reader = Reader::from_str(src);
	loop {
		match reader.read_event().context("parsing SVG")? {
			Event::Eof => return Ok(false),
			Event::Start(tag) | Event::Empty(tag) => {
				for attr in tag.attributes() {
					let attr = attr.map_err(|err| anyhow!("attribute: {err}"))?;
					let key = attr.key.into_inner();
					if !can_point(key) {
						continue;
					}
					let val = attr.normalized_value(quick_xml::XmlVersion::Implicit1_0)?;
					if mentions(&val, id) {
						return Ok(true);
					}
				}
			}
			Event::Text(text) if mentions(&text, id) => return Ok(true),
			Event::CData(text) if mentions(&text, id) => return Ok(true),
			_ => {}
		}
	}
}

/// Whether the attribute `key` can name an element.
fn can_point(key: &str) -> bool {
	match key.split_once(':') {
		Some((prefix, _)) => return prefix == "xlink",
		None => return key != "id" && key != "xmlns",
	}
}

/// Whether `text` holds `id` as a whole name.
///
/// A name goes on while the characters an `id` accepts go on, so `svg1` is no mention of `svg`. A
/// dot ends a name, since `begin="svg1.click"` names the element before it.
fn mentions(text: &str, id: &str) -> bool {
	let mut rest = text;
	while let Some((head, after)) = rest.split_once(id) {
		let before = head.chars().next_back();
		if !is_name_char(before) && !is_name_char(after.chars().next()) {
			return true;
		}
		rest = after;
	}
	return false;
}

/// Whether the character can be in an `id`.
fn is_name_char(chr: Option<char>) -> bool {
	return chr.is_some_and(|chr| return chr.is_alphanumeric() || chr == '-' || chr == '_');
}

/// Copy of the element without `id`.
fn without_id(tag: &BytesStart) -> Result<BytesStart<'static>> {
	let mut out = BytesStart::new(tag.name().into_inner().to_owned());

	for attr in tag.attributes() {
		let attr = attr.map_err(|err| anyhow!("attribute: {err}"))?;
		let key = attr.key.into_inner();
		if key == "id" {
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
	fn a_dead_root_id_goes() {
		let src = r#"<svg xmlns="http://www.w3.org/2000/svg" id="svg2423"><path d="M0 0"/></svg>"#;
		let out = strip_dead_root_id(src, false).unwrap();
		assert!(!out.contains("id="), "{out}");
		assert!(out.contains(r#"d="M0 0""#), "{out}");
	}

	/// Only the root is looked at: an `id` further down can be a target of `<use>`.
	#[test]
	fn an_id_below_the_root_stays() {
		let src = r#"<svg xmlns="http://www.w3.org/2000/svg" id="a"><path id="b" d="M0 0"/></svg>"#;
		let out = strip_dead_root_id(src, false).unwrap();
		assert!(!out.contains(r#"id="a""#), "{out}");
		assert!(out.contains(r#"id="b""#), "{out}");
	}

	#[test]
	fn a_used_root_id_stays() {
		let src = r##"<svg xmlns="http://www.w3.org/2000/svg" id="a"><use href="#a"/></svg>"##;
		let out = strip_dead_root_id(src, false).unwrap();
		assert!(out.contains(r#"id="a""#), "{out}");
	}

	/// A selector or a script can name the root, and neither is read here.
	#[test]
	fn a_dynamic_document_keeps_its_root_id() {
		let src = r#"<svg xmlns="http://www.w3.org/2000/svg" id="a"><path d="M0 0"/></svg>"#;
		let out = strip_dead_root_id(src, true).unwrap();
		assert!(out.contains(r#"id="a""#), "{out}");
	}

	/// A longer name that starts with the `id` is no mention of it.
	#[test]
	fn a_longer_name_is_no_mention() {
		let src = r##"<svg xmlns="http://www.w3.org/2000/svg" id="a"><use href="#ab"/></svg>"##;
		let out = strip_dead_root_id(src, false).unwrap();
		assert!(!out.contains(r#"id="a""#), "{out}");
	}

	/// An animation names the element before the dot.
	#[test]
	fn a_dot_ends_a_name() {
		let src =
			r#"<svg xmlns="http://www.w3.org/2000/svg" id="a"><animate begin="a.click"/></svg>"#;
		let out = strip_dead_root_id(src, false).unwrap();
		assert!(out.contains(r#"id="a""#), "{out}");
	}

	/// A document without a root `id` comes back as it is.
	#[test]
	fn a_document_without_a_root_id_is_untouched() {
		let src = r#"<svg xmlns="http://www.w3.org/2000/svg"><path d="M0 0"/></svg>"#;
		assert_eq!(strip_dead_root_id(src, false).unwrap(), src);
	}

	/// An editor attribute names the file, not the element.
	#[test]
	fn an_editor_attribute_is_no_use() {
		let src = r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:sodipodi="http://sodipodi.sourceforge.net/DTD/sodipodi-0.dtd" xmlns:inkscape="http://www.inkscape.org/namespaces/inkscape" id="dev" sodipodi:docname="dev.svg" inkscape:current-layer="dev"><path d="M0 0"/></svg>"#;
		let out = strip_dead_root_id(src, false).unwrap();
		assert!(!out.contains(r#"id="dev""#), "{out}");
	}

	/// The namespace URL ends with `svg`, which is no use.
	#[test]
	fn a_namespace_is_no_use() {
		let src = r#"<svg xmlns="http://www.w3.org/2000/svg" id="svg"><path d="M0 0"/></svg>"#;
		let out = strip_dead_root_id(src, false).unwrap();
		assert!(!out.contains(r#"id="svg""#), "{out}");
	}

	#[test]
	fn an_xlink_href_is_a_use() {
		let src = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" id="a"><use xlink:href="#a"/></svg>"##;
		let out = strip_dead_root_id(src, false).unwrap();
		assert!(out.contains(r#"id="a""#), "{out}");
	}
}
