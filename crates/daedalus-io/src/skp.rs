//! SketchUp models, through `.dskp`: a plain text description written by the `skp-convert` helper, which reads `.skp`
//! files with OpenSKP (MIT; SketchUp's own format is closed, and OpenSKP was made by studying it). This side builds a
//! Daedalus scene from it: every SketchUp definition becomes a group or component, with its faces (holes kept), loose
//! lines, soft edges (hidden seams, so curved surfaces show no stripes), face colours, and its placed copies.
//!
//! ```text
//! daedalus-skp 1
//! material ID r g b a "name"
//! def ID group|component "name"      a definition, up to "end"; the top level is "top" ... "end"
//! v x y z                            metres, Z up; numbered from 0 in each block
//! f MATERIAL nx ny nz LOOPS  N i i i ...  N i i i ...    a face: its normal, then loops (the outer one first)
//! l a b                              a loose edge
//! s a b                              a soft or smooth edge (drawn hidden)
//! i DEF r00 r01 r02 tx r10 r11 r12 ty r20 r21 r22 tz     a copy of definition DEF
//! end
//! ```
//!
//! SketchUp lets three or more faces share an edge (walls meeting in a T inside one group) and lets neighbouring faces face
//! opposite ways; an editable Daedalus surface allows neither. Each face is turned round if that fits, and the faces that
//! still do not fit go into a group of their own inside the same context ("... (overlapping faces)"), so everything stays
//! editable.

use crate::IoError;
use daedalus_core::scene::{Context, Definition, Instance, Scene, Xform};
use daedalus_core::sketch::Plane;
use daedalus_core::Mesh;
use std::collections::{HashMap, HashSet};

#[derive(Default)]
struct FaceIn {
    material: i64,
    normal: [f32; 3],
    rings: Vec<Vec<u32>>,
}

#[derive(Default)]
struct Block {
    name: String,
    component: bool,
    positions: Vec<[f32; 3]>,
    faces: Vec<FaceIn>,
    lines: Vec<[u32; 2]>,
    soft: Vec<[u32; 2]>,
    instances: Vec<(i64, [f32; 12])>,
}

/// Words of a line, "quoted text" as one word.
fn words(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = line.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
        } else if c == '"' {
            chars.next();
            let mut w = String::new();
            while let Some(ch) = chars.next() {
                match ch {
                    '"' => break,
                    '\\' => {
                        if let Some(e) = chars.next() {
                            w.push(e);
                        }
                    }
                    ch => w.push(ch),
                }
            }
            out.push(w);
        } else {
            let mut w = String::new();
            while let Some(&ch) = chars.peek() {
                if ch.is_whitespace() {
                    break;
                }
                w.push(ch);
                chars.next();
            }
            out.push(w);
        }
    }
    out
}

fn num<T: std::str::FromStr>(w: Option<&String>, line: usize) -> Result<T, IoError> {
    w.and_then(|s| s.parse().ok()).ok_or_else(|| IoError::Parse(format!("line {line}: a number is missing or unreadable")))
}

