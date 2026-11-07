//! The element: an 8-node brick (trilinear) with Wilson's incompatible modes.
//!
//! A plain 8-node brick is far too stiff in bending (it cannot curve, so it shears instead): a beam a few bricks thick bends
//! half as much as it should. Three extra "bubble" shapes per direction (1 - ξ², 1 - η², 1 - ζ²) let each brick curve; they
//! belong to the brick alone and are solved out of its stiffness before assembly (static condensation), so the grid still
//! has three unknowns per corner. For bricks of a regular grid (cubes) the extra shapes pass the patch test as they are.
//!
//! Every voxel is the same cube, so the 24x24 matrix is worked out once, for a cube of side 1 and E = 1; a cube of side h and
//! modulus E has `E * h` times it.

use crate::material::Material;

/// The corners in the order used everywhere: bottom face (z = 0) anticlockwise from the origin, then the top face.
pub const CORNERS: [[usize; 3]; 8] = [[0, 0, 0], [1, 0, 0], [1, 1, 0], [0, 1, 0], [0, 0, 1], [1, 0, 1], [1, 1, 1], [0, 1, 1]];

fn sign(c: usize) -> f64 {
    if c == 0 { -1.0 } else { 1.0 }
}

/// Gradients (x, y, z) of the eight shape functions at a point of the unit cube's parent element (ξ, η, ζ in -1..1), for a
/// cube of side 1 (d/dx = 2 d/dξ).
fn shape_gradients(p: [f64; 3]) -> [[f64; 3]; 8] {
    let mut g = [[0.0; 3]; 8];
    for (i, c) in CORNERS.iter().enumerate() {
        let s = [sign(c[0]), sign(c[1]), sign(c[2])];
        let f = [1.0 + s[0] * p[0], 1.0 + s[1] * p[1], 1.0 + s[2] * p[2]];
        g[i] = [2.0 * s[0] * f[1] * f[2] / 8.0, 2.0 * s[1] * f[0] * f[2] / 8.0, 2.0 * s[2] * f[0] * f[1] / 8.0];
    }
    g
}

/// Gradients of the three bubble shapes 1 - ξ², 1 - η², 1 - ζ² (each changes along its own direction only).
fn bubble_gradients(p: [f64; 3]) -> [[f64; 3]; 3] {
    let mut g = [[0.0; 3]; 3];
    for m in 0..3 {
        g[m][m] = 2.0 * -2.0 * p[m];
    }
    g
}

/// The strain matrix (6 rows: xx, yy, zz, xy, yz, zx) for shapes with the given gradients, 3 columns per shape.
fn strain_matrix(grads: &[[f64; 3]]) -> Vec<[f64; 6]> {
    // stored by column: one [f64; 6] per unknown
    let mut cols = Vec::with_capacity(grads.len() * 3);
    for g in grads {
        cols.push([g[0], 0.0, 0.0, g[1], 0.0, g[2]]);
        cols.push([0.0, g[1], 0.0, g[0], g[2], 0.0]);
        cols.push([0.0, 0.0, g[2], 0.0, g[1], g[0]]);
    }
    cols
}

fn btdb(a: &[[f64; 6]], b: &[[f64; 6]], d: &[[f64; 6]; 6], w: f64, out: &mut [Vec<f64>]) {
    for (i, ca) in a.iter().enumerate() {
        let mut da = [0.0; 6];
        for r in 0..6 {
            for k in 0..6 {
                da[r] += d[r][k] * ca[k];
            }
        }
        for (j, cb) in b.iter().enumerate() {
            let mut s = 0.0;
            for r in 0..6 {
                s += da[r] * cb[r];
            }
            out[i][j] += s * w;
        }
    }
}

/// Solves `a x = b` for several right-hand sides (columns of `b`), in place; `a` is small and symmetric positive definite.
fn solve_dense(mut a: Vec<Vec<f64>>, mut b: Vec<Vec<f64>>) -> Vec<Vec<f64>> {
    let n = a.len();
    for k in 0..n {
        let p = (k..n).max_by(|&i, &j| a[i][k].abs().total_cmp(&a[j][k].abs())).unwrap();
        a.swap(k, p);
        b.swap(k, p);
        for i in k + 1..n {
            let f = a[i][k] / a[k][k];
            if f == 0.0 {
                continue;
            }
            for j in k..n {
                a[i][j] -= f * a[k][j];
            }
            for j in 0..b[i].len() {
                b[i][j] -= f * b[k][j];
            }
        }
    }
    for k in (0..n).rev() {
        for j in 0..b[k].len() {
            let mut s = b[k][j];
            for i in k + 1..n {
                s -= a[k][i] * b[i][j];
            }
            b[k][j] = s / a[k][k];
        }
    }
    b
}

/// The cube element for one Poisson's ratio: its stiffness and how its bubble shapes follow from the corner displacements.
pub struct Element {
    /// 24x24 stiffness of a cube of side 1 with E = 1, unknowns ordered corner by corner (x, y, z each).
    pub stiffness: [[f64; 24]; 24],
    /// The bubble amplitudes (9) from the corner displacements (24): -Kaa^-1 Kau. The same for every cube size and E.
    pub bubbles: [[f64; 24]; 9],
}

