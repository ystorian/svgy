// src/svg_display.rs

//! Drop a `display` that holds its initial value.
//!
//! Inkscape writes `style="display:inline"` on every layer. `inline` is the initial value of
//! `display`, but oxvg gives `display` no default, so `remove_unknowns_and_defaults` keeps it. A
//! group that keeps an attribute is a group that `collapse_groups` cannot flatten.
//!
//! `display` is not inherited, so the initial value is what the element gets without it.

use anyhow::{Context as _, Result, anyhow};
use quick_xml::events::{BytesStart, Event};
use quick_xml::reader::Reader;
use quick_xml::writer::Writer;

/// The initial value of `display`.
const INITIAL: &str = "inline";

/// Remove every `display:inline` of `src`, as an attribute or in a `style`.
///
/// `dynamic` says the document has a `<style>` or a `<script>`. A rule of the stylesheet can set
/// another `display`, and a `style` attribute beats that rule, so the document stays as it is.
pub fn strip_inline(src: &str, dynamic: bool) -> Result<String> {
	if dynamic {
		return Ok(src.to_owned());
	}
	let mut reader = Reader::from_str(src);
	let mut writer = Writer::new(Vec::new());

	loop {
		match reader.read_event().context("parsing SVG")? {
			Event::Eof => break,
			Event::Start(elem) => writer.write_event(Event::Start(stripped(&elem)?))?,
			Event::Empty(elem) => writer.write_event(Event::Empty(stripped(&elem)?))?,
			other => writer.write_event(other)?,
		}
	}

	return Ok(String::from_utf8(writer.into_inner())?);
}

/// Rewrite one element without its initial `display`.
fn stripped(elem: &BytesStart) -> Result<BytesStart<'static>> {
	let mut out = BytesStart::new(elem.name().into_inner().to_owned());

	for attr in elem.attributes() {
		let attr = attr.map_err(|err| anyhow!("attribute: {err}"))?;
		let key = attr.key.into_inner();
		let val = attr
			.normalized_value(quick_xml::XmlVersion::Implicit1_0)?
			.into_owned();
		match key {
			"display" if val.trim() == INITIAL => {}
			"style" => {
				let style = without_display(&val);
				// An empty `style` is dropped with its last declaration.
				if !style.is_empty() {
					out.push_attribute((key, style.as_str()));
				}
			}
			_ => out.push_attribute((key, val.as_str())),
		}
	}
	return Ok(out);
}

/// One `style` value without its initial `display`.
fn without_display(style: &str) -> String {
	return style
		.split(';')
		.map(str::trim)
		.filter(|part| return !part.is_empty())
		.filter(|declaration| {
			return !declaration.split_once(':').is_some_and(|(name, value)| {
				return name.trim().eq_ignore_ascii_case("display") && value.trim() == INITIAL;
			});
		})
		.collect::<Vec<_>>()
		.join(";");
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn an_inline_style_is_dropped() {
		let src = r#"<svg xmlns="http://www.w3.org/2000/svg"><g style="display:inline"><path d="M0 0"/></g></svg>"#;
		let out = strip_inline(src, false).unwrap();
		assert!(out.contains("<g>"), "{out}");
	}

	#[test]
	fn an_inline_attribute_is_dropped() {
		let src = r#"<svg xmlns="http://www.w3.org/2000/svg"><g display="inline"><path d="M0 0"/></g></svg>"#;
		let out = strip_inline(src, false).unwrap();
		assert!(out.contains("<g>"), "{out}");
	}

	/// Every other declaration is untouched.
	#[test]
	fn other_declarations_are_kept() {
		let src = r#"<svg xmlns="http://www.w3.org/2000/svg"><g style="display:inline;opacity:0.5"/></svg>"#;
		let out = strip_inline(src, false).unwrap();
		assert!(out.contains(r#"style="opacity:0.5""#), "{out}");
	}

	/// `none` hides the element, so it must stay.
	#[test]
	fn a_hidden_element_is_kept() {
		let src = r#"<svg xmlns="http://www.w3.org/2000/svg"><g style="display:none"/><g display="none"/></svg>"#;
		let out = strip_inline(src, false).unwrap();
		assert!(out.contains(r#"style="display:none""#), "{out}");
		assert!(out.contains(r#"display="none""#), "{out}");
	}

	/// A stylesheet can set another `display`, which the `style` overrides.
	#[test]
	fn a_stylesheet_keeps_the_display() {
		let src = r#"<svg xmlns="http://www.w3.org/2000/svg"><style>g{display:none}</style><g style="display:inline"/></svg>"#;
		let out = strip_inline(src, true).unwrap();
		assert_eq!(out, src);
	}
}
