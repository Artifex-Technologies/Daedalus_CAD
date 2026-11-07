//! The demo models in the README's screenshots, drawn with the same calls the tools make (rectangle, push/pull, lines,
//! outlines that cut holes, paint, groups, text, dimensions, a section plane).
//!
//!     cargo run -p daedalus-io --example demo_models -- OUT_FOLDER
//!
//! writes `house.daedalus` and `house-section.daedalus` (the same house with a section plane cutting through it).

use daedalus_core::model::Model;
use daedalus_core::scene::{Dimension, Label, Section};
use std::path::Path;

const WALL: [u8; 3] = [232, 222, 198];
const ROOF: [u8; 3] = [146, 62, 48];
const GLASS: [u8; 3] = [126, 164, 204];
const DOOR: [u8; 3] = [118, 80, 52];
const WOOD: [u8; 3] = [168, 128, 84];

fn points(m: &Model, f: usize) -> Vec<[f32; 3]> {
    let h = m.mesh();
    h.face_vertices(f).iter().map(|&v| h.positions[v as usize]).collect()
}

/// The first face whose corners and normal pass `pred`.
fn find(m: &Model, pred: impl Fn(&[[f32; 3]], [f32; 3]) -> bool) -> usize {
    (0..m.mesh().face_count()).find(|&f| pred(&points(m, f), m.mesh().face_normal(f))).expect("the face looked for is there")
}

fn faces_where(m: &Model, pred: impl Fn(&[[f32; 3]], [f32; 3]) -> bool) -> Vec<usize> {
    (0..m.mesh().face_count()).filter(|&f| pred(&points(m, f), m.mesh().face_normal(f))).collect()
}

fn vertex_at(m: &Model, p: [f32; 3]) -> u32 {
    let h = m.mesh();
    (0..h.positions.len()).find(|&i| (0..3).all(|k| (h.positions[i][k] - p[k]).abs() < 1e-4)).expect("a corner there") as u32
}

/// A hole cut into a wall and pushed in `depth` (the opening of a window or door); returns the face at its back.
fn opening(m: &mut Model, outline: &[[f32; 3]], depth: f32) -> usize {
    let f = m.add_outline(outline).unwrap().expect("the outline cut a hole and filled it");
    m.push_pull(f, -depth).unwrap()
}

fn house() -> Model {
    let mut m = Model::new();
    // an 8 x 6 m box, 3 m high
    m.add_rectangle([0.0; 3], [8.0, 0.0, 0.0], [0.0, 6.0, 0.0]).unwrap();
    m.push_pull(0, 3.0).unwrap();
    // the ridge: a line across the top, lifted 2.2 m (the gable walls follow it up)
    m.add_line([0.0, 3.0, 3.0], [8.0, 3.0, 3.0]).unwrap();
    let ridge = [vertex_at(&m, [0.0, 3.0, 3.0]), vertex_at(&m, [8.0, 3.0, 3.0])];
    m.transform_vertices(&ridge, |p| [p[0], p[1], p[2] + 2.2]).unwrap();
    let walls = faces_where(&m, |_, n| n[2].abs() < 0.1);
    m.paint(&walls, Some(WALL)).unwrap();
    let roof = faces_where(&m, |_, n| n[2] > 0.1);
    m.paint(&roof, Some(ROOF)).unwrap();
    // windows in the front (y = 0), the back (y = 6) and the gable end (x = 8), and a door
    let mut glass = vec![];
    for (x0, x1) in [(0.9, 2.5), (5.5, 7.1)] {
        glass.push([[x0, 0.0, 1.0], [x1, 0.0, 1.0], [x1, 0.0, 2.2], [x0, 0.0, 2.2]]);
        glass.push([[x0, 6.0, 1.0], [x0, 6.0, 2.2], [x1, 6.0, 2.2], [x1, 6.0, 1.0]]);
    }
    glass.push([[8.0, 2.2, 1.0], [8.0, 3.8, 1.0], [8.0, 3.8, 2.2], [8.0, 2.2, 2.2]]);
    glass.push([[8.0, 2.6, 3.4], [8.0, 3.4, 3.4], [8.0, 3.4, 4.2], [8.0, 2.6, 4.2]]);
    for w in &glass {
        let back = opening(&mut m, w, 0.15);
        m.paint(&[back], Some(GLASS)).unwrap();
    }
    let door = opening(&mut m, &[[3.4, 0.0, 0.05], [4.6, 0.0, 0.05], [4.6, 0.0, 2.2], [3.4, 0.0, 2.2]], 0.1);
    m.paint(&[door], Some(DOOR)).unwrap();
    // the house becomes a group, so the shed beside it stays apart
    let all: Vec<usize> = (0..m.mesh().face_count()).collect();
    m.make_group(&all, &[], false).unwrap();

    // a shed: 2.4 x 2 m, 2.2 m high, a lean-to roof
    let shed = m.add_rectangle([10.0, 1.0, 0.0], [2.4, 0.0, 0.0], [0.0, 2.0, 0.0]).unwrap();
    m.push_pull(shed, 2.2).unwrap();
    let back_top = [vertex_at(&m, [10.0, 3.0, 2.2]), vertex_at(&m, [12.4, 3.0, 2.2])];
    m.transform_vertices(&back_top, |p| [p[0], p[1], p[2] - 0.4]).unwrap();
    let all: Vec<usize> = (0..m.mesh().face_count()).collect();
    m.paint(&all, Some(WOOD)).unwrap();
    let top = find(&m, |_, n| n[2] > 0.5);
    m.paint(&[top], Some(ROOF)).unwrap();
    m.make_group(&all, &[], false).unwrap();

    m.edit_notes(|n| {
        n.dims.push(Dimension { a: [0.0, 0.0, 0.0], b: [8.0, 0.0, 0.0], offset: [0.0, -1.2, 0.0] });
        n.labels.push(Label { point: [4.0, 0.0, 1.2], at: [4.6, -2.4, 3.6], text: "Front door".into() });
    });
    m
}

fn main() {
    let out = std::env::args().nth(1).expect("usage: demo_models OUT_FOLDER");
    let out = Path::new(&out);
    std::fs::create_dir_all(out).unwrap();
    let mut m = house();
    daedalus_io::save_scene(&m.document(), &out.join("house.daedalus")).unwrap();
    // the same house cut through the middle, looking into the rooms
    m.edit_notes(|n| {
        n.labels.clear();
        n.dims.clear();
        n.section = Some(Section { point: [0.0, 2.6, 0.0], normal: [0.0, -1.0, 0.0], active: true });
    });
    daedalus_io::save_scene(&m.document(), &out.join("house-section.daedalus")).unwrap();
    println!("written to {}", out.display());
}
