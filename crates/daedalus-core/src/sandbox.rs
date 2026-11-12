//! The Sandbox: terrain, as SketchUp's Sandbox tools make and shape it. A terrain is a surface of triangles whose inner
//! edges are soft (hidden seams), so it reads as one smooth ground.
//!
//! From Contours (a Delaunay triangulation of the contour lines' corners, seen from above), From Scratch (a flat grid),
//! Smoove (corners raised or lowered, gently less toward the edge of a circle), Stamp (the ground flattened under a shape,
//! with a slope round it), Drape (lines laid onto the ground), Add Detail (triangles split) and Flip Edge (a quad's other
//! diagonal).

use crate::Mesh;
use std::collections::{HashMap, HashSet};

type V = [f64; 3];
type P = [f64; 2];

fn orient(a: P, b: P, c: P) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

/// Whether `p` lies inside the circle through `a`, `b`, `c` (counter-clockwise).
fn in_circle(a: P, b: P, c: P, p: P) -> bool {
    let (ax, ay) = (a[0] - p[0], a[1] - p[1]);
    let (bx, by) = (b[0] - p[0], b[1] - p[1]);
    let (cx, cy) = (c[0] - p[0], c[1] - p[1]);
    let det =
        (ax * ax + ay * ay) * (bx * cy - cx * by) - (bx * bx + by * by) * (ax * cy - cx * ay) + (cx * cx + cy * cy) * (ax * by - bx * ay);
    det > 0.0
}

/// Delaunay triangles of `pts` (Bowyer-Watson), counter-clockwise, as indices into `pts`.
pub fn delaunay(pts: &[P]) -> Vec<[usize; 3]> {
    let n = pts.len();
    if n < 3 {
        return Vec::new();
    }
    let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
    for p in pts {
        for k in 0..2 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    let span = (hi[0] - lo[0]).max(hi[1] - lo[1]).max(1e-9);
    let mid = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0];
    // each point nudged by a hair (a fixed amount from its number, a billionth of the span) for the tests only: contours are
    // often rings of points on one circle, where rounding would decide the ties every which way and triangles overlap
    let nudge = |i: usize| {
        let h = (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xD1B5_4A32_D192_ED03;
        let a = ((h >> 11) as f64 / (1u64 << 53) as f64) * 2.0 - 1.0;
        let b = ((h.rotate_left(29) >> 11) as f64 / (1u64 << 53) as f64) * 2.0 - 1.0;
        [a * span * 1e-9, b * span * 1e-9]
    };
    // a triangle round everything, far out
    let mut all: Vec<P> = pts
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let d = nudge(i);
            [p[0] + d[0], p[1] + d[1]]
        })
        .collect();
    all.push([mid[0] - 40.0 * span, mid[1] - 20.0 * span]);
    all.push([mid[0] + 40.0 * span, mid[1] - 20.0 * span]);
    all.push([mid[0], mid[1] + 40.0 * span]);
    let mut tris: Vec<[usize; 3]> = vec![[n, n + 1, n + 2]];
    for i in 0..n {
        let p = all[i];
        let (bad, good): (Vec<[usize; 3]>, Vec<[usize; 3]>) = tris.into_iter().partition(|t| in_circle(all[t[0]], all[t[1]], all[t[2]], p));
        tris = good;
        // the hole's border: edges of the bad triangles not shared by two of them
        let mut count: HashMap<(usize, usize), usize> = HashMap::new();
        for t in &bad {
            for k in 0..3 {
                let (a, b) = (t[k], t[(k + 1) % 3]);
                *count.entry((a.min(b), a.max(b))).or_default() += 1;
            }
        }
        for t in &bad {
            for k in 0..3 {
                let (a, b) = (t[k], t[(k + 1) % 3]);
                if count[&(a.min(b), a.max(b))] == 1 {
                    tris.push([a, b, i]);
                }
            }
        }
    }
    tris.into_iter()
        .filter(|t| t.iter().all(|&v| v < n))
        .map(|t| if orient(all[t[0]], all[t[1]], all[t[2]]) < 0.0 { [t[0], t[2], t[1]] } else { t })
        .filter(|t| orient(all[t[0]], all[t[1]], all[t[2]]).abs() > 1e-12 * span * span)
        .collect()
}

