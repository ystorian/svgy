//! Numeric formatting and length/number parsing shared by the coordinate-baking commands (`svg`
//! resize and `round`).

use anyhow::{Result, bail};

/// Format a scaled number: round to 6 decimals, trim, avoid "-0".
pub fn fmt_num(v: f64) -> String {
	let r = (v * 1_000_000.0).round() / 1_000_000.0;
	let r = if r == 0.0 { 0.0 } else { r };
	format!("{r}")
}

/// Split a length into `(number, unit)`; `unit` is `""` for unitless values.
pub fn split_num_unit(v: &str) -> Option<(f64, &str)> {
	let v = v.trim();
	let end = v
		.find(|c: char| !(c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E')))
		.unwrap_or(v.len());
	let (num, unit) = v.split_at(end);
	let n: f64 = num.parse().ok()?;
	Some((n, unit.trim()))
}

/// Scale a single length; only unitless and `px` values are scaled.
pub fn scale_length(v: &str, s: f64) -> String {
	match split_num_unit(v) {
		Some((n, "")) => fmt_num(n * s),
		Some((n, "px")) => format!("{}px", fmt_num(n * s)),
		_ => v.to_string(),
	}
}

/// Apply an affine `s*n + t` to a single length (position component); only unitless and `px` values
/// are transformed.
pub fn affine_length(v: &str, s: f64, t: f64) -> String {
	match split_num_unit(v) {
		Some((n, "")) => fmt_num(s * n + t),
		Some((n, "px")) => format!("{}px", fmt_num(s * n + t)),
		_ => v.to_string(),
	}
}

/// Parse a length that must be unitless or `px` (e.g. root width/height).
pub fn parse_px(v: &str) -> Result<f64> {
	match split_num_unit(v) {
		Some((n, "" | "px")) => Ok(n),
		_ => bail!("unsupported length '{v}' (need unitless or px, or add a viewBox)"),
	}
}

/// Parse a whitespace/comma separated list of numbers, dropping non-numbers.
pub fn parse_num_list(v: &str) -> Vec<f64> {
	v.split([',', ' ', '\t', '\n', '\r'])
		.filter(|t| !t.is_empty())
		.filter_map(|t| t.parse::<f64>().ok())
		.collect()
}

/// Scale a list of lengths (e.g. `stroke-dasharray`) by `s`.
pub fn scale_len_list(v: &str, s: f64) -> String {
	v.split([',', ' ', '\t', '\n', '\r'])
		.filter(|t| !t.is_empty())
		.map(|t| scale_length(t, s))
		.collect::<Vec<_>>()
		.join(" ")
}
