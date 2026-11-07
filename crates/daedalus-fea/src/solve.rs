//! Assembling and solving K u = f on the voxel grid without storing K.
//!
//! Every cube has the same 24x24 stiffness, so K times a vector is worked out corner by corner: a node's force is the sum,
//! over the (up to eight) solid cubes around it, of that cube's rows for the node times the cube's corner displacements. Each
//! node writes only its own three numbers, so the nodes are shared out between threads with nothing to lock. The system is
//! solved by conjugate gradients with the diagonal as preconditioner; supported nodes are held at zero.

use crate::hex8;
use crate::voxel::Grid;

pub struct System<'a> {
    pub grid: &'a Grid,
    /// E * h times the unit cube's matrix.
    ke: [[f64; 24]; 24],
    /// For every grid node: its number among the nodes that touch a solid cube, or `NONE`.
    pub index: Vec<u32>,
    /// The grid number of each such node.
    pub nodes: Vec<usize>,
    /// Per unknown (3 per node): held at zero.
    pub fixed: Vec<bool>,
    diag: Vec<f64>,
}

pub const NONE: u32 = u32::MAX;

pub struct Solved {
    /// Per node of `System::nodes`: its displacement, m.
    pub displacement: Vec<[f64; 3]>,
    pub iterations: usize,
    /// |f - K u| / |f| at the end.
    pub residual: f64,
    pub converged: bool,
}

fn threads() -> usize {
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1).min(32)
}

impl<'a> System<'a> {
    pub fn new(grid: &'a Grid, youngs_modulus: f64, poissons_ratio: f64) -> System<'a> {
        let unit = hex8::unit_stiffness(poissons_ratio);
        let s = youngs_modulus * grid.h;
        let ke = unit.map(|row| row.map(|v| v * s));
        let total = grid.nodes().iter().product::<usize>();
        let mut index = vec![NONE; total];
        let mut nodes = Vec::new();
        for c in 0..grid.solid.len() {
            if grid.solid[c] {
                for n in grid.cube_nodes(c) {
                    if index[n] == NONE {
                        index[n] = 0; // numbered below, in grid order
                    }
                }
            }
        }
        for n in 0..total {
            if index[n] != NONE {
                index[n] = nodes.len() as u32;
                nodes.push(n);
            }
        }
        let mut sys = System { grid, ke, index, fixed: vec![false; nodes.len() * 3], diag: vec![0.0; nodes.len() * 3], nodes };
        for c in 0..grid.solid.len() {
            if grid.solid[c] {
                for (l, n) in grid.cube_nodes(c).iter().enumerate() {
                    let at = sys.index[*n] as usize * 3;
                    for d in 0..3 {
                        sys.diag[at + d] += sys.ke[3 * l + d][3 * l + d];
                    }
                }
            }
        }
        sys
    }

    pub fn unknowns(&self) -> usize {
        self.nodes.len() * 3
    }

    /// The cubes around grid node (i, j, k) that are solid, with the node's corner number in each.
    fn cubes_around(&self, n: usize, out: &mut Vec<(usize, usize)>) {
        out.clear();
        let [i, j, k] = self.grid.node_ijk(n);
        for (l, c) in hex8::CORNERS.iter().enumerate() {
            // the node is corner l of the cube whose low corner is node - c
            let (ci, cj, ck) = (i as isize - c[0] as isize, j as isize - c[1] as isize, k as isize - c[2] as isize);
            if self.grid.solid_at(ci, cj, ck) {
                out.push((self.grid.cube(ci as usize, cj as usize, ck as usize), l));
            }
        }
    }

    /// y = K x (x and y per unknown).
    pub fn apply(&self, x: &[f64], y: &mut [f64]) {
        let per = self.nodes.len().div_ceil(threads()).max(256);
        std::thread::scope(|s| {
            for (chunk, out) in y.chunks_mut(per * 3).enumerate() {
                s.spawn(move || {
                    let mut around = Vec::with_capacity(8);
                    for (o, slot) in out.chunks_mut(3).enumerate() {
                        let me = chunk * per + o;
                        let mut f = [0.0; 3];
                        self.cubes_around(self.nodes[me], &mut around);
                        for &(c, l) in &around {
                            for (q, m) in self.grid.cube_nodes(c).iter().enumerate() {
                                let at = self.index[*m] as usize * 3;
                                let u = [x[at], x[at + 1], x[at + 2]];
                                for d in 0..3 {
                                    let row = &self.ke[3 * l + d];
                                    f[d] += row[3 * q] * u[0] + row[3 * q + 1] * u[1] + row[3 * q + 2] * u[2];
                                }
                            }
                        }
                        slot.copy_from_slice(&f);
                    }
                });
            }
        });
    }

    /// Solves K u = f with the fixed unknowns held at zero.
    pub fn solve(&self, f: &[f64], tolerance: f64, max_iterations: usize, progress: &mut dyn FnMut(usize, f64)) -> Solved {
        let n = self.unknowns();
        let mut u = vec![0.0; n];
        let mut r: Vec<f64> = f.iter().zip(&self.fixed).map(|(v, fx)| if *fx { 0.0 } else { *v }).collect();
        let norm_f = dot(&r, &r).sqrt();
        if norm_f == 0.0 {
            return Solved { displacement: vec![[0.0; 3]; self.nodes.len()], iterations: 0, residual: 0.0, converged: true };
        }
        let precond = |r: &[f64], z: &mut [f64]| {
            for i in 0..r.len() {
                z[i] = if self.fixed[i] || self.diag[i] == 0.0 { 0.0 } else { r[i] / self.diag[i] };
            }
        };
        let mut z = vec![0.0; n];
        precond(&r, &mut z);
        let mut p = z.clone();
        let mut ap = vec![0.0; n];
        let mut rz = dot(&r, &z);
        let mut residual = 1.0;
        let mut it = 0;
        while it < max_iterations {
            self.apply(&p, &mut ap);
            for i in 0..n {
                if self.fixed[i] {
                    ap[i] = 0.0;
                }
            }
            let pap = dot(&p, &ap);
            if !(pap > 0.0) {
                break; // not positive: the part is not held (it can move without straining)
            }
            let alpha = rz / pap;
            for i in 0..n {
                u[i] += alpha * p[i];
                r[i] -= alpha * ap[i];
            }
            it += 1;
            residual = dot(&r, &r).sqrt() / norm_f;
            if it % 25 == 0 {
                progress(it, residual);
            }
            if residual < tolerance {
                break;
            }
            precond(&r, &mut z);
            let rz_new = dot(&r, &z);
            let beta = rz_new / rz;
            rz = rz_new;
            for i in 0..n {
                p[i] = z[i] + beta * p[i];
            }
        }
        let displacement = u.chunks(3).map(|c| [c[0], c[1], c[2]]).collect();
        Solved { displacement, iterations: it, residual, converged: residual < tolerance }
    }

    /// The corner displacements of a cube.
    pub fn cube_displacements(&self, c: usize, u: &[[f64; 3]]) -> [[f64; 3]; 8] {
        self.grid.cube_nodes(c).map(|n| u[self.index[n] as usize])
    }
}

pub fn dot(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}
