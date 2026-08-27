//! Remove `<image>` from an SVG document.
//!
//! resvg is built without its `raster-images` feature (see `Cargo.toml`), so a bitmap never
//! decodes: it does not rasterize, and it does not contribute to the bounding box `--round`
//! measures. An `<image>` left in place would survive into the `--svg` output and be missing from
//! `--png`, `--ico` and `--icns`, so the targets would disagree about what the artwork is.
//!
//! Unlike text, an image cannot be converted to paths by svgy, so the caller stops. `--strip-images`
//! removes the element instead and converts the vector artwork that remains.

use anyhow::{Context, Result};
use quick_xml::events::{BytesStart, Event};
use quick_xml::reader::Reader;
use quick_xml::writer::Writer;

/// Strip every `<image>` element from `src`.
///
/// Returns the rewritten document and whether anything was removed. Every `<image>` counts, whatever
/// its `href` holds: a data URI, a file path or an SVG. A namespace prefix (`<svg:image>`) is matched
/// on the local name.
pub fn strip_images(src: &str) -> Result<(String, bool)> {
	let mut reader = Reader::from_str(src);
	let mut writer = Writer::new(Vec::new());
	let mut removed = false;

	loop {
		match reader.read_event().context("parsing SVG")? {
			Event::Eof => break,
			// Skipping to the matching end tag takes the whole subtree with it.
			Event::Start(e) if is_image(&e) => {
				reader
					.read_to_end(e.name())
					.context("unterminated <image> element")?;
				removed = true;
			}
			Event::Empty(e) if is_image(&e) => removed = true,
			other => writer.write_event(other)?,
		}
	}

	Ok((String::from_utf8(writer.into_inner())?, removed))
}

fn is_image(e: &BytesStart) -> bool {
	e.local_name().as_ref() == "image"
}

#[cfg(test)]
mod tests {
	use super::*;

	/// The image goes, its siblings stay.
	#[test]
	fn strips_images_and_keeps_the_artwork() {
		let src = r#"<svg xmlns="http://www.w3.org/2000/svg"><path d="M0 0h8v8z"/><image x="1" y="2" href="data:image/png;base64,iVBORw0KGgo="/><circle r="4"/></svg>"#;
		let (out, removed) = strip_images(src).unwrap();
		assert!(removed);
		assert!(!out.contains("<image"));
		assert!(!out.contains("base64"));
		assert!(out.contains(r#"<path d="M0 0h8v8z"/>"#));
		assert!(out.contains(r#"<circle r="4"/>"#));
	}

	/// An `<image>` hides anywhere an element can, such as inside a `<pattern>` in `<defs>`.
	#[test]
	fn strips_a_nested_image_with_its_children() {
		let src = r#"<svg><defs><pattern id="p"><image href="a.png"><title>t</title></image></pattern></defs><rect fill="url(#p)"/></svg>"#;
		let (out, removed) = strip_images(src).unwrap();
		assert!(removed);
		assert_eq!(
			out,
			r#"<svg><defs><pattern id="p"></pattern></defs><rect fill="url(#p)"/></svg>"#
		);
	}

	#[test]
	fn strips_empty_and_prefixed_images() {
		let (out, removed) = strip_images(r#"<svg><image href="a.png"/><rect/></svg>"#).unwrap();
		assert!(removed);
		assert_eq!(out, "<svg><rect/></svg>");

		let (out, removed) =
			strip_images(r#"<svg:svg><svg:image href="a.png"/><svg:rect/></svg:svg>"#).unwrap();
		assert!(removed);
		assert_eq!(out, "<svg:svg><svg:rect/></svg:svg>");
	}

	/// Path artwork is the common case: nothing removed, and neither an `image-rendering` attribute
	/// nor a lookalike element name is a false hit.
	#[test]
	fn path_artwork_is_left_alone() {
		let src = r##"<svg xmlns="http://www.w3.org/2000/svg" image-rendering="optimizeSpeed"><path d="M0 0h8v8z" fill="#000"/></svg>"##;
		let (out, removed) = strip_images(src).unwrap();
		assert!(!removed);
		assert_eq!(out, src);
	}

	/// An unterminated `<image>` is a broken document, and reported as one rather than silently
	/// swallowing the rest of the file.
	#[test]
	fn unterminated_image_is_an_error() {
		assert!(strip_images(r#"<svg><image href="a.png">"#).is_err());
	}
}
