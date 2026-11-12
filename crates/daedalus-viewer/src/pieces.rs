//! The groups and components as drawn, kept piece by piece between edits.
//!
//! A part (an instance of the open context, or what lies outside it) is drawn as its pieces one after the other: each
//! definition's own geometry where it is placed (see `Scene::part_pieces`). A piece placed is split into triangles and edges
//! once, and kept while its geometry (compared bit for bit) and its place stay the same; so an edit splits again only the
//! pieces it changed, not every group and component in the model. What is drawn is exactly what splitting everything again
//! would give: a face's triangles and edges depend on that face alone, and the meshes are the same ones `Model::parts` makes.

use daedalus_core::scene::{Piece, Xform};
use daedalus_core::{Mesh, Paint};
use std::collections::HashMap;
use std::rc::Rc;

/// Each face's triangles.
pub type Tris = Vec<Vec<[u32; 3]>>;

/// A piece placed: its geometry in the model, each face's triangles and the edges to draw (`Mesh::visible_edges`).
#[derive(Debug)]
pub struct Placed {
    pub mesh: Mesh,
    pub tris: Tris,
    pub edges: Vec<(u32, u32)>,
}

/// The parts as drawn: each instance of the open context (its number and its pieces), then what lies outside it.
#[derive(Debug, Default)]
pub struct Parts {
    pub inside: Vec<(usize, Vec<Rc<Placed>>)>,
    pub outside: Vec<Rc<Placed>>,
}

/// The pieces placed so far, kept between builds.
#[derive(Debug, Default)]
pub struct PieceCache {
    /// Every geometry seen, by a hash of it: the geometry itself (a hash alone is not trusted) and where it went.
    seen: HashMap<u64, Vec<Seen>>,
    /// Builds so far.
    build: u64,
    /// How many pieces the last build placed and split (the others it already had).
    pub made: usize,
}

/// Pieces a build did not use are kept, the most recently used first, while they come to no more than those it did use
/// (at most twice the memory, then) or to this many corners and faces: so undo and redo, and opening and closing groups,
/// mostly find their pieces still here.
const KEEP_AT_LEAST: usize = 200_000;

/// A geometry, and the places it was put (each with the last build that used it there).
#[derive(Debug)]
struct Seen {
    mesh: Mesh,
    placed: HashMap<Spot, (Rc<Placed>, u64)>,
}

/// Where a piece goes, bit for bit: its transform, and the Material painting its unpainted faces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct Spot([u64; 12], Option<Paint>);

impl PieceCache {
    /// The parts drawn from the pieces `Model::with_part_pieces` hands over.
    pub fn build(&mut self, inside: Vec<(usize, Vec<Piece>)>, outside: Vec<Piece>) -> Parts {
        self.build += 1;
        self.made = 0;
        // each geometry is looked up once a build: the pieces borrow it from one scene, so one address is one mesh
        let mut found: HashMap<*const Mesh, (u64, usize)> = HashMap::new();
        let parts = Parts {
            inside: inside.into_iter().map(|(i, pieces)| (i, pieces.iter().map(|p| self.place(&mut found, p)).collect())).collect(),
            outside: outside.iter().map(|p| self.place(&mut found, p)).collect(),
        };
        self.let_go();
        parts
    }

    /// Drops the pieces least recently used beyond what is kept (see [`KEEP_AT_LEAST`]).
    fn let_go(&mut self) {
        let build = self.build;
        let size = |p: &Placed| p.mesh.positions.len() + p.mesh.faces.len();
        let (mut used_now, mut earlier) = (0, std::collections::BTreeMap::<u64, usize>::new());
        for (p, used) in self.seen.values().flatten().flat_map(|s| s.placed.values()) {
            if *used == build {
                used_now += size(p);
            } else {
                *earlier.entry(*used).or_default() += size(p);
            }
        }
        // the oldest build whose pieces are still kept (those of every later build are)
        let (mut kept, mut oldest) = (0, build);
        for (&b, &s) in earlier.iter().rev() {
            if kept + s > used_now.max(KEEP_AT_LEAST) {
                break;
            }
            kept += s;
            oldest = b;
        }
        for list in self.seen.values_mut() {
            list.retain_mut(|s| {
                s.placed.retain(|_, (_, used)| *used >= oldest);
                !s.placed.is_empty()
            });
        }
        self.seen.retain(|_, list| !list.is_empty());
    }

