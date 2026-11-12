use crate::scene::{Context, Definition, Instance, Notes, Piece, Scene, Xform};
use crate::{HalfEdgeMesh, Mesh, Paint, TopologyError};

/// Merges coplanar faces of `m` in place; returns where `face` went (or `face` when nothing could be rebuilt).
fn merge_in_place(m: &mut HalfEdgeMesh, face: usize) -> usize {
    let mesh = m.to_mesh();
    let (merged, map) = crate::merge::merge_coplanar(&mesh, mesh.tolerance());
    match HalfEdgeMesh::from_mesh(&merged) {
        Ok(h) => {
            *m = h;
            map.get(face).copied().unwrap_or(face)
        }
        // never expected; the push stays as it was rather than being undone
        Err(_) => face,
    }
}

/// The openings the instances `ids` of the open context cut, taken away (they are being erased).
fn uncut_openings(m: &mut HalfEdgeMesh, s: &Scene, open: &[usize], ids: &[usize]) -> Result<(), TopologyError> {
    let Some(ctx) = s.ctx(open) else { return Ok(()) };
    if !ids.iter().filter_map(|&i| ctx.instances.get(i)).any(|inst| s.defs.get(inst.def).is_some_and(|d| d.cut)) {
        return Ok(());
    }
    let mut mesh = m.to_mesh();
    let outlines: Vec<Vec<[f64; 3]>> =
        ids.iter().filter_map(|&i| ctx.instances.get(i)).filter_map(|inst| crate::opening::footprint(s, open, inst, &mesh)).collect();
    for o in &outlines {
        crate::opening::uncut(&mut mesh, o);
    }
    *m = HalfEdgeMesh::from_mesh(&mesh)?;
    Ok(())
}

/// The Solid Tools (see [`Model::solid_tool`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SolidTool {
    OuterShell,
    Intersect,
    Union,
    Subtract,
    Trim,
    Split,
}

/// Definition `def`'s size along its own axes (its geometry and what is placed in it), and its lowest corner.
fn def_box(scene: &Scene, def: usize) -> ([f64; 3], [f64; 3]) {
    // its own content, not a copy of it: a copy's Copies would count, and placing those asks for this size
    let mut m = Mesh::new();
    scene.flatten_contents(def, &mut m);
    match m.bounds() {
        Some((lo, hi)) => ([hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]], lo),
        None => ([0.0; 3], [0.0; 3]),
    }
}

