//! Groups and components, as SketchUp has them: geometry kept apart from the rest, so it does not stick to what it touches,
//! and can be moved, turned and copied as one thing.
//!
//! A [`Definition`] holds geometry (and placed instances of other definitions) in its own coordinates. An [`Instance`] places
//! a definition with a transform. A group is a definition placed once and copied when its instance is copied; a component's
//! copies share their definition, so editing one edits all of them. The model's own loose geometry is the root [`Context`].
//!
//! Notes that are not geometry (guides, dimensions, text, the section plane, the drawing axes) live beside it in [`Notes`].

use crate::Mesh;

/// An affine transform, three rows of a 4x4 matrix: `p' = M * [p, 1]`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Xform(pub [[f64; 4]; 3]);

impl Default for Xform {
    fn default() -> Self {
        Xform::IDENTITY
    }
}

impl Xform {
    pub const IDENTITY: Xform = Xform([[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]]);

    pub fn translation(d: [f64; 3]) -> Xform {
        Xform([[1.0, 0.0, 0.0, d[0]], [0.0, 1.0, 0.0, d[1]], [0.0, 0.0, 1.0, d[2]]])
    }

    /// The transform that sends points where `f` does; `f` must be affine (a move, a turn, a scale or a mix of them).
    pub fn from_fn(f: impl Fn([f64; 3]) -> [f64; 3]) -> Xform {
        let o = f([0.0; 3]);
        let cols = [f([1.0, 0.0, 0.0]), f([0.0, 1.0, 0.0]), f([0.0, 0.0, 1.0])];
        let mut m = [[0.0; 4]; 3];
        for r in 0..3 {
            for c in 0..3 {
                m[r][c] = cols[c][r] - o[r];
            }
            m[r][3] = o[r];
        }
        Xform(m)
    }

    pub fn apply(&self, p: [f64; 3]) -> [f64; 3] {
        let m = &self.0;
        [0, 1, 2].map(|r| m[r][0] * p[0] + m[r][1] * p[1] + m[r][2] * p[2] + m[r][3])
    }

    /// Where the definition's origin lands.
    pub fn origin(&self) -> [f64; 3] {
        [self.0[0][3], self.0[1][3], self.0[2][3]]
    }

    /// `self` after `other`: the transform that does `other` first, then `self`.
    pub fn after(&self, other: &Xform) -> Xform {
        let (a, b) = (&self.0, &other.0);
        let mut m = [[0.0; 4]; 3];
        for r in 0..3 {
            for c in 0..4 {
                m[r][c] = a[r][0] * b[0][c] + a[r][1] * b[1][c] + a[r][2] * b[2][c] + if c == 3 { a[r][3] } else { 0.0 };
            }
        }
        Xform(m)
    }

    pub fn det(&self) -> f64 {
        let m = &self.0;
        m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
    }

    /// The transform that undoes this one, or None when it flattens space (a scale of 0).
    pub fn inverse(&self) -> Option<Xform> {
        let d = self.det();
        if d.abs() < 1e-12 {
            return None;
        }
        let m = &self.0;
        let mut inv = [[0.0f64; 4]; 3];
        // the inverse of the 3x3 part, by cofactors
        inv[0][0] = (m[1][1] * m[2][2] - m[1][2] * m[2][1]) / d;
        inv[0][1] = (m[0][2] * m[2][1] - m[0][1] * m[2][2]) / d;
        inv[0][2] = (m[0][1] * m[1][2] - m[0][2] * m[1][1]) / d;
        inv[1][0] = (m[1][2] * m[2][0] - m[1][0] * m[2][2]) / d;
        inv[1][1] = (m[0][0] * m[2][2] - m[0][2] * m[2][0]) / d;
        inv[1][2] = (m[0][2] * m[1][0] - m[0][0] * m[1][2]) / d;
        inv[2][0] = (m[1][0] * m[2][1] - m[1][1] * m[2][0]) / d;
        inv[2][1] = (m[0][1] * m[2][0] - m[0][0] * m[2][1]) / d;
        inv[2][2] = (m[0][0] * m[1][1] - m[0][1] * m[1][0]) / d;
        // and the translation taken back through it
        for r in 0..3 {
            inv[r][3] = -(inv[r][0] * m[0][3] + inv[r][1] * m[1][3] + inv[r][2] * m[2][3]);
        }
        Some(Xform(inv))
    }
}

