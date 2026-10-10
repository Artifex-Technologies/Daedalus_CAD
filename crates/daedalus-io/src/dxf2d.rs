//! 2D DXF output for flat patterns (laser / waterjet / CNC cutting services).
//!
//! Cut geometry goes on one layer (`CUT`) as closed paths with no duplicate
//! lines; bend lines go on their own layer (`BEND`) at the centre of each bend.
//! Output is 1:1 with the units recorded in the file header.

use daedalus_core::sheetmetal::FlatPattern;
use std::fmt::Write;

pub const CUT_LAYER: &str = "CUT";
pub const BEND_LAYER: &str = "BEND";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Units {
    Millimeters,
    Inches,
}

impl Units {
    /// Value of the DXF `$INSUNITS` header variable.
    fn insunits(self) -> i32 {
        match self {
            Units::Millimeters => 4,
            Units::Inches => 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Segment {
    pub layer: &'static str,
    pub a: [f32; 2],
    pub b: [f32; 2],
}

fn layer_color(name: &str) -> i32 {
    match name {
        BEND_LAYER => 1, // red
        _ => 7,          // white/black
    }
}

/// Writes line segments as an ASCII DXF with header units and a layer table.
pub fn write_segments(segments: &[Segment], units: Units) -> String {
    let mut layers: Vec<&str> = Vec::new();
    for s in segments {
        if !layers.contains(&s.layer) {
            layers.push(s.layer);
        }
    }

    let mut d = String::new();
    let _ = write!(d, "0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n{}\n0\nENDSEC\n", units.insunits());
    d.push_str("0\nSECTION\n2\nTABLES\n");
    d.push_str("0\nTABLE\n2\nLTYPE\n70\n1\n0\nLTYPE\n2\nCONTINUOUS\n70\n0\n3\nSolid line\n72\n65\n73\n0\n40\n0.0\n0\nENDTAB\n");
    let _ = write!(d, "0\nTABLE\n2\nLAYER\n70\n{}\n", layers.len());
    for l in &layers {
        let _ = write!(d, "0\nLAYER\n2\n{l}\n70\n0\n62\n{}\n6\nCONTINUOUS\n", layer_color(l));
    }
    d.push_str("0\nENDTAB\n0\nENDSEC\n");

    d.push_str("0\nSECTION\n2\nENTITIES\n");
    for s in segments {
        let _ = write!(
            d,
            "0\nLINE\n8\n{}\n10\n{}\n20\n{}\n30\n0.0\n11\n{}\n21\n{}\n31\n0.0\n",
            s.layer, s.a[0], s.a[1], s.b[0], s.b[1]
        );
    }
    d.push_str("0\nENDSEC\n0\nEOF\n");
    d
}

/// Outline of the strip on `CUT` and one line per bend on `BEND`.
/// The strip runs along X (length) and Y (width), starting at the origin.
pub fn flat_pattern_dxf(flat: &FlatPattern, units: Units) -> String {
    let (l, w) = (flat.length, flat.width);
    let cut = |a, b| Segment { layer: CUT_LAYER, a, b };
    let mut segments = vec![
        cut([0.0, 0.0], [l, 0.0]),
        cut([l, 0.0], [l, w]),
        cut([l, w], [0.0, w]),
        cut([0.0, w], [0.0, 0.0]),
    ];
    for b in &flat.bend_lines {
        segments.push(Segment { layer: BEND_LAYER, a: [b.position, 0.0], b: [b.position, w] });
    }
    write_segments(&segments, units)
}

#[cfg(test)]
mod tests {
    use super::*;
    use daedalus_core::sheetmetal::{flat_pattern, Bend, Material, Profile};

    fn count(s: &str, needle: &str) -> usize {
        s.lines().filter(|l| *l == needle).count()
    }

    #[test]
    fn l_bracket_dxf_has_outline_and_one_bend_line() {
        let bend = Bend { angle_deg: 90.0, inside_radius: 2.0 };
        let p = Profile { flanges: vec![50.0, 30.0], bends: vec![bend] };
        let flat = flat_pattern(&p, Material { thickness: 2.0, k_factor: 0.44 }, 40.0).unwrap();
        let dxf = flat_pattern_dxf(&flat, Units::Millimeters);

        assert_eq!(count(&dxf, "LINE"), 5);
        assert_eq!(count(&dxf, "CUT"), 4 + 1); // 4 entities + 1 layer-table entry
        assert_eq!(count(&dxf, "BEND"), 1 + 1);
        assert!(dxf.contains("$INSUNITS\n70\n4\n"));
        assert!(dxf.ends_with("0\nEOF\n"));
    }

    #[test]
    fn inches_set_the_header_unit() {
        let dxf = write_segments(&[Segment { layer: CUT_LAYER, a: [0.0, 0.0], b: [1.0, 0.0] }], Units::Inches);
        assert!(dxf.contains("$INSUNITS\n70\n1\n"));
    }
}
