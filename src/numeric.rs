// src/numeric.rs

//! Numeric formatting and length/number parsing shared by the coordinate-baking commands (`svg`
//! resize and `round`).

use anyhow::{Result, bail};

/// Format a scaled number: round to 6 decimals, trim, avoid "-0".
pub fn fmt_num(value: f64) -> String {
	let rounded = (value * 1_000_000.0).round() / 1_000_000.0;
	let rounded = if rounded == 0.0 { 0.0 } else { rounded };
	return format!("{rounded}");
}

/// Split a length into `(number, unit)`; `unit` is `""` for unitless values.
pub fn split_num_unit(value: &str) -> Option<(f64, &str)> {
	let value = value.trim();
	let end = value
		.find(|ch: char| {
			return !(ch.is_ascii_digit() || matches!(ch, '.' | '-' | '+' | 'e' | 'E'));
		})
		.unwrap_or(value.len());
	let (num, unit) = value.split_at(end);
	let n: f64 = num.parse().ok()?;
	return Some((n, unit.trim()));
}

/// Scale a single length; only unitless and `px` values are scaled.
pub fn scale_length(value: &str, scale: f64) -> String {
	match split_num_unit(value) {
		Some((n, "")) => return fmt_num(n * scale),
		Some((n, "px")) => return format!("{}px", fmt_num(n * scale)),
		_ => return value.to_owned(),
	}
}

/// Apply an affine `scale*n + offset` to a single length (position component); only unitless and
/// `px` values are transformed.
pub fn affine_length(value: &str, scale: f64, offset: f64) -> String {
	match split_num_unit(value) {
		Some((n, "")) => return fmt_num(scale * n + offset),
		Some((n, "px")) => return format!("{}px", fmt_num(scale * n + offset)),
		_ => return value.to_owned(),
	}
}

/// Parse a length that must be unitless or `px` (e.g. root width/height).
pub fn parse_px(value: &str) -> Result<f64> {
	match split_num_unit(value) {
		Some((n, "" | "px")) => return Ok(n),
		_ => bail!("unsupported length '{value}' (need unitless or px, or add a viewBox)"),
	}
}

/// Parse a whitespace/comma separated list of numbers, dropping non-numbers.
pub fn parse_num_list(value: &str) -> Vec<f64> {
	return value
		.split([',', ' ', '\t', '\n', '\r'])
		.filter(|token| return !token.is_empty())
		.filter_map(|token| return token.parse::<f64>().ok())
		.collect();
}

/// Scale a list of lengths (e.g. `stroke-dasharray`) by `scale`.
pub fn scale_len_list(value: &str, scale: f64) -> String {
	return value
		.split([',', ' ', '\t', '\n', '\r'])
		.filter(|token| return !token.is_empty())
		.map(|token| return scale_length(token, scale))
		.collect::<Vec<_>>()
		.join(" ");
}
