//! Write each element at its own precision.
//!
//! The precision search turns one knob for the whole document, so every element carries the
//! decimals that the worst element needs. The candidates the search already renders are the same
//! document at every precision, and a path carries its own numbers, so one element can take its
//! numbers from one candidate and its neighbour from another.
//!
//! The mix starts with every element at the lowest precision and buys decimals back where they pay.
//! Which element pays is estimated, not measured: the estimate only sets the order of the tries, and
//! a render of the whole document decides whether the result is inside the budget.

use anyhow::{Context, Result, anyhow};
use oxvg_path::{Path, command::Data, parser::Parse as _};
use quick_xml::events::Event;
use quick_xml::reader::Reader;

/// The most upgrades to try. Each one costs a render.
const MAX_STEPS: usize = 32;

/// The attributes that name another element. A mix must not move these.
const NAMES: &[&str] = &["id", "href", "xlink:href", "class"];

/// What the mix settled on.
pub struct Mixed {
	pub svg: String,
	/// Rendered difference against the source, 0..1.
	pub difference: f64,
	/// How many elements were written above the lowest precision.
	pub upgrades: usize,
}

/// The elements of one candidate.
struct Doc<'a> {
	src: &'a str,
	/// The span of each start tag, in document order.
	spans: Vec<(usize, usize)>,
	names: Vec<String>,
	/// The attribute names of each element, and the values that name another element.
	shapes: Vec<Vec<(String, Option<String>)>>,
	/// The nesting depth of each element.
	depths: Vec<usize>,
	/// The `d` of each element, when it has one.
	paths: Vec<Option<String>>,
}

impl<'a> Doc<'a> {
	fn parse(src: &'a str) -> Result<Self> {
		let mut reader = Reader::from_str(src);
		let mut doc = Self {
			src,
			spans: Vec::new(),
			names: Vec::new(),
			shapes: Vec::new(),
			depths: Vec::new(),
			paths: Vec::new(),
		};
		let mut depth: usize = 0;

		loop {
			let start = usize::try_from(reader.buffer_position())?;
			let event = reader.read_event().context("parsing SVG")?;
			let end = usize::try_from(reader.buffer_position())?;

			let (e, opens) = match event {
				Event::Eof => break,
				Event::Start(e) => (e, true),
				Event::Empty(e) => (e, false),
				Event::End(_) => {
					depth = depth.saturating_sub(1);
					continue;
				}
				_ => continue,
			};

			let mut shape = Vec::new();
			let mut d = None;
			for attr in e.attributes() {
				let attr = attr.map_err(|err| anyhow!("attribute: {err}"))?;
				let key = attr.key.into_inner().to_string();
				let value = attr
					.normalized_value(quick_xml::XmlVersion::Implicit1_0)?
					.into_owned();
				if key == "d" {
					d = Some(value.clone());
				}
				let named = NAMES.contains(&key.as_str()).then_some(value);
				shape.push((key, named));
			}

			doc.spans.push((start, end));
			doc.names.push(e.local_name().into_inner().to_string());
			doc.shapes.push(shape);
			doc.depths.push(depth);
			doc.paths.push(d);
			if opens {
				depth += 1;
			}
		}
		Ok(doc)
	}

	fn tag(&self, element: usize) -> &'a str {
		let (start, end) = self.spans[element];
		&self.src[start..end]
	}

	fn len(&self) -> usize {
		self.spans.len()
	}
}

/// Whether two candidates hold the same document, so that one element can move between them.
///
/// The names and the nesting must match, and so must every attribute name. An attribute that names
/// another element must also hold the same name, since a mix moves attributes and not references.
fn aligned(a: &Doc, b: &Doc) -> bool {
	a.len() == b.len() && a.names == b.names && a.depths == b.depths && a.shapes == b.shapes
}

