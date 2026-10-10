//! Core geometry types for Daedalus CAD.

pub mod halfedge;
pub mod inspect;
pub mod mesh;
pub mod model;
pub mod sheetmetal;

pub use halfedge::{HalfEdgeMesh, TopologyError};
pub use mesh::Mesh;
pub use model::Model;
