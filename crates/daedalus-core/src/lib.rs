//! Core geometry types for Daedalus CAD.

pub mod csg;
pub mod curve;
pub mod dynamic;
pub mod halfedge;
pub mod imprint;
pub mod inspect;
pub mod merge;
pub mod mesh;
pub mod model;
pub mod opening;
pub mod profile;
pub mod sandbox;
pub mod scene;
pub mod section;
pub mod sheetmetal;
pub mod sketch;
pub mod triangulate;
pub mod units;

pub use halfedge::{HalfEdgeMesh, TopologyError};
pub use mesh::{Mesh, Paint};
pub use model::Model;
pub use sketch::Plane;
