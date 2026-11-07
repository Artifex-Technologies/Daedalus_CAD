//! `.daedalus`: Daedalus's own file. Plain UTF-8 text, so it is readable, diffable and survives a version-control merge:
//!
//! ```text
//! daedalus 4
//! # anything after # is a comment (outside "quotes")
//! v 0 0 0          one line per vertex: x y z (metres)
//! f 0 1 2 3        one line per face: vertex indices, counter-clockwise seen from outside
//! h 4 7 6 5        a hole in the face above (a window in a wall): its vertices the other way round (version 4)
//! l 0 4            a loose line (version 2)
//! s 0 1            a hidden seam: a face edge not drawn (version 2)
//! c 2 200 80 40    face 2 painted red, green, blue (version 2)
//! i 0 1 0 0 2  0 1 0 0  0 0 1 0     an instance of definition 0: three rows of its transform (version 3)
//! def "Group#1" group               a definition: its own v, f, l, s, c, i lines up to "end" (version 3)
//! end
//! guide line 0 0 0 1 0 0            notes (version 3): guide point x y z, guide line x y z dx dy dz,
//! dim 0 0 0 1 0 0 0 -0.3 0          dim a b offset, text point at "words", section point normal on|off (one line each, one on at most),
//! axes 0 0 0 1 0 0 0 1 0 0 0 1      axes origin red green blue
//! ```
//!
//! The first line names the format and its version. A reader refuses a newer version instead of guessing, and keeps working for
//! every older one. Coordinates are written with enough digits to come back bit-identical (Rust's shortest round-trip form).
//! Vertex numbers count from 0 in each block (the top level, each definition). Definitions are numbered in the order they
//! appear. A file is written in the oldest version that holds everything in it.

use crate::{IoError, MeshExporter, MeshImporter};
use daedalus_core::scene::{Axes, Context, Definition, Dimension, Guide, Instance, Label, Scene, Section, Xform};
use daedalus_core::Mesh;

/// Version 2 added loose lines, seams and colours; version 3 groups and components (`def`, `i`) and notes.
pub const VERSION: u32 = 4;
const MAGIC: &str = "daedalus";

pub struct Native;

/// A line's words, with "quoted text" as one word (`\"` and `\\` inside); `#` outside quotes ends the line.
fn words(line: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut chars = line.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
        } else if c == '#' {
            break;
        } else if c == '"' {
            chars.next();
            let mut w = String::new();
            loop {
                match chars.next() {
                    Some('"') => break,
                    Some('\\') => match chars.next() {
                        Some('n') => w.push('\n'),
                        Some(e) => w.push(e),
                        None => return Err("a \\ at the end of the line".into()),
                    },
                    Some(ch) => w.push(ch),
                    None => return Err("text in quotes is not closed".into()),
                }
            }
            out.push(w);
        } else {
            let mut w = String::new();
            while let Some(&ch) = chars.peek() {
                if ch.is_whitespace() || ch == '#' {
                    break;
                }
                w.push(ch);
                chars.next();
            }
            out.push(w);
        }
    }
    Ok(out)
}

fn quote(t: &str) -> String {
    let mut s = String::from("\"");
    for c in t.chars() {
        match c {
            '"' => s.push_str("\\\""),
            '\\' => s.push_str("\\\\"),
            '\n' => s.push_str("\\n"),
            c => s.push(c),
        }
    }
    s.push('"');
    s
}

fn numbers(parts: &[String], n: usize, what: &str, line: usize) -> Result<Vec<f64>, IoError> {
    let c: Vec<f64> = parts.iter().map(|p| p.parse::<f64>()).collect::<Result<_, _>>().map_err(|_| IoError::Parse(format!("line {line}: {what}")))?;
    if c.len() != n {
        return Err(IoError::Parse(format!("line {line}: {what}")));
    }
    if c.iter().any(|x| !x.is_finite()) {
        return Err(IoError::Parse(format!("line {line}: not a finite number")));
    }
    Ok(c)
}

fn indices(parts: &[String], what: &str, line: usize) -> Result<Vec<u32>, IoError> {
    parts.iter().map(|p| p.parse::<u32>()).collect::<Result<_, _>>().map_err(|_| IoError::Parse(format!("line {line}: {what}")))
}

