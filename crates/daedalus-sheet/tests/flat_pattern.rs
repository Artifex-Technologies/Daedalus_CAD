//! Folding and unfolding checked against hand numbers: where a jog's bend lines fall in the blank, the folded part's size
//! from mould lines, setbacks and bend allowances (BA = angle x (R + K T)), the sheet's thickness on every flat panel, and
//! the volume (the neutral layer keeps its length, so the folded part differs from the blank by exactly
//! angle x T² x length x (0.5 - K) per bend: nothing when K is 0.5).

use daedalus_core::Mesh;
use daedalus_io::dxf2d::Units;
use daedalus_sheet::kernel;
use daedalus_sheet::part::{FlangePosition, FoldPosition, SheetPart};
use daedalus_sheet::sheet::RuleValues;
use solidcore_record::{Loop2, Region2, Vec2};
use solidcraft_geom::prelude::*;

const T: f64 = 2.0;
const R: f64 = 2.0;

fn rule(k: f64) -> RuleValues {
    RuleValues { k_factor: k, bend_radius: R, ..RuleValues::steel(T) }
}

/// A 120 x 60 mm plate, 2 mm, inside radius 2 mm, the K-factor given.
fn plate(k: f64) -> SheetPart {
    let outer = Loop2::polygon(&[Vec2::new(0.0, 0.0), Vec2::new(120.0, 0.0), Vec2::new(120.0, 60.0), Vec2::new(0.0, 60.0)]).ccw();
    SheetPart::from_region("Steel 2 mm", rule(k), Region2 { outer, holes: Vec::new() }).unwrap()
}

/// Bend allowance by hand: the neutral layer's length through the bend.
fn ba(angle_deg: f64, k: f64) -> f64 {
    angle_deg.to_radians() * (R + k * T)
}

/// Outside setback by hand: from where the bend starts to the virtual sharp on its outside face.
fn ossb(angle_deg: f64) -> f64 {
    (R + T) * (angle_deg.to_radians() / 2.0).tan()
}

/// The web of a jog stepping `step` (top face to top face) through two bends of `angle_deg`: its length between the two
/// virtual sharps (they lie on opposite faces of the web), and its flat length between the bends.
fn jog_web(step: f64, angle_deg: f64) -> (f64, f64) {
    let a = angle_deg.to_radians();
    let mould = (T + step - T * a.cos()) / a.sin();
    (mould, mould - 2.0 * ossb(angle_deg))
}

fn folded(p: &SheetPart) -> kernel::Body {
    let mut s = p.sheet.clone();
    s.flat = false;
    s.solid().unwrap()
}

/// (volume of the folded part, volume of the blank, what the bends should add: the sum of angle x T² x length x (0.5 - K)).
fn volumes(p: &SheetPart) -> (f64, f64, f64) {
    let bent = kernel::measure(&folded(p)).unwrap().volume;
    let flat = kernel::measure(&p.sheet.flat_solid().unwrap()).unwrap().volume;
    let k = p.rule.k_factor;
    let extra = p.sheet.bends().iter().map(|b| b.angle_deg.to_radians() * T * T * b.length * (0.5 - k)).sum();
    (bent, flat, extra)
}

