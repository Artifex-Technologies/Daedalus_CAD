//! Flat outlines made into faces, and faces made into solids: what 3D Text is built from (and any other profile given as
//! closed outlines, such as an imported drawing later).
//!
//! Outlines may nest: a letter's outside, the holes in it (the inside of an O), an island in a hole. What lies inside an
//! odd number of outlines is solid; the rest is open.

use crate::sketch::Plane;
use crate::{HalfEdgeMesh, Mesh, TopologyError};

/// Whether `p` lies inside the closed outline `ring` (even-odd).
pub fn inside(ring: &[[f32; 2]], p: [f32; 2]) -> bool {
    let mut c = false;
    let n = ring.len();
    for i in 0..n {
        let (a, b) = (ring[i], ring[(i + 1) % n]);
        if (a[1] > p[1]) != (b[1] > p[1]) {
            let x = a[0] + (p[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]);
            if p[0] < x {
                c = !c;
            }
        }
    }
    c
}

/// Faces for the solid parts of `outlines` (closed, each listed once round, any winding), laid on `plane`, all facing the
/// plane's normal. A face with a hole is two faces joined by hidden seams, as the Line tool makes it.
pub fn faces(outlines: &[Vec<[f32; 2]>], plane: &Plane) -> Result<Mesh, TopologyError> {
    let rings: Vec<&Vec<[f32; 2]>> = outlines.iter().filter(|r| r.len() >= 3).collect();
    // how many other outlines hold each one: outsides first, then holes, then islands, so each is drawn into what holds it
    let depth = |r: &Vec<[f32; 2]>| rings.iter().filter(|o| !std::ptr::eq(**o, r) && inside(o, r[0])).count();
    let mut order: Vec<(usize, &Vec<[f32; 2]>)> = rings.iter().map(|r| (depth(r), *r)).collect();
    order.sort_by_key(|(d, _)| *d);
    let mut h = HalfEdgeMesh::default();
    for (_, r) in &order {
        for i in 0..r.len() {
            let (a, b) = (r[i], r[(i + 1) % r.len()]);
            h.add_line(plane.to_world(a[0], a[1]), plane.to_world(b[0], b[1]))?;
        }
    }
    // the faces inside an even number of outlines are holes: taken out
    let mut m = h.to_mesh();
    let open: Vec<usize> = (0..m.faces.len())
        .filter(|&f| {
            let tri = crate::triangulate::triangulate(&m.positions, &m.faces[f]);
            let Some(t) = tri.first() else { return false };
            let c = t.iter().fold([0.0f32; 3], |s, &v| {
                let p = m.positions[v as usize];
                [s[0] + p[0] / 3.0, s[1] + p[1] / 3.0, s[2] + p[2] / 3.0]
            });
            let q = plane.to_local(c);
            rings.iter().filter(|r| inside(r, q)).count() % 2 == 0
        })
        .collect();
    for &f in open.iter().rev() {
        m.remove_face(f);
    }
    m.lines.clear();
    // every face toward the plane's normal
    for f in &mut m.faces {
        let n = newell(&m.positions, f);
        if n[0] * plane.normal[0] + n[1] * plane.normal[1] + n[2] * plane.normal[2] < 0.0 {
            f.reverse();
        }
    }
    let edges: std::collections::HashSet<(u32, u32)> = m.edges().into_iter().collect();
    m.seams.retain(|&[a, b]| edges.contains(&(a.min(b), a.max(b))));
    m.compact();
    Ok(m)
}

fn newell(positions: &[[f32; 3]], ring: &[u32]) -> [f32; 3] {
    let mut n = [0.0f32; 3];
    for i in 0..ring.len() {
        let (p, q) = (positions[ring[i] as usize], positions[ring[(i + 1) % ring.len()] as usize]);
        n[0] += (p[1] - q[1]) * (p[2] + q[2]);
        n[1] += (p[2] - q[2]) * (p[0] + q[0]);
        n[2] += (p[0] - q[0]) * (p[1] + q[1]);
    }
    n
}

