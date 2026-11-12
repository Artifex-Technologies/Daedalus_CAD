//! Workplanes and the 2D shapes the drawing tools put on them.
//!
//! Every shape comes back as a ring of world-space points wound counter-clockwise
//! around the plane normal, ready for [`crate::Model::add_polygon`].

/// SketchUp's default number of segments for a circle.
pub const DEFAULT_CIRCLE_SEGMENTS: u32 = 24;

/// SketchUp's default number of segments for an arc (and a pie).
pub const DEFAULT_ARC_SEGMENTS: u32 = 12;

const EPS: f64 = 1e-6;

/// A drawing plane: `origin` plus orthonormal in-plane axes `u`, `v` with
/// `normal = u x v`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Plane {
    pub origin: [f64; 3],
    pub u: [f64; 3],
    pub v: [f64; 3],
    pub normal: [f64; 3],
}

impl Plane {
    /// The ground plane z = 0, axes along X and Y.
    pub fn ground() -> Self {
        Plane { origin: [0.0; 3], u: [1.0, 0.0, 0.0], v: [0.0, 1.0, 0.0], normal: [0.0, 0.0, 1.0] }
    }

    /// Plane through `origin` facing `normal`. On a wall the `u` axis is kept
    /// horizontal, on a floor or ceiling it follows X, so typed sizes read the
    /// same way they do in SketchUp. Returns None for a zero normal.
    pub fn from_point_normal(origin: [f64; 3], normal: [f64; 3]) -> Option<Self> {
        let n = normalize(normal)?;
        let u = if n[2].abs() > 0.999 { normalize(sub([1.0, 0.0, 0.0], scale(n, n[0])))? } else { normalize(cross([0.0, 0.0, 1.0], n))? };
        Some(Plane { origin, u, v: cross(n, u), normal: n })
    }

    /// Where a ray meets the plane, from either side; None if parallel or behind.
    pub fn intersect_ray(&self, origin: [f64; 3], dir: [f64; 3]) -> Option<[f64; 3]> {
        let denom = dot(self.normal, dir);
        if denom.abs() < EPS {
            return None;
        }
        let t = dot(self.normal, sub(self.origin, origin)) / denom;
        (t > 0.0).then(|| add(origin, scale(dir, t)))
    }

    /// In-plane coordinates of a point (its offset along the normal is dropped).
    pub fn to_local(&self, p: [f64; 3]) -> [f64; 2] {
        let d = sub(p, self.origin);
        [dot(d, self.u), dot(d, self.v)]
    }

    pub fn to_world(&self, x: f64, y: f64) -> [f64; 3] {
        add(self.origin, add(scale(self.u, x), scale(self.v, y)))
    }
}

/// Rectangle with opposite corners `a` and `b`, edges along the plane axes.
/// None if it has no width or height.
pub fn rectangle(plane: &Plane, a: [f64; 3], b: [f64; 3]) -> Option<Vec<[f64; 3]>> {
    let (pa, pb) = (plane.to_local(a), plane.to_local(b));
    let (x0, x1) = (pa[0].min(pb[0]), pa[0].max(pb[0]));
    let (y0, y1) = (pa[1].min(pb[1]), pa[1].max(pb[1]));
    if x1 - x0 < EPS || y1 - y0 < EPS {
        return None;
    }
    Some(vec![plane.to_world(x0, y0), plane.to_world(x1, y0), plane.to_world(x1, y1), plane.to_world(x0, y1)])
}

/// Regular polygon centred at `center` with its first corner towards `rim`
/// (SketchUp's Polygon tool; with [`DEFAULT_CIRCLE_SEGMENTS`] it is the Circle tool).
/// None for fewer than 3 sides or a zero radius.
pub fn regular_polygon(plane: &Plane, center: [f64; 3], rim: [f64; 3], sides: u32) -> Option<Vec<[f64; 3]>> {
    let c = plane.to_local(center);
    let r = plane.to_local(rim);
    let (dx, dy) = (r[0] - c[0], r[1] - c[1]);
    let radius = (dx * dx + dy * dy).sqrt();
    if sides < 3 || radius < EPS {
        return None;
    }
    let start = dy.atan2(dx);
    let step = std::f64::consts::TAU / sides as f64;
    Some(
        (0..sides)
            .map(|i| {
                let a = start + step * i as f64;
                plane.to_world(c[0] + radius * a.cos(), c[1] + radius * a.sin())
            })
            .collect(),
    )
}

