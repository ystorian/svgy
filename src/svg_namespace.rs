//! Drop namespace declarations that nothing uses.
//!
//! Editors leave declarations like `xmlns:svg` behind, and the optimizer copies them onto every
//! element it writes. A prefix that no element or attribute name uses carries no meaning, so the
//! declaration goes. The default `xmlns` always stays: it is what makes the document an SVG.

use anyhow::{Context, Result, anyhow};
use quick_xml::events::{BytesStart, Event};
use quick_xml::reader::Reader;
use quick_xml::writer::Writer;
use std::collections::HashSet;

/// Remove every `xmlns:<prefix>` whose prefix is unused in `src`.
pub fn strip_unused(src: &str) -> Result<String> {
	let used = used_prefixes(src)?;
	let mut reader = Reader::from_str(src);
	let mut writer = Writer::new(Vec::new());

	loop {
		match reader.read_event().context("parsing SVG")? {
			Event::Eof => break,
			Event::Start(e) => writer.write_event(Event::Start(without_unused(&e, &used)?))?,
			Event::Empty(e) => writer.write_event(Event::Empty(without_unused(&e, &used)?))?,
			other => writer.write_event(other)?,
		}
	}

	Ok(String::from_utf8(writer.into_inner())?)
}

/// Prefixes that an element or attribute name spells out, `xlink:href` -> `xlink`.
fn used_prefixes(src: &str) -> Result<HashSet<String>> {
	let mut reader = Reader::from_str(src);
	let mut used = HashSet::new();

	loop {
		let e = match reader.read_event().context("parsing SVG")? {
			Event::Eof => break,
			Event::Start(e) | Event::Empty(e) => e,
			_ => continue,
		};
		if let Some(prefix) = prefix_of(e.name().into_inner()) {
			used.insert(prefix.to_string());
		}
		for attr in e.attributes() {
			let attr = attr.map_err(|err| anyhow!("attribute: {err}"))?;
			let key = attr.key.into_inner();
			// A declaration is not a use of the prefix it declares.
			if key.starts_with("xmlns:") {
				continue;
			}
			if let Some(prefix) = prefix_of(key) {
				used.insert(prefix.to_string());
			}
		}
	}
	Ok(used)
}

/// The prefix of a qualified name, `svg:path` -> `svg`.
fn prefix_of(name: &str) -> Option<&str> {
	name.split_once(':').map(|(prefix, _)| prefix)
}

fn without_unused(e: &BytesStart, used: &HashSet<String>) -> Result<BytesStart<'static>> {
	let mut out = BytesStart::new(e.name().into_inner().to_string());

	for attr in e.attributes() {
		let attr = attr.map_err(|err| anyhow!("attribute: {err}"))?;
		let key = attr.key.into_inner();
		if let Some(prefix) = key.strip_prefix("xmlns:")
			&& !used.contains(prefix)
		{
			continue;
		}
		let val = attr
			.normalized_value(quick_xml::XmlVersion::Implicit1_0)?
			.into_owned();
		out.push_attribute((key, val.as_str()));
	}
	Ok(out)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn an_unused_declaration_goes() {
		let src = r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:svg="http://www.w3.org/2000/svg"><path xmlns:svg="http://www.w3.org/2000/svg" d="M0 0"/></svg>"#;
		let out = strip_unused(src).unwrap();
		assert!(!out.contains("xmlns:svg"), "{out}");
		assert!(
			out.contains(r#"xmlns="http://www.w3.org/2000/svg""#),
			"{out}"
		);
		assert!(out.contains(r#"d="M0 0""#), "{out}");
	}

	#[test]
	fn a_used_declaration_stays() {
		let src = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink"><use xlink:href="#a"/></svg>"##;
		let out = strip_unused(src).unwrap();
		assert!(out.contains("xmlns:xlink"), "{out}");
	}

	/// A prefix is also used when it qualifies an element name.
	#[test]
	fn a_prefixed_element_keeps_its_declaration() {
		let src = r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:svg="http://www.w3.org/2000/svg"><svg:path d="M0 0"/></svg>"#;
		let out = strip_unused(src).unwrap();
		assert!(out.contains("xmlns:svg"), "{out}");
	}
}
