//! Round the opacity declarations of a `style` attribute.
//!
//! `cleanup_numeric_values` rounds attributes, and `convert_style_to_attrs` moves a declaration to
//! an attribute only when it maps to one. `stop-opacity` maps to none of them, so a value such as
//! `stop-opacity:0.98039216` reaches the output with every decimal an editor wrote.
//!
//! An opacity holds a fraction of 0 to 1, so three decimals name every step the eye can tell apart.
//! A length has no such limit, and the precision search is what decides those.

use anyhow::{Context, Result, anyhow};
use quick_xml::events::{BytesStart, Event};
use quick_xml::reader::Reader;
use quick_xml::writer::Writer;

use crate::numeric::{fmt_num, split_num_unit};

/// How many decimals an opacity keeps.
const DECIMALS: i32 = 3;

/// The properties that hold a fraction of 0 to 1.
const OPACITIES: &[&str] = &[
	"opacity",
	"fill-opacity",
	"stroke-opacity",
	"stop-opacity",
	"flood-opacity",
];

/// Round every opacity of every `style` attribute in `src`.
pub fn round_opacities(src: &str) -> Result<String> {
	let mut reader = Reader::from_str(src);
	let mut writer = Writer::new(Vec::new());

	loop {
		match reader.read_event().context("parsing SVG")? {
			Event::Eof => break,
			Event::Start(e) => writer.write_event(Event::Start(rounded(&e)?))?,
			Event::Empty(e) => writer.write_event(Event::Empty(rounded(&e)?))?,
			other => writer.write_event(other)?,
		}
	}

	Ok(String::from_utf8(writer.into_inner())?)
}

/// Rewrite one element with its opacities rounded.
fn rounded(e: &BytesStart) -> Result<BytesStart<'static>> {
	let mut out = BytesStart::new(e.name().into_inner().to_string());

	for attr in e.attributes() {
		let attr = attr.map_err(|err| anyhow!("attribute: {err}"))?;
		let key = attr.key.into_inner();
		let val = attr
			.normalized_value(quick_xml::XmlVersion::Implicit1_0)?
			.into_owned();
		if key == "style" {
			out.push_attribute((key, round_declarations(&val).as_str()));
		} else {
			out.push_attribute((key, val.as_str()));
		}
	}
	Ok(out)
}

/// Round the opacities of one `style` value, and leave every other declaration as it is.
fn round_declarations(style: &str) -> String {
	style
		.split(';')
		.map(str::trim)
		.filter(|d| !d.is_empty())
		.map(|declaration| match declaration.split_once(':') {
			Some((name, value)) => round_one(name.trim(), value.trim())
				.unwrap_or_else(|| format!("{}:{}", name.trim(), value.trim())),
			None => declaration.to_string(),
		})
		.collect::<Vec<_>>()
		.join(";")
}

/// One declaration, rounded. `None` when this one keeps its value.
///
/// A percentage and a keyword are left alone: only a plain number is a fraction to round.
fn round_one(name: &str, value: &str) -> Option<String> {
	if !OPACITIES.contains(&name.to_ascii_lowercase().as_str()) {
		return None;
	}
	let (number, "") = split_num_unit(value)? else {
		return None;
	};

	let scale = f64::from(10_i32.pow(u32::try_from(DECIMALS).ok()?));
	let short = fmt_num((number * scale).round() / scale);
	// A value that is already short keeps its own spelling.
	(short.len() < value.len()).then(|| format!("{name}:{short}"))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_long_opacity_is_rounded() {
		let src = r#"<svg xmlns="http://www.w3.org/2000/svg"><stop style="stop-color:#52c0ee;stop-opacity:0.98039216"/></svg>"#;
		let out = round_opacities(src).unwrap();
		assert!(out.contains("stop-opacity:0.98"), "{out}");
		// Every other declaration is untouched.
		assert!(out.contains("stop-color:#52c0ee"), "{out}");
	}

	/// A length is what the precision search decides, so it keeps every decimal here.
	#[test]
	fn a_length_is_left_alone() {
		let src = r#"<svg xmlns="http://www.w3.org/2000/svg"><path style="stroke-width:0.98039216" d="M0 0"/></svg>"#;
		let out = round_opacities(src).unwrap();
		assert!(out.contains("stroke-width:0.98039216"), "{out}");
	}

	/// Rounding to three decimals must not turn a small opacity into nothing.
	#[test]
	fn a_small_opacity_stays_visible() {
		let src = r#"<svg xmlns="http://www.w3.org/2000/svg"><path style="opacity:0.0126" d="M0 0"/></svg>"#;
		let out = round_opacities(src).unwrap();
		assert!(out.contains("opacity:0.013"), "{out}");
	}

	#[test]
	fn a_percentage_is_left_alone() {
		let src = r#"<svg xmlns="http://www.w3.org/2000/svg"><path style="fill-opacity:98.039216%" d="M0 0"/></svg>"#;
		let out = round_opacities(src).unwrap();
		assert!(out.contains("fill-opacity:98.039216%"), "{out}");
	}

	/// A document without a `style` attribute comes back with its elements as they are.
	#[test]
	fn an_element_without_a_style_is_untouched() {
		let src = r#"<svg xmlns="http://www.w3.org/2000/svg"><path fill-opacity="0.98039216" d="M0 0"/></svg>"#;
		let out = round_opacities(src).unwrap();
		assert!(out.contains(r#"fill-opacity="0.98039216""#), "{out}");
	}
}