fn p3(c: &[f64]) -> [f64; 3] {
    [c[0], c[1], c[2]]
}

/// Reads a whole `.daedalus` file: geometry, groups and components, notes.
pub fn read_scene(bytes: &[u8]) -> Result<Scene, IoError> {
    let text = std::str::from_utf8(bytes).map_err(|_| IoError::Parse("not UTF-8 text".into()))?;
    let mut lines = Vec::new();
    for (i, l) in text.lines().enumerate() {
        let w = words(l).map_err(|e| IoError::Parse(format!("line {}: {e}", i + 1)))?;
        if !w.is_empty() {
            lines.push((i + 1, w));
        }
    }
    let mut lines = lines.into_iter();
    let (_, head) = lines.next().ok_or_else(|| IoError::Parse("empty file".into()))?;
    if head.first().map(String::as_str) != Some(MAGIC) {
        return Err(IoError::Parse("not a .daedalus file".into()));
    }
    let version: u32 = head.get(1).and_then(|v| v.parse().ok()).ok_or_else(|| IoError::Parse("no version on the first line".into()))?;
    if version > VERSION {
        return Err(IoError::Unsupported(format!("a .daedalus file of version {version}, made by a newer Daedalus (this one reads up to {VERSION})")));
    }
    let mut scene = Scene::default();
    // the block being read: None the top level, Some(d) definition d
    let mut block: Option<usize> = None;
    for (n, w) in lines {
        let kind = w[0].as_str();
        let rest = &w[1..];
        let v2 = version >= 2;
        let v3 = version >= 3;
        let v4 = version >= 4;
        let ctx: &mut Context = match block {
            None => &mut scene.root,
            Some(d) => &mut scene.defs[d].ctx,
        };
        let mesh = &mut ctx.mesh;
        match kind {
            "v" => {
                let c = numbers(rest, 3, "a vertex needs three numbers", n)?;
                mesh.add_vertex(p3(&c));
            }
            "f" => {
                let idx = indices(rest, "a face lists vertex numbers", n)?;
                if idx.len() < 3 {
                    return Err(IoError::Parse(format!("line {n}: a face needs at least three vertices")));
                }
                if let Some(bad) = idx.iter().find(|&&i| i as usize >= mesh.positions.len()) {
                    return Err(IoError::Parse(format!("line {n}: vertex {bad} is not defined above")));
                }
                mesh.add_face(idx);
            }
            "h" if v4 => {
                let idx = indices(rest, "a hole lists vertex numbers", n)?;
                if idx.len() < 3 || mesh.faces.is_empty() {
                    return Err(IoError::Parse(format!("line {n}: a hole needs three vertices and a face above it")));
                }
                if let Some(bad) = idx.iter().find(|&&i| i as usize >= mesh.positions.len()) {
                    return Err(IoError::Parse(format!("line {n}: vertex {bad} is not defined above")));
                }
                let f = mesh.faces.len() - 1;
                let mut holes = mesh.holes_of(f).to_vec();
                holes.push(idx);
                mesh.set_holes(f, holes);
            }
            "l" | "s" if v2 => {
                let idx = indices(rest, "a line lists two vertex numbers", n)?;
                let [a, b] = idx[..] else { return Err(IoError::Parse(format!("line {n}: a line lists two vertex numbers"))) };
                if a.max(b) as usize >= mesh.positions.len() {
                    return Err(IoError::Parse(format!("line {n}: vertex {} is not defined above", a.max(b))));
                }
                if kind == "l" { mesh.lines.push([a, b]) } else { mesh.seams.push([a, b]) }
            }
            "c" if v2 => {
                let v = indices(rest, "a colour is a face number and red, green, blue", n)?;
                let [f, r, g, b] = v[..] else { return Err(IoError::Parse(format!("line {n}: a colour is a face number and red, green, blue"))) };
                if f as usize >= mesh.faces.len() || r.max(g).max(b) > 255 {
                    return Err(IoError::Parse(format!("line {n}: face {f} is not defined above, or a colour is over 255")));
                }
                mesh.set_color(f as usize, Some([r as u8, g as u8, b as u8]));
            }
            "i" if v3 => {
                let what = "an instance is a definition number and twelve numbers";
                let def: usize = rest.first().and_then(|d| d.parse().ok()).ok_or_else(|| IoError::Parse(format!("line {n}: {what}")))?;
                let c = numbers(&rest[1..], 12, what, n)?;
                let xf = Xform([[c[0], c[1], c[2], c[3]], [c[4], c[5], c[6], c[7]], [c[8], c[9], c[10], c[11]]]);
                ctx.instances.push(Instance { def, xf });
            }
            "def" if v3 => {
                if block.is_some() {
                    return Err(IoError::Parse(format!("line {n}: a definition starts inside another (no \"end\" before it)")));
                }
                let name = rest.first().cloned().unwrap_or_default();
                let component = match rest.get(1).map(String::as_str) {
                    Some("component") => true,
                    Some("group") | None => false,
                    Some(o) => return Err(IoError::Parse(format!("line {n}: a definition is a group or a component, not {o:?}"))),
                };
                scene.defs.push(Definition { name, component, ctx: Context::default() });
                block = Some(scene.defs.len() - 1);
            }
            "end" if v3 => {
                if block.take().is_none() {
                    return Err(IoError::Parse(format!("line {n}: \"end\" with no definition open")));
                }
            }
            "guide" if v3 => match rest.first().map(String::as_str) {
                Some("point") => scene.notes.guides.push(Guide::Point(p3(&numbers(&rest[1..], 3, "a guide point is x y z", n)?))),
                Some("line") => {
                    let c = numbers(&rest[1..], 6, "a guide line is a point and a direction", n)?;
                    scene.notes.guides.push(Guide::Line(p3(&c), p3(&c[3..])));
                }
                _ => return Err(IoError::Parse(format!("line {n}: a guide is a point or a line"))),
            },
            "dim" if v3 => {
                let c = numbers(rest, 9, "a dimension is two points and an offset", n)?;
                scene.notes.dims.push(Dimension { a: p3(&c), b: p3(&c[3..]), offset: p3(&c[6..]) });
            }
            "text" if v3 => {
                let what = "text is a point, where it is shown, and the words in quotes";
                if rest.len() != 7 {
                    return Err(IoError::Parse(format!("line {n}: {what}")));
                }
                let c = numbers(&rest[..6], 6, what, n)?;
                scene.notes.labels.push(Label { point: p3(&c), at: p3(&c[3..]), text: rest[6].clone() });
            }
            "section" if v3 => {
                let what = "a section is a point, a normal and on or off";
                if rest.len() != 7 {
                    return Err(IoError::Parse(format!("line {n}: {what}")));
                }
                let c = numbers(&rest[..6], 6, what, n)?;
                scene.notes.sections.push(Section { point: p3(&c), normal: p3(&c[3..]), active: rest[6] != "off" });
            }
            "axes" if v3 => {
                let c = numbers(rest, 12, "axes are an origin and three directions", n)?;
                scene.notes.axes = Axes { origin: p3(&c), dirs: [p3(&c[3..]), p3(&c[6..]), p3(&c[9..])] };
            }
            other => return Err(IoError::Parse(format!("line {n}: unknown entry {other:?}"))),
        }
    }
    if block.is_some() {
        return Err(IoError::Parse("the last definition has no \"end\"".into()));
    }
    // saved before faces had holes: a face with a hole was two faces joined by seams; one face with its hole again
    if version < 4 {
        scene.root.mesh.join_seamed_faces();
        for d in &mut scene.defs {
            d.ctx.mesh.join_seamed_faces();
        }
    }
    // every instance names a definition that is there, and no definition holds itself
    let count = scene.defs.len();
    let all = std::iter::once(&scene.root).chain(scene.defs.iter().map(|d| &d.ctx));
    if all.flat_map(|c| &c.instances).any(|i| i.def >= count) {
        return Err(IoError::Parse("an instance names a definition that is not in the file".into()));
    }
    for d in 0..count {
        let mut seen = vec![false; count];
        let mut todo = vec![d];
        while let Some(x) = todo.pop() {
            for i in &scene.defs[x].ctx.instances {
                if i.def == d {
                    return Err(IoError::Parse(format!("definition {d} holds itself")));
                }
                if !seen[i.def] {
                    seen[i.def] = true;
                    todo.push(i.def);
                }
            }
        }
    }
    Ok(scene)
}

