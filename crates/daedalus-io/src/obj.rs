use crate::{IoError, MeshExporter, MeshImporter};
use daedalus_core::Mesh;
use std::fmt::Write;

/// Wavefront OBJ: geometry only (v, f). Normals, UVs and materials are ignored.
pub struct Obj;

impl MeshImporter for Obj {
    fn extensions(&self) -> &'static [&'static str] {
        &["obj"]
    }

    fn import(&self, bytes: &[u8]) -> Result<Mesh, IoError> {
        let text = std::str::from_utf8(bytes).map_err(|_| IoError::Parse("OBJ is not UTF-8".into()))?;
        let mut mesh = Mesh::new();
        for line in text.lines() {
            let mut parts = line.split_whitespace();
            match parts.next() {
                Some("v") => {
                    let c: Vec<f64> = parts.take(3).filter_map(|s| s.parse().ok()).collect();
                    if c.len() != 3 {
                        return Err(IoError::Parse(format!("bad vertex: {line}")));
                    }
                    mesh.add_vertex([c[0], c[1], c[2]]);
                }
                Some("f") => {
                    let n = mesh.positions.len() as i64;
                    let mut idx = Vec::new();
                    for tok in parts {
                        let raw: i64 = tok
                            .split('/')
                            .next()
                            .and_then(|s| s.parse().ok())
                            .ok_or_else(|| IoError::Parse(format!("bad face index: {tok}")))?;
                        // OBJ is 1-based; negative indices count back from the last vertex.
                        let i = if raw < 0 { n + raw } else { raw - 1 };
                        if i < 0 || i >= n {
                            return Err(IoError::Parse(format!("face index out of range: {raw}")));
                        }
                        idx.push(i as u32);
                    }
                    if idx.len() < 3 {
                        return Err(IoError::Parse(format!("face with < 3 vertices: {line}")));
                    }
                    mesh.add_face(idx);
                }
                _ => {}
            }
        }
        Ok(mesh)
    }
}

impl MeshExporter for Obj {
    fn extensions(&self) -> &'static [&'static str] {
        &["obj"]
    }

    fn export(&self, mesh: &Mesh) -> Result<Vec<u8>, IoError> {
        let mut s = String::from("# Daedalus CAD\n");
        for p in &mesh.positions {
            let _ = writeln!(s, "v {} {} {}", p[0], p[1], p[2]);
        }
        // a face with holes goes out as its triangles (OBJ has no holes)
        for f in &mesh.polygons().0 {
            s.push('f');
            for i in f {
                let _ = write!(s, " {}", i + 1);
            }
            s.push('\n');
        }
        Ok(s.into_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let src = "v 0 0 0\nv 1 0 0\nv 1 1 0\nf 1 2 3\nf -3 -2 -1\n";
        let m = Obj.import(src.as_bytes()).unwrap();
        assert_eq!(m.faces, vec![vec![0, 1, 2], vec![0, 1, 2]]);
        let out = Obj.export(&m).unwrap();
        assert_eq!(Obj.import(&out).unwrap(), m);
    }
}
