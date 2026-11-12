//! The demo models in the README's screenshots, drawn with the same calls the tools make (rectangle, push/pull, lines,
//! outlines that cut holes, paint, groups, text, dimensions, a section plane).
//!
//!     cargo run -p daedalus-io --example demo_models -- OUT_FOLDER
//!
//! writes `house.daedalus`, `house-section.daedalus` (the same house with a section plane cutting through it) and
//! `house-textured.daedalus` (brick walls and a tiled roof; the two images are drawn here too, as PNG files beside it).

use daedalus_core::model::Model;
use daedalus_core::scene::Texture;
use daedalus_core::scene::{Dimension, Label};
use daedalus_core::Paint;
use std::path::Path;

const WALL: [u8; 3] = [232, 222, 198];
const ROOF: [u8; 3] = [146, 62, 48];
const GLASS: [u8; 3] = [126, 164, 204];
const DOOR: [u8; 3] = [118, 80, 52];
const WOOD: [u8; 3] = [168, 128, 84];

fn points(m: &Model, f: usize) -> Vec<[f64; 3]> {
    let h = m.mesh();
    h.face_vertices(f).iter().map(|&v| h.positions[v as usize]).collect()
}

/// The first face whose corners and normal pass `pred`.
fn find(m: &Model, pred: impl Fn(&[[f64; 3]], [f64; 3]) -> bool) -> usize {
    (0..m.mesh().face_count()).find(|&f| pred(&points(m, f), m.mesh().face_normal(f))).expect("the face looked for is there")
}

fn faces_where(m: &Model, pred: impl Fn(&[[f64; 3]], [f64; 3]) -> bool) -> Vec<usize> {
    (0..m.mesh().face_count()).filter(|&f| pred(&points(m, f), m.mesh().face_normal(f))).collect()
}

fn vertex_at(m: &Model, p: [f64; 3]) -> u32 {
    let h = m.mesh();
    (0..h.positions.len()).find(|&i| (0..3).all(|k| (h.positions[i][k] - p[k]).abs() < 1e-4)).expect("a corner there") as u32
}

/// A hole cut into a wall and pushed in `depth` (the opening of a window or door); returns the face at its back.
fn opening(m: &mut Model, outline: &[[f64; 3]], depth: f64) -> usize {
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
    m.paint(&walls, Some(WALL.into())).unwrap();
    let roof = faces_where(&m, |_, n| n[2] > 0.1);
    m.paint(&roof, Some(ROOF.into())).unwrap();
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
        m.paint(&[back], Some(GLASS.into())).unwrap();
    }
    let door = opening(&mut m, &[[3.4, 0.0, 0.05], [4.6, 0.0, 0.05], [4.6, 0.0, 2.2], [3.4, 0.0, 2.2]], 0.1);
    m.paint(&[door], Some(DOOR.into())).unwrap();
    // the house becomes a group, so the shed beside it stays apart
    let all: Vec<usize> = (0..m.mesh().face_count()).collect();
    m.make_group(&all, &[], false).unwrap();

    // a shed: 2.4 x 2 m, 2.2 m high, a lean-to roof
    let shed = m.add_rectangle([10.0, 1.0, 0.0], [2.4, 0.0, 0.0], [0.0, 2.0, 0.0]).unwrap();
    m.push_pull(shed, 2.2).unwrap();
    let back_top = [vertex_at(&m, [10.0, 3.0, 2.2]), vertex_at(&m, [12.4, 3.0, 2.2])];
    m.transform_vertices(&back_top, |p| [p[0], p[1], p[2] - 0.4]).unwrap();
    let all: Vec<usize> = (0..m.mesh().face_count()).collect();
    m.paint(&all, Some(WOOD.into())).unwrap();
    let top = find(&m, |_, n| n[2] > 0.5);
    m.paint(&[top], Some(ROOF.into())).unwrap();
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
        n.add_section([0.0, 2.6, 0.0], [0.0, -1.0, 0.0]);
    });
    daedalus_io::save_scene(&m.document(), &out.join("house-section.daedalus")).unwrap();
    // the same house in brick, with a tiled roof
    m.edit_notes(|n| n.sections.clear());
    let brick = out.join("brick.png");
    image::RgbImage::from_fn(256, 128, |x, y| {
        // courses 64 px high, every other one shifted half a brick; mortar lines 6 px
        let course = y / 64;
        let xs = (x + if course % 2 == 1 { 64 } else { 0 }) % 128;
        let mortar = y % 64 < 6 || xs < 6;
        let shade = ((x * 7 + y * 13) % 23) as u8;
        if mortar {
            image::Rgb([205, 200, 190])
        } else {
            image::Rgb([150 + shade, 62 + shade / 2, 44])
        }
    })
    .save(&brick)
    .unwrap();
    let tiles = out.join("roof-tiles.png");
    image::RgbImage::from_fn(128, 128, |x, y| {
        let edge = y % 64 < 5 || (x + if (y / 64) % 2 == 1 { 32 } else { 0 }) % 64 < 3;
        if edge {
            image::Rgb([70, 40, 35])
        } else {
            image::Rgb([120, (48 + y % 64 / 4) as u8, 40])
        }
    })
    .save(&tiles)
    .unwrap();
    let wall_tex = m.add_texture(Texture { name: "Brick".into(), path: brick.display().to_string(), size: [0.9, 0.45] });
    let roof_tex = m.add_texture(Texture { name: "Roof tiles".into(), path: tiles.display().to_string(), size: [0.6, 0.6] });
    // inside the house group: its walls and roof
    m.open(0).unwrap();
    let walls = faces_where(&m, |_, n| n[2].abs() < 0.1);
    let walls: Vec<usize> = walls.into_iter().filter(|&f| m.mesh().to_mesh().color_of(f).map(|c| c.rgb) == Some(WALL)).collect();
    m.paint(&walls, Some(Paint { rgb: [150, 70, 50], texture: Some(wall_tex as u16) })).unwrap();
    let roof = faces_where(&m, |_, n| n[2] > 0.1);
    m.paint(&roof, Some(Paint { rgb: [120, 55, 42], texture: Some(roof_tex as u16) })).unwrap();
    m.close();
    daedalus_io::save_scene(&m.document(), &out.join("house-textured.daedalus")).unwrap();
    println!("written to {}", out.display());
}