    /// Piece `p` placed: kept from before when the same geometry went to the same place, else made now.
    fn place(&mut self, found: &mut HashMap<*const Mesh, (u64, usize)>, p: &Piece) -> Rc<Placed> {
        let build = self.build;
        let seen = &mut self.seen;
        let (hash, k) = *found.entry(p.mesh as *const Mesh).or_insert_with(|| {
            let hash = hash_mesh(p.mesh);
            let list = seen.entry(hash).or_default();
            let k = list.iter().position(|s| same(&s.mesh, p.mesh)).unwrap_or_else(|| {
                list.push(Seen { mesh: p.mesh.clone(), placed: HashMap::new() });
                list.len() - 1
            });
            (hash, k)
        });
        let seen = &mut self.seen.get_mut(&hash).expect("found above")[k];
        let made = &mut self.made;
        let (placed, used) = seen.placed.entry(Spot(bits(&p.xf), p.paint)).or_insert_with(|| {
            *made += 1;
            let mesh = p.placed();
            let tris = mesh.face_triangles();
            let edges = mesh.visible_edges();
            (Rc::new(Placed { mesh, tris, edges }), build)
        });
        *used = build;
        placed.clone()
    }
}

/// Adds piece `p` to what is drawn: its geometry (sent through `xf` first while it is being moved), its triangles and its
/// edges, numbered on from what is there already.
pub fn append(display: &mut Mesh, tris: &mut Tris, edges: &mut Vec<(u32, u32)>, p: &Placed, xf: Option<&Xform>) {
    let base = display.positions.len() as u32;
    tris.extend(p.tris.iter().map(|ts| ts.iter().map(|t| t.map(|v| v + base)).collect::<Vec<_>>()));
    edges.extend(p.edges.iter().map(|&(a, b)| (a + base, b + base)));
    match xf {
        Some(xf) => display.append(&p.mesh.transformed(xf)),
        None => display.append(&p.mesh),
    }
}

fn bits(xf: &Xform) -> [u64; 12] {
    std::array::from_fn(|k| xf.0[k / 4][k % 4].to_bits())
}

/// Two meshes the same bit for bit (a NaN equals itself, 0 and -0 differ: what is worked out from them is the same).
fn same(a: &Mesh, b: &Mesh) -> bool {
    a.positions.len() == b.positions.len()
        && a.positions.iter().zip(&b.positions).all(|(p, q)| p.map(f64::to_bits) == q.map(f64::to_bits))
        && a.faces == b.faces
        && a.holes == b.holes
        && a.lines == b.lines
        && a.seams == b.seams
        && a.colors == b.colors
}

