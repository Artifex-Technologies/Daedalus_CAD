use daedalus_core::inspect;
use daedalus_core::Mesh;
use std::path::{Path, PathBuf};
use std::process::exit;

const USAGE: &str = "\
usage:
  daedalus-app <model>                         print mesh statistics
  daedalus-app inspect <model>                 check whether the model is a valid solid
  daedalus-app inspect <model> --fix <out>     repair it and write the result

  daedalus-app flatten <out.dxf> --thickness T --k K --width W [--units mm|in] <flange> [<angle>:<radius> <flange>]...
      sheet-metal flat pattern: outside flange lengths with bends between them,
      e.g. an L-bracket 50 x 30 with a 90 degree, 2.0 radius bend:
        flatten bracket.dxf --thickness 2 --k 0.44 --width 40 50 90:2 30
      angle is negative for a bend down; K-factor and radius come from your fabricator

formats: stl, obj, dxf, dae";

fn load(path: &Path) -> Mesh {
    daedalus_io::import_mesh(path).unwrap_or_else(|e| {
        eprintln!("error: {}: {e}", path.display());
        exit(1)
    })
}

fn print_report(report: &inspect::Report) {
    println!("{}", report.summary());
}

fn inspect_command(args: &[String]) {
    let Some(model) = args.first() else {
        eprintln!("{USAGE}");
        exit(2)
    };
    let mesh = load(&PathBuf::from(model));
    println!("{model}: {} vertices, {} faces", mesh.positions.len(), mesh.faces.len());
    let report = inspect::inspect(&mesh, mesh.tolerance() * 10.0);
    print_report(&report);

    match args.get(1).map(String::as_str) {
        None => exit(if report.is_solid() { 0 } else { 1 }),
        Some("--fix") => {
            let Some(out) = args.get(2) else {
                eprintln!("{USAGE}");
                exit(2)
            };
            let (fixed, summary) = inspect::repair(&mesh, mesh.tolerance() * 10.0);
            println!("\nrepair: {summary:#?}");
            println!("after repair:");
            let after = inspect::inspect(&fixed, fixed.tolerance() * 10.0);
            print_report(&after);
            if let Err(e) = daedalus_io::export_mesh(&fixed, Path::new(out)) {
                eprintln!("error: could not write {out}: {e}");
                exit(1);
            }
            println!("wrote {out}");
            exit(if after.is_solid() { 0 } else { 1 });
        }
        Some(other) => {
            eprintln!("unknown option {other}\n{USAGE}");
            exit(2)
        }
    }
}

fn fail(msg: &str) -> ! {
    eprintln!("error: {msg}\n\n{USAGE}");
    exit(2)
}

fn number(s: &str, what: &str) -> f64 {
    s.parse().unwrap_or_else(|_| fail(&format!("{what}: '{s}' is not a number")))
}

fn flatten_command(args: &[String]) {
    use daedalus_core::sheetmetal::{flat_pattern, Bend, Material, Profile};
    use daedalus_io::dxf2d::{flat_pattern_dxf, Units};

    let Some(out) = args.first() else { fail("missing output file") };
    let (mut thickness, mut k, mut width) = (None, None, None);
    let mut units = Units::Millimeters;
    let mut profile: Vec<&str> = Vec::new();

    let mut rest = args[1..].iter();
    while let Some(a) = rest.next() {
        let mut value = |name: &str| rest.next().unwrap_or_else(|| fail(&format!("{name} needs a value"))).as_str();
        match a.as_str() {
            "--thickness" => thickness = Some(number(value("--thickness"), "--thickness")),
            "--k" => k = Some(number(value("--k"), "--k")),
            "--width" => width = Some(number(value("--width"), "--width")),
            "--units" => {
                units = match value("--units") {
                    "mm" => Units::Millimeters,
                    "in" => Units::Inches,
                    other => fail(&format!("--units must be mm or in, not '{other}'")),
                }
            }
            other => profile.push(other),
        }
    }
    let material = Material {
        thickness: thickness.unwrap_or_else(|| fail("--thickness is required")),
        k_factor: k.unwrap_or_else(|| fail("--k is required")),
    };
    let width = width.unwrap_or_else(|| fail("--width is required"));

    // Profile tokens alternate: flange, bend, flange, bend, ..., flange
    let (mut flanges, mut bends) = (Vec::new(), Vec::new());
    for (i, tok) in profile.iter().enumerate() {
        if i % 2 == 0 {
            flanges.push(number(tok, "flange length"));
        } else {
            let Some((angle, radius)) = tok.split_once(':') else { fail(&format!("bend '{tok}' must look like ANGLE:RADIUS, e.g. 90:2")) };
            bends.push(Bend { angle_deg: number(angle, "bend angle"), inside_radius: number(radius, "bend radius") });
        }
    }

    let flat = flat_pattern(&Profile { flanges, bends }, material, width).unwrap_or_else(|e| {
        eprintln!("error: {e}");
        exit(1)
    });
    println!("flat length: {:.4}   width: {:.4}", flat.length, flat.width);
    for (i, b) in flat.bend_lines.iter().enumerate() {
        let dir = if b.angle_deg >= 0.0 { "up" } else { "down" };
        println!(
            "bend {}: line at {:.4}, {:.1} deg {dir}, radius {}, allowance {:.4}",
            i + 1,
            b.position,
            b.angle_deg.abs(),
            b.inside_radius,
            b.allowance
        );
    }
    if let Err(e) = std::fs::write(out, flat_pattern_dxf(&flat, units)) {
        eprintln!("error: could not write {out}: {e}");
        exit(1);
    }
    println!("wrote {out}");
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None | Some("-h") | Some("--help") => {
            eprintln!("{USAGE}");
            exit(2)
        }
        Some("inspect") => inspect_command(&args[1..]),
        Some("flatten") => flatten_command(&args[1..]),
        Some(path) => {
            let mesh = load(&PathBuf::from(path));
            println!("vertices: {}", mesh.positions.len());
            println!("faces:    {}", mesh.faces.len());
            println!("bounds:   {:?}", mesh.bounds());
        }
    }
}
