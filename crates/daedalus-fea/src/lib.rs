//! Stress testing (linear static finite elements) for Daedalus.
//!
//! A solid part (a closed triangle mesh, faces numbered) is filled with cubes (`voxel`), each cube an 8-node brick with
//! incompatible modes (`hex8`), and K u = f is solved on the grid without storing K (`solve`). Faces of the part are held
//! (supports) or pushed (loads: a total force spread evenly over the faces); the part's own weight can be added. The result
//! is the displacement of every grid node, the stress of every cube, the highest von Mises stress and the safety factor
//! against the material's yield strength.
//!
//! What it is: linear (small deflections, the material stays elastic), one material, static loads. What it is not: buckling,
//! fatigue, contact, plasticity, bolts or welds. A voxel model has stair-stepped surfaces, so stresses at sharp inside corners
//! and at the edges of supports read high; finer cubes bring them closer (until the corner's true peak, which a perfectly
//! sharp corner does not have).
//!
//! Checked against textbook answers in the tests: a bar in tension (stress F/A, stretch FL/AE) and a cantilever (tip
//! deflection PL³/3EI plus shear, stress Mc/I).

pub mod hex8;
pub mod material;
pub mod solve;
pub mod voxel;

pub use material::Material;
use voxel::{Grid, Tri};

/// A part: its surface as triangles (m, outward), each with the number of the model face it belongs to.
#[derive(Clone, Debug, Default)]
pub struct Part {
    pub triangles: Vec<Tri>,
    pub face_of: Vec<usize>,
}

/// A force on faces of the part: `force` (N, in model axes) is the total, spread evenly over the faces' area.
#[derive(Clone, Debug, PartialEq)]
pub struct Load {
    pub faces: Vec<usize>,
    pub force: [f64; 3],
}

#[derive(Clone, Debug)]
pub struct Study {
    pub material: Material,
    /// Faces held in place (all three directions).
    pub fixed: Vec<usize>,
    pub loads: Vec<Load>,
    /// Gravity (m/s², e.g. [0, 0, -9.81]) for the part's own weight, or none.
    pub gravity: Option<[f64; 3]>,
    /// Cubes along the part's longest side (the cube size follows).
    pub divisions: usize,
}

impl Study {
    pub fn new(material: Material) -> Study {
        Study { material, fixed: vec![], loads: vec![], gravity: None, divisions: 60 }
    }
}

/// One square of the cubes' outer surface (for drawing the result): its corners, their displacements and stresses.
#[derive(Clone, Debug)]
pub struct SurfaceQuad {
    pub corners: [[f64; 3]; 4],
    pub displacement: [[f64; 3]; 4],
    /// Von Mises stress at each corner, Pa (averaged over the cubes that share the corner).
    pub stress: [f64; 4],
    /// The model face it was matched to (supports and loads are given by these).
    pub face: Option<usize>,
}

#[derive(Clone, Debug)]
pub struct Report {
    pub cubes: usize,
    /// Cube side, m.
    pub cube_size: f64,
    /// Highest von Mises stress, Pa, and the grid point where it is.
    pub max_stress: f64,
    pub max_stress_at: [f64; 3],
    /// Largest displacement, m, and where.
    pub max_displacement: f64,
    pub max_displacement_at: [f64; 3],
    /// Yield strength / highest stress.
    pub safety_factor: f64,
    /// Mass of the cubes, kg.
    pub mass: f64,
    /// (cubes' volume - part's volume) / part's volume.
    pub volume_error: f64,
    pub iterations: usize,
    pub converged: bool,
    /// Sum of all forces on the part at the supports (should balance the loads): N.
    pub reaction: [f64; 3],
    pub surface: Vec<SurfaceQuad>,
    /// Things to know about this result (thin walls, a mesh not closed, ...).
    pub notes: Vec<String>,
}

/// Outward normals and the cube corners (hex8 order) on each side: -x, +x, -y, +y, -z, +z.
const SIDES: [([i8; 3], [usize; 4]); 6] = [
    ([-1, 0, 0], [0, 4, 7, 3]),
    ([1, 0, 0], [1, 2, 6, 5]),
    ([0, -1, 0], [0, 1, 5, 4]),
    ([0, 1, 0], [3, 7, 6, 2]),
    ([0, 0, -1], [0, 3, 2, 1]),
    ([0, 0, 1], [4, 5, 6, 7]),
];

