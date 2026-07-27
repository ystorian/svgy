//! `round`: scale + center an SVG's content so its actual drawn shape fits inside
//! the circle inscribed in the viewBox, keeping the viewBox unchanged.
//!
//! The shape's *minimum enclosing circle* is measured by rasterizing and running
//! Welzl's algorithm over the opaque-pixel hull. The uniform scale + translation
//! that maps that circle onto the inscribed circle is then baked into every
//! coordinate (no transforms added; element structure preserved).

use anyhow::{Context, Result, anyhow, bail};
use quick_xml::events::{BytesStart, Event};
use quick_xml::reader::Reader;
use quick_xml::writer::Writer;
use resvg::usvg;

use crate::cli::DEFAULT_ROUND_PADDING;
use crate::numeric::{
    affine_length, fmt_num, parse_num_list, parse_px, scale_len_list, scale_length,
    split_num_unit,
};
use crate::render;

/// Pixmap max dimension used for shape measurement.
const MEASURE_PX: f64 = 1024.0;

/// A rounded document, plus what it took to get there.
pub struct Round {
    /// The rewritten SVG.
    pub svg: String,
    /// The padding fraction actually applied.
    pub padding: f64,
    /// True when `padding` was svgy's choice rather than the caller's, which
    /// happens when the source has a full-canvas background.
    pub auto_padding: bool,
    /// Uniform scale applied to the artwork.
    pub scale: f64,
    /// Radius of the circle the artwork was fitted into.
    pub radius: f64,
}

/// Fit `src`'s shape into the circle inscribed in its viewBox, leaving a
/// `padding` fraction of the radius empty. `None` picks the default: an inset
/// when the source has a full-canvas background, flush otherwise.
pub fn round_str(src: &str, padding: Option<f64>) -> Result<Round> {
    let (min_x, min_y, w, h) = parse_root_box(src)?;

    // Measure only the foreground: strip full-canvas background fills first.
    let (foreground, has_background) = strip_background(src, min_x, min_y, w, h)?;
    let tree = render::load_tree_from_data(foreground.as_bytes())?;
    let (cx, cy, r) = shape_enclosing_circle(&tree, min_x, min_y, w, h)?;
    if r <= 1e-9 {
        bail!("content has no measurable extent");
    }

    // Artwork sitting on a background reads better inset from the rim, so that
    // is the default there; artwork alone fills the circle.
    let auto_padding = padding.is_none();
    let padding = padding.unwrap_or(if has_background {
        DEFAULT_ROUND_PADDING
    } else {
        0.0
    });
    if !(0.0..1.0).contains(&padding) {
        bail!("--padding must be in 0.0..1.0");
    }

    let radius = (w.min(h) / 2.0) * (1.0 - padding);
    let (ox, oy) = (min_x + w / 2.0, min_y + h / 2.0);
    let s = radius / r;
    let tx = ox - s * cx;
    let ty = oy - s * cy;

    Ok(Round {
        svg: bake_affine(src, s, tx, ty, (min_x, min_y, w, h))?,
        padding,
        auto_padding,
        scale: s,
        radius,
    })
}

// --- shape measurement -----------------------------------------------------

/// Minimum enclosing circle of the drawn shape, in userspace coordinates.
// The measurement raster is a few thousand pixels a side: exact in both f32 and
// f64, so narrowing the scale and widening the pixel indices are both faithful.
#[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
fn shape_enclosing_circle(
    tree: &usvg::Tree,
    min_x: f64,
    min_y: f64,
    w: f64,
    h: f64,
) -> Result<(f64, f64, f64)> {
    let size = tree.size();
    let (sw, sh) = (f64::from(size.width()), f64::from(size.height()));
    let k = MEASURE_PX / sw.max(sh);
    let pixmap = render::render_scale(tree, k as f32)?;
    let (pw, ph) = (pixmap.width() as usize, pixmap.height() as usize);
    let pixels = pixmap.pixels();

    // Per-row min/max opaque x captures every convex-hull vertex of the shape.
    let mut pts: Vec<(f64, f64)> = Vec::new();
    for y in 0..ph {
        let row = y * pw;
        let mut lo: Option<usize> = None;
        let mut hi = 0usize;
        for x in 0..pw {
            if pixels[row + x].alpha() > 0 {
                if lo.is_none() {
                    lo = Some(x);
                }
                hi = x;
            }
        }
        if let Some(l) = lo {
            for &dx in &[l, hi] {
                // device -> tree (/k) -> userspace (viewBox mapping).
                let ux = min_x + ((dx as f64 + 0.5) / k) * (w / sw);
                let uy = min_y + ((y as f64 + 0.5) / k) * (h / sh);
                pts.push((ux, uy));
            }
        }
    }

    if pts.is_empty() {
        bail!("no foreground content to fit (only a full-canvas background?)");
    }
    let hull = convex_hull(pts);
    let c = min_enclosing_circle(&hull);
    Ok((c.cx, c.cy, c.r))
}

