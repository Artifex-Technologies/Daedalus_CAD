use crate::weld::Welder;
use crate::{IoError, MeshExporter, MeshImporter};
use daedalus_core::Mesh;

pub struct Stl;

fn is_binary(bytes: &[u8]) -> bool {
    if bytes.len() < 84 {
        return false;
    }
    let n = u32::from_le_bytes([bytes[80], bytes[81], bytes[82], bytes[83]]) as usize;
    84 + n * 50 == bytes.len()
}

fn f32_at(b: &[u8], o: usize) -> f64 {
    f32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]) as f64
}

impl MeshImporter for Stl {
    fn extensions(&self) -> &'static [&'static str] {
        &["stl"]
    }

    fn import(&self, bytes: &[u8]) -> Result<Mesh, IoError> {
        let mut triangles: Vec<[[f64; 3]; 3]> = Vec::new();
        if is_binary(bytes) {
            let n = u32::from_le_bytes([bytes[80], bytes[81], bytes[82], bytes[83]]) as usize;
            for i in 0..n {
                let base = 84 + i * 50 + 12; // skip the normal
                let corner = |v: usize| {
                    let o = base + v * 12;
                    [f32_at(bytes, o), f32_at(bytes, o + 4), f32_at(bytes, o + 8)]
                };
                triangles.push([corner(0), corner(1), corner(2)]);
            }
        } else {
            let text = std::str::from_utf8(bytes).map_err(|_| IoError::Parse("STL is neither valid binary nor UTF-8 ASCII".into()))?;
            let mut current: Vec<[f64; 3]> = Vec::new();
            for line in text.lines() {
                let mut parts = line.split_whitespace();
                match parts.next() {
                    Some("vertex") => {
                        let c: Vec<f64> = parts.filter_map(|s| s.parse().ok()).collect();
                        if c.len() != 3 {
                            return Err(IoError::Parse(format!("bad vertex line: {line}")));
                        }
                        current.push([c[0], c[1], c[2]]);
                    }
                    Some("endfacet") => {
                        if current.len() != 3 {
                            return Err(IoError::Parse("facet without 3 vertices".into()));
                        }
                        triangles.push([current[0], current[1], current[2]]);
                        current.clear();
                    }
                    _ => {}
                }
            }
        }

        // STL repeats each corner in every triangle, and exporters write the same point a little differently (rounded to
        // f32, or printed with fewer digits): corners closer than a millionth of the model's size are one vertex. A
        // triangle whose corners become one or two points is left out.
        let mut lo = [f64::INFINITY; 3];
        let mut hi = [f64::NEG_INFINITY; 3];
        for p in triangles.iter().flatten().filter(|p| p.iter().all(|c| c.is_finite())) {
            for a in 0..3 {
                lo[a] = lo[a].min(p[a]);
                hi[a] = hi[a].max(p[a]);
            }
        }
        let diagonal = ((hi[0] - lo[0]).powi(2) + (hi[1] - lo[1]).powi(2) + (hi[2] - lo[2]).powi(2)).sqrt();
        let mut welder = Welder::within(diagonal * WELD_FRACTION);
        let mut mesh = Mesh::new();
        for t in &triangles {
            let idx: Vec<u32> = t.iter().map(|p| welder.vertex(&mut mesh, *p)).collect();
            if idx[0] != idx[1] && idx[1] != idx[2] && idx[0] != idx[2] {
                mesh.add_face(idx);
            }
        }
        if mesh.faces.is_empty() {
            return Err(IoError::Parse("STL contains no triangles".into()));
        }
        Ok(mesh)
    }
}

/// Corners closer than this share of the model's bounding-box diagonal are welded on import.
const WELD_FRACTION: f64 = 1e-6;

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
            let p: Vec<[f64; 3]> = t.iter().map(|&i| mesh.positions[i as usize]).collect();
            let u = [p[1][0] - p[0][0], p[1][1] - p[0][1], p[1][2] - p[0][2]];
            let v = [p[2][0] - p[0][0], p[2][1] - p[0][1], p[2][2] - p[0][2]];
            let mut n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
            let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            if len > 0.0 {
                n.iter_mut().for_each(|c| *c /= len);
            }
            for c in n.iter().chain(p.iter().flatten()) {
                out.extend_from_slice(&(*c as f32).to_le_bytes()); // binary STL holds 32-bit floats
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

    /// Near-equal corners (DAE-21): an ASCII STL whose shared corners differ by under a micrometre is one quad of four vertices;
    /// corners a thousandth apart stay apart; a triangle that welds to a line is left out.
    #[test]
    fn near_equal_corners_are_welded() {
        let facet =
            |a: &str, b: &str, c: &str| format!("facet normal 0 0 1\nouter loop\nvertex {a}\nvertex {b}\nvertex {c}\nendloop\nendfacet\n");
        let s = format!(
            "solid t\n{}{}{}endsolid t\n",
            facet("0 0 0", "1000 0 0", "1000 1000 0"),
            facet("0.0000001 0 0", "1000.0000004 999.9999996 0", "0 1000 0"),
            // a sliver: two corners a ten-millionth of a millimetre apart on a metre-sized model
            facet("0 1000 0", "0 1000.0000001 0", "500 500 0"),
        );
        let m = Stl.import(s.as_bytes()).unwrap();
        assert_eq!(m.positions.len(), 5, "{:?}", m.positions);
        assert_eq!(m.faces.len(), 2, "the sliver is gone: {:?}", m.faces);
        let apart = format!("solid t\n{}{}endsolid t\n", facet("0 0 0", "1 0 0", "1 1 0"), facet("0.001 0 0", "1 1 0", "0 1 0"));
        assert_eq!(Stl.import(apart.as_bytes()).unwrap().positions.len(), 5);
    }

    #[test]
    fn ascii_parses() {
        let s = "solid t\nfacet normal 0 0 1\nouter loop\nvertex 0 0 0\nvertex 1 0 0\nvertex 0 1 0\nendloop\nendfacet\nendsolid t\n";
        let m = Stl.import(s.as_bytes()).unwrap();
        assert_eq!(m.faces.len(), 1);
    }
}