/// Every flat face of the folded part that is a panel's top or bottom (not an end, a side, or a facet of a bend) has a face
/// facing the other way exactly `T` behind it. Returns how many such faces there are. `axis` is the bends' direction.
fn panels_keep_thickness(mesh: &Mesh, axis: [f64; 3]) -> usize {
    let mm = |p: [f64; 3]| [p[0] * 1000.0, p[1] * 1000.0, p[2] * 1000.0];
    let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let cross = |a: [f64; 3], b: [f64; 3]| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    // each face: its normal (Newell), a point on it, and its extent across the bend axis in its plane
    type Face = ([f64; 3], [f64; 3], (f64, f64));
    let faces: Vec<Face> = mesh
        .faces
        .iter()
        .map(|f| {
            let pts: Vec<[f64; 3]> = f.iter().map(|&v| mm(mesh.positions[v as usize])).collect();
            let mut n = [0.0; 3];
            for (i, a) in pts.iter().enumerate() {
                let b = pts[(i + 1) % pts.len()];
                n[0] += (a[1] - b[1]) * (a[2] + b[2]);
                n[1] += (a[2] - b[2]) * (a[0] + b[0]);
                n[2] += (a[0] - b[0]) * (a[1] + b[1]);
            }
            let l = dot(n, n).sqrt();
            let n = [n[0] / l, n[1] / l, n[2] / l];
            let across = cross(n, axis);
            let (lo, hi) = pts.iter().map(|p| dot(*p, across)).fold((f64::MAX, f64::MIN), |(lo, hi), x| (lo.min(x), hi.max(x)));
            (n, pts[0], (lo, hi))
        })
        .collect();
    let mut panels = 0;
    for (n, p, span) in &faces {
        if dot(*n, axis).abs() > 1e-9 || span.1 - span.0 < 1.5 * T {
            continue;
        }
        panels += 1;
        let across = cross(*n, axis);
        let partner = faces.iter().any(|(m, q, other)| {
            // facing the other way, its span runs along the opposite direction
            let overlap = span.1.min(-other.0) - span.0.max(-other.1);
            let behind = dot([q[0] - p[0], q[1] - p[1], q[2] - p[2]], *n);
            dot(*m, *n) < -1.0 + 1e-9 && (behind + T).abs() < 1e-6 && overlap > T && dot(cross(*m, axis), across) < 0.0
        });
        if !partner {
            for (m, q, other) in &faces {
                eprintln!("face at {q:?} facing {m:?} spans {other:?}");
            }
        }
        assert!(partner, "a panel face at {p:?} facing {n:?} spanning {span:?} has no face {T} mm behind it");
    }
    panels
}

/// The LINE entities on `layer` of a DXF: ((x1, y1), (x2, y2)).
fn dxf_lines(dxf: &str, layer: &str) -> Vec<([f64; 2], [f64; 2])> {
    let codes: Vec<&str> = dxf.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i + 1 < codes.len() {
        if codes[i] == "0" && codes[i + 1] == "LINE" {
            let mut j = i + 2;
            let (mut l, mut v) = ("", [0.0; 4]);
            while j + 1 < codes.len() && codes[j] != "0" {
                let x = codes[j + 1].parse::<f64>().unwrap_or(0.0);
                match codes[j] {
                    "8" => l = codes[j + 1],
                    "10" => v[0] = x,
                    "20" => v[1] = x,
                    "11" => v[2] = x,
                    "21" => v[3] = x,
                    _ => {}
                }
                j += 2;
            }
            if l == layer {
                out.push(([v[0], v[1]], [v[2], v[3]]));
            }
            i = j;
        } else {
            i += 1;
        }
    }
    out
}

/// The jogs tried: (step mm, angle degrees, down). Each leaves a web of more than 1.5 T between its bends.
const JOGS: [(f64, f64, bool); 4] = [(12.0, 90.0, false), (8.0, 45.0, false), (8.0, 60.0, true), (15.0, 90.0, true)];

#[test]
fn a_jogs_two_bend_lines_fall_where_hand_numbers_put_them_and_go_out_in_the_dxf() {
    for (step, angle, down) in JOGS {
        let what = format!("{step} mm at {angle} degrees{}", if down { ", down" } else { "" });
        let mut p = plate(0.44);
        p.add_jog([80.0, -10.0, T], [80.0, 70.0, T], step, angle, down).unwrap();
        let mut bends = p.sheet.bends();
        bends.sort_by(|a, b| a.start.x.total_cmp(&b.start.x));
        assert_eq!(bends.len(), 2, "{what}");
        let (_, web) = jog_web(step, angle);
        let b = ba(angle, 0.44);
        // the first bend centred on the line drawn, the second a bend allowance and the web's flat length further on
        let want = [80.0, 80.0 + b + web];
        for (bend, x) in bends.iter().zip(want) {
            assert!(
                (bend.start.x - x).abs() < 1e-6 && (bend.end.x - x).abs() < 1e-6,
                "{what}: a bend line at {:?}, want x = {x}",
                bend.start
            );
            assert!((bend.length - 60.0).abs() < 1e-9 && (bend.allowance - b).abs() < 1e-9, "{what}: {bend:?}");
            assert!((bend.angle_deg - angle).abs() < 1e-9, "{what}");
        }
        // opposite ways round: the first the way asked
        assert_eq!(bends[0].up, !down, "{what}");
        assert_eq!(bends[1].up, down, "{what}");
        // the bend table the Flat pattern tab shows says so too: one up, one down
        assert_eq!(p.bend_table().iter().filter(|b| b.3).count(), 1, "{what}");
        // the blank is still the plate (a jog drawn on a panel takes its bends out of it)
        let (w, h) = p.flat_size().unwrap();
        assert!((w - 120.0).abs() < 1e-6 && (h - 60.0).abs() < 1e-6, "{what}: blank {w} x {h}");
        // the DXF: the four sides on CUT, both bend lines on BEND where the flat has them, in mm and in inches
        let dxf = p.flat_dxf(Units::Millimeters).unwrap();
        assert_eq!(dxf_lines(&dxf, "CUT").len(), 4, "{what}");
        let mut lines = dxf_lines(&dxf, "BEND");
        lines.sort_by(|a, b| a.0[0].total_cmp(&b.0[0]));
        assert_eq!(lines.len(), 2, "{what}");
        for (l, x) in lines.iter().zip(want) {
            assert!((l.0[0] - x).abs() < 1e-6 && (l.1[0] - x).abs() < 1e-6 && (l.0[1] - l.1[1]).abs() > 60.0 - 1e-6, "{what}: {l:?}");
        }
        let inches = dxf_lines(&p.flat_dxf(Units::Inches).unwrap(), "BEND");
        assert!(inches.iter().any(|l| (l.0[0] - want[1] / 25.4).abs() < 1e-6), "{what}: {inches:?}");
    }
}

