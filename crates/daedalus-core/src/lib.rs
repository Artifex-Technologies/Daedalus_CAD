//! Core geometry types for Daedalus CAD.

pub mod halfedge;
pub mod inspect;
pub mod merge;
pub mod mesh;
pub mod model;
pub mod sheetmetal;
pub mod sketch;
pub mod triangulate;
pub mod units;

pub use halfedge::{HalfEdgeMesh, TopologyError};
pub use mesh::Mesh;
pub use model::Model;
pub use sketch::Plane;
