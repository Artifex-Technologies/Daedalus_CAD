//! Merging coplanar faces, as SketchUp does: neighbouring faces that lie in one plane become one face, and a point left in the
//! middle of a straight edge (used by no other edge) is removed. Pushing a face of a box again then leaves no seam around the
//! sides, and an STL's triangle pairs become the quads they were drawn as.
//!
//! A group of faces is merged only when its outline is one simple loop. A group whose outline would have a hole or a pinch (two
//! loops, or a corner shared twice) is left as it is, because faces with holes are not supported yet.

use crate::Mesh;
use std::collections::{BTreeMap, HashMap};

type V3 = [f64; 3];

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn len(a: V3) -> f64 {
    dot(a, a).sqrt()
}
fn p(m: &Mesh, i: u32) -> V3 {
    let q = m.positions[i as usize];
    [q[0], q[1], q[2]]
}

/// A face's plane: unit normal (Newell) and offset, or None for a face with no area.
fn plane(m: &Mesh, f: &[u32]) -> Option<(V3, f64)> {
    let mut n = [0.0; 3];
    for i in 0..f.len() {
        let (a, b) = (p(m, f[i]), p(m, f[(i + 1) % f.len()]));
        n[0] += (a[1] - b[1]) * (a[2] + b[2]);
        n[1] += (a[2] - b[2]) * (a[0] + b[0]);
        n[2] += (a[0] - b[0]) * (a[1] + b[1]);
    }
    let l = len(n);
    if l < 1e-12 {
        return None;
    }
    let n = [n[0] / l, n[1] / l, n[2] / l];
    Some((n, dot(n, p(m, f[0]))))
}

fn find(parent: &mut [usize], mut x: usize) -> usize {
    while parent[x] != x {
        parent[x] = parent[parent[x]];
        x = parent[x];
    }
    x
}

/// The outline of a group of faces as one loop of vertices, or None when it is not one simple loop.
fn outline(group: &[&Vec<u32>]) -> Option<Vec<u32>> {
    let mut directed: HashMap<(u32, u32), usize> = HashMap::new();
    for f in group {
        for i in 0..f.len() {
            *directed.entry((f[i], f[(i + 1) % f.len()])).or_default() += 1;
        }
    }
    // the outline: edges whose reverse is not in the group
    let mut next: HashMap<u32, u32> = HashMap::new();
    for (&(a, b), &n) in &directed {
        if n != 1 {
            return None; // the group is not cleanly oriented
        }
        if !directed.contains_key(&(b, a)) && next.insert(a, b).is_some() {
            return None; // a corner the outline passes twice: a pinch
        }
    }
    let start = *next.keys().min()?;
    let mut loop_ = vec![start];
    let mut at = next[&start];
    while at != start {
        loop_.push(at);
        at = *next.get(&at)?;
        if loop_.len() > next.len() {
            return None;
        }
    }
    // every outline edge must be on this one loop (more edges: a hole, or a second piece)
    (loop_.len() == next.len()).then_some(loop_)
}

/// Faces `a` and `b` as one outline, if they lie in one plane, face the same way and together make one simple loop (the
/// Eraser on the edge between them, as in SketchUp). None otherwise.
pub(crate) fn join_pair(mesh: &Mesh, a: usize, b: usize, tol: f64) -> Option<Vec<u32>> {
    coplanar(mesh, a, b, tol).then(|| outline(&[&mesh.faces[a], &mesh.faces[b]])).flatten()
}

/// Whether faces `a` and `b` lie in one plane (within `tol`) and face the same way.
pub(crate) fn coplanar(mesh: &Mesh, a: usize, b: usize, tol: f64) -> bool {
    let (Some((na, da)), Some((nb, db))) = (plane(mesh, &mesh.faces[a]), plane(mesh, &mesh.faces[b])) else { return false };
    let in_plane = mesh.faces[b].iter().all(|&v| (dot(na, p(mesh, v)) - da).abs() <= tol) && (da - db).abs() <= tol * 4.0;
    dot(na, nb) > 1.0 - 1e-6 && in_plane
}