#[test]
fn a_jogged_part_folds_to_the_size_its_mould_lines_setbacks_and_allowances_give() {
    for (step, angle, down) in JOGS {
        let what = format!("{step} mm at {angle} degrees{}", if down { ", down" } else { "" });
        let mut p = plate(0.44);
        p.add_jog([80.0, -10.0, T], [80.0, 70.0, T], step, angle, down).unwrap();
        let a = angle.to_radians();
        let (b, s) = (ba(angle, 0.44), ossb(angle));
        let (mould, web) = jog_web(step, angle);
        // the base runs flat to the first bend, the far panel from the second to the end of the 120 mm blank
        let base_flat = 80.0 - b / 2.0;
        let far_flat = 120.0 - (base_flat + b + web + b);
        // flat length = the flat lengths and the allowances; each flat length is its mould length less its setbacks
        let (base_mould, far_mould) = (base_flat + s, far_flat + s);
        assert!((base_mould - s + b + (mould - 2.0 * s) + b + (far_mould - s) - 120.0).abs() < 1e-9);
        // folded: the far panel's end is the base's mould line, then the web's across, then the far panel's mould line
        // (the web's two sharps are a thickness apart across it)
        let length = base_mould + mould * a.cos() - T * a.sin() + far_mould;
        let bb = kernel::measure(&folded(&p)).unwrap().bbox;
        let size = bb.max - bb.min;
        assert!((size.x - length).abs() < 1e-6, "{what}: folded {} long, by hand {length}", size.x);
        assert!((size.y - 60.0).abs() < 1e-6, "{what}: {} wide", size.y);
        assert!((size.z - (T + step)).abs() < 1e-6, "{what}: {} high, by hand T + step = {}", size.z, T + step);
    }
}

#[test]
fn a_flange_and_a_jog_together_unfold_to_the_sum_of_their_flat_lengths_and_allowances() {
    let mut p = plate(0.44);
    // a 25 mm flange (outside size) on the end at x = 0, then a 12 mm jog at x = 80
    p.add_flange([0.0, 30.0, T], 25.0, 90.0, FlangePosition::Inside, false).unwrap();
    p.add_jog([80.0, -10.0, T], [80.0, 70.0, T], 12.0, 90.0, false).unwrap();
    assert_eq!(p.sheet.bends().len(), 3);
    // the flange's outside face stays at x = 0: the base loses a setback, the flange is its height less one
    let want = (120.0 - ossb(90.0)) + (25.0 - ossb(90.0)) + ba(90.0, 0.44);
    let (w, h) = p.flat_size().unwrap();
    assert!((w - want).abs() < 1e-6 && (h - 60.0).abs() < 1e-6, "blank {w} x {h}, by hand {want} x 60");
    let bb = kernel::measure(&folded(&p)).unwrap().bbox;
    assert!((bb.max.z - bb.min.z - 25.0).abs() < 1e-6, "the flange stands 25 mm: {:?}", bb);
}

