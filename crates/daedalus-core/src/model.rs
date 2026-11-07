use crate::scene::{Context, Definition, Instance, Notes, Scene, Xform};
use crate::{HalfEdgeMesh, Mesh, TopologyError};

/// Within this distance (model units, metres) two faces' planes count as one when coplanar faces are merged.

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
        Model { mesh, ..Default::default() }
    }

    /// A whole scene (an opened file): its top level must be editable.
    pub fn from_scene(mut scene: Scene) -> Result<Self, TopologyError> {
        let mesh = HalfEdgeMesh::from_mesh(&scene.root.mesh)?;
        scene.root.mesh = Mesh::new();
        Ok(Model { mesh, scene, ..Default::default() })
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
        now
    }

    fn edit<R>(
        &mut self,
        f: impl FnOnce(&mut HalfEdgeMesh) -> Result<R, TopologyError>,
    ) -> Result<R, TopologyError> {
        self.edit_scene(|m, _, _| f(m))
    }

    /// An edit that may change the scene too (groups, notes): one undo step, undone whole when it fails.
    fn edit_scene<R>(
        &mut self,
        f: impl FnOnce(&mut HalfEdgeMesh, &mut Scene, &[usize]) -> Result<R, TopologyError>,
    ) -> Result<R, TopologyError> {
        let before = self.snapshot();
        let open = self.open.clone();
        match f(&mut self.mesh, &mut self.scene, &open) {
            Ok(r) => {
                self.undo.push((before, self.state));
                self.redo.clear();
                self.next_state += 1;
                self.state = self.next_state;
                Ok(r)
            }
            Err(e) => {
                self.restore(before);
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
            let inner: Vec<Instance> = taken.iter().map(|&i| Instance { def: ctx.instances[i].def, xf: un.after(&ctx.instances[i].xf) }).collect();
            let name = s.new_name(component);
            s.defs.push(Definition { name, component, ctx: Context { mesh: part.transformed(&to_def), instances: inner } });
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
            s.defs.push(Definition { name, component, ctx: Context { mesh: mesh.transformed(&to_def), instances: Vec::new() } });
            let def = s.defs.len() - 1;
            let ctx = s.ctx_mut(open).ok_or_else(nothing)?;
            ctx.instances.push(Instance { def, xf });
            Ok(ctx.instances.len() - 1)
        })
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
            for &v in vertices {
                if let Some(p) = m.positions.get_mut(v as usize) {
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
                    let ctx = s.ctx_mut(open).ok_or_else(nothing)?;
                    ctx.instances.push(Instance { def, xf: local.after(&inst.xf) });
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
        self.edit_scene(|_, s, open| {
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

    /// Paint Bucket: paints `faces` with `color` (None: back to the default material), as one undo step.
    pub fn paint(&mut self, faces: &[usize], color: Option<[u8; 3]>) -> Result<(), TopologyError> {
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
                if l > 0.0 { a.map(|c| c / l) } else { a }
            };
            // the directions along the path, one per piece (a closed path has one more, back to the start)
            let pieces = if closed { n } else { n - 1 };
            let dirs: Vec<[f64; 3]> = (0..pieces).map(|i| unit(sub(path[(i + 1) % n], path[i]))).collect();
            if dirs.iter().any(|d| dot(*d, *d) < 0.5) {
                return Err(TopologyError::DegenerateFace(profile));
            }
            let color = mesh.color_of(profile);
            let mut ring = mesh.remove_face(profile);
            // the profile wound counter-clockwise about the way the path sets off
            let pts: Vec<[f64; 3]> = ring.iter().map(|&v| mesh.positions[v as usize]).collect();
            let mut nrm = [0.0f64; 3];
            for i in 0..pts.len() {
                let (p, q) = (pts[i], pts[(i + 1) % pts.len()]);
                nrm[0] += (p[1] - q[1]) * (p[2] + q[2]);
                nrm[1] += (p[2] - q[2]) * (p[0] + q[0]);
                nrm[2] += (p[0] - q[0]) * (p[1] + q[1]);
            }
            if dot(nrm, dirs[0]) < 0.0 {
                ring.reverse();
            }
            // a closed path has a corner at its start too: the profile is carried onto that corner's mitre plane first
            if closed {
                let d0 = dirs[0];
                let last = dirs[pieces - 1];
                let mitre = unit([last[0] + d0[0], last[1] + d0[1], last[2] + d0[2]]);
                let denom = dot(d0, mitre);
                if denom.abs() < 1e-6 {
                    return Err(TopologyError::DegenerateNormal);
                }
                ring = ring
                    .iter()
                    .map(|&v| {
                        let p = mesh.positions[v as usize];
                        let t = dot(sub(path[0], p), mitre) / denom;
                        mesh.add_vertex([p[0] + d0[0] * t, p[1] + d0[1] * t, p[2] + d0[2] * t])
                    })
                    .collect();
            }
            // each next cross-section: the last one carried along the piece onto the mitre plane at the next corner
            let mut sections: Vec<Vec<u32>> = vec![ring.clone()];
            let corners = if closed { n } else { n - 1 };
            for i in 1..=corners {
                let at = path[i % n];
                let d_in = dirs[i - 1];
                let mitre = if !closed && i == n - 1 { d_in } else { unit([d_in[0] + dirs[i % pieces][0], d_in[1] + dirs[i % pieces][1], d_in[2] + dirs[i % pieces][2]]) };
                let denom = dot(d_in, mitre);
                if denom.abs() < 1e-6 {
                    return Err(TopologyError::DegenerateNormal); // a bend that turns right back
                }
                let prev = sections.last().expect("one at least").clone();
                if closed && i == n {
                    break; // the ring closes onto the first section
                }
                let next: Vec<u32> = prev
                    .iter()
                    .map(|&v| {
                        let p = mesh.positions[v as usize];
                        let t = dot(sub(at, p), mitre) / denom;
                        mesh.add_vertex([p[0] + d_in[0] * t, p[1] + d_in[1] * t, p[2] + d_in[2] * t])
                    })
                    .collect();
                sections.push(next);
            }
            if closed {
                // the last corner's section is the profile itself, carried round: join back to it
                let first = sections[0].clone();
                sections.push(first);
            }
            let k = ring.len();
            for w in sections.windows(2) {
                let (s0, s1) = (&w[0], &w[1]);
                for j in 0..k {
                    let (a, b) = (s0[j], s0[(j + 1) % k]);
                    let (a2, b2) = (s1[j], s1[(j + 1) % k]);
                    mesh.faces.push(vec![a, b, b2, a2]);
                }
            }
            if !closed {
                // caps: the start faces back along the path, the end forward
                mesh.faces.push(sections[0].iter().rev().copied().collect());
                mesh.set_color(mesh.faces.len() - 1, color);
                mesh.faces.push(sections.last().expect("two at least").clone());
                mesh.set_color(mesh.faces.len() - 1, color);
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
        // the two seams of the ring around it are not drawn; the square's own edges are
        let mesh = m.mesh().to_mesh();
        assert_eq!(mesh.seams.len(), 2);
        assert_eq!(mesh.all_edges().len() - mesh.visible_edges().len(), 2);
        // pushed in by 1: a pocket of 2 x 2 x 1
        m.push_pull(inner, -1.0).unwrap();
        assert!(m.mesh().is_closed());
        assert!((m.mesh().signed_volume() - 28.0).abs() < 1e-3, "{}", m.mesh().signed_volume());
    }

    #[test]
    fn paint_stays_on_its_face_through_edits() {
        let red = Some([200, 40, 30]);
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
}
