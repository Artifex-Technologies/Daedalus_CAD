//! Where a section plane cuts the model: the lines drawn round the cut (SketchUp's section cut lines).

use crate::Mesh;

/// The pieces of line where the plane through `point` with normal `normal` crosses the faces of `mesh`. Each face is cut
/// whole (not by its triangles), so a concave face gives one piece per span inside it and no lines across it.
pub fn cut_lines(mesh: &Mesh, point: [f64; 3], normal: [f64; 3]) -> Vec<([f64; 3], [f64; 3])> {
    let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let off = dot(point, normal);
    // a vertex on the plane counts as in front, so a crossing there is found once
    let side = |p: [f64; 3]| dot(p, normal) - off;
    let mut out = Vec::new();
    for f in &mesh.faces {
        let n = f.len();
        let mut hits: Vec<[f64; 3]> = Vec::new();
        for i in 0..n {
            let (a, b) = (mesh.positions[f[i] as usize], mesh.positions[f[(i + 1) % n] as usize]);
            let (da, db) = (side(a), side(b));
            if (da >= 0.0) != (db >= 0.0) {
                let t = da / (da - db);
                hits.push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]);
            }
        }
        if hits.len() < 2 {
            continue;
        }
        // along the cut line through the face: pair the crossings in order
        let far = hits.iter().copied().fold((hits[0], 0.0f64), |best, p| {
            let d = dot([p[0] - hits[0][0], p[1] - hits[0][1], p[2] - hits[0][2]], [p[0] - hits[0][0], p[1] - hits[0][1], p[2] - hits[0][2]]);
            if d > best.1 { (p, d) } else { best }
        });
        let dir = [far.0[0] - hits[0][0], far.0[1] - hits[0][1], far.0[2] - hits[0][2]];
        hits.sort_by(|p, q| dot(*p, dir).total_cmp(&dot(*q, dir)));
        for pair in hits.as_chunks::<2>().0 {
            out.push((pair[0], pair[1]));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cube_cut_through_the_middle_gives_a_square() {
        let mut m = Mesh::new();
        for p in [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 1.0], [1.0, 1.0, 1.0], [0.0, 1.0, 1.0]] {
            m.add_vertex(p);
        }
        for f in [vec![0, 3, 2, 1], vec![4, 5, 6, 7], vec![0, 1, 5, 4], vec![1, 2, 6, 5], vec![2, 3, 7, 6], vec![3, 0, 4, 7]] {
            m.add_face(f);
        }
        let lines = cut_lines(&m, [0.0, 0.0, 0.5], [0.0, 0.0, 1.0]);
        assert_eq!(lines.len(), 4, "one piece on each side wall");
        let total: f64 = lines.iter().map(|(a, b)| ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()).sum();
        assert!((total - 4.0).abs() < 1e-5);
        assert!(lines.iter().all(|(a, b)| (a[2] - 0.5).abs() < 1e-6 && (b[2] - 0.5).abs() < 1e-6));
        assert!(cut_lines(&m, [0.0, 0.0, 2.0], [0.0, 0.0, 1.0]).is_empty(), "a plane above it misses it");
    }

    #[test]
    fn a_concave_face_is_cut_in_two_spans() {
        // a U shape standing up in the x-z plane, cut across both arms
        let mut m = Mesh::new();
        for p in [[0.0, 0.0, 0.0], [3.0, 0.0, 0.0], [3.0, 0.0, 2.0], [2.0, 0.0, 2.0], [2.0, 0.0, 1.0], [1.0, 0.0, 1.0], [1.0, 0.0, 2.0], [0.0, 0.0, 2.0]] {
            m.add_vertex(p);
        }
        m.add_face((0..8).collect());
        let lines = cut_lines(&m, [0.0, 0.0, 1.5], [0.0, 0.0, 1.0]);
        assert_eq!(lines.len(), 2);
    }
}