/// Mix `candidates`, which are one document at rising precision, `candidates[0]` being the lowest.
///
/// `measure` renders a document and returns the difference against the source, or `None` when it
/// does not render. `Ok(None)` means no mix inside the budget was found, and the caller keeps what
/// the search chose.
pub fn mix(
	candidates: &[String],
	threshold: f64,
	measure: &mut dyn FnMut(&str) -> Result<Option<f64>>,
) -> Result<Option<Mixed>> {
	if candidates.len() < 2 {
		return Ok(None);
	}
	let docs: Vec<Doc> = candidates
		.iter()
		.map(|c| Doc::parse(c))
		.collect::<Result<_>>()?;
	if docs.iter().any(|d| !aligned(&docs[0], d)) {
		return Ok(None);
	}

	let weights = weights(&docs[0]);
	let top = docs.len() - 1;
	let mut choice = vec![0_usize; docs[0].len()];
	// Whether the elements with no edge have already gone up together.
	let mut bulk = false;

	// An upgrade that costs nothing is taken before the first render.
	for (element, level) in choice.iter_mut().enumerate() {
		while *level < top && cost(&docs, element, *level) <= 0 {
			*level += 1;
		}
	}

	for _ in 0..MAX_STEPS {
		let svg = splice(&docs, &choice);
		if let Some(difference) = measure(&svg)?
			&& difference <= threshold
		{
			// A level that writes the same tag is no upgrade, whatever its number says.
			let upgrades = choice
				.iter()
				.enumerate()
				.filter(|&(element, &level)| docs[level].tag(element) != docs[0].tag(element))
				.count();
			return Ok(Some(Mixed {
				svg,
				difference,
				upgrades,
			}));
		}

		// The next try is the element that buys the most accuracy for the fewest bytes.
		let next = (0..choice.len())
			.filter(|&e| choice[e] < top)
			.filter(|&e| weights[e] > 0.0)
			.max_by(|&a, &b| {
				let rate = |e: usize| {
					#[allow(clippy::cast_precision_loss)]
					let bytes = cost(&docs, e, choice[e]).max(1) as f64;
					weights[e] / bytes
				};
				rate(a).total_cmp(&rate(b))
			});

		match next {
			Some(element) => choice[element] += 1,
			// An element with no edge of its own holds a gradient or a stop, which costs a handful
			// of bytes. They all go up together, since a small gap can close on them alone.
			None if !bulk => {
				bulk = true;
				let mut moved = false;
				for level in &mut choice {
					if *level < top {
						*level += 1;
						moved = true;
					}
				}
				if !moved {
					return Ok(None);
				}
			}
			None => return Ok(None),
		}
	}
	Ok(None)
}

/// The bytes an upgrade of `element` from `level` to the next one costs.
fn cost(docs: &[Doc], element: usize, level: usize) -> isize {
	let now = docs[level].tag(element).len();
	let next = docs[level + 1].tag(element).len();
	next.cast_signed() - now.cast_signed()
}

/// The document with every element written at the level `choice` names.
fn splice(docs: &[Doc], choice: &[usize]) -> String {
	let base = &docs[0];
	let mut out = String::with_capacity(base.src.len());
	let mut last = 0;

	for (element, &level) in choice.iter().enumerate() {
		let (start, end) = base.spans[element];
		out.push_str(&base.src[last..start]);
		out.push_str(docs[level].tag(element));
		last = end;
	}
	out.push_str(&base.src[last..]);
	out
}

/// What every element stands to gain from another decimal.
///
/// A rounded coordinate moves an edge sideways, so what the eye sees is the length of the edge that
/// moves. A container has no edge of its own: it holds the edges of its children, since its own
/// rounded `transform` moves all of them.
fn weights(doc: &Doc) -> Vec<f64> {
	let mut own: Vec<f64> = doc
		.paths
		.iter()
		.map(|d| d.as_deref().map_or(0.0, edge_length))
		.collect();

	// Deepest first, so a child is counted before the container that holds it.
	let mut order: Vec<usize> = (0..doc.len()).collect();
	order.sort_by_key(|&e| std::cmp::Reverse(doc.depths[e]));
	for element in order {
		let mine = own[element];
		if let Some(parent) = parent_of(doc, element) {
			own[parent] += mine;
		}
	}
	own
}

/// The element that holds `element`, which is the closest one before it that is less deep.
fn parent_of(doc: &Doc, element: usize) -> Option<usize> {
	let depth = doc.depths[element];
	(0..element).rev().find(|&e| doc.depths[e] < depth)
}

/// The length of the outline of a path.
///
/// A curve counts as the line through its control points, and an arc as its chord. Both are
/// estimates, and an estimate is all that the order of the tries needs.
fn edge_length(d: &str) -> f64 {
	let Ok(path) = Path::parse_string(d) else {
		return 0.0;
	};
	let mut length = 0.0;
	// The current point, and the point a `Z` returns to.
	let (mut x, mut y) = (0.0, 0.0);
	let (mut sx, mut sy) = (0.0, 0.0);

	for data in &path.0 {
		let mut data = data;
		while let Data::Implicit(inner) = data {
			data = inner;
		}
		// A relative command counts from the current point, an absolute one from the origin.
		let (ox, oy) = match data {
			Data::MoveBy(_)
			| Data::LineBy(_)
			| Data::HorizontalLineBy(_)
			| Data::VerticalLineBy(_)
			| Data::CubicBezierBy(_)
			| Data::SmoothBezierBy(_)
			| Data::QuadraticBezierBy(_)
			| Data::SmoothQuadraticBezierBy(_)
			| Data::ArcBy(_) => (x, y),
			_ => (0.0, 0.0),
		};
		// The points the command passes through, its control points included.
		let points: Vec<(f64, f64)> = match data {
			Data::MoveTo(a) | Data::MoveBy(a) => {
				(x, y) = (ox + a[0], oy + a[1]);
				(sx, sy) = (x, y);
				continue;
			}
			Data::ClosePath => vec![(sx, sy)],
			Data::LineTo(a)
			| Data::LineBy(a)
			| Data::SmoothQuadraticBezierTo(a)
			| Data::SmoothQuadraticBezierBy(a) => vec![(ox + a[0], oy + a[1])],
			Data::HorizontalLineTo(a) | Data::HorizontalLineBy(a) => vec![(ox + a[0], y)],
			Data::VerticalLineTo(a) | Data::VerticalLineBy(a) => vec![(x, oy + a[0])],
			Data::CubicBezierTo(a) | Data::CubicBezierBy(a) => vec![
				(ox + a[0], oy + a[1]),
				(ox + a[2], oy + a[3]),
				(ox + a[4], oy + a[5]),
			],
			Data::SmoothBezierTo(a)
			| Data::SmoothBezierBy(a)
			| Data::QuadraticBezierTo(a)
			| Data::QuadraticBezierBy(a) => {
				vec![(ox + a[0], oy + a[1]), (ox + a[2], oy + a[3])]
			}
			Data::ArcTo(a) | Data::ArcBy(a) => vec![(ox + a[5], oy + a[6])],
			Data::Implicit(_) => continue,
		};

		for (px, py) in points {
			length += (px - x).hypot(py - y);
			(x, y) = (px, py);
		}
	}
	length
}