/// Reads a `.dskp` file into a scene.
pub fn read(bytes: &[u8]) -> Result<Scene, IoError> {
    let text = std::str::from_utf8(bytes).map_err(|_| IoError::Parse("not UTF-8 text".into()))?;
    let mut lines = text.lines().enumerate().map(|(i, l)| (i + 1, l)).filter(|(_, l)| !l.trim().is_empty() && !l.trim_start().starts_with('#'));
    let (_, head) = lines.next().ok_or_else(|| IoError::Parse("empty file".into()))?;
    let head = words(head);
    if head.first().map(String::as_str) != Some("daedalus-skp") {
        return Err(IoError::Parse("not a .dskp file (from skp-convert)".into()));
    }
    if num::<u32>(head.get(1), 1)? > 1 {
        return Err(IoError::Unsupported("a .dskp file from a newer skp-convert".into()));
    }
    let mut colors: HashMap<i64, [u8; 3]> = HashMap::new();
    let mut defs: Vec<(i64, Block)> = Vec::new();
    let mut top: Option<Block> = None;
    // the block being read: Some(true) the top level, Some(false) the last definition
    let mut open: Option<bool> = None;
    for (n, line) in lines {
        let w = words(line);
        let Some(kind) = w.first().map(String::as_str) else { continue };
        let block = match open {
            Some(true) => top.as_mut(),
            Some(false) => defs.last_mut().map(|d| &mut d.1),
            None => None,
        };
        match (kind, block) {
            ("material", _) => {
                let id: i64 = num(w.get(1), n)?;
                let c: Vec<u8> = (2..5).map(|i| num::<u8>(w.get(i), n)).collect::<Result<_, _>>()?;
                colors.insert(id, [c[0], c[1], c[2]]);
            }
            ("def", None) => {
                let id: i64 = num(w.get(1), n)?;
                let component = w.get(2).map(String::as_str) != Some("group");
                defs.push((id, Block { name: w.get(3).cloned().unwrap_or_default(), component, ..Default::default() }));
                open = Some(false);
            }
            ("top", None) => {
                top = Some(Block::default());
                open = Some(true);
            }
            ("end", Some(_)) => open = None,
            ("v", Some(b)) => {
                let c: Vec<f32> = (1..4).map(|i| num::<f32>(w.get(i), n)).collect::<Result<_, _>>()?;
                if c.iter().any(|x| !x.is_finite()) {
                    return Err(IoError::Parse(format!("line {n}: not a finite number")));
                }
                b.positions.push([c[0], c[1], c[2]]);
            }
            ("f", Some(b)) => {
                let material: i64 = num(w.get(1), n)?;
                let normal = [num::<f32>(w.get(2), n)?, num::<f32>(w.get(3), n)?, num::<f32>(w.get(4), n)?];
                let loops: usize = num(w.get(5), n)?;
                let mut at = 6;
                let mut rings = Vec::new();
                for _ in 0..loops {
                    let len: usize = num(w.get(at), n)?;
                    at += 1;
                    let ring: Vec<u32> = (at..at + len).map(|i| num::<u32>(w.get(i), n)).collect::<Result<_, _>>()?;
                    at += len;
                    if ring.iter().any(|&i| i as usize >= b.positions.len()) {
                        return Err(IoError::Parse(format!("line {n}: a face uses a vertex not defined above")));
                    }
                    rings.push(ring);
                }
                b.faces.push(FaceIn { material, normal, rings });
            }
            ("l" | "s", Some(b)) => {
                let (a, c): (u32, u32) = (num(w.get(1), n)?, num(w.get(2), n)?);
                if a.max(c) as usize >= b.positions.len() {
                    return Err(IoError::Parse(format!("line {n}: an edge uses a vertex not defined above")));
                }
                if kind == "l" { b.lines.push([a, c]) } else { b.soft.push([a, c]) }
            }
            ("i", Some(b)) => {
                let def: i64 = num(w.get(1), n)?;
                let mut m = [0.0f32; 12];
                for (k, v) in m.iter_mut().enumerate() {
                    *v = num(w.get(2 + k), n)?;
                }
                b.instances.push((def, m));
            }
            (other, _) => return Err(IoError::Parse(format!("line {n}: unexpected {other:?}"))),
        }
    }
    let top = top.ok_or_else(|| IoError::Parse("the file has no top level".into()))?;
    Ok(assemble(defs, top, &colors))
}

/// Polygons ready to add: vertex positions (indices into a shared list), each with its colour.
struct Poly {
    ring: Vec<u32>,
    color: Option<[u8; 3]>,
}

/// Shared vertex list that joins points closer than a hundredth of a millimetre.
#[derive(Default)]
struct Points {
    positions: Vec<[f32; 3]>,
    at: HashMap<[i64; 3], u32>,
}

impl Points {
    fn add(&mut self, p: [f32; 3]) -> u32 {
        let key = p.map(|c| (c / 1e-5).round() as i64);
        if let Some(&i) = self.at.get(&key) {
            return i;
        }
        let i = self.positions.len() as u32;
        self.positions.push(p);
        self.at.insert(key, i);
        i
    }
}

fn newell(positions: &[[f32; 3]], ring: &[u32]) -> [f32; 3] {
    let mut n = [0.0f32; 3];
    for i in 0..ring.len() {
        let (p, q) = (positions[ring[i] as usize], positions[ring[(i + 1) % ring.len()] as usize]);
        n[0] += (p[1] - q[1]) * (p[2] + q[2]);
        n[1] += (p[2] - q[2]) * (p[0] + q[0]);
        n[2] += (p[0] - q[0]) * (p[1] + q[1]);
    }
    n
}

