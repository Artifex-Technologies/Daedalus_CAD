//! Ear-clipping triangulation of planar polygons, so concave faces (an L-shaped
//! room drawn with the Line tool) render, pick and export correctly.

/// Triangulates one planar face ring (vertex indices into `positions`) by ear
/// clipping. Triangles keep the ring's winding. Degenerate input falls back to
/// a fan so that every face still produces `ring.len() - 2` triangles.
pub fn triangulate(positions: &[[f64; 3]], ring: &[u32]) -> Vec<[u32; 3]> {
    let n = ring.len();
    if n < 3 {
        return Vec::new();
    }
    if n == 3 {
        return vec![[ring[0], ring[1], ring[2]]];
    }

    // Project onto the plane most perpendicular to the Newell normal.
    let normal = newell(positions, ring);
    let (ax, ay) = match dominant_axis(normal) {
        0 => (1, 2),
        1 => (2, 0),
        _ => (0, 1),
    };
    let pts: Vec<[f64; 2]> = ring
        .iter()
        .map(|&v| {
            let p = positions[v as usize];
            [p[ax], p[ay]]
        })
        .collect();
    // After dropping the dominant axis the ring may appear clockwise; ears are
    // convex corners relative to the ring's own orientation.
    let orientation = signed_area_2d(&pts).signum();

    let mut idx: Vec<usize> = (0..n).collect();
    let mut out = Vec::with_capacity(n - 2);
    let mut guard = 0;
    while idx.len() > 3 && guard < n * n {
        guard += 1;
        let m = idx.len();
        let ear = (0..m).find(|&i| {
            let (a, b, c) = (idx[(i + m - 1) % m], idx[i], idx[(i + 1) % m]);
            let turn = cross_2d(pts[a], pts[b], pts[c]) * orientation;
            if turn <= 0.0 {
                return false;
            }
            !idx.iter().any(|&p| p != a && p != b && p != c && in_triangle(pts[p], pts[a], pts[b], pts[c]))
        });
        match ear {
            Some(i) => {
                let (a, b, c) = (idx[(i + m - 1) % m], idx[i], idx[(i + 1) % m]);
                out.push([ring[a], ring[b], ring[c]]);
                idx.remove(i);
            }
            None => break,
        }
    }
    // Three left, or no ear found (collinear or self-touching input): fan the rest.
    for i in 1..idx.len() - 1 {
        out.push([ring[idx[0]], ring[idx[i]], ring[idx[i + 1]]]);
    }
    out
}