fn write_mesh(s: &mut String, mesh: &Mesh) {
    for p in &mesh.positions {
        s.push_str(&format!("v {} {} {}\n", p[0], p[1], p[2]));
    }
    for (fi, f) in mesh.faces.iter().enumerate() {
        s.push('f');
        for i in f {
            s.push_str(&format!(" {i}"));
        }
        s.push('\n');
        for h in mesh.holes_of(fi) {
            s.push('h');
            for i in h {
                s.push_str(&format!(" {i}"));
            }
            s.push('\n');
        }
    }
    for [a, b] in &mesh.lines {
        s.push_str(&format!("l {a} {b}\n"));
    }
    for [a, b] in &mesh.seams {
        s.push_str(&format!("s {a} {b}\n"));
    }
    for (f, c) in mesh.colors.iter().enumerate() {
        if let Some([r, g, b]) = c {
            s.push_str(&format!("c {f} {r} {g} {b}\n"));
        }
    }
}

fn write_instances(s: &mut String, instances: &[Instance]) {
    for i in instances {
        s.push_str(&format!("i {}", i.def));
        for row in &i.xf.0 {
            for c in row {
                s.push_str(&format!(" {c}"));
            }
        }
        s.push('\n');
    }
}

fn v3(p: [f64; 3]) -> String {
    format!("{} {} {}", p[0], p[1], p[2])
}

