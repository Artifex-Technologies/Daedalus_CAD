//! Opens models the way Daedalus does and says what it found, for checking a batch of files (SketchUp conversions, say):
//! groups and components, faces once every instance is put in place, the size, and whether the model survives a save to
//! `.daedalus` and back. Nothing is written.
//!
//!   cargo run -p daedalus-io --example open_check -- FILE...
use std::path::Path;
use std::time::Instant;

fn main() {
    let mut failed = 0;
    for arg in std::env::args().skip(1) {
        let path = Path::new(&arg);
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or(arg.clone());
        let t = Instant::now();
        match daedalus_io::open_scene(path) {
            Err(e) => {
                failed += 1;
                println!("FAIL  {name}: {e}");
            }
            Ok(scene) => {
                let flat = scene.flatten();
                let size = flat.bounds().map(|(lo, hi)| [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]]).unwrap_or([0.0; 3]);
                let groups = scene.defs.iter().filter(|d| !d.component).count();
                let back = daedalus_io::native::read_scene(&daedalus_io::native::write_scene(&scene));
                let round = match back {
                    Ok(b) if b.flatten().faces.len() == flat.faces.len() && b.defs.len() == scene.defs.len() => "saves and reopens the same".to_string(),
                    Ok(b) => format!("SAVE DIFFERS ({} faces, {} definitions back)", b.flatten().faces.len(), b.defs.len()),
                    Err(e) => format!("SAVE FAILS: {e}"),
                };
                if round.starts_with("SAVE") {
                    failed += 1;
                }
                println!(
                    "ok    {name}: {} groups, {} components, {} top-level instances, {} faces placed, {:.2} x {:.2} x {:.2} m, {:.1} s; {round}",
                    groups,
                    scene.defs.len() - groups,
                    scene.root.instances.len(),
                    flat.faces.len(),
                    size[0],
                    size[1],
                    size[2],
                    t.elapsed().as_secs_f64()
                );
            }
        }
    }
    if failed > 0 {
        println!("{failed} file(s) with a problem");
        std::process::exit(1);
    }
}