/// Triangulates a face with holes, the holes left open. Each hole is joined to the outline by a bridge (a cut walked there
/// and back), the hole reaching furthest along the face first, so the face becomes one outline for ear clipping.
pub fn triangulate_with_holes(positions: &[[f64; 3]], outer: &[u32], holes: &[Vec<u32>]) -> Vec<[u32; 3]> {
    if outer.len() < 3 {
        return Vec::new();
    }
    let (ax, ay) = match dominant_axis(newell(positions, outer)) {
        0 => (1, 2),
        1 => (2, 0),
        _ => (0, 1),
    };
    let flat = |v: u32| {
        let p = positions[v as usize];
        [p[ax], p[ay]]
    };
    let area = |r: &[u32]| signed_area_2d(&r.iter().map(|&v| flat(v)).collect::<Vec<_>>());
    // worked with the outline counter-clockwise (x mirrored if it is not) and every hole clockwise
    let mirror = if area(outer) < 0.0 { -1.0 } else { 1.0 };
    let pt = |v: u32| {
        let q = flat(v);
        [q[0] * mirror, q[1]]
    };
    let mut ring: Vec<u32> = outer.to_vec();
    let mut hs: Vec<Vec<u32>> = holes
        .iter()
        .filter(|h| h.len() >= 3)
        .map(|h| {
            let mut h = h.clone();
            if area(&h) * mirror > 0.0 {
                h.reverse();
            }
            h
        })
        .collect();
    let reach = |h: &Vec<u32>| h.iter().map(|&v| pt(v)[0]).fold(f64::MIN, f64::max);
    hs.sort_by(|a, b| reach(b).total_cmp(&reach(a)));
    for h in hs {
        // M: the hole's corner furthest along x
        let mi = (0..h.len()).max_by(|&a, &b| pt(h[a])[0].total_cmp(&pt(h[b])[0])).expect("three corners");
        let m = pt(h[mi]);
        // the nearest edge of the outline (holes joined so far included) met by a ray from M along +x
        let mut hit: Option<(f64, usize)> = None;
        for i in 0..ring.len() {
            let (a, b) = (pt(ring[i]), pt(ring[(i + 1) % ring.len()]));
            if (a[1] - m[1]) * (b[1] - m[1]) > 0.0 || a[1] == b[1] {
                continue;
            }
            let x = a[0] + (m[1] - a[1]) * (b[0] - a[0]) / (b[1] - a[1]);
            if x >= m[0] && hit.is_none_or(|(bx, _)| x < bx) {
                hit = Some((x, i));
            }
        }
        let pi = match hit {
            Some((x, i)) => {
                let j = (i + 1) % ring.len();
                // the end of that edge further along x, unless another corner inside the triangle M, I, P would block the
                // bridge: then the one of those at the smallest angle from the ray
                let p = if pt(ring[i])[0] >= pt(ring[j])[0] { i } else { j };
                let (pp, ip) = (pt(ring[p]), [x, m[1]]);
                let blocking = (0..ring.len())
                    .filter(|&k| k != p && ring[k] != ring[p] && pt(ring[k])[0] > m[0] && in_triangle(pt(ring[k]), m, ip, pp))
                    .min_by(|&a, &b| {
                        let ang = |k: usize| {
                            let q = pt(ring[k]);
                            ((q[1] - m[1]).abs() / (q[0] - m[0]), (q[0] - m[0]).powi(2) + (q[1] - m[1]).powi(2))
                        };
                        ang(a).partial_cmp(&ang(b)).unwrap_or(std::cmp::Ordering::Equal)
                    });
                blocking.unwrap_or(p)
            }
            // nothing to the right (a hole touching the outline): the nearest corner
            None => (0..ring.len())
                .min_by(|&a, &b| {
                    let d = |k: usize| (pt(ring[k])[0] - m[0]).powi(2) + (pt(ring[k])[1] - m[1]).powi(2);
                    d(a).total_cmp(&d(b))
                })
                .expect("an outline"),
        };
        // the corner may be on the outline twice (a bridge to an earlier hole left from it): the copy whose corner opens
        // toward M, or the new bridge would cross the old one
        let pi = {
            let v = ring[pi];
            let copies: Vec<usize> = (0..ring.len()).filter(|&k| ring[k] == v).collect();
            let len = ring.len();
            copies
                .iter()
                .copied()
                .find(|&k| in_cone(pt(ring[(k + len - 1) % len]), pt(ring[k]), pt(ring[(k + 1) % len]), m))
                .unwrap_or(pi)
        };
        let mut joined = Vec::with_capacity(ring.len() + h.len() + 2);
        joined.extend_from_slice(&ring[..=pi]);
        joined.extend(h[mi..].iter().chain(&h[..mi]).copied());
        joined.push(h[mi]);
        joined.extend_from_slice(&ring[pi..]);
        ring = joined;
    }
    let pts: Vec<[f64; 2]> = ring.iter().map(|&v| pt(v)).collect();
    clip(&ring, &pts, 1.0)
}

/// Whether point `b` lies in the open corner at `a` between its neighbours `prev` and `next`, the outline running
/// counter-clockwise (O'Rourke's InCone).
fn in_cone(prev: [f64; 2], a: [f64; 2], next: [f64; 2], b: [f64; 2]) -> bool {
    let left = |p: [f64; 2], q: [f64; 2], r: [f64; 2]| cross_2d(p, q, r) > 0.0;
    let left_on = |p: [f64; 2], q: [f64; 2], r: [f64; 2]| cross_2d(p, q, r) >= 0.0;
    if left_on(a, next, prev) {
        left(a, b, prev) && left(b, a, next)
    } else {
        !(left_on(a, b, next) && left_on(b, a, prev))
    }
}