struct Quad {
    cube: usize,
    nodes: [usize; 4],
    face: Option<usize>,
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot3(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn len(a: [f64; 3]) -> f64 {
    dot3(a, a).sqrt()
}

/// Distance from p to the triangle (closest point by regions, as in Ericson's Real-Time Collision Detection 5.1.5).
fn distance_to_triangle(p: [f64; 3], t: &Tri) -> f64 {
    let [a, b, c] = *t;
    let (ab, ac, ap) = (sub(b, a), sub(c, a), sub(p, a));
    let (d1, d2) = (dot3(ab, ap), dot3(ac, ap));
    if d1 <= 0.0 && d2 <= 0.0 {
        return len(ap);
    }
    let bp = sub(p, b);
    let (d3, d4) = (dot3(ab, bp), dot3(ac, bp));
    if d3 >= 0.0 && d4 <= d3 {
        return len(bp);
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = d1 / (d1 - d3);
        return len(sub(p, [a[0] + v * ab[0], a[1] + v * ab[1], a[2] + v * ab[2]]));
    }
    let cp = sub(p, c);
    let (d5, d6) = (dot3(ab, cp), dot3(ac, cp));
    if d6 >= 0.0 && d5 <= d6 {
        return len(cp);
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let w = d2 / (d2 - d6);
        return len(sub(p, [a[0] + w * ac[0], a[1] + w * ac[1], a[2] + w * ac[2]]));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        let bc = sub(c, b);
        return len(sub(p, [b[0] + w * bc[0], b[1] + w * bc[1], b[2] + w * bc[2]]));
    }
    let denom = 1.0 / (va + vb + vc);
    let (v, w) = (vb * denom, vc * denom);
    let q = [a[0] + ab[0] * v + ac[0] * w, a[1] + ab[1] * v + ac[1] * w, a[2] + ab[2] * v + ac[2] * w];
    len(sub(p, q))
}

/// The cubes' outer squares, each matched to the model face nearest to it that faces the same way.
fn surface(grid: &Grid, part: &Part) -> Vec<Quad> {
    let mut quads = Vec::new();
    for c in 0..grid.solid.len() {
        if !grid.solid[c] {
            continue;
        }
        let [i, j, k] = grid.cube_ijk(c);
        let nodes = grid.cube_nodes(c);
        for (n, corners) in SIDES.iter() {
            if !grid.solid_at(i as isize + n[0] as isize, j as isize + n[1] as isize, k as isize + n[2] as isize) {
                quads.push(Quad { cube: c, nodes: corners.map(|l| nodes[l]), face: None });
            }
        }
    }
    // triangle normals and bounds, once
    let info: Vec<([f64; 3], [f64; 3], [f64; 3])> = part
        .triangles
        .iter()
        .map(|t| {
            let n = cross(sub(t[1], t[0]), sub(t[2], t[0]));
            let l = len(n).max(1e-300);
            let (lo, hi) = voxel::bounds(std::slice::from_ref(t));
            ([n[0] / l, n[1] / l, n[2] / l], lo, hi)
        })
        .collect();
    let reach = grid.h;
    let per = quads.len().div_ceil(std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1)).max(64);
    std::thread::scope(|s| {
        for chunk in quads.chunks_mut(per) {
            let info = &info;
            s.spawn(move || {
                for q in chunk {
                    let side = SIDES.iter().find(|(_, corners)| corners.map(|l| grid.cube_nodes(q.cube)[l]) == q.nodes).unwrap().0;
                    let normal = [side[0] as f64, side[1] as f64, side[2] as f64];
                    let ps: Vec<[f64; 3]> = q.nodes.iter().map(|n| grid.node_position(*n)).collect();
                    let centre = [0, 1, 2].map(|a| ps.iter().map(|p| p[a]).sum::<f64>() / 4.0);
                    let mut best: Option<(f64, usize)> = None;
                    for (t, (tn, lo, hi)) in info.iter().enumerate() {
                        if (0..3).any(|a| centre[a] < lo[a] - reach || centre[a] > hi[a] + reach) || dot3(*tn, normal) < 0.1 {
                            continue;
                        }
                        let d = distance_to_triangle(centre, &part.triangles[t]);
                        if d <= reach && best.is_none_or(|(bd, _)| d < bd) {
                            best = Some((d, t));
                        }
                    }
                    q.face = best.map(|(_, t)| part.face_of[t]);
                }
            });
        }
    });
    quads
}

/// The face of the part nearest to a point (a click on it), if the part has any.
pub fn nearest_face(part: &Part, p: [f64; 3]) -> Option<usize> {
    part.triangles
        .iter()
        .zip(&part.face_of)
        .map(|(t, f)| (distance_to_triangle(p, t), *f))
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, f)| f)
}

