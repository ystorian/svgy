// src/svg_path.rs

//! Round path data without drift.
//!
//! `convert_path_data` rounds each relative command on its own. A path reads its relative commands
//! one after the other, so the errors add up: on a path of 300 points at one decimal, a point moves
//! by more than a unit. SVGO measures each relative command from the rounded point before it, which
//! keeps every point within half a step of where it was.
//!
//! oxvg therefore rounds path data with extra decimals, and this pass brings it down to the
//! precision of the search. The next oxvg run drops the segments that rounding made empty.

use anyhow::{Context as _, Result, anyhow};
use oxvg_path::{Path, command::Data, parser::Parse as _};
use quick_xml::events::{BytesStart, Event};
use quick_xml::reader::Reader;
use quick_xml::writer::Writer;

/// Round the `d` of every element in `src` to `precision` decimals.
pub fn round_paths(src: &str, precision: i32) -> Result<String> {
	let mut reader = Reader::from_str(src);
	let mut writer = Writer::new(Vec::new());

	loop {
		match reader.read_event().context("parsing SVG")? {
			Event::Eof => break,
			Event::Start(tag) => writer.write_event(Event::Start(rounded(&tag, precision)?))?,
			Event::Empty(tag) => writer.write_event(Event::Empty(rounded(&tag, precision)?))?,
			other => writer.write_event(other)?,
		}
	}

	return Ok(String::from_utf8(writer.into_inner())?);
}

/// Rewrite one element with its `d` rounded.
fn rounded(tag: &BytesStart, precision: i32) -> Result<BytesStart<'static>> {
	let mut out = BytesStart::new(tag.name().into_inner().to_owned());

	for attr in tag.attributes() {
		let attr = attr.map_err(|err| anyhow!("attribute: {err}"))?;
		let key = attr.key.into_inner();
		let val = attr
			.normalized_value(quick_xml::XmlVersion::Implicit1_0)?
			.into_owned();
		match key {
			"d" => out.push_attribute((key, round_d(&val, precision).as_str())),
			_ => out.push_attribute((key, val.as_str())),
		}
	}
	return Ok(out);
}

/// One `d`, rounded. A `d` that does not parse stays as it is.
fn round_d(data: &str, precision: i32) -> String {
	let Ok(mut path) = Path::parse_string(data) else {
		return data.to_owned();
	};
	let scale = 10_f64.powi(precision);
	let round = |value: f64| {
		let out = (value * scale).round() / scale;
		// A `-0` costs a byte for nothing.
		if out == 0.0 {
			return 0.0;
		}
		return out;
	};

	// `exact` is where the source is, `written` is where the output is.
	let mut exact = [0.0, 0.0];
	let mut written = [0.0, 0.0];
	let mut start = (exact, written);

	for data in &mut path.0 {
		let data = data.as_explicit_mut();
		let relative = data.is_by();
		let is_move = matches!(data, Data::MoveTo(_) | Data::MoveBy(_));
		let is_close = matches!(data, Data::ClosePath);
		let axis = match data {
			Data::HorizontalLineTo(_) | Data::HorizontalLineBy(_) => Some(0),
			Data::VerticalLineTo(_) | Data::VerticalLineBy(_) => Some(1),
			_ => None,
		};
		if is_close {
			(exact, written) = start;
			continue;
		}

		let args = data.args_mut();
		// An arc has radii, a rotation and two flags before its point.
		let first_point = if args.len() == 7 { 5 } else { 0 };
		for value in &mut args[..first_point.min(3)] {
			*value = round(*value);
		}

		// Every value of a relative command is measured from the start of the segment.
		let (from_exact, from_written) = (exact, written);
		let mut place = |value: &mut f64, i: usize| {
			let target = if relative {
				from_exact[i] + *value
			} else {
				*value
			};
			let base = if relative { from_written[i] } else { 0.0 };
			*value = round(target - base);
			// The last value on an axis is the end of the segment.
			exact[i] = target;
			written[i] = base + *value;
		};
		match axis {
			Some(i) => place(&mut args[0], i),
			None => {
				for pair in args[first_point..].as_chunks_mut::<2>().0 {
					place(&mut pair[0], 0);
					place(&mut pair[1], 1);
				}
			}
		}
		if is_move {
			start = (exact, written);
		}
	}
	return path.to_string();
}

#[cfg(test)]
mod tests {
	use super::*;

	/// The end point of a run of relative commands, read the way a renderer reads it.
	fn end(data: &str) -> [f64; 2] {
		let path = Path::parse_string(data).unwrap();
		let mut at = [0.0, 0.0];
		for data in &path.0 {
			match data.as_explicit() {
				Data::MoveTo(point) | Data::LineTo(point) => at = *point,
				Data::MoveBy(step) | Data::LineBy(step) => at = [at[0] + step[0], at[1] + step[1]],
				_ => panic!("unexpected command"),
			}
		}
		return at;
	}

	/// Ten steps of 0.14 round to 0.1 each, and that drift is what this pass removes.
	#[test]
	fn relative_steps_do_not_drift() {
		let data = format!("M0 0{}", "l.14 .14".repeat(10));
		let out = round_d(&data, 1);
		let [x, y] = end(&out);
		assert!((x - 1.4).abs() < 0.051, "{out}");
		assert!((y - 1.4).abs() < 0.051, "{out}");
	}

	#[test]
	fn absolute_values_are_rounded() {
		assert_eq!(round_d("M1.234 5.678L9.876 5.432", 1), "M1.2 5.7 9.9 5.4");
	}

	/// The flags of an arc are no numbers to round.
	#[test]
	fn arc_flags_stay() {
		let out = round_d("M0 0a5.55 5.55 0 1 0 10.04 0", 1);
		assert!(out.contains("1 0"), "{out}");
		assert!(out.ends_with("10 0"), "{out}");
	}

	/// After `z`, a relative command starts from the start of the subpath.
	#[test]
	fn close_returns_to_start() {
		let out = round_d("M.04 0l.14 0l0 .14zl.14 .14", 1);
		// The start is written at 0, so `.18` becomes `.2`.
		assert_eq!(out, "M0 0l.2 0 0 .1Zl.2.1");
	}

	#[test]
	fn other_attributes_stay() {
		let src =
			r#"<svg xmlns="http://www.w3.org/2000/svg"><path fill="red" d="M1.26 0h1.26"/></svg>"#;
		let out = round_paths(src, 1).unwrap();
		assert!(out.contains(r#"fill="red""#), "{out}");
		assert!(out.contains(r#"d="M1.3 0h1.2""#), "{out}");
	}
}
