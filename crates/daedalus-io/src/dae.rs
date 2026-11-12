use crate::{IoError, MeshExporter, MeshImporter};
use daedalus_core::Mesh;
use roxmltree::{Document, Node};
use std::collections::HashMap;
use std::fmt::Write;

/// COLLADA (.dae). Reads `triangles`, `polylist` and `polygons` primitives
/// from every geometry; materials, scenes and transforms are ignored.
pub struct Dae;

fn children<'a, 'b>(n: Node<'a, 'b>, name: &'static str) -> impl Iterator<Item = Node<'a, 'b>> {
    n.children().filter(move |c| c.is_element() && c.tag_name().name() == name)
}

fn floats(n: Node) -> Vec<f64> {
    n.text().unwrap_or("").split_whitespace().filter_map(|s| s.parse().ok()).collect()
}

fn ints(n: Node) -> Vec<usize> {
    n.text().unwrap_or("").split_whitespace().filter_map(|s| s.parse().ok()).collect()
}

fn strip_hash(s: &str) -> &str {
    s.strip_prefix('#').unwrap_or(s)
}

impl MeshImporter for Dae {
    fn extensions(&self) -> &'static [&'static str] {
        &["dae"]
    }

    fn import(&self, bytes: &[u8]) -> Result<Mesh, IoError> {
        let text = std::str::from_utf8(bytes).map_err(|_| IoError::Parse("DAE is not UTF-8".into()))?;
        let doc = Document::parse(text).map_err(|e| IoError::Parse(e.to_string()))?;
        let mut out = Mesh::new();

        for geometry in doc.descendants().filter(|n| n.is_element() && n.tag_name().name() == "geometry") {
            for mesh in children(geometry, "mesh") {
                // source id -> (data, stride)
                let mut sources: HashMap<&str, (Vec<f64>, usize)> = HashMap::new();
                for src in children(mesh, "source") {
                    let Some(id) = src.attribute("id") else { continue };
                    let data = children(src, "float_array").next().map(floats).unwrap_or_default();
                    let stride = src
                        .descendants()
                        .find(|n| n.is_element() && n.tag_name().name() == "accessor")
                        .and_then(|a| a.attribute("stride"))
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(1);
                    sources.insert(id, (data, stride));
                }

                // <vertices id> -> id of its POSITION source
                let mut vertices: HashMap<&str, &str> = HashMap::new();
                for v in children(mesh, "vertices") {
                    let Some(id) = v.attribute("id") else { continue };
                    for input in children(v, "input") {
                        if input.attribute("semantic") == Some("POSITION") {
                            if let Some(s) = input.attribute("source") {
                                vertices.insert(id, strip_hash(s));
                            }
                        }
                    }
                }

                // Source vertex index -> output vertex index, per source.
                let mut remap: HashMap<(&str, usize), u32> = HashMap::new();

                for prim in
                    mesh.children().filter(|c| c.is_element() && matches!(c.tag_name().name(), "triangles" | "polylist" | "polygons"))
                {
                    let inputs: Vec<Node> = children(prim, "input").collect();
                    let stride = inputs
                        .iter()
                        .filter_map(|i| i.attribute("offset").and_then(|o| o.parse::<usize>().ok()))
                        .max()
                        .map_or(1, |m| m + 1);
                    let Some(vin) = inputs.iter().find(|i| i.attribute("semantic") == Some("VERTEX")) else {
                        continue;
                    };
                    let voffset: usize = vin.attribute("offset").and_then(|o| o.parse().ok()).unwrap_or(0);
                    let vref = strip_hash(vin.attribute("source").unwrap_or(""));
                    let Some(&src_id) = vertices.get(vref) else { continue };
                    let Some((pos, pstride)) = sources.get(src_id) else { continue };
                    if *pstride < 3 {
                        continue;
                    }

                    let mut corner = |index: usize, out: &mut Mesh| -> Option<u32> {
                        if let Some(&v) = remap.get(&(src_id, index)) {
                            return Some(v);
                        }
                        let o = index * pstride;
                        let p = pos.get(o..o + 3)?;
                        let v = out.add_vertex([p[0], p[1], p[2]]);
                        remap.insert((src_id, index), v);
                        Some(v)
                    };

                    // Each entry is the interleaved index list for one face.
                    let mut face_lists: Vec<Vec<usize>> = Vec::new();
                    match prim.tag_name().name() {
                        "triangles" => {
                            if let Some(p) = children(prim, "p").next() {
                                for tri in ints(p).chunks_exact(stride * 3) {
                                    face_lists.push(tri.to_vec());
                                }
                            }
                        }
                        "polylist" => {
                            let vcount = children(prim, "vcount").next().map(ints).unwrap_or_default();
                            if let Some(p) = children(prim, "p").next() {
                                let all = ints(p);
                                let mut at = 0;
                                for n in vcount {
                                    let end = at + n * stride;
                                    if end > all.len() {
                                        return Err(IoError::Parse("polylist <p> shorter than <vcount>".into()));
                                    }
                                    face_lists.push(all[at..end].to_vec());
                                    at = end;
                                }
                            }
                        }
                        _ => {
                            for p in children(prim, "p") {
                                face_lists.push(ints(p));
                            }
                        }
                    }

                    for list in face_lists {
                        let idx: Vec<u32> = list.chunks_exact(stride).filter_map(|c| corner(c[voffset], &mut out)).collect();
                        if idx.len() >= 3 {
                            out.add_face(idx);
                        }
                    }
                }
            }
        }

        if out.faces.is_empty() {
            return Err(IoError::Parse("DAE contains no polygon geometry".into()));
        }
        Ok(out)
    }
}

