use crate::weld::Welder;
use crate::{IoError, MeshExporter, MeshImporter};
use daedalus_core::Mesh;
use std::fmt::Write;

/// ASCII DXF. Reads `3DFACE` entities and polyface-mesh `POLYLINE`s from the
/// ENTITIES section; other entities are ignored. Writes `3DFACE` entities.
pub struct Dxf;

fn pairs(text: &str) -> Vec<(i32, &str)> {
    let mut lines = text.lines();
    let mut out = Vec::new();
    while let (Some(c), Some(v)) = (lines.next(), lines.next()) {
        if let Ok(code) = c.trim().parse::<i32>() {
            out.push((code, v.trim()));
        }
    }
    out
}

fn num(v: &str) -> f64 {
    v.parse::<f64>().unwrap_or(0.0)
}

struct Entity<'a> {
    name: &'a str,
    data: Vec<(i32, &'a str)>,
}

fn entities<'a>(pairs: &[(i32, &'a str)]) -> Vec<Entity<'a>> {
    let mut out: Vec<Entity<'a>> = Vec::new();
    let mut in_entities = false;
    let mut i = 0;
    while i < pairs.len() {
        let (code, val) = pairs[i];
        if code == 0 && val == "SECTION" {
            in_entities = pairs.get(i + 1).is_some_and(|&(c, v)| c == 2 && v == "ENTITIES");
        } else if code == 0 && val == "ENDSEC" {
            in_entities = false;
        } else if in_entities && code == 0 {
            out.push(Entity { name: val, data: Vec::new() });
        } else if in_entities {
            if let Some(e) = out.last_mut() {
                e.data.push((code, val));
            }
        }
        i += 1;
    }
    out
}

/// Polyface state while reading: (vertices, 1-based face records).
type Polyface = (Vec<[f64; 3]>, Vec<Vec<i32>>);

impl MeshImporter for Dxf {
    fn extensions(&self) -> &'static [&'static str] {
        &["dxf"]
    }

    fn import(&self, bytes: &[u8]) -> Result<Mesh, IoError> {
        let text = std::str::from_utf8(bytes).map_err(|_| IoError::Unsupported("binary or non-UTF-8 DXF".into()))?;
        let pairs = pairs(text);
        let mut mesh = Mesh::new();
        let mut welder = Welder::default();

        let mut poly: Option<Polyface> = None;

        for e in entities(&pairs) {
            match e.name {
                "3DFACE" => {
                    let mut p = [[0.0f64; 3]; 4];
                    for &(code, v) in &e.data {
                        let slot = (code % 10) as usize;
                        if slot < 4 {
                            match code / 10 {
                                1 => p[slot][0] = num(v),
                                2 => p[slot][1] = num(v),
                                3 => p[slot][2] = num(v),
                                _ => {}
                            }
                        }
                    }
                    let mut idx: Vec<u32> = Vec::new();
                    for pt in p {
                        let i = welder.vertex(&mut mesh, pt);
                        if idx.last() != Some(&i) {
                            idx.push(i);
                        }
                    }
                    if idx.len() > 1 && idx.first() == idx.last() {
                        idx.pop();
                    }
                    if idx.len() >= 3 {
                        mesh.add_face(idx);
                    }
                }
                "POLYLINE" => {
                    let flags = e.data.iter().find(|d| d.0 == 70).map_or(0, |d| d.1.parse().unwrap_or(0));
                    poly = (flags & 64 != 0).then(|| (Vec::new(), Vec::new()));
                }
                "VERTEX" => {
                    if let Some((verts, faces)) = poly.as_mut() {
                        let flags: i32 = e.data.iter().find(|d| d.0 == 70).map_or(0, |d| d.1.parse().unwrap_or(0));
                        if flags & 128 != 0 && flags & 64 == 0 {
                            let f: Vec<i32> = e
                                .data
                                .iter()
                                .filter(|d| (71..=74).contains(&d.0))
                                .filter_map(|d| d.1.parse().ok())
                                .filter(|&i| i != 0)
                                .collect();
                            faces.push(f);
                        } else {
                            let get = |c: i32| e.data.iter().find(|d| d.0 == c).map_or(0.0, |d| num(d.1));
                            verts.push([get(10), get(20), get(30)]);
                        }
                    }
                }
                "SEQEND" => {
                    if let Some((verts, faces)) = poly.take() {
                        let base: Vec<u32> = verts.iter().map(|&v| welder.vertex(&mut mesh, v)).collect();
                        for f in faces {
                            // Negative indices mark invisible edges; the vertex is still used.
                            let idx: Vec<u32> = f.iter().filter_map(|&i| base.get(i.unsigned_abs() as usize - 1).copied()).collect();
                            if idx.len() >= 3 {
                                mesh.add_face(idx);
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        if mesh.faces.is_empty() {
            return Err(IoError::Parse("DXF contains no 3DFACE or polyface geometry".into()));
        }
        Ok(mesh)
    }
}

impl MeshExporter for Dxf {
    fn extensions(&self) -> &'static [&'static str] {
        &["dxf"]
    }

    fn export(&self, mesh: &Mesh) -> Result<Vec<u8>, IoError> {
        let mut s = String::from("0\nSECTION\n2\nENTITIES\n");
        for t in mesh.triangles() {
            let p: Vec<[f64; 3]> = t.iter().map(|&i| mesh.positions[i as usize]).collect();
            s.push_str("0\n3DFACE\n8\n0\n");
            // The fourth corner repeats the third, which DXF uses to mean a triangle.
            for (k, pt) in [p[0], p[1], p[2], p[2]].iter().enumerate() {
                let _ = write!(s, "{}\n{}\n{}\n{}\n{}\n{}\n", 10 + k, pt[0], 20 + k, pt[1], 30 + k, pt[2]);
            }
        }
        s.push_str("0\nENDSEC\n0\nEOF\n");
        Ok(s.into_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_3dface() {
        let mut m = Mesh::new();
        let a = m.add_vertex([0.0, 0.0, 0.0]);
        let b = m.add_vertex([1.0, 0.0, 0.0]);
        let c = m.add_vertex([0.0, 1.0, 0.0]);
        m.add_face(vec![a, b, c]);
        let back = Dxf.import(&Dxf.export(&m).unwrap()).unwrap();
        assert_eq!(back.faces.len(), 1);
        assert_eq!(back.faces[0].len(), 3);
        assert_eq!(back.positions.len(), 3);
    }

    #[test]
    fn polyface_mesh() {
        let src = "0\nSECTION\n2\nENTITIES\n\
0\nPOLYLINE\n70\n64\n\
0\nVERTEX\n10\n0\n20\n0\n30\n0\n70\n192\n\
0\nVERTEX\n10\n1\n20\n0\n30\n0\n70\n192\n\
0\nVERTEX\n10\n0\n20\n1\n30\n0\n70\n192\n\
0\nVERTEX\n70\n128\n71\n1\n72\n2\n73\n-3\n\
0\nSEQEND\n0\nENDSEC\n0\nEOF\n";
        let m = Dxf.import(src.as_bytes()).unwrap();
        assert_eq!(m.positions.len(), 3);
        assert_eq!(m.faces, vec![vec![0, 1, 2]]);
    }
}