/// One SketchUp face as Daedalus polygons (a face with holes becomes the faces that cover it, joined by hidden seams).
fn face_polys(b: &Block, f: &FaceIn, points: &mut Points, colors: &HashMap<i64, [u8; 3]>, seams: &mut Vec<[u32; 2]>) -> Vec<Poly> {
    let color = colors.get(&f.material).copied();
    if f.rings.len() == 1 {
        let mut ring: Vec<u32> = f.rings[0].iter().map(|&i| points.add(b.positions[i as usize])).collect();
        ring.dedup();
        while ring.len() > 1 && ring.first() == ring.last() {
            ring.pop();
        }
        let distinct: HashSet<u32> = ring.iter().copied().collect();
        if distinct.len() < 3 || distinct.len() != ring.len() {
            return Vec::new();
        }
        let n = newell(&points.positions, &ring);
        if n[0] * f.normal[0] + n[1] * f.normal[1] + n[2] * f.normal[2] < 0.0 {
            ring.reverse();
        }
        return vec![Poly { ring, color }];
    }
    // holes: the outlines laid out on the face's plane and covered as Daedalus covers them (core profile)
    let first = b.positions[f.rings[0][0] as usize];
    let normal = if f.normal == [0.0; 3] { newell(&b.positions, &f.rings[0]) } else { f.normal };
    let Some(plane) = Plane::from_point_normal(first, normal) else { return Vec::new() };
    let outlines: Vec<Vec<[f32; 2]>> = f.rings.iter().map(|r| r.iter().map(|&i| plane.to_local(b.positions[i as usize])).collect()).collect();
    let Ok(cap) = daedalus_core::profile::faces(&outlines, &plane) else { return Vec::new() };
    let map: Vec<u32> = cap.positions.iter().map(|&p| points.add(p)).collect();
    seams.extend(cap.seams.iter().map(|s| s.map(|v| map[v as usize])));
    cap.faces.iter().map(|ring| Poly { ring: ring.iter().map(|&v| map[v as usize]).collect(), color }).collect()
}

/// A context's geometry: the faces that make one editable surface, and the rest (to go into a group of their own).
fn surface(polys: Vec<Vec<Poly>>, points: &Points) -> (Mesh, Vec<Vec<Poly>>) {
    let mut taken: HashSet<(u32, u32)> = HashSet::new();
    let mut mesh = Mesh { positions: points.positions.clone(), ..Default::default() };
    let mut rest = Vec::new();
    let edges = |unit: &[Poly], rev: bool| -> Vec<(u32, u32)> {
        unit.iter()
            .flat_map(|p| (0..p.ring.len()).map(move |i| (p.ring[i], p.ring[(i + 1) % p.ring.len()])))
            .map(|(a, b)| if rev { (b, a) } else { (a, b) })
            .collect()
    };
    for unit in polys {
        let fits = |es: &[(u32, u32)]| {
            let set: HashSet<&(u32, u32)> = es.iter().collect();
            set.len() == es.len() && es.iter().all(|e| !taken.contains(e))
        };
        let (fwd, back) = (edges(&unit, false), edges(&unit, true));
        let rev = if fits(&fwd) {
            false
        } else if fits(&back) {
            true
        } else {
            rest.push(unit);
            continue;
        };
        taken.extend(if rev { back } else { fwd });
        for p in unit {
            let ring = if rev { p.ring.into_iter().rev().collect() } else { p.ring };
            mesh.faces.push(ring);
            let f = mesh.faces.len() - 1;
            mesh.set_color(f, p.color);
        }
    }
    (mesh, rest)
}

/// Builds one context (a definition's inside or the top level): its surface, and nested groups for faces that do not fit.
fn context(b: &Block, colors: &HashMap<i64, [u8; 3]>, scene: &mut Scene, ids: &HashMap<i64, usize>) -> Context {
    let mut points = Points::default();
    let mut seams = Vec::new();
    let polys: Vec<Vec<Poly>> = b.faces.iter().map(|f| face_polys(b, f, &mut points, colors, &mut seams)).filter(|u| !u.is_empty()).collect();
    let line_at = |points: &mut Points, [a, c]: [u32; 2]| [points.add(b.positions[a as usize]), points.add(b.positions[c as usize])];
    let lines: Vec<[u32; 2]> = b.lines.iter().map(|&l| line_at(&mut points, l)).filter(|l| l[0] != l[1]).collect();
    let soft: Vec<[u32; 2]> = b.soft.iter().map(|&l| line_at(&mut points, l)).collect();
    let mut instances: Vec<Instance> = b
        .instances
        .iter()
        .filter_map(|(def, m)| {
            let d = *ids.get(def)?;
            Some(Instance { def: d, xf: Xform([[m[0], m[1], m[2], m[3]], [m[4], m[5], m[6], m[7]], [m[8], m[9], m[10], m[11]]]) })
        })
        .collect();
    let (mut mesh, mut rest) = surface(polys, &points);
    mesh.lines = lines;
    mesh.seams = seams.into_iter().chain(soft).collect();
    // the faces that did not fit: a group (or several, nested no deeper than four) of their own, in place
    let mut depth = 0;
    let mut holder: Option<usize> = None;
    while !rest.is_empty() {
        depth += 1;
        let (m, more) = if depth >= 4 { (everything(rest, &points), Vec::new()) } else { surface(rest, &points) };
        rest = more;
        let mut m = m;
        m.seams = mesh.seams.clone();
        tidy(&mut m);
        let name = format!("{} (overlapping faces{})", if b.name.is_empty() { "Model" } else { &b.name }, if depth > 1 { format!(" {depth}") } else { String::new() });
        scene.defs.push(Definition { name, component: false, ctx: Context { mesh: m, instances: Vec::new() } });
        let d = scene.defs.len() - 1;
        match holder {
            None => instances.push(Instance { def: d, xf: Xform::IDENTITY }),
            Some(h) => scene.defs[h].ctx.instances.push(Instance { def: d, xf: Xform::IDENTITY }),
        }
        holder = Some(d);
    }
    tidy(&mut mesh);
    Context { mesh, instances }
}