/// Writes a whole scene, in the oldest version that holds it.
pub fn write_scene(scene: &Scene) -> Vec<u8> {
    let extras = |m: &Mesh| !m.lines.is_empty() || !m.seams.is_empty() || m.colors.iter().any(|c| c.is_some());
    let holes = std::iter::once(&scene.root).chain(scene.defs.iter().map(|d| &d.ctx)).any(|c| c.mesh.holes.iter().any(|h| !h.is_empty()));
    let version = if holes {
        4
    } else if !scene.is_plain() || !scene.defs.is_empty() {
        3
    } else if extras(&scene.root.mesh) {
        2
    } else {
        1
    };
    let mut s = format!("{MAGIC} {version}\n# Daedalus CAD model: v x y z (metres), f vertex numbers counter-clockwise from outside, l a loose line\n");
    write_mesh(&mut s, &scene.root.mesh);
    write_instances(&mut s, &scene.root.instances);
    for d in &scene.defs {
        s.push_str(&format!("def {} {}\n", quote(&d.name), if d.component { "component" } else { "group" }));
        write_mesh(&mut s, &d.ctx.mesh);
        write_instances(&mut s, &d.ctx.instances);
        s.push_str("end\n");
    }
    let n = &scene.notes;
    for g in &n.guides {
        match g {
            Guide::Point(p) => s.push_str(&format!("guide point {}\n", v3(*p))),
            Guide::Line(p, d) => s.push_str(&format!("guide line {} {}\n", v3(*p), v3(*d))),
        }
    }
    for d in &n.dims {
        s.push_str(&format!("dim {} {} {}\n", v3(d.a), v3(d.b), v3(d.offset)));
    }
    for l in &n.labels {
        s.push_str(&format!("text {} {} {}\n", v3(l.point), v3(l.at), quote(&l.text)));
    }
    for sec in &n.sections {
        s.push_str(&format!("section {} {} {}\n", v3(sec.point), v3(sec.normal), if sec.active { "on" } else { "off" }));
    }
    if !n.axes.is_default() {
        let a = n.axes;
        s.push_str(&format!("axes {} {} {} {}\n", v3(a.origin), v3(a.dirs[0]), v3(a.dirs[1]), v3(a.dirs[2])));
    }
    s.into_bytes()
}

impl MeshImporter for Native {
    fn extensions(&self) -> &'static [&'static str] {
        &["daedalus"]
    }

    /// The file's geometry as one mesh (groups and components put in place).
    fn import(&self, bytes: &[u8]) -> Result<Mesh, IoError> {
        Ok(read_scene(bytes)?.flatten())
    }
}

