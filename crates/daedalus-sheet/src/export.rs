//! A whole model to STEP (File > Export > STEP): every sheet-metal part as its exact solid, every other closed group or
//! component (and closed loose geometry) as an exact solid of its flat faces, all placed as they are in the model and in mm. What is
//! not a closed solid is left out and named, so the person knows what the file holds.

use crate::part::{self, SheetPart};
use crate::util::Result;
use daedalus_core::scene::{Context, Scene, Xform};
use daedalus_core::Mesh;
use solidcore_record::Vec3;

/// What went into a STEP file.
#[derive(Debug, Default)]
pub struct StepReport {
    pub text: String,
    /// Sheet-metal parts and other solids (flat-faced) written.
    pub parts: usize,
    pub solids: usize,
    /// Groups and components left out (not closed solids), by name; "loose geometry" for the model's own faces.
    pub skipped: Vec<String>,
}

/// The model as STEP. Fails only when there is nothing to write or the writer fails.
pub fn scene_step(scene: &Scene) -> Result<StepReport> {
    let mut bodies = Vec::new();
    let mut report = StepReport::default();
    if !scene.root.mesh.faces.is_empty() {
        add_mesh(&scene.root.mesh, &Xform::IDENTITY, "loose geometry", &mut bodies, &mut report);
    }
    walk(scene, &scene.root, &Xform::IDENTITY, &mut bodies, &mut report, 0)?;
    if bodies.is_empty() {
        return Err(crate::util::DocError::Invalid("there is no closed solid or sheet-metal part to export".into()));
    }
    report.text = part::step(&bodies)?;
    Ok(report)
}

fn walk(scene: &Scene, ctx: &Context, world: &Xform, bodies: &mut Vec<crate::kernel::Body>, report: &mut StepReport, depth: usize) -> Result<()> {
    if depth > 64 {
        return Ok(());
    }
    for inst in &ctx.instances {
        let Some(def) = scene.defs.get(inst.def) else { continue };
        let w = world.after(&inst.xf);
        match def.part.as_deref().filter(|t| part::is_sheet(t)) {
            Some(text) => {
                let p = SheetPart::from_text(text)?;
                bodies.push(p.solid_placed(&w)?);
                report.parts += 1;
            }
            None => {
                if !def.ctx.mesh.faces.is_empty() {
                    add_mesh(&def.ctx.mesh, &w, &def.name, bodies, report);
                }
            }
        }
        walk(scene, &def.ctx, &w, bodies, report, depth + 1)?;
    }
    Ok(())
}

/// A closed mesh as a solid of its flat faces (mm, placed by `w`), sewn edge to edge; anything else is noted as left out.
fn add_mesh(mesh: &Mesh, w: &Xform, name: &str, bodies: &mut Vec<crate::kernel::Body>, report: &mut StepReport) {
    let solid = daedalus_core::inspect::inspect(mesh, mesh.tolerance()).is_solid();
    let made = solid.then(|| sewn(mesh, w).ok()).flatten();
    match made {
        Some(b) => {
            bodies.push(b);
            report.solids += 1;
        }
        None => report.skipped.push(name.to_string()),
    }
}

/// Each face (outline and holes) a plane bounded by straight edges, sewn into one closed solid.
fn sewn(mesh: &Mesh, w: &Xform) -> Result<crate::kernel::Body> {
    use crate::kernel::{EdgeSpec, FaceSpec, SurfSpec};
    let pt = |i: u32| {
        let p = w.apply(mesh.positions[i as usize]);
        Vec3::new(p[0] * 1000.0, p[1] * 1000.0, p[2] * 1000.0)
    };
    let faces: Vec<FaceSpec> = (0..mesh.faces.len())
        .map(|f| FaceSpec {
            loops: mesh.loops(f).map(|lp| (0..lp.len()).map(|i| EdgeSpec::Line { a: pt(lp[i]), b: pt(lp[(i + 1) % lp.len()]) }).collect()).collect(),
            surface: SurfSpec::Plane,
            probe: None,
        })
        .collect();
    Ok(crate::kernel::sew_touching(&faces)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::part::{part_from_face, FlangePosition};
    use crate::sheet::RuleValues;
    use daedalus_core::scene::{Definition, Instance};

    fn cube(s: f64) -> Mesh {
        let mut m = Mesh::new();
        for z in [0.0, s] {
            for (x, y) in [(0.0, 0.0), (s, 0.0), (s, s), (0.0, s)] {
                m.add_vertex([x, y, z]);
            }
        }
        for f in [[0, 3, 2, 1], [4, 5, 6, 7], [0, 1, 5, 4], [1, 2, 6, 5], [2, 3, 7, 6], [3, 0, 4, 7]] {
            m.add_face(f.to_vec());
        }
        m
    }

    #[test]
    fn a_model_with_a_part_a_box_and_a_loose_face_goes_out_with_what_is_solid() {
        let mut plate = Mesh::new();
        for p in [[0.0, 0.0, 0.0], [0.1, 0.0, 0.0], [0.1, 0.05, 0.0], [0.0, 0.05, 0.0]] {
            plate.add_vertex(p);
        }
        plate.add_face(vec![0, 1, 2, 3]);
        let rule = RuleValues { k_factor: 0.44, bend_radius: 2.0, ..RuleValues::steel(2.0) };
        let (mut p, place) = part_from_face(&plate, 0, "Steel", rule).unwrap();
        p.add_flange([50.0, 0.0, 2.0], 20.0, 90.0, FlangePosition::Inside).unwrap();
        let mut scene = Scene::default();
        let def = |name: &str, part: Option<String>, mesh: Mesh| Definition {
            name: name.into(),
            component: true,
            glue: false,
            cut: false,
            attributes: Vec::new(),
            part,
            ctx: Context { mesh, instances: Vec::new() },
        };
        scene.defs.push(def("Sheet Part#1", Some(p.to_text()), p.faces(false).unwrap()));
        scene.defs.push(def("Box", None, cube(0.05)));
        let mut open = Mesh::new();
        for q in [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0]] {
            open.add_vertex(q);
        }
        open.add_face(vec![0, 1, 2]);
        scene.defs.push(def("Open", None, open));
        scene.root.instances = vec![
            Instance { def: 0, xf: place },
            Instance { def: 1, xf: Xform::translation([1.0, 0.0, 0.0]) },
            Instance { def: 1, xf: Xform::translation([2.0, 0.0, 0.0]) },
            Instance { def: 2, xf: Xform::IDENTITY },
        ];
        let r = scene_step(&scene).unwrap();
        assert_eq!((r.parts, r.solids), (1, 2));
        assert_eq!(r.skipped, vec!["Open".to_string()]);
        assert!(r.text.starts_with("ISO-10303-21;"));
        assert!(r.text.contains("CYLINDRICAL_SURFACE"), "the part's bend is exact");
        // nothing solid: an error that says so
        let mut empty = Scene::default();
        empty.defs.push(def("Open", None, scene.defs[2].ctx.mesh.clone()));
        empty.root.instances = vec![Instance { def: 0, xf: Xform::IDENTITY }];
        assert!(scene_step(&empty).unwrap_err().to_string().contains("no closed solid or sheet-metal part"));
    }
}