/// Ear clipping of `ring` (vertex numbers, a number may come twice along a bridge) with its points `pts`, wound as
/// `orientation` says. A point with the same number as a corner of the ear does not block it.
fn clip(ring: &[u32], pts: &[[f64; 2]], orientation: f64) -> Vec<[u32; 3]> {
    let n = ring.len();
    let mut idx: Vec<usize> = (0..n).collect();
    let mut out = Vec::with_capacity(n.saturating_sub(2));
    let mut guard = 0;
    while idx.len() > 3 && guard < n * n {
        guard += 1;
        let m = idx.len();
        let ear = (0..m).find(|&i| {
            let (a, b, c) = (idx[(i + m - 1) % m], idx[i], idx[(i + 1) % m]);
            let turn = cross_2d(pts[a], pts[b], pts[c]) * orientation;
            if turn <= 0.0 {
                return false;
            }
            let corner = |p: usize| ring[p] == ring[a] || ring[p] == ring[b] || ring[p] == ring[c];
            !idx.iter().any(|&p| !corner(p) && in_triangle(pts[p], pts[a], pts[b], pts[c]))
        });
        match ear {
            Some(i) => {
                let (a, b, c) = (idx[(i + m - 1) % m], idx[i], idx[(i + 1) % m]);
                out.push([ring[a], ring[b], ring[c]]);
                idx.remove(i);
            }
            None => {
                // a bridge's two sides meet with no area between them: a corner on a straight line goes without a triangle
                match (0..m).find(|&i| cross_2d(pts[idx[(i + m - 1) % m]], pts[idx[i]], pts[idx[(i + 1) % m]]).abs() < 1e-18) {
                    Some(i) => {
                        idx.remove(i);
                    }
                    None => break,
                }
            }
        }
    }
    for i in 1..idx.len().saturating_sub(1) {
        let t = [ring[idx[0]], ring[idx[i]], ring[idx[i + 1]]];
        if t[0] != t[1] && t[1] != t[2] && t[0] != t[2] {
            out.push(t);
        }
    }
    out
}

fn newell(positions: &[[f64; 3]], ring: &[u32]) -> [f64; 3] {
    let mut n = [0.0f64; 3];
    for i in 0..ring.len() {
        let a = positions[ring[i] as usize];
        let b = positions[ring[(i + 1) % ring.len()] as usize];
        n[0] += (a[1] - b[1]) * (a[2] + b[2]);
        n[1] += (a[2] - b[2]) * (a[0] + b[0]);
        n[2] += (a[0] - b[0]) * (a[1] + b[1]);
    }
    n
}

fn dominant_axis(n: [f64; 3]) -> usize {
    let a = n.map(f64::abs);
    if a[0] >= a[1] && a[0] >= a[2] {
        0
    } else if a[1] >= a[2] {
        1
    } else {
        2
    }
}

fn signed_area_2d(pts: &[[f64; 2]]) -> f64 {
    let mut s = 0.0;
    for i in 0..pts.len() {
        let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
        s += a[0] * b[1] - b[0] * a[1];
    }
    s * 0.5
}