/// Runs the study. `progress` gets a fraction (0..1) and what is being done.
pub fn run(part: &Part, study: &Study, progress: &mut dyn FnMut(f32, &str)) -> Result<Report, String> {
    study.material.check()?;
    if part.triangles.is_empty() || part.triangles.len() != part.face_of.len() {
        return Err("the part has no surface".into());
    }
    // supports and loads are matched by which way faces look: a part wound inward is turned round first
    let turned;
    let part = if voxel::mesh_volume(&part.triangles) < 0.0 {
        turned = Part { triangles: part.triangles.iter().map(|t| [t[0], t[2], t[1]]).collect(), face_of: part.face_of.clone() };
        &turned
    } else {
        part
    };
    if study.fixed.is_empty() {
        return Err("nothing holds the part: choose at least one face as a support".into());
    }
    let (lo, hi) = voxel::bounds(&part.triangles);
    let longest = (0..3).map(|a| hi[a] - lo[a]).fold(0.0, f64::max);
    if !(longest > 0.0) {
        return Err("the part has no size".into());
    }
    let divisions = study.divisions.clamp(4, 400);
    let h = longest / divisions as f64;
    progress(0.02, "filling the part with cubes");
    let grid = voxel::voxelize(&part.triangles, h);
    let cubes = grid.solid_count();
    if cubes == 0 {
        return Err("the part is too thin for this many cubes: raise the number of cubes".into());
    }
    if cubes > 3_000_000 {
        return Err(format!("{cubes} cubes is too many: lower the number of cubes"));
    }
    let mut notes = Vec::new();
    if grid.odd_rows > 0 {
        notes.push(format!("the part's surface is not closed everywhere ({} rows of cubes crossed it an odd number of times)", grid.odd_rows));
    }
    let true_volume = voxel::mesh_volume(&part.triangles).abs();
    let volume_error = if true_volume > 0.0 { (grid.volume() - true_volume) / true_volume } else { 0.0 };
    if volume_error.abs() > 0.05 {
        notes.push(format!(
            "the cubes' volume differs from the part's by {:.0}%: walls are thin for this cube size; more cubes give a truer result",
            volume_error * 100.0
        ));
    }
    progress(0.05, "finding the supports and loads");
    let quads = surface(&grid, part);
    let sys0 = solve::System::new(&grid, study.material.youngs_modulus, study.material.poissons_ratio);
    let mut sys = sys0;
    let mut held = 0;
    for q in quads.iter().filter(|q| q.face.is_some_and(|f| study.fixed.contains(&f))) {
        held += 1;
        for n in q.nodes {
            let at = sys.index[n] as usize * 3;
            for d in 0..3 {
                sys.fixed[at + d] = true;
            }
        }
    }
    if held == 0 {
        return Err("the faces chosen as supports are too small for this cube size: raise the number of cubes".into());
    }
    let mut f = vec![0.0; sys.unknowns()];
    for (li, load) in study.loads.iter().enumerate() {
        let on: Vec<&Quad> = quads.iter().filter(|q| q.face.is_some_and(|fc| load.faces.contains(&fc))).collect();
        if on.is_empty() {
            return Err(format!("the faces of load {} are too small for this cube size: raise the number of cubes", li + 1));
        }
        let share = 1.0 / (on.len() as f64 * 4.0);
        for q in on {
            for n in q.nodes {
                let at = sys.index[n] as usize * 3;
                for d in 0..3 {
                    f[at + d] += load.force[d] * share;
                }
            }
        }
    }
    let cube_mass = study.material.density * h.powi(3);
    if let Some(g) = study.gravity {
        for c in 0..grid.solid.len() {
            if grid.solid[c] {
                for n in grid.cube_nodes(c) {
                    let at = sys.index[n] as usize * 3;
                    for d in 0..3 {
                        f[at + d] += cube_mass * g[d] / 8.0;
                    }
                }
            }
        }
    }
    held_everywhere(&grid, &sys)?;
    let unknowns = sys.unknowns();
    progress(0.08, &format!("solving {unknowns} unknowns"));
    let max_iter = (unknowns / 2).clamp(2000, 40_000);
    let solved = sys.solve(&f, 1e-8, max_iter, &mut |it, r| {
        // the residual falls roughly evenly on a log scale: from 1 to 1e-8
        let done = (-r.max(1e-12).log10() / 8.0).clamp(0.0, 1.0) as f32;
        progress(0.08 + 0.88 * done, &format!("solving: step {it}"));
    });
    let u = &solved.displacement;
    // stresses: each cube's at its corners (bubbles included, so a bent cube shows its outer-fibre stress), averaged at each
    // node over the cubes that share it
    let d = study.material.elasticity();
    let element = hex8::Element::new(study.material.poissons_ratio);
    let mut sum = vec![[0.0f64; 6]; sys.nodes.len()];
    let mut count = vec![0u8; sys.nodes.len()];
    for c in 0..grid.solid.len() {
        if !grid.solid[c] {
            continue;
        }
        let strains = element.corner_strains(&sys.cube_displacements(c, u), h);
        for (corner, n) in grid.cube_nodes(c).iter().enumerate() {
            let at = sys.index[*n] as usize;
            for r in 0..6 {
                for k in 0..6 {
                    sum[at][r] += d[r][k] * strains[corner][k];
                }
            }
            count[at] += 1;
        }
    }
    let node_stress: Vec<f64> = sum.iter().zip(&count).map(|(s, c)| hex8::von_mises(&s.map(|v| v / (*c).max(1) as f64))).collect();
    let (mut max_stress, mut max_stress_at) = (0.0f64, [0.0; 3]);
    for (i, vm) in node_stress.iter().enumerate() {
        if *vm > max_stress {
            max_stress = *vm;
            max_stress_at = grid.node_position(sys.nodes[i]);
        }
    }
    let (mut max_displacement, mut max_displacement_at) = (0.0f64, [0.0; 3]);
    for (i, v) in u.iter().enumerate() {
        let l = len(*v);
        if l > max_displacement {
            max_displacement = l;
            max_displacement_at = grid.node_position(sys.nodes[i]);
        }
    }
    // reactions: K u at the supports
    let mut ku = vec![0.0; unknowns];
    let flat: Vec<f64> = u.iter().flatten().copied().collect();
    sys.apply(&flat, &mut ku);
    let mut reaction = [0.0; 3];
    for i in 0..unknowns {
        if sys.fixed[i] {
            reaction[i % 3] += ku[i] - f[i];
        }
    }
    if !solved.converged {
        notes.push(format!("the solver stopped before it settled (after {} steps); the numbers are rough", solved.iterations));
    }
    let surface = quads
        .iter()
        .map(|q| SurfaceQuad {
            corners: q.nodes.map(|n| grid.node_position(n)),
            displacement: q.nodes.map(|n| u[sys.index[n] as usize]),
            stress: q.nodes.map(|n| node_stress[sys.index[n] as usize]),
            face: q.face,
        })
        .collect();
    progress(1.0, "done");
    Ok(Report {
        cubes,
        cube_size: h,
        max_stress,
        max_stress_at,
        max_displacement,
        max_displacement_at,
        safety_factor: if max_stress > 0.0 { study.material.yield_strength / max_stress } else { f64::INFINITY },
        mass: cube_mass * cubes as f64,
        volume_error,
        iterations: solved.iterations,
        converged: solved.converged,
        reaction,
        surface,
        notes,
    })
}

