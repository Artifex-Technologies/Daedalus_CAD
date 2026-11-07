//! A closed triangle mesh turned into cubes (voxels) of one size: a cube is solid when its centre is inside the mesh.
//!
//! Inside is found per row of cubes along x: a ray through the row's centres crosses the surface an even number of times, and
//! the cubes between the first and second crossing, third and fourth, ... are inside. The ray is moved a hair off the row so it
//! does not run exactly through the mesh's edges and corners.

pub type Tri = [[f64; 3]; 3];

#[derive(Clone, Debug)]
pub struct Grid {
    /// The corner of cube (0, 0, 0) with the smallest coordinates.
    pub origin: [f64; 3],
    /// The side of every cube, m.
    pub h: f64,
    /// Cubes along x, y, z.
    pub n: [usize; 3],
    /// Per cube (x fastest, then y, then z): inside the part.
    pub solid: Vec<bool>,
    /// Rows whose ray crossed the surface an odd number of times (the mesh is not closed there).
    pub odd_rows: usize,
}

impl Grid {
    pub fn cube(&self, i: usize, j: usize, k: usize) -> usize {
        i + self.n[0] * (j + self.n[1] * k)
    }

    pub fn cube_ijk(&self, c: usize) -> [usize; 3] {
        [c % self.n[0], (c / self.n[0]) % self.n[1], c / (self.n[0] * self.n[1])]
    }

    /// Whether the cube at (i, j, k) is solid; outside the grid nothing is.
    pub fn solid_at(&self, i: isize, j: isize, k: isize) -> bool {
        if i < 0 || j < 0 || k < 0 || i as usize >= self.n[0] || j as usize >= self.n[1] || k as usize >= self.n[2] {
            return false;
        }
        self.solid[self.cube(i as usize, j as usize, k as usize)]
    }

    /// Corner points (nodes) of the grid along each axis: one more than cubes.
    pub fn nodes(&self) -> [usize; 3] {
        [self.n[0] + 1, self.n[1] + 1, self.n[2] + 1]
    }

    pub fn node(&self, i: usize, j: usize, k: usize) -> usize {
        let m = self.nodes();
        i + m[0] * (j + m[1] * k)
    }

    pub fn node_ijk(&self, n: usize) -> [usize; 3] {
        let m = self.nodes();
        [n % m[0], (n / m[0]) % m[1], n / (m[0] * m[1])]
    }

    pub fn node_position(&self, n: usize) -> [f64; 3] {
        let [i, j, k] = self.node_ijk(n);
        [self.origin[0] + i as f64 * self.h, self.origin[1] + j as f64 * self.h, self.origin[2] + k as f64 * self.h]
    }

    pub fn cube_centre(&self, c: usize) -> [f64; 3] {
        let [i, j, k] = self.cube_ijk(c);
        [
            self.origin[0] + (i as f64 + 0.5) * self.h,
            self.origin[1] + (j as f64 + 0.5) * self.h,
            self.origin[2] + (k as f64 + 0.5) * self.h,
        ]
    }

    /// The eight corner nodes of a cube, in hex8's corner order.
    pub fn cube_nodes(&self, c: usize) -> [usize; 8] {
        let [i, j, k] = self.cube_ijk(c);
        crate::hex8::CORNERS.map(|d| self.node(i + d[0], j + d[1], k + d[2]))
    }

    pub fn solid_count(&self) -> usize {
        self.solid.iter().filter(|s| **s).count()
    }

    pub fn volume(&self) -> f64 {
        self.solid_count() as f64 * self.h.powi(3)
    }
}

pub fn bounds(tris: &[Tri]) -> ([f64; 3], [f64; 3]) {
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for t in tris {
        for p in t {
            for a in 0..3 {
                lo[a] = lo[a].min(p[a]);
                hi[a] = hi[a].max(p[a]);
            }
        }
    }
    (lo, hi)
}

/// The volume a closed mesh encloses (signed: positive when its triangles face outward).
pub fn mesh_volume(tris: &[Tri]) -> f64 {
    tris.iter()
        .map(|[a, b, c]| {
            (a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0]) + a[2] * (b[0] * c[1] - b[1] * c[0])) / 6.0
        })
        .sum()
}