impl MeshExporter for Native {
    fn extensions(&self) -> &'static [&'static str] {
        &["daedalus"]
    }

    fn export(&self, mesh: &Mesh) -> Result<Vec<u8>, IoError> {
        Ok(write_scene(&Scene::from_mesh(mesh.clone())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cube() -> Mesh {
        let mut m = Mesh::new();
        for p in [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 1.0], [1.0, 1.0, 1.0], [0.0, 1.0, 1.0]] {
            m.add_vertex(p);
        }
        for f in [vec![0, 3, 2, 1], vec![4, 5, 6, 7], vec![0, 1, 5, 4], vec![1, 2, 6, 5], vec![2, 3, 7, 6], vec![3, 0, 4, 7]] {
            m.add_face(f);
        }
        m
    }

    #[test]
    fn a_model_comes_back_exactly() {
        let mut m = cube();
        m.positions[6] = [0.1 + 0.2, 1.0 / 3.0, -2.5e-7];
        let bytes = Native.export(&m).unwrap();
        let back = Native.import(&bytes).unwrap();
        assert_eq!(back.positions, m.positions, "every coordinate bit-identical");
        assert_eq!(back.faces, m.faces);
        assert!(String::from_utf8(bytes).unwrap().starts_with("daedalus 1\n"));
    }

    #[test]
    fn lines_come_back_in_a_version_2_file() {
        let mut m = cube();
        m.add_vertex([5.0, 0.0, 0.0]);
        m.lines.push([0, 8]);
        m.seams.push([0, 1]);
        m.set_color(2, Some([10, 200, 30]));
        let bytes = Native.export(&m).unwrap();
        assert!(String::from_utf8(bytes.clone()).unwrap().starts_with("daedalus 2\n"));
        let back = Native.import(&bytes).unwrap();
        assert_eq!(back, m);
    }

    #[test]
    fn bad_files_are_refused_with_the_line() {
        let e = |t: &str| Native.import(t.as_bytes()).unwrap_err().to_string();
        assert!(e("hello\n").contains("not a .daedalus file"));
        assert!(e("daedalus 5\n").contains("newer Daedalus"), "{}", e("daedalus 5\n"));
        assert!(e("daedalus 3\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 0 1 2\nh 0 1 2\n").contains("unknown entry"), "holes need version 4");
        assert!(e("daedalus 1\nv 0 0 0\nv 1 0 0\nl 0 1\n").contains("unknown entry"), "lines need version 2");
        assert!(e("daedalus 2\nv 0 0 0\nl 0 1\n").contains("vertex 1 is not defined"));
        assert!(e("daedalus 1\nv 0 0\n").contains("line 2"));
        assert!(e("daedalus 1\nv 0 0 0\nf 0 1 2\n").contains("vertex 1 is not defined"));
        assert!(e("daedalus 1\nv 0 0 NaN\n").contains("finite"));
        assert!(e("daedalus 1\nx 1\n").contains("unknown entry"));
        assert!(Native.import(b"daedalus 1\n# a comment\n\nv 0 0 0 # here too\n").is_ok());
    }

    #[test]
    fn groups_components_and_notes_come_back() {
        let mut s = Scene::from_mesh(cube());
        s.defs.push(Definition { name: "Shelf \"A\"".into(), component: true, ctx: Context { mesh: cube(), instances: vec![] } });
        let turn = Xform::from_fn(|p| [-p[1] + 2.0, p[0], p[2] + 0.5]);
        s.defs.push(Definition { name: "Group#1".into(), component: false, ctx: Context { mesh: Mesh::new(), instances: vec![Instance { def: 0, xf: turn }] } });
        s.root.instances.push(Instance { def: 1, xf: Xform::translation([3.0, 0.0, 0.0]) });
        s.root.instances.push(Instance { def: 0, xf: Xform::IDENTITY });
        s.notes.guides = vec![Guide::Point([1.0, 2.0, 3.0]), Guide::Line([0.0; 3], [0.0, 1.0, 0.0])];
        s.notes.dims.push(Dimension { a: [0.0; 3], b: [1.0, 0.0, 0.0], offset: [0.0, -0.3, 0.0] });
        s.notes.labels.push(Label { point: [1.0; 3], at: [1.5, 1.5, 2.0], text: "Area # 1\nsecond \\ line".into() });
        s.notes.sections.push(Section { point: [0.5, 0.0, 0.0], normal: [1.0, 0.0, 0.0], active: false });
        s.notes.sections.push(Section { point: [0.0, 0.0, 1.0], normal: [0.0, 0.0, 1.0], active: true });
        s.notes.axes = Axes { origin: [1.0, 1.0, 0.0], dirs: [[0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]] };
        let bytes = write_scene(&s);
        assert!(String::from_utf8(bytes.clone()).unwrap().starts_with("daedalus 3\n"));
        assert_eq!(read_scene(&bytes).unwrap(), s);
        // read as a plain mesh, the instances are put in place
        assert_eq!(Native.import(&bytes).unwrap().faces.len(), 6 * 3);
    }

    #[test]
    fn broken_definitions_are_refused() {
        let e = |t: &str| read_scene(t.as_bytes()).unwrap_err().to_string();
        assert!(e("daedalus 3\ni 4 1 0 0 0 0 1 0 0 0 0 1 0\n").contains("not in the file"));
        assert!(e("daedalus 3\ndef \"a\" group\ni 0 1 0 0 0 0 1 0 0 0 0 1 0\nend\n").contains("holds itself"));
        assert!(e("daedalus 3\ndef \"a\" group\n").contains("no \"end\""));
        assert!(e("daedalus 2\ndef \"a\" group\nend\n").contains("unknown entry"), "definitions need version 3");
        assert!(e("daedalus 3\ntext 0 0 0 0 0 0 \"open\n").contains("not closed"));
    }

    #[test]
    fn a_face_with_a_hole_is_saved_and_read_back() {
        let mut m = Mesh::new();
        for p in [[0.0, 0.0, 0.0], [4.0, 0.0, 0.0], [4.0, 4.0, 0.0], [0.0, 4.0, 0.0], [1.0, 1.0, 0.0], [3.0, 1.0, 0.0], [3.0, 3.0, 0.0], [1.0, 3.0, 0.0]] {
            m.add_vertex(p);
        }
        m.add_face(vec![0, 1, 2, 3]);
        m.set_holes(0, vec![vec![4, 7, 6, 5]]);
        m.add_face(vec![4, 5, 6, 7]);
        let bytes = write_scene(&Scene::from_mesh(m.clone()));
        assert!(std::str::from_utf8(&bytes).unwrap().starts_with("daedalus 4\n"));
        let back = read_scene(&bytes).unwrap().root.mesh;
        assert_eq!(back.faces, m.faces);
        assert_eq!(back.holes_of(0), m.holes_of(0));
        assert!(back.holes_of(1).is_empty());
    }

    #[test]
    fn an_older_file_with_a_seamed_ring_opens_with_a_true_hole() {
        // version 2: the ring as two faces and two seams, as Daedalus saved it before faces had holes
        let old = "daedalus 2\nv 0 0 0\nv 9 0 0\nv 9 9 0\nv 0 9 0\nv 2 2 0\nv 7 2 0\nv 7 7 0\nv 2 7 0\n";
        let mut m = Mesh::new();
        for p in [[0.0, 0.0, 0.0], [9.0, 0.0, 0.0], [9.0, 9.0, 0.0], [0.0, 9.0, 0.0], [2.0, 2.0, 0.0], [7.0, 2.0, 0.0], [7.0, 7.0, 0.0], [2.0, 7.0, 0.0]] {
            m.add_vertex(p);
        }
        m.add_face(vec![0, 1, 2, 3]);
        daedalus_core::imprint::cut_hole_avoiding(&mut m, 0, &[4, 5, 6, 7], &[]).unwrap();
        let mut text = old.to_string();
        for f in &m.faces {
            text.push_str(&format!("f {}\n", f.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(" ")));
        }
        for [a, b] in &m.seams {
            text.push_str(&format!("s {a} {b}\n"));
        }
        let back = read_scene(text.as_bytes()).unwrap().root.mesh;
        assert_eq!(back.faces.len(), 2);
        assert!(back.seams.is_empty());
        assert_eq!(back.holes.iter().map(Vec::len).sum::<usize>(), 1);
    }
}