/// Triangles over `pts` (seen from above) as a terrain: faces looking up, inner edges soft.
fn terrain(pts: Vec<V>, tris: &[[usize; 3]]) -> Mesh {
    let mut m = Mesh { positions: pts, ..Default::default() };
    for t in tris {
        m.faces.push(t.iter().map(|&v| v as u32).collect());
    }
    soften(&mut m);
    m
}

/// Every edge between two faces becomes soft (a hidden seam).
pub fn soften(m: &mut Mesh) {
    let mut count: HashMap<(u32, u32), usize> = HashMap::new();
    for (_, a, b) in m.face_edges() {
        *count.entry((a.min(b), a.max(b))).or_default() += 1;
    }
    m.seams = count.into_iter().filter(|&(_, c)| c == 2).map(|((a, b), _)| [a, b]).collect();
    m.seams.sort_unstable();
}

/// From Contours: a terrain through the corners of contour lines (points closer than `tol` across are taken once).
pub fn from_contours(points: &[V], tol: f64) -> Option<Mesh> {
    let mut seen: HashSet<[i64; 2]> = HashSet::new();
    let pts: Vec<V> = points.iter().copied().filter(|p| seen.insert([(p[0] / tol).round() as i64, (p[1] / tol).round() as i64])).collect();
    let flat: Vec<P> = pts.iter().map(|p| [p[0], p[1]]).collect();
    let tris = delaunay(&flat);
    (!tris.is_empty()).then(|| terrain(pts, &tris))
}

/// From Scratch: a flat grid from `origin` across `width` (along x) and `depth` (along y), squares of `spacing` (the last
/// row and column take what is left), each split into two triangles.
pub fn grid(origin: V, width: f64, depth: f64, spacing: f64) -> Option<Mesh> {
    if !(width > 0.0 && depth > 0.0 && spacing > 0.0) {
        return None;
    }
    let steps = |len: f64| -> Vec<f64> {
        let n = (len / spacing).ceil().max(1.0) as usize;
        (0..=n).map(|k| (k as f64 * spacing).min(len)).collect()
    };
    let (xs, ys) = (steps(width), steps(depth));
    if xs.len() * ys.len() > 250_000 {
        return None; // too fine to be of use
    }
    let mut pts = Vec::new();
    for &y in &ys {
        for &x in &xs {
            pts.push([origin[0] + x, origin[1] + y, origin[2]]);
        }
    }
    let w = xs.len();
    let mut tris = Vec::new();
    let mut diagonals = Vec::new();
    for j in 0..ys.len() - 1 {
        for i in 0..w - 1 {
            let (a, b, c, d) = (j * w + i, j * w + i + 1, (j + 1) * w + i + 1, (j + 1) * w + i);
            tris.push([a, b, c]);
            tris.push([a, c, d]);
            diagonals.push([a.min(c) as u32, a.max(c) as u32]);
        }
    }
    // the grid's squares show; only their diagonals are soft, as SketchUp draws it
    let mut m = terrain(pts, &tris);
    m.seams = diagonals;
    Some(m)
}

/// Smoove: every corner within `radius` (seen from above) of `center` raised by `height`, less the further out it is (a
/// smooth bump: all of it at the middle, none at the edge). Returns how many moved.
pub fn smoove(m: &mut Mesh, center: V, radius: f64, height: f64) -> usize {
    let mut moved = 0;
    for p in &mut m.positions {
        let d = ((p[0] - center[0]).powi(2) + (p[1] - center[1]).powi(2)).sqrt();
        if d < radius {
            p[2] += height * 0.5 * (1.0 + (std::f64::consts::PI * d / radius).cos());
            moved += 1;
        }
    }
    moved
}

