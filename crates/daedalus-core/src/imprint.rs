//! Drawing a line into what is already there, as SketchUp does: the line's ends join the corners and edges they land
//! on, it breaks at every edge it crosses (and those edges break with it), a piece that runs across a face from one
//! boundary point to another splits the face in two, and the rest are loose lines. Works on the interchange [`Mesh`]
//! (rings of vertex numbers); the half-edge mesh is rebuilt from it afterwards.

use crate::Mesh;

type V = [f64; 3];

fn sub(a: V, b: V) -> V {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn add(a: V, b: V) -> V {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn scale(a: V, s: f64) -> V {
    [a[0] * s, a[1] * s, a[2] * s]
}
fn dot(a: V, b: V) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn len(a: V) -> f64 {
    dot(a, a).sqrt()
}

/// Where `p` lies along segment `a`-`b` (0..1) when it is on it, within `tol`.
pub fn on_segment(p: V, a: V, b: V, tol: f64) -> Option<f64> {
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
pub fn crossing(a: V, b: V, c: V, d: V, tol: f64) -> Option<(f64, V)> {
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
    let mut n = [0.0f64; 3];
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
pub fn inside_face(m: &Mesh, ring: &[u32], p: V, tol: f64) -> bool {
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
pub fn vertex_at(m: &mut Mesh, p: V, tol: f64) -> u32 {
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
pub fn add_segment(m: &mut Mesh, a: V, b: V, tol: f64) -> (Vec<usize>, bool) {
    let ia = vertex_at(m, a, tol);
    let ib = vertex_at(m, b, tol);
    if ia == ib {
        return (Vec::new(), false);
    }
    let (pa, pb) = (m.positions[ia as usize], m.positions[ib as usize]);
    // break points along the line: corners it passes through, and edges it crosses (which break too)
    let mut stops: Vec<(f64, u32)> = vec![(0.0, ia), (1.0, ib)];
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
                let c = m.color_of(f);
                m.set_color(m.faces.len() - 1, c);
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
    cut_hole_avoiding(m, outer, inner, &[])
}

/// [`cut_hole`], with seams that also keep clear of `avoid`: other holes of the same face still to be cut (a seam through
/// one of them would leave it in two faces, and it could not be cut).
pub fn cut_hole_avoiding(m: &mut Mesh, outer: usize, inner: &[u32], avoid: &[Vec<u32>]) -> Option<usize> {
    let ring = m.faces.get(outer)?.clone();
    let (n, k) = (ring.len(), inner.len());
    if k < 3 || n < 3 || inner.iter().any(|v| ring.contains(v)) {
        return None;
    }
    // everything is judged flat, in the face's plane
    let (normal, _) = face_plane(m, &ring)?;
    let (u, v) = {
        let t = if normal[0].abs() < 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] };
        let u = unit(cross(t, normal))?;
        (u, cross(normal, u))
    };
    let flat = |q: u32| {
        let p = m.positions[q as usize];
        [dot(p, u), dot(p, v)]
    };
    let outer2: Vec<[f64; 2]> = ring.iter().map(|&q| flat(q)).collect();
    let others: Vec<Vec<[f64; 2]>> = avoid.iter().map(|r| r.iter().map(|&q| flat(q)).collect()).collect();
    // the hole's outline taken the same way round as the face's (the walk below goes back along it), and the face that
    // fills it then faces the same way as the face
    let mut inner: Vec<u32> = inner.to_vec();
    let mut inner2: Vec<[f64; 2]> = inner.iter().map(|&q| flat(q)).collect();
    if signed_area(&outer2).signum() != signed_area(&inner2).signum() {
        inner.reverse();
        inner2.reverse();
    }
    let inner = &inner[..];
    let span = outer2.iter().chain(&inner2).fold(0.0f64, |s, p| s.max(p[0].abs()).max(p[1].abs())).max(1e-6);
    let eps = span * 1e-6;
    // a seam may join outer corner p and hole corner q only if it stays inside the face: it crosses no edge of the outline
    // or of the hole (touching them only at its own two ends), and its middle is inside the outline and outside the hole
    let clear = |pi: usize, qi: usize| -> bool {
        let (a, b) = (outer2[pi], inner2[qi]);
        let crosses = |poly: &[[f64; 2]], skip: Option<usize>| {
            (0..poly.len()).any(|i| {
                let j = (i + 1) % poly.len();
                if skip.is_some_and(|s| s == i || s == j) {
                    return false;
                }
                segments_cross(a, b, poly[i], poly[j], eps)
            })
        };
        let mid = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
        !crosses(&outer2, Some(pi))
            && !crosses(&inner2, Some(qi))
            && point_in(&outer2, mid)
            && !point_in(&inner2, mid)
            && others.iter().all(|o| !crosses(o, None) && !point_in(o, mid))
    };
    let d2 = |pi: usize, qi: usize| {
        let (a, b) = (outer2[pi], inner2[qi]);
        (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)
    };
    let mut pairs: Vec<(usize, usize)> = (0..n).flat_map(|pi| (0..k).map(move |qi| (pi, qi))).filter(|&(pi, qi)| clear(pi, qi)).collect();
    pairs.sort_by(|x, y| d2(x.0, x.1).total_cmp(&d2(y.0, y.1)));
    let area = |r: &[u32]| {
        let pts: Vec<[f64; 2]> = r.iter().map(|&q| flat(q)).collect();
        signed_area(&pts).abs()
    };
    let want = signed_area(&outer2).abs() - signed_area(&inner2).abs();
    // the outer walk from p1 to p2 (as the face winds), and the hole's outline walked backwards from q2 to q1
    let walk = |from: usize, to: usize| -> Vec<u32> {
        let mut out = vec![ring[from]];
        let mut i = from;
        while i != to {
            i = (i + 1) % n;
            out.push(ring[i]);
        }
        out
    };
    let back = |from: usize, to: usize| -> Vec<u32> {
        let mut out = vec![inner[from]];
        let mut i = from;
        while i != to {
            i = (i + k - 1) % k;
            out.push(inner[i]);
        }
        out
    };
    // the two seams: the shortest clear one, then the shortest clear one that does not cross it, ends elsewhere, and leaves
    // two faces that are each simple and together cover the face minus the hole
    for (i, &(p1, q1)) in pairs.iter().enumerate() {
        for &(p2, q2) in pairs.iter().skip(i + 1).take(64) {
            if p2 == p1 || q2 == q1 || segments_cross(outer2[p1], inner2[q1], outer2[p2], inner2[q2], eps) {
                continue;
            }
            let mut a = walk(p1, p2);
            a.extend(back(q2, q1));
            let mut b = walk(p2, p1);
            b.extend(back(q1, q2));
            if a.len() < 3 || b.len() < 3 {
                continue;
            }
            let simple = |r: &[u32]| {
                let pts: Vec<[f64; 2]> = r.iter().map(|&q| flat(q)).collect();
                !self_crossing(&pts, eps)
            };
            if (area(&a) + area(&b) - want).abs() > want.abs() * 1e-3 + eps || !simple(&a) || !simple(&b) {
                continue;
            }
            m.faces[outer] = a;
            m.faces.push(b);
            m.faces.push(inner.to_vec());
            // the ring and the new face inside it keep the face's colour
            let c = m.color_of(outer);
            m.set_color(m.faces.len() - 2, c);
            m.set_color(m.faces.len() - 1, c);
            m.seams.push([ring[p1], inner[q1]]);
            m.seams.push([ring[p2], inner[q2]]);
            return Some(m.faces.len() - 1);
        }
    }
    None
}

fn cross(a: V, b: V) -> V {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn unit(a: V) -> Option<V> {
    let l = len(a);
    (l > 1e-12).then(|| [a[0] / l, a[1] / l, a[2] / l])
}

fn signed_area(pts: &[[f64; 2]]) -> f64 {
    let mut s = 0.0;
    for i in 0..pts.len() {
        let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
        s += a[0] * b[1] - b[0] * a[1];
    }
    s * 0.5
}

/// Whether segments a-b and c-d cross or touch anywhere but at a shared end.
fn segments_cross(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2], eps: f64) -> bool {
    let same = |p: [f64; 2], q: [f64; 2]| (p[0] - q[0]).abs() <= eps && (p[1] - q[1]).abs() <= eps;
    if same(a, c) || same(a, d) || same(b, c) || same(b, d) {
        return false;
    }
    let o = |p: [f64; 2], q: [f64; 2], r: [f64; 2]| (q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0]);
    let (d1, d2, d3, d4) = (o(c, d, a), o(c, d, b), o(a, b, c), o(a, b, d));
    let tol = eps * eps.max(1e-12).sqrt().max(eps);
    if ((d1 > tol && d2 < -tol) || (d1 < -tol && d2 > tol)) && ((d3 > tol && d4 < -tol) || (d3 < -tol && d4 > tol)) {
        return true;
    }
    // touching: an end of one lying on the other
    let on = |p: [f64; 2], q: [f64; 2], r: [f64; 2], v: f64| {
        v.abs() <= tol && r[0] >= p[0].min(q[0]) - eps && r[0] <= p[0].max(q[0]) + eps && r[1] >= p[1].min(q[1]) - eps && r[1] <= p[1].max(q[1]) + eps
    };
    on(c, d, a, d1) || on(c, d, b, d2) || on(a, b, c, d3) || on(a, b, d, d4)
}

/// Even-odd point in polygon.
fn point_in(poly: &[[f64; 2]], p: [f64; 2]) -> bool {
    let mut c = false;
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        if (a[1] > p[1]) != (b[1] > p[1]) && p[0] < a[0] + (p[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]) {
            c = !c;
        }
    }
    c
}

/// Whether a closed outline crosses itself (two edges that are not neighbours meet).
fn self_crossing(pts: &[[f64; 2]], eps: f64) -> bool {
    let n = pts.len();
    for i in 0..n {
        for j in i + 2..n {
            if i == 0 && j == n - 1 {
                continue;
            }
            if segments_cross(pts[i], pts[(i + 1) % n], pts[j], pts[(j + 1) % n], eps) {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_square_hole_in_a_square_face() {
        let mut m = Mesh::new();
        for p in [[0.0, 0.0, 0.0], [9.0, 0.0, 0.0], [9.0, 9.0, 0.0], [0.0, 9.0, 0.0], [2.0, 2.0, 0.0], [7.0, 2.0, 0.0], [7.0, 7.0, 0.0], [2.0, 7.0, 0.0]] {
            m.add_vertex(p);
        }
        m.add_face(vec![0, 1, 2, 3]);
        for inner in [vec![4u32, 5, 6, 7], vec![7u32, 6, 5, 4]] {
            let mut t = m.clone();
            let f = cut_hole(&mut t, 0, &inner);
            assert!(f.is_some(), "{inner:?}");
            let area: f64 = t.faces[..2].iter().map(|r| { let p: Vec<[f64; 2]> = r.iter().map(|&v| [t.positions[v as usize][0], t.positions[v as usize][1]]).collect(); signed_area(&p) }).sum();
            assert!((area - 56.0).abs() < 1e-3, "{area}");
            assert_eq!(t.seams.len(), 2);
        }
    }

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
