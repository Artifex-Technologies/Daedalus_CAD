//! Drawing a line into what is already there, as SketchUp does: the line's ends join the corners and edges they land
//! on, it breaks at every edge it crosses (and those edges break with it), a piece that runs across a face from one
//! boundary point to another splits the face in two, and the rest are loose lines. Works on the interchange [`Mesh`]
//! (rings of vertex numbers); the half-edge mesh is rebuilt from it afterwards.

use crate::Mesh;

type V = [f32; 3];

fn sub(a: V, b: V) -> V {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn add(a: V, b: V) -> V {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn scale(a: V, s: f32) -> V {
    [a[0] * s, a[1] * s, a[2] * s]
}
fn dot(a: V, b: V) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn len(a: V) -> f32 {
    dot(a, a).sqrt()
}

/// Where `p` lies along segment `a`-`b` (0..1) when it is on it, within `tol`.
pub fn on_segment(p: V, a: V, b: V, tol: f32) -> Option<f32> {
    let d = sub(b, a);
    let l2 = dot(d, d);
    if l2 <= tol * tol {
        return None;
    }
    let t = dot(sub(p, a), d) / l2;
    let closest = add(a, scale(d, t));
    (len(sub(p, closest)) <= tol && (-1e-6..=1.0 + 1e-6).contains(&t)).then_some(t.clamp(0.0, 1.0))
}

/// Where segments `a`-`b` and `c`-`d` cross (within `tol` of each other), strictly inside both: (t along ab, point).
pub fn crossing(a: V, b: V, c: V, d: V, tol: f32) -> Option<(f32, V)> {
    let (u, v, w0) = (sub(b, a), sub(d, c), sub(a, c));
    let (aa, bb, cc, dd, ee) = (dot(u, u), dot(u, v), dot(v, v), dot(u, w0), dot(v, w0));
    let denom = aa * cc - bb * bb;
    if denom.abs() <= 1e-12 * aa.max(1e-12) * cc.max(1e-12) {
        return None; // parallel
    }
    let s = (bb * ee - cc * dd) / denom;
    let t = (aa * ee - bb * dd) / denom;
    let eps_s = tol / aa.sqrt();
    let eps_t = tol / cc.sqrt();
    if s <= eps_s || s >= 1.0 - eps_s || t <= eps_t || t >= 1.0 - eps_t {
        return None;
    }
    let (p, q) = (add(a, scale(u, s)), add(c, scale(v, t)));
    (len(sub(p, q)) <= tol).then_some((s, scale(add(p, q), 0.5)))
}

/// The face's unit normal (Newell) and a point on it.
pub fn face_plane(m: &Mesh, ring: &[u32]) -> Option<(V, V)> {
    let mut n = [0.0f32; 3];
    for i in 0..ring.len() {
        let (p, q) = (m.positions[ring[i] as usize], m.positions[ring[(i + 1) % ring.len()] as usize]);
        n[0] += (p[1] - q[1]) * (p[2] + q[2]);
        n[1] += (p[2] - q[2]) * (p[0] + q[0]);
        n[2] += (p[0] - q[0]) * (p[1] + q[1]);
    }
    let l = len(n);
    (l > 1e-12).then(|| (scale(n, 1.0 / l), m.positions[ring[0] as usize]))
}

/// Whether `p` lies in the face's plane (within `tol`) and inside its outline (even-odd rule, in the plane).
pub fn inside_face(m: &Mesh, ring: &[u32], p: V, tol: f32) -> bool {
    let Some((n, o)) = face_plane(m, ring) else { return false };
    if dot(sub(p, o), n).abs() > tol {
        return false;
    }
    // drop the normal's largest axis to work in 2D
    let k = (0..3).max_by(|&i, &j| n[i].abs().total_cmp(&n[j].abs())).unwrap_or(2);
    let (x, y) = match k {
        0 => (1, 2),
        1 => (0, 2),
        _ => (0, 1),
    };
    let mut inside = false;
    for i in 0..ring.len() {
        let a = m.positions[ring[i] as usize];
        let b = m.positions[ring[(i + 1) % ring.len()] as usize];
        if (a[y] > p[y]) != (b[y] > p[y]) {
            let at = a[x] + (p[y] - a[y]) / (b[y] - a[y]) * (b[x] - a[x]);
            if p[x] < at {
                inside = !inside;
            }
        }
    }
    inside
}

/// Every edge of the mesh: face edges and loose lines, as (u, v), each once.
fn all_edges(m: &Mesh) -> Vec<(u32, u32)> {
    m.all_edges()
}

/// Puts vertex `w` into the edge `u`-`v` wherever it is: between them in every face ring that has that edge, and a loose
/// line `u`-`v` becomes two.
pub fn insert_on_edge(m: &mut Mesh, u: u32, v: u32, w: u32) {
    for ring in &mut m.faces {
        let n = ring.len();
        if let Some(i) = (0..n).find(|&i| {
            let (a, b) = (ring[i], ring[(i + 1) % n]);
            (a == u && b == v) || (a == v && b == u)
        }) {
            ring.insert(i + 1, w);
        }
    }
    let mut extra = Vec::new();
    for l in &mut m.lines {
        if (l[0] == u && l[1] == v) || (l[0] == v && l[1] == u) {
            let end = l[1];
            l[1] = w;
            extra.push([w, end]);
        }
    }
    m.lines.extend(extra);
    let mut extra = Vec::new();
    for l in &mut m.seams {
        if (l[0] == u && l[1] == v) || (l[0] == v && l[1] == u) {
            let end = l[1];
            l[1] = w;
            extra.push([w, end]);
        }
    }
    m.seams.extend(extra);
}

/// The vertex at `p`: an existing one within `tol`, else a new one, put into any edge it lies on.
pub fn vertex_at(m: &mut Mesh, p: V, tol: f32) -> u32 {
    if let Some(i) = m.positions.iter().position(|q| len(sub(*q, p)) <= tol) {
        return i as u32;
    }
    let edges = all_edges(m);
    let w = m.add_vertex(p);
    for (u, v) in edges {
        let (a, b) = (m.positions[u as usize], m.positions[v as usize]);
        if on_segment(p, a, b, tol).is_some_and(|t| t > 0.0 && t < 1.0) {
            insert_on_edge(m, u, v, w);
        }
    }
    w
}

fn has_edge(m: &Mesh, u: u32, v: u32) -> bool {
    let key = (u.min(v), u.max(v));
    m.all_edges().binary_search(&key).is_ok()
}

/// Draws the line `a`-`b` into `m` (see the module notes). Returns the loose lines it added (indices into `m.lines`), for
/// the caller to try closing into faces, and whether it split a face.
pub fn add_segment(m: &mut Mesh, a: V, b: V, tol: f32) -> (Vec<usize>, bool) {
    let ia = vertex_at(m, a, tol);
    let ib = vertex_at(m, b, tol);
    if ia == ib {
        return (Vec::new(), false);
    }
    let (pa, pb) = (m.positions[ia as usize], m.positions[ib as usize]);
    // break points along the line: corners it passes through, and edges it crosses (which break too)
    let mut stops: Vec<(f32, u32)> = vec![(0.0, ia), (1.0, ib)];
    for (i, p) in m.positions.clone().into_iter().enumerate() {
        let i = i as u32;
        if i != ia && i != ib {
            if let Some(t) = on_segment(p, pa, pb, tol) {
                if t > 0.0 && t < 1.0 {
                    stops.push((t, i));
                }
            }
        }
    }
    for (u, v) in all_edges(m) {
        if [u, v].iter().any(|x| stops.iter().any(|s| s.1 == *x)) {
            continue;
        }
        let (c, d) = (m.positions[u as usize], m.positions[v as usize]);
        if let Some((t, p)) = crossing(pa, pb, c, d, tol) {
            let w = m.add_vertex(p);
            insert_on_edge(m, u, v, w);
            stops.push((t, w));
        }
    }
    stops.sort_by(|x, y| x.0.total_cmp(&y.0));
    stops.dedup_by_key(|s| s.1);

    let mut added = Vec::new();
    let mut split = false;
    for pair in stops.windows(2) {
        let (u, w) = (pair[0].1, pair[1].1);
        if u == w || has_edge(m, u, w) {
            continue;
        }
        let mid = scale(add(m.positions[u as usize], m.positions[w as usize]), 0.5);
        // a piece across a face, between two of its corners: the face splits along it
        let face = m.faces.iter().position(|ring| {
            let (Some(i), Some(j)) = (ring.iter().position(|&x| x == u), ring.iter().position(|&x| x == w)) else { return false };
            let n = ring.len();
            (i + 1) % n != j && (j + 1) % n != i && inside_face(m, ring, mid, tol * 10.0)
        });
        match face {
            Some(f) => {
                let ring = m.faces[f].clone();
                let n = ring.len();
                let i = ring.iter().position(|&x| x == u).expect("found above");
                let j = ring.iter().position(|&x| x == w).expect("found above");
                let (lo, hi) = (i.min(j), i.max(j));
                let first: Vec<u32> = ring[lo..=hi].to_vec();
                let mut second: Vec<u32> = ring[hi..].to_vec();
                second.extend_from_slice(&ring[..=lo]);
                debug_assert!(first.len() >= 3 && second.len() >= 3 && first.len() + second.len() == n + 2);
                m.faces[f] = first;
                m.faces.push(second);
                split = true;
            }
            None => {
                m.lines.push([u, w]);
                added.push(m.lines.len() - 1);
            }
        }
    }
    (added, split)
}

/// A closed outline `inner` drawn wholly inside face `outer`: the outline becomes a face of its own and the rest of
/// `outer` the ring around it. A face here has one outline and no holes, so the ring is made of two faces, joined by two
/// short seams from the outline to the outside edge, kept in `Mesh::seams` and not drawn. `inner` must wind the same way as `outer`. Returns the new inner face's index.
pub fn cut_hole(m: &mut Mesh, outer: usize, inner: &[u32]) -> Option<usize> {
    let ring = m.faces.get(outer)?.clone();
    let (n, k) = (ring.len(), inner.len());
    if k < 3 || n < 3 || inner.iter().any(|v| ring.contains(v)) {
        return None;
    }
    let pos = |v: u32| m.positions[v as usize];
    let nearest = |q: u32, not: Option<usize>| -> Option<usize> {
        (0..n).filter(|&i| Some(i) != not).min_by(|&i, &j| len(sub(pos(ring[i]), pos(q))).total_cmp(&len(sub(pos(ring[j]), pos(q)))))
    };
    // two seams: from the outline's first corner and from the one halfway round, each to its nearest outer corner
    let (q1, q2) = (0usize, k / 2);
    let p1 = nearest(inner[q1], None)?;
    let p2 = nearest(inner[q2], Some(p1))?;
    // the outer walk from p1 to p2 and from p2 to p1 (both counter-clockwise, as the face winds)
    let walk = |from: usize, to: usize| -> Vec<u32> {
        let mut v = vec![ring[from]];
        let mut i = from;
        while i != to {
            i = (i + 1) % n;
            v.push(ring[i]);
        }
        v
    };
    // the inner outline walked backwards (the ring goes round it the other way), from index `from` to index `to`
    let back = |from: usize, to: usize| -> Vec<u32> {
        let mut v = vec![inner[from]];
        let mut i = from;
        while i != to {
            i = (i + k - 1) % k;
            v.push(inner[i]);
        }
        v
    };
    let mut a = walk(p1, p2);
    a.extend(back(q2, q1));
    let mut b = walk(p2, p1);
    b.extend(back(q1, q2));
    if a.len() < 3 || b.len() < 3 {
        return None;
    }
    m.faces[outer] = a;
    m.faces.push(b);
    m.faces.push(inner.to_vec());
    m.seams.push([ring[p1], inner[q1]]);
    m.seams.push([ring[p2], inner[q2]]);
    Some(m.faces.len() - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crossing_segments_meet_and_parallel_ones_do_not() {
        let (t, p) = crossing([0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [1.0, -1.0, 0.0], [1.0, 1.0, 0.0], 1e-5).unwrap();
        assert!((t - 0.5).abs() < 1e-6 && (p[0] - 1.0).abs() < 1e-6);
        assert!(crossing([0.0; 3], [2.0, 0.0, 0.0], [0.0, 1.0, 0.0], [2.0, 1.0, 0.0], 1e-5).is_none());
        assert!(crossing([0.0; 3], [2.0, 0.0, 0.0], [1.0, -1.0, 1.0], [1.0, 1.0, 1.0], 1e-5).is_none(), "one above the other");
        assert!(crossing([0.0; 3], [2.0, 0.0, 0.0], [2.0, 0.0, 0.0], [2.0, 1.0, 0.0], 1e-5).is_none(), "touching at an end");
    }

    #[test]
    fn inside_a_face() {
        let mut m = Mesh::new();
        for p in [[0.0, 0.0, 0.0], [4.0, 0.0, 0.0], [4.0, 4.0, 0.0], [0.0, 4.0, 0.0]] {
            m.add_vertex(p);
        }
        m.add_face(vec![0, 1, 2, 3]);
        assert!(inside_face(&m, &m.faces[0].clone(), [1.0, 1.0, 0.0], 1e-5));
        assert!(!inside_face(&m, &m.faces[0].clone(), [5.0, 1.0, 0.0], 1e-5));
        assert!(!inside_face(&m, &m.faces[0].clone(), [1.0, 1.0, 0.5], 1e-5), "off the plane");
    }
}