/// A placed copy of a definition.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Instance {
    pub def: usize,
    pub xf: Xform,
}

/// Loose geometry and the instances placed among it: the model's top level, or the inside of a definition.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Context {
    pub mesh: Mesh,
    pub instances: Vec<Instance>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Definition {
    pub name: String,
    /// A component (copies share it); otherwise a group (a copy gets its own).
    pub component: bool,
    /// Glue to faces: placed on a face, it stays in that face's plane when moved (SketchUp's "Glue to").
    pub glue: bool,
    /// Cut opening: its footprint (its faces lying flat against a face round it) cuts a hole in that face, as a window does
    /// in a wall.
    pub cut: bool,
    pub ctx: Context,
}

/// A construction guide: drawn dashed, snapped to, never part of the geometry (SketchUp's guide lines and points).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Guide {
    Point([f64; 3]),
    /// Through a point, along a unit direction, endless.
    Line([f64; 3], [f64; 3]),
}

/// A dimension: the two points measured and how far the dimension line stands off from them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Dimension {
    pub a: [f64; 3],
    pub b: [f64; 3],
    pub offset: [f64; 3],
}

/// Text (the Text tool): `text` shown at `at`, with a leader line to `point` when the two differ.
#[derive(Debug, Clone, PartialEq)]
pub struct Label {
    pub point: [f64; 3],
    pub at: [f64; 3],
    pub text: String,
}

/// A section plane: a point on it and the normal of the side it cuts away; `active` while it cuts.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Section {
    pub point: [f64; 3],
    pub normal: [f64; 3],
    pub active: bool,
}

/// The drawing axes (Axes tool): where they meet and their red, green and blue directions (unit length, at right angles).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Axes {
    pub origin: [f64; 3],
    pub dirs: [[f64; 3]; 3],
}

impl Default for Axes {
    fn default() -> Self {
        Axes { origin: [0.0; 3], dirs: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]] }
    }
}

impl Axes {
    pub fn is_default(&self) -> bool {
        *self == Axes::default()
    }
}

/// What a model holds besides geometry.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Notes {
    pub guides: Vec<Guide>,
    pub dims: Vec<Dimension>,
    pub labels: Vec<Label>,
    /// Section planes, in the order they were placed; at most one cuts (`active`), as in SketchUp.
    pub sections: Vec<Section>,
    pub axes: Axes,
}

impl Notes {
    pub fn is_empty(&self) -> bool {
        self.guides.is_empty() && self.dims.is_empty() && self.labels.is_empty() && self.sections.is_empty() && self.axes.is_default()
    }

    /// Which section plane is current: the one cutting, else the last placed.
    pub fn current_section(&self) -> Option<usize> {
        self.sections.iter().position(|s| s.active).or(self.sections.len().checked_sub(1))
    }

    /// The current section plane (see [`Notes::current_section`]).
    pub fn section(&self) -> Option<Section> {
        self.current_section().map(|i| self.sections[i])
    }

    pub fn section_mut(&mut self) -> Option<&mut Section> {
        self.current_section().map(|i| &mut self.sections[i])
    }

    /// The section plane that cuts, if one does.
    pub fn cutting(&self) -> Option<Section> {
        self.sections.iter().copied().find(|s| s.active)
    }

    /// A new section plane, cutting (the others stop).
    pub fn add_section(&mut self, point: [f64; 3], normal: [f64; 3]) {
        for s in &mut self.sections {
            s.active = false;
        }
        self.sections.push(Section { point, normal, active: true });
    }