/// Cubes of side `h` filling the mesh. The grid is centred on the mesh's bounds.
pub fn voxelize(tris: &[Tri], h: f64) -> Grid {
    let (lo, hi) = bounds(tris);
    let mut n = [0usize; 3];
    let mut origin = [0.0; 3];
    for a in 0..3 {
        let size = (hi[a] - lo[a]).max(0.0);
        n[a] = ((size / h).ceil() as usize).max(1);
        origin[a] = lo[a] - (n[a] as f64 * h - size) / 2.0;
    }
    let mut grid = Grid { origin, h, n, solid: vec![false; n[0] * n[1] * n[2]], odd_rows: 0 };
    // triangles by the rows (j, k) their y-z bounds cover
    let mut rows: Vec<Vec<usize>> = vec![Vec::new(); n[1] * n[2]];
    let row_of = |v: f64, a: usize| ((v - origin[a]) / h - 0.5).floor();
    for (t, tri) in tris.iter().enumerate() {
        let (ylo, yhi) = (tri.iter().map(|p| p[1]).fold(f64::INFINITY, f64::min), tri.iter().map(|p| p[1]).fold(f64::NEG_INFINITY, f64::max));
        let (zlo, zhi) = (tri.iter().map(|p| p[2]).fold(f64::INFINITY, f64::min), tri.iter().map(|p| p[2]).fold(f64::NEG_INFINITY, f64::max));
        let j0 = (row_of(ylo, 1).max(-1.0) + 1.0) as usize;
        let j1 = (row_of(yhi, 1) as isize).min(n[1] as isize - 1);
        let k0 = (row_of(zlo, 2).max(-1.0) + 1.0) as usize;
        let k1 = (row_of(zhi, 2) as isize).min(n[2] as isize - 1);
        if j1 < 0 || k1 < 0 {
            continue;
        }
        for k in k0..=k1 as usize {
            for j in j0..=j1 as usize {
                rows[j + n[1] * k].push(t);
            }
        }
    }
    // the ray a hair off the row's centre line, by amounts unlikely to line up with a model's edges
    let (dy, dz) = (h * 1.234_567e-5, h * 2.718_281e-5);
    let mut xs: Vec<f64> = Vec::new();
    for k in 0..n[2] {
        for j in 0..n[1] {
            let y = origin[1] + (j as f64 + 0.5) * h + dy;
            let z = origin[2] + (k as f64 + 0.5) * h + dz;
            xs.clear();
            for &t in &rows[j + n[1] * k] {
                if let Some(x) = ray_x(&tris[t], y, z) {
                    xs.push(x);
                }
            }
            if xs.len() % 2 == 1 {
                grid.odd_rows += 1;
                xs.pop();
            }
            xs.sort_by(f64::total_cmp);
            for pair in xs.chunks(2) {
                let (a, b) = (pair[0], pair[1]);
                // cubes whose centre lies between the two crossings
                let i0 = ((a - origin[0]) / h - 0.5).ceil().max(0.0) as usize;
                let i1 = ((b - origin[0]) / h - 0.5).floor();
                if i1 < 0.0 {
                    continue;
                }
                for i in i0..=(i1 as usize).min(n[0] - 1) {
                    let c = grid.cube(i, j, k);
                    grid.solid[c] = true;
                }
            }
        }
    }
    grid
}

/// Where the line (t, y, z) crosses the triangle, if it does: its x.
fn ray_x(t: &Tri, y: f64, z: f64) -> Option<f64> {
    let [a, b, c] = t;
    // barycentric coordinates in the y-z plane
    let d = (b[1] - a[1]) * (c[2] - a[2]) - (c[1] - a[1]) * (b[2] - a[2]);
    if d.abs() < 1e-30 {
        return None; // edge-on to the ray
    }
    let u = ((y - a[1]) * (c[2] - a[2]) - (c[1] - a[1]) * (z - a[2])) / d;
    let v = ((b[1] - a[1]) * (z - a[2]) - (y - a[1]) * (b[2] - a[2])) / d;
    if u < 0.0 || v < 0.0 || u + v > 1.0 {
        return None;
    }
    Some(a[0] + u * (b[0] - a[0]) + v * (c[0] - a[0]))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A closed box as 12 outward-facing triangles; the face number of each pair is 0..6 = -x, +x, -y, +y, -z, +z.
    pub fn boxed(lo: [f64; 3], hi: [f64; 3]) -> (Vec<Tri>, Vec<usize>) {
        let p = |i: usize| [if i & 1 == 0 { lo[0] } else { hi[0] }, if i & 2 == 0 { lo[1] } else { hi[1] }, if i & 4 == 0 { lo[2] } else { hi[2] }];
        // quads wound outward
        let quads = [[0, 4, 6, 2], [1, 3, 7, 5], [0, 1, 5, 4], [2, 6, 7, 3], [0, 2, 3, 1], [4, 5, 7, 6]];
        let mut tris = Vec::new();
        let mut faces = Vec::new();
        for (f, q) in quads.iter().enumerate() {
            tris.push([p(q[0]), p(q[1]), p(q[2])]);
            tris.push([p(q[0]), p(q[2]), p(q[3])]);
            faces.extend([f, f]);
        }
        (tris, faces)
    }

    #[test]
    fn a_box_fills_its_cubes_exactly() {
        let (tris, _) = boxed([0.0, 0.0, 0.0], [0.1, 0.01, 0.02]);
        assert!((mesh_volume(&tris) - 0.1 * 0.01 * 0.02).abs() < 1e-15, "outward triangles");
        let g = voxelize(&tris, 0.0025);
        assert_eq!(g.n, [40, 4, 8]);
        assert_eq!(g.solid_count(), 40 * 4 * 8);
        assert_eq!(g.odd_rows, 0);
    }

    #[test]
    fn an_l_shape_leaves_its_corner_empty() {
        // two boxes meeting in an L: 20 x 10 and 10 x 10 (x by y), 10 thick; as one closed surface they would share a wall,
        // so test with the two boxes' union volume from separate meshes laid side by side
        let (mut tris, _) = boxed([0.0, 0.0, 0.0], [0.02, 0.01, 0.01]);
        let (more, _) = boxed([0.0, 0.01, 0.0], [0.01, 0.02, 0.01]);
        tris.extend(more);
        let g = voxelize(&tris, 0.001);
        assert_eq!(g.n, [20, 20, 10]);
        // the shared wall at y = 0.01 is crossed twice (leaving one box, entering the other): both stay solid
        assert_eq!(g.solid_count(), (200 + 100) * 10);
        assert!(!g.solid[g.cube(15, 15, 5)], "the L's missing corner");
    }
}