/// Every piece of the part (cubes joined face to face) must touch a support, or it could move freely.
fn held_everywhere(grid: &Grid, sys: &solve::System) -> Result<(), String> {
    let mut seen = vec![false; grid.solid.len()];
    let mut stack = Vec::new();
    for start in 0..grid.solid.len() {
        if !grid.solid[start] || seen[start] {
            continue;
        }
        let mut held = false;
        seen[start] = true;
        stack.push(start);
        while let Some(c) = stack.pop() {
            if !held && grid.cube_nodes(c).iter().any(|n| sys.fixed[sys.index[*n] as usize * 3]) {
                held = true;
            }
            let [i, j, k] = grid.cube_ijk(c);
            for (n, _) in SIDES.iter() {
                let (a, b, e) = (i as isize + n[0] as isize, j as isize + n[1] as isize, k as isize + n[2] as isize);
                if grid.solid_at(a, b, e) {
                    let m = grid.cube(a as usize, b as usize, e as usize);
                    if !seen[m] {
                        seen[m] = true;
                        stack.push(m);
                    }
                }
            }
        }
        if !held {
            return Err("a piece of the part is not held by any support (it would move freely): add a support on it".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use voxel::tests::boxed;

    fn steel() -> Material {
        Material::new("steel", 200.0, 0.3, 250.0, 7850.0)
    }

    fn bar(divisions: usize, load: [f64; 3]) -> (Part, Study) {
        // 100 mm long along x, 10 x 10 mm; held at x = 0 (face 0), pushed at x = 100 mm (face 1)
        let (triangles, face_of) = boxed([0.0, 0.0, 0.0], [0.1, 0.01, 0.01]);
        let mut s = Study::new(steel());
        s.fixed = vec![0];
        s.loads = vec![Load { faces: vec![1], force: load }];
        s.divisions = divisions;
        (Part { triangles, face_of }, s)
    }

    #[test]
    fn a_bar_in_tension_has_stress_f_over_a_and_stretches_fl_over_ae() {
        let (part, study) = bar(40, [1000.0, 0.0, 0.0]);
        let r = run(&part, &study, &mut |_, _| {}).unwrap();
        assert_eq!(r.cubes, 40 * 4 * 4);
        assert!(r.converged);
        // F/A = 1000 / 1e-4 = 10 MPa away from the ends; the held end adds a little (Poisson's contraction is stopped there)
        let sigma = 1000.0 / 1e-4;
        let mid: Vec<f64> = r
            .surface
            .iter()
            .flat_map(|q| q.corners.iter().zip(q.stress).filter(|(p, _)| (p[0] - 0.05).abs() < 1e-9).map(|(_, s)| s).collect::<Vec<_>>())
            .collect();
        assert!(!mid.is_empty());
        for s in mid {
            assert!((s - sigma).abs() / sigma < 0.01, "stress in the middle {s}");
        }
        // stretch FL/AE = 1000 * 0.1 / (1e-4 * 200e9) = 5 µm
        let stretch = 1000.0 * 0.1 / (1e-4 * 200e9);
        assert!((r.max_displacement - stretch).abs() / stretch < 0.03, "stretch {} vs {}", r.max_displacement, stretch);
        // the support takes the load
        assert!((r.reaction[0] + 1000.0).abs() < 1.0 && r.reaction[1].abs() < 1.0, "reaction {:?}", r.reaction);
        assert!(r.volume_error.abs() < 1e-9);
        assert!((r.safety_factor - 250e6 / r.max_stress).abs() < 1e-9);
    }

    #[test]
    fn a_cantilever_bends_as_the_beam_formula_says() {
        // P = 100 N down at the free end: δ = PL³/3EI (+ shear PL/kGA), σ = M c / I at the held end
        let (part, study) = bar(40, [0.0, 0.0, -100.0]);
        let r = run(&part, &study, &mut |_, _| {}).unwrap();
        let (p, l, e, b) = (100.0, 0.1, 200e9, 0.01);
        let i = b * b * b * b / 12.0;
        let g = e / (2.0 * 1.3);
        let bending = p * l * l * l / (3.0 * e * i);
        let shear = p * l / (5.0 / 6.0 * g * b * b);
        let expected = bending + shear;
        assert!(r.converged, "{} steps", r.iterations);
        assert!((r.max_displacement - expected).abs() / expected < 0.05, "tip {} mm vs {} mm", r.max_displacement * 1e3, expected * 1e3);
        // the bending stress halfway along, on the top surface: M c / I with M = P L/2, c = 5 mm
        let at_mid = p * l / 2.0 * 0.005 / i;
        let top: Vec<f64> = r
            .surface
            .iter()
            .filter(|q| q.face == Some(5))
            .flat_map(|q| q.corners.iter().zip(q.stress).filter(|(c, _)| (c[0] - 0.05).abs() < 1e-9).map(|(_, s)| s).collect::<Vec<_>>())
            .collect();
        assert!(!top.is_empty());
        for s in top {
            assert!((s - at_mid).abs() / at_mid < 0.05, "stress halfway {} MPa vs {} MPa", s / 1e6, at_mid / 1e6);
        }
        // the highest stress is at the held end's top and bottom edge: M c / I = 60 MPa, a little more where the support also
        // stops the sideways (Poisson) shrinking
        let root = p * l * 0.005 / i;
        assert!(r.max_stress > 0.95 * root && r.max_stress < 1.4 * root, "peak {} MPa vs {} MPa", r.max_stress / 1e6, root / 1e6);
        assert!(r.max_stress_at[0] < 2.0 * r.cube_size, "at the held end: {:?}", r.max_stress_at);
        assert!((r.reaction[2] - 100.0).abs() < 0.5, "the support holds it up: {:?}", r.reaction);
    }

    #[test]
    fn a_part_wound_inward_and_a_click_on_a_face() {
        let (part, study) = bar(20, [1000.0, 0.0, 0.0]);
        assert_eq!(nearest_face(&part, [0.1, 0.005, 0.004]), Some(1), "a point on the end at x = 100 mm");
        assert_eq!(nearest_face(&part, [0.05, 0.005, 0.0101]), Some(5), "just above the top");
        let inward = Part { triangles: part.triangles.iter().map(|t| [t[0], t[2], t[1]]).collect(), face_of: part.face_of.clone() };
        let (a, b) = (run(&part, &study, &mut |_, _| {}).unwrap(), run(&inward, &study, &mut |_, _| {}).unwrap());
        assert!((a.max_displacement - b.max_displacement).abs() < 1e-15);
    }

    #[test]
    fn finer_cubes_agree() {
        let coarse = run(&bar(20, [0.0, 0.0, -100.0]).0, &bar(20, [0.0, 0.0, -100.0]).1, &mut |_, _| {}).unwrap();
        let fine = run(&bar(40, [0.0, 0.0, -100.0]).0, &bar(40, [0.0, 0.0, -100.0]).1, &mut |_, _| {}).unwrap();
        assert!((coarse.max_displacement - fine.max_displacement).abs() / fine.max_displacement < 0.05);
    }

    #[test]
    fn own_weight_and_what_cannot_be_solved() {
        let (part, mut study) = bar(20, [0.0, 0.0, 0.0]);
        study.loads.clear();
        study.gravity = Some([0.0, 0.0, -9.81]);
        let r = run(&part, &study, &mut |_, _| {}).unwrap();
        let weight = 7850.0 * 1e-5 * 9.81;
        assert!((r.mass - 7850.0 * 1e-5).abs() < 1e-9);
        assert!((r.reaction[2] - weight).abs() / weight < 0.01, "{:?} vs {weight}", r.reaction);
        study.fixed.clear();
        assert!(run(&part, &study, &mut |_, _| {}).unwrap_err().contains("nothing holds"));
        // a second box, apart from the first: not held
        let (mut tris, mut faces) = boxed([0.0, 0.0, 0.0], [0.1, 0.01, 0.01]);
        let (t2, f2) = boxed([0.0, 0.03, 0.0], [0.1, 0.04, 0.01]);
        tris.extend(t2);
        faces.extend(f2.iter().map(|f| f + 6));
        let mut s = Study::new(steel());
        s.fixed = vec![0];
        s.gravity = Some([0.0, 0.0, -9.81]);
        assert!(run(&Part { triangles: tris, face_of: faces }, &s, &mut |_, _| {}).unwrap_err().contains("not held"));
    }
}
