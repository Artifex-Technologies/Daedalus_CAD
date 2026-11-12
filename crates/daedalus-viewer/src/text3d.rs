//! 3D Text: words turned into flat outlines (then faces, then a solid; core `profile`). The letters come from a font on the
//! system (TrueType or OpenType, found in the usual font folders), or Ubuntu Light, the font egui already carries (Ubuntu
//! Font Licence); read with ttf-parser; curves become short straight pieces.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use ttf_parser::{Face, OutlineBuilder};

/// A font 3D Text can use: its name, and its file (None: the built-in Ubuntu Light).
#[derive(Debug, Clone, PartialEq)]
pub struct Font {
    pub name: String,
    pub path: Option<PathBuf>,
}

/// The built-in font first, then every TrueType or OpenType font found on the system, by name (read once).
pub fn fonts() -> &'static [Font] {
    static ALL: OnceLock<Vec<Font>> = OnceLock::new();
    ALL.get_or_init(|| {
        let mut dirs: Vec<PathBuf> = vec!["/usr/share/fonts".into(), "/usr/local/share/fonts".into()];
        if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
            dirs.push(home.join(".local/share/fonts"));
            dirs.push(home.join(".fonts"));
        }
        if let Some(win) = std::env::var_os("WINDIR").map(PathBuf::from) {
            dirs.push(win.join("Fonts"));
        }
        let mut found: Vec<Font> = Vec::new();
        let mut todo = dirs;
        while let Some(d) = todo.pop() {
            let Ok(entries) = std::fs::read_dir(&d) else { continue };
            for e in entries.flatten() {
                let path = e.path();
                if path.is_dir() {
                    todo.push(path);
                    continue;
                }
                let ext = path.extension().and_then(|x| x.to_str()).map(str::to_ascii_lowercase);
                if !matches!(ext.as_deref(), Some("ttf" | "otf")) {
                    continue;
                }
                let Ok(bytes) = std::fs::read(&path) else { continue };
                let Ok(face) = Face::parse(&bytes, 0) else { continue };
                if let Some(name) = full_name(&face) {
                    found.push(Font { name, path: Some(path) });
                }
            }
        }
        found.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        found.dedup_by(|a, b| a.name == b.name);
        std::iter::once(Font { name: "Ubuntu Light (built in)".into(), path: None }).chain(found).collect()
    })
}

/// A font's full name ("DejaVu Sans Bold"), from its name table.
fn full_name(face: &Face) -> Option<String> {
    let names = face.names();
    let get = |id: u16| names.into_iter().filter(|n| n.name_id == id).find_map(|n| n.to_string()).filter(|s| !s.trim().is_empty());
    get(ttf_parser::name_id::FULL_NAME).or_else(|| get(ttf_parser::name_id::FAMILY))
}

/// The font's bytes: the file, or the built-in font.
fn font_bytes(path: Option<&Path>) -> Option<std::borrow::Cow<'static, [u8]>> {
    match path {
        None => Some(std::borrow::Cow::Borrowed(epaint_default_fonts::UBUNTU_LIGHT)),
        Some(p) => std::fs::read(p).ok().map(std::borrow::Cow::Owned),
    }
}

/// Pieces per curve in a letter: enough to look round at the sizes text is drawn.
const CURVE_STEPS: usize = 5;

#[derive(Default)]
struct Outline {
    rings: Vec<Vec<[f64; 2]>>,
    now: Vec<[f64; 2]>,
    scale: f64,
    at: [f64; 2],
}

impl Outline {
    fn point(&self, x: f32, y: f32) -> [f64; 2] {
        [self.at[0] + x as f64 * self.scale, self.at[1] + y as f64 * self.scale]
    }

    fn last(&self) -> [f64; 2] {
        *self.now.last().unwrap_or(&self.at)
    }

    fn finish(&mut self) {
        let mut r = std::mem::take(&mut self.now);
        if r.len() > 1 && r.first() == r.last() {
            r.pop();
        }
        // pieces too short to keep (a curve that hardly bends)
        r.dedup_by(|a, b| (a[0] - b[0]).abs() < 1e-7 && (a[1] - b[1]).abs() < 1e-7);
        if r.len() >= 3 {
            self.rings.push(r);
        }
    }
}