    /// Section plane `i` cuts and the others not (None: none cuts).
    pub fn activate_section(&mut self, i: Option<usize>) {
        for (k, s) in self.sections.iter_mut().enumerate() {
            s.active = Some(k) == i;
        }
    }
}

/// A whole model: the top level, the definitions placed in it, and the notes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Scene {
    pub root: Context,
    pub defs: Vec<Definition>,
    pub notes: Notes,
}

impl Scene {
    pub fn from_mesh(mesh: Mesh) -> Scene {
        Scene { root: Context { mesh, instances: Vec::new() }, ..Default::default() }
    }

    /// Whether this is only loose geometry (no instances, no notes): it can be written as a plain mesh.
    pub fn is_plain(&self) -> bool {
        self.root.instances.is_empty() && self.notes.is_empty()
    }

    /// The context reached by opening the instances along `path` (indices into each level's instances), from the top.
    pub fn ctx(&self, path: &[usize]) -> Option<&Context> {
        let mut c = &self.root;
        for &i in path {
            c = &self.defs.get(c.instances.get(i)?.def)?.ctx;
        }
        Some(c)
    }

    pub fn ctx_mut(&mut self, path: &[usize]) -> Option<&mut Context> {
        let mut def: Option<usize> = None;
        for &i in path {
            let c: &Context = match def {
                None => &self.root,
                Some(d) => &self.defs.get(d)?.ctx,
            };
            def = Some(c.instances.get(i)?.def);
        }
        match def {
            None => Some(&mut self.root),
            Some(d) => self.defs.get_mut(d).map(|d| &mut d.ctx),
        }
    }

    /// Where the context at `path` sits in the model: its coordinates to the model's.
    pub fn world(&self, path: &[usize]) -> Xform {
        let mut w = Xform::IDENTITY;
        let mut c = &self.root;
        for &i in path {
            let Some(inst) = c.instances.get(i) else { break };
            w = w.after(&inst.xf);
            c = &self.defs[inst.def].ctx;
        }
        w
    }

    /// Adds instance `inst`'s geometry (and everything nested in it), placed by `parent` then its own transform, to `out`.
    pub fn flatten_instance(&self, inst: &Instance, parent: &Xform, out: &mut Mesh) {
        self.flatten_def(inst.def, &parent.after(&inst.xf), out, 0);
    }

    fn flatten_def(&self, def: usize, xf: &Xform, out: &mut Mesh, depth: usize) {
        // a definition cannot hold itself, but a damaged file could say so
        let Some(d) = self.defs.get(def) else { return };
        if depth > 64 {
            return;
        }
        out.append(&d.ctx.mesh.transformed(xf));
        for i in &d.ctx.instances {
            self.flatten_def(i.def, &xf.after(&i.xf), out, depth + 1);
        }
    }

    /// The whole model as one mesh, every instance put in place (for export, and for the solid inspector).
    pub fn flatten(&self) -> Mesh {
        let mut out = self.root.mesh.clone();
        for i in &self.root.instances {
            self.flatten_instance(i, &Xform::IDENTITY, &mut out);
        }
        out
    }

    /// While the context at `path` is open: each of its instances as placed in the model (by index), and everything
    /// outside it (shown faded). The open context's own loose geometry is not included.
    pub fn parts(&self, path: &[usize]) -> (Vec<(usize, Mesh)>, Mesh) {
        let mut outside = Mesh::new();
        let mut c = &self.root;
        let mut w = Xform::IDENTITY;
        for &step in path {
            outside.append(&c.mesh.transformed(&w));
            for (i, inst) in c.instances.iter().enumerate() {
                if i != step {
                    self.flatten_instance(inst, &w, &mut outside);
                }
            }
            let Some(inst) = c.instances.get(step) else { break };
            w = w.after(&inst.xf);
            c = &self.defs[inst.def].ctx;
        }
        let inside = c
            .instances
            .iter()
            .enumerate()
            .map(|(i, inst)| {
                let mut m = Mesh::new();
                self.flatten_instance(inst, &w, &mut m);
                (i, m)
            })
            .collect();
        (inside, outside)
    }

