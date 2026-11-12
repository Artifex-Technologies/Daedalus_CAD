//! Solid Tools: union, subtraction and intersection of closed solids (SketchUp's Union, Subtract, Trim, Intersect, Split and
//! Outer Shell are made of these).
//!
//! The classic way with a BSP tree (binary space partitioning, as Laidlaw, Trumbore and Hughes, and later csg.js, did it):
//! each solid's faces, cut into triangles so every piece is convex, sort space into in front of and behind their planes;
//! each solid's pieces are clipped against the other's tree, keeping what lies outside (or inside) it. The pieces that are
//! left are welded back together, edges that end part-way along another are split there (T-junctions), and pieces lying in
//! one plane are merged back into whole faces.
//!
//! A piece keeps the paint of the face it came from. Everything is f64, with a tolerance from the solids' size.

use crate::{Mesh, Paint};

type V = [f64; 3];

fn sub(a: V, b: V) -> V {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: V, b: V) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: V, b: V) -> V {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn lerp(a: V, b: V, t: f64) -> V {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

#[derive(Debug, Clone, Copy)]
struct Plane {
    n: V,
    w: f64,
}

impl Plane {
    fn through(a: V, b: V, c: V) -> Option<Plane> {
        let n = cross(sub(b, a), sub(c, a));
        let l = dot(n, n).sqrt();
        (l > 1e-300).then(|| {
            let n = n.map(|x| x / l);
            Plane { n, w: dot(n, a) }
        })
    }
    fn flipped(self) -> Plane {
        Plane { n: self.n.map(|x| -x), w: -self.w }
    }
}

/// A convex piece of a face: its corners (counter-clockwise from outside), its plane, and the paint of its face.
#[derive(Debug, Clone)]
struct Poly {
    pts: Vec<V>,
    plane: Plane,
    paint: Option<Paint>,
}

impl Poly {
    fn flip(&mut self) {
        self.pts.reverse();
        self.plane = self.plane.flipped();
    }
}

/// Splits `p` by `plane` into the lists it belongs to (csg.js's splitPolygon), `eps` the thickness of the plane.
fn split(plane: Plane, p: Poly, eps: f64, cf: &mut Vec<Poly>, cb: &mut Vec<Poly>, front: &mut Vec<Poly>, back: &mut Vec<Poly>) {
    const COPLANAR: u8 = 0;
    const FRONT: u8 = 1;
    const BACK: u8 = 2;
    let kinds: Vec<u8> = p
        .pts
        .iter()
        .map(|&v| {
            let t = dot(plane.n, v) - plane.w;
            if t < -eps {
                BACK
            } else if t > eps {
                FRONT
            } else {
                COPLANAR
            }
        })
        .collect();
    let all = kinds.iter().fold(0, |a, &k| a | k);
    match all {
        COPLANAR => {
            if dot(plane.n, p.plane.n) > 0.0 {
                cf.push(p)
            } else {
                cb.push(p)
            }
        }
        FRONT => front.push(p),
        BACK => back.push(p),
        _ => {
            let (mut f, mut b) = (Vec::new(), Vec::new());
            let n = p.pts.len();
            for i in 0..n {
                let j = (i + 1) % n;
                let (ki, kj) = (kinds[i], kinds[j]);
                let (vi, vj) = (p.pts[i], p.pts[j]);
                if ki != BACK {
                    f.push(vi);
                }
                if ki != FRONT {
                    b.push(vi);
                }
                if (ki | kj) == (FRONT | BACK) {
                    let t = (plane.w - dot(plane.n, vi)) / dot(plane.n, sub(vj, vi));
                    let v = lerp(vi, vj, t);
                    f.push(v);
                    b.push(v);
                }
            }
            if f.len() >= 3 {
                front.push(Poly { pts: f, ..p.clone() });
            }
            if b.len() >= 3 {
                back.push(Poly { pts: b, ..p });
            }
        }
    }
}

/// A BSP tree node: a splitting plane, the pieces lying in it, and what lies in front of and behind it.
#[derive(Default)]
struct Node {
    plane: Option<Plane>,
    polys: Vec<Poly>,
    front: Option<Box<Node>>,
    back: Option<Box<Node>>,
}

impl Node {
    fn new(polys: Vec<Poly>, eps: f64) -> Node {
        let mut n = Node::default();
        n.build(polys, eps);
        n
    }

    fn build(&mut self, polys: Vec<Poly>, eps: f64) {
        if polys.is_empty() {
            return;
        }
        let plane = *self.plane.get_or_insert(polys[0].plane);
        let (mut front, mut back) = (Vec::new(), Vec::new());
        let mut cf = Vec::new();
        let mut cb = Vec::new();
        for p in polys {
            split(plane, p, eps, &mut cf, &mut cb, &mut front, &mut back);
        }
        self.polys.extend(cf);
        self.polys.extend(cb);
        if !front.is_empty() {
            self.front.get_or_insert_with(Box::default).build(front, eps);
        }
        if !back.is_empty() {
            self.back.get_or_insert_with(Box::default).build(back, eps);
        }
    }

    /// Solid and empty space swap (the solid's complement).
    fn invert(&mut self) {
        for p in &mut self.polys {
            p.flip();
        }
        if let Some(pl) = self.plane.as_mut() {
            *pl = pl.flipped();
        }
        if let Some(f) = self.front.as_mut() {
            f.invert();
        }
        if let Some(b) = self.back.as_mut() {
            b.invert();
        }
        std::mem::swap(&mut self.front, &mut self.back);
    }

    /// The parts of `polys` outside the solid this tree stands for.
    fn clip_polys(&self, polys: Vec<Poly>, eps: f64) -> Vec<Poly> {
        let Some(plane) = self.plane else { return polys };
        let (mut front, mut back) = (Vec::new(), Vec::new());
        let (mut cf, mut cb) = (Vec::new(), Vec::new());
        for p in polys {
            split(plane, p, eps, &mut cf, &mut cb, &mut front, &mut back);
        }
        front.extend(cf);
        back.extend(cb);
        let front = match &self.front {
            Some(f) => f.clip_polys(front, eps),
            None => front,
        };
        let back = match &self.back {
            Some(b) => b.clip_polys(back, eps),
            None => Vec::new(),
        };
        front.into_iter().chain(back).collect()
    }

    /// Removes from this tree everything inside `other`.
    fn clip_to(&mut self, other: &Node, eps: f64) {
        self.polys = other.clip_polys(std::mem::take(&mut self.polys), eps);
        if let Some(f) = self.front.as_mut() {
            f.clip_to(other, eps);
        }
        if let Some(b) = self.back.as_mut() {
            b.clip_to(other, eps);
        }
    }

    fn all(&self, out: &mut Vec<Poly>) {
        out.extend(self.polys.iter().cloned());
        if let Some(f) = &self.front {
            f.all(out);
        }
        if let Some(b) = &self.back {
            b.all(out);
        }
    }
}

/// The boolean operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Union,
    /// The first minus the second.
    Subtract,
    Intersect,
}

/// The faces of a closed solid as convex pieces (each face's triangles).
fn pieces(m: &Mesh) -> Vec<Poly> {
    let mut out = Vec::new();
    for f in 0..m.faces.len() {
        let paint = m.color_of(f);
        for t in m.triangulate_face(f) {
            let pts = t.map(|v| m.positions[v as usize]);
            if let Some(plane) = Plane::through(pts[0], pts[1], pts[2]) {
                out.push(Poly { pts: pts.to_vec(), plane, paint });
            }
        }
    }
    out
}

/// `op` on the closed solids `a` and `b` (in one coordinate frame): the resulting surface, welded and with its pieces in one
/// plane merged into faces. The work runs on its own thread with room for a deep tree.
pub fn boolean(a: &Mesh, b: &Mesh, op: Op) -> Mesh {
    let (a, b) = (a.clone(), b.clone());
    std::thread::Builder::new()
        .stack_size(512 << 20)
        .spawn(move || boolean_here(&a, &b, op))
        .and_then(|h| h.join().map_err(|_| std::io::Error::other("the solid operation failed")))
        .unwrap_or_default()
}

fn boolean_here(a: &Mesh, b: &Mesh, op: Op) -> Mesh {
    let size = {
        let mut both = a.clone();
        both.append(b);
        both.tolerance()
    };
    // the planes' thickness: well above rounding, well below any detail kept
    let eps = (size * 10.0).max(1e-10);
    let mut ta = Node::new(pieces(a), eps);
    let mut tb = Node::new(pieces(b), eps);
    match op {
        Op::Union => {
            ta.clip_to(&tb, eps);
            tb.clip_to(&ta, eps);
            tb.invert();
            tb.clip_to(&ta, eps);
            tb.invert();
        }
        Op::Subtract => {
            ta.invert();
            ta.clip_to(&tb, eps);
            tb.clip_to(&ta, eps);
            tb.invert();
            tb.clip_to(&ta, eps);
            tb.invert();
        }
        Op::Intersect => {
            ta.invert();
            tb.clip_to(&ta, eps);
            tb.invert();
            ta.clip_to(&tb, eps);
            tb.clip_to(&ta, eps);
        }
    }
    let mut polys = Vec::new();
    tb.all(&mut polys);
    let mut rest = Vec::new();
    ta.all(&mut rest);
    let mut both = Node::default();
    both.polys = rest;
    both.polys.extend(polys);
    let mut all = both.polys;
    if matches!(op, Op::Subtract | Op::Intersect) {
        for p in &mut all {
            p.flip();
        }
    }
    assemble(all, eps)
}

/// The pieces as one surface: corners within `eps` welded, T-junctions split, then pieces in one plane merged.
fn assemble(polys: Vec<Poly>, eps: f64) -> Mesh {
    let mut m = Mesh::new();
    let key = |p: V| p.map(|c| (c / eps).round() as i64);
    let mut at: std::collections::HashMap<[i64; 3], u32> = std::collections::HashMap::new();
    let mut vertex = |m: &mut Mesh, p: V| -> u32 {
        let k = key(p);
        // a corner within eps of one already there, also across a grid line
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    if let Some(&v) = at.get(&[k[0] + dx, k[1] + dy, k[2] + dz]) {
                        let q = m.positions[v as usize];
                        if dot(sub(p, q), sub(p, q)).sqrt() <= eps {
                            return v;
                        }
                    }
                }
            }
        }
        let v = m.add_vertex(p);
        at.insert(k, v);
        v
    };
    for p in polys {
        let mut ring: Vec<u32> = p.pts.iter().map(|&q| vertex(&mut m, q)).collect();
        ring.dedup();
        while ring.len() > 1 && ring.first() == ring.last() {
            ring.pop();
        }
        if ring.len() < 3 {
            continue;
        }
        m.faces.push(ring);
        let f = m.faces.len() - 1;
        m.set_color(f, p.paint);
    }
    split_t_junctions(&mut m, eps);
    let (merged, _) = crate::merge::merge_coplanar(&m, eps);
    merged
}