#[cfg(test)]
mod tests {
	use super::*;

	const LOW: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">
	<path d="M0 0h100v100H0Z"/>
	<path d="M10 10h20v20H10Z"/>
</svg>"#;

	const HIGH: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">
	<path d="M0.25 0.25h100.5v100.5H0.25Z"/>
	<path d="M10.25 10.25h20.5v20.5H10.25Z"/>
</svg>"#;

	/// Nothing is measured until the mix is asked for, and the first try is the lowest of all.
	#[test]
	fn the_first_try_is_every_element_at_the_lowest_precision() {
		let mut seen = Vec::new();
		let out = mix(
			&[LOW.to_string(), HIGH.to_string()],
			1.0,
			&mut |svg: &str| {
				seen.push(svg.to_string());
				Ok(Some(0.5))
			},
		)
		.unwrap()
		.unwrap();
		assert_eq!(seen.len(), 1);
		assert_eq!(out.svg, LOW);
		assert_eq!(out.upgrades, 0);
		assert!((out.difference - 0.5).abs() < f64::EPSILON);
	}

	/// The longer path is tried first, since it moves more edge for its bytes.
	#[test]
	fn the_longest_edge_is_upgraded_first() {
		let mut seen = Vec::new();
		let out = mix(
			&[LOW.to_string(), HIGH.to_string()],
			0.1,
			&mut |svg: &str| {
				seen.push(svg.to_string());
				// Only the document that holds the long path at the high precision is good enough.
				Ok(Some(if svg.contains("M0.25 0.25") {
					0.05
				} else {
					0.5
				}))
			},
		)
		.unwrap()
		.unwrap();
		assert_eq!(seen.len(), 2, "{seen:#?}");
		assert_eq!(out.upgrades, 1);
		// The small path keeps the low precision, so the mix is shorter than the high candidate.
		assert!(out.svg.contains(r#"d="M10 10h20v20H10Z""#), "{}", out.svg);
		assert!(out.svg.len() < HIGH.len());
	}

	/// A budget that no mix reaches gives up, and the caller keeps the choice of the search.
	#[test]
	fn an_unreachable_budget_gives_up() {
		let out = mix(&[LOW.to_string(), HIGH.to_string()], -1.0, &mut |_| {
			Ok(Some(0.5))
		})
		.unwrap();
		assert!(out.is_none());
	}

	/// A candidate that holds another document is no candidate for a mix.
	#[test]
	fn a_document_that_does_not_align_is_refused() {
		let other = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">
	<path d="M0 0h100v100H0Z"/>
</svg>"#;
		let out = mix(&[LOW.to_string(), other.to_string()], 1.0, &mut |_| {
			Ok(Some(0.0))
		})
		.unwrap();
		assert!(out.is_none());
	}

	/// A reference must stay with the element that holds it.
	#[test]
	fn a_moved_reference_is_refused() {
		let low = r#"<svg xmlns="http://www.w3.org/2000/svg"><path id="a" d="M0 0h1"/></svg>"#;
		let high = r#"<svg xmlns="http://www.w3.org/2000/svg"><path id="b" d="M0 0h1.5"/></svg>"#;
		let out = mix(&[low.to_string(), high.to_string()], 1.0, &mut |_| {
			Ok(Some(0.0))
		})
		.unwrap();
		assert!(out.is_none());
	}

	#[test]
	fn a_straight_edge_has_the_length_of_its_sides() {
		// Three sides of 100, and the `Z` closes the fourth.
		let length = edge_length("M0 0h100v100H0Z");
		assert!((length - 400.0).abs() < 0.001, "{length}");
	}

	#[test]
	fn a_relative_curve_counts_its_control_points() {
		// One `c` from the origin: (0,0) to (10,0) to (10,10) to (20,10).
		let length = edge_length("M0 0c10 0 10 10 20 10");
		assert!((length - 30.0).abs() < 0.001, "{length}");
	}
}
