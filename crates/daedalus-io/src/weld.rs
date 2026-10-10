use daedalus_core::Mesh;
use std::collections::HashMap;

/// Welds vertices with bit-identical coordinates.
#[derive(Default)]
pub(crate) struct Welder {
    map: HashMap<[u32; 3], u32>,
}

impl Welder {
    pub(crate) fn vertex(&mut self, mesh: &mut Mesh, p: [f32; 3]) -> u32 {
        let key = [p[0].to_bits(), p[1].to_bits(), p[2].to_bits()];
        *self.map.entry(key).or_insert_with(|| mesh.add_vertex(p))
    }
}
