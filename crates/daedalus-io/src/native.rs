//! `.daedalus`: Daedalus's own file. Plain UTF-8 text, so it is readable, diffable and survives a version-control merge:
//!
//! ```text
//! daedalus 1
//! # anything after # is a comment
//! v 0 0 0          one line per vertex: x y z (metres)
//! f 0 1 2 3        one line per face: vertex indices, counter-clockwise seen from outside
//! ```
//!
//! The first line names the format and its version. A reader refuses a newer version instead of guessing, and keeps working for
//! every older one. Coordinates are written with enough digits to come back bit-identical (Rust's shortest round-trip form).
//!
//! Version 1 holds the geometry only. Later versions add what the model gains (groups, materials, units, scenes) as new line
//! kinds; a version-1 reader is never asked to read them.

use crate::{IoError, MeshExporter, MeshImporter};
use daedalus_core::Mesh;

/// Version 2 adds loose lines (`l a b`); a file without lines is still written as version 1, which older Daedalus reads.
pub const VERSION: u32 = 2;
const MAGIC: &str = "daedalus";

pub struct Native;

impl MeshImporter for Native {
    fn extensions(&self) -> &'static [&'static str] {
        &["daedalus"]
    }

    fn import(&self, bytes: &[u8]) -> Result<Mesh, IoError> {
        let text = std::str::from_utf8(bytes).map_err(|_| IoError::Parse("not UTF-8 text".into()))?;
        let mut lines = text.lines().enumerate().map(|(i, l)| (i + 1, l.split('#').next().unwrap_or("").trim())).filter(|(_, l)| !l.is_empty());
        let (_, head) = lines.next().ok_or_else(|| IoError::Parse("empty file".into()))?;
        let mut head = head.split_whitespace();
        if head.next() != Some(MAGIC) {
            return Err(IoError::Parse("not a .daedalus file".into()));
        }
        let version: u32 = head.next().and_then(|v| v.parse().ok()).ok_or_else(|| IoError::Parse("no version on the first line".into()))?;
        if version > VERSION {
            return Err(IoError::Unsupported(format!("a .daedalus file of version {version}, made by a newer Daedalus (this one reads up to {VERSION})")));
        }
        let mut mesh = Mesh::new();
        for (n, line) in lines {
            let mut parts = line.split_whitespace();
            match parts.next() {
                Some("v") => {
                    let c: Vec<f32> = parts.map(|p| p.parse::<f32>()).collect::<Result<_, _>>().map_err(|_| IoError::Parse(format!("line {n}: a vertex needs three numbers")))?;
                    let [x, y, z] = c[..] else { return Err(IoError::Parse(format!("line {n}: a vertex needs three numbers"))) };
                    if !(x.is_finite() && y.is_finite() && z.is_finite()) {
                        return Err(IoError::Parse(format!("line {n}: not a finite number")));
                    }
                    mesh.add_vertex([x, y, z]);
                }
                Some("f") => {
                    let idx: Vec<u32> = parts.map(|p| p.parse::<u32>()).collect::<Result<_, _>>().map_err(|_| IoError::Parse(format!("line {n}: a face lists vertex numbers")))?;
                    if idx.len() < 3 {
                        return Err(IoError::Parse(format!("line {n}: a face needs at least three vertices")));
                    }
                    if let Some(bad) = idx.iter().find(|&&i| i as usize >= mesh.positions.len()) {
                        return Err(IoError::Parse(format!("line {n}: vertex {bad} is not defined above")));
                    }
                    mesh.add_face(idx);
                }
                // a line kind this version does not know: only possible in a file that claims a version it does not have
                Some("l") if version >= 2 => {
                    let idx: Vec<u32> = parts.map(|p| p.parse::<u32>()).collect::<Result<_, _>>().map_err(|_| IoError::Parse(format!("line {n}: a line lists two vertex numbers")))?;
                    let [a, b] = idx[..] else { return Err(IoError::Parse(format!("line {n}: a line lists two vertex numbers"))) };
                    if a.max(b) as usize >= mesh.positions.len() {
                        return Err(IoError::Parse(format!("line {n}: vertex {} is not defined above", a.max(b))));
                    }
                    mesh.lines.push([a, b]);
                }
                Some(other) => return Err(IoError::Parse(format!("line {n}: unknown entry {other:?}"))),
                None => {}
            }
        }
        Ok(mesh)
    }
}

impl MeshExporter for Native {
    fn extensions(&self) -> &'static [&'static str] {
        &["daedalus"]
    }

    fn export(&self, mesh: &Mesh) -> Result<Vec<u8>, IoError> {
        let version = if mesh.lines.is_empty() { 1 } else { VERSION };
        let mut s = format!("{MAGIC} {version}\n# Daedalus CAD model: v x y z (metres), f vertex numbers counter-clockwise from outside, l a loose line\n");
        for p in &mesh.positions {
            s.push_str(&format!("v {} {} {}\n", p[0], p[1], p[2]));
        }
        for f in &mesh.faces {
            s.push('f');
            for i in f {
                s.push_str(&format!(" {i}"));
            }
            s.push('\n');
        }
        for [a, b] in &mesh.lines {
            s.push_str(&format!("l {a} {b}\n"));
        }
        Ok(s.into_bytes())
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
        let bytes = Native.export(&m).unwrap();
        assert!(String::from_utf8(bytes.clone()).unwrap().starts_with("daedalus 2\n"));
        let back = Native.import(&bytes).unwrap();
        assert_eq!(back, m);
    }

    #[test]
    fn bad_files_are_refused_with_the_line() {
        let e = |t: &str| Native.import(t.as_bytes()).unwrap_err().to_string();
        assert!(e("hello\n").contains("not a .daedalus file"));
        assert!(e("daedalus 3\n").contains("newer Daedalus"), "{}", e("daedalus 3\n"));
        assert!(e("daedalus 1\nv 0 0 0\nv 1 0 0\nl 0 1\n").contains("unknown entry"), "lines need version 2");
        assert!(e("daedalus 2\nv 0 0 0\nl 0 1\n").contains("vertex 1 is not defined"));
        assert!(e("daedalus 1\nv 0 0\n").contains("line 2"));
        assert!(e("daedalus 1\nv 0 0 0\nf 0 1 2\n").contains("vertex 1 is not defined"));
        assert!(e("daedalus 1\nv 0 0 NaN\n").contains("finite"));
        assert!(e("daedalus 1\nx 1\n").contains("unknown entry"));
        assert!(Native.import(b"daedalus 1\n# a comment\n\nv 0 0 0 # here too\n").is_ok());
    }
}