/// A flat set of faces made solid: the faces, a copy of them moved by `offset`, and walls round their outer edges. Walls
/// along a curve (bending less than `smooth` radians from the next) are joined by hidden seams, so a round letter shows
/// no stripes.
pub fn extrude(cap: &Mesh, offset: [f32; 3], smooth: f32) -> Mesh {
    let n = cap.positions.len() as u32;
    let mut m = Mesh { positions: cap.positions.clone(), ..Default::default() };
    m.positions.extend(cap.positions.iter().map(|p| [p[0] + offset[0], p[1] + offset[1], p[2] + offset[2]]));
    let up = cap.faces.first().map(|f| {
        let nn = newell(&cap.positions, f);
        nn[0] * offset[0] + nn[1] * offset[1] + nn[2] * offset[2] >= 0.0
    });
    let up = up.unwrap_or(true);
    for f in &cap.faces {
        let moved: Vec<u32> = f.iter().map(|v| v + n).collect();
        if up {
            m.faces.push(f.iter().rev().copied().collect());
            m.faces.push(moved);
        } else {
            m.faces.push(f.clone());
            m.faces.push(moved.into_iter().rev().collect());
        }
    }
    // the outline's edges: those only one face uses
    let directed: std::collections::HashSet<(u32, u32)> =
        cap.faces.iter().flat_map(|f| (0..f.len()).map(move |i| (f[i], f[(i + 1) % f.len()]))).collect();
    let border: Vec<(u32, u32)> = directed.iter().copied().filter(|&(a, b)| !directed.contains(&(b, a))).collect();
    let mut walls: Vec<(u32, u32)> = Vec::new();
    for &(a, b) in &border {
        if up {
            m.faces.push(vec![a, b, b + n, a + n]);
        } else {
            m.faces.push(vec![b, a, a + n, b + n]);
        }
        walls.push((a, b));
    }
    for s in &cap.seams {
        m.seams.push(*s);
        m.seams.push([s[0] + n, s[1] + n]);
    }
    // an upright edge between two walls that hardly bend is hidden
    let dir = |(a, b): (u32, u32)| {
        let (p, q) = (cap.positions[a as usize], cap.positions[b as usize]);
        let d = [q[0] - p[0], q[1] - p[1], q[2] - p[2]];
        let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().max(1e-12);
        d.map(|c| c / l)
    };
    let into: std::collections::HashMap<u32, (u32, u32)> = walls.iter().map(|&(a, b)| (b, (a, b))).collect();
    for &(a, b) in &walls {
        if let Some(&prev) = into.get(&a) {
            let (d0, d1) = (dir(prev), dir((a, b)));
            let cos = d0[0] * d1[0] + d0[1] * d1[1] + d0[2] * d1[2];
            if cos > smooth.cos() {
                m.seams.push([a, a + n]);
            }
        }
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(x: f32, y: f32, s: f32) -> Vec<[f32; 2]> {
        vec![[x, y], [x + s, y], [x + s, y + s], [x, y + s]]
    }

    #[test]
    fn a_square_with_a_hole_and_an_island() {
        let outlines = vec![square(0.0, 0.0, 9.0), square(2.0, 2.0, 5.0), square(4.0, 4.0, 1.0)];
        let cap = faces(&outlines, &Plane::ground()).unwrap();
        let area: f32 = cap
            .faces
            .iter()
            .map(|f| {
                let n = newell(&cap.positions, f);
                assert!(n[2] > 0.0, "every face looks up");
                n[2] / 2.0
            })
            .sum();
        assert!((area - (81.0 - 25.0 + 1.0)).abs() < 1e-3, "area {area}");
        // made solid: closed, and as big as its area times the depth
        let solid = extrude(&cap, [0.0, 0.0, 2.0], 0.4);
        let h = HalfEdgeMesh::from_mesh(&solid).unwrap();
        assert!(h.is_closed());
        assert!((h.signed_volume() - 2.0 * 57.0).abs() < 1e-2, "{}", h.signed_volume());
        // the other way, down from the plane
        let down = HalfEdgeMesh::from_mesh(&extrude(&cap, [0.0, 0.0, -1.0], 0.4)).unwrap();
        assert!((down.signed_volume() - 57.0).abs() < 1e-2, "{}", down.signed_volume());
    }

    #[test]
    fn a_round_wall_hides_its_upright_edges() {
        let ring: Vec<[f32; 2]> = (0..24).map(|i| {
            let a = i as f32 / 24.0 * std::f32::consts::TAU;
            [a.cos(), a.sin()]
        }).collect();
        let cap = faces(&[ring], &Plane::ground()).unwrap();
        let solid = extrude(&cap, [0.0, 0.0, 1.0], 0.5);
        assert_eq!(solid.seams.len(), 24, "every upright edge of a 24-sided round");
        let square = extrude(&faces(&[square(0.0, 0.0, 1.0)], &Plane::ground()).unwrap(), [0.0, 0.0, 1.0], 0.5);
        assert!(square.seams.is_empty(), "a box keeps its corners");
    }
}