#[test]
fn folding_keeps_the_volume_the_neutral_layer_says_and_the_thickness_on_every_panel() {
    type Make = fn(&mut SheetPart);
    let parts: [(&str, Make); 6] = [
        ("a flange", |p| p.add_flange([60.0, 0.0, T], 30.0, 90.0, FlangePosition::Inside, false).unwrap()),
        ("a fold at 60 degrees", |p| p.add_fold([80.0, -10.0, T], [80.0, 70.0, T], 60.0, FoldPosition::Centre, false).unwrap()),
        ("a 90 degree jog", |p| p.add_jog([80.0, -10.0, T], [80.0, 70.0, T], 12.0, 90.0, false).unwrap()),
        ("a 45 degree jog down", |p| p.add_jog([80.0, -10.0, T], [80.0, 70.0, T], 8.0, 45.0, true).unwrap()),
        ("a flange and a jog", |p| {
            p.add_flange([0.0, 30.0, T], 25.0, 90.0, FlangePosition::Inside, false).unwrap();
            p.add_jog([80.0, -10.0, T], [80.0, 70.0, T], 12.0, 90.0, false).unwrap();
        }),
        ("a hem", |p| p.add_hem([60.0, 0.0, T], 12.0, false).unwrap()),
    ];
    for k in [0.5, 0.44, 0.33] {
        for (what, make) in parts {
            let mut p = plate(k);
            make(&mut p);
            let (bent, flat, extra) = volumes(&p);
            if k == 0.5 {
                assert!(extra == 0.0, "{what}");
            }
            // SMLib integrates to 1e-8; the bends' share is about 1e-3 of the part, so it is seen plainly
            assert!((bent - (flat + extra)).abs() < 1e-6 * flat, "{what}, K {k}: folded {bent} mm³, blank {flat} + bends {extra}");
            // the hem's bend axis is along X, the rest along Y
            let axis = if what == "a hem" || what == "a flange" { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] };
            let n = panels_keep_thickness(&p.faces(false).unwrap(), axis);
            assert!(n >= 4, "{what}: {n} panel faces checked");
        }
    }
}

#[test]
fn a_jog_on_a_standing_flange_steps_it_and_the_blank_stays_the_same() {
    let mut p = plate(0.44);
    // a 50 mm flange up on the end at x = 120 (outer face there), then a jog across it 25 mm up
    p.add_flange([120.0, 30.0, T], 50.0, 90.0, FlangePosition::Inside, false).unwrap();
    let before = p.flat_size().unwrap();
    let (v0, f0, _) = volumes(&p);
    p.add_jog([120.0, -10.0, 25.0], [120.0, 70.0, 25.0], 10.0, 90.0, false).unwrap();
    assert_eq!(p.sheet.bends().len(), 3);
    let after = p.flat_size().unwrap();
    assert!((after.0 - before.0).abs() < 1e-6 && (after.1 - before.1).abs() < 1e-6, "{before:?} -> {after:?}");
    let (bent, flat, extra) = volumes(&p);
    assert!((flat - f0).abs() < 1e-6 * f0, "the blank keeps its volume");
    assert!((bent - (flat + extra)).abs() < 1e-6 * flat && bent != v0, "folded {bent}, blank {flat} + {extra}");
    // up on the flange is toward its top (inner) face: the flange's upper part steps 10 mm in, toward the base, and the
    // flange stands lower by what the jog takes up in the flat (two allowances and the web's flat length) less what it
    // reaches along the flange folded (two setbacks, and the web's mould line along the flange: none at 90 degrees, and its
    // two sharps a thickness apart across it take T back)
    let (mould, web) = jog_web(10.0, 90.0);
    let a = 90_f64.to_radians();
    let shrink = 2.0 * ba(90.0, 0.44) + web - (2.0 * ossb(90.0) + mould * a.cos() - T * a.sin());
    let bb = kernel::measure(&folded(&p)).unwrap().bbox;
    assert!((bb.max.z - bb.min.z - (50.0 - shrink)).abs() < 1e-6, "{bb:?}, by hand {} high", 50.0 - shrink);
    assert!((bb.max.x - 120.0).abs() < 1e-6 && bb.min.x.abs() < 1e-6, "{bb:?}");
    // the stepped part's outer face is 10 mm in from the flange's (x = 120), up to the top
    let mesh = p.faces(false).unwrap();
    let stepped = mesh.faces.iter().any(|f| {
        let xs: Vec<[f64; 3]> = f.iter().map(|&v| mesh.positions[v as usize].map(|c| c * 1000.0)).collect();
        xs.iter().all(|q| (q[0] - 110.0).abs() < 1e-6) && xs.iter().any(|q| (q[2] - (50.0 - shrink)).abs() < 1e-6)
    });
    assert!(stepped, "no outer face at x = 110 reaching the top");
    panels_keep_thickness(&p.faces(false).unwrap(), [0.0, 1.0, 0.0]);
}
