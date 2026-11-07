//! Select tool drag, as in SketchUp: dragged left to right, a window takes what lies wholly inside it; dragged right to
//! left, a crossing box (drawn dashed) takes everything it touches. Works on screen positions, so it needs no GPU: the
//! viewer hands over its display mesh and a projection.

use daedalus_core::Mesh;

/// A screen rectangle (pixels), from the press to the release in either order.
#[derive(Clone, Copy, Debug)]
pub struct Rect {
    pub min: (f32, f32),
    pub max: (f32, f32),
}

impl Rect {
    pub fn new(a: (f32, f32), b: (f32, f32)) -> Rect {
        Rect { min: (a.0.min(b.0), a.1.min(b.1)), max: (a.0.max(b.0), a.1.max(b.1)) }
    }

    fn contains(&self, p: (f32, f32)) -> bool {
        p.0 >= self.min.0 && p.0 <= self.max.0 && p.1 >= self.min.1 && p.1 <= self.max.1
    }

    fn corners(&self) -> [(f32, f32); 4] {
        [self.min, (self.max.0, self.min.1), self.max, (self.min.0, self.max.1)]
    }

    /// Whether segment ab touches the rectangle (an end inside, or crossing a side).
    fn touches_segment(&self, a: (f32, f32), b: (f32, f32)) -> bool {
        if self.contains(a) || self.contains(b) {
            return true;
        }
        let c = self.corners();
        (0..4).any(|i| segments_cross(a, b, c[i], c[(i + 1) % 4]))
    }
}

/// A drag from `from` to `to` makes a crossing box when it goes right to left.
pub fn is_crossing(from: (f32, f32), to: (f32, f32)) -> bool {
    to.0 < from.0
}

/// What the box takes.
#[derive(Debug, Default, PartialEq)]
pub struct Picked {
    /// Faces of the geometry being edited.
    pub faces: Vec<usize>,
    /// Edges and loose lines of the geometry being edited (lower vertex first).
    pub edges: Vec<[u32; 2]>,
    /// Groups and components of the open context.
    pub instances: Vec<usize>,
}

/// The display as the viewer keeps it: faces below `active_faces` (and vertices below `active_vertices`) are the geometry
/// being edited; faces from there up to `active_faces + owners.len()` belong to the instance `owners` names; the rest lie
/// outside the open group and cannot be taken.
pub struct Display<'a> {
    pub mesh: &'a Mesh,
    pub edges: &'a [(u32, u32)],
    pub active_faces: usize,
    pub active_vertices: u32,
    pub owners: &'a [usize],
}

/// Everything the box takes, given where each point lands on screen (None: behind the eye).
pub fn pick(d: &Display, project: impl Fn([f64; 3]) -> Option<(f32, f32)>, r: Rect, crossing: bool) -> Picked {
    let n = d.mesh.positions.len();
    let screen: Vec<Option<(f32, f32)>> = d.mesh.positions.iter().map(|&p| project(p)).collect();
    let at = |i: u32| screen.get(i as usize).copied().flatten();
    let mut out = Picked::default();
    // an instance is taken when all its faces are inside (window) or any one touches (crossing); `None` once one fails
    let mut inst: Vec<(usize, bool)> = Vec::new();
    let last = (d.active_faces + d.owners.len()).min(d.mesh.faces.len());
    for f in 0..last {
        let loops: Vec<Vec<Option<(f32, f32)>>> = d.mesh.loops(f).map(|lp| lp.iter().map(|&i| at(i)).collect()).collect();
        let hit = if crossing { face_touches(&loops, r) } else { loops.iter().flatten().all(|p| p.is_some_and(|p| r.contains(p))) };
        if f < d.active_faces {
            if hit {
                out.faces.push(f);
            }
            continue;
        }
        let owner = d.owners[f - d.active_faces];
        match inst.iter_mut().find(|(o, _)| *o == owner) {
            Some((_, h)) => *h = if crossing { *h || hit } else { *h && hit },
            None => inst.push((owner, hit)),
        }
    }
    out.instances = inst.into_iter().filter(|&(_, h)| h).map(|(o, _)| o).collect();
    out.instances.sort_unstable();
    out.instances.dedup();
    for &(a, b) in d.edges {
        if a >= d.active_vertices || b >= d.active_vertices || a as usize >= n || b as usize >= n {
            continue;
        }
        let (Some(pa), Some(pb)) = (at(a), at(b)) else { continue };
        let hit = if crossing { r.touches_segment(pa, pb) } else { r.contains(pa) && r.contains(pb) };
        if hit {
            out.edges.push([a.min(b), a.max(b)]);
        }
    }
    out.edges.sort_unstable();
    out.edges.dedup();
    out
}

/// A face (outline, then holes, on screen) touches the box: a corner or side of it does, or the box lies on the face (in
/// its outline and not in a hole).
fn face_touches(loops: &[Vec<Option<(f32, f32)>>], r: Rect) -> bool {
    for lp in loops {
        for i in 0..lp.len() {
            match (lp[i], lp[(i + 1) % lp.len()]) {
                (Some(a), Some(b)) if r.touches_segment(a, b) => return true,
                (Some(a), _) if r.contains(a) => return true,
                _ => {}
            }
        }
    }
    let Some((outline, holes)) = loops.split_first() else { return false };
    let flat = |lp: &Vec<Option<(f32, f32)>>| lp.iter().copied().collect::<Option<Vec<_>>>();
    let Some(outline) = flat(outline) else { return false };
    let c = r.min;
    inside(&outline, c) && !holes.iter().filter_map(flat).any(|h| inside(&h, c))
}