impl Element {
    pub fn new(poissons_ratio: f64) -> Element {
        let unit = Material { poissons_ratio, youngs_modulus: 1.0, ..Material::new("", 1.0, poissons_ratio, 1.0, 1.0) };
        let d = unit.elasticity();
        let g = 1.0 / 3f64.sqrt();
        let mut kuu = vec![vec![0.0; 24]; 24];
        let mut kua = vec![vec![0.0; 9]; 24];
        let mut kaa = vec![vec![0.0; 9]; 9];
        // detJ for a cube of side 1 mapped from -1..1: (1/2)^3; Gauss weights are 1
        let w = 0.125;
        for &x in &[-g, g] {
            for &y in &[-g, g] {
                for &z in &[-g, g] {
                    let p = [x, y, z];
                    let bu = strain_matrix(&shape_gradients(p));
                    let ba = strain_matrix(&bubble_gradients(p));
                    btdb(&bu, &bu, &d, w, &mut kuu);
                    btdb(&bu, &ba, &d, w, &mut kua);
                    btdb(&ba, &ba, &d, w, &mut kaa);
                }
            }
        }
        // K = Kuu - Kua Kaa^-1 Kau
        let kau: Vec<Vec<f64>> = (0..9).map(|i| (0..24).map(|j| kua[j][i]).collect()).collect();
        let x = solve_dense(kaa, kau); // 9 x 24: Kaa^-1 Kau
        let mut k = [[0.0; 24]; 24];
        for i in 0..24 {
            for j in 0..24 {
                let mut s = kuu[i][j];
                for m in 0..9 {
                    s -= kua[i][m] * x[m][j];
                }
                k[i][j] = s;
            }
        }
        // exact symmetry (rounding)
        for i in 0..24 {
            for j in i + 1..24 {
                let m = 0.5 * (k[i][j] + k[j][i]);
                k[i][j] = m;
                k[j][i] = m;
            }
        }
        let bubbles = std::array::from_fn(|m| std::array::from_fn(|j| -x[m][j]));
        Element { stiffness: k, bubbles }
    }

    /// The strain at each of the cube's corners (hex8 order) from its corner displacements, bubbles included, for a cube of
    /// side `h`. At the corners a bent cube shows its outer-fibre strain; at its centre it shows the average.
    pub fn corner_strains(&self, u: &[[f64; 3]; 8], h: f64) -> [[f64; 6]; 8] {
        let flat: [f64; 24] = std::array::from_fn(|i| u[i / 3][i % 3]);
        let alpha: [f64; 9] = std::array::from_fn(|m| (0..24).map(|j| self.bubbles[m][j] * flat[j]).sum());
        std::array::from_fn(|corner| {
            let c = CORNERS[corner];
            let p = [sign(c[0]), sign(c[1]), sign(c[2])];
            let bu = strain_matrix(&shape_gradients(p));
            let ba = strain_matrix(&bubble_gradients(p));
            let mut e = [0.0; 6];
            for (col, v) in bu.iter().zip(flat.iter()).chain(ba.iter().zip(alpha.iter())) {
                for r in 0..6 {
                    e[r] += col[r] * v / h;
                }
            }
            e
        })
    }
}

/// The 24x24 stiffness of a cube of side 1 with E = 1 (see `Element`).
pub fn unit_stiffness(poissons_ratio: f64) -> [[f64; 24]; 24] {
    Element::new(poissons_ratio).stiffness
}


/// Von Mises stress from a stress vector (xx, yy, zz, xy, yz, zx).
pub fn von_mises(s: &[f64; 6]) -> f64 {
    let (a, b, c) = (s[0] - s[1], s[1] - s[2], s[2] - s[0]);
    (0.5 * (a * a + b * b + c * c) + 3.0 * (s[3] * s[3] + s[4] * s[4] + s[5] * s[5])).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rigid_motion_costs_nothing_and_the_matrix_is_positive() {
        let k = unit_stiffness(0.3);
        // a translation and a small rotation about z: no force at any corner
        for motion in [|_: [usize; 3]| [1.0, 0.0, 0.0], |c: [usize; 3]| [-(c[1] as f64), c[0] as f64, 0.0]] {
            let u: Vec<f64> = CORNERS.iter().flat_map(|&c| motion(c)).collect();
            for i in 0..24 {
                let f: f64 = (0..24).map(|j| k[i][j] * u[j]).sum();
                assert!(f.abs() < 1e-12, "row {i}: {f}");
            }
        }
        for i in 0..24 {
            assert!(k[i][i] > 0.0);
        }
    }

    #[test]
    fn a_stretched_cube_has_the_stress_of_the_material() {
        // every corner moved by x * 0.001: strain xx = 0.001 everywhere
        let u: [[f64; 3]; 8] = std::array::from_fn(|i| [CORNERS[i][0] as f64 * 0.001, 0.0, 0.0]);
        let el = Element::new(0.3);
        for e in el.corner_strains(&u, 1.0) {
            assert!((e[0] - 0.001).abs() < 1e-15 && e[1].abs() < 1e-15 && e[3].abs() < 1e-15, "the same at every corner");
        }
        assert!((von_mises(&[100.0, 0.0, 0.0, 0.0, 0.0, 0.0]) - 100.0).abs() < 1e-12);
        assert!((von_mises(&[0.0, 0.0, 0.0, 10.0, 0.0, 0.0]) - 10.0 * 3f64.sqrt()).abs() < 1e-12);
    }
}