fn cross_2d(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

/// Inclusive point-in-triangle test, independent of the triangle's winding.
fn in_triangle(p: [f64; 2], a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> bool {
    let d1 = cross_2d(a, b, p);
    let d2 = cross_2d(b, c, p);
    let d3 = cross_2d(c, a, p);
    let neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(neg && pos)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area_3d(positions: &[[f64; 3]], tris: &[[u32; 3]]) -> f64 {
        tris.iter()
            .map(|t| {
                let [a, b, c] = t.map(|v| positions[v as usize]);
                let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
                let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
                let n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
                (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt() * 0.5
            })
            .sum()
    }

    /// L-shape: a 2 x 2 square with the top-right 1 x 1 quarter removed, area 3.
    fn l_shape() -> (Vec<[f64; 3]>, Vec<u32>) {
        let p = vec![
            [0.0, 0.0, 0.0],
            [2.0, 0.0, 0.0],
            [2.0, 1.0, 0.0],
            [1.0, 1.0, 0.0],
            [1.0, 2.0, 0.0],
            [0.0, 2.0, 0.0],
        ];
        // Start at (1, 2): a fan from there would spill into the missing quarter.
        (p, vec![4, 5, 0, 1, 2, 3])
    }

    #[test]
    fn concave_l_shape_covers_its_area_exactly() {
        let (p, ring) = l_shape();
        let tris = triangulate(&p, &ring);
        assert_eq!(tris.len(), 4);
        assert!((area_3d(&p, &tris) - 3.0).abs() < 1e-5);
        // Every triangle keeps the ring's counter-clockwise (+Z) winding.
        for t in &tris {
            let [a, b, c] = t.map(|v| p[v as usize]);
            assert!(cross_2d([a[0], a[1]], [b[0], b[1]], [c[0], c[1]]) > 0.0);
        }
    }

    #[test]
    fn works_on_vertical_and_clockwise_projected_faces() {
        // The same L standing in the XZ plane, wound so its normal points -Y.
        let (p, ring) = l_shape();
        let p: Vec<[f64; 3]> = p.iter().map(|q| [q[0], 0.0, q[1]]).collect();
        let tris = triangulate(&p, &ring);
        assert!((area_3d(&p, &tris) - 3.0).abs() < 1e-5);
    }

    #[test]
    fn collinear_points_still_give_n_minus_2_triangles() {
        let p = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0], [2.0, 1.0, 0.0], [0.0, 1.0, 0.0]];
        let ring: Vec<u32> = (0..5).collect();
        let tris = triangulate(&p, &ring);
        assert_eq!(tris.len(), 3);
        assert!((area_3d(&p, &tris) - 2.0).abs() < 1e-5);
    }

    #[test]
    fn a_square_with_a_square_hole_covers_the_ring_only() {
        // 4 x 4 square, a 2 x 2 hole in the middle: 16 - 4 = 12 square units
        let positions = vec![
            [0.0, 0.0, 0.0], [4.0, 0.0, 0.0], [4.0, 4.0, 0.0], [0.0, 4.0, 0.0],
            [1.0, 1.0, 0.0], [3.0, 1.0, 0.0], [3.0, 3.0, 0.0], [1.0, 3.0, 0.0],
        ];
        for hole in [vec![4, 7, 6, 5], vec![4, 5, 6, 7]] {
            let tris = triangulate_with_holes(&positions, &[0, 1, 2, 3], &[hole]);
            assert!((area_3d(&positions, &tris) - 12.0).abs() < 1e-9, "area {}", area_3d(&positions, &tris));
            for t in &tris {
                let c = [0, 1, 2].map(|k| t.iter().map(|&v| positions[v as usize][k]).sum::<f64>() / 3.0);
                assert!(!(c[0] > 1.0 && c[0] < 3.0 && c[1] > 1.0 && c[1] < 3.0), "no triangle in the hole: {t:?}");
            }
        }
    }

    #[test]
    fn two_holes_side_by_side() {
        // 6 x 2 wall, two 1 x 1 windows: 12 - 2 = 10
        let positions = vec![
            [0.0, 0.0, 0.0], [6.0, 0.0, 0.0], [6.0, 2.0, 0.0], [0.0, 2.0, 0.0],
            [1.0, 0.5, 0.0], [2.0, 0.5, 0.0], [2.0, 1.5, 0.0], [1.0, 1.5, 0.0],
            [4.0, 0.5, 0.0], [5.0, 0.5, 0.0], [5.0, 1.5, 0.0], [4.0, 1.5, 0.0],
        ];
        let tris = triangulate_with_holes(&positions, &[0, 1, 2, 3], &[vec![4, 7, 6, 5], vec![8, 11, 10, 9]]);
        assert!((area_3d(&positions, &tris) - 10.0).abs() < 1e-9, "area {}", area_3d(&positions, &tris));
    }
}
