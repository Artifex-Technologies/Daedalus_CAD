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
use daedalus_core::Mesh;
use std::collections::{HashMap, HashSet};

#[derive(Default)]
struct FaceIn {
    material: i64,
    normal: [f64; 3],
    rings: Vec<Vec<u32>>,
}

#[derive(Default)]
struct Block {
    name: String,
    component: bool,
    positions: Vec<[f64; 3]>,
    faces: Vec<FaceIn>,
    lines: Vec<[u32; 2]>,
    soft: Vec<[u32; 2]>,
    instances: Vec<(i64, [f64; 12])>,
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
                let c: Vec<f64> = (1..4).map(|i| num::<f64>(w.get(i), n)).collect::<Result<_, _>>()?;
                if c.iter().any(|x| !x.is_finite()) {
                    return Err(IoError::Parse(format!("line {n}: not a finite number")));
                }
                b.positions.push([c[0], c[1], c[2]]);
            }
            ("f", Some(b)) => {
                let material: i64 = num(w.get(1), n)?;
                let normal = [num::<f64>(w.get(2), n)?, num::<f64>(w.get(3), n)?, num::<f64>(w.get(4), n)?];
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
                let mut m = [0.0f64; 12];
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
    /// Its openings, wound the other way round.
    holes: Vec<Vec<u32>>,
    color: Option<[u8; 3]>,
}

impl Poly {
    /// Its edges, outline and holes, as directed pairs (turned round with `rev`).
    fn edges(&self, rev: bool) -> impl Iterator<Item = (u32, u32)> + '_ {
        std::iter::once(&self.ring).chain(&self.holes).flat_map(move |r| (0..r.len()).map(move |i| if rev { (r[(i + 1) % r.len()], r[i]) } else { (r[i], r[(i + 1) % r.len()]) }))
    }

    /// Turned round: the outline and every hole.
    fn reversed(mut self) -> Poly {
        self.ring.reverse();
        for h in &mut self.holes {
            h.reverse();
        }
        self
    }
}

/// Shared vertex list that joins points closer than a hundredth of a millimetre.
#[derive(Default)]
struct Points {
    positions: Vec<[f64; 3]>,
    at: HashMap<[i64; 3], u32>,
}