    /// How many instances of `def` are placed anywhere.
    pub fn uses(&self, def: usize) -> usize {
        std::iter::once(&self.root).chain(self.defs.iter().map(|d| &d.ctx)).flat_map(|c| &c.instances).filter(|i| i.def == def).count()
    }

    /// Drops definitions no instance uses any more (after Explode or Delete), renumbering the rest.
    pub fn purge(&mut self) {
        loop {
            let unused: Vec<usize> = (0..self.defs.len()).filter(|&d| self.uses(d) == 0).collect();
            let Some(&d) = unused.last() else { return };
            self.defs.remove(d);
            for c in std::iter::once(&mut self.root).chain(self.defs.iter_mut().map(|d| &mut d.ctx)) {
                for i in &mut c.instances {
                    if i.def > d {
                        i.def -= 1;
                    }
                }
            }
        }
    }

    /// A name for a new definition: "Group#3", "Component#1", one more than the highest of its kind.
    pub fn new_name(&self, component: bool) -> String {
        let stem = if component { "Component#" } else { "Group#" };
        let n = self.defs.iter().filter_map(|d| d.name.strip_prefix(stem)?.parse::<u32>().ok()).max().unwrap_or(0);
        format!("{stem}{}", n + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: [f64; 3], b: [f64; 3]) -> bool {
        (0..3).all(|i| (a[i] - b[i]).abs() < 1e-5)
    }

    #[test]
    fn transforms_compose_and_invert() {
        let turn = Xform::from_fn(|p| [-p[1], p[0], p[2]]); // a quarter turn about blue
        let mv = Xform::translation([1.0, 2.0, 3.0]);
        let both = mv.after(&turn);
        assert!(close(both.apply([1.0, 0.0, 0.0]), [1.0, 3.0, 3.0]));
        let back = both.inverse().unwrap();
        assert!(close(back.apply(both.apply([0.3, -0.7, 2.0])), [0.3, -0.7, 2.0]));
        assert!((turn.det() - 1.0).abs() < 1e-6);
        assert!(Xform::from_fn(|p| [p[0], p[1], 0.0]).inverse().is_none());
    }

    #[test]
    fn a_mirrored_copy_still_faces_out() {
        let mut m = Mesh::new();
        for p in [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
            m.add_vertex(p);
        }
        m.add_face(vec![0, 1, 2]);
        let mirrored = m.transformed(&Xform::from_fn(|p| [-p[0], p[1], p[2]]));
        assert_eq!(mirrored.faces[0], vec![2, 1, 0]);
    }

    #[test]
    fn nested_instances_flatten_into_place() {
        let mut unit = Mesh::new();
        for p in [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
            unit.add_vertex(p);
        }
        unit.add_face(vec![0, 1, 2]);
        let mut s = Scene::default();
        s.defs.push(Definition { name: "Leaf".into(), component: true, glue: false, cut: false, ctx: Context { mesh: unit, instances: vec![] } });
        let inner = Instance { def: 0, xf: Xform::translation([0.0, 0.0, 1.0]) };
        s.defs.push(Definition { name: "Box".into(), component: false, glue: false, cut: false, ctx: Context { mesh: Mesh::new(), instances: vec![inner, inner] } });
        s.root.instances.push(Instance { def: 1, xf: Xform::translation([5.0, 0.0, 0.0]) });
        let flat = s.flatten();
        assert_eq!(flat.faces.len(), 2);
        assert!(close(flat.positions[0], [5.0, 0.0, 1.0]));
        assert_eq!(s.world(&[0]), Xform::translation([5.0, 0.0, 0.0]));
        // opened: the two leaves are its instances, nothing is outside
        let (inside, outside) = s.parts(&[0]);
        assert_eq!(inside.len(), 2);
        assert!(outside.faces.is_empty());
        assert_eq!(s.uses(0), 2);
        s.root.instances.clear();
        s.purge();
        assert!(s.defs.is_empty());
    }
}
