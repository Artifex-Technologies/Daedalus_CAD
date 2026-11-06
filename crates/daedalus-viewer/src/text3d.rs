//! 3D Text: words turned into flat outlines (then faces, then a solid; core `profile`). The letters come from Ubuntu Light,
//! the font egui already carries (Ubuntu Font Licence), read with ttf-parser; curves become short straight pieces.

use ttf_parser::{Face, OutlineBuilder};

/// Pieces per curve in a letter: enough to look round at the sizes text is drawn.
const CURVE_STEPS: usize = 5;

#[derive(Default)]
struct Outline {
    rings: Vec<Vec<[f32; 2]>>,
    now: Vec<[f32; 2]>,
    scale: f32,
    at: [f32; 2],
}

impl Outline {
    fn point(&self, x: f32, y: f32) -> [f32; 2] {
        [self.at[0] + x * self.scale, self.at[1] + y * self.scale]
    }

    fn last(&self) -> [f32; 2] {
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
            let t = i as f32 / CURVE_STEPS as f32;
            let u = 1.0 - t;
            self.now.push([u * u * p0[0] + 2.0 * u * t * p1[0] + t * t * p2[0], u * u * p0[1] + 2.0 * u * t * p1[1] + t * t * p2[1]]);
        }
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let (p0, p1, p2, p3) = (self.last(), self.point(x1, y1), self.point(x2, y2), self.point(x, y));
        for i in 1..=CURVE_STEPS + 1 {
            let t = i as f32 / (CURVE_STEPS + 1) as f32;
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
/// origin and further lines below it. None when the font cannot be read (it is built in, so never expected).
pub fn outlines(text: &str, height: f32) -> Option<Vec<Vec<Vec<[f32; 2]>>>> {
    let face = Face::parse(epaint_default_fonts::UBUNTU_LIGHT, 0).ok()?;
    let cap = face.capital_height().filter(|&c| c > 0).unwrap_or(face.ascender()) as f32;
    let scale = height / cap;
    let line = (face.ascender() - face.descender() + face.line_gap()) as f32 * scale;
    let mut out = Outline { scale, ..Default::default() };
    let mut letters = Vec::new();
    for (row, words) in text.lines().enumerate() {
        let mut x = 0.0;
        let y = -(row as f32) * line;
        for ch in words.chars() {
            let Some(g) = face.glyph_index(ch).or_else(|| face.glyph_index('?')) else { continue };
            out.at = [x, y];
            face.outline_glyph(g, &mut out);
            out.finish();
            let rings = std::mem::take(&mut out.rings);
            if !rings.is_empty() {
                letters.push(rings);
            }
            x += face.glyph_hor_advance(g).unwrap_or(0) as f32 * scale;
        }
    }
    Some(letters)
}

/// The solid (or, with no depth, flat) geometry of `text` laid on `plane`: each letter built on its own (letters do not
/// touch, and it keeps the work small), then put together.
pub fn geometry(text: &str, height: f32, depth: Option<f32>, plane: &daedalus_core::Plane) -> Result<daedalus_core::Mesh, String> {
    let letters = outlines(text, height).ok_or("the built-in font could not be read")?;
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
        let o = outlines("O", 1.0).unwrap().concat();
        assert_eq!(o.len(), 2, "an O is an outside and a hole");
        let top = o.iter().flatten().map(|p| p[1]).fold(f32::MIN, f32::max);
        assert!(top > 0.95 && top < 1.1, "a capital about the height asked for ({top})");
        assert_eq!(outlines("B", 1.0).unwrap().concat().len(), 3);
        assert_eq!(outlines("il", 1.0).unwrap().concat().len(), 3, "the dot of the i is its own outline");
        assert!(outlines(" ", 1.0).unwrap().is_empty());
        // a second line sits below the first
        let two = outlines("I\nI", 1.0).unwrap().concat();
        assert!(two[1].iter().all(|p| p[1] < 0.0));
    }

    #[test]
    fn words_become_a_closed_solid() {
        let solid = geometry("Daedalus 3D", 0.2, Some(0.05), &daedalus_core::Plane::ground()).unwrap();
        let h = daedalus_core::HalfEdgeMesh::from_mesh(&solid).unwrap();
        assert!(h.is_closed());
        assert!(h.signed_volume() > 0.0);
    }
}