impl OutlineBuilder for Outline {
    fn move_to(&mut self, x: f32, y: f32) {
        self.finish();
        self.now.push(self.point(x, y));
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let p = self.point(x, y);
        self.now.push(p);
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let (p0, p1, p2) = (self.last(), self.point(x1, y1), self.point(x, y));
        for i in 1..=CURVE_STEPS {
            let t = i as f64 / CURVE_STEPS as f64;
            let u = 1.0 - t;
            self.now.push([u * u * p0[0] + 2.0 * u * t * p1[0] + t * t * p2[0], u * u * p0[1] + 2.0 * u * t * p1[1] + t * t * p2[1]]);
        }
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let (p0, p1, p2, p3) = (self.last(), self.point(x1, y1), self.point(x2, y2), self.point(x, y));
        for i in 1..=CURVE_STEPS + 1 {
            let t = i as f64 / (CURVE_STEPS + 1) as f64;
            let u = 1.0 - t;
            let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            self.now.push([a * p0[0] + b * p1[0] + c * p2[0] + d * p3[0], a * p0[1] + b * p1[1] + c * p2[1] + d * p3[1]]);
        }
    }

    fn close(&mut self) {
        self.finish();
    }
}

/// The outlines of `text`, one list per letter, capital letters `height` tall, the first line's baseline along x from the
/// origin and further lines below it, in the font at `font` (None: the built-in one). None when the font cannot be read.
pub fn outlines(text: &str, height: f64, font: Option<&Path>) -> Option<Vec<Vec<Vec<[f64; 2]>>>> {
    let bytes = font_bytes(font)?;
    let face = Face::parse(&bytes, 0).ok()?;
    let cap = face.capital_height().filter(|&c| c > 0).unwrap_or(face.ascender()) as f64;
    let scale = height / cap;
    let line = (face.ascender() - face.descender() + face.line_gap()) as f64 * scale;
    let mut out = Outline { scale, ..Default::default() };
    let mut letters = Vec::new();
    for (row, words) in text.lines().enumerate() {
        let mut x = 0.0;
        let y = -(row as f64) * line;
        for ch in words.chars() {
            let Some(g) = face.glyph_index(ch).or_else(|| face.glyph_index('?')) else { continue };
            out.at = [x, y];
            face.outline_glyph(g, &mut out);
            out.finish();
            let rings = std::mem::take(&mut out.rings);
            if !rings.is_empty() {
                letters.push(rings);
            }
            x += face.glyph_hor_advance(g).unwrap_or(0) as f64 * scale;
        }
    }
    Some(letters)
}

/// The solid (or, with no depth, flat) geometry of `text` laid on `plane`: each letter built on its own (letters do not
/// touch, and it keeps the work small), then put together.
pub fn geometry(
    text: &str,
    height: f64,
    depth: Option<f64>,
    plane: &daedalus_core::Plane,
    font: Option<&Path>,
) -> Result<daedalus_core::Mesh, String> {
    let letters = outlines(text, height, font).ok_or("the font could not be read")?;
    let mut all = daedalus_core::Mesh::new();
    for rings in &letters {
        let cap = daedalus_core::profile::faces(rings, plane).map_err(|e| e.to_string())?;
        let part = match depth {
            Some(d) if d != 0.0 => daedalus_core::profile::extrude(&cap, plane.normal.map(|c| c * d), 0.5),
            _ => cap,
        };
        all.append(&part);
    }
    if all.faces.is_empty() {
        return Err("there are no letters to make".into());
    }
    Ok(all)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_have_their_holes() {
        let o = outlines("O", 1.0, None).unwrap().concat();
        assert_eq!(o.len(), 2, "an O is an outside and a hole");
        let top = o.iter().flatten().map(|p| p[1]).fold(f64::MIN, f64::max);
        assert!(top > 0.95 && top < 1.1, "a capital about the height asked for ({top})");
        assert_eq!(outlines("B", 1.0, None).unwrap().concat().len(), 3);
        assert_eq!(outlines("il", 1.0, None).unwrap().concat().len(), 3, "the dot of the i is its own outline");
        assert!(outlines(" ", 1.0, None).unwrap().is_empty());
        // a second line sits below the first
        let two = outlines("I\nI", 1.0, None).unwrap().concat();
        assert!(two[1].iter().all(|p| p[1] < 0.0));
    }

    #[test]
    fn words_become_a_closed_solid() {
        let solid = geometry("Daedalus 3D", 0.2, Some(0.05), &daedalus_core::Plane::ground(), None).unwrap();
        let h = daedalus_core::HalfEdgeMesh::from_mesh(&solid).unwrap();
        assert!(h.is_closed());
        assert!(h.signed_volume() > 0.0);
    }

    #[test]
    fn system_fonts_are_listed_after_the_built_in_one() {
        let all = fonts();
        assert_eq!(all[0].path, None);
        // any system font found can be read and makes letters
        if let Some(f) = all.get(1) {
            assert!(outlines("A", 1.0, f.path.as_deref()).is_some(), "{}", f.name);
        }
    }
}