/// Rotated Rectangle: the first side from `a` to `b` (any direction in the plane), the opposite side through `c`'s
/// distance from that line, on `c`'s side. None if a side has no length.
pub fn rotated_rectangle(plane: &Plane, a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> Option<Vec<[f64; 3]>> {
    let (la, lb, lc) = (plane.to_local(a), plane.to_local(b), plane.to_local(c));
    let (dx, dy) = (lb[0] - la[0], lb[1] - la[1]);
    let len = (dx * dx + dy * dy).sqrt();
    if len < EPS {
        return None;
    }
    // the side's left-hand normal in the plane, and how far c lies along it
    let (nx, ny) = (-dy / len, dx / len);
    let h = (lc[0] - la[0]) * nx + (lc[1] - la[1]) * ny;
    if h.abs() < EPS {
        return None;
    }
    let w = |x: f64, y: f64| plane.to_world(x, y);
    let pts = vec![w(la[0], la[1]), w(lb[0], lb[1]), w(lb[0] + nx * h, lb[1] + ny * h), w(la[0] + nx * h, la[1] + ny * h)];
    // counter-clockwise around the normal: reversed when c is on the right of a -> b
    Some(if h > 0.0 { pts } else { pts.into_iter().rev().collect() })
}

/// The angle (radians, counter-clockwise about the plane normal) from the direction `center -> from` to `center -> to`,
/// in (-pi, pi]. None when either point is on the centre.
pub fn angle_between(plane: &Plane, center: [f64; 3], from: [f64; 3], to: [f64; 3]) -> Option<f64> {
    let (c, f, t) = (plane.to_local(center), plane.to_local(from), plane.to_local(to));
    let (a, b) = ([f[0] - c[0], f[1] - c[1]], [t[0] - c[0], t[1] - c[1]]);
    if a[0].hypot(a[1]) < EPS || b[0].hypot(b[1]) < EPS {
        return None;
    }
    Some((a[0] * b[1] - a[1] * b[0]).atan2(a[0] * b[0] + a[1] * b[1]))
}

/// Pie: the slice of a circle around `center`, starting at `start` (which sets the radius) and turning `sweep` radians
/// counter-clockwise about the plane normal (negative: clockwise), in `segments` straight pieces. None for no radius or
/// no sweep.
pub fn pie(plane: &Plane, center: [f64; 3], start: [f64; 3], sweep: f64, segments: u32) -> Option<Vec<[f64; 3]>> {
    let (c, s) = (plane.to_local(center), plane.to_local(start));
    let radius = (s[0] - c[0]).hypot(s[1] - c[1]);
    if radius < EPS || sweep.abs() < 1e-4 || segments == 0 {
        return None;
    }
    let sweep = sweep.clamp(-std::f64::consts::TAU, std::f64::consts::TAU);
    let a0 = (s[1] - c[1]).atan2(s[0] - c[0]);
    let mut pts = vec![center];
    pts.extend((0..=segments).map(|i| {
        let a = a0 + sweep * i as f64 / segments as f64;
        plane.to_world(c[0] + radius * a.cos(), c[1] + radius * a.sin())
    }));
    // a full turn ends on its start: no zero-length side
    if (sweep.abs() - std::f64::consts::TAU).abs() < 1e-4 {
        pts.pop();
    }
    Some(if sweep > 0.0 { pts } else { pts.into_iter().rev().collect() })
}

/// 2 Point Arc: from `a` to `b`, bowed out by `bulge` (the height of its middle above the chord, measured along the plane's
/// normal turned onto the chord's left; negative bows the other way), in `segments` straight pieces. Points from `a` to
/// `b`. None for a zero chord; a zero bulge is the straight line.
pub fn arc_by_bulge(plane: &Plane, a: [f64; 3], b: [f64; 3], bulge: f64, segments: u32) -> Option<Vec<[f64; 3]>> {
    let (la, lb) = (plane.to_local(a), plane.to_local(b));
    let (dx, dy) = (lb[0] - la[0], lb[1] - la[1]);
    let chord = dx.hypot(dy);
    if chord < EPS || segments == 0 {
        return None;
    }
    if bulge.abs() < EPS {
        return Some(vec![a, b]);
    }
    let half = chord / 2.0;
    // radius from the chord and the bulge; the centre sits on the chord's perpendicular through its middle
    let r = (half * half + bulge * bulge) / (2.0 * bulge.abs());
    let (nx, ny) = (-dy / chord, dx / chord); // left of a -> b
    let side = bulge.signum();
    let (mx, my) = (la[0] + dx / 2.0, la[1] + dy / 2.0);
    let off = r - bulge.abs(); // from the middle of the chord to the centre, away from the bulge
    let (cx, cy) = (mx - nx * side * off, my - ny * side * off);
    let a0 = (la[1] - cy).atan2(la[0] - cx);
    let a1 = (lb[1] - cy).atan2(lb[0] - cx);
    // the sweep that passes through the bulge's side
    let mut sweep = a1 - a0;
    let tau = std::f64::consts::TAU;
    let want_ccw = side < 0.0; // bulging left of a -> b goes clockwise round the centre
    if want_ccw && sweep < 0.0 {
        sweep += tau;
    }
    if !want_ccw && sweep > 0.0 {
        sweep -= tau;
    }
    Some(
        (0..=segments)
            .map(|i| {
                let t = a0 + sweep * i as f64 / segments as f64;
                plane.to_world(cx + r * t.cos(), cy + r * t.sin())
            })
            .collect(),
    )
}

/// 3 Point Arc: the arc from `a` through `m` to `b`, in `segments` pieces. None when the three points are in a line.
pub fn arc_three_points(plane: &Plane, a: [f64; 3], m: [f64; 3], b: [f64; 3], segments: u32) -> Option<Vec<[f64; 3]>> {
    let (p, q, r) = (plane.to_local(a), plane.to_local(m), plane.to_local(b));
    let d = 2.0 * (p[0] * (q[1] - r[1]) + q[0] * (r[1] - p[1]) + r[0] * (p[1] - q[1]));
    if d.abs() < EPS || segments == 0 {
        return None;
    }
    let sq = |v: [f64; 2]| v[0] * v[0] + v[1] * v[1];
    let cx = (sq(p) * (q[1] - r[1]) + sq(q) * (r[1] - p[1]) + sq(r) * (p[1] - q[1])) / d;
    let cy = (sq(p) * (r[0] - q[0]) + sq(q) * (p[0] - r[0]) + sq(r) * (q[0] - p[0])) / d;
    let rad = (p[0] - cx).hypot(p[1] - cy);
    let ang = |v: [f64; 2]| (v[1] - cy).atan2(v[0] - cx);
    let tau = std::f64::consts::TAU;
    let (a0, am, a1) = (ang(p), ang(q), ang(r));
    // counter-clockwise from a: does it meet m before b? if not, go the other way round
    let ccw = |x: f64| (x - a0).rem_euclid(tau);
    let sweep = if ccw(am) <= ccw(a1) { ccw(a1) } else { ccw(a1) - tau };
    Some(
        (0..=segments)
            .map(|i| {
                let t = a0 + sweep * i as f64 / segments as f64;
                plane.to_world(cx + rad * t.cos(), cy + rad * t.sin())
            })
            .collect(),
    )
}

/// Arc (by its centre): around `center` from `start` (which sets the radius), turning `sweep` radians counter-clockwise
/// about the normal. Points along the arc only (a pie without its two radii).
pub fn arc_from_center(plane: &Plane, center: [f64; 3], start: [f64; 3], sweep: f64, segments: u32) -> Option<Vec<[f64; 3]>> {
    let mut v = pie(plane, center, start, sweep, segments)?;
    // pie() puts the centre first (or last when it turned the ring round)
    if sweep > 0.0 {
        v.remove(0);
    } else {
        v.pop();
        v.reverse();
    }
    Some(v)
}

/// Offset: the outline `ring` (counter-clockwise about `normal`) moved `d` inward (negative: outward), each corner where its
/// two moved sides meet. None when it collapses (a side would turn round) or the outline has no area.
pub fn offset_ring(ring: &[[f64; 3]], normal: [f64; 3], d: f64) -> Option<Vec<[f64; 3]>> {
    let n = ring.len();
    if n < 3 {
        return None;
    }
    let plane = Plane::from_point_normal(ring[0], normal)?;
    let pts: Vec<[f64; 2]> = ring.iter().map(|&p| plane.to_local(p)).collect();
    // the inward side of each edge (left of a counter-clockwise ring)
    let side = |i: usize| -> Option<([f64; 2], [f64; 2])> {
        let (a, b) = (pts[i], pts[(i + 1) % n]);
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let l = dx.hypot(dy);
        (l > EPS).then(|| ([a[0] - dy / l * d, a[1] + dx / l * d], [dx / l, dy / l]))
    };
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let (p1, d1) = side((i + n - 1) % n)?;
        let (p2, d2) = side(i)?;
        let den = d1[0] * d2[1] - d1[1] * d2[0];
        let q = if den.abs() < 1e-6 {
            // straight on: the corner just moves in
            p2
        } else {
            let t = ((p2[0] - p1[0]) * d2[1] - (p2[1] - p1[1]) * d2[0]) / den;
            [p1[0] + d1[0] * t, p1[1] + d1[1] * t]
        };
        out.push(q);
    }
    // collapsed: a side that now runs backwards
    for i in 0..n {
        let (a, b) = (pts[i], pts[(i + 1) % n]);
        let (c, e) = (out[i], out[(i + 1) % n]);
        if (b[0] - a[0]) * (e[0] - c[0]) + (b[1] - a[1]) * (e[1] - c[1]) <= 0.0 {
            return None;
        }
    }
    Some(out.into_iter().map(|q| plane.to_world(q[0], q[1])).collect())
}

fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scale(a: [f64; 3], s: f64) -> [f64; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn normalize(a: [f64; 3]) -> Option<[f64; 3]> {
    let len = dot(a, a).sqrt();
    (len > EPS).then(|| scale(a, 1.0 / len))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::HalfEdgeMesh;

    fn close(a: [f64; 3], b: [f64; 3]) -> bool {
        (0..3).all(|i| (a[i] - b[i]).abs() < 1e-5)
    }

    /// Area and normal of a ring, via a one-face half-edge mesh.
    fn face_of(points: &[[f64; 3]]) -> HalfEdgeMesh {
        let mut h = HalfEdgeMesh::default();
        h.add_polygon(points).unwrap();
        h
    }

    #[test]
    fn rectangle_faces_up_whichever_way_it_is_dragged() {
        let g = Plane::ground();
        for (a, b) in [([0.0, 0.0, 0.0], [3.0, 2.0, 0.0]), ([3.0, 2.0, 0.0], [0.0, 0.0, 0.0]), ([3.0, 0.0, 0.0], [0.0, 2.0, 0.0])] {
            let r = rectangle(&g, a, b).unwrap();
            assert!(close(r[0], [0.0, 0.0, 0.0]) && close(r[2], [3.0, 2.0, 0.0]));
            let h = face_of(&r);
            assert!(close(h.face_normal(0), [0.0, 0.0, 1.0]));
        }
        assert!(rectangle(&g, [0.0; 3], [3.0, 0.0, 0.0]).is_none(), "zero height");
    }

    #[test]
    fn rectangle_pulled_up_has_width_times_depth_times_height() {
        let r = rectangle(&Plane::ground(), [1.0, 1.0, 0.0], [4.0, 3.0, 0.0]).unwrap();
        let mut h = face_of(&r);
        h.push_pull(0, 2.5).unwrap();
        assert!(h.is_closed());
        assert_eq!(h.euler_characteristic(), 2);
        assert!((h.signed_volume() - 3.0 * 2.0 * 2.5).abs() < 1e-4);
    }

    #[test]
    fn plane_on_a_wall_keeps_u_horizontal_and_faces_out() {
        let p = Plane::from_point_normal([0.0, 5.0, 0.0], [0.0, 2.0, 0.0]).unwrap();
        assert!(close(p.normal, [0.0, 1.0, 0.0]));
        assert!(p.u[2].abs() < 1e-6);
        assert!(close(cross(p.u, p.v), p.normal));
        let r = rectangle(&p, p.to_world(0.0, 0.0), p.to_world(2.0, 1.0)).unwrap();
        assert!(close(face_of(&r).face_normal(0), [0.0, 1.0, 0.0]));
        assert!(Plane::from_point_normal([0.0; 3], [0.0; 3]).is_none());
    }

    #[test]
    fn ray_hits_plane_from_above_and_below() {
        let g = Plane::ground();
        assert!(close(g.intersect_ray([1.0, 2.0, 5.0], [0.0, 0.0, -1.0]).unwrap(), [1.0, 2.0, 0.0]));
        assert!(close(g.intersect_ray([1.0, 2.0, -5.0], [0.0, 0.0, 1.0]).unwrap(), [1.0, 2.0, 0.0]));
        assert!(g.intersect_ray([0.0, 0.0, 5.0], [0.0, 0.0, 1.0]).is_none(), "pointing away");
        assert!(g.intersect_ray([0.0, 0.0, 5.0], [1.0, 0.0, 0.0]).is_none(), "parallel");
    }

    #[test]
    fn circle_has_24_segments_and_approaches_pi_r_squared() {
        let c = regular_polygon(&Plane::ground(), [0.0; 3], [2.0, 0.0, 0.0], DEFAULT_CIRCLE_SEGMENTS).unwrap();
        assert_eq!(c.len(), 24);
        assert!(close(c[0], [2.0, 0.0, 0.0]));
        assert!(c.iter().all(|p| ((p[0] * p[0] + p[1] * p[1]).sqrt() - 2.0).abs() < 1e-5));
        // Inscribed 24-gon: area = n/2 r^2 sin(2 pi / n).
        let mut h = face_of(&c);
        let expected = 12.0 * 4.0 * (std::f64::consts::TAU / 24.0).sin();
        h.push_pull(0, 1.0).unwrap();
        assert!((h.signed_volume() - expected).abs() < 1e-3);
        assert!(close(h.face_normal(h.face_count() - 1), [0.0, 0.0, 1.0]));
    }

    #[test]
    fn rotated_rectangle_faces_up_on_either_side() {
        let g = Plane::ground();
        let (a, b) = ([0.0, 0.0, 0.0], [3.0, 4.0, 0.0]); // a 5 m side at an angle
        for c in [[-4.0, 3.0, 0.0], [4.0, -3.0, 0.0]] {
            let r = rotated_rectangle(&g, a, b, c).unwrap();
            let mut h = face_of(&r);
            assert!(close(h.face_normal(0), [0.0, 0.0, 1.0]));
            h.push_pull(0, 1.0).unwrap();
            assert!((h.signed_volume() - 5.0 * 5.0).abs() < 1e-3, "5 x 5 x 1");
        }
        assert!(rotated_rectangle(&g, a, b, [6.0, 8.0, 0.0]).is_none(), "c on the first side's line");
    }

    #[test]
    fn pie_is_a_slice_with_the_right_area() {
        let g = Plane::ground();
        // a quarter of a 2 m circle, either way round
        for sweep in [std::f64::consts::FRAC_PI_2, -std::f64::consts::FRAC_PI_2] {
            let p = pie(&g, [0.0; 3], [2.0, 0.0, 0.0], sweep, 12).unwrap();
            assert_eq!(p.len(), 14, "centre plus 13 points on the arc");
            let mut h = face_of(&p);
            assert!(close(h.face_normal(0), [0.0, 0.0, 1.0]));
            h.push_pull(0, 1.0).unwrap();
            // 12 triangles of the slice: 12 * 1/2 r^2 sin(step)
            let expected = 12.0 * 0.5 * 4.0 * (std::f64::consts::FRAC_PI_2 / 12.0).sin();
            assert!((h.signed_volume() - expected).abs() < 1e-3);
        }
        assert!(pie(&g, [0.0; 3], [0.0; 3], 1.0, 12).is_none());
        let a = angle_between(&g, [0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]).unwrap();
        assert!((a - std::f64::consts::FRAC_PI_2).abs() < 1e-6);
    }

    #[test]
    fn arcs_pass_through_their_points() {
        let g = Plane::ground();
        // a half circle over a 2 m chord: bulge 1 gives radius 1
        let arc = arc_by_bulge(&g, [0.0, 0.0, 0.0], [2.0, 0.0, 0.0], 1.0, 12).unwrap();
        assert_eq!(arc.len(), 13);
        assert!(close(arc[0], [0.0; 3]) && close(arc[12], [2.0, 0.0, 0.0]));
        assert!(arc.iter().all(|p| ((p[0] - 1.0).hypot(p[1]) - 1.0).abs() < 1e-5));
        assert!(close(arc[6], [1.0, 1.0, 0.0]), "bulging to the left of a -> b: {:?}", arc[6]);
        let other = arc_by_bulge(&g, [0.0, 0.0, 0.0], [2.0, 0.0, 0.0], -0.5, 12).unwrap();
        assert!((other[6][1] + 0.5).abs() < 1e-5, "a shallow arc the other way: {:?}", other[6]);
        // 3 points: through the middle one
        let arc = arc_three_points(&g, [0.0, 0.0, 0.0], [1.0, -1.0, 0.0], [2.0, 0.0, 0.0], 12).unwrap();
        assert!(close(arc[6], [1.0, -1.0, 0.0]), "{:?}", arc[6]);
        assert!(close(arc[12], [2.0, 0.0, 0.0]));
        assert!(arc_three_points(&g, [0.0; 3], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0], 12).is_none(), "in a line");
        // by its centre: a quarter
        let q = arc_from_center(&g, [0.0; 3], [1.0, 0.0, 0.0], std::f64::consts::FRAC_PI_2, 6).unwrap();
        assert_eq!(q.len(), 7);
        assert!(close(q[0], [1.0, 0.0, 0.0]) && close(q[6], [0.0, 1.0, 0.0]));
    }

    #[test]
    fn offset_moves_every_side_in() {
        let sq = [[0.0, 0.0, 0.0], [4.0, 0.0, 0.0], [4.0, 4.0, 0.0], [0.0, 4.0, 0.0]];
        let inner = offset_ring(&sq, [0.0, 0.0, 1.0], 1.0).unwrap();
        for (p, q) in inner.iter().zip([[1.0, 1.0, 0.0], [3.0, 1.0, 0.0], [3.0, 3.0, 0.0], [1.0, 3.0, 0.0]]) {
            assert!(close(*p, q), "{p:?} vs {q:?}");
        }
        let outer = offset_ring(&sq, [0.0, 0.0, 1.0], -1.0).unwrap();
        assert!(close(outer[0], [-1.0, -1.0, 0.0]));
        assert!(offset_ring(&sq, [0.0, 0.0, 1.0], 2.5).is_none(), "past the middle it collapses");
        // an L: the inside corner moves the right way too
        let l = [[0.0, 0.0, 0.0], [4.0, 0.0, 0.0], [4.0, 2.0, 0.0], [2.0, 2.0, 0.0], [2.0, 4.0, 0.0], [0.0, 4.0, 0.0]];
        let inner = offset_ring(&l, [0.0, 0.0, 1.0], 0.5).unwrap();
        assert!(close(inner[3], [1.5, 1.5, 0.0]), "{:?}", inner[3]);
    }

    #[test]
    fn polygon_rejects_bad_input() {
        let g = Plane::ground();
        assert!(regular_polygon(&g, [0.0; 3], [1.0, 0.0, 0.0], 2).is_none());
        assert!(regular_polygon(&g, [1.0; 3], [1.0, 1.0, 0.0], 6).is_none(), "zero radius in plane");
        let hex = regular_polygon(&g, [0.0; 3], [0.0, 1.0, 0.0], 6).unwrap();
        assert!(close(hex[0], [0.0, 1.0, 0.0]), "first corner points at the rim");
    }
}
