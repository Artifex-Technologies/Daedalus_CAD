//! Exact solids for Daedalus: sheet-metal parts and STEP export, on SolidCraft's B-rep kernel (see `NOTICE.md` in this
//! crate for its licences). The SketchUp-style modelling keeps Daedalus's own polygon kernel (`daedalus-core`); this crate
//! is where true curved surfaces live: bends, round holes, and what a fabricator needs in a STEP file.

pub use solidcraft_kernel as kernel;

pub mod part;
#[allow(dead_code)] // SolidCraft helpers its other features used; kept so updates stay easy to compare
pub mod sheet;
pub mod util;

#[cfg(test)]
mod tests {
    use solidcore_record::{Loop2, Plane, Region2, Seg2, Vec2};
    use std::f64::consts::TAU;

    #[test]
    fn a_plate_with_a_round_hole_goes_out_as_exact_step() {
        // 40 x 20 mm, 3 mm thick, a 6 mm hole
        let rect = vec![
            Seg2::Line { a: Vec2::new(0.0, 0.0), b: Vec2::new(40.0, 0.0) },
            Seg2::Line { a: Vec2::new(40.0, 0.0), b: Vec2::new(40.0, 20.0) },
            Seg2::Line { a: Vec2::new(40.0, 20.0), b: Vec2::new(0.0, 20.0) },
            Seg2::Line { a: Vec2::new(0.0, 20.0), b: Vec2::new(0.0, 0.0) },
        ];
        let hole = Loop2 { segs: vec![Seg2::Arc { center: Vec2::new(10.0, 10.0), radius: 3.0, start: 0.0, sweep: -TAU }] };
        let plate = crate::kernel::extrude(&Plane::XY, &[Region2 { outer: Loop2 { segs: rect }, holes: vec![hole] }], 0.0, 3.0).unwrap();
        assert_eq!(plate.len(), 1, "one solid");
        let step = crate::kernel::step_export(&[&plate[0]], "Daedalus CAD").unwrap();
        assert!(step.starts_with("ISO-10303-21;"));
        assert!(step.contains("MANIFOLD_SOLID_BREP"), "a solid");
        assert!(step.contains("CYLINDRICAL_SURFACE"), "the hole is a true cylinder, not facets");
    }

    /// An L-bracket as a shop would make it: 2 mm steel, K 0.44, inside radius 2 mm; base 80 x 40, a 30 mm leg (outer
    /// sizes). By hand: outside setback T + R = 4 mm, bend allowance (pi/2)(R + K T) = 4.5239 mm, so the flat blank is
    /// (40 - 4) + (30 - 4) + 4.5239 = 66.5239 mm long. Folded it is one solid with true cylinders in the bend.
    #[test]
    fn an_l_bracket_folds_unfolds_and_goes_out_as_step() {
        use crate::sheet::{RuleValues, base_flange};
        use solidcore_record::Vec3;
        let line = |a: (f64, f64), b: (f64, f64)| Seg2::Line { a: Vec2::new(a.0, a.1), b: Vec2::new(b.0, b.1) };
        let outline = vec![line((0., 0.), (80., 0.)), line((80., 0.), (80., 40.)), line((80., 40.), (0., 40.)), line((0., 40.), (0., 0.))];
        let rv = RuleValues { k_factor: 0.44, bend_radius: 2.0, ..RuleValues::steel(2.0) }.checked().unwrap();
        let mut part = base_flange("Bracket", "Steel 2 mm", &rv, Plane::XY, Region2 { outer: Loop2 { segs: outline }, holes: vec![] });
        part.add_flange(Vec3::new(40.0, 0.0, 2.0), 30.0, std::f64::consts::FRAC_PI_2, 2.0, "inside", false).unwrap();
        let ba = std::f64::consts::FRAC_PI_2 * (2.0 + 0.44 * 2.0);
        let bends = part.bends();
        assert_eq!(bends.len(), 1);
        assert!((bends[0].allowance - ba).abs() < 1e-9 && (bends[0].angle_deg - 90.0).abs() < 1e-9 && bends[0].length == 80.0);
        let (w, h) = part.flat_size().unwrap();
        assert!((w - 80.0).abs() < 1e-9 && (h - (36.0 + 26.0 + ba)).abs() < 1e-6, "flat {w} x {h}");
        // folded: 80 x 40 x 30 outside
        let solid = part.solid().unwrap();
        let m = crate::kernel::measure(&solid).unwrap();
        let size = m.bbox.max - m.bbox.min;
        assert!((size.x - 80.0).abs() < 1e-6 && (size.y - 40.0).abs() < 1e-6 && (size.z - 30.0).abs() < 1e-6, "{size:?}");
        // the blank's volume is the folded part's (the neutral axis keeps its length)
        let flat = part.flat_solid().unwrap();
        let fv = crate::kernel::measure(&flat).unwrap().volume;
        assert!((fv - 80.0 * h * 2.0).abs() < 0.5 && (m.volume - fv).abs() / fv < 0.02, "flat {fv}, folded {}", m.volume);
        let step = crate::kernel::step_export(&[&solid], "Daedalus CAD").unwrap();
        assert_eq!(step.matches("MANIFOLD_SOLID_BREP").count(), 1, "one solid");
        assert!(step.matches("CYLINDRICAL_SURFACE").count() >= 2, "inner and outer bend faces are true cylinders");
    }
}
