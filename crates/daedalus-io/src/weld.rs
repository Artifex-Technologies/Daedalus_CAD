use daedalus_core::Mesh;
use std::collections::HashMap;

/// Welds vertices: bit-identical ones (`default`), or (`within`) any closer than a tolerance, found through a grid of
/// tolerance-sized cells: a new point is compared with the points of the 27 cells around its own, and the first one within
/// the tolerance is used as it is (points never move).
#[derive(Default)]
pub(crate) struct Welder {
    tol: f64,
    exact: HashMap<[u64; 3], u32>,
    grid: HashMap<[i64; 3], Vec<u32>>,
}

impl Welder {
    pub(crate) fn within(tol: f64) -> Welder {
        Welder { tol: if tol.is_finite() && tol > 0.0 { tol } else { 0.0 }, ..Welder::default() }
    }

    pub(crate) fn vertex(&mut self, mesh: &mut Mesh, p: [f64; 3]) -> u32 {
        if self.tol == 0.0 || !p.iter().all(|c| c.is_finite()) {
            let key = [p[0].to_bits(), p[1].to_bits(), p[2].to_bits()];
            return *self.exact.entry(key).or_insert_with(|| mesh.add_vertex(p));
        }
        let cell = |x: f64| (x / self.tol).floor() as i64;
        let c = [cell(p[0]), cell(p[1]), cell(p[2])];
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    for &i in self.grid.get(&[c[0] + dx, c[1] + dy, c[2] + dz]).into_iter().flatten() {
                        let q = mesh.positions[i as usize];
                        let d2 = (p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2) + (p[2] - q[2]).powi(2);
                        if d2 <= self.tol * self.tol {
                            return i;
                        }
                    }
                }
            }
        }
        let i = mesh.add_vertex(p);
        self.grid.entry(c).or_default().push(i);
        i
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn points_within_the_tolerance_are_one_vertex() {
        let mut m = Mesh::new();
        let mut w = Welder::within(1e-6);
        let a = w.vertex(&mut m, [1.0, 2.0, 3.0]);
        // across a cell boundary and within the tolerance
        assert_eq!(w.vertex(&mut m, [1.0 + 4e-7, 2.0 - 4e-7, 3.0]), a);
        assert_ne!(w.vertex(&mut m, [1.0 + 2e-6, 2.0, 3.0]), a);
        // exact welding by default
        let mut e = Welder::default();
        let b = e.vertex(&mut m, [5.0, 5.0, 5.0]);
        assert_ne!(e.vertex(&mut m, [5.0 + 1e-12, 5.0, 5.0]), b);
        assert_eq!(e.vertex(&mut m, [5.0, 5.0, 5.0]), b);
    }
}
