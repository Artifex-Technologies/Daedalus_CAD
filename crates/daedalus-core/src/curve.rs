//! Arcs and circles as one piece, as SketchUp treats them: a curve is drawn as straight segments (they snap and align like
//! any edge), and a chain of them is recognised as one arc or circle from its shape, so it can be selected whole and its
//! radius and number of segments changed (Entity Info).
//!
//! Recognised: at least three segments of equal length, in one plane, every corner the same distance from one centre and
//! turning the same way by the same angle, of at most 60 degrees (so a rectangle or a hexagon drawn as such is not taken
//! for a circle). Nothing is stored, so the curve survives every edit that keeps its shape.

use crate::Mesh;
use std::collections::HashMap;

type V = [f64; 3];

fn sub(a: V, b: V) -> V {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn add(a: V, b: V) -> V {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn scale(a: V, s: f64) -> V {
    [a[0] * s, a[1] * s, a[2] * s]
}
fn dot(a: V, b: V) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: V, b: V) -> V {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn len(a: V) -> f64 {
    dot(a, a).sqrt()
}

/// The steepest turn between two segments still taken for a curve (a hexagon's corners turn exactly this much: not a curve).
const MAX_TURN: f64 = std::f64::consts::PI / 3.0 - 1e-6;

/// An arc or circle found in a mesh.
#[derive(Debug, Clone, PartialEq)]
pub struct Curve {
    /// Its vertices in order along it (a circle's first is not repeated at the end).
    pub verts: Vec<u32>,
    pub closed: bool,
    pub center: V,
    /// Unit normal of its plane, the curve running counter-clockwise about it.
    pub normal: V,
    pub radius: f64,
    /// The angle it sweeps, radians (a full turn for a circle).
    pub sweep: f64,
}

impl Curve {
    pub fn segments(&self) -> usize {
        if self.closed {
            self.verts.len()
        } else {
            self.verts.len() - 1
        }
    }

    /// Its length along the segments.
    pub fn length(&self, mesh: &Mesh) -> f64 {
        let p = |i: usize| mesh.positions[self.verts[i % self.verts.len()] as usize];
        (0..self.segments()).map(|i| len(sub(p(i + 1), p(i)))).sum()
    }

    /// Its edges, as vertex pairs (lower number first).
    pub fn edges(&self) -> Vec<[u32; 2]> {
        let n = self.verts.len();
        (0..self.segments())
            .map(|i| {
                let (a, b) = (self.verts[i], self.verts[(i + 1) % n]);
                [a.min(b), a.max(b)]
            })
            .collect()
    }
}

/// The centre of the circle through three points, and its normal (a to b to c counter-clockwise about it).
fn circle_through(a: V, b: V, c: V) -> Option<(V, V)> {
    let (ab, ac) = (sub(b, a), sub(c, a));
    let n = cross(ab, ac);
    let nn = dot(n, n);
    if nn < 1e-30 {
        return None;
    }
    let to = add(scale(cross(n, ab), dot(ac, ac)), scale(cross(ac, n), dot(ab, ab)));
    Some((add(a, scale(to, 0.5 / nn)), scale(n, 1.0 / nn.sqrt())))
}

/// The arc or circle the edge `a`-`b` belongs to, if it is part of one.
pub fn curve_through(mesh: &Mesh, a: u32, b: u32) -> Option<Curve> {
    let tol = mesh.tolerance() * 10.0;
    let mut nbrs: HashMap<u32, Vec<u32>> = HashMap::new();
    for (x, y) in mesh.visible_edges() {
        nbrs.entry(x).or_default().push(y);
        nbrs.entry(y).or_default().push(x);
    }
    let p = |v: u32| mesh.positions[v as usize];
    let seg = len(sub(p(b), p(a)));
    if seg <= tol {
        return None;
    }
    // the next corner after `from` -> `at` that continues the same turn (`turn`: the angle and the plane's normal)
    let step = |from: u32, at: u32, shape: Option<(V, V, f64)>| -> Option<(u32, V, V, f64)> {
        let mut found = None;
        for &next in nbrs.get(&at)?.iter() {
            if next == from || (len(sub(p(next), p(at))) - seg).abs() > tol.max(seg * 1e-6) {
                continue;
            }
            let Some((c, n)) = circle_through(p(from), p(at), p(next)) else { continue };
            let r = len(sub(p(at), c));
            let turn = (dot(sub(p(at), p(from)), sub(p(next), p(at))) / (seg * seg)).clamp(-1.0, 1.0).acos();
            if turn > MAX_TURN {
                continue;
            }
            if let Some((c0, n0, r0)) = shape {
                // walked either way along it, the plane's normal may point either way
                if len(sub(c, c0)) > tol.max(r0 * 1e-6) || dot(n, n0).abs() < 1.0 - 1e-6 || (r - r0).abs() > tol.max(r0 * 1e-6) {
                    continue;
                }
            }
            if found.is_some() {
                return None; // two ways on: not one curve
            }
            found = Some((next, c, n, r));
        }
        found
    };
    // the shape from the first turn found either side of a-b
    let (center, normal, radius) = match step(a, b, None) {
        Some((_, c, n, r)) => (c, n, r),
        None => {
            let (_, c, n, r) = step(b, a, None)?;
            (c, n, r)
        }
    };
    let shape = Some((center, normal, radius));
    // walk on past b, then back past a
    let mut chain = vec![a, b];
    let (mut from, mut at) = (a, b);
    let mut closed = false;
    while let Some((next, ..)) = step(from, at, shape) {
        if next == chain[0] {
            closed = true;
            break;
        }
        if chain.contains(&next) {
            return None;
        }
        chain.push(next);
        (from, at) = (at, next);
    }
    if !closed {
        let (mut from, mut at) = (b, a);
        while let Some((next, ..)) = step(from, at, shape) {
            if chain.contains(&next) {
                return None;
            }
            chain.insert(0, next);
            (from, at) = (at, next);
        }
    }
    let segments = if closed { chain.len() } else { chain.len() - 1 };
    if segments < 3 {
        return None;
    }
    // counter-clockwise about the normal
    let mut normal = normal;
    let turn_dir = cross(sub(p(chain[1]), p(chain[0])), sub(p(chain[2]), p(chain[1])));
    if dot(turn_dir, normal) < 0.0 {
        normal = scale(normal, -1.0);
    }
    let angle = 2.0 * (seg / (2.0 * radius)).clamp(-1.0, 1.0).asin();
    let sweep = if closed { std::f64::consts::TAU } else { angle * segments as f64 };
    Some(Curve { verts: chain, closed, center, normal, radius, sweep })
}

/// Whether the curve can still be reshaped: every face on it lies in its plane (SketchUp allows it until the curve is
/// pushed or pulled into walls).
pub fn reshapeable(mesh: &Mesh, c: &Curve) -> bool {
    let tol = mesh.tolerance() * 10.0;
    let mine: std::collections::HashSet<(u32, u32)> = c.edges().iter().map(|e| (e[0], e[1])).collect();
    (0..mesh.faces.len()).all(|f| {
        let touches =
            mesh.loops(f).any(|r| (0..r.len()).any(|i| mine.contains(&(r[i].min(r[(i + 1) % r.len()]), r[i].max(r[(i + 1) % r.len()])))));
        !touches || mesh.loops(f).flatten().all(|&v| dot(sub(mesh.positions[v as usize], c.center), c.normal).abs() <= tol)
    })
}

/// Redraws the curve with `segments` segments and radius `radius` (same centre, plane, start and sweep), in every face it
/// bounds. Its old corners stay in the mesh, unused. Fails when the curve has been made into walls (see [`reshapeable`]).
pub fn reshape(mesh: &mut Mesh, c: &Curve, segments: usize, radius: f64) -> Result<Curve, &'static str> {
    let need = if c.closed { 3 } else { 1 };
    if segments < need || segments > 1000 {
        return Err("a curve needs from 3 (a circle) or 1 (an arc) up to 1000 segments");
    }
    if !(radius.is_finite() && radius > 0.0) {
        return Err("the radius must be more than zero");
    }
    if !reshapeable(mesh, c) {
        return Err("This curve has been pushed or pulled into walls: its segments can no longer change (as in SketchUp).");
    }
    // the start direction and the in-plane axis a quarter turn on
    let start = sub(mesh.positions[c.verts[0] as usize], c.center);
    let u = scale(start, 1.0 / len(start));
    let v = cross(c.normal, u);
    let point = |k: usize| {
        let t = c.sweep * k as f64 / segments as f64;
        add(c.center, add(scale(u, radius * t.cos()), scale(v, radius * t.sin())))
    };
    // the new corners: an arc keeps its ends (moved out to the new radius), a circle its first corner
    let count = if c.closed { segments } else { segments + 1 };
    let ends = (c.verts[0], *c.verts.last().expect("three at least"));
    let mut new: Vec<u32> = Vec::with_capacity(count);
    for k in 0..count {
        if k == 0 {
            mesh.positions[ends.0 as usize] = point(0);
            new.push(ends.0);
        } else if !c.closed && k == count - 1 {
            mesh.positions[ends.1 as usize] = point(k);
            new.push(ends.1);
        } else {
            new.push(mesh.add_vertex(point(k)));
        }
    }
    // in each loop that runs along the old curve (either way), the old run of corners becomes the new one
    let old = &c.verts;
    let replace = |ring: &mut Vec<u32>| {
        let n = ring.len();
        let Some(i0) = ring.iter().position(|&x| x == old[0]) else { return };
        let backward = ring[(i0 + 1) % n] != old[1];
        if backward && ring[(i0 + n - 1) % n] != old[1] {
            return;
        }
        // walked the curve's way, starting at its start
        let mut r: Vec<u32> = if backward { ring.iter().rev().copied().collect() } else { ring.clone() };
        let j0 = r.iter().position(|&x| x == old[0]).expect("there");
        r.rotate_left(j0);
        if r.len() < old.len() || r[..old.len()] != old[..] {
            return;
        }
        let mut out: Vec<u32> = new.clone();
        out.extend_from_slice(&r[old.len()..]);
        if out.len() < 3 {
            return;
        }
        if backward {
            out.reverse();
        }
        *ring = out;
    };
    for ring in mesh.faces.iter_mut().chain(mesh.holes.iter_mut().flatten()) {
        replace(ring);
    }
    // loose lines along it
    let edges: std::collections::HashSet<(u32, u32)> = c.edges().iter().map(|e| (e[0], e[1])).collect();
    let before = mesh.lines.len();
    mesh.lines.retain(|&[a, b]| !edges.contains(&(a.min(b), a.max(b))));
    if mesh.lines.len() < before {
        for k in 0..new.len() - 1 {
            mesh.lines.push([new[k], new[k + 1]]);
        }
        if c.closed {
            mesh.lines.push([*new.last().expect("some"), new[0]]);
        }
    }
    Ok(Curve { verts: new, closed: c.closed, center: c.center, normal: c.normal, radius, sweep: c.sweep })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::HalfEdgeMesh;

    fn circle(segments: usize, r: f64) -> Vec<[f64; 3]> {
        (0..segments)
            .map(|k| {
                let t = std::f64::consts::TAU * k as f64 / segments as f64;
                [r * t.cos(), r * t.sin(), 0.0]
            })
            .collect()
    }

    #[test]
    fn a_circle_is_found_from_any_of_its_edges_and_a_square_is_not() {
        let mut h = HalfEdgeMesh::default();
        h.add_polygon(&circle(24, 2.0)).unwrap();
        let m = h.to_mesh();
        let ring = m.faces[0].clone();
        let c = curve_through(&m, ring[5], ring[6]).expect("a circle");
        assert!(c.closed);
        assert_eq!(c.segments(), 24);
        assert!((c.radius - 2.0).abs() < 1e-9 && len(c.center) < 1e-9);
        assert!((c.length(&m) - 24.0 * 2.0 * 2.0 * (std::f64::consts::PI / 24.0).sin()).abs() < 1e-9);
        let mut sq = HalfEdgeMesh::default();
        sq.add_polygon(&[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]]).unwrap();
        assert!(curve_through(&sq.to_mesh(), 0, 1).is_none(), "a square is no circle");
        let mut hex = HalfEdgeMesh::default();
        hex.add_polygon(&circle(6, 1.0)).unwrap();
        assert!(curve_through(&hex.to_mesh(), 0, 1).is_none(), "nor is a hexagon");
    }

    #[test]
    fn a_circle_gets_more_segments_and_its_face_follows() {
        let mut h = HalfEdgeMesh::default();
        h.add_polygon(&circle(12, 1.0)).unwrap();
        let mut m = h.to_mesh();
        let c = curve_through(&m, m.faces[0][0], m.faces[0][1]).unwrap();
        let c2 = reshape(&mut m, &c, 48, 1.5).unwrap();
        assert_eq!(m.faces[0].len(), 48);
        assert_eq!(c2.segments(), 48);
        let h2 = HalfEdgeMesh::from_mesh(&m).unwrap();
        assert_eq!(h2.face_count(), 1);
        // its area is near a circle's of radius 1.5
        let area: f64 = m
            .triangulate_face(0)
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|v| m.positions[v as usize]);
                len(cross(sub(b, a), sub(c, a))) / 2.0
            })
            .sum();
        assert!((area - std::f64::consts::PI * 2.25).abs() < 0.03, "{area}");
        let again = curve_through(&m, m.faces[0][3], m.faces[0][4]).unwrap();
        assert_eq!(again.segments(), 48);
    }

    #[test]
    fn a_pushed_circle_keeps_its_segments() {
        let mut h = HalfEdgeMesh::default();
        h.add_polygon(&circle(12, 1.0)).unwrap();
        h.push_pull(0, 1.0).unwrap();
        let mut m = h.to_mesh();
        let ring = m.faces.iter().find(|f| f.len() == 12).unwrap().clone();
        let c = curve_through(&m, ring[0], ring[1]).expect("still a circle");
        assert!(!reshapeable(&m, &c));
        assert!(reshape(&mut m, &c, 24, 1.0).is_err());
    }

    #[test]
    fn an_arc_keeps_its_ends() {
        // a quarter circle of 6 segments as loose lines
        let mut m = Mesh::new();
        let pts: Vec<u32> = (0..=6)
            .map(|k| {
                let t = std::f64::consts::FRAC_PI_2 * k as f64 / 6.0;
                m.add_vertex([t.cos(), t.sin(), 0.0])
            })
            .collect();
        for w in pts.windows(2) {
            m.lines.push([w[0], w[1]]);
        }
        let c = curve_through(&m, pts[2], pts[3]).unwrap();
        assert!(!c.closed);
        assert_eq!(c.segments(), 6);
        assert!((c.sweep - std::f64::consts::FRAC_PI_2).abs() < 1e-9);
        let c2 = reshape(&mut m, &c, 3, 1.0).unwrap();
        assert_eq!(c2.segments(), 3);
        assert_eq!(m.lines.len(), 3);
        assert_eq!((c2.verts[0], *c2.verts.last().unwrap()), (pts[0], pts[6]));
    }
}