/// Merges coplanar neighbours. `tol` is the distance (model units) within which two planes count as one. Returns the merged mesh
/// and, for every face of `mesh`, the index of the face it became part of.
pub fn merge_coplanar(mesh: &Mesh, tol: f64) -> (Mesh, Vec<usize>) {
    let planes: Vec<Option<(V3, f64)>> = mesh.faces.iter().map(|f| plane(mesh, f)).collect();
    let mut parent: Vec<usize> = (0..mesh.faces.len()).collect();
    // faces on each undirected edge
    let mut on_edge: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
    for (fi, f) in mesh.faces.iter().enumerate() {
        for i in 0..f.len() {
            let (a, b) = (f[i], f[(i + 1) % f.len()]);
            on_edge.entry((a.min(b), a.max(b))).or_default().push(fi);
        }
    }
    for faces in on_edge.values() {
        let [a, b] = faces[..] else { continue }; // only an edge between exactly two faces
        let (Some((na, da)), Some((nb, db))) = (planes[a], planes[b]) else { continue };
        let same_direction = dot(na, nb) > 1.0 - 1e-6;
        // the other face's corners all lie in this face's plane (a distance, so the tolerance means the same at any size)
        let in_plane = mesh.faces[b].iter().all(|&v| (dot(na, p(mesh, v)) - da).abs() <= tol) && (da - db).abs() <= tol * 4.0;
        if same_direction && in_plane {
            let (ra, rb) = (find(&mut parent, a), find(&mut parent, b));
            if ra != rb {
                parent[ra] = rb;
            }
        }
    }
    let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for fi in 0..mesh.faces.len() {
        let r = find(&mut parent, fi);
        groups.entry(r).or_default().push(fi);
    }
    let mut faces: Vec<Vec<u32>> = vec![];
    let mut holes: Vec<Vec<Vec<u32>>> = vec![];
    let mut colors: Vec<Option<[u8; 3]>> = vec![];
    let mut map = vec![0usize; mesh.faces.len()];
    for members in groups.values() {
        // faces of different colours stay apart
        let same_color = members.iter().all(|&i| mesh.color_of(i) == mesh.color_of(members[0]));
        let merged = if members.len() > 1 && same_color { outline(&members.iter().map(|&i| &mesh.faces[i]).collect::<Vec<_>>()) } else { None };
        match merged {
            Some(loop_) => {
                for &i in members {
                    map[i] = faces.len();
                }
                faces.push(loop_);
                // the holes of the faces joined, all in the one face now
                holes.push(members.iter().flat_map(|&i| mesh.holes_of(i).iter().cloned()).collect());
                colors.push(mesh.color_of(members[0]));
            }
            None => {
                for &i in members {
                    map[i] = faces.len();
                    faces.push(mesh.faces[i].clone());
                    holes.push(mesh.holes_of(i).to_vec());
                    colors.push(mesh.color_of(i));
                }
            }
        }
    }
    // a point in the middle of a straight edge, used by no other edge, goes
    let mut neighbours: HashMap<u32, Vec<u32>> = HashMap::new();
    for f in faces.iter().chain(holes.iter().flatten()) {
        for i in 0..f.len() {
            let (a, b) = (f[i], f[(i + 1) % f.len()]);
            neighbours.entry(a).or_default().push(b);
            neighbours.entry(b).or_default().push(a);
        }
    }
    // a loose line at a corner keeps it
    for &[a, b] in &mesh.lines {
        neighbours.entry(a).or_default().push(b);
        neighbours.entry(b).or_default().push(a);
    }
    let straight = |v: u32| -> bool {
        let mut n = neighbours[&v].clone();
        n.sort_unstable();
        n.dedup();
        let [a, b] = n[..] else { return false };
        let (d1, d2) = (sub(p(mesh, v), p(mesh, a)), sub(p(mesh, b), p(mesh, v)));
        let (l1, l2) = (len(d1), len(d2));
        l1 > 0.0 && l2 > 0.0 && len(cross(d1, d2)) <= tol * (l1 + l2) && dot(d1, d2) > 0.0
    };
    for f in faces.iter_mut().chain(holes.iter_mut().flatten()) {
        if f.len() > 3 {
            let keep: Vec<u32> = f.iter().copied().filter(|&v| !straight(v)).collect();
            if keep.len() >= 3 {
                *f = keep;
            }
        }
    }
    // only the vertices still used, numbered in their old order
    let mut used = vec![false; mesh.positions.len()];
    for f in faces.iter().chain(holes.iter().flatten()) {
        for &v in f {
            used[v as usize] = true;
        }
    }
    for l in &mesh.lines {
        for &v in l {
            used[v as usize] = true;
        }
    }
    let mut renumber = vec![u32::MAX; mesh.positions.len()];
    let mut out = Mesh::new();
    for (i, &u) in used.iter().enumerate() {
        if u {
            renumber[i] = out.add_vertex(mesh.positions[i]);
        }
    }
    for f in faces {
        out.add_face(f.iter().map(|&v| renumber[v as usize]).collect());
    }
    out.lines = mesh.lines.iter().map(|l| l.map(|v| renumber[v as usize])).collect();
    out.colors = colors;
    out.holes = holes.into_iter().map(|hs| hs.into_iter().map(|h| h.iter().map(|&v| renumber[v as usize]).collect()).collect()).collect();
    out.seams = mesh.seams.iter().map(|l| l.map(|v| renumber[v as usize])).filter(|l| l[0] != u32::MAX && l[1] != u32::MAX).collect();
    (out, map)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::HalfEdgeMesh;

    fn quad_box() -> Mesh {
        let mut m = Mesh::new();
        for p in [[0.0, 0.0, 0.0], [3.0, 0.0, 0.0], [3.0, 2.0, 0.0], [0.0, 2.0, 0.0]] {
            m.add_vertex(p);
        }
        m.add_face(vec![0, 1, 2, 3]);
        m
    }

    #[test]
    fn a_box_pushed_twice_has_no_seam() {
        let mut h = HalfEdgeMesh::from_mesh(&quad_box()).unwrap();
        let top = h.push_pull(0, 1.0).unwrap();
        let top = h.push_pull(top, 0.5).unwrap();
        let before = h.to_mesh();
        assert_eq!(before.faces.len(), 10, "the second push left a band of four side faces");
        let (after, map) = merge_coplanar(&before, 1e-5);
        assert_eq!(after.faces.len(), 6, "a box again");
        assert_eq!(after.positions.len(), 8, "the seam's corners are gone");
        assert!(after.faces.iter().all(|f| f.len() == 4));
        let h2 = HalfEdgeMesh::from_mesh(&after).unwrap();
        assert!(h2.is_closed());
        assert_eq!(h2.euler_characteristic(), 2);
        assert!((h2.signed_volume() - h.signed_volume()).abs() < 1e-4 && (h2.signed_volume() - 9.0).abs() < 1e-4, "{}", h2.signed_volume());
        assert!(map[top] < after.faces.len());
        assert_eq!(after.faces[map[top]].len(), 4, "the pushed face is still the top");
    }

    #[test]
    fn triangle_pairs_become_quads() {
        let mut m = Mesh::new();
        for p in [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 1.0], [1.0, 1.0, 1.0], [0.0, 1.0, 1.0]] {
            m.add_vertex(p);
        }
        for q in [[0, 3, 2, 1], [4, 5, 6, 7], [0, 1, 5, 4], [1, 2, 6, 5], [2, 3, 7, 6], [3, 0, 4, 7]] {
            m.add_face(vec![q[0], q[1], q[2]]);
            m.add_face(vec![q[0], q[2], q[3]]);
        }
        let (after, _) = merge_coplanar(&m, 1e-5);
        assert_eq!(after.faces.len(), 6);
        assert!(HalfEdgeMesh::from_mesh(&after).unwrap().is_closed());
    }

    #[test]
    fn different_planes_and_holes_are_left_alone() {
        // a ring of four quads around a square hole: one plane, but the outline would have a hole
        let mut m = Mesh::new();
        let pts = [[0.0, 0.0], [3.0, 0.0], [3.0, 3.0], [0.0, 3.0], [1.0, 1.0], [2.0, 1.0], [2.0, 2.0], [1.0, 2.0]];
        for [x, y] in pts {
            m.add_vertex([x, y, 0.0]);
        }
        for f in [[0, 1, 5, 4], [1, 2, 6, 5], [2, 3, 7, 6], [3, 0, 4, 7]] {
            m.add_face(f.to_vec());
        }
        let (after, _) = merge_coplanar(&m, 1e-5);
        assert_eq!(after.faces.len(), 4, "a face with a hole cannot be made yet");
        // two quads at an angle (a fold) stay two
        let mut fold = Mesh::new();
        for p in [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0], [1.0, 1.0, 1.0], [0.0, 1.0, 1.0]] {
            fold.add_vertex(p);
        }
        fold.add_face(vec![0, 1, 2, 3]);
        fold.add_face(vec![3, 2, 4, 5]);
        assert_eq!(merge_coplanar(&fold, 1e-5).0.faces.len(), 2);
    }
}