impl Points {
    fn add(&mut self, p: [f64; 3]) -> u32 {
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

fn newell(positions: &[[f64; 3]], ring: &[u32]) -> [f64; 3] {
    let mut n = [0.0f64; 3];
    for i in 0..ring.len() {
        let (p, q) = (positions[ring[i] as usize], positions[ring[(i + 1) % ring.len()] as usize]);
        n[0] += (p[1] - q[1]) * (p[2] + q[2]);
        n[1] += (p[2] - q[2]) * (p[0] + q[0]);
        n[2] += (p[0] - q[0]) * (p[1] + q[1]);
    }
    n
}

/// One SketchUp face as a Daedalus face (with its holes, if it has any).
fn face_polys(b: &Block, f: &FaceIn, points: &mut Points, colors: &HashMap<i64, [u8; 3]>) -> Vec<Poly> {
    let color = colors.get(&f.material).copied();
    if f.rings.len() == 1 {
        let mut ring: Vec<u32> = f.rings[0].iter().map(|&i| points.add(b.positions[i as usize])).collect();
        ring.dedup();
        while ring.len() > 1 && ring.first() == ring.last() {
            ring.pop();
        }
        // a corner may come twice (a pinched outline), an edge may not
        let distinct: HashSet<u32> = ring.iter().copied().collect();
        let edges: HashSet<(u32, u32)> = (0..ring.len()).map(|i| (ring[i], ring[(i + 1) % ring.len()])).collect();
        let doubled = (0..ring.len()).any(|i| edges.contains(&(ring[(i + 1) % ring.len()], ring[i])));
        if distinct.len() < 3 || edges.len() != ring.len() || doubled {
            return Vec::new();
        }
        let n = newell(&points.positions, &ring);
        if n[0] * f.normal[0] + n[1] * f.normal[1] + n[2] * f.normal[2] < 0.0 {
            ring.reverse();
        }
        return vec![Poly { ring, holes: Vec::new(), color }];
    }
    // a face with holes: the outline facing as SketchUp says, each hole wound the other way
    let rings: Vec<Vec<u32>> = f
        .rings
        .iter()
        .map(|r| {
            let mut r: Vec<u32> = r.iter().map(|&i| points.add(b.positions[i as usize])).collect();
            r.dedup();
            while r.len() > 1 && r.first() == r.last() {
                r.pop();
            }
            r
        })
        .collect();
    let distinct = |r: &Vec<u32>| r.iter().collect::<HashSet<_>>().len() == r.len();
    // a hole touching the outline at one corner: the outline runs into it there and back out (that corner twice), as
    // SketchUp's own face does
    let mut rings = rings;
    let mut spliced = Vec::new();
    for (k, h) in rings.iter().enumerate().skip(1) {
        let shared: Vec<u32> = h.iter().copied().filter(|v| rings[0].contains(v)).collect();
        if shared.len() == 1 && distinct(h) && h.len() >= 3 {
            spliced.push((k, shared[0]));
        }
    }
    for &(k, v) in spliced.iter().rev() {
        let mut h = rings.remove(k);
        let at = rings[0].iter().position(|&x| x == v).expect("shared");
        let from = h.iter().position(|&x| x == v).expect("shared");
        h.rotate_left(from);
        // wound against the outline (a hole), so the walk round it continues the outline's way
        let n0 = newell(&points.positions, &rings[0]);
        let nh = newell(&points.positions, &h);
        if n0[0] * nh[0] + n0[1] * nh[1] + n0[2] * nh[2] > 0.0 {
            h[1..].reverse();
        }
        let tail = rings[0].split_off(at + 1);
        rings[0].extend(h[1..].iter().copied());
        rings[0].push(v);
        rings[0].extend(tail);
    }
    // an outline may pass a corner twice (pinched, as where a hole was spliced in), but never an edge twice
    let pinched_ok = |r: &Vec<u32>| {
        let edges: HashSet<(u32, u32)> = (0..r.len()).map(|i| (r[i], r[(i + 1) % r.len()])).collect();
        edges.len() == r.len() && (0..r.len()).all(|i| !edges.contains(&(r[(i + 1) % r.len()], r[i])))
    };
    let touching = rings[1..].iter().any(|h| h.iter().any(|v| rings[0].contains(v)));
    if rings[0].len() < 3 || !pinched_ok(&rings[0]) || touching {
        // an outline that touches itself or a hole: drawn as Daedalus draws such outlines (core profile), line by line
        let first = b.positions[f.rings[0][0] as usize];
        let normal = if f.normal == [0.0; 3] { newell(&b.positions, &f.rings[0]) } else { f.normal };
        let Some(plane) = daedalus_core::sketch::Plane::from_point_normal(first, normal) else { return Vec::new() };
        let outlines: Vec<Vec<[f64; 2]>> = f.rings.iter().map(|r| r.iter().map(|&i| plane.to_local(b.positions[i as usize])).collect()).collect();
        let Ok(cap) = daedalus_core::profile::faces(&outlines, &plane) else { return Vec::new() };
        let map: Vec<u32> = cap.positions.iter().map(|&p| points.add(p)).collect();
        let at = |r: &Vec<u32>| r.iter().map(|&v| map[v as usize]).collect::<Vec<u32>>();
        return (0..cap.faces.len()).map(|k| Poly { ring: at(&cap.faces[k]), holes: cap.holes_of(k).iter().map(at).collect(), color }).collect();
    }
    let mut ring = rings[0].clone();
    let n = newell(&points.positions, &ring);
    let normal = if f.normal == [0.0; 3] { n } else { f.normal };
    let along = |r: &[u32]| {
        let m = newell(&points.positions, r);
        m[0] * normal[0] + m[1] * normal[1] + m[2] * normal[2]
    };
    if along(&ring) < 0.0 {
        ring.reverse();
    }
    let holes: Vec<Vec<u32>> = rings[1..]
        .iter()
        .filter(|h| h.len() >= 3 && distinct(h) && !h.iter().any(|v| ring.contains(v)))
        .map(|h| {
            let mut h = h.clone();
            if along(&h) > 0.0 {
                h.reverse();
            }
            h
        })
        .collect();
    vec![Poly { ring, holes, color }]
}

/// A context's geometry: the faces that make one editable surface, and the rest (to go into a group of their own).
fn surface(polys: Vec<Vec<Poly>>, points: &Points) -> (Mesh, Vec<Vec<Poly>>) {
    let mut taken: HashSet<(u32, u32)> = HashSet::new();
    let mut mesh = Mesh { positions: points.positions.clone(), ..Default::default() };
    let mut rest = Vec::new();
    let edges = |unit: &[Poly], rev: bool| -> Vec<(u32, u32)> {
        unit.iter().flat_map(|p| p.edges(rev)).collect()
    };
    // faces in the order of a walk across their shared edges, so each one meets its neighbours already placed and takes its
    // facing from them (SketchUp lets neighbours face opposite ways; in file order two far-apart faces could be set facing
    // opposite ways before the face between them came, which then fitted neither way)
    let order = {
        let mut by_edge: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
        for (k, unit) in polys.iter().enumerate() {
            for (a, b) in edges(unit, false) {
                by_edge.entry((a.min(b), a.max(b))).or_default().push(k);
            }
        }
        let mut seen = vec![false; polys.len()];
        let mut order = Vec::with_capacity(polys.len());
        for start in 0..polys.len() {
            if seen[start] {
                continue;
            }
            seen[start] = true;
            let mut queue = std::collections::VecDeque::from([start]);
            while let Some(k) = queue.pop_front() {
                order.push(k);
                for (a, b) in edges(&polys[k], false) {
                    for &n in by_edge.get(&(a.min(b), a.max(b))).map(Vec::as_slice).unwrap_or(&[]) {
                        if !seen[n] {
                            seen[n] = true;
                            queue.push_back(n);
                        }
                    }
                }
            }
        }
        order
    };
    let mut slots: Vec<Option<Vec<Poly>>> = polys.into_iter().map(Some).collect();
    for k in order {
        let unit = slots[k].take().expect("each once");
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
            let p = if rev { p.reversed() } else { p };
            mesh.faces.push(p.ring);
            let f = mesh.faces.len() - 1;
            mesh.set_color(f, p.color);
            mesh.set_holes(f, p.holes);
        }
    }
    (mesh, rest)
}

/// Builds one context (a definition's inside or the top level): its surface, and nested groups for faces that do not fit.
fn context(b: &Block, colors: &HashMap<i64, [u8; 3]>, scene: &mut Scene, ids: &HashMap<i64, usize>) -> Context {
    let mut points = Points::default();
    let polys: Vec<Vec<Poly>> = b.faces.iter().map(|f| face_polys(b, f, &mut points, colors)).filter(|u| !u.is_empty()).collect();
    // a face exactly on top of another (the same corners) is shown as one in SketchUp and adds nothing: kept once (Dream
    // House v3's top level had 72 such repeats, some edges under 17 faces)
    let mut seen: HashSet<Vec<u32>> = HashSet::new();
    let polys: Vec<Vec<Poly>> = polys
        .into_iter()
        .filter(|u| {
            let mut key: Vec<u32> = u.iter().flat_map(|p| p.ring.iter().copied()).collect();
            key.sort_unstable();
            seen.insert(key)
        })
        .collect();
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
    mesh.seams = soft;
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
        scene.defs.push(Definition { name, component: false, glue: false, cut: false, ctx: Context { mesh: m, instances: Vec::new() } });
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
        m.set_holes(f, p.holes);
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
        scene.defs.push(Definition { name: b.name.clone(), component: b.component, glue: false, cut: false, ctx: Context::default() });
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
        // the box is the top level and editable: the front has a window, one face with a hole (six sides, no seams)
        let model = Model::from_scene(s.clone()).expect("the top level is an editable surface");
        assert_eq!(model.mesh().face_count(), 6);
        assert_eq!(model.mesh().lines.len(), 1, "the loose line");
        assert!(model.mesh().seams.is_empty());
        assert!((0..6).any(|f| model.mesh().has_holes(f)), "the front has its window");
        // the front (with the hole) is red
        assert!(model.mesh().colors.contains(&Some([200, 40, 40])));
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
    fn a_face_stored_twice_comes_in_once() {
        let twice = "daedalus-skp 1\ntop\nv 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nf -1 0 0 1 1 4 0 1 2 3\nf -1 0 0 1 1 4 1 2 3 0\nf -1 0 0 -1 1 4 3 2 1 0\nend\n";
        let s = read(twice.as_bytes()).unwrap();
        assert_eq!(s.root.mesh.faces.len(), 1);
        assert!(s.root.instances.is_empty(), "nothing left over for an overlap group");
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
        let spilled: usize = s.defs.iter().filter(|d| d.name.contains("(overlapping faces")).map(|d| d.ctx.mesh.faces.len()).sum();
        let tri_spilled: usize = s.defs.iter().filter(|d| d.name.contains("(overlapping faces")).map(|d| d.ctx.mesh.faces.iter().filter(|f| f.len() == 3).count()).sum();
        let all: usize = s.root.mesh.faces.len() + s.defs.iter().map(|d| d.ctx.mesh.faces.len()).sum::<usize>();
        let tris_all: usize = s.root.mesh.faces.iter().chain(s.defs.iter().flat_map(|d| d.ctx.mesh.faces.iter())).filter(|f| f.len() == 3).count();
        println!("faces {all} (triangles {tris_all}); in overlapping groups {spilled} (triangles {tri_spilled})");
        if let Ok(m) = m {
            let t = std::time::Instant::now();
            let _ = m.parts();
            println!("parts: {:?}", t.elapsed());
        }
    }
}

#[cfg(test)]
mod holes_check {
    use super::*;
    /// `DSKP=file`: every face with holes, covered, and its triangles' area against the face's own (outer minus holes).
    #[test]
    #[ignore]
    fn faces_with_holes_are_covered_exactly() {
        let Some(path) = std::env::var_os("DSKP") else { return };
        let s = std::fs::read_to_string(path).unwrap();
        let mut b = Block::default();
        let (mut bad, mut all) = (0, 0);
        for line in s.lines() {
            let w = words(line);
            match w.first().map(String::as_str) {
                Some("def") | Some("top") => b = Block::default(),
                Some("v") => b.positions.push([w[1].parse().unwrap(), w[2].parse().unwrap(), w[3].parse().unwrap()]),
                Some("f") => {
                    let loops: usize = w[5].parse().unwrap();
                    if loops < 2 {
                        continue;
                    }
                    let mut at = 6;
                    let mut rings = vec![];
                    for _ in 0..loops {
                        let n: usize = w[at].parse().unwrap();
                        at += 1;
                        rings.push((at..at + n).map(|i| w[i].parse().unwrap()).collect::<Vec<u32>>());
                        at += n;
                    }
                    let f = FaceIn { material: -1, normal: [w[2].parse().unwrap(), w[3].parse().unwrap(), w[4].parse().unwrap()], rings };
                    let area = |pts: &[[f64; 3]]| {
                        let mut n = [0.0f64; 3];
                        for i in 0..pts.len() {
                            let (p, q) = (pts[i], pts[(i + 1) % pts.len()]);
                            n[0] += (p[1] - q[1]) * (p[2] + q[2]);
                            n[1] += (p[2] - q[2]) * (p[0] + q[0]);
                            n[2] += (p[0] - q[0]) * (p[1] + q[1]);
                        }
                        (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt() / 2.0
                    };
                    let rp = |r: &Vec<u32>| r.iter().map(|&i| b.positions[i as usize]).collect::<Vec<_>>();
                    let want = area(&rp(&f.rings[0])) - f.rings[1..].iter().map(|r| area(&rp(r))).sum::<f64>();
                    let mut points = Points::default();
                    let polys = face_polys(&b, &f, &mut points, &HashMap::new());
                    // what is drawn: each face's triangles, its holes left open
                    let mut m = Mesh { positions: points.positions.clone(), ..Default::default() };
                    for p in &polys {
                        m.faces.push(p.ring.clone());
                        m.set_holes(m.faces.len() - 1, p.holes.clone());
                    }
                    let tri_area: f64 = (0..m.faces.len())
                        .flat_map(|f| m.triangulate_face(f))
                        .map(|t| area(&t.iter().map(|&i| points.positions[i as usize]).collect::<Vec<_>>()))
                        .sum();
                    let faces_area = tri_area;
                    all += 1;
                    let off = |x: f64| (x - want).abs() > 1e-3 * want.max(1e-6);
                    if off(faces_area) || off(tri_area) {
                        bad += 1;
                        if bad == 1 {
                            if let Ok(out) = std::env::var("DSKP_DUMP") {
                                let rings: Vec<Vec<[f64; 3]>> = f.rings.iter().map(|r| r.iter().map(|&i| b.positions[i as usize]).collect()).collect();
                                std::fs::write(out, format!("{rings:?}")).unwrap();
                            }
                        }
                        if bad <= 6 {
                            println!("{} holes: area {want:.4}, faces {faces_area:.4} ({} faces), triangles {tri_area:.4}", f.rings.len() - 1, polys.len());
                        }
                    }
                }
                _ => {}
            }
        }
        println!("faces with holes: {all}, covered wrongly: {bad}");
    }
}