/// The built-in attributes of a copy where it is now (lower-case names): X, Y, Z, LenX, LenY, LenZ, RotZ.
fn placed(scene: &Scene, inst: &Instance) -> std::collections::HashMap<String, f64> {
    let (size, _) = def_box(scene, inst.def);
    let m = &inst.xf.0;
    let col = |c: usize| [m[0][c], m[1][c], m[2][c]];
    let len = |v: [f64; 3]| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    let o = inst.xf.origin();
    [
        ("x", o[0]),
        ("y", o[1]),
        ("z", o[2]),
        ("lenx", size[0] * len(col(0))),
        ("leny", size[1] * len(col(1))),
        ("lenz", size[2] * len(col(2))),
        ("rotz", col(0)[1].atan2(col(0)[0]).to_degrees()),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect()
}

/// The placement attributes X, Y, Z, LenX, LenY, LenZ, RotZ worked out (`v`) made into a copy's transform: what they set
/// replaces where it is (`inst`) and how big; what they leave alone stays. `size` is its definition's size.
fn dynamic_xf(
    size: [f64; 3],
    attrs: &[crate::dynamic::Attribute],
    inst: &Instance,
    given: &std::collections::HashMap<String, f64>,
    v: &std::collections::HashMap<String, crate::dynamic::Value>,
) -> Xform {
    let sets = |n: &str| attrs.iter().any(|a| a.name.eq_ignore_ascii_case(n));
    let num = |n: &str| v.get(n).map(|x| x.num()).unwrap_or_else(|| given.get(n).copied().unwrap_or(0.0));
    let m = &inst.xf.0;
    let col = |c: usize| [m[0][c], m[1][c], m[2][c]];
    let len = |a: [f64; 3]| (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
    // its axes: turned to RotZ when that is set, else as they are
    let axes: [[f64; 3]; 3] = if sets("rotz") {
        let t = num("rotz").to_radians();
        [[t.cos(), t.sin(), 0.0], [-t.sin(), t.cos(), 0.0], [0.0, 0.0, 1.0]]
    } else {
        std::array::from_fn(|c| {
            let a = col(c);
            let l = len(a).max(1e-12);
            a.map(|x| x / l)
        })
    };
    // its scale along each axis: from LenX/LenY/LenZ when set (a size of nothing stays at 1), else as it is
    let scale: [f64; 3] = std::array::from_fn(|c| {
        let name = ["lenx", "leny", "lenz"][c];
        if sets(name) && size[c] > 1e-12 {
            (num(name) / size[c]).max(1e-9)
        } else {
            len(col(c))
        }
    });
    let o = [num("x"), num("y"), num("z")];
    let mut xf = [[0.0; 4]; 3];
    for r in 0..3 {
        for c in 0..3 {
            xf[r][c] = axes[c][r] * scale[c];
        }
        xf[r][3] = o[r];
    }
    Xform(xf)
}

/// A copy's built-ins where it is, with the values of the component it sits in under `parent!<name>` (for `Parent!LenX` in
/// a formula) and COPY = `copy` (0: the copy itself; 1, 2, ...: the copies its Copies attribute adds).
fn given_for(
    scene: &Scene,
    inst: &Instance,
    parent: &std::collections::HashMap<String, f64>,
    copy: usize,
) -> std::collections::HashMap<String, f64> {
    let mut given = placed(scene, inst);
    for (k, v) in parent {
        given.insert(format!("parent!{k}"), *v);
    }
    given.insert("copy".into(), copy as f64);
    given
}

/// How a copy is drawn, from its Dynamic Component attributes (SketchUp's names): not at all when Hidden is true; the copy
/// itself and, with Copies = n, n more placed by the same formulas with COPY = 1 .. n; Material (a colour name, `#rrggbb` or
/// `r,g,b`) on every face left unpainted. A formula that goes wrong draws the copy as it is.
pub(crate) struct DcView {
    pub xfs: Vec<Xform>,
    pub paint: Option<Paint>,
    /// Its own values (built-ins and number attributes, lower-case), for the components inside it (`Parent!`).
    pub values: std::collections::HashMap<String, f64>,
}

pub(crate) fn dc_view(scene: &Scene, inst: &Instance, parent: &std::collections::HashMap<String, f64>) -> Option<DcView> {
    use crate::dynamic::{evaluate, Value};
    let plain = || DcView { xfs: vec![inst.xf], paint: None, values: std::collections::HashMap::new() };
    let Some(d) = scene.defs.get(inst.def) else { return Some(plain()) };
    if d.attributes.is_empty() {
        return Some(plain());
    }
    let attrs = &d.attributes;
    let given = given_for(scene, inst, parent, 0);
    let Ok(v) = evaluate(attrs, &given) else { return Some(plain()) };
    let truthy = |n: &str| match v.get(n) {
        Some(Value::Num(x)) => *x != 0.0,
        Some(Value::Text(t)) => matches!(t.trim().to_ascii_lowercase().as_str(), "true" | "yes" | "1"),
        None => false,
    };
    if truthy("hidden") {
        return None;
    }
    let paint = v.get("material").and_then(|m| crate::dynamic::parse_color(&m.text())).map(|rgb| Paint { rgb, texture: None });
    let copies = v.get("copies").map(|c| c.num().round().clamp(0.0, crate::dynamic::MAX_COPIES as f64) as usize).unwrap_or(0);
    let mut xfs = vec![inst.xf];
    if copies > 0 {
        let (size, _) = def_box(scene, inst.def);
        for k in 1..=copies {
            let g = given_for(scene, inst, parent, k);
            if let Ok(vk) = evaluate(attrs, &g) {
                xfs.push(dynamic_xf(size, attrs, inst, &g, &vk));
            }
        }
    }
    let mut values: std::collections::HashMap<String, f64> = given.into_iter().filter(|(k, _)| !k.starts_with("parent!")).collect();
    for (k, x) in &v {
        if let Value::Num(n) = x {
            values.insert(k.clone(), *n);
        }
    }
    Some(DcView { xfs, paint, values })
}

/// The values of the component whose inside is open at `open` (none at the top level), for `Parent!` in its copies' formulas.
fn owner_values(scene: &Scene, open: &[usize]) -> std::collections::HashMap<String, f64> {
    let mut vals = std::collections::HashMap::new();
    let mut c = &scene.root;
    for &step in open {
        let Some(inst) = c.instances.get(step) else { break };
        vals = dc_view(scene, inst, &vals).map(|v| v.values).unwrap_or_default();
        c = &scene.defs[inst.def].ctx;
    }
    vals
}

/// Every copy of `def` in the context at `open` placed and sized by its attributes: the built-ins it sets (X, Y, Z, LenX,
/// LenY, LenZ, RotZ) replace where it is and how big; the rest stay as they were.
/// The name for a copy of component `name` made its own: `Door#2` (one more than the highest number on that name), `Door#3`
/// from `Door#2`.
fn copy_name(scene: &Scene, name: &str) -> String {
    let stem = match name.rsplit_once('#') {
        Some((s, n)) if !s.is_empty() && n.parse::<u32>().is_ok() => s,
        _ => name,
    };
    let n = scene.defs.iter().filter_map(|d| d.name.strip_prefix(stem)?.strip_prefix('#')?.parse::<u32>().ok()).max().unwrap_or(1);
    format!("{stem}#{}", n + 1)
}

fn apply_dynamic(scene: &mut Scene, open: &[usize], def: usize) -> Result<(), String> {
    let (size, _) = def_box(scene, def);
    let attrs = scene.defs[def].attributes.clone();
    let parent = owner_values(scene, open);
    let copies: Vec<usize> =
        scene.ctx(open).map(|c| (0..c.instances.len()).filter(|&i| c.instances[i].def == def).collect()).unwrap_or_default();
    for i in copies {
        let inst = scene.ctx(open).expect("there").instances[i];
        let given = given_for(scene, &inst, &parent, 0);
        let v = crate::dynamic::evaluate(&attrs, &given)?;
        let xf = dynamic_xf(size, &attrs, &inst, &given, &v);
        scene.ctx_mut(open).expect("there").instances[i].xf = xf;
    }
    Ok(())
}

/// The editable document: the geometry being edited, the rest of the scene (groups, components, notes), and an undo/redo
/// history.
///
/// Every edit is one atomic history step; a failed edit leaves the model untouched. History stores whole snapshots, which
/// is simple and correct; it should move to diffs/commands once models get large.
///
/// While a group or component is open for editing, [`Model::mesh`] is its inside, in model coordinates (placed where the
/// instance stands), so every tool works on it the same way; it goes back into the definition when it is closed.
#[derive(Debug, Clone, Default)]
pub struct Model {
    mesh: HalfEdgeMesh,
    /// Everything else. The open context's own mesh in here is stale while it is open: `mesh` holds it.
    scene: Scene,
    /// The instances opened, from the top (see `Scene::ctx`).
    open: Vec<usize>,
    undo: Vec<(Snapshot, u64)>,
    redo: Vec<(Snapshot, u64)>,
    /// Which state the mesh is in: a new number for every edit, carried along by undo and redo, so "unchanged since saving"
    /// is a comparison with the number at the save (undoing back to it counts as unchanged).
    state: u64,
    next_state: u64,
    /// See [`Model::revision`], and the stamp it had when the scene last changed (edits of the geometry being edited
    /// alone leave that one: see [`Model::parts_revision`]).
    revision: u64,
    scene_revision: u64,
}

/// Stamps for [`Model::revision`]: one count for all models, so no stamp is ever given out twice.
static REVISIONS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn fresh_revision() -> u64 {
    REVISIONS.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

#[derive(Debug, Clone)]
struct Snapshot {
    mesh: HalfEdgeMesh,
    scene: Scene,
    open: Vec<usize>,
}

/// Why a group operation could not be done.
fn nothing() -> TopologyError {
    TopologyError::FaceOutOfRange(usize::MAX)
}

impl Model {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_mesh(mesh: HalfEdgeMesh) -> Self {
        let revision = fresh_revision();
        Model { mesh, revision, scene_revision: revision, ..Default::default() }
    }

    /// A whole scene (an opened file): its top level must be editable.
    pub fn from_scene(mut scene: Scene) -> Result<Self, TopologyError> {
        let mesh = HalfEdgeMesh::from_mesh(&scene.root.mesh)?;
        scene.root.mesh = Mesh::new();
        let revision = fresh_revision();
        Ok(Model { mesh, scene, revision, scene_revision: revision, ..Default::default() })
    }

    /// A stamp that changes whenever anything here may have changed: an edit (even one that failed), undo, redo, opening
    /// or closing a group. Unlike [`Model::state`] it never comes back, so what is worked out from the model and kept is up
    /// to date while the stamp it was made at is still this one. A new, empty model has 0; a copy keeps its model's stamp
    /// until either changes; no other model ever has it.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// As [`Model::revision`], for [`Model::parts`] (and [`Model::with_part_pieces`]): at the top level an edit of the
    /// loose geometry alone leaves it, as no part is made of that geometry there. Inside an open group every change counts
    /// (copies of a component outside it show the geometry being edited).
    pub fn parts_revision(&self) -> u64 {
        if self.open.is_empty() {
            self.scene_revision
        } else {
            self.revision
        }
    }

    /// The geometry being edited: the top level, or the inside of the open group or component (in model coordinates).
    pub fn mesh(&self) -> &HalfEdgeMesh {
        &self.mesh
    }

    /// The scene as it is now, with the open context's geometry in its place: what a save writes.
    pub fn document(&self) -> Scene {
        let mut s = self.scene.clone();
        let back = self.scene.world(&self.open).inverse().unwrap_or(Xform::IDENTITY);
        let local = if self.open.is_empty() { self.mesh.to_mesh() } else { self.mesh.to_mesh().transformed(&back) };
        if let Some(c) = s.ctx_mut(&self.open) {
            c.mesh = local;
        }
        s
    }

    /// Everything as one mesh in model coordinates (export, solid inspector).
    pub fn flatten(&self) -> Mesh {
        self.document().flatten()
    }

    /// The open context's instances, placed in the model (by index), and what lies outside the open context (empty at the
    /// top level). For drawing and picking.
    pub fn parts(&self) -> (Vec<(usize, Mesh)>, Mesh) {
        self.document().parts(&self.open)
    }

    /// [`Model::parts`] piece by piece ([`Scene::part_pieces`] of the [`Model::document`]), handed to `f`, without copying
    /// the scene: while `f` runs, the open context's geometry stands in its definition as a save would write it.
    pub fn with_part_pieces<R>(&mut self, f: impl for<'s> FnOnce(Vec<(usize, Vec<Piece<'s>>)>, Vec<Piece<'s>>) -> R) -> R {
        // at the top level no part is made of the loose geometry being edited
        let live = if self.open.is_empty() {
            None
        } else {
            let back = self.scene.world(&self.open).inverse().unwrap_or(Xform::IDENTITY);
            let local = self.mesh.to_mesh().transformed(&back);
            self.scene.ctx_mut(&self.open).map(|c| std::mem::replace(&mut c.mesh, local))
        };
        let (inside, outside) = self.scene.part_pieces(&self.open);
        let r = f(inside, outside);
        if let (Some(stale), Some(c)) = (live, self.scene.ctx_mut(&self.open)) {
            c.mesh = stale;
        }
        r
    }

    pub fn notes(&self) -> &Notes {
        &self.scene.notes
    }

    /// How deep groups are open (0: the top level).
    pub fn open_depth(&self) -> usize {
        self.open.len()
    }

    /// The open context's instances (their transforms are in the open context's coordinates).
    pub fn instances(&self) -> &[Instance] {
        self.scene.ctx(&self.open).map_or(&[], |c| &c.instances)
    }

    /// Instance `i` of the open context as placed in the model.
    pub fn instance_world(&self, i: usize) -> Option<Xform> {
        let inst = self.instances().get(i)?;
        Some(self.scene.world(&self.open).after(&inst.xf))
    }

    pub fn definition(&self, def: usize) -> Option<&Definition> {
        self.scene.defs.get(def)
    }

    /// The names of the definitions opened, from the top ("Group#1 > Component#2").
    /// The groups and components open for editing, as instance numbers from the top.
    pub fn open_path(&self) -> &[usize] {
        &self.open
    }

    /// The Outliner: every group and component placed, depth first from the top: (depth, name, component, path of instance
    /// numbers from the top level down to it).
    pub fn outline(&self) -> Vec<(usize, String, bool, Vec<usize>)> {
        fn walk(
            scene: &Scene,
            ctx: &crate::scene::Context,
            path: &mut Vec<usize>,
            out: &mut Vec<(usize, String, bool, Vec<usize>)>,
            depth: usize,
        ) {
            for (i, inst) in ctx.instances.iter().enumerate() {
                let Some(d) = scene.defs.get(inst.def) else { continue };
                path.push(i);
                out.push((depth, d.name.clone(), d.component, path.clone()));
                if depth < 32 {
                    walk(scene, &d.ctx, path, out, depth + 1);
                }
                path.pop();
            }
        }
        let mut out = Vec::new();
        walk(&self.scene, &self.scene.root, &mut Vec::new(), &mut out, 0);
        out
    }

    pub fn open_names(&self) -> Vec<String> {
        (1..=self.open.len())
            .filter_map(|k| {
                let parent = self.scene.ctx(&self.open[..k - 1])?;
                Some(self.scene.defs.get(parent.instances.get(self.open[k - 1])?.def)?.name.clone())
            })
            .collect()
    }

    /// The state the model is in now (see the field). Keep it at a save; the model is unchanged while this equals it.
    pub fn state(&self) -> u64 {
        self.state
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot { mesh: self.mesh.clone(), scene: self.scene.clone(), open: self.open.clone() }
    }

    fn restore(&mut self, s: Snapshot) -> Snapshot {
        let now = self.snapshot();
        self.mesh = s.mesh;
        self.scene = s.scene;
        self.open = s.open;
        self.revision = fresh_revision();
        self.scene_revision = self.revision;
        now
    }

    fn edit<R>(&mut self, f: impl FnOnce(&mut HalfEdgeMesh) -> Result<R, TopologyError>) -> Result<R, TopologyError> {
        self.edit_with(false, |m, _, _| f(m))
    }

    /// An edit that may change the scene too (groups, notes): one undo step, undone whole when it fails.
    fn edit_scene<R>(
        &mut self,
        f: impl FnOnce(&mut HalfEdgeMesh, &mut Scene, &[usize]) -> Result<R, TopologyError>,
    ) -> Result<R, TopologyError> {
        self.edit_with(true, f)
    }

    /// An edit as one undo step; `scene`: `f` may change the scene (else it only gets to see the geometry being edited).
    fn edit_with<R>(
        &mut self,
        scene: bool,
        f: impl FnOnce(&mut HalfEdgeMesh, &mut Scene, &[usize]) -> Result<R, TopologyError>,
    ) -> Result<R, TopologyError> {
        let before = self.snapshot();
        let open = self.open.clone();
        let scene_before = self.scene_revision;
        let done = f(&mut self.mesh, &mut self.scene, &open);
        self.revision = fresh_revision();
        if scene {
            self.scene_revision = self.revision;
        }
        match done {
            Ok(r) => {
                self.undo.push((before, self.state));
                self.redo.clear();
                self.next_state += 1;
                self.state = self.next_state;
                Ok(r)
            }
            Err(e) => {
                self.restore(before);
                if !scene {
                    // `f` never saw the scene, and what was put back is a copy of it
                    self.scene_revision = scene_before;
                }
                Err(e)
            }
        }
    }

    /// Changes the notes (a guide placed, a dimension deleted ...), as one undo step.
    pub fn edit_notes(&mut self, f: impl FnOnce(&mut Notes)) {
        let _ = self.edit_scene(|_, s, _| {
            f(&mut s.notes);
            Ok(())
        });
    }

    /// Make Group / Make Component: `faces` of the open context, and its `instances`, become one new instance of a new
    /// definition, whose origin is the lowest corner of what went in. Returns the new instance's index.
    pub fn make_group(&mut self, faces: &[usize], instances: &[usize], component: bool) -> Result<usize, TopologyError> {
        self.edit_scene(|m, s, open| {
            let w = s.world(open);
            let back = w.inverse().ok_or(TopologyError::DegenerateNormal)?;
            let mesh = m.to_mesh();
            if let Some(&bad) = faces.iter().find(|&&f| f >= mesh.faces.len()) {
                return Err(TopologyError::FaceOutOfRange(bad));
            }
            let ctx = s.ctx(open).ok_or_else(nothing)?;
            let mut taken: Vec<usize> = instances.to_vec();
            taken.sort_unstable();
            taken.dedup();
            if taken.iter().any(|&i| i >= ctx.instances.len()) || (faces.is_empty() && taken.is_empty()) {
                return Err(nothing());
            }
            let (rest, part) = if faces.is_empty() { (mesh.clone(), Mesh::new()) } else { mesh.extract(faces) };
            // the lowest corner of the faces and of the instances' origins, in the open context's coordinates
            let mut pts: Vec<[f64; 3]> = part.positions.iter().map(|&p| back.apply(p)).collect();
            pts.extend(taken.iter().map(|&i| ctx.instances[i].xf.origin()));
            let corner = pts.iter().fold([f64::INFINITY; 3], |lo, p| [lo[0].min(p[0]), lo[1].min(p[1]), lo[2].min(p[2])]);
            let xf = Xform::translation(corner);
            let to_def = w.after(&xf).inverse().ok_or(TopologyError::DegenerateNormal)?;
            let un = xf.inverse().ok_or(TopologyError::DegenerateNormal)?;
            let inner: Vec<Instance> =
                taken.iter().map(|&i| Instance { def: ctx.instances[i].def, xf: un.after(&ctx.instances[i].xf) }).collect();
            let name = s.new_name(component);
            s.defs.push(Definition {
                name,
                component,
                glue: false,
                cut: false,
                attributes: Vec::new(),
                part: None,
                ctx: Context { mesh: part.transformed(&to_def), instances: inner },
            });
            let def = s.defs.len() - 1;
            let ctx = s.ctx_mut(open).ok_or_else(nothing)?;
            for &i in taken.iter().rev() {
                ctx.instances.remove(i);
            }
            ctx.instances.push(Instance { def, xf });
            let n = ctx.instances.len() - 1;
            *m = HalfEdgeMesh::from_mesh(&rest)?;
            Ok(n)
        })
    }

    /// Adds `mesh` (in model coordinates) as a new group or component in the open context, its origin at the lowest
    /// corner (3D Text). One undo step; returns the new instance's index.
    pub fn place_group(&mut self, mesh: &Mesh, component: bool) -> Result<usize, TopologyError> {
        if mesh.faces.is_empty() {
            return Err(nothing());
        }
        HalfEdgeMesh::from_mesh(mesh)?;
        self.edit_scene(|_, s, open| {
            let w = s.world(open);
            let back = w.inverse().ok_or(TopologyError::DegenerateNormal)?;
            let (lo, _) = mesh.bounds().ok_or_else(nothing)?;
            let xf = Xform::translation(back.apply(lo));
            let to_def = w.after(&xf).inverse().ok_or(TopologyError::DegenerateNormal)?;
            let name = s.new_name(component);
            s.defs.push(Definition {
                name,
                component,
                glue: false,
                cut: false,
                attributes: Vec::new(),
                part: None,
                ctx: Context { mesh: mesh.transformed(&to_def), instances: Vec::new() },
            });
            let def = s.defs.len() - 1;
            let ctx = s.ctx_mut(open).ok_or_else(nothing)?;
            ctx.instances.push(Instance { def, xf });
            Ok(ctx.instances.len() - 1)
        })
    }

    /// An exact part (sheet metal): a new component in the open context standing for `text` (the part's own description,
    /// see `Definition::part`), its faces `faces` in the part's coordinates (metres), put at `place` (model coordinates).
    /// `from_face`: a face of the geometry being edited that the part is made from; it goes, as it would into a component.
    /// One undo step; returns the new instance.
    pub fn place_part(
        &mut self,
        stem: &str,
        text: String,
        faces: &Mesh,
        place: &Xform,
        from_face: Option<usize>,
    ) -> Result<usize, TopologyError> {
        if faces.faces.is_empty() {
            return Err(nothing());
        }
        self.edit_scene(|m, s, open| {
            let back = s.world(open).inverse().ok_or(TopologyError::DegenerateNormal)?;
            if let Some(f) = from_face {
                let mesh = m.to_mesh();
                if f >= mesh.faces.len() {
                    return Err(TopologyError::FaceOutOfRange(f));
                }
                let (rest, _) = mesh.extract(&[f]);
                *m = HalfEdgeMesh::from_mesh(&rest)?;
            }
            let n = s.defs.iter().filter_map(|d| d.name.strip_prefix(stem)?.strip_prefix('#')?.parse::<u32>().ok()).max().unwrap_or(0);
            let name = format!("{stem}#{}", n + 1);
            s.defs.push(Definition {
                name,
                component: true,
                glue: false,
                cut: false,
                attributes: Vec::new(),
                part: Some(text),
                ctx: Context { mesh: faces.clone(), instances: Vec::new() },
            });
            let def = s.defs.len() - 1;
            let ctx = s.ctx_mut(open).ok_or_else(nothing)?;
            ctx.instances.push(Instance { def, xf: back.after(place) });
            Ok(ctx.instances.len() - 1)
        })
    }

    /// A part's new description and faces (every copy of it changes), as one undo step. A group or component that is no
    /// part yet becomes one (Convert to Sheet Metal): it must have no groups inside it, and it becomes a component.
    pub fn set_part(&mut self, def: usize, text: String, faces: Mesh) -> Result<(), TopologyError> {
        self.edit_scene(|_, s, _| {
            let d = s.defs.get_mut(def).ok_or_else(nothing)?;
            if d.part.is_none() {
                if !d.ctx.instances.is_empty() {
                    return Err(nothing());
                }
                d.component = true;
            }
            d.part = Some(text);
            d.ctx.mesh = faces;
            Ok(())
        })
    }

    /// Solid Tools on instances `first` and `second` of the open context (both must be closed solids), as SketchUp's: Union
    /// and Outer Shell join them; Intersect keeps what they share; Subtract takes `first` away from `second` (and `first`
    /// goes); Trim does the same but keeps `first`; Split leaves the three parts (each alone, and the shared part). The
    /// results are new groups, all as one undo step. Returns their instance numbers.
    pub fn solid_tool(&mut self, first: usize, second: usize, tool: SolidTool) -> Result<Vec<usize>, String> {
        use crate::csg::{boolean, Op};
        if first == second {
            return Err("pick two different solids".into());
        }
        let w = self.scene.world(&self.open);
        let solid = |i: usize| -> Result<Mesh, String> {
            let inst = *self.instances().get(i).ok_or("that is not a group or component here")?;
            let name = self.scene.defs.get(inst.def).map(|d| d.name.clone()).unwrap_or_default();
            let mut m = Mesh::new();
            self.scene.flatten_instance(&inst, &w, &mut m);
            let closed = HalfEdgeMesh::from_mesh(&m).map(|h| h.is_closed()).unwrap_or(false);
            if !closed || m.faces.is_empty() {
                return Err(format!("{name} is not a solid (it must be closed all round, every edge between two faces)"));
            }
            Ok(m)
        };
        let (a, b) = (solid(first)?, solid(second)?);
        let results: Vec<Mesh> = match tool {
            SolidTool::Union | SolidTool::OuterShell => vec![boolean(&a, &b, Op::Union)],
            SolidTool::Intersect => vec![boolean(&a, &b, Op::Intersect)],
            SolidTool::Subtract | SolidTool::Trim => vec![boolean(&b, &a, Op::Subtract)],
            SolidTool::Split => vec![boolean(&a, &b, Op::Subtract), boolean(&b, &a, Op::Subtract), boolean(&a, &b, Op::Intersect)],
        };
        let results: Vec<Mesh> = results.into_iter().filter(|m| !m.faces.is_empty()).collect();
        if results.is_empty() {
            return Err(match tool {
                SolidTool::Intersect => "they do not overlap: there is nothing they share".into(),
                _ => "nothing is left".into(),
            });
        }
        for r in &results {
            HalfEdgeMesh::from_mesh(r).map_err(|e| format!("the result is not a clean surface ({e}); nothing was changed"))?;
        }
        let mut gone = if tool == SolidTool::Trim { vec![second] } else { vec![first, second] };
        gone.sort_unstable_by(|x, y| y.cmp(x));
        self.edit_scene(|_, s, open| {
            let w = s.world(open);
            let back = w.inverse().ok_or(TopologyError::DegenerateNormal)?;
            let ctx = s.ctx_mut(open).ok_or_else(nothing)?;
            for &i in &gone {
                ctx.instances.remove(i);
            }
            let mut made = Vec::new();
            for r in &results {
                let (lo, _) = r.bounds().ok_or_else(nothing)?;
                let xf = Xform::translation(back.apply(lo));
                let to_def = w.after(&xf).inverse().ok_or(TopologyError::DegenerateNormal)?;
                let name = s.new_name(false);
                s.defs.push(Definition {
                    name,
                    component: false,
                    glue: false,
                    cut: false,
                    attributes: Vec::new(),
                    part: None,
                    ctx: Context { mesh: r.transformed(&to_def), instances: Vec::new() },
                });
                let def = s.defs.len() - 1;
                let ctx = s.ctx_mut(open).ok_or_else(nothing)?;
                ctx.instances.push(Instance { def, xf });
                made.push(ctx.instances.len() - 1);
            }
            s.purge();
            Ok(made)
        })
        .map_err(|e| e.to_string())
    }

    /// Sandbox From Contours: a terrain group through the corners of `edges` (selected contour lines of the open context).
    pub fn sandbox_from_contours(&mut self, edges: &[[u32; 2]]) -> Result<usize, String> {
        let mesh = self.mesh.to_mesh();
        let pts: Vec<[f64; 3]> = edges.iter().flatten().filter_map(|&v| mesh.positions.get(v as usize).copied()).collect();
        if pts.len() < 3 {
            return Err("select contour lines first (three corners at least)".into());
        }
        let t = crate::sandbox::from_contours(&pts, mesh.tolerance().max(1e-6) * 10.0)
            .ok_or("those lines make no ground (are they all in a row?)")?;
        self.place_group(&t, false).map_err(|e| e.to_string())
    }

    /// Sandbox From Scratch: a flat terrain group of squares `spacing` across, over the rectangle from `a` to `b` (seen from
    /// above, at `a`'s height).
    pub fn sandbox_from_scratch(&mut self, a: [f64; 3], b: [f64; 3], spacing: f64) -> Result<usize, String> {
        let lo = [a[0].min(b[0]), a[1].min(b[1]), a[2]];
        let t = crate::sandbox::grid(lo, (a[0] - b[0]).abs(), (a[1] - b[1]).abs(), spacing)
            .ok_or("drag out a rectangle, and keep the grid coarser than 250,000 corners")?;
        self.place_group(&t, false).map_err(|e| e.to_string())
    }

    /// Smoove on the open context (a terrain being edited): corners within `radius` of `center` raised by `height`, easing
    /// to nothing at the edge. One undo step.
    pub fn smoove(&mut self, center: [f64; 3], radius: f64, height: f64) -> Result<usize, TopologyError> {
        self.edit(|m| {
            let mut mesh = m.to_mesh();
            let n = crate::sandbox::smoove(&mut mesh, center, radius, height);
            *m = HalfEdgeMesh::from_mesh(&mesh)?;
            Ok(n)
        })
    }

    /// Add Detail on faces of the open context. One undo step.
    pub fn add_detail(&mut self, faces: &[usize]) -> Result<(), TopologyError> {
        self.edit(|m| {
            let mut mesh = m.to_mesh();
            crate::sandbox::add_detail(&mut mesh, faces);
            *m = HalfEdgeMesh::from_mesh(&mesh)?;
            Ok(())
        })
    }

    /// Flip Edge on the edge `a`-`b` of the open context. One undo step; fails (changing nothing) when it cannot flip.
    pub fn flip_edge(&mut self, a: u32, b: u32) -> Result<(), TopologyError> {
        self.edit(|m| {
            let mut mesh = m.to_mesh();
            crate::sandbox::flip_edge(&mut mesh, a, b).ok_or(TopologyError::InvalidSplit)?;
            *m = HalfEdgeMesh::from_mesh(&mesh)?;
            Ok(())
        })
    }

    /// Stamp and Drape: instance `object` of the open context worked onto the terrain instance `terrain`. Stamp flattens the
    /// ground under the object's lowest face to its height, easing back within `offset` round it; Drape lays the object's
    /// edges onto the ground as lines. The terrain's own geometry changes (every copy of it, if it is a component). One
    /// undo step.
    pub fn sandbox_onto(&mut self, terrain: usize, object: usize, drape: bool, offset: f64) -> Result<(), String> {
        if terrain == object {
            return Err("click the object first, then the ground".into());
        }
        let w = self.scene.world(&self.open);
        let t_inst = *self.instances().get(terrain).ok_or("that is not a group here")?;
        let o_inst = *self.instances().get(object).ok_or("that is not a group here")?;
        let place = w.after(&t_inst.xf);
        let back = place.inverse().ok_or("the ground's group cannot be turned back")?;
        let mut ground = self.scene.defs.get(t_inst.def).ok_or("no such group")?.ctx.mesh.transformed(&place);
        let mut obj = Mesh::new();
        self.scene.flatten_instance(&o_inst, &w, &mut obj);
        if obj.faces.is_empty() && obj.lines.is_empty() {
            return Err("the object is empty".into());
        }
        if drape {
            let lines: Vec<(u32, u32)> = obj.visible_edges();
            let mut runs = Vec::new();
            for (a, b) in lines {
                let (pa, pb) = (obj.positions[a as usize], obj.positions[b as usize]);
                runs.extend(crate::sandbox::drape(&ground, &[[pa[0], pa[1]], [pb[0], pb[1]]]));
            }
            if runs.is_empty() {
                return Err("the object's edges are not over the ground".into());
            }
            let mut h = HalfEdgeMesh::from_mesh(&ground).map_err(|e| e.to_string())?;
            for run in runs {
                for seg in run.windows(2) {
                    h.add_line(seg[0], seg[1]).map_err(|e| e.to_string())?;
                }
            }
            ground = h.to_mesh();
        } else {
            // the object's lowest face, seen from above, and its height
            let low = (0..obj.faces.len())
                .filter(|&f| crate::imprint::face_plane(&obj, &obj.faces[f]).is_some_and(|(n, _)| n[2] < -0.9))
                .min_by(|&a, &b| {
                    let z = |f: usize| obj.faces[f].iter().map(|&v| obj.positions[v as usize][2]).fold(f64::MAX, f64::min);
                    z(a).total_cmp(&z(b))
                })
                .ok_or("the object has no face underneath to stamp with")?;
            let ring: Vec<[f64; 2]> =
                obj.faces[low].iter().map(|&v| [obj.positions[v as usize][0], obj.positions[v as usize][1]]).collect();
            let z = obj.positions[obj.faces[low][0] as usize][2];
            if crate::sandbox::stamp(&mut ground, &ring, z, offset) == 0 {
                return Err("the object is not over the ground".into());
            }
        }
        let local = ground.transformed(&back);
        HalfEdgeMesh::from_mesh(&local).map_err(|e| e.to_string())?;
        self.edit_scene(|_, s, _| {
            s.defs.get_mut(t_inst.def).ok_or_else(nothing)?.ctx.mesh = local;
            Ok(())
        })
        .map_err(|e| e.to_string())
    }

    /// Dynamic Components: definition `def`'s attributes.
    pub fn attributes(&self, def: usize) -> &[crate::dynamic::Attribute] {
        self.scene.defs.get(def).map(|d| d.attributes.as_slice()).unwrap_or(&[])
    }

    /// The values of copy `i` (of the open context)'s attributes, the built-ins included (from where it is).
    pub fn attribute_values(&self, i: usize) -> Result<std::collections::HashMap<String, crate::dynamic::Value>, String> {
        let inst = *self.instances().get(i).ok_or("that is not a component here")?;
        let given = given_for(&self.scene, &inst, &owner_values(&self.scene, &self.open), 0);
        crate::dynamic::evaluate(&self.scene.defs[inst.def].attributes, &given)
    }

    /// Component Attributes: definition `def`'s attributes replaced by `attrs`, and every copy of it in the open context
    /// placed and sized by them. One undo step; nothing changes when a formula goes wrong.
    pub fn set_attributes(&mut self, def: usize, attrs: Vec<crate::dynamic::Attribute>) -> Result<(), String> {
        if def >= self.scene.defs.len() {
            return Err("no such component".into());
        }
        let mut trial = self.scene.clone();
        trial.defs[def].attributes = attrs;
        apply_dynamic(&mut trial, &self.open, def)?;
        self.edit_scene(|_, s, _| {
            *s = trial;
            Ok(())
        })
        .map_err(|e| e.to_string())
    }

    /// Component Options: copy `i`'s attributes become `attrs`, for that copy alone. A component placed more than once gets a
    /// definition of its own first (`Door#2` beside `Door`, as SketchUp's dynamic components do), so the other copies keep
    /// their values. One undo step; nothing changes when a formula goes wrong.
    pub fn set_copy_attributes(&mut self, i: usize, attrs: Vec<crate::dynamic::Attribute>) -> Result<(), String> {
        let inst = *self.instances().get(i).ok_or("that is not a component here")?;
        let mut trial = self.scene.clone();
        let def = if trial.uses(inst.def) > 1 {
            let mut d = trial.defs[inst.def].clone();
            d.name = copy_name(&trial, &d.name);
            trial.defs.push(d);
            let def = trial.defs.len() - 1;
            trial.ctx_mut(&self.open).ok_or("that is not a component here")?.instances[i].def = def;
            def
        } else {
            inst.def
        };
        trial.defs[def].attributes = attrs;
        apply_dynamic(&mut trial, &self.open, def)?;
        self.edit_scene(|_, s, _| {
            *s = trial;
            Ok(())
        })
        .map_err(|e| e.to_string())
    }

    /// Interact: copy `i`'s onClick actions run; they change that copy's attributes alone (see
    /// [`set_copy_attributes`](Self::set_copy_attributes)). Returns whether it had anything to do.
    pub fn interact(&mut self, i: usize) -> Result<bool, String> {
        let inst = *self.instances().get(i).ok_or("click a dynamic component")?;
        let mut attrs = self.scene.defs[inst.def].attributes.clone();
        let given = given_for(&self.scene, &inst, &owner_values(&self.scene, &self.open), 0);
        if !crate::dynamic::on_click(&mut attrs, &given)? {
            return Ok(false);
        }
        self.set_copy_attributes(i, attrs)?;
        Ok(true)
    }

    /// Explode: instance `i`'s geometry joins the open context's (welded where it touches, when that stays a valid
    /// surface), and its own instances become the context's.
    pub fn explode(&mut self, i: usize) -> Result<(), TopologyError> {
        self.edit_scene(|m, s, open| {
            let w = s.world(open);
            let inst = *s.ctx(open).and_then(|c| c.instances.get(i)).ok_or_else(nothing)?;
            let def = s.defs.get(inst.def).ok_or_else(nothing)?.ctx.clone();
            let geo = def.mesh.transformed(&w.after(&inst.xf));
            let mut joined = m.to_mesh();
            let tol = joined.tolerance().max(geo.tolerance());
            joined.append_welded(&geo, tol);
            *m = match HalfEdgeMesh::from_mesh(&joined) {
                Ok(h) => h,
                Err(_) => {
                    let mut apart = m.to_mesh();
                    apart.append(&geo);
                    HalfEdgeMesh::from_mesh(&apart)?
                }
            };
            let ctx = s.ctx_mut(open).ok_or_else(nothing)?;
            ctx.instances.remove(i);
            ctx.instances.extend(def.instances.iter().map(|n| Instance { def: n.def, xf: inst.xf.after(&n.xf) }));
            s.purge();
            Ok(())
        })
    }

    /// Move, Rotate, Scale on instances: `t` (in model coordinates) applied to each, as one undo step.
    pub fn transform_instances(&mut self, ids: &[usize], t: &Xform) -> Result<(), TopologyError> {
        self.edit_scene(|_, s, open| {
            let w = s.world(open);
            let back = w.inverse().ok_or(TopologyError::DegenerateNormal)?;
            let local = back.after(&t.after(&w));
            let ctx = s.ctx_mut(open).ok_or_else(nothing)?;
            for &i in ids {
                let inst = ctx.instances.get_mut(i).ok_or_else(nothing)?;
                inst.xf = local.after(&inst.xf);
            }
            Ok(())
        })
    }

    /// Move or Rotate with Ctrl on instances: copies of each, `t` applied, as one undo step. A group's copy gets its own
    /// definition; a component's copy shares it. Returns the copies' indices.
    pub fn copy_instances(&mut self, ids: &[usize], t: &Xform) -> Result<Vec<usize>, TopologyError> {
        self.edit_scene(|_, s, open| {
            let w = s.world(open);
            let back = w.inverse().ok_or(TopologyError::DegenerateNormal)?;
            let local = back.after(&t.after(&w));
            let mut made = Vec::new();
            for &i in ids {
                let inst = *s.ctx(open).and_then(|c| c.instances.get(i)).ok_or_else(nothing)?;
                let def = if s.defs[inst.def].component {
                    inst.def
                } else {
                    let mut d = s.defs[inst.def].clone();
                    d.name = s.new_name(false);
                    s.defs.push(d);
                    s.defs.len() - 1
                };
                let ctx = s.ctx_mut(open).ok_or_else(nothing)?;
                ctx.instances.push(Instance { def, xf: local.after(&inst.xf) });
                made.push(ctx.instances.len() - 1);
            }
            Ok(made)
        })
    }

    /// Move with Ctrl on loose geometry: a copy of `faces`, `t` applied, added beside them (welded where it touches).
    pub fn copy_faces(&mut self, faces: &[usize], t: &Xform) -> Result<(), TopologyError> {
        self.edit(|m| {
            let mesh = m.to_mesh();
            if let Some(&bad) = faces.iter().find(|&&f| f >= mesh.faces.len()) {
                return Err(TopologyError::FaceOutOfRange(bad));
            }
            let (_, part) = mesh.extract(faces);
            let mut joined = mesh.clone();
            {
                let part = part.transformed(t);
                let tol = joined.tolerance().max(part.tolerance());
                joined.append_welded(&part, tol);
            }
            *m = HalfEdgeMesh::from_mesh(&joined)?;
            Ok(())
        })
    }

    /// Move, Rotate, Scale on a mixed selection: `vertices` of the open context and its `instances`, `t` applied to
    /// both, as one undo step.
    pub fn transform_all(&mut self, vertices: &[u32], instances: &[usize], t: &Xform) -> Result<(), TopologyError> {
        self.edit_scene(|m, s, open| {
            // components that cut an opening take it with them: their old holes go before the move, new ones are cut after
            let cuts = |s: &Scene| {
                s.ctx(open).is_some_and(|c| {
                    instances.iter().filter_map(|&i| c.instances.get(i)).any(|inst| s.defs.get(inst.def).is_some_and(|d| d.cut))
                })
            };
            let cutting = |s: &Scene, host: &Mesh| -> Vec<Vec<[f64; 3]>> {
                let Some(ctx) = s.ctx(open) else { return Vec::new() };
                instances
                    .iter()
                    .filter_map(|&i| ctx.instances.get(i))
                    .filter_map(|inst| crate::opening::footprint(s, open, inst, host))
                    .collect()
            };
            let mut mesh = cuts(s).then(|| m.to_mesh());
            if let Some(mesh) = mesh.as_mut() {
                for o in cutting(s, mesh) {
                    crate::opening::uncut(mesh, &o);
                }
            }
            for &v in vertices {
                let p = match mesh.as_mut() {
                    Some(mesh) => mesh.positions.get_mut(v as usize),
                    None => m.positions.get_mut(v as usize),
                };
                if let Some(p) = p {
                    *p = t.apply(*p);
                }
            }
            let w = s.world(open);
            let local = w.inverse().ok_or(TopologyError::DegenerateNormal)?.after(&t.after(&w));
            let ctx = s.ctx_mut(open).ok_or_else(nothing)?;
            for &i in instances {
                let inst = ctx.instances.get_mut(i).ok_or_else(nothing)?;
                inst.xf = local.after(&inst.xf);
            }
            if let Some(mut mesh) = mesh {
                for o in cutting(s, &mesh) {
                    crate::opening::cut(&mut mesh, &o);
                }
                *m = HalfEdgeMesh::from_mesh(&mesh)?;
            }
            Ok(())
        })
    }

    /// The plane a glued instance `i` of the open context slides in (its face's normal), if it is glued and sits on one.
    pub fn glue_normal(&self, i: usize) -> Option<[f64; 3]> {
        let inst = self.instances().get(i)?;
        crate::opening::glue_normal(&self.scene, &self.open, inst, &self.mesh.to_mesh())
    }

    /// Entity Info on a component: glued to faces, and cutting an opening in the face it is on. The openings of its copies
    /// in the open context are cut or taken away to match, as one undo step.
    pub fn set_glue_cut(&mut self, def: usize, glue: bool, cut: bool) -> Result<(), TopologyError> {
        self.edit_scene(|m, s, open| {
            let d = s.defs.get(def).ok_or_else(nothing)?;
            let was = d.cut;
            let copies: Vec<Instance> =
                s.ctx(open).map(|c| c.instances.iter().copied().filter(|i| i.def == def).collect()).unwrap_or_default();
            let mut mesh = m.to_mesh();
            if was {
                for inst in &copies {
                    if let Some(o) = crate::opening::footprint(s, open, inst, &mesh.clone()) {
                        crate::opening::uncut(&mut mesh, &o);
                    }
                }
            }
            s.defs[def].glue = glue || cut;
            s.defs[def].cut = cut;
            if cut {
                for inst in &copies {
                    if let Some(o) = crate::opening::footprint(s, open, inst, &mesh.clone()) {
                        crate::opening::cut(&mut mesh, &o);
                    }
                }
            }
            *m = HalfEdgeMesh::from_mesh(&mesh)?;
            Ok(())
        })
    }

    /// Move with Ctrl, and arrays: for each transform in `ts`, a copy of `faces` and of `instances` put there, all as
    /// one undo step. Returns the new faces and the new instances (the last copy's, which SketchUp leaves selected).
    pub fn copy_all(&mut self, faces: &[usize], instances: &[usize], ts: &[Xform]) -> Result<(Vec<usize>, Vec<usize>), TopologyError> {
        self.edit_scene(|m, s, open| {
            let mut made = (Vec::new(), Vec::new());
            let w = s.world(open);
            let back = w.inverse().ok_or(TopologyError::DegenerateNormal)?;
            for t in ts {
                if !faces.is_empty() {
                    let mesh = m.to_mesh();
                    if let Some(&bad) = faces.iter().find(|&&f| f >= mesh.faces.len()) {
                        return Err(TopologyError::FaceOutOfRange(bad));
                    }
                    let (_, part) = mesh.extract(faces);
                    let mut joined = mesh.clone();
                    {
                        let part = part.transformed(t);
                        let tol = joined.tolerance().max(part.tolerance());
                        joined.append_welded(&part, tol);
                    }
                    *m = HalfEdgeMesh::from_mesh(&joined)?;
                    made.0 = (mesh.faces.len()..joined.faces.len()).collect();
                }
                let local = back.after(&t.after(&w));
                made.1.clear();
                for &i in instances {
                    let inst = *s.ctx(open).and_then(|c| c.instances.get(i)).ok_or_else(nothing)?;
                    let def = if s.defs[inst.def].component {
                        inst.def
                    } else {
                        let mut d = s.defs[inst.def].clone();
                        d.name = s.new_name(false);
                        s.defs.push(d);
                        s.defs.len() - 1
                    };
                    let copy = Instance { def, xf: local.after(&inst.xf) };
                    // a copy of a component that cuts an opening cuts its own
                    let mut mesh = m.to_mesh();
                    if let Some(o) = crate::opening::footprint(s, open, &copy, &mesh) {
                        if crate::opening::cut(&mut mesh, &o) {
                            *m = HalfEdgeMesh::from_mesh(&mesh)?;
                        }
                    }
                    let ctx = s.ctx_mut(open).ok_or_else(nothing)?;
                    ctx.instances.push(copy);
                    made.1.push(ctx.instances.len() - 1);
                }
            }
            Ok(made)
        })
    }

    /// Delete on a mixed selection: faces, edges (vertex pairs, see [`HalfEdgeMesh::erase_edges`]) and instances of the open
    /// context, as one undo step. Returns how many edges were kept (two faces in one plane that could not become one).
    pub fn erase_all(&mut self, faces: &[usize], edges: &[[u32; 2]], instances: &[usize]) -> Result<usize, TopologyError> {
        let mut faces = faces.to_vec();
        faces.sort_unstable_by(|a, b| b.cmp(a));
        faces.dedup();
        let mut ids = instances.to_vec();
        ids.sort_unstable_by(|a, b| b.cmp(a));
        ids.dedup();
        self.edit_scene(|m, s, open| {
            faces.iter().try_for_each(|&f| m.erase_face(f))?;
            let kept = if edges.is_empty() { 0 } else { m.erase_edges(edges)? };
            uncut_openings(m, s, open, &ids)?;
            let ctx = s.ctx_mut(open).ok_or_else(nothing)?;
            for &i in &ids {
                if i >= ctx.instances.len() {
                    return Err(nothing());
                }
                ctx.instances.remove(i);
            }
            s.purge();
            Ok(kept)
        })
    }

    /// Replaces the open context's geometry (Repair Solid), as one undo step.
    pub fn set_mesh(&mut self, mesh: HalfEdgeMesh) {
        let _ = self.edit(|m| {
            *m = mesh;
            Ok(())
        });
    }

    /// Deletes instances of the open context, as one undo step.
    pub fn erase_instances(&mut self, ids: &[usize]) -> Result<(), TopologyError> {
        let mut ids = ids.to_vec();
        ids.sort_unstable_by(|a, b| b.cmp(a));
        ids.dedup();
        self.edit_scene(|m, s, open| {
            uncut_openings(m, s, open, &ids)?;
            let ctx = s.ctx_mut(open).ok_or_else(nothing)?;
            for &i in &ids {
                if i >= ctx.instances.len() {
                    return Err(nothing());
                }
                ctx.instances.remove(i);
            }
            s.purge();
            Ok(())
        })
    }

    /// Make Unique: instance `i` of a component gets a definition of its own, so editing it leaves the other copies alone.
    pub fn make_unique(&mut self, i: usize) -> Result<(), TopologyError> {
        self.edit_scene(|_, s, open| {
            let inst = *s.ctx(open).and_then(|c| c.instances.get(i)).ok_or_else(nothing)?;
            let mut d = s.defs[inst.def].clone();
            d.name = s.new_name(d.component);
            s.defs.push(d);
            let def = s.defs.len() - 1;
            s.ctx_mut(open).ok_or_else(nothing)?.instances[i].def = def;
            Ok(())
        })
    }

    /// Edit Group / Edit Component: opens instance `i` of the open context, so the tools work on its inside. Not an undo
    /// step (an edit made inside it is).
    pub fn open(&mut self, i: usize) -> Result<(), TopologyError> {
        let inst = *self.instances().get(i).ok_or_else(nothing)?;
        let w = self.scene.world(&self.open);
        let back = w.inverse().ok_or(TopologyError::DegenerateNormal)?;
        let inside = self.scene.defs[inst.def].ctx.mesh.transformed(&w.after(&inst.xf));
        let mesh = HalfEdgeMesh::from_mesh(&inside)?;
        let local = if self.open.is_empty() { self.mesh.to_mesh() } else { self.mesh.to_mesh().transformed(&back) };
        if let Some(c) = self.scene.ctx_mut(&self.open) {
            c.mesh = local;
        }
        self.open.push(i);
        self.mesh = mesh;
        self.revision = fresh_revision();
        self.scene_revision = self.revision;
        Ok(())
    }

    /// Close Group / Component: back to the context around it. False when nothing is open.
    pub fn close(&mut self) -> bool {
        if self.open.is_empty() {
            return false;
        }
        let back = self.scene.world(&self.open).inverse().unwrap_or(Xform::IDENTITY);
        let local = self.mesh.to_mesh().transformed(&back);
        if let Some(c) = self.scene.ctx_mut(&self.open) {
            c.mesh = local;
        }
        self.open.pop();
        let w = self.scene.world(&self.open);
        let outer = self.scene.ctx(&self.open).map(|c| c.mesh.clone()).unwrap_or_default();
        let outer = if self.open.is_empty() { outer } else { outer.transformed(&w) };
        // it was editable when it was opened
        self.mesh = HalfEdgeMesh::from_mesh(&outer).unwrap_or_default();
        self.revision = fresh_revision();
        self.scene_revision = self.revision;
        true
    }

    /// Closes every open group and component.
    pub fn close_all(&mut self) {
        while self.close() {}
    }

    /// Draws a rectangle spanned by edge vectors `u` and `v` from `origin`.
    pub fn add_rectangle(&mut self, origin: [f64; 3], u: [f64; 3], v: [f64; 3]) -> Result<usize, TopologyError> {
        let add = |a: [f64; 3], b: [f64; 3]| [a[0] + b[0], a[1] + b[1], a[2] + b[2]];
        let pts = [origin, add(origin, u), add(add(origin, u), v), add(origin, v)];
        self.edit(|m| m.add_polygon(&pts))
    }

    /// Adds a closed outline as a new face (Line, Rectangle, Circle and Polygon tools).
    pub fn add_polygon(&mut self, points: &[[f64; 3]]) -> Result<usize, TopologyError> {
        self.edit(|m| m.add_polygon(points))
    }

    pub fn erase_face(&mut self, face: usize) -> Result<(), TopologyError> {
        self.edit(|m| m.erase_face(face))
    }

    /// Line tool: one segment, as one undo step. Returns the face it closed, if it closed one.
    pub fn add_line(&mut self, a: [f64; 3], b: [f64; 3]) -> Result<Option<usize>, TopologyError> {
        self.edit(|m| m.add_line(a, b))
    }

    /// A shape's outline (Rectangle, Circle, Polygon, Pie ...) drawn as its lines in one undo step, so it joins and cuts what
    /// is there like lines do: on a face it splits it, or cuts a hole that it fills. Returns the face it made, if any.
    pub fn add_outline(&mut self, points: &[[f64; 3]]) -> Result<Option<usize>, TopologyError> {
        if points.len() < 3 {
            return Err(TopologyError::DegenerateFace(self.mesh.face_count()));
        }
        self.edit(|m| {
            let mut made = None;
            for i in 0..points.len() {
                if let Some(f) = m.add_line(points[i], points[(i + 1) % points.len()])? {
                    made = Some(f);
                }
            }
            Ok(made)
        })
    }

    /// The arc or circle the edge `a`-`b` of the open context belongs to, if any (see [`crate::curve`]).
    pub fn curve_at(&self, a: u32, b: u32) -> Option<crate::curve::Curve> {
        crate::curve::curve_through(&self.mesh.to_mesh(), a, b)
    }

    /// Entity Info on an arc or circle: redraws it with `segments` segments and radius `radius`, as one undo step. Fails
    /// (and changes nothing) once it has been pushed or pulled into walls, as in SketchUp.
    pub fn reshape_curve(&mut self, a: u32, b: u32, segments: usize, radius: f64) -> Result<crate::curve::Curve, String> {
        let mesh = self.mesh.to_mesh();
        let c = crate::curve::curve_through(&mesh, a, b).ok_or("that edge is not part of an arc or circle")?;
        let mut trial = mesh.clone();
        let made = crate::curve::reshape(&mut trial, &c, segments, radius)?;
        HalfEdgeMesh::from_mesh(&trial).map_err(|e| e.to_string())?;
        self.edit(|m| {
            *m = HalfEdgeMesh::from_mesh(&trial)?;
            Ok(())
        })
        .map_err(|e| e.to_string())?;
        Ok(made)
    }

    /// Offset outward: a ring face round face `face`, its outline `outline` (the face's outline moved outward, same way
    /// round) and its hole the face's own outline, so the two share those edges. One undo step; returns the ring's index.
    /// Fails when the face's edges already border other faces (the ring would make them shared three ways).
    pub fn offset_outward(&mut self, face: usize, outline: &[[f64; 3]]) -> Result<usize, TopologyError> {
        if face >= self.mesh.face_count() {
            return Err(TopologyError::FaceOutOfRange(face));
        }
        self.edit(|m| {
            let mut mesh = m.to_mesh();
            let inner: Vec<u32> = mesh.faces[face].iter().rev().copied().collect();
            let ring: Vec<u32> = outline.iter().map(|&p| mesh.add_vertex(p)).collect();
            mesh.faces.push(ring);
            let f = mesh.faces.len() - 1;
            mesh.set_holes(f, vec![inner]);
            let c = mesh.color_of(face);
            mesh.set_color(f, c);
            *m = HalfEdgeMesh::from_mesh(&mesh)?;
            Ok(f)
        })
    }

    /// The textures faces can be painted with (`Paint::texture` numbers them).
    pub fn textures(&self) -> &[crate::scene::Texture] {
        &self.scene.textures
    }

    /// Adds a texture (or finds the one from the same file), as one undo step; returns its number.
    pub fn add_texture(&mut self, t: crate::scene::Texture) -> usize {
        if let Some(i) = self.scene.textures.iter().position(|x| x.path == t.path) {
            return i;
        }
        self.edit_scene(|_, s, _| {
            s.textures.push(t);
            Ok(s.textures.len() - 1)
        })
        .unwrap_or(0)
    }

    /// Entity Info on a textured face: the size of one tile of texture `i` (width, height, metres), as one undo step.
    pub fn set_texture_size(&mut self, i: usize, size: [f64; 2]) -> Result<(), TopologyError> {
        if !(size[0] > 0.0 && size[1] > 0.0) {
            return Err(TopologyError::ZeroDistance);
        }
        self.edit_scene(|_, s, _| {
            s.textures.get_mut(i).ok_or_else(nothing)?.size = size;
            Ok(())
        })
    }

    /// Paint Bucket: paints `faces` with `color` (None: back to the default material), as one undo step.
    pub fn paint(&mut self, faces: &[usize], color: Option<crate::Paint>) -> Result<(), TopologyError> {
        self.edit(|m| {
            let mut mesh = m.to_mesh();
            for &f in faces {
                if f >= mesh.faces.len() {
                    return Err(TopologyError::FaceOutOfRange(f));
                }
                mesh.set_color(f, color);
            }
            *m = HalfEdgeMesh::from_mesh(&mesh)?;
            Ok(())
        })
    }

    /// Follow Me: sweeps face `profile` along `path` (points in order; `closed` when it runs round back to its start). The
    /// profile is taken to stand at the path's first point; at every bend the cross-section is mitred (projected onto the
    /// plane halfway between the two directions). An open path gets end caps (the profile's place and the far end); a
    /// closed one makes a ring with no caps. One undo step; returns the end cap's face (or the last face made).
    pub fn follow_me(&mut self, profile: usize, path: &[[f64; 3]], closed: bool) -> Result<usize, TopologyError> {
        self.edit(|m| {
            let mut mesh = m.to_mesh();
            if profile >= mesh.faces.len() {
                return Err(TopologyError::FaceOutOfRange(profile));
            }
            let n = path.len();
            if n < 2 || (closed && n < 3) {
                return Err(TopologyError::InvalidSplit);
            }
            let sub = |a: [f64; 3], b: [f64; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
            let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
            let unit = |a: [f64; 3]| {
                let l = dot(a, a).sqrt();
                if l > 0.0 {
                    a.map(|c| c / l)
                } else {
                    a
                }
            };
            // the directions along the path, one per piece (a closed path has one more, back to the start)
            let pieces = if closed { n } else { n - 1 };
            let dirs: Vec<[f64; 3]> = (0..pieces).map(|i| unit(sub(path[(i + 1) % n], path[i]))).collect();
            if dirs.iter().any(|d| dot(*d, *d) < 0.5) {
                return Err(TopologyError::DegenerateFace(profile));
            }
            let color = mesh.color_of(profile);
            let (outline, holes) = mesh.remove_face_loops(profile);
            // the profile wound counter-clockwise about the way the path sets off (its holes the other way)
            let pts: Vec<[f64; 3]> = outline.iter().map(|&v| mesh.positions[v as usize]).collect();
            let mut nrm = [0.0f64; 3];
            for i in 0..pts.len() {
                let (p, q) = (pts[i], pts[(i + 1) % pts.len()]);
                nrm[0] += (p[1] - q[1]) * (p[2] + q[2]);
                nrm[1] += (p[2] - q[2]) * (p[0] + q[0]);
                nrm[2] += (p[0] - q[0]) * (p[1] + q[1]);
            }
            let mut loops: Vec<Vec<u32>> = std::iter::once(outline).chain(holes).collect();
            if dot(nrm, dirs[0]) < 0.0 {
                for l in &mut loops {
                    l.reverse();
                }
            }
            // a point carried along direction `d` onto the plane through `at` with normal `mitre`
            let carry = |mesh: &mut Mesh, v: u32, at: [f64; 3], d: [f64; 3], mitre: [f64; 3], denom: f64| {
                let p = mesh.positions[v as usize];
                let t = dot(sub(at, p), mitre) / denom;
                mesh.add_vertex([p[0] + d[0] * t, p[1] + d[1] * t, p[2] + d[2] * t])
            };
            // a closed path has a corner at its start too: the profile is carried onto that corner's mitre plane first
            if closed {
                let d0 = dirs[0];
                let last = dirs[pieces - 1];
                let mitre = unit([last[0] + d0[0], last[1] + d0[1], last[2] + d0[2]]);
                let denom = dot(d0, mitre);
                if denom.abs() < 1e-6 {
                    return Err(TopologyError::DegenerateNormal);
                }
                for l in &mut loops {
                    *l = l.iter().map(|&v| carry(&mut mesh, v, path[0], d0, mitre, denom)).collect();
                }
            }
            // each next cross-section: the last one carried along the piece onto the mitre plane at the next corner
            let mut sections: Vec<Vec<Vec<u32>>> = vec![loops.clone()];
            let corners = if closed { n } else { n - 1 };
            for i in 1..=corners {
                let at = path[i % n];
                let d_in = dirs[i - 1];
                let mitre = if !closed && i == n - 1 {
                    d_in
                } else {
                    unit([d_in[0] + dirs[i % pieces][0], d_in[1] + dirs[i % pieces][1], d_in[2] + dirs[i % pieces][2]])
                };
                let denom = dot(d_in, mitre);
                if denom.abs() < 1e-6 {
                    return Err(TopologyError::DegenerateNormal); // a bend that turns right back
                }
                if closed && i == n {
                    break; // the ring closes onto the first section
                }
                let prev = sections.last().expect("one at least").clone();
                let next: Vec<Vec<u32>> =
                    prev.iter().map(|l| l.iter().map(|&v| carry(&mut mesh, v, at, d_in, mitre, denom)).collect()).collect();
                sections.push(next);
            }
            if closed {
                // the last corner's section is the profile itself, carried round: join back to it
                let first = sections[0].clone();
                sections.push(first);
            }
            // walls along every loop: round the outside, and inside each hole (its loop runs the other way, so they face in)
            for w in sections.windows(2) {
                for (l0, l1) in w[0].iter().zip(&w[1]) {
                    let k = l0.len();
                    for j in 0..k {
                        let (a, b) = (l0[j], l0[(j + 1) % k]);
                        let (a2, b2) = (l1[j], l1[(j + 1) % k]);
                        mesh.faces.push(vec![a, b, b2, a2]);
                    }
                }
            }
            if !closed {
                // caps, with their holes: the start faces back along the path, the end forward
                let rev = |ls: &Vec<Vec<u32>>| -> Vec<Vec<u32>> { ls.iter().map(|l| l.iter().rev().copied().collect()).collect() };
                for mut cap in [rev(&sections[0]), sections.last().expect("two at least").clone()] {
                    let outline = cap.remove(0);
                    mesh.faces.push(outline);
                    let f = mesh.faces.len() - 1;
                    mesh.set_color(f, color);
                    mesh.set_holes(f, cap);
                }
            }
            *m = HalfEdgeMesh::from_mesh(&mesh)?;
            Ok(m.face_count() - 1)
        })
    }

    /// An open chain of lines (an arc, a freehand stroke) in one undo step. Returns the face it closed, if any.
    pub fn add_polyline(&mut self, points: &[[f64; 3]]) -> Result<Option<usize>, TopologyError> {
        self.edit(|m| {
            let mut made = None;
            for w in points.windows(2) {
                if let Some(f) = m.add_line(w[0], w[1])? {
                    made = Some(f);
                }
            }
            Ok(made)
        })
    }

    pub fn erase_line(&mut self, i: usize) -> Result<(), TopologyError> {
        self.edit(|m| m.erase_line(i))
    }

    /// Erases several faces as one undo step (Delete on a selection). Highest numbers first, so the others keep theirs.
    pub fn erase_faces(&mut self, faces: &[usize]) -> Result<(), TopologyError> {
        let mut faces = faces.to_vec();
        faces.sort_unstable_by(|a, b| b.cmp(a));
        faces.dedup();
        self.edit(|m| faces.iter().try_for_each(|&f| m.erase_face(f)))
    }

    /// Pushes or pulls a face, then merges coplanar faces as SketchUp does (pushing a face of a solid again leaves no seam).
    /// Returns the pushed face's index after the merge.
    pub fn push_pull(&mut self, face: usize, distance: f64) -> Result<usize, TopologyError> {
        self.edit(|m| {
            let f = m.push_pull(face, distance)?;
            Ok(merge_in_place(m, f))
        })
    }

    /// Merges coplanar neighbouring faces (Tools > Merge Coplanar Faces). Returns how many faces went.
    pub fn merge_coplanar(&mut self) -> Result<usize, TopologyError> {
        let before = self.mesh.face_count();
        let n = self.edit(|m| {
            merge_in_place(m, 0);
            Ok(before - m.face_count())
        })?;
        if n == 0 {
            // nothing merged: not a step worth undoing
            self.undo();
            self.redo.clear();
        }
        Ok(n)
    }

    /// Move, Rotate and Scale: puts each of `vertices` where `f` sends it, as one undo step. Faces keep their numbers.
    /// Faces that share some of the vertices but not all of them stretch to follow, as in SketchUp.
    pub fn transform_vertices(&mut self, vertices: &[u32], f: impl Fn([f64; 3]) -> [f64; 3]) -> Result<(), TopologyError> {
        self.edit(|m| {
            for &v in vertices {
                if let Some(p) = m.positions.get_mut(v as usize) {
                    *p = f(*p);
                }
            }
            Ok(())
        })
    }

    pub fn split_face(&mut self, face: usize, a: u32, b: u32) -> Result<usize, TopologyError> {
        self.edit(|m| m.split_face(face, a, b))
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo(&mut self) -> bool {
        match self.undo.pop() {
            Some((prev, state)) => {
                let now = self.restore(prev);
                self.redo.push((now, self.state));
                self.state = state;
                true
            }
            None => false,
        }
    }

    pub fn redo(&mut self) -> bool {
        match self.redo.pop() {
            Some((next, state)) => {
                let now = self.restore(next);
                self.undo.push((now, self.state));
                self.state = state;
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pushing_a_solid_face_again_merges_the_seam() {
        let mut m = Model::new();
        let f = m.add_rectangle([0.0; 3], [3.0, 0.0, 0.0], [0.0, 2.0, 0.0]).unwrap();
        let top = m.push_pull(f, 1.0).unwrap();
        let top = m.push_pull(top, 0.5).unwrap();
        assert_eq!(m.mesh().face_count(), 6);
        assert_eq!(m.mesh().vertex_count(), 8);
        assert!((m.mesh().signed_volume() - 9.0).abs() < 1e-4);
        assert_eq!(m.mesh().face_vertices(top).len(), 4);
        // the top can be pushed once more, by its returned index
        m.push_pull(top, -0.5).unwrap();
        assert!((m.mesh().signed_volume() - 6.0).abs() < 1e-4);
        m.undo();
        assert!((m.mesh().signed_volume() - 9.0).abs() < 1e-4, "push and merge are one undo step");
        let state = m.state();
        assert_eq!(m.merge_coplanar().unwrap(), 0);
        assert_eq!(m.state(), state, "merging nothing is no step");
    }

    #[test]
    fn the_state_follows_undo_and_redo() {
        let mut m = Model::new();
        let saved = m.state();
        let f = m.add_rectangle([0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]).unwrap();
        let drawn = m.state();
        assert_ne!(drawn, saved);
        m.undo();
        assert_eq!(m.state(), saved, "undone back to the save: unchanged");
        m.redo();
        assert_eq!(m.state(), drawn);
        m.push_pull(f, 1.0).unwrap();
        m.undo();
        m.undo();
        m.add_rectangle([5.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]).unwrap();
        assert_ne!(m.state(), saved, "a different edit from the same start is a change");
        assert_ne!(m.state(), drawn);
        assert!(m.push_pull(99, 1.0).is_err());
        let before = m.state();
        assert_eq!(m.state(), before, "a failed edit changes nothing");
    }

    #[test]
    fn draw_pull_undo_redo() {
        let mut m = Model::new();
        let f = m.add_rectangle([0.0; 3], [2.0, 0.0, 0.0], [0.0, 2.0, 0.0]).unwrap();
        m.push_pull(f, 3.0).unwrap();
        assert_eq!(m.mesh().face_count(), 6);
        assert!((m.mesh().signed_volume() - 12.0).abs() < 1e-4);

        assert!(m.undo());
        assert_eq!(m.mesh().face_count(), 1);
        assert!(m.undo());
        assert_eq!(m.mesh().face_count(), 0);
        assert!(!m.undo());

        assert!(m.redo() && m.redo());
        assert_eq!(m.mesh().face_count(), 6);
        assert!(!m.redo());
    }

    #[test]
    fn failed_edit_changes_nothing_and_adds_no_history() {
        let mut m = Model::new();
        m.add_rectangle([0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]).unwrap();
        assert!(m.push_pull(9, 1.0).is_err());
        assert_eq!(m.mesh().face_count(), 1);
        assert!(m.undo());
        assert!(!m.can_undo());
    }

    #[test]
    fn degenerate_outline_is_rejected_without_a_history_step() {
        let mut m = Model::new();
        assert!(m.add_polygon(&[[0.0; 3], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]]).is_err());
        assert_eq!(m.mesh().face_count(), 0);
        assert!(!m.can_undo());
    }

    #[test]
    fn new_edit_clears_redo() {
        let mut m = Model::new();
        m.add_rectangle([0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]).unwrap();
        m.undo();
        m.add_rectangle([5.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]).unwrap();
        assert!(!m.can_redo());
    }

    #[test]
    fn lines_stay_until_they_close_a_face() {
        let mut m = Model::new();
        assert_eq!(m.add_line([0.0, 0.0, 0.0], [3.0, 0.0, 0.0]).unwrap(), None);
        assert_eq!(m.add_line([3.0, 0.0, 0.0], [3.0, 2.0, 0.0]).unwrap(), None);
        assert_eq!(m.add_line([3.0, 2.0, 0.0], [0.0, 2.0, 0.0]).unwrap(), None);
        assert_eq!(m.mesh().lines.len(), 3, "three loose lines, no face yet");
        assert_eq!(m.mesh().vertex_count(), 4, "corners are shared");
        let face = m.add_line([0.0, 2.0, 0.0], [0.0, 0.0, 0.0]).unwrap().expect("the fourth line closes the face");
        assert!(m.mesh().lines.is_empty(), "the lines became the face's edges");
        assert_eq!(m.mesh().face_normal(face), [0.0, 0.0, 1.0], "a floor faces up");
        m.push_pull(face, 1.0).unwrap();
        assert!((m.mesh().signed_volume() - 6.0).abs() < 1e-4);
        // each line was its own step
        m.undo();
        m.undo();
        assert_eq!(m.mesh().lines.len(), 3);
    }

    #[test]
    fn a_wall_from_lines_and_a_crooked_loop() {
        let mut m = Model::new();
        let pts = [[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [2.0, 0.0, 3.0], [0.0, 0.0, 3.0]];
        let mut face = None;
        for i in 0..4 {
            face = m.add_line(pts[i], pts[(i + 1) % 4]).unwrap();
        }
        let n = m.mesh().face_normal(face.unwrap());
        assert!((n[1] + 1.0).abs() < 1e-6, "an upright face looks to the front: {n:?}");

        // four lines that do not lie in one plane stay lines
        let mut m = Model::new();
        let pts = [[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [2.0, 2.0, 1.0], [0.0, 2.0, 0.0]];
        for i in 0..4 {
            assert_eq!(m.add_line(pts[i], pts[(i + 1) % 4]).unwrap(), None);
        }
        assert_eq!(m.mesh().lines.len(), 4);
        assert_eq!(m.mesh().face_count(), 0);
        // the same line twice is one line
        m.add_line([0.0, 0.0, 0.0], [2.0, 0.0, 0.0]).unwrap();
        assert_eq!(m.mesh().lines.len(), 4);
    }

    #[test]
    fn erasing_a_face_leaves_its_edges_and_lines_survive_edits() {
        let mut m = Model::new();
        let f = m.add_rectangle([0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]).unwrap();
        m.erase_face(f).unwrap();
        assert_eq!(m.mesh().face_count(), 0);
        assert_eq!(m.mesh().lines.len(), 4, "SketchUp keeps the edges");
        // a box elsewhere, pushed (which merges coplanar faces): the lines stay
        let b = m.add_rectangle([5.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]).unwrap();
        m.push_pull(b, 1.0).unwrap();
        assert_eq!(m.mesh().lines.len(), 4);
        let mesh = m.mesh().to_mesh();
        assert!(mesh.lines.iter().all(|l| l.iter().all(|&v| mesh.positions[v as usize][0] < 2.0)), "still the old square");
        m.erase_line(0).unwrap();
        assert_eq!(m.mesh().lines.len(), 3);
    }

    #[test]
    fn a_line_across_a_face_splits_it() {
        let mut m = Model::new();
        m.add_rectangle([0.0; 3], [2.0, 0.0, 0.0], [0.0, 2.0, 0.0]).unwrap();
        // from the middle of one side to the middle of the other: two faces, the sides get a corner each
        m.add_line([1.0, 0.0, 0.0], [1.0, 2.0, 0.0]).unwrap();
        assert_eq!(m.mesh().face_count(), 2);
        assert!(m.mesh().lines.is_empty());
        assert_eq!(m.mesh().vertex_count(), 6);
        // corner to corner: a triangle splits off
        let mut m = Model::new();
        m.add_rectangle([0.0; 3], [2.0, 0.0, 0.0], [0.0, 2.0, 0.0]).unwrap();
        m.add_line([0.0, 0.0, 0.0], [2.0, 2.0, 0.0]).unwrap();
        assert_eq!(m.mesh().face_count(), 2);
        assert!((0..2).all(|f| m.mesh().face_vertices(f).len() == 3));
        // one half pulls up on its own
        let half = (0..2).find(|&f| m.mesh().face_normal(f)[2] > 0.9).unwrap();
        m.push_pull(half, 1.0).unwrap();
        assert!((m.mesh().signed_volume() - 2.0).abs() < 1e-4, "a triangular prism of 2 x 2 / 2 x 1");
    }

    #[test]
    fn crossing_lines_meet() {
        let mut m = Model::new();
        m.add_line([0.0, 1.0, 0.0], [2.0, 1.0, 0.0]).unwrap();
        m.add_line([1.0, 0.0, 0.0], [1.0, 2.0, 0.0]).unwrap();
        assert_eq!(m.mesh().lines.len(), 4, "both lines broke where they cross");
        assert_eq!(m.mesh().vertex_count(), 5);
        // a line ending on another's middle breaks it too
        m.add_line([0.0, 1.0, 0.0], [0.0, 3.0, 0.0]).unwrap();
        m.add_line([-1.0, 2.0, 0.0], [0.0, 2.0, 0.0]).unwrap();
        assert_eq!(m.mesh().lines.len(), 7);
    }

    #[test]
    fn lines_close_a_face_against_a_face_edge() {
        let mut m = Model::new();
        m.add_rectangle([0.0; 3], [2.0, 0.0, 0.0], [0.0, 2.0, 0.0]).unwrap();
        m.add_line([2.0, 0.0, 0.0], [3.0, 0.0, 0.0]).unwrap();
        m.add_line([3.0, 0.0, 0.0], [3.0, 2.0, 0.0]).unwrap();
        let f = m.add_line([3.0, 2.0, 0.0], [2.0, 2.0, 0.0]).unwrap();
        assert!(f.is_some(), "the square's side closes the new face");
        assert_eq!(m.mesh().face_count(), 2);
        assert!(m.mesh().lines.is_empty());
        // the two faces share the side: pulled together they make one closed box
        let up = m.mesh().to_mesh();
        assert_eq!(up.edges().len(), 7);
    }

    #[test]
    fn a_square_drawn_inside_a_face_becomes_a_hole_that_pushes_in() {
        let mut m = Model::new();
        m.add_rectangle([0.0; 3], [4.0, 0.0, 0.0], [0.0, 4.0, 0.0]).unwrap();
        let top = m.push_pull(0, 2.0).unwrap();
        assert_eq!(m.mesh().face_normal(top), [0.0, 0.0, 1.0]);
        let sq = [[1.0, 1.0, 2.0], [3.0, 1.0, 2.0], [3.0, 3.0, 2.0], [1.0, 3.0, 2.0]];
        let mut inner = None;
        for i in 0..4 {
            inner = m.add_line(sq[i], sq[(i + 1) % 4]).unwrap();
        }
        let inner = inner.expect("the square closed into a face");
        assert!(m.mesh().lines.is_empty());
        assert_eq!(m.mesh().face_vertices(inner).len(), 4);
        assert!((m.mesh().face_normal(inner)[2] - 1.0).abs() < 1e-6, "faces up like the top around it");
        assert!(m.mesh().is_closed(), "still a closed solid");
        assert!((m.mesh().signed_volume() - 32.0).abs() < 1e-3);
        // a true hole in the top: no seams, every edge drawn
        let mesh = m.mesh().to_mesh();
        assert!(mesh.seams.is_empty());
        assert_eq!(mesh.all_edges().len(), mesh.visible_edges().len());
        assert_eq!(mesh.holes.iter().map(Vec::len).sum::<usize>(), 1, "the top has one hole");
        assert_eq!(m.mesh().euler_characteristic(), 2);
        // pushed in by 1: a pocket of 2 x 2 x 1
        m.push_pull(inner, -1.0).unwrap();
        assert!(m.mesh().is_closed());
        assert!((m.mesh().signed_volume() - 28.0).abs() < 1e-3, "{}", m.mesh().signed_volume());
    }

    #[test]
    fn follow_me_sweeps_a_profile_with_a_hole_into_a_tube() {
        let mut m = Model::new();
        // a 2 x 2 square standing in the x = 0 plane, a 1 x 1 hole in it
        m.add_rectangle([0.0; 3], [0.0, 2.0, 0.0], [0.0, 0.0, 2.0]).unwrap();
        let inner = m.add_outline(&[[0.0, 0.5, 0.5], [0.0, 1.5, 0.5], [0.0, 1.5, 1.5], [0.0, 0.5, 1.5]]).unwrap().expect("a hole");
        m.erase_face(inner).unwrap();
        let ring = (0..m.mesh().face_count()).find(|&f| m.mesh().has_holes(f)).expect("the ring");
        m.follow_me(ring, &[[0.0, 1.0, 1.0], [3.0, 1.0, 1.0]], false).unwrap();
        assert!(m.mesh().is_closed(), "a closed tube");
        assert!((m.mesh().signed_volume().abs() - 9.0).abs() < 1e-9, "{}", m.mesh().signed_volume());
        assert_eq!(m.mesh().euler_characteristic(), 0, "a tube: one hole right through");
    }

    #[test]
    fn a_window_component_takes_its_opening_along() {
        let mut m = Model::new();
        // a 6 x 3 wall standing in the x-z plane
        m.add_rectangle([0.0; 3], [6.0, 0.0, 0.0], [0.0, 0.0, 3.0]).unwrap();
        // a window drawn on it (a hole and the pane), the pane pushed out 0.1 into a frame
        let pane = m.add_outline(&[[1.0, 0.0, 1.0], [2.0, 0.0, 1.0], [2.0, 0.0, 2.0], [1.0, 0.0, 2.0]]).unwrap().unwrap();
        m.push_pull(pane, 0.1).unwrap();
        let wall = (0..m.mesh().face_count()).find(|&f| m.mesh().has_holes(f)).unwrap();
        let window: Vec<usize> = (0..m.mesh().face_count()).filter(|&f| f != wall).collect();
        let inst = m.make_group(&window, &[], true).unwrap();
        let def = m.instances()[inst].def;
        m.set_glue_cut(def, true, true).unwrap();
        assert!(m.definition(def).unwrap().glue);
        let hole_x = |m: &Model| -> Vec<f64> {
            let mesh = m.mesh().to_mesh();
            let mut xs: Vec<f64> = mesh.holes.iter().flatten().flatten().map(|&v| mesh.positions[v as usize][0]).collect();
            xs.sort_by(f64::total_cmp);
            xs.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
            xs
        };
        assert_eq!(hole_x(&m), vec![1.0, 2.0], "the opening where it was drawn");
        // moved 2 m along the wall: the opening goes with it
        m.transform_all(&[], &[inst], &Xform::translation([2.0, 0.0, 0.0])).unwrap();
        assert_eq!(hole_x(&m), vec![3.0, 4.0]);
        assert_eq!(m.mesh().to_mesh().holes.iter().map(Vec::len).sum::<usize>(), 1, "one opening, not two");
        // erased: the wall is whole again
        m.erase_instances(&[inst]).unwrap();
        assert!(m.mesh().to_mesh().holes.iter().all(|h| h.is_empty()));
    }

    #[test]
    fn solid_tools_on_two_box_groups() {
        // two 2 m cubes as groups, overlapping by 1 m along x
        let two_boxes = || {
            let mut m = Model::new();
            m.add_rectangle([0.0; 3], [2.0, 0.0, 0.0], [0.0, 2.0, 0.0]).unwrap();
            m.push_pull(0, 2.0).unwrap();
            let all: Vec<usize> = (0..m.mesh().face_count()).collect();
            let a = m.make_group(&all, &[], false).unwrap();
            let f = m.add_rectangle([1.0, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0, 2.0, 0.0]).unwrap();
            m.push_pull(f, 2.0).unwrap();
            let all: Vec<usize> = (0..m.mesh().face_count()).collect();
            let b = m.make_group(&all, &[], false).unwrap();
            (m, a, b)
        };
        let volume = |m: &Model, i: usize| {
            let mut mesh = Mesh::new();
            m.scene.flatten_instance(&m.instances()[i], &Xform::IDENTITY, &mut mesh);
            HalfEdgeMesh::from_mesh(&mesh).unwrap().signed_volume()
        };
        let (mut m, a, b) = two_boxes();
        // the first clicked is taken away from the second
        let made = m.solid_tool(a, b, SolidTool::Subtract).unwrap();
        assert_eq!(m.instances().len(), 1);
        assert!((volume(&m, made[0]) - 4.0).abs() < 1e-9);
        assert!(m.undo());
        assert_eq!(m.instances().len(), 2, "one undo step");
        let (mut m, a, b) = two_boxes();
        let made = m.solid_tool(a, b, SolidTool::Split).unwrap();
        let mut v: Vec<f64> = made.iter().map(|&i| volume(&m, i)).collect();
        v.sort_by(f64::total_cmp);
        assert!(v.iter().zip([4.0, 4.0, 4.0]).all(|(x, y)| (x - y).abs() < 1e-9), "{v:?}");
        // loose geometry is no solid group
        let (mut m, a, _) = two_boxes();
        assert!(m.solid_tool(a, a, SolidTool::Union).is_err());
    }

    #[test]
    fn sandbox_ground_stamped_and_draped() {
        let mut m = Model::new();
        let t = m.sandbox_from_scratch([0.0; 3], [10.0, 10.0, 0.0], 1.0).unwrap();
        // a 2 x 2 x 1 box group standing 1 m up, over the middle
        let f = m.add_rectangle([4.3, 4.3, 1.0], [2.0, 0.0, 0.0], [0.0, 2.0, 0.0]).unwrap();
        m.push_pull(f, 1.0).unwrap();
        let all: Vec<usize> = (0..m.mesh().face_count()).collect();
        let o = m.make_group(&all, &[], false).unwrap();
        let ground = |m: &Model| {
            let mut g = Mesh::new();
            m.scene.flatten_instance(&m.instances()[t], &Xform::IDENTITY, &mut g);
            g
        };
        m.sandbox_onto(t, o, false, 1.0).unwrap();
        let g = ground(&m);
        assert!((crate::sandbox::height_at(&g, [5.3, 5.3]).unwrap() - 1.0).abs() < 1e-9, "flat at the box foot");
        assert_eq!(crate::sandbox::height_at(&g, [0.5, 0.5]), Some(0.0), "untouched far off");
        let lines_before = ground(&m).lines.len() + ground(&m).faces.len();
        m.sandbox_onto(t, o, true, 0.0).unwrap();
        assert!(ground(&m).lines.len() + ground(&m).faces.len() > lines_before, "the box's outline laid on the ground");
        // one undo takes the drape back, another the stamp
        assert!(m.undo() && m.undo());
        assert_eq!(crate::sandbox::height_at(&ground(&m), [5.0, 5.0]), Some(0.0));
    }

    #[test]
    fn a_dynamic_component_sized_by_its_attributes() {
        use crate::dynamic::Attribute;
        let a = |n: &str, f: &str| Attribute { name: n.into(), formula: f.into(), label: None };
        let mut m = Model::new();
        m.add_rectangle([0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]).unwrap();
        m.push_pull(0, 1.0).unwrap();
        let all: Vec<usize> = (0..m.mesh().face_count()).collect();
        let i = m.make_group(&all, &[], true).unwrap();
        let def = m.instances()[i].def;
        let size = |m: &Model| {
            let mut mesh = Mesh::new();
            m.scene.flatten_instance(&m.instances()[i], &Xform::IDENTITY, &mut mesh);
            let (lo, hi) = mesh.bounds().unwrap();
            ([hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]], lo)
        };
        m.set_attributes(def, vec![a("Width", "2"), a("LenX", "=Width"), a("onClick", "ALTER(\"Width\", 2, 3)")]).unwrap();
        let (s, _) = size(&m);
        assert!((s[0] - 2.0).abs() < 1e-9 && (s[1] - 1.0).abs() < 1e-9, "{s:?}");
        assert!(m.interact(i).unwrap());
        assert!((size(&m).0[0] - 3.0).abs() < 1e-9, "clicked: 3 m");
        let mut attrs = m.attributes(def).to_vec();
        attrs.push(a("X", "5"));
        m.set_attributes(def, attrs).unwrap();
        assert!((size(&m).1[0] - 5.0).abs() < 1e-9, "moved to x = 5");
        // a bad formula changes nothing
        assert!(m.set_attributes(def, vec![a("LenX", "=Nope*2")]).is_err());
        assert!((size(&m).0[0] - 3.0).abs() < 1e-9);
        assert_eq!(m.attribute_values(i).unwrap()["width"].num(), 3.0);
    }

    #[test]
    fn options_changed_on_one_copy_leave_the_other_copies_as_they_were() {
        use crate::dynamic::Attribute;
        let a = |n: &str, f: &str| Attribute { name: n.into(), formula: f.into(), label: Some("Width".into()).filter(|_| n == "Width") };
        let (mut m, i, def) = cube_component();
        m.set_attributes(def, vec![a("Width", "1"), a("LenX", "=Width"), a("onClick", "ALTER(\"Width\", 1, 2)")]).unwrap();
        let j = m.copy_instances(&[i], &Xform::translation([3.0, 0.0, 0.0])).unwrap()[0];
        let width = |m: &Model, k: usize| {
            let mut mesh = Mesh::new();
            m.scene.flatten_instance(&m.instances()[k], &Xform::IDENTITY, &mut mesh);
            let (lo, hi) = mesh.bounds().unwrap();
            hi[0] - lo[0]
        };
        assert_eq!(m.instances()[j].def, def, "copies share their component");
        let defs = m.scene.defs.len();
        // Component Options on the copy: it gets a component of its own, the first keeps 1 m
        m.set_copy_attributes(j, vec![a("Width", "2.5"), a("LenX", "=Width"), a("onClick", "ALTER(\"Width\", 1, 2)")]).unwrap();
        assert!((width(&m, j) - 2.5).abs() < 1e-9 && (width(&m, i) - 1.0).abs() < 1e-9);
        assert_eq!(m.scene.defs.len(), defs + 1);
        let own = m.instances()[j].def;
        assert_ne!(own, def);
        assert_eq!(
            m.scene.defs[own].name,
            format!("{}#2", m.scene.defs[def].name.rsplit_once('#').map(|p| p.0).unwrap_or(&m.scene.defs[def].name))
        );
        // now alone in its component, it changes in place
        m.set_copy_attributes(j, vec![a("Width", "3"), a("LenX", "=Width")]).unwrap();
        assert_eq!((m.instances()[j].def, m.scene.defs.len()), (own, defs + 1));
        // Interact on the first: only it turns to 2 m
        let k = m.copy_instances(&[i], &Xform::translation([0.0, 3.0, 0.0])).unwrap()[0];
        assert!(m.interact(i).unwrap());
        assert!((width(&m, i) - 2.0).abs() < 1e-9 && (width(&m, k) - 1.0).abs() < 1e-9 && (width(&m, j) - 3.0).abs() < 1e-9);
        // each change one undo step, a bad one none
        let before = m.scene.defs.len();
        assert!(m.set_copy_attributes(k, vec![a("LenX", "=Nope")]).is_err());
        assert_eq!(m.scene.defs.len(), before);
        assert!(m.undo());
        assert!((width(&m, i) - 1.0).abs() < 1e-9 && m.instances()[i].def == def);
    }

    /// A 1 m cube made a component; returns the model, its instance and its definition.
    fn cube_component() -> (Model, usize, usize) {
        let mut m = Model::new();
        m.add_rectangle([0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]).unwrap();
        m.push_pull(0, 1.0).unwrap();
        let all: Vec<usize> = (0..m.mesh().face_count()).collect();
        let i = m.make_group(&all, &[], true).unwrap();
        let def = m.instances()[i].def;
        (m, i, def)
    }

    #[test]
    fn hidden_copies_and_material_as_a_dynamic_component_draws_them() {
        use crate::dynamic::Attribute;
        let a = |n: &str, f: &str| Attribute { name: n.into(), formula: f.into(), label: None };
        let (mut m, i, def) = cube_component();
        let drawn = |m: &Model| {
            let mut mesh = Mesh::new();
            m.scene.flatten_instance(&m.instances()[i], &Xform::IDENTITY, &mut mesh);
            mesh
        };
        // Hidden follows its formula
        m.set_attributes(def, vec![a("Width", "1"), a("Hidden", "=Width>2")]).unwrap();
        assert_eq!(drawn(&m).faces.len(), 6);
        m.set_attributes(def, vec![a("Width", "3"), a("Hidden", "=Width>2")]).unwrap();
        assert_eq!(drawn(&m).faces.len(), 0, "hidden");
        assert_eq!(m.scene.flatten().faces.len(), 0, "and not exported either");
        // Copies: two more, each placed by X with COPY = 1, 2
        m.set_attributes(def, vec![a("Copies", "2"), a("X", "=COPY*1.5")]).unwrap();
        let mesh = drawn(&m);
        assert_eq!(mesh.faces.len(), 18, "the copy and two more");
        let (lo, hi) = mesh.bounds().unwrap();
        assert!(lo[0].abs() < 1e-9 && (hi[0] - 4.0).abs() < 1e-9, "at x = 0, 1.5 and 3: {lo:?} {hi:?}");
        assert_eq!(m.instances()[i].xf.origin()[0], 0.0, "the copy itself is where COPY = 0 puts it");
        // Material paints what is unpainted; a painted face keeps its paint
        m.set_attributes(def, vec![a("Material", "red")]).unwrap();
        let mesh = drawn(&m);
        let red = crate::dynamic::parse_color("red").unwrap();
        assert!((0..mesh.faces.len()).all(|f| mesh.color_of(f).map(|p| p.rgb) == Some(red)));
        m.set_attributes(def, vec![a("Material", "#00ff00"), a("Copies", "=-3")]).unwrap();
        let mesh = drawn(&m);
        assert_eq!(mesh.faces.len(), 6, "fewer than none is none");
        assert_eq!(mesh.color_of(0).map(|p| p.rgb), Some([0, 255, 0]));
        // a formula that goes wrong is refused, and the component stays as it was
        assert!(m.set_attributes(def, vec![a("Hidden", "=Nope")]).is_err());
        assert_eq!(drawn(&m).faces.len(), 6);
    }

    #[test]
    fn a_component_inside_another_reads_its_parent() {
        use crate::dynamic::Attribute;
        let a = |n: &str, f: &str| Attribute { name: n.into(), formula: f.into(), label: None };
        let (mut m, inner_i, inner) = cube_component();
        let outer_i = m.make_group(&[], &[inner_i], true).unwrap();
        let outer = m.instances()[outer_i].def;
        m.set_attributes(outer, vec![a("Width", "3")]).unwrap();
        m.open(outer_i).unwrap();
        m.set_attributes(inner, vec![a("LenX", "=Parent!Width"), a("Copies", "=Parent!Width-2"), a("X", "=COPY*4")]).unwrap();
        assert_eq!(m.attribute_values(0).unwrap()["lenx"].num(), 3.0, "the parent's Width");
        m.close();
        let mut mesh = Mesh::new();
        m.scene.flatten_instance(&m.instances()[outer_i], &Xform::IDENTITY, &mut mesh);
        assert_eq!(mesh.faces.len(), 12, "Width 3: one copy more");
        let (lo, hi) = mesh.bounds().unwrap();
        assert!((hi[0] - lo[0] - 7.0).abs() < 1e-9, "3 m wide, the copy 4 m on: {lo:?} {hi:?}");
        assert_eq!(crate::dynamic::parse_color("12, 200, 7"), Some([12, 200, 7]));
        assert_eq!(crate::dynamic::parse_color("rgb(1,2,3)"), Some([1, 2, 3]));
        assert_eq!(crate::dynamic::parse_color("#abc"), Some([170, 187, 204]));
        assert_eq!(crate::dynamic::parse_color("no colour"), None);
    }

    #[test]
    fn paint_stays_on_its_face_through_edits() {
        let red = Some(crate::Paint::from([200, 40, 30]));
        let mut m = Model::new();
        let f = m.add_rectangle([0.0; 3], [2.0, 0.0, 0.0], [0.0, 2.0, 0.0]).unwrap();
        m.paint(&[f], red).unwrap();
        // a line across splits it: both halves red
        m.add_line([1.0, 0.0, 0.0], [1.0, 2.0, 0.0]).unwrap();
        let mesh = m.mesh().to_mesh();
        assert_eq!((0..2).filter(|&i| mesh.color_of(i) == red).count(), 2);
        // pushed up: the top stays red, the new walls are unpainted
        let top = m.push_pull(0, 1.0).unwrap();
        let mesh = m.mesh().to_mesh();
        assert_eq!(mesh.color_of(top), red);
        assert!((0..mesh.faces.len()).any(|i| mesh.color_of(i).is_none()));
        // undo takes the paint back too
        m.undo();
        m.undo();
        m.undo();
        assert_eq!(m.mesh().to_mesh().color_of(0), None);
    }

    #[test]
    fn follow_me_along_a_line_and_round_a_corner() {
        // a 1 x 1 square standing in the YZ plane at the origin, swept 3 m along +X: a 3 m bar
        let mut m = Model::new();
        let f = m.add_rectangle([0.0; 3], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]).unwrap();
        m.follow_me(f, &[[0.0; 3], [3.0, 0.0, 0.0]], false).unwrap();
        assert!(m.mesh().is_closed());
        assert!((m.mesh().signed_volume() - 3.0).abs() < 1e-4, "{}", m.mesh().signed_volume());
        // with a right-angle bend: 3 m out, then 2 m along +Y, mitred at the corner
        let mut m = Model::new();
        let f = m.add_rectangle([0.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]).unwrap();
        m.follow_me(f, &[[0.0, 0.5, 0.0], [3.0, 0.5, 0.0], [3.0, 2.5, 0.0]], false).unwrap();
        assert!(m.mesh().is_closed());
        assert!((m.mesh().signed_volume() - 5.0).abs() < 1e-3, "{}", m.mesh().signed_volume());
    }

    #[test]
    fn follow_me_round_a_closed_path_makes_a_ring() {
        // a small square profile at the edge of a square path: a square frame with no caps
        let mut m = Model::new();
        let f = m.add_rectangle([0.0, -0.5, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]).unwrap();
        let path = [[0.0, 0.0, 0.0], [4.0, 0.0, 0.0], [4.0, 4.0, 0.0], [0.0, 4.0, 0.0]];
        m.follow_me(f, &path, true).unwrap();
        assert!(m.mesh().is_closed());
        assert_eq!(m.mesh().euler_characteristic(), 0, "a ring (a torus)");
        assert!((m.mesh().signed_volume().abs() - 16.0).abs() < 1e-3, "{}", m.mesh().signed_volume());
    }

    /// A 2 x 1 x 1 box at (x, 0, 0), every face of it.
    fn box_at(m: &mut Model, x: f64) -> Vec<usize> {
        let before = m.mesh().face_count();
        let f = m.add_rectangle([x, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0, 1.0, 0.0]).unwrap();
        m.push_pull(f, 1.0).unwrap();
        (before..m.mesh().face_count()).collect()
    }

    fn volume(mesh: &Mesh) -> f64 {
        HalfEdgeMesh::from_mesh(mesh).unwrap().signed_volume()
    }

    #[test]
    fn a_group_keeps_its_box_apart_and_moves_whole() {
        let mut m = Model::new();
        let faces = box_at(&mut m, 0.0);
        let g = m.make_group(&faces, &[], false).unwrap();
        assert_eq!(m.mesh().face_count(), 0, "the faces went into the group");
        assert_eq!(m.instances().len(), 1);
        assert_eq!(m.instance_world(g).unwrap().origin(), [0.0, 0.0, 0.0]);
        let flat = m.flatten();
        assert!((volume(&flat) - 2.0).abs() < 1e-4);
        m.transform_instances(&[g], &Xform::translation([5.0, 0.0, 0.0])).unwrap();
        let (lo, hi) = m.flatten().bounds().unwrap();
        assert_eq!((lo[0], hi[0]), (5.0, 7.0));
        m.undo();
        m.undo();
        assert_eq!(m.mesh().face_count(), 6, "undo brings the loose box back");
        assert!(m.instances().is_empty());
    }

    #[test]
    fn components_share_an_edit_and_groups_do_not() {
        for component in [true, false] {
            let mut m = Model::new();
            let faces = box_at(&mut m, 0.0);
            let c = m.make_group(&faces, &[], component).unwrap();
            let copy = m.copy_instances(&[c], &Xform::translation([3.0, 0.0, 0.0])).unwrap()[0];
            assert_ne!(c, copy);
            // open the first, pull its top up by 1, close it
            m.open(c).unwrap();
            assert_eq!(m.open_depth(), 1);
            let top = (0..m.mesh().face_count()).find(|&f| m.mesh().face_normal(f)[2] > 0.9).unwrap();
            m.push_pull(top, 1.0).unwrap();
            assert!((m.flatten().bounds().unwrap().1[2] - 2.0).abs() < 1e-5);
            m.close();
            assert_eq!(m.open_depth(), 0);
            let total = volume(&{
                let mut one = Mesh::new();
                for (_, part) in m.parts().0 {
                    one.append(&part);
                }
                one
            });
            let want = if component { 8.0 } else { 6.0 };
            assert!((total - want).abs() < 1e-3, "component {component}: volume {total}");
        }
    }

    #[test]
    fn a_group_inside_a_group_and_explode() {
        let mut m = Model::new();
        let a = box_at(&mut m, 0.0);
        let ga = m.make_group(&a, &[], false).unwrap();
        let b = box_at(&mut m, 4.0);
        // the second box and the first group together become one outer group
        let outer = m.make_group(&b, &[ga], false).unwrap();
        assert_eq!(m.instances().len(), 1);
        assert_eq!(m.definition(m.instances()[outer].def).unwrap().ctx.instances.len(), 1);
        assert!((volume(&m.flatten()) - 4.0).abs() < 1e-3);
        // inside the outer group: the inner one is an instance there, the outer's own box is the mesh
        m.open(outer).unwrap();
        assert_eq!(m.mesh().face_count(), 6);
        assert_eq!(m.instances().len(), 1);
        assert_eq!(m.open_names(), vec!["Group#2".to_string()]);
        let (lo, _) = m.mesh().to_mesh().bounds().unwrap();
        assert_eq!(lo[0], 4.0, "the inside is shown where it stands");
        m.close();
        m.explode(outer).unwrap();
        assert_eq!(m.mesh().face_count(), 6, "the outer box is loose again");
        assert_eq!(m.instances().len(), 1, "and the inner group is at the top");
        assert!((volume(&m.flatten()) - 4.0).abs() < 1e-3);
        assert_eq!(m.document().defs.len(), 1, "the exploded definition is gone");
    }

    #[test]
    fn an_edit_inside_a_group_undoes_back_inside() {
        let mut m = Model::new();
        let faces = box_at(&mut m, 0.0);
        let g = m.make_group(&faces, &[], false).unwrap();
        m.transform_instances(&[g], &Xform::translation([0.0, 0.0, 2.0])).unwrap();
        m.open(g).unwrap();
        let (lo, _) = m.mesh().to_mesh().bounds().unwrap();
        assert!((lo[2] - 2.0).abs() < 1e-6);
        m.add_rectangle([0.0, 0.0, 2.0], [0.5, 0.0, 0.0], [0.0, -0.5, 0.0]).unwrap();
        m.close();
        m.undo();
        assert_eq!(m.open_depth(), 1, "undo goes back to where the edit was made");
        assert_eq!(m.mesh().face_count(), 6);
        m.close();
        assert!(m.copy_faces(&[], &Xform::IDENTITY).is_ok());
    }

    #[test]
    fn copying_loose_faces_with_move() {
        let mut m = Model::new();
        let f = m.add_rectangle([0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]).unwrap();
        m.copy_faces(&[f], &Xform::translation([1.0, 0.0, 0.0])).unwrap();
        assert_eq!(m.mesh().face_count(), 2);
        assert_eq!(m.mesh().vertex_count(), 6, "the copy shares the edge it touches");
    }

    #[test]
    fn a_part_takes_the_face_it_is_made_from_and_changes_as_one_step() {
        let mut m = Model::new();
        let f = m.add_rectangle([0.0, 0.0, 0.0], [0.1, 0.0, 0.0], [0.0, 0.05, 0.0]).unwrap();
        let faces_before = m.mesh().to_mesh().faces.len();
        let mut part_faces = Mesh::new();
        for p in [[0.0, 0.0, 0.0], [0.1, 0.0, 0.0], [0.1, 0.05, 0.0]] {
            part_faces.add_vertex(p);
        }
        part_faces.add_face(vec![0, 1, 2]);
        let place = Xform::translation([1.0, 2.0, 0.0]);
        let i = m.place_part("Sheet Part", "{part}".into(), &part_faces, &place, Some(f)).unwrap();
        assert_eq!(m.mesh().to_mesh().faces.len(), faces_before - 1, "the face went into the part");
        let inst = m.instances()[i];
        assert_eq!(inst.xf, place);
        let d = m.definition(inst.def).unwrap();
        assert_eq!((d.name.as_str(), d.part.as_deref(), d.component), ("Sheet Part#1", Some("{part}"), true));
        // a new description and faces, then undo brings the old ones back
        let mut bigger = part_faces.clone();
        bigger.positions[1] = [0.2, 0.0, 0.0];
        m.set_part(inst.def, "{part 2}".into(), bigger.clone()).unwrap();
        assert_eq!(m.definition(inst.def).unwrap().ctx.mesh, bigger);
        m.undo();
        assert_eq!(m.definition(inst.def).unwrap().part.as_deref(), Some("{part}"));
        m.undo();
        assert_eq!(m.mesh().to_mesh().faces.len(), faces_before);
        assert!(m.instances().is_empty());
        // no such definition
        assert!(m.set_part(99, "x".into(), Mesh::new()).is_err());
        // a group becomes a part (and a component)
        let f = m.add_rectangle([2.0, 0.0, 0.0], [0.1, 0.0, 0.0], [0.0, 0.1, 0.0]).unwrap();
        let g = m.make_group(&[f], &[], false).unwrap();
        let def = m.instances()[g].def;
        m.set_part(def, "{converted}".into(), part_faces.clone()).unwrap();
        let d = m.definition(def).unwrap();
        assert!(d.component && d.part.as_deref() == Some("{converted}") && d.ctx.mesh == part_faces);
    }

    #[test]
    fn the_revision_changes_with_every_change_and_never_comes_back() {
        let (mut m, i, _) = cube_component();
        let mut seen = vec![Model::new().revision()];
        let mut fresh = |m: &Model| {
            assert!(!seen.contains(&m.revision()), "a stamp given out again");
            seen.push(m.revision());
        };
        fresh(&m);
        m.transform_instances(&[i], &Xform::translation([2.0, 0.0, 0.0])).unwrap();
        fresh(&m);
        assert!(m.push_pull(99, 1.0).is_err());
        fresh(&m);
        // undo and redo bring the state back, not the stamp
        m.undo();
        fresh(&m);
        m.redo();
        fresh(&m);
        m.open(i).unwrap();
        fresh(&m);
        m.close();
        fresh(&m);
        // looking leaves it be
        let r = m.revision();
        let _ = (m.parts(), m.document(), m.flatten(), m.outline());
        m.with_part_pieces(|_, _| ());
        assert!(!m.close() && !m.redo());
        assert_eq!(m.revision(), r);
        // a copy keeps it until either changes; another model has its own
        let mut copy = m.clone();
        assert_eq!(copy.revision(), r);
        copy.undo();
        assert_ne!(copy.revision(), r);
        assert_eq!(m.revision(), r);
        fresh(&Model::from_mesh(m.mesh().clone()));
    }

    #[test]
    fn the_parts_revision_stays_while_only_the_top_levels_loose_geometry_changes() {
        let (mut m, i, _) = cube_component();
        let parts = |m: &Model| m.parts_revision();
        let r = parts(&m);
        // drawing on the ground, pulling it up, an edit that fails: no part changes
        let f = m.add_rectangle([3.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]).unwrap();
        m.push_pull(f, 1.0).unwrap();
        assert!(m.erase_face(99).is_err());
        assert_eq!(parts(&m), r);
        assert_ne!(m.revision(), r);
        // a group moved, an undo, a group opened: they may
        m.transform_instances(&[i], &Xform::translation([0.0, 2.0, 0.0])).unwrap();
        let moved = parts(&m);
        assert_ne!(moved, r);
        m.undo();
        let undone = parts(&m);
        assert!(undone != moved && undone != r);
        m.open(i).unwrap();
        let opened = parts(&m);
        assert!(opened != undone);
        // inside a component every edit counts: its other copies show it
        m.add_rectangle([0.2, 0.2, 1.0], [0.1, 0.0, 0.0], [0.0, 0.1, 0.0]).unwrap();
        assert_ne!(parts(&m), opened);
        assert_eq!(parts(&m), m.revision());
        m.close();
        assert_eq!(parts(&m), m.revision());
    }

    #[test]
    fn the_parts_pieces_add_up_to_the_parts() {
        // a component placed twice, one copy open and changed (the other, outside, shows the change), nested in a group;
        // a mirrored copy; a dynamic component drawing copies of itself
        use crate::dynamic::Attribute;
        let (mut m, i, def) = cube_component();
        m.set_attributes(
            def,
            vec![
                Attribute { name: "Copies".into(), formula: "2".into(), label: None },
                Attribute { name: "X".into(), formula: "=COPY*1.5".into(), label: None },
            ],
        )
        .unwrap();
        let copies = m.copy_instances(&[i], &Xform::from_fn(|p| [-p[0] - 3.0, p[1], p[2]])).unwrap();
        let g = m.make_group(&[], &[i], false).unwrap();
        m.add_rectangle([0.0, -5.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]).unwrap();
        let joined = |pieces: &[Piece]| {
            let mut mesh = Mesh::new();
            for p in pieces {
                mesh.append(&p.placed());
            }
            mesh
        };
        let check = |m: &mut Model| {
            let (inside, outside) = m.parts();
            let (pieces_inside, pieces_outside) =
                m.with_part_pieces(|inside, outside| (inside.iter().map(|(k, ps)| (*k, joined(ps))).collect::<Vec<_>>(), joined(&outside)));
            assert_eq!(pieces_inside, inside);
            assert_eq!(pieces_outside, outside);
            let doc = m.document();
            let (a, b) = doc.part_pieces(m.open_path());
            assert_eq!((a.iter().map(|(k, ps)| (*k, joined(ps))).collect::<Vec<_>>(), joined(&b)), (inside, outside));
        };
        check(&mut m);
        m.open(g).unwrap();
        check(&mut m);
        m.open(0).unwrap();
        check(&mut m);
        let top = (0..m.mesh().face_count()).find(|&f| m.mesh().face_normal(f)[2] > 0.9).unwrap();
        m.push_pull(top, 0.5).unwrap();
        check(&mut m);
        assert!(m.parts().1.faces.len() > 6 * 3, "the copies outside are drawn too");
        m.close_all();
        check(&mut m);
        assert_eq!(copies.len(), 1);
    }
}
