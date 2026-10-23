//! Workplanes and the 2D shapes the drawing tools put on them.
//!
//! Every shape comes back as a ring of world-space points wound counter-clockwise
//! around the plane normal, ready for [`crate::Model::add_polygon`].

/// SketchUp's default number of segments for a circle.
pub const DEFAULT_CIRCLE_SEGMENTS: u32 = 24;

const EPS: f32 = 1e-6;

/// A drawing plane: `origin` plus orthonormal in-plane axes `u`, `v` with
/// `normal = u x v`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Plane {
    pub origin: [f32; 3],
    pub u: [f32; 3],
    pub v: [f32; 3],
    pub normal: [f32; 3],
}

impl Plane {
    /// The ground plane z = 0, axes along X and Y.
    pub fn ground() -> Self {
        Plane { origin: [0.0; 3], u: [1.0, 0.0, 0.0], v: [0.0, 1.0, 0.0], normal: [0.0, 0.0, 1.0] }
    }

    /// Plane through `origin` facing `normal`. On a wall the `u` axis is kept
    /// horizontal, on a floor or ceiling it follows X, so typed sizes read the
    /// same way they do in SketchUp. Returns None for a zero normal.
    pub fn from_point_normal(origin: [f32; 3], normal: [f32; 3]) -> Option<Self> {
        let n = normalize(normal)?;
        let u = if n[2].abs() > 0.999 {
            normalize(sub([1.0, 0.0, 0.0], scale(n, n[0])))?
        } else {
            normalize(cross([0.0, 0.0, 1.0], n))?
        };
        Some(Plane { origin, u, v: cross(n, u), normal: n })
    }

    /// Where a ray meets the plane, from either side; None if parallel or behind.
    pub fn intersect_ray(&self, origin: [f32; 3], dir: [f32; 3]) -> Option<[f32; 3]> {
        let denom = dot(self.normal, dir);
        if denom.abs() < EPS {
            return None;
        }
        let t = dot(self.normal, sub(self.origin, origin)) / denom;
        (t > 0.0).then(|| add(origin, scale(dir, t)))
    }

    /// In-plane coordinates of a point (its offset along the normal is dropped).
    pub fn to_local(&self, p: [f32; 3]) -> [f32; 2] {
        let d = sub(p, self.origin);
        [dot(d, self.u), dot(d, self.v)]
    }

    pub fn to_world(&self, x: f32, y: f32) -> [f32; 3] {
        add(self.origin, add(scale(self.u, x), scale(self.v, y)))
    }
}

/// Rectangle with opposite corners `a` and `b`, edges along the plane axes.
/// None if it has no width or height.
pub fn rectangle(plane: &Plane, a: [f32; 3], b: [f32; 3]) -> Option<Vec<[f32; 3]>> {
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
pub fn regular_polygon(plane: &Plane, center: [f32; 3], rim: [f32; 3], sides: u32) -> Option<Vec<[f32; 3]>> {
    let c = plane.to_local(center);
    let r = plane.to_local(rim);
    let (dx, dy) = (r[0] - c[0], r[1] - c[1]);
    let radius = (dx * dx + dy * dy).sqrt();
    if sides < 3 || radius < EPS {
        return None;
    }
    let start = dy.atan2(dx);
    let step = std::f32::consts::TAU / sides as f32;
    Some(
        (0..sides)
            .map(|i| {
                let a = start + step * i as f32;
                plane.to_world(c[0] + radius * a.cos(), c[1] + radius * a.sin())
            })
            .collect(),
    )
}

fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scale(a: [f32; 3], s: f32) -> [f32; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn normalize(a: [f32; 3]) -> Option<[f32; 3]> {
    let len = dot(a, a).sqrt();
    (len > EPS).then(|| scale(a, 1.0 / len))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::HalfEdgeMesh;

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        (0..3).all(|i| (a[i] - b[i]).abs() < 1e-5)
    }

    /// Area and normal of a ring, via a one-face half-edge mesh.
    fn face_of(points: &[[f32; 3]]) -> HalfEdgeMesh {
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
        let expected = 12.0 * 4.0 * (std::f32::consts::TAU / 24.0).sin();
        h.push_pull(0, 1.0).unwrap();
        assert!((h.signed_volume() - expected).abs() < 1e-3);
        assert!(close(h.face_normal(h.face_count() - 1), [0.0, 0.0, 1.0]));
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