/// Every remaining face as it is (the last resort: shown, but not editable as one surface).
fn everything(units: Vec<Vec<Poly>>, points: &Points) -> Mesh {
    let mut m = Mesh { positions: points.positions.clone(), ..Default::default() };
    for p in units.into_iter().flatten() {
        m.faces.push(p.ring);
        let f = m.faces.len() - 1;
        m.set_color(f, p.color);
    }
    m
}

/// Seams only where they are face edges; no unused vertices.
fn tidy(m: &mut Mesh) {
    let edges: HashSet<(u32, u32)> = m.edges().into_iter().collect();
    m.seams.retain(|&[a, b]| edges.contains(&(a.min(b), a.max(b))));
    m.seams.sort_unstable();
    m.seams.dedup();
    m.compact();
}

fn assemble(defs: Vec<(i64, Block)>, top: Block, colors: &HashMap<i64, [u8; 3]>) -> Scene {
    let mut scene = Scene::default();
    // definitions get their numbers first, so instances can name them; their insides are filled in after
    let ids: HashMap<i64, usize> = defs.iter().enumerate().map(|(i, (id, _))| (*id, i)).collect();
    for (_, b) in &defs {
        scene.defs.push(Definition { name: b.name.clone(), component: b.component, ctx: Context::default() });
    }
    for (i, (_, b)) in defs.iter().enumerate() {
        let ctx = context(b, colors, &mut scene, &ids);
        scene.defs[i].ctx = ctx;
    }
    scene.root = context(&top, colors, &mut scene, &ids);
    // a definition that holds itself (only in a damaged file) loses that instance
    for d in 0..scene.defs.len() {
        scene.defs[d].ctx.instances.retain(|i| i.def != d);
    }
    scene.purge();
    scene
}

#[cfg(test)]
mod tests {
    use super::*;
    use daedalus_core::{HalfEdgeMesh, Model};

    /// A 1 m box at the top level, a window hole in its front, a component placed twice, and a T of three faces on one edge.
    const SAMPLE: &str = r#"daedalus-skp 1
material 7 200 40 40 255 "Red"
def 900 component "Post"
v 0 0 0
v 0.1 0 0
v 0.1 0.1 0
v 0 0.1 0
f 7 0 0 1 1 4 0 1 2 3
end
def 901 group "Tee"
v 0 0 0
v 1 0 0
v 1 0 1
v 0 0 1
v 1 1 0
v 1 1 1
v 1 -1 0
v 1 -1 1
f -1 0 -1 0 1 4 0 1 2 3
f -1 1 0 0 1 4 1 4 5 2
f -1 1 0 0 1 4 6 1 2 7
end
top
v 0 0 0
v 1 0 0
v 1 1 0
v 0 1 0
v 0 0 1
v 1 0 1
v 1 1 1
v 0 1 1
v 0.3 0 0.3
v 0.7 0 0.3
v 0.7 0 0.7
v 0.3 0 0.7
f -1 0 0 -1 1 4 0 3 2 1
f -1 0 0 1 1 4 4 5 6 7
f 7 0 -1 0 2 4 0 1 5 4 4 8 11 10 9
f -1 1 0 0 1 4 1 2 6 5
f -1 0 1 0 1 4 2 3 7 6
f -1 -1 0 0 1 4 3 0 4 7
l 0 6
i 900 1 0 0 2 0 1 0 0 0 0 1 0
i 900 1 0 0 3 0 1 0 0 0 0 1 0
i 901 1 0 0 5 0 1 0 0 0 0 1 0
end
"#;

