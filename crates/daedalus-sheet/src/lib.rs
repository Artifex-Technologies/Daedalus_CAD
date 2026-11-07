//! Exact solids for Daedalus: sheet-metal parts and STEP export, on SolidCraft's B-rep kernel (see `NOTICE.md` in this
//! crate for its licences). The SketchUp-style modelling keeps Daedalus's own polygon kernel (`daedalus-core`); this crate
//! is where true curved surfaces live: bends, round holes, and what a fabricator needs in a STEP file.

pub use solidcraft_kernel as kernel;

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
}
