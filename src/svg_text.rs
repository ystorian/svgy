//! Remove `<text>` from an SVG document.
//!
//! resvg is built without its `text` feature (see `Cargo.toml`), so glyphs are never shaped: they
//! do not rasterize, and they do not contribute to the bounding box `--round` measures. Rather than
//! let each target disagree about what the artwork is, text is stripped once, up front, so the
//! `--svg` output shows exactly what the raster targets and the round fit see.
//!
//! Text is a property of the source rather than an error, so the caller warns and converts the
//! remaining artwork.

use anyhow::{Context, Result};
use quick_xml::events::{BytesStart, Event};
use quick_xml::reader::Reader;
use quick_xml::writer::Writer;

/// Strip every `<text>` element from `src`.
///
/// Returns the rewritten document and whether anything was removed. Children of a `<text>`
/// (`<tspan>`, `<textPath>`) go with their parent, and a namespace prefix (`<svg:text>`) is matched
/// on the local name.
pub fn strip_text(src: &str) -> Result<(String, bool)> {
	let mut reader = Reader::from_str(src);
	let mut writer = Writer::new(Vec::new());
	let mut removed = false;

	loop {
		match reader.read_event().context("parsing SVG")? {
			Event::Eof => break,
			// Skipping to the matching end tag takes the whole subtree with it.
			Event::Start(e) if is_text(&e) => {
				reader
					.read_to_end(e.name())
					.context("unterminated <text> element")?;
				removed = true;
			}
			Event::Empty(e) if is_text(&e) => removed = true,
			other => writer.write_event(other)?,
		}
	}

	Ok((String::from_utf8(writer.into_inner())?, removed))
}

fn is_text(e: &BytesStart) -> bool {
	e.local_name().as_ref() == "text"
}

#[cfg(test)]
mod tests {
	use super::*;

	/// The text goes, its siblings stay.
	#[test]
	fn strips_text_and_keeps_the_artwork() {
		let src = r#"<svg xmlns="http://www.w3.org/2000/svg"><path d="M0 0h8v8z"/><text x="1" y="2">Hi</text><circle r="4"/></svg>"#;
		let (out, removed) = strip_text(src).unwrap();
		assert!(removed);
		assert!(!out.contains("<text"));
		assert!(!out.contains("Hi"));
		assert!(out.contains(r#"<path d="M0 0h8v8z"/>"#));
		assert!(out.contains(r#"<circle r="4"/>"#));
	}

	/// `<tspan>` and `<textPath>` live inside `<text>` and leave with it.
	#[test]
	fn strips_nested_text_children() {
		let src =
			r##"<svg><text><tspan>a</tspan><textPath href="#p">b</textPath></text><rect/></svg>"##;
		let (out, removed) = strip_text(src).unwrap();
		assert!(removed);
		assert_eq!(out, "<svg><rect/></svg>");
	}

	#[test]
	fn strips_empty_and_prefixed_text() {
		let (out, removed) = strip_text(r#"<svg><text x="1"/><rect/></svg>"#).unwrap();
		assert!(removed);
		assert_eq!(out, "<svg><rect/></svg>");

		let (out, removed) =
			strip_text(r"<svg:svg><svg:text>Hi</svg:text><svg:rect/></svg:svg>").unwrap();
		assert!(removed);
		assert_eq!(out, "<svg:svg><svg:rect/></svg:svg>");
	}

	/// Path artwork is the common case: nothing removed, nothing to warn about, and neither a
	/// `font-size` nor a lookalike element name is a false hit.
	#[test]
	fn path_artwork_is_left_alone() {
		let src = r##"<svg xmlns="http://www.w3.org/2000/svg" font-size="12"><path d="M0 0h8v8z" fill="#000"/></svg>"##;
		let (out, removed) = strip_text(src).unwrap();
		assert!(!removed);
		assert_eq!(out, src);
	}

	/// An unterminated `<text>` is a broken document, and reported as one rather than silently
	/// swallowing the rest of the file.
	#[test]
	fn unterminated_text_is_an_error() {
		assert!(strip_text("<svg><text>unclosed").is_err());
	}
}
