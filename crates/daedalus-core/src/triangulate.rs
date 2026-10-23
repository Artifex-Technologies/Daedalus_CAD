//! Ear-clipping triangulation of planar polygons, so concave faces (an L-shaped
//! room drawn with the Line tool) render, pick and export correctly.

/// Triangulates one planar face ring (vertex indices into `positions`) by ear
/// clipping. Triangles keep the ring's winding. Degenerate input falls back to
/// a fan so that every face still produces `ring.len() - 2` triangles.
pub fn triangulate(positions: &[[f32; 3]], ring: &[u32]) -> Vec<[u32; 3]> {
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
    let pts: Vec<[f32; 2]> = ring
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

fn newell(positions: &[[f32; 3]], ring: &[u32]) -> [f32; 3] {
    let mut n = [0.0f32; 3];
    for i in 0..ring.len() {
        let a = positions[ring[i] as usize];
        let b = positions[ring[(i + 1) % ring.len()] as usize];
        n[0] += (a[1] - b[1]) * (a[2] + b[2]);
        n[1] += (a[2] - b[2]) * (a[0] + b[0]);
        n[2] += (a[0] - b[0]) * (a[1] + b[1]);
    }
    n
}

fn dominant_axis(n: [f32; 3]) -> usize {
    let a = n.map(f32::abs);
    if a[0] >= a[1] && a[0] >= a[2] {
        0
    } else if a[1] >= a[2] {
        1
    } else {
        2
    }
}

fn signed_area_2d(pts: &[[f32; 2]]) -> f32 {
    let mut s = 0.0;
    for i in 0..pts.len() {
        let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
        s += a[0] * b[1] - b[0] * a[1];
    }
    s * 0.5
}

fn cross_2d(a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> f32 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

/// Inclusive point-in-triangle test, independent of the triangle's winding.
fn in_triangle(p: [f32; 2], a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> bool {
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

    fn area_3d(positions: &[[f32; 3]], tris: &[[u32; 3]]) -> f32 {
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
    fn l_shape() -> (Vec<[f32; 3]>, Vec<u32>) {
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
        let p: Vec<[f32; 3]> = p.iter().map(|q| [q[0], 0.0, q[1]]).collect();
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
}