/// Even-odd point in polygon.
fn inside(poly: &[(f32, f32)], p: (f32, f32)) -> bool {
    let mut odd = false;
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        if (a.1 > p.1) != (b.1 > p.1) && p.0 < a.0 + (p.1 - a.1) / (b.1 - a.1) * (b.0 - a.0) {
            odd = !odd;
        }
    }
    odd
}

fn segments_cross(a: (f32, f32), b: (f32, f32), c: (f32, f32), d: (f32, f32)) -> bool {
    let side = |p: (f32, f32), q: (f32, f32), r: (f32, f32)| (q.0 - p.0) * (r.1 - p.1) - (q.1 - p.1) * (r.0 - p.0);
    let (d1, d2) = (side(c, d, a), side(c, d, b));
    let (d3, d4) = (side(a, b, c), side(a, b, d));
    ((d1 > 0.0) != (d2 > 0.0)) && ((d3 > 0.0) != (d4 > 0.0)) && d1 != 0.0 && d2 != 0.0 && d3 != 0.0 && d4 != 0.0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Looking straight down: x and y are the screen, 100 px a metre, y down the screen as the model's y goes up.
    fn top(p: [f64; 3]) -> Option<(f32, f32)> {
        Some(((p[0] * 100.0) as f32, (-p[1] * 100.0) as f32))
    }

    fn square(m: &mut Mesh, x: f64, y: f64, s: f64) -> usize {
        let v = [[x, y, 0.0], [x + s, y, 0.0], [x + s, y + s, 0.0], [x, y + s, 0.0]].map(|p| m.add_vertex(p));
        m.add_face(v.to_vec());
        m.faces.len() - 1
    }

    fn edges_of(m: &Mesh) -> Vec<(u32, u32)> {
        let mut e = Vec::new();
        for f in 0..m.faces.len() {
            for lp in m.loops(f) {
                for i in 0..lp.len() {
                    e.push((lp[i], lp[(i + 1) % lp.len()]));
                }
            }
        }
        e
    }

    #[test]
    fn a_window_takes_what_is_wholly_inside_and_a_crossing_box_what_it_touches() {
        // two squares, one 1 m at the origin, one 1 m at x = 3; a box round the first and half over the second
        let mut m = Mesh::new();
        let a = square(&mut m, 0.0, 0.0, 1.0);
        let b = square(&mut m, 3.0, 0.0, 1.0);
        let edges = edges_of(&m);
        let d = Display { mesh: &m, edges: &edges, active_faces: 2, active_vertices: m.positions.len() as u32, owners: &[] };
        let r = Rect::new((-10.0, 10.0), (350.0, -110.0));
        let w = pick(&d, top, r, false);
        assert_eq!(w.faces, vec![a]);
        assert_eq!(w.edges.len(), 4 + 1, "the first square's four edges, and the second's left edge (x = 3, inside)");
        let c = pick(&d, top, r, true);
        assert_eq!(c.faces, vec![a, b]);
        assert_eq!(c.edges.len(), 4 + 3, "the second square's right edge is not touched");
        // a small box in the middle of a face: crossing takes the face, a window takes nothing
        let small = Rect::new((40.0, -40.0), (60.0, -60.0));
        assert_eq!(pick(&d, top, small, true).faces, vec![a]);
        assert_eq!(pick(&d, top, small, false), Picked::default());
    }

    #[test]
    fn a_crossing_box_in_a_hole_does_not_take_the_face_round_it() {
        let mut m = Mesh::new();
        let f = square(&mut m, 0.0, 0.0, 3.0);
        let h = [[1.0, 1.0, 0.0], [1.0, 2.0, 0.0], [2.0, 2.0, 0.0], [2.0, 1.0, 0.0]].map(|p| m.add_vertex(p));
        m.holes = vec![vec![h.to_vec()]];
        let d = Display { mesh: &m, edges: &[], active_faces: 1, active_vertices: m.positions.len() as u32, owners: &[] };
        assert!(pick(&d, top, Rect::new((140.0, -140.0), (160.0, -160.0)), true).faces.is_empty());
        assert_eq!(pick(&d, top, Rect::new((40.0, -40.0), (60.0, -60.0)), true).faces, vec![f]);
    }

    #[test]
    fn a_group_goes_in_whole_or_not_at_all() {
        // the open context has nothing of its own; faces 0 and 1 belong to instance 4, face 2 to instance 7
        let mut m = Mesh::new();
        square(&mut m, 0.0, 0.0, 1.0);
        square(&mut m, 1.5, 0.0, 1.0);
        square(&mut m, 5.0, 0.0, 1.0);
        let owners = [4, 4, 7];
        let d = Display { mesh: &m, edges: &[], active_faces: 0, active_vertices: 0, owners: &owners };
        // round the first square only: a window leaves the group out, a crossing box takes it
        let r = Rect::new((-10.0, 10.0), (120.0, -110.0));
        assert!(pick(&d, top, r, false).instances.is_empty());
        assert_eq!(pick(&d, top, r, true).instances, vec![4]);
        // round both of its squares: the window takes it, still not the other group
        let r = Rect::new((-10.0, 10.0), (300.0, -110.0));
        assert_eq!(pick(&d, top, r, false).instances, vec![4]);
        // behind the eye counts as outside a window
        assert!(pick(&d, |_| None, r, false).instances.is_empty());
    }

    #[test]
    fn the_drag_direction_decides() {
        assert!(!is_crossing((10.0, 10.0), (90.0, 50.0)));
        assert!(is_crossing((90.0, 10.0), (10.0, 50.0)));
    }
}