/// Every corner lying part-way along another face's edge goes into that edge too, so neighbouring pieces share whole edges.
/// One pass: corners are only put into rings, never made, so each edge gets all of its in order along it.
fn split_t_junctions(m: &mut Mesh, eps: f64) {
    // corners by a grid of cells, so each edge is tested against nearby corners only
    let cell = {
        let (lo, hi) = m.bounds().unwrap_or(([0.0; 3], [1.0; 3]));
        let span = (0..3).map(|i| hi[i] - lo[i]).fold(0.0f64, f64::max);
        (span / 64.0).max(eps * 4.0)
    };
    let cell_of = |p: V| p.map(|c| (c / cell).floor() as i64);
    let mut grid: std::collections::HashMap<[i64; 3], Vec<u32>> = std::collections::HashMap::new();
    for (v, &p) in m.positions.iter().enumerate() {
        grid.entry(cell_of(p)).or_default().push(v as u32);
    }
    let positions = m.positions.clone();
    for ring in &mut m.faces {
        let mut out = Vec::with_capacity(ring.len());
        for i in 0..ring.len() {
            let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
            out.push(a);
            let (pa, pb) = (positions[a as usize], positions[b as usize]);
            let d = sub(pb, pa);
            let l2 = dot(d, d);
            if l2 <= 0.0 {
                continue;
            }
            let (ca, cb) = (cell_of(pa), cell_of(pb));
            let mut on: Vec<(f64, u32)> = Vec::new();
            let mut test = |v: u32| {
                if v == a || v == b {
                    return;
                }
                let p = positions[v as usize];
                let t = dot(sub(p, pa), d) / l2;
                if t <= 0.0 || t >= 1.0 {
                    return;
                }
                let off = sub(p, lerp(pa, pb, t));
                if dot(off, off).sqrt() <= eps {
                    on.push((t, v));
                }
            };
            let cells: i64 = (0..3).map(|k| (ca[k] - cb[k]).abs() + 1).product();
            if cells as usize > positions.len() {
                // a long edge across many cells: every corner is fewer to look at
                (0..positions.len() as u32).for_each(&mut test);
            } else {
                for x in ca[0].min(cb[0])..=ca[0].max(cb[0]) {
                    for y in ca[1].min(cb[1])..=ca[1].max(cb[1]) {
                        for z in ca[2].min(cb[2])..=ca[2].max(cb[2]) {
                            for &v in grid.get(&[x, y, z]).map(Vec::as_slice).unwrap_or(&[]) {
                                test(v);
                            }
                        }
                    }
                }
            }
            on.sort_by(|x, y| x.0.total_cmp(&y.0));
            on.dedup_by_key(|x| x.1);
            out.extend(on.into_iter().map(|x| x.1));
        }
        *ring = out;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::HalfEdgeMesh;

    fn cuboid(lo: V, hi: V) -> Mesh {
        let mut m = Mesh::new();
        for z in [lo[2], hi[2]] {
            for y in [lo[1], hi[1]] {
                for x in [lo[0], hi[0]] {
                    m.add_vertex([x, y, z]);
                }
            }
        }
        for f in [[0, 2, 3, 1], [4, 5, 7, 6], [0, 1, 5, 4], [2, 6, 7, 3], [0, 4, 6, 2], [1, 3, 7, 5]] {
            m.add_face(f.to_vec());
        }
        m
    }

    fn solid(m: &Mesh) -> HalfEdgeMesh {
        let h = HalfEdgeMesh::from_mesh(m).expect("a good surface");
        assert!(h.is_closed(), "closed");
        h
    }

    #[test]
    fn two_overlapping_boxes() {
        // 2 x 2 x 2 boxes overlapping by 1 x 2 x 2
        let a = cuboid([0.0; 3], [2.0, 2.0, 2.0]);
        let b = cuboid([1.0, 0.0, 0.0], [3.0, 2.0, 2.0]);
        let u = solid(&boolean(&a, &b, Op::Union));
        assert!((u.signed_volume() - 12.0).abs() < 1e-9, "union {}", u.signed_volume());
        assert_eq!(u.face_count(), 6, "one box 3 x 2 x 2, its faces merged whole");
        let s = solid(&boolean(&a, &b, Op::Subtract));
        assert!((s.signed_volume() - 4.0).abs() < 1e-9, "subtract {}", s.signed_volume());
        let i = solid(&boolean(&a, &b, Op::Intersect));
        assert!((i.signed_volume() - 4.0).abs() < 1e-9, "intersect {}", i.signed_volume());
    }

    #[test]
    fn a_box_with_a_hole_drilled_through() {
        // a 4 x 4 x 1 slab, a 1 x 1 bar through it: a slab with a square hole (a solid with a tunnel)
        let slab = cuboid([0.0; 3], [4.0, 4.0, 1.0]);
        let bar = cuboid([1.5, 1.5, -1.0], [2.5, 2.5, 2.0]);
        let r = solid(&boolean(&slab, &bar, Op::Subtract));
        assert!((r.signed_volume() - 15.0).abs() < 1e-9, "{}", r.signed_volume());
        assert_eq!(r.euler_characteristic(), 0, "one hole right through");
    }

    #[test]
    fn a_corner_bite() {
        // a box with a smaller box biting into a corner: an L-shaped notch
        let a = cuboid([0.0; 3], [2.0, 2.0, 2.0]);
        let b = cuboid([1.0, 1.0, 1.0], [3.0, 3.0, 3.0]);
        let s = solid(&boolean(&a, &b, Op::Subtract));
        assert!((s.signed_volume() - 7.0).abs() < 1e-9, "{}", s.signed_volume());
        assert_eq!(s.euler_characteristic(), 2);
        let u = solid(&boolean(&a, &b, Op::Union));
        assert!((u.signed_volume() - 15.0).abs() < 1e-9, "{}", u.signed_volume());
    }

    #[test]
    fn paint_follows_the_pieces() {
        let mut a = cuboid([0.0; 3], [2.0, 2.0, 2.0]);
        for f in 0..6 {
            a.set_color(f, Some([200, 40, 30].into()));
        }
        let b = cuboid([1.0, 1.0, 1.0], [3.0, 3.0, 3.0]);
        let s = boolean(&a, &b, Op::Subtract);
        // the outside is red; the faces of the notch came from b and are unpainted
        assert!((0..s.faces.len()).any(|f| s.color_of(f).is_some()));
        assert!((0..s.faces.len()).any(|f| s.color_of(f).is_none()));
    }

    #[test]
    fn boxes_touching_face_to_face_become_one() {
        let a = cuboid([0.0; 3], [1.0, 1.0, 1.0]);
        let b = cuboid([1.0, 0.0, 0.0], [2.0, 1.0, 1.0]);
        let u = solid(&boolean(&a, &b, Op::Union));
        assert!((u.signed_volume() - 2.0).abs() < 1e-9, "{}", u.signed_volume());
        assert_eq!(u.face_count(), 6, "no wall left between them");
    }

    #[test]
    fn a_round_hole_through_a_box() {
        // a 24-sided prism of radius 0.5 through a 2 x 2 x 1 box
        let mut h = HalfEdgeMesh::default();
        let ring: Vec<V> = (0..24)
            .map(|k| {
                let t = std::f64::consts::TAU * k as f64 / 24.0;
                [1.0 + 0.5 * t.cos(), 1.0 + 0.5 * t.sin(), -0.5]
            })
            .collect();
        h.add_polygon(&ring).unwrap();
        // the prism stands up from the ring (its face may look down: push it whichever way gives a solid upward)
        let n = h.face_normal(0);
        h.push_pull(0, if n[2] > 0.0 { 2.0 } else { -2.0 }).unwrap();
        let rod = h.to_mesh();
        let area = 24.0 * 0.5 * 0.25 * (std::f64::consts::TAU / 24.0).sin();
        let r = solid(&boolean(&cuboid([0.0; 3], [2.0, 2.0, 1.0]), &rod, Op::Subtract));
        assert!((r.signed_volume() - (4.0 - area)).abs() < 1e-9, "{} vs {}", r.signed_volume(), 4.0 - area);
        assert_eq!(r.euler_characteristic(), 0);
    }
}