fn point_in(poly: &[P], p: P) -> bool {
    let mut inside = false;
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        if (a[1] > p[1]) != (b[1] > p[1]) && p[0] < a[0] + (p[1] - a[1]) * (b[0] - a[0]) / (b[1] - a[1]) {
            inside = !inside;
        }
    }
    inside
}

fn dist_to_poly(poly: &[P], p: P) -> f64 {
    (0..poly.len())
        .map(|i| {
            let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
            let d = [b[0] - a[0], b[1] - a[1]];
            let l2 = d[0] * d[0] + d[1] * d[1];
            let t = if l2 > 0.0 { (((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / l2).clamp(0.0, 1.0) } else { 0.0 };
            ((p[0] - a[0] - d[0] * t).powi(2) + (p[1] - a[1] - d[1] * t).powi(2)).sqrt()
        })
        .fold(f64::MAX, f64::min)
}

/// Stamp: the ground under the outline `footprint` (seen from above) set to `height`, and within `offset` round it eased
/// back to where it was. Returns how many corners moved.
pub fn stamp(m: &mut Mesh, footprint: &[P], height: f64, offset: f64) -> usize {
    let mut moved = 0;
    for p in &mut m.positions {
        let q = [p[0], p[1]];
        if point_in(footprint, q) {
            p[2] = height;
            moved += 1;
        } else if offset > 0.0 {
            let d = dist_to_poly(footprint, q);
            if d < offset {
                let t = d / offset;
                let w = 0.5 * (1.0 + (std::f64::consts::PI * t).cos());
                p[2] = height * w + p[2] * (1.0 - w);
                moved += 1;
            }
        }
    }
    moved
}

/// The ground's height under `p` (seen from above), from the triangle it lies in, if any.
pub fn height_at(m: &Mesh, p: P) -> Option<f64> {
    for f in 0..m.faces.len() {
        for t in m.triangulate_face(f) {
            let [a, b, c] = t.map(|v| m.positions[v as usize]);
            let (a2, b2, c2) = ([a[0], a[1]], [b[0], b[1]], [c[0], c[1]]);
            let area = orient(a2, b2, c2);
            if area.abs() < 1e-15 {
                continue;
            }
            let (w0, w1, w2) = (orient(b2, c2, p) / area, orient(c2, a2, p) / area, orient(a2, b2, p) / area);
            if w0 >= -1e-9 && w1 >= -1e-9 && w2 >= -1e-9 {
                return Some(w0 * a[2] + w1 * b[2] + w2 * c[2]);
            }
        }
    }
    None
}

/// Drape: the path `path` (seen from above) laid on the ground: its corners and every place it crosses one of the ground's
/// edges, each at the ground's height. Pieces off the ground are left out (the result may be several runs).
pub fn drape(m: &Mesh, path: &[P]) -> Vec<Vec<V>> {
    let edges = m.edges();
    let mut runs: Vec<Vec<V>> = Vec::new();
    let mut run: Vec<V> = Vec::new();
    let put = |run: &mut Vec<V>, runs: &mut Vec<Vec<V>>, p: P| match height_at(m, p) {
        Some(z) => {
            let q = [p[0], p[1], z];
            if run.last().is_none_or(|l: &V| (l[0] - q[0]).abs() + (l[1] - q[1]).abs() > 1e-9) {
                run.push(q);
            }
        }
        None => {
            if run.len() >= 2 {
                runs.push(std::mem::take(run));
            }
            run.clear();
        }
    };
    for w in path.windows(2) {
        let (a, b) = (w[0], w[1]);
        put(&mut run, &mut runs, a);
        // crossings with the ground's edges, in order along the piece
        let mut cross: Vec<(f64, P)> = Vec::new();
        for &(u, v) in &edges {
            let (pu, pv) = (m.positions[u as usize], m.positions[v as usize]);
            let (c, d) = ([pu[0], pu[1]], [pv[0], pv[1]]);
            let r = [b[0] - a[0], b[1] - a[1]];
            let s = [d[0] - c[0], d[1] - c[1]];
            let den = r[0] * s[1] - r[1] * s[0];
            if den.abs() < 1e-15 {
                continue;
            }
            let t = ((c[0] - a[0]) * s[1] - (c[1] - a[1]) * s[0]) / den;
            let k = ((c[0] - a[0]) * r[1] - (c[1] - a[1]) * r[0]) / den;
            if t > 1e-9 && t < 1.0 - 1e-9 && (0.0..=1.0).contains(&k) {
                cross.push((t, [a[0] + r[0] * t, a[1] + r[1] * t]));
            }
        }
        cross.sort_by(|x, y| x.0.total_cmp(&y.0));
        for (_, p) in cross {
            put(&mut run, &mut runs, p);
        }
    }
    if let Some(&last) = path.last() {
        put(&mut run, &mut runs, last);
    }
    if run.len() >= 2 {
        runs.push(run);
    }
    runs
}

/// Add Detail: each of `faces` split into triangles round a new corner at its middle (its edges stay, so neighbours are
/// untouched). The new edges are soft.
pub fn add_detail(m: &mut Mesh, faces: &[usize]) {
    let mut faces: Vec<usize> = faces.iter().copied().filter(|&f| f < m.faces.len() && m.holes_of(f).is_empty()).collect();
    faces.sort_unstable_by(|a, b| b.cmp(a));
    faces.dedup();
    for f in faces {
        let ring = m.faces[f].clone();
        let n = ring.len() as f64;
        let c = ring.iter().fold([0.0; 3], |s, &v| {
            let p = m.positions[v as usize];
            [s[0] + p[0] / n, s[1] + p[1] / n, s[2] + p[2] / n]
        });
        let paint = m.color_of(f);
        let mid = m.add_vertex(c);
        m.remove_face(f);
        for i in 0..ring.len() {
            m.faces.push(vec![ring[i], ring[(i + 1) % ring.len()], mid]);
            let k = m.faces.len() - 1;
            m.set_color(k, paint);
            m.seams.push([ring[i].min(mid), ring[i].max(mid)]);
        }
    }
}

/// Flip Edge: the edge `a`-`b` between two triangles swapped for the other diagonal of the four corners round it. None
/// when it is not between two triangles, or the swap would fold the surface over (the four corners not convex from above).
pub fn flip_edge(m: &mut Mesh, a: u32, b: u32) -> Option<()> {
    let on: Vec<usize> = (0..m.faces.len())
        .filter(|&f| {
            let r = &m.faces[f];
            r.len() == 3 && m.holes_of(f).is_empty() && r.contains(&a) && r.contains(&b)
        })
        .collect();
    let [f, g] = on[..] else { return None };
    let other = |f: usize| *m.faces[f].iter().find(|&&v| v != a && v != b).expect("a triangle");
    let (c, d) = (other(f), other(g));
    // f runs a -> b (then c), g runs b -> a (then d), or the other way round
    let fr = &m.faces[f];
    let i = fr.iter().position(|&v| v == a).expect("there");
    let (a, b) = if fr[(i + 1) % 3] == b { (a, b) } else { (b, a) };
    let p = |v: u32| [m.positions[v as usize][0], m.positions[v as usize][1]];
    // the new triangles c-a-d and d-b-c must both turn the same way as the old ones
    let old = orient(p(a), p(b), p(c)).signum();
    if orient(p(c), p(a), p(d)).signum() != old || orient(p(d), p(b), p(c)).signum() != old {
        return None;
    }
    let (pf, pg) = (m.color_of(f), m.color_of(g));
    m.faces[f] = vec![c, a, d];
    m.faces[g] = vec![d, b, c];
    m.set_color(f, pf);
    m.set_color(g, pg);
    let key = (a.min(b), a.max(b));
    m.seams.retain(|&[x, y]| (x.min(y), x.max(y)) != key);
    m.seams.push([c.min(d), c.max(d)]);
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::HalfEdgeMesh;

    #[test]
    fn delaunay_of_a_square_and_its_middle() {
        let t = delaunay(&[[0.0, 0.0], [2.0, 0.0], [2.0, 2.0], [0.0, 2.0], [1.0, 1.0]]);
        assert_eq!(t.len(), 4);
    }

    #[test]
    fn contours_make_a_hill() {
        // three rings, the higher the smaller: a hill
        let mut pts = Vec::new();
        for (r, z) in [(10.0, 0.0), (6.0, 2.0), (2.0, 4.0)] {
            for k in 0..16 {
                let t = std::f64::consts::TAU * k as f64 / 16.0;
                pts.push([r * t.cos(), r * t.sin(), z]);
            }
        }
        let m = from_contours(&pts, 1e-6).unwrap();
        let h = HalfEdgeMesh::from_mesh(&m).expect("a good surface");
        assert!(h.face_count() > 40);
        assert!((0..h.face_count()).all(|f| h.face_normal(f)[2] > 0.0), "every face looks up");
        assert!(!m.seams.is_empty(), "inner edges are soft");
        assert!(height_at(&m, [0.0, 0.0]).unwrap() > 3.9, "the top");
    }

    #[test]
    fn a_grid_smooved_into_a_bump_then_stamped_flat() {
        let mut m = grid([0.0; 3], 10.0, 10.0, 1.0).unwrap();
        assert_eq!(m.faces.len(), 200);
        assert_eq!(smoove(&mut m, [5.0, 5.0, 0.0], 3.0, 2.0), 25, "the corners within 3 m");
        assert!((height_at(&m, [5.0, 5.0]).unwrap() - 2.0).abs() < 1e-9, "all of it at the middle");
        assert_eq!(height_at(&m, [9.0, 9.0]), Some(0.0), "none outside the circle");
        stamp(&mut m, &[[4.0, 4.0], [6.0, 4.0], [6.0, 6.0], [4.0, 6.0]], 1.0, 1.0);
        assert!((height_at(&m, [5.0, 5.0]).unwrap() - 1.0).abs() < 1e-9, "flat under the stamp");
        HalfEdgeMesh::from_mesh(&m).expect("still a good surface");
    }

    #[test]
    fn a_line_draped_over_a_bump() {
        let mut m = grid([0.0; 3], 10.0, 10.0, 1.0).unwrap();
        smoove(&mut m, [5.0, 5.0, 0.0], 3.0, 2.0);
        let runs = drape(&m, &[[0.5, 5.0], [9.5, 5.0]]);
        assert_eq!(runs.len(), 1);
        let peak = runs[0].iter().map(|p| p[2]).fold(f64::MIN, f64::max);
        assert!((peak - 2.0).abs() < 1e-9, "over the top: {peak}");
        assert!(runs[0].len() > 10, "a point at every edge crossed");
    }

    #[test]
    fn detail_and_flip() {
        let mut m = grid([0.0; 3], 2.0, 1.0, 1.0).unwrap();
        assert_eq!(m.faces.len(), 4);
        add_detail(&mut m, &[0]);
        assert_eq!(m.faces.len(), 6);
        HalfEdgeMesh::from_mesh(&m).expect("still a good surface");
        // the diagonal of the second square: 1 -> 5 (corners 1, 2 along the bottom, 4, 5 along the top)
        let mut g = grid([0.0; 3], 2.0, 1.0, 1.0).unwrap();
        flip_edge(&mut g, 1, 5).expect("flipped");
        assert!(g.edges().contains(&(2, 4)), "the other diagonal");
        HalfEdgeMesh::from_mesh(&g).expect("still a good surface");
    }
}