/// A quick hash of everything in a mesh (FxHash's mixing; a match is checked with `same`).
fn hash_mesh(m: &Mesh) -> u64 {
    struct H(u64);
    impl H {
        fn add(&mut self, w: u64) {
            self.0 = (self.0.rotate_left(5) ^ w).wrapping_mul(0x51_7C_C1_B7_27_22_0A_95);
        }
        fn ring(&mut self, r: &[u32]) {
            self.add(r.len() as u64);
            for c in r.chunks(2) {
                self.add(c[0] as u64 | (c.get(1).map_or(u32::MAX as u64, |&v| v as u64) << 32));
            }
        }
    }
    let mut h = H(0);
    h.add(m.positions.len() as u64);
    for p in &m.positions {
        for c in p {
            h.add(c.to_bits());
        }
    }
    h.add(m.faces.len() as u64);
    for f in &m.faces {
        h.ring(f);
    }
    h.add(m.holes.len() as u64);
    for hs in &m.holes {
        h.add(hs.len() as u64);
        for r in hs {
            h.ring(r);
        }
    }
    for l in [&m.lines, &m.seams] {
        h.add(l.len() as u64);
        for e in l.iter() {
            h.add(e[0] as u64 | (e[1] as u64) << 32);
        }
    }
    h.add(m.colors.len() as u64);
    for c in &m.colors {
        h.add(c.map_or(u64::MAX, |p| {
            u64::from_le_bytes([p.rgb[0], p.rgb[1], p.rgb[2], p.texture.is_some() as u8, 0, 0, 0, 0])
                | (p.texture.unwrap_or(0) as u64) << 32
        }));
    }
    h.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use daedalus_core::scene::{Context, Definition, Instance, Scene};
    use daedalus_core::Model;

    /// What is drawn worked out the old way, everything split again: `Model::parts`, each part's `face_triangles`, and the
    /// edges of the whole.
    fn from_scratch(model: &Model) -> (Mesh, Tris, Vec<(u32, u32)>) {
        let (inside, outside) = model.parts();
        let (mut display, mut tris) = (Mesh::new(), Tris::new());
        for m in inside.iter().map(|(_, m)| m).chain([&outside]) {
            let base = display.positions.len() as u32;
            tris.extend(m.face_triangles().iter().map(|ts| ts.iter().map(|t| t.map(|v| v + base)).collect::<Vec<_>>()));
            display.append(m);
        }
        let edges = display.visible_edges();
        (display, tris, edges)
    }

    /// The same from the cache.
    fn cached(model: &mut Model, cache: &mut PieceCache) -> (Mesh, Tris, Vec<(u32, u32)>) {
        let parts = model.with_part_pieces(|inside, outside| cache.build(inside, outside));
        let (mut display, mut tris, mut edges) = (Mesh::new(), Tris::new(), Vec::new());
        for p in parts.inside.iter().flat_map(|(_, ps)| ps).chain(&parts.outside) {
            append(&mut display, &mut tris, &mut edges, p, None);
        }
        (display, tris, edges)
    }

    /// The cache draws exactly what splitting everything again draws (meshes compared bit for bit); returns how many pieces
    /// it had to split.
    fn check(model: &mut Model, cache: &mut PieceCache) -> usize {
        let want = from_scratch(model);
        let got = cached(model, cache);
        assert_eq!(
            got.0.positions.iter().map(|p| p.map(f64::to_bits)).collect::<Vec<_>>(),
            want.0.positions.iter().map(|p| p.map(f64::to_bits)).collect::<Vec<_>>()
        );
        assert_eq!(got, want);
        cache.made
    }

    /// A box from `lo` to `hi`, faces outward.
    fn cube(lo: [f64; 3], hi: [f64; 3]) -> Mesh {
        let mut m = Mesh::new();
        for k in 0..8 {
            m.add_vertex([
                if k & 1 == 0 { lo[0] } else { hi[0] },
                if k & 2 == 0 { lo[1] } else { hi[1] },
                if k & 4 == 0 { lo[2] } else { hi[2] },
            ]);
        }
        for f in [[0, 2, 3, 1], [4, 5, 7, 6], [0, 1, 5, 4], [2, 6, 7, 3], [0, 4, 6, 2], [1, 3, 7, 5]] {
            m.add_face(f.to_vec());
        }
        m
    }

    /// A floor with a round well in it: one face, a 300-corner hole (the clipping of a big ring with a hole).
    fn floor_with_well() -> Mesh {
        let mut m = Mesh::new();
        for p in [[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [10.0, 8.0, 0.0], [0.0, 8.0, 0.0]] {
            m.add_vertex(p);
        }
        m.add_face(vec![0, 1, 2, 3]);
        let hole: Vec<u32> = (0..300)
            .map(|i| {
                let a = -(i as f64) / 300.0 * std::f64::consts::TAU;
                m.add_vertex([5.0 + 2.0 * a.cos(), 4.0 + 2.0 * a.sin(), 0.0])
            })
            .collect();
        m.set_holes(0, vec![hole]);
        m
    }

    fn def(name: &str, component: bool, mesh: Mesh, instances: Vec<Instance>) -> Definition {
        Definition {
            name: name.into(),
            component,
            glue: false,
            cut: false,
            attributes: Vec::new(),
            part: None,
            ctx: Context { mesh, instances },
        }
    }

    fn at(def: usize, d: [f64; 3]) -> Instance {
        Instance { def, xf: Xform::translation(d) }
    }

    /// Ground at the top level; groups A, B (with a component inside it) and the floor; a component placed twice more, once
    /// turned and once mirrored; and a dynamic component drawing two copies of itself.
    fn house() -> Scene {
        let mut s = Scene::from_mesh(cube([-20.0, -20.0, -0.1], [20.0, 20.0, 0.0]));
        s.defs.push(def("Chair", true, cube([0.0; 3], [0.5, 0.5, 0.9]), vec![]));
        s.defs.push(def("A", false, cube([0.0; 3], [1.0, 2.0, 3.0]), vec![]));
        s.defs.push(def("B", false, cube([0.0; 3], [2.0, 1.0, 1.0]), vec![at(0, [0.3, 0.2, 1.0])]));
        s.defs.push(def("Floor", false, floor_with_well(), vec![]));
        let mut shelf = def("Shelf", true, cube([0.0; 3], [1.0, 0.3, 0.3]), vec![]);
        let attribute =
            |name: &str, formula: &str| daedalus_core::dynamic::Attribute { name: name.into(), formula: formula.into(), label: None };
        shelf.attributes = vec![attribute("Copies", "1"), attribute("X", "=-7+COPY*1.5")];
        s.defs.push(shelf);
        s.root.instances = vec![
            at(1, [1.0, 1.0, 0.0]),
            at(2, [5.0, 0.0, 0.0]),
            at(3, [0.0, 9.0, 0.0]),
            Instance { def: 0, xf: Xform::from_fn(|p| [-p[1] + 3.0, p[0] - 4.0, p[2]]) },
            Instance { def: 0, xf: Xform::from_fn(|p| [-p[0] - 6.0, p[1], p[2]]) },
            at(4, [-7.0, 3.0, 0.0]),
        ];
        s
    }

    fn model() -> Model {
        Model::from_scene(house()).expect("the ground is a clean surface")
    }

    #[test]
    fn the_first_build_splits_every_piece_and_draws_what_parts_does() {
        let (mut m, mut cache) = (model(), PieceCache::default());
        // A, B and its chair, the floor, two more chairs, the shelf drawn twice
        assert_eq!(check(&mut m, &mut cache), 8);
        // nothing changed: nothing split again
        assert_eq!(check(&mut m, &mut cache), 0);
    }

    #[test]
    fn moving_one_group_splits_only_that_one_and_undo_and_redo_split_nothing() {
        let (mut m, mut cache) = (model(), PieceCache::default());
        check(&mut m, &mut cache);
        m.transform_instances(&[0], &Xform::translation([0.0, 0.5, 0.0])).unwrap();
        assert_eq!(check(&mut m, &mut cache), 1, "group A alone");
        // B holds a chair: both of its pieces move
        m.transform_instances(&[1], &Xform::translation([1.0, 0.0, 0.0])).unwrap();
        assert_eq!(check(&mut m, &mut cache), 2);
        // every place either group has been is still known
        for step in [Model::undo, Model::undo, Model::redo, Model::redo] {
            assert!(step(&mut m));
            assert_eq!(check(&mut m, &mut cache), 0);
        }
    }

    #[test]
    fn editing_inside_a_group_splits_only_that_group() {
        let (mut m, mut cache) = (model(), PieceCache::default());
        check(&mut m, &mut cache);
        // open A: its inside is being edited (drawn apart), the rest outside; the top level's ground is a piece now
        m.open(0).unwrap();
        assert_eq!(check(&mut m, &mut cache), 1, "the ground, now outside");
        let top = (0..m.mesh().face_count()).find(|&f| m.mesh().face_normal(f)[2] > 0.9).unwrap();
        m.push_pull(top, 0.5).unwrap();
        assert_eq!(check(&mut m, &mut cache), 0, "A's own geometry is not one of the pieces while it is open");
        m.close();
        assert_eq!(check(&mut m, &mut cache), 1, "A, changed");
        m.undo();
        check(&mut m, &mut cache);
        m.close_all();
        check(&mut m, &mut cache);
    }

    #[test]
    fn editing_one_copy_of_a_component_redraws_the_others() {
        let (mut m, mut cache) = (model(), PieceCache::default());
        check(&mut m, &mut cache);
        // the turned chair, opened: the other chairs (one inside B) are outside and follow every edit inside it
        m.open(3).unwrap();
        check(&mut m, &mut cache);
        let top = (0..m.mesh().face_count()).find(|&f| m.mesh().face_normal(f)[2] > 0.9).unwrap();
        m.push_pull(top, 0.2).unwrap();
        assert_eq!(check(&mut m, &mut cache), 2, "the mirrored chair and the one in B");
        m.close();
        assert!(check(&mut m, &mut cache) <= 3, "the chairs and nothing else");
    }

    #[test]
    fn opening_another_file_draws_that_file() {
        let (mut m, mut cache) = (model(), PieceCache::default());
        check(&mut m, &mut cache);
        // a different model in the same cache, then the first one again (all of it still known)
        let mut other = house();
        other.defs[1].ctx.mesh = cube([0.0; 3], [4.0, 4.0, 4.0]);
        other.root.instances.truncate(3);
        let mut m2 = Model::from_scene(other).unwrap();
        assert_eq!(check(&mut m2, &mut cache), 1, "A is another shape there; B, its chair and the floor are the same");
        let mut m3 = model();
        assert_eq!(check(&mut m3, &mut cache), 0);
        // a model with nothing placed
        let mut empty = Model::new();
        assert_eq!(check(&mut empty, &mut cache), 0);
    }

    #[test]
    fn a_long_run_of_edits_always_draws_what_splitting_it_all_draws() {
        let (mut m, mut cache) = (model(), PieceCache::default());
        check(&mut m, &mut cache);
        let steps: Vec<(&str, fn(&mut Model))> = vec![
            ("mirror the floor", |m| {
                let _ = m.transform_instances(&[2], &Xform::from_fn(|p| [p[0], -p[1], p[2]]));
            }),
            ("copy B (a group: the copy gets a definition of its own)", |m| {
                let _ = m.copy_instances(&[1], &Xform::translation([0.0, 3.0, 0.0]));
            }),
            ("make the turned chair unique", |m| {
                let _ = m.make_unique(3);
            }),
            ("erase A (the definitions after it are numbered down)", |m| {
                let _ = m.erase_instances(&[0]);
            }),
            ("open B", |m| {
                let _ = m.open(0);
            }),
            ("open the chair in it", |m| {
                let _ = m.open(0);
            }),
            ("push a side of that chair", |m| {
                if let Some(f) = (0..m.mesh().face_count()).find(|&f| m.mesh().face_normal(f)[0] > 0.9) {
                    let _ = m.push_pull(f, 0.1);
                }
            }),
            ("close the chair", |m| {
                m.close();
            }),
            ("undo the push", |m| {
                m.undo();
            }),
            ("undo the erase", |m| {
                m.undo();
            }),
            ("redo the erase", |m| {
                m.redo();
            }),
            ("close everything", |m| m.close_all()),
            ("explode the floor", |m| {
                let _ = m.explode(1);
            }),
            ("undo the explode", |m| {
                m.undo();
            }),
            ("paint the ground", |m| {
                let _ = m.paint(&[0], Some([200, 10, 10].into()));
            }),
        ];
        for (name, step) in steps {
            step(&mut m);
            eprintln!("after: {name}");
            check(&mut m, &mut cache);
        }
    }
}