    #[test]
    fn a_sketchup_model_comes_in_with_its_groups_and_components() {
        let s = read(SAMPLE.as_bytes()).unwrap();
        // the box is the top level and editable: the window face has a hole, so the front is a ring of faces
        let model = Model::from_scene(s.clone()).expect("the top level is an editable surface");
        assert!(model.mesh().face_count() >= 7, "{}", model.mesh().face_count());
        assert_eq!(model.mesh().lines.len(), 1, "the loose line");
        assert!(!model.mesh().seams.is_empty(), "the hole's seams are hidden");
        // the front (with the hole) is red
        assert!(model.mesh().colors.iter().any(|c| *c == Some([200, 40, 40])));
        // the post component is placed twice, sharing one definition
        let posts: Vec<_> = s.root.instances.iter().filter(|i| s.defs[i.def].name == "Post").collect();
        assert_eq!(posts.len(), 2);
        assert_eq!(posts[0].def, posts[1].def);
        assert!(s.defs[posts[0].def].component);
        assert_eq!(posts[1].xf.origin(), [3.0, 0.0, 0.0]);
        // the T: two faces on its edge fit, the third goes into a group of its own inside it
        let tee = s.root.instances.iter().find(|i| s.defs[i.def].name == "Tee").unwrap().def;
        assert!(!s.defs[tee].component);
        assert_eq!(s.defs[tee].ctx.mesh.faces.len(), 2);
        let spill = s.defs[tee].ctx.instances[0].def;
        assert_eq!(s.defs[spill].name, "Tee (overlapping faces)");
        assert_eq!(s.defs[spill].ctx.mesh.faces.len(), 1);
        assert!(HalfEdgeMesh::from_mesh(&s.defs[tee].ctx.mesh).is_ok());
        // and the hole in the front lets light through: less area than a full 1 x 1 face
        let flat = s.flatten();
        assert_eq!(flat.faces.len(), model.mesh().face_count() + 2 + 3);
    }

    #[test]
    fn bad_files_are_refused() {
        assert!(read(b"hello\n").is_err());
        assert!(read(b"daedalus-skp 1\ntop\nf -1 0 0 1 1 3 0 1 2\nend\n").unwrap_err().to_string().contains("vertex not defined"));
        assert!(read(b"daedalus-skp 2\n").is_err());
        assert!(read(b"daedalus-skp 1\n").unwrap_err().to_string().contains("no top level"));
    }
}

#[cfg(test)]
mod timing {
    /// `DSKP=file cargo test -p daedalus-io --release timing -- --ignored --nocapture`: how long reading and the per-mouse-move
    /// work take on a real model.
    #[test]
    #[ignore]
    fn time_a_real_model() {
        let Some(path) = std::env::var_os("DSKP") else { return };
        let t = std::time::Instant::now();
        let s = super::read(&std::fs::read(path).unwrap()).unwrap();
        println!("read: {:?}", t.elapsed());
        let t = std::time::Instant::now();
        let m = daedalus_core::Model::from_scene(s.clone());
        println!("model: {:?} ok={}", t.elapsed(), m.is_ok());
        let flat = s.flatten();
        println!("flat: {} vertices, {} faces, {} defs, {} top instances", flat.positions.len(), flat.faces.len(), s.defs.len(), s.root.instances.len());
        let t = std::time::Instant::now();
        let e = flat.visible_edges();
        println!("visible_edges: {:?} ({} edges)", t.elapsed(), e.len());
        let t = std::time::Instant::now();
        let hit = flat.pick_face([10.0, -30.0, 20.0], [0.0, 0.8, -0.6]);
        println!("pick_face: {:?} {:?}", t.elapsed(), hit.map(|h| h.0));
        let t = std::time::Instant::now();
        let tris = flat.face_triangles();
        println!("face_triangles (once): {:?}", t.elapsed());
        let t = std::time::Instant::now();
        let hit2 = flat.pick_triangles(&tris, [10.0, -30.0, 20.0], [0.0, 0.8, -0.6]);
        println!("pick_triangles (each move): {:?} {:?}", t.elapsed(), hit2.map(|h| h.0));
        if let Ok(m) = m {
            let t = std::time::Instant::now();
            let _ = m.parts();
            println!("parts: {:?}", t.elapsed());
        }
    }
}