#[derive(Clone, Copy)]
struct Circle {
    cx: f64,
    cy: f64,
    r: f64,
}

fn convex_hull(mut p: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
    p.sort_by(|a, b| {
        a.0.partial_cmp(&b.0)
            .unwrap()
            .then(a.1.partial_cmp(&b.1).unwrap())
    });
    p.dedup();
    let n = p.len();
    if n <= 2 {
        return p;
    }
    let cross = |o: (f64, f64), a: (f64, f64), b: (f64, f64)| {
        (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0)
    };

    let mut lower: Vec<(f64, f64)> = Vec::new();
    for &pt in &p {
        while lower.len() >= 2
            && cross(lower[lower.len() - 2], lower[lower.len() - 1], pt) <= 0.0
        {
            lower.pop();
        }
        lower.push(pt);
    }
    let mut upper: Vec<(f64, f64)> = Vec::new();
    for &pt in p.iter().rev() {
        while upper.len() >= 2
            && cross(upper[upper.len() - 2], upper[upper.len() - 1], pt) <= 0.0
        {
            upper.pop();
        }
        upper.push(pt);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

/// Welzl's minimum enclosing circle over a (small) hull point set.
fn min_enclosing_circle(pts: &[(f64, f64)]) -> Circle {
    welzl(pts, &mut Vec::new())
}

fn welzl(p: &[(f64, f64)], boundary: &mut Vec<(f64, f64)>) -> Circle {
    if p.is_empty() || boundary.len() == 3 {
        return trivial(boundary);
    }
    let idx = p.len() - 1;
    let point = p[idx];
    let d = welzl(&p[..idx], boundary);
    if in_circle(&d, point) {
        return d;
    }
    boundary.push(point);
    let d = welzl(&p[..idx], boundary);
    boundary.pop();
    d
}

fn in_circle(c: &Circle, p: (f64, f64)) -> bool {
    let dx = p.0 - c.cx;
    let dy = p.1 - c.cy;
    (dx * dx + dy * dy).sqrt() <= c.r + 1e-7
}

fn trivial(b: &[(f64, f64)]) -> Circle {
    match b.len() {
        0 => Circle { cx: 0.0, cy: 0.0, r: 0.0 },
        1 => Circle { cx: b[0].0, cy: b[0].1, r: 0.0 },
        2 => circle_from_2(b[0], b[1]),
        _ => {
            if let Some(c) = circumcircle(b[0], b[1], b[2])
                && encloses(&c, b) {
                    return c;
                }
            // Degenerate/near-collinear: smallest pair-diameter circle covering all.
            [(0, 1), (0, 2), (1, 2)]
                .iter()
                .map(|&(i, j)| circle_from_2(b[i], b[j]))
                .filter(|c| encloses(c, b))
                .min_by(|a, c| a.r.partial_cmp(&c.r).unwrap())
                .unwrap_or(Circle { cx: 0.0, cy: 0.0, r: f64::MAX })
        }
    }
}

fn encloses(c: &Circle, pts: &[(f64, f64)]) -> bool {
    pts.iter().all(|&p| in_circle(c, p))
}

fn circle_from_2(a: (f64, f64), b: (f64, f64)) -> Circle {
    let cx = f64::midpoint(a.0, b.0);
    let cy = f64::midpoint(a.1, b.1);
    let r = ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt() / 2.0;
    Circle { cx, cy, r }
}

fn circumcircle(a: (f64, f64), b: (f64, f64), c: (f64, f64)) -> Option<Circle> {
    let d = 2.0 * (a.0 * (b.1 - c.1) + b.0 * (c.1 - a.1) + c.0 * (a.1 - b.1));
    if d.abs() < 1e-12 {
        return None;
    }
    let (ax2, ay2) = (a.0 * a.0 + a.1 * a.1, 0.0);
    let _ = ay2;
    let bx2 = b.0 * b.0 + b.1 * b.1;
    let cx2 = c.0 * c.0 + c.1 * c.1;
    let ux = (ax2 * (b.1 - c.1) + bx2 * (c.1 - a.1) + cx2 * (a.1 - b.1)) / d;
    let uy = (ax2 * (c.0 - b.0) + bx2 * (a.0 - c.0) + cx2 * (b.0 - a.0)) / d;
    let r = ((a.0 - ux).powi(2) + (a.1 - uy).powi(2)).sqrt();
    Some(Circle { cx: ux, cy: uy, r })
}

// --- affine coordinate baking ----------------------------------------------

fn bake_affine(
    src: &str,
    s: f64,
    tx: f64,
    ty: f64,
    bx: (f64, f64, f64, f64),
) -> Result<String> {
    let mut reader = Reader::from_str(src);
    let mut writer = Writer::new(Vec::new());
    let mut seen_root = false;
    // Depth counter while inside a background subtree, which is copied verbatim.
    let mut asis = 0usize;

    loop {
        match reader.read_event().context("parsing SVG")? {
            Event::Eof => break,
            Event::Start(e) => {
                if asis > 0 {
                    asis += 1;
                    writer.write_event(Event::Start(e))?;
                } else if !seen_root {
                    seen_root = true;
                    writer.write_event(Event::Start(e))?; // canvas/viewBox unchanged
                } else if is_background(&e, bx)? {
                    asis = 1;
                    writer.write_event(Event::Start(e))?;
                } else {
                    writer.write_event(Event::Start(process_element(&e, s, tx, ty)?))?;
                }
            }
            Event::End(e) => {
                asis = asis.saturating_sub(1);
                writer.write_event(Event::End(e))?;
            }
            Event::Empty(e) => {
                if asis > 0 || !seen_root {
                    seen_root = true;
                    writer.write_event(Event::Empty(e))?;
                } else if is_background(&e, bx)? {
                    writer.write_event(Event::Empty(e))?; // kept as-is
                } else {
                    writer.write_event(Event::Empty(process_element(&e, s, tx, ty)?))?;
                }
            }
            other => writer.write_event(other)?,
        }
    }
    Ok(String::from_utf8(writer.into_inner())?)
}

/// Rewrite the SVG with full-canvas background fills removed, for measurement.
/// The flag reports whether anything was actually removed.
fn strip_background(
    src: &str,
    min_x: f64,
    min_y: f64,
    w: f64,
    h: f64,
) -> Result<(String, bool)> {
    let bx = (min_x, min_y, w, h);
    let mut reader = Reader::from_str(src);
    let mut writer = Writer::new(Vec::new());
    let mut seen_root = false;
    let mut skip = 0usize;
    let mut stripped = false;

    loop {
        match reader.read_event().context("parsing SVG")? {
            Event::Eof => break,
            Event::Start(e) => {
                if skip > 0 {
                    skip += 1;
                } else if !seen_root {
                    seen_root = true;
                    writer.write_event(Event::Start(e))?;
                } else if is_background(&e, bx)? {
                    skip = 1; // drop this element and its subtree
                    stripped = true;
                } else {
                    writer.write_event(Event::Start(e))?;
                }
            }
            Event::End(e) => {
                if skip > 0 {
                    skip -= 1;
                } else {
                    writer.write_event(Event::End(e))?;
                }
            }
            Event::Empty(e) => {
                if skip > 0 {
                    // nested empty inside a skipped subtree: drop
                } else if !seen_root {
                    seen_root = true;
                    writer.write_event(Event::Empty(e))?;
                } else if is_background(&e, bx)? {
                    stripped = true; // dropped
                } else {
                    writer.write_event(Event::Empty(e))?;
                }
            }
            other => {
                if skip == 0 {
                    writer.write_event(other)?;
                }
            }
        }
    }
    Ok((String::from_utf8(writer.into_inner())?, stripped))
}

/// A drawable, visibly-filled shape whose bounding box covers the whole viewBox.
fn is_background(e: &BytesStart, bx: (f64, f64, f64, f64)) -> Result<bool> {
    let (min_x, min_y, w, h) = bx;
    let local = e.local_name();
    let local = std::str::from_utf8(local.as_ref())?.to_string();

    let Some((bminx, bminy, bmaxx, bmaxy)) = element_bbox(&local, e)? else {
        return Ok(false);
    };
    let filled = attr_value(e, "fill")?.is_none_or(|f| f.trim() != "none");
    if !filled {
        return Ok(false);
    }

    let tol_x = w.abs() * 0.005 + 1e-6;
    let tol_y = h.abs() * 0.005 + 1e-6;
    let covers = bminx <= min_x + tol_x
        && bminy <= min_y + tol_y
        && bmaxx >= min_x + w - tol_x
        && bmaxy >= min_y + h - tol_y;
    Ok(covers)
}

/// Local (transform-ignoring) bounding box of a shape from its own attributes.
fn element_bbox(local: &str, e: &BytesStart) -> Result<Option<(f64, f64, f64, f64)>> {
    let num = |key: &str| -> Result<Option<f64>> {
        Ok(attr_value(e, key)?
            .and_then(|v| split_num_unit(&v).map(|(n, _)| n)))
    };
    let bbox = match local {
        "rect" => {
            let x = num("x")?.unwrap_or(0.0);
            let y = num("y")?.unwrap_or(0.0);
            match (num("width")?, num("height")?) {
                (Some(w), Some(h)) => Some((x, y, x + w, y + h)),
                _ => None,
            }
        }
        "circle" => {
            let cx = num("cx")?.unwrap_or(0.0);
            let cy = num("cy")?.unwrap_or(0.0);
            num("r")?.map(|r| (cx - r, cy - r, cx + r, cy + r))
        }
        "ellipse" => {
            let cx = num("cx")?.unwrap_or(0.0);
            let cy = num("cy")?.unwrap_or(0.0);
            match (num("rx")?, num("ry")?) {
                (Some(rx), Some(ry)) => Some((cx - rx, cy - ry, cx + rx, cy + ry)),
                _ => None,
            }
        }
        "polygon" | "polyline" => {
            attr_value(e, "points")?.and_then(|p| points_bbox(&p))
        }
        "path" => attr_value(e, "d")?.and_then(|d| path_bbox(&d)),
        _ => None,
    };
    Ok(bbox)
}

fn points_bbox(v: &str) -> Option<(f64, f64, f64, f64)> {
    let nums = parse_num_list(v);
    let mut bb: Option<(f64, f64, f64, f64)> = None;
    for c in nums.chunks(2) {
        if c.len() == 2 {
            extend(&mut bb, c[0], c[1]);
        }
    }
    bb
}

/// Conservative path bounding box (curve control points bound the curve).
fn path_bbox(d: &str) -> Option<(f64, f64, f64, f64)> {
    use svgtypes::{PathParser, PathSegment};

    let mut bb: Option<(f64, f64, f64, f64)> = None;
    let mut cur = (0.0f64, 0.0f64);
    let mut start = (0.0f64, 0.0f64);
    let abs = |c: (f64, f64), x: f64, y: f64, is_abs: bool| {
        if is_abs { (x, y) } else { (c.0 + x, c.1 + y) }
    };

    for seg in PathParser::from(d) {
        let seg = seg.ok()?;
        match seg {
            PathSegment::MoveTo { abs: a, x, y } => {
                cur = abs(cur, x, y, a);
                start = cur;
                extend(&mut bb, cur.0, cur.1);
            }
            // Segments for which only the endpoint is tracked. A smooth
            // quadratic's reflected control point and an arc's bulge are not
            // accounted for, exactly as before these arms were merged.
            PathSegment::LineTo { abs: a, x, y }
            | PathSegment::SmoothQuadratic { abs: a, x, y }
            | PathSegment::EllipticalArc { abs: a, x, y, .. } => {
                cur = abs(cur, x, y, a);
                extend(&mut bb, cur.0, cur.1);
            }
            PathSegment::HorizontalLineTo { abs: a, x } => {
                cur.0 = if a { x } else { cur.0 + x };
                extend(&mut bb, cur.0, cur.1);
            }
            PathSegment::VerticalLineTo { abs: a, y } => {
                cur.1 = if a { y } else { cur.1 + y };
                extend(&mut bb, cur.0, cur.1);
            }
            PathSegment::CurveTo { abs: a, x1, y1, x2, y2, x, y } => {
                let p1 = abs(cur, x1, y1, a);
                let p2 = abs(cur, x2, y2, a);
                let p = abs(cur, x, y, a);
                extend(&mut bb, p1.0, p1.1);
                extend(&mut bb, p2.0, p2.1);
                extend(&mut bb, p.0, p.1);
                cur = p;
            }
            PathSegment::SmoothCurveTo { abs: a, x2, y2, x, y } => {
                let p2 = abs(cur, x2, y2, a);
                let p = abs(cur, x, y, a);
                extend(&mut bb, p2.0, p2.1);
                extend(&mut bb, p.0, p.1);
                cur = p;
            }
            PathSegment::Quadratic { abs: a, x1, y1, x, y } => {
                let p1 = abs(cur, x1, y1, a);
                let p = abs(cur, x, y, a);
                extend(&mut bb, p1.0, p1.1);
                extend(&mut bb, p.0, p.1);
                cur = p;
            }
            PathSegment::ClosePath { .. } => cur = start,
        }
    }
    bb
}

fn extend(bb: &mut Option<(f64, f64, f64, f64)>, x: f64, y: f64) {
    match bb {
        None => *bb = Some((x, y, x, y)),
        Some((minx, miny, maxx, maxy)) => {
            *minx = minx.min(x);
            *miny = miny.min(y);
            *maxx = maxx.max(x);
            *maxy = maxy.max(y);
        }
    }
}

fn process_element(
    e: &BytesStart,
    s: f64,
    tx: f64,
    ty: f64,
) -> Result<BytesStart<'static>> {
    let local = e.local_name();
    let local = std::str::from_utf8(local.as_ref())?.to_string();

    // Gradient/pattern coords are only user-space (scalable) under userSpaceOnUse.
    let is_grad = matches!(local.as_str(), "linearGradient" | "radialGradient" | "pattern");
    let do_scale = if is_grad {
        let key = if local == "pattern" { "patternUnits" } else { "gradientUnits" };
        attr_value(e, key)?.as_deref() == Some("userSpaceOnUse")
    } else {
        true
    };

    let name = std::str::from_utf8(e.name().as_ref())?.to_string();
    let mut out = BytesStart::new(name);
    for attr in e.attributes() {
        let attr = attr.map_err(|err| anyhow!("attribute: {err}"))?;
        let key = std::str::from_utf8(attr.key.as_ref())?.to_string();
        let val = attr
            .normalized_value(quick_xml::XmlVersion::Implicit1_0)?
            .into_owned();
        let new_val = if do_scale {
            affine_attr(&key, &val, s, tx, ty)
        } else {
            val
        };
        out.push_attribute((key.as_str(), new_val.as_str()));
    }
    Ok(out)
}

/// Apply the affine to one attribute. Position components get `s*n + t`; lengths
/// and relative offsets get `s*n`.
fn affine_attr(key: &str, val: &str, s: f64, tx: f64, ty: f64) -> String {
    match key {
        "transform" | "gradientTransform" | "patternTransform" => {
            affine_transform(val, s, tx, ty)
        }
        "d" => affine_path_data(val, s, tx, ty),
        "points" => affine_points(val, s, tx, ty),
        "stroke-dasharray" | "dx" | "dy" => scale_len_list(val, s),
        "x" | "cx" | "x1" | "x2" | "fx" => affine_length(val, s, tx),
        "y" | "cy" | "y1" | "y2" | "fy" => affine_length(val, s, ty),
        "width" | "height" | "r" | "rx" | "ry" | "stroke-width" | "stroke-dashoffset"
        | "font-size" => scale_length(val, s),
        _ => val.to_string(),
    }
}

fn affine_points(v: &str, s: f64, tx: f64, ty: f64) -> String {
    parse_num_list(v)
        .chunks(2)
        .map(|c| {
            if c.len() == 2 {
                format!("{} {}", fmt_num(s * c[0] + tx), fmt_num(s * c[1] + ty))
            } else {
                fmt_num(s * c[0] + tx)
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Conjugate a `transform` list by the affine `A(p) = s*p + t`: each primitive
/// becomes `A T A^-1`. translate/rotate keep their kind; scale/skew/matrix become
/// an equivalent `matrix(...)` (their fixed point moves under translation).
fn affine_transform(v: &str, s: f64, tx: f64, ty: f64) -> String {
    let mut out = String::new();
    let mut rest = v;
    loop {
        let trimmed = rest.trim_start_matches([' ', ',', '\t', '\n', '\r']);
        if trimmed.is_empty() {
            break;
        }
        let Some(open) = trimmed.find('(') else {
            out.push_str(trimmed);
            break;
        };
        let Some(close) = trimmed[open + 1..].find(')') else {
            out.push_str(trimmed);
            break;
        };
        let name = trimmed[..open].trim();
        let args = parse_num_list(&trimmed[open + 1..open + 1 + close]);
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&conjugate(name, &args, s, tx, ty));
        rest = &trimmed[open + 1 + close + 1..];
    }
    out
}

fn conjugate(name: &str, args: &[f64], s: f64, tx: f64, ty: f64) -> String {
    let g = |i: usize, d: f64| args.get(i).copied().unwrap_or(d);
    match name {
        "translate" => {
            let (dx, dy) = (g(0, 0.0), g(1, 0.0));
            format!("translate({} {})", fmt_num(s * dx), fmt_num(s * dy))
        }
        "rotate" => {
            // center is a position -> full affine (default origin -> t).
            let (a, cx, cy) = (g(0, 0.0), g(1, 0.0), g(2, 0.0));
            format!(
                "rotate({} {} {})",
                fmt_num(a),
                fmt_num(s * cx + tx),
                fmt_num(s * cy + ty)
            )
        }
        "scale" => {
            let a = g(0, 1.0);
            let b = g(1, a);
            matrix_str(a, 0.0, 0.0, b, (1.0 - a) * tx, (1.0 - b) * ty)
        }
        "skewX" => {
            let c = (g(0, 0.0).to_radians()).tan();
            matrix_str(1.0, 0.0, c, 1.0, -c * ty, 0.0)
        }
        "skewY" => {
            let b = (g(0, 0.0).to_radians()).tan();
            matrix_str(1.0, b, 0.0, 1.0, 0.0, -b * tx)
        }
        "matrix" => {
            let (a, b, c, d, e, f) = (g(0, 1.0), g(1, 0.0), g(2, 0.0), g(3, 1.0), g(4, 0.0), g(5, 0.0));
            let e2 = s * e + (1.0 - a) * tx - c * ty;
            let f2 = s * f - b * tx + (1.0 - d) * ty;
            matrix_str(a, b, c, d, e2, f2)
        }
        _ => {
            let joined = args.iter().map(|a| fmt_num(*a)).collect::<Vec<_>>().join(" ");
            format!("{name}({joined})")
        }
    }
}

fn matrix_str(a: f64, b: f64, c: f64, d: f64, e: f64, f: f64) -> String {
    format!(
        "matrix({} {} {} {} {} {})",
        fmt_num(a),
        fmt_num(b),
        fmt_num(c),
        fmt_num(d),
        fmt_num(e),
        fmt_num(f)
    )
}

/// Apply the affine to path `d`: absolute coords get `s*p + t`, relative deltas
/// get `s*p`; arc rx/ry scale, rotation and flags are preserved. On parse error
/// the original string is returned.
fn affine_path_data(d: &str, s: f64, tx: f64, ty: f64) -> String {
    use svgtypes::{PathParser, PathSegment};

    // Absolute point -> affine; relative point -> scale-only.
    let pt = |x: f64, y: f64, abs: bool| {
        if abs {
            (s * x + tx, s * y + ty)
        } else {
            (s * x, s * y)
        }
    };

    let mut out = String::new();
    for seg in PathParser::from(d) {
        let Ok(seg) = seg else {
            return d.to_string();
        };
        match seg {
            PathSegment::MoveTo { abs, x, y } => {
                let (x, y) = pt(x, y, abs);
                cmd(&mut out, 'M', abs);
                out.push_str(&two(x, y));
            }
            PathSegment::LineTo { abs, x, y } => {
                let (x, y) = pt(x, y, abs);
                cmd(&mut out, 'L', abs);
                out.push_str(&two(x, y));
            }
            PathSegment::HorizontalLineTo { abs, x } => {
                let x = if abs { s * x + tx } else { s * x };
                cmd(&mut out, 'H', abs);
                out.push_str(&fmt_num(x));
            }
            PathSegment::VerticalLineTo { abs, y } => {
                let y = if abs { s * y + ty } else { s * y };
                cmd(&mut out, 'V', abs);
                out.push_str(&fmt_num(y));
            }
            PathSegment::CurveTo { abs, x1, y1, x2, y2, x, y } => {
                let (x1, y1) = pt(x1, y1, abs);
                let (x2, y2) = pt(x2, y2, abs);
                let (x, y) = pt(x, y, abs);
                cmd(&mut out, 'C', abs);
                out.push_str(&nums(&[x1, y1, x2, y2, x, y]));
            }
            PathSegment::SmoothCurveTo { abs, x2, y2, x, y } => {
                let (x2, y2) = pt(x2, y2, abs);
                let (x, y) = pt(x, y, abs);
                cmd(&mut out, 'S', abs);
                out.push_str(&nums(&[x2, y2, x, y]));
            }
            PathSegment::Quadratic { abs, x1, y1, x, y } => {
                let (x1, y1) = pt(x1, y1, abs);
                let (x, y) = pt(x, y, abs);
                cmd(&mut out, 'Q', abs);
                out.push_str(&nums(&[x1, y1, x, y]));
            }
            PathSegment::SmoothQuadratic { abs, x, y } => {
                let (x, y) = pt(x, y, abs);
                cmd(&mut out, 'T', abs);
                out.push_str(&two(x, y));
            }
            PathSegment::EllipticalArc {
                abs, rx, ry, x_axis_rotation, large_arc, sweep, x, y,
            } => {
                let (x, y) = pt(x, y, abs);
                cmd(&mut out, 'A', abs);
                // The two flags are single digits, not numbers to format.
                out.push_str(&nums(&[rx * s, ry * s, x_axis_rotation]));
                out.push(' ');
                out.push(if large_arc { '1' } else { '0' });
                out.push(' ');
                out.push(if sweep { '1' } else { '0' });
                out.push(' ');
                out.push_str(&two(x, y));
            }
            PathSegment::ClosePath { abs } => cmd(&mut out, 'Z', abs),
        }
        out.push(' ');
    }
    out.trim_end().to_string()
}

fn cmd(out: &mut String, upper: char, abs: bool) {
    out.push(if abs { upper } else { upper.to_ascii_lowercase() });
    out.push(' ');
}

fn two(a: f64, b: f64) -> String {
    nums(&[a, b])
}

/// Space-separated `fmt_num` of each value.
fn nums(vs: &[f64]) -> String {
    vs.iter().map(|v| fmt_num(*v)).collect::<Vec<_>>().join(" ")
}

// --- root parsing ----------------------------------------------------------

/// Read the root `<svg>` coordinate box: `(min_x, min_y, w, h)` from `viewBox`,
/// else from `width`/`height`.
fn parse_root_box(src: &str) -> Result<(f64, f64, f64, f64)> {
    let mut reader = Reader::from_str(src);
    loop {
        match reader.read_event().context("parsing SVG")? {
            Event::Eof => bail!("no root <svg> element found"),
            Event::Start(e) | Event::Empty(e) => return root_box(&e),
            _ => {}
        }
    }
}

fn root_box(e: &BytesStart) -> Result<(f64, f64, f64, f64)> {
    let local = e.local_name();
    let local = std::str::from_utf8(local.as_ref())?;
    if local != "svg" {
        bail!("root element is <{local}>, expected <svg>");
    }
    if let Some(vb) = attr_value(e, "viewBox")? {
        let n = parse_num_list(&vb);
        if n.len() != 4 {
            bail!("viewBox must have 4 numbers, got {}", n.len());
        }
        return Ok((n[0], n[1], n[2], n[3]));
    }
    match (attr_value(e, "width")?, attr_value(e, "height")?) {
        (Some(w), Some(h)) => Ok((0.0, 0.0, parse_px(&w)?, parse_px(&h)?)),
        _ => bail!("root <svg> has neither viewBox nor numeric width/height"),
    }
}

fn attr_value(e: &BytesStart, key: &str) -> Result<Option<String>> {
    for attr in e.attributes() {
        let attr = attr.map_err(|err| anyhow!("attribute: {err}"))?;
        if attr.key.as_ref() == key.as_bytes() {
            return Ok(Some(
                attr.normalized_value(quick_xml::XmlVersion::Implicit1_0)?
                    .into_owned(),
            ));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mec_of_square_corners() {
        // Unit square: MEC center (0.5,0.5), radius = sqrt(2)/2.
        let c = min_enclosing_circle(&[(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]);
        assert!((c.cx - 0.5).abs() < 1e-6);
        assert!((c.cy - 0.5).abs() < 1e-6);
        assert!((c.r - 0.5_f64.sqrt()).abs() < 1e-6);
    }

    #[test]
    fn affine_path_absolute_and_relative() {
        // abs M gets scale+translate; rel l gets scale only.
        let out = affine_path_data("M10 10 l5 0", 2.0, 3.0, 4.0);
        assert_eq!(out, "M 23 24 l 10 0");
    }

    #[test]
    fn affine_transform_translate_and_rotate() {
        assert_eq!(
            affine_transform("translate(5 5)", 2.0, 3.0, 4.0),
            "translate(10 10)"
        );
        // rotate about origin -> rotate about t.
        assert_eq!(
            affine_transform("rotate(90)", 2.0, 3.0, 4.0),
            "rotate(90 3 4)"
        );
    }

    #[test]
    fn detects_full_canvas_background() {
        let bx = (0.0, 0.0, 1024.0, 1024.0);
        let bg = BytesStart::from_content(r##"path fill="#f60" d="M0 0h1024v1024H0z""##, 4);
        assert!(is_background(&bg, bx).unwrap());

        let fg = BytesStart::from_content(r#"rect x="70" y="10" width="10" height="10""#, 4);
        assert!(!is_background(&fg, bx).unwrap());

        let unfilled =
            BytesStart::from_content(r#"path fill="none" d="M0 0h1024v1024H0z""#, 4);
        assert!(!is_background(&unfilled, bx).unwrap());
    }

    #[test]
    fn strip_removes_background_keeps_foreground() {
        let src = r##"<svg viewBox="0 0 100 100"><rect width="100" height="100" fill="#000"/><rect x="70" y="10" width="10" height="10" fill="#fff"/></svg>"##;
        let (fg, stripped) = strip_background(src, 0.0, 0.0, 100.0, 100.0).unwrap();
        assert!(stripped);
        assert!(!fg.contains(r#"width="100""#));
        assert!(fg.contains(r#"x="70""#));
    }

    // The padding values are copied through, never computed, so comparing them
    // exactly is the assertion being made.
    #[allow(clippy::float_cmp)]
    #[test]
    fn padding_default_follows_the_background() {
        let plain = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><rect x="70" y="10" width="10" height="10" fill="#fff"/></svg>"##;
        let on_bg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><rect width="100" height="100" fill="#000"/><rect x="70" y="10" width="10" height="10" fill="#fff"/></svg>"##;

        let r = round_str(plain, None).unwrap();
        assert!(r.auto_padding);
        assert_eq!(r.padding, 0.0);

        let r = round_str(on_bg, None).unwrap();
        assert!(r.auto_padding);
        assert_eq!(r.padding, DEFAULT_ROUND_PADDING);

        // An explicit value wins over both.
        let r = round_str(on_bg, Some(0.25)).unwrap();
        assert!(!r.auto_padding);
        assert_eq!(r.padding, 0.25);
    }

    #[test]
    fn padding_out_of_range_is_rejected() {
        let src = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><rect x="70" y="10" width="10" height="10" fill="#fff"/></svg>"##;
        assert!(round_str(src, Some(1.0)).is_err());
        assert!(round_str(src, Some(-0.1)).is_err());
    }

    #[test]
    fn foreground_fits_circle_background_kept() {
        let src = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><rect width="100" height="100" fill="#000"/><rect x="70" y="10" width="10" height="10" fill="#fff"/></svg>"##;
        let bx = (0.0, 0.0, 100.0, 100.0);

        // Measure foreground only.
        let (fg, _) = strip_background(src, 0.0, 0.0, 100.0, 100.0).unwrap();
        let tree = crate::render::load_tree_from_data(fg.as_bytes()).unwrap();
        let (cx, cy, r) = shape_enclosing_circle(&tree, 0.0, 0.0, 100.0, 100.0).unwrap();

        let radius = 50.0;
        let s = radius / r;
        let outs = bake_affine(src, s, 50.0 - s * cx, 50.0 - s * cy, bx).unwrap();

        // Background left as-is.
        assert!(outs.contains(r##"fill="#000""##));
        assert!(outs.contains(r#"width="100" height="100""#));

        // Rounded foreground now fills the inscribed circle (center 50,50, r 50).
        let (fg2, _) = strip_background(&outs, 0.0, 0.0, 100.0, 100.0).unwrap();
        let tree2 = crate::render::load_tree_from_data(fg2.as_bytes()).unwrap();
        let (cx2, cy2, r2) = shape_enclosing_circle(&tree2, 0.0, 0.0, 100.0, 100.0).unwrap();
        assert!((cx2 - 50.0).abs() < 1.0, "cx {cx2}");
        assert!((cy2 - 50.0).abs() < 1.0, "cy {cy2}");
        assert!((r2 - 50.0).abs() < 1.5, "r {r2}");
    }
}
