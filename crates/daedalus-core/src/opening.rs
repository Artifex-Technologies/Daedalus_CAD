//! Openings cut by glued components, as SketchUp's "Cut opening": a component whose definition cuts (a window) makes a
//! hole the shape of its footprint in the face it sits on. The footprint is the largest closed loop of the component's
//! edges lying in the plane of a face round it. The hole is a true hole in that face; it is found again by its corners when
//! the component moves or goes, so nothing else needs to remember it.

use crate::scene::{Instance, Scene};
use crate::Mesh;
use std::collections::{HashMap, HashSet};

type V = [f64; 3];

fn sub(a: V, b: V) -> V {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: V, b: V) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Twice the area vector of a loop of points (Newell).
fn area_vec(pts: &[V]) -> V {
    let mut n = [0.0f64; 3];
    for i in 0..pts.len() {
        let (p, q) = (pts[i], pts[(i + 1) % pts.len()]);
        n[0] += (p[1] - q[1]) * (p[2] + q[2]);
        n[1] += (p[2] - q[2]) * (p[0] + q[0]);
        n[2] += (p[0] - q[0]) * (p[1] + q[1]);
    }
    n
}

/// Instance `inst` (of the context at `open`)'s footprint, placed, if its definition cuts an opening: the largest closed
/// loop of its edges lying in the plane of a face of `host` (the open context's geometry), in `host`'s coordinates. A window
/// drawn on a wall and made a component touches the wall along its outline, whether it keeps a back face there or only
/// the frame's sides: that loop is the opening.
pub fn footprint(scene: &Scene, open: &[usize], inst: &Instance, host: &Mesh) -> Option<Vec<V>> {
    if !scene.defs.get(inst.def)?.cut {
        return None;
    }
    contact(scene, open, inst, host)
}

/// The unit normal of the face a glued instance sits on (from where it touches it), if its definition is glued.
pub fn glue_normal(scene: &Scene, open: &[usize], inst: &Instance, host: &Mesh) -> Option<V> {
    if !scene.defs.get(inst.def)?.glue {
        return None;
    }
    let n = area_vec(&contact(scene, open, inst, host)?);
    let l = dot(n, n).sqrt();
    (l > 0.0).then(|| n.map(|c| c / l))
}

/// Where the instance touches a face of `host`: the largest closed loop of its edges in such a face's plane.
fn contact(scene: &Scene, open: &[usize], inst: &Instance, host: &Mesh) -> Option<Vec<V>> {
    let d = scene.defs.get(inst.def)?;
    let w = scene.world(open).after(&inst.xf);
    let m = d.ctx.mesh.transformed(&w);
    let tol = host.tolerance().max(m.tolerance()) * 10.0;
    let mut planes: Vec<(V, f64)> = Vec::new();
    for f in 0..host.faces.len() {
        if let Some((n, o)) = crate::imprint::face_plane(host, &host.faces[f]) {
            let off = dot(n, o);
            if !planes.iter().any(|&(q, k)| dot(q, n).abs() > 1.0 - 1e-9 && (k - off * dot(q, n).signum()).abs() <= tol) {
                planes.push((n, off));
            }
        }
    }
    let edges = m.edges();
    let mut best: Option<(f64, Vec<V>)> = None;
    for (n, off) in planes {
        let on = |v: u32| (dot(m.positions[v as usize], n) - off).abs() <= tol;
        let mut adj: HashMap<u32, Vec<u32>> = HashMap::new();
        for &(a, b) in &edges {
            if on(a) && on(b) {
                adj.entry(a).or_default().push(b);
                adj.entry(b).or_default().push(a);
            }
        }
        if adj.len() < 3 || adj.values().any(|v| v.len() != 2) {
            continue; // not simple loops (a corner meeting three of them): no clear opening there
        }
        let mut left: HashSet<u32> = adj.keys().copied().collect();
        while let Some(&start) = left.iter().min() {
            let mut lp = vec![start];
            left.remove(&start);
            let (mut prev, mut at) = (start, adj[&start][0]);
            while at != start && left.remove(&at) {
                lp.push(at);
                let nx = if adj[&at][0] == prev { adj[&at][1] } else { adj[&at][0] };
                (prev, at) = (at, nx);
            }
            let pts: Vec<V> = lp.iter().map(|&v| m.positions[v as usize]).collect();
            let a = { let n = area_vec(&pts); dot(n, n).sqrt() };
            if pts.len() >= 3 && best.as_ref().is_none_or(|b| a > b.0) {
                best = Some((a, pts));
            }
        }
    }
    best.map(|b| b.1)
}

/// Cuts the opening `outline` into the face it lies on (in its plane, wholly on it, not over a hole): a hole there. False
/// when no face takes it.
pub fn cut(mesh: &mut Mesh, outline: &[V]) -> bool {
    let tol = mesh.tolerance().max(1e-9) * 10.0;
    let Some(host) = (0..mesh.faces.len()).find(|&f| outline.iter().all(|&p| crate::imprint::on_face(mesh, f, p, tol))) else { return false };
    let Some((n, _)) = crate::imprint::face_plane(mesh, &mesh.faces[host]) else { return false };
    let mut ring: Vec<u32> = outline.iter().map(|&p| mesh.add_vertex(p)).collect();
    // a hole runs the other way round from its face
    if dot(area_vec(outline), n) > 0.0 {
        ring.reverse();
    }
    let mut holes = mesh.holes_of(host).to_vec();
    holes.push(ring);
    mesh.set_holes(host, holes);
    true
}

/// Takes away the hole with the corners of `outline` (the opening a component cut there before it moved or went). False
/// when there is none.
pub fn uncut(mesh: &mut Mesh, outline: &[V]) -> bool {
    let tol = mesh.tolerance().max(1e-9) * 10.0;
    let near = |a: V, b: V| dot(sub(a, b), sub(a, b)).sqrt() <= tol;
    for f in 0..mesh.faces.len() {
        let holes = mesh.holes_of(f).to_vec();
        if let Some(k) = holes.iter().position(|h| h.len() == outline.len() && outline.iter().all(|&p| h.iter().any(|&v| near(mesh.positions[v as usize], p)))) {
            let mut rest = holes;
            rest.remove(k);
            mesh.set_holes(f, rest);
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_opening_is_cut_and_taken_back() {
        // a 4 x 3 wall in the x-z plane, and a 1 x 1.5 window outline on it
        let mut m = Mesh::new();
        for p in [[0.0, 0.0, 0.0], [4.0, 0.0, 0.0], [4.0, 0.0, 3.0], [0.0, 0.0, 3.0]] {
            m.add_vertex(p);
        }
        m.add_face(vec![0, 1, 2, 3]);
        let window = [[1.0, 0.0, 1.0], [2.0, 0.0, 1.0], [2.0, 0.0, 2.5], [1.0, 0.0, 2.5]];
        assert!(cut(&mut m, &window));
        assert_eq!(m.holes_of(0).len(), 1);
        crate::HalfEdgeMesh::from_mesh(&m).expect("still a good surface");
        // off the wall: no host
        assert!(!cut(&mut m.clone(), &[[5.0, 0.0, 1.0], [6.0, 0.0, 1.0], [6.0, 0.0, 2.0]]));
        assert!(uncut(&mut m, &window));
        assert!(m.holes_of(0).is_empty());
        assert!(!uncut(&mut m, &window));
    }
}
