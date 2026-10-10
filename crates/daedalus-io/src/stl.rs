use crate::{IoError, MeshExporter, MeshImporter};
use daedalus_core::Mesh;
use crate::weld::Welder;

pub struct Stl;

fn is_binary(bytes: &[u8]) -> bool {
    if bytes.len() < 84 {
        return false;
    }
    let n = u32::from_le_bytes([bytes[80], bytes[81], bytes[82], bytes[83]]) as usize;
    84 + n * 50 == bytes.len()
}

fn f32_at(b: &[u8], o: usize) -> f32 {
    f32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

impl MeshImporter for Stl {
    fn extensions(&self) -> &'static [&'static str] {
        &["stl"]
    }

    fn import(&self, bytes: &[u8]) -> Result<Mesh, IoError> {
        let mut mesh = Mesh::new();
        let mut welder = Welder::default();

        if is_binary(bytes) {
            let n = u32::from_le_bytes([bytes[80], bytes[81], bytes[82], bytes[83]]) as usize;
            for i in 0..n {
                let base = 84 + i * 50 + 12; // skip the normal
                let mut idx = Vec::with_capacity(3);
                for v in 0..3 {
                    let o = base + v * 12;
                    let p = [f32_at(bytes, o), f32_at(bytes, o + 4), f32_at(bytes, o + 8)];
                    idx.push(welder.vertex(&mut mesh, p));
                }
                mesh.add_face(idx);
            }
        } else {
            let text = std::str::from_utf8(bytes)
                .map_err(|_| IoError::Parse("STL is neither valid binary nor UTF-8 ASCII".into()))?;
            let mut current: Vec<u32> = Vec::new();
            for line in text.lines() {
                let mut parts = line.split_whitespace();
                match parts.next() {
                    Some("vertex") => {
                        let c: Vec<f32> = parts.filter_map(|s| s.parse().ok()).collect();
                        if c.len() != 3 {
                            return Err(IoError::Parse(format!("bad vertex line: {line}")));
                        }
                        current.push(welder.vertex(&mut mesh, [c[0], c[1], c[2]]));
                    }
                    Some("endfacet") => {
                        if current.len() != 3 {
                            return Err(IoError::Parse("facet without 3 vertices".into()));
                        }
                        mesh.add_face(std::mem::take(&mut current));
                    }
                    _ => {}
                }
            }
        }

        if mesh.faces.is_empty() {
            return Err(IoError::Parse("STL contains no triangles".into()));
        }
        Ok(mesh)
    }
}

impl MeshExporter for Stl {
    fn extensions(&self) -> &'static [&'static str] {
        &["stl"]
    }

    /// Writes binary STL.
    fn export(&self, mesh: &Mesh) -> Result<Vec<u8>, IoError> {
        let tris = mesh.triangles();
        let mut out = vec![0u8; 80];
        out.extend_from_slice(&(tris.len() as u32).to_le_bytes());
        for t in tris {
            let p: Vec<[f32; 3]> = t.iter().map(|&i| mesh.positions[i as usize]).collect();
            let u = [p[1][0] - p[0][0], p[1][1] - p[0][1], p[1][2] - p[0][2]];
            let v = [p[2][0] - p[0][0], p[2][1] - p[0][1], p[2][2] - p[0][2]];
            let mut n = [
                u[1] * v[2] - u[2] * v[1],
                u[2] * v[0] - u[0] * v[2],
                u[0] * v[1] - u[1] * v[0],
            ];
            let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            if len > 0.0 {
                n.iter_mut().for_each(|c| *c /= len);
            }
            for c in n.iter().chain(p.iter().flatten()) {
                out.extend_from_slice(&c.to_le_bytes());
            }
            out.extend_from_slice(&[0, 0]);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_round_trip_welds_vertices() {
        let mut m = Mesh::new();
        let a = m.add_vertex([0.0, 0.0, 0.0]);
        let b = m.add_vertex([1.0, 0.0, 0.0]);
        let c = m.add_vertex([1.0, 1.0, 0.0]);
        let d = m.add_vertex([0.0, 1.0, 0.0]);
        m.add_face(vec![a, b, c]);
        m.add_face(vec![a, c, d]);
        let bytes = Stl.export(&m).unwrap();
        let back = Stl.import(&bytes).unwrap();
        assert_eq!(back.faces.len(), 2);
        assert_eq!(back.positions.len(), 4);
    }

    #[test]
    fn ascii_parses() {
        let s = "solid t\nfacet normal 0 0 1\nouter loop\nvertex 0 0 0\nvertex 1 0 0\nvertex 0 1 0\nendloop\nendfacet\nendsolid t\n";
        let m = Stl.import(s.as_bytes()).unwrap();
        assert_eq!(m.faces.len(), 1);
    }
}