impl MeshExporter for Dae {
    fn extensions(&self) -> &'static [&'static str] {
        &["dae"]
    }

    fn export(&self, mesh: &Mesh) -> Result<Vec<u8>, IoError> {
        let mut s = String::new();
        s.push_str("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n");
        s.push_str("<COLLADA xmlns=\"http://www.collada.org/2005/11/COLLADASchema\" version=\"1.4.1\">\n");
        s.push_str("<asset><unit name=\"meter\" meter=\"1\"/><up_axis>Z_UP</up_axis></asset>\n");
        s.push_str("<library_geometries><geometry id=\"mesh\" name=\"mesh\"><mesh>\n");
        let _ = writeln!(s, "<source id=\"pos\"><float_array id=\"pos-array\" count=\"{}\">", mesh.positions.len() * 3);
        for p in &mesh.positions {
            let _ = write!(s, "{} {} {} ", p[0], p[1], p[2]);
        }
        let _ = writeln!(
            s,
            "</float_array><technique_common><accessor source=\"#pos-array\" count=\"{}\" stride=\"3\">\
<param name=\"X\" type=\"float\"/><param name=\"Y\" type=\"float\"/><param name=\"Z\" type=\"float\"/>\
</accessor></technique_common></source>",
            mesh.positions.len()
        );
        s.push_str("<vertices id=\"verts\"><input semantic=\"POSITION\" source=\"#pos\"/></vertices>\n");
        // a face with holes goes out as its triangles (a polylist has no holes)
        let (polys, _) = mesh.polygons();
        let _ = writeln!(s, "<polylist count=\"{}\"><input semantic=\"VERTEX\" source=\"#verts\" offset=\"0\"/><vcount>", polys.len());
        for f in &polys {
            let _ = write!(s, "{} ", f.len());
        }
        s.push_str("</vcount><p>");
        for f in &polys {
            for i in f {
                let _ = write!(s, "{i} ");
            }
        }
        s.push_str("</p></polylist>\n</mesh></geometry></library_geometries>\n");
        s.push_str("<library_visual_scenes><visual_scene id=\"scene\"><node id=\"n\"><instance_geometry url=\"#mesh\"/></node></visual_scene></library_visual_scenes>\n");
        s.push_str("<scene><instance_visual_scene url=\"#scene\"/></scene>\n</COLLADA>\n");
        Ok(s.into_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_quad() {
        let mut m = Mesh::new();
        let a = m.add_vertex([0.0, 0.0, 0.0]);
        let b = m.add_vertex([1.0, 0.0, 0.0]);
        let c = m.add_vertex([1.0, 1.0, 0.0]);
        let d = m.add_vertex([0.0, 1.0, 0.0]);
        m.add_face(vec![a, b, c, d]);
        let back = Dae.import(&Dae.export(&m).unwrap()).unwrap();
        assert_eq!(back.faces, vec![vec![0, 1, 2, 3]]);
        assert_eq!(back.positions, m.positions);
    }

    #[test]
    fn triangles_with_normals_offset() {
        let src = r##"<COLLADA><library_geometries><geometry id="g"><mesh>
<source id="p"><float_array id="pa" count="9">0 0 0 1 0 0 0 1 0</float_array>
<technique_common><accessor source="#pa" count="3" stride="3"/></technique_common></source>
<source id="n"><float_array id="na" count="3">0 0 1</float_array>
<technique_common><accessor source="#na" count="1" stride="3"/></technique_common></source>
<vertices id="v"><input semantic="POSITION" source="#p"/></vertices>
<triangles count="1"><input semantic="VERTEX" source="#v" offset="0"/><input semantic="NORMAL" source="#n" offset="1"/>
<p>0 0 1 0 2 0</p></triangles></mesh></geometry></library_geometries></COLLADA>"##;
        let m = Dae.import(src.as_bytes()).unwrap();
        assert_eq!(m.faces, vec![vec![0, 1, 2]]);
    }
}
