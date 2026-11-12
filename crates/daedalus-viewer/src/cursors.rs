//! The tools' mouse cursors, as SketchUp has them: over the model, the cursor shows the tool in use. Each is drawn from the
//! tool's own toolbar icon (icons.rs): a small crosshair marks the point that is clicked, the icon sits beside it. The
//! navigation tools show their icon alone, the point in its middle. Drawn into an image here (egui's tessellator, then a
//! small rasteriser), in dark ink with a white rim so it reads on any background.

use crate::icons::{self, Icon};
use crate::tools::Tool;
use egui::epaint::{Mesh, TessellationOptions, Tessellator};
use egui::{pos2, Color32, Rect, Shape, Stroke};

/// A cursor's picture: RGBA rows (square), and the point that clicks (from the top left).
pub struct Image {
    pub rgba: Vec<u8>,
    pub hotspot: (u16, u16),
}

/// Whether a tool has a cursor of its own (Select keeps the system arrow).
pub fn has_cursor(t: Tool) -> bool {
    t != Tool::Select
}

/// The cursor for `tool`, `size` pixels square.
pub fn image(tool: Tool, size: u32) -> Image {
    let s = size as f32;
    let ink = Color32::from_gray(25);
    let navigates = tool.navigates() || tool == Tool::PositionCamera;
    let mut shapes = Vec::new();
    let hotspot = if navigates {
        // the icon alone, filling the square; the point is its middle
        shapes.extend(icons::shapes(Rect::from_min_max(pos2(s * 0.08, s * 0.08), pos2(s * 0.92, s * 0.92)), Icon::Tool(tool), false, ink));
        ((s * 0.5) as u16, (s * 0.5) as u16)
    } else {
        // a crosshair at the top left, the icon below and to its right
        let c = pos2(s * 0.2, s * 0.2);
        let r = s * 0.15;
        let st = Stroke::new((s / 32.0).max(1.0), ink);
        shapes.push(Shape::line_segment([pos2(c.x - r, c.y), pos2(c.x + r, c.y)], st));
        shapes.push(Shape::line_segment([pos2(c.x, c.y - r), pos2(c.x, c.y + r)], st));
        shapes.extend(icons::shapes(Rect::from_min_max(pos2(s * 0.36, s * 0.36), pos2(s, s)), Icon::Tool(tool), false, ink));
        (c.x as u16, c.y as u16)
    };
    let ink_layer = raster(&shapes, size);
    // the white rim: everything within a pixel and a half of the ink, under it
    let n = size as usize;
    let reach = (s / 21.0).max(1.0) as i32 + 1;
    let alpha = |x: i32, y: i32| -> u8 {
        if x < 0 || y < 0 || x >= n as i32 || y >= n as i32 {
            0
        } else {
            ink_layer[(y as usize * n + x as usize) * 4 + 3]
        }
    };
    let mut out = vec![0u8; n * n * 4];
    for y in 0..n as i32 {
        for x in 0..n as i32 {
            let mut rim = 0u8;
            for dy in -reach..=reach {
                for dx in -reach..=reach {
                    if dx * dx + dy * dy <= reach * reach {
                        rim = rim.max(alpha(x + dx, y + dy));
                    }
                }
            }
            let i = (y as usize * n + x as usize) * 4;
            // premultiplied ink over white rim
            let a = ink_layer[i + 3] as f32 / 255.0;
            let rim_a = (rim as f32 / 255.0).min(1.0) * 0.95;
            let out_a = a + rim_a * (1.0 - a);
            for k in 0..3 {
                let c = ink_layer[i + k] as f32 / 255.0 + 1.0 * rim_a * (1.0 - a);
                // straight (not premultiplied) colour for the cursor image
                out[i + k] = if out_a > 0.0 { ((c / out_a).min(1.0) * 255.0) as u8 } else { 0 };
            }
            out[i + 3] = (out_a * 255.0) as u8;
        }
    }
    Image { rgba: out, hotspot }
}

/// The shapes drawn into a `size`-square premultiplied RGBA image (egui's tessellation, its feathered edges blended).
fn raster(shapes: &[Shape], size: u32) -> Vec<u8> {
    let mut tess = Tessellator::new(1.0, TessellationOptions::default(), [1, 1], Vec::new());
    let mut mesh = Mesh::default();
    for s in shapes {
        tess.tessellate_shape(s.clone(), &mut mesh);
    }
    let n = size as usize;
    let mut px = vec![[0.0f32; 4]; n * n];
    for tri in mesh.indices.chunks_exact(3) {
        let v = [mesh.vertices[tri[0] as usize], mesh.vertices[tri[1] as usize], mesh.vertices[tri[2] as usize]];
        let (a, b, c) = (v[0].pos, v[1].pos, v[2].pos);
        let area = (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
        if area.abs() < 1e-9 {
            continue;
        }
        let (x0, x1) = (a.x.min(b.x).min(c.x).floor().max(0.0) as usize, (a.x.max(b.x).max(c.x).ceil() as usize).min(n));
        let (y0, y1) = (a.y.min(b.y).min(c.y).floor().max(0.0) as usize, (a.y.max(b.y).max(c.y).ceil() as usize).min(n));
        for y in y0..y1 {
            for x in x0..x1 {
                let p = pos2(x as f32 + 0.5, y as f32 + 0.5);
                let w0 = ((b.x - p.x) * (c.y - p.y) - (b.y - p.y) * (c.x - p.x)) / area;
                let w1 = ((c.x - p.x) * (a.y - p.y) - (c.y - p.y) * (a.x - p.x)) / area;
                let w2 = 1.0 - w0 - w1;
                if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                    continue;
                }
                // premultiplied colour of the point, laid over what is there
                let col: [f32; 4] =
                    std::array::from_fn(|k| (w0 * v[0].color[k] as f32 + w1 * v[1].color[k] as f32 + w2 * v[2].color[k] as f32) / 255.0);
                let d = &mut px[y * n + x];
                for k in 0..4 {
                    d[k] = col[k] + d[k] * (1.0 - col[3]);
                }
            }
        }
    }
    px.iter().flat_map(|c| c.map(|x| (x.clamp(0.0, 1.0) * 255.0) as u8)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_but_select_has_a_cursor_with_ink_and_a_rim() {
        for t in Tool::ALL {
            if !has_cursor(t) {
                continue;
            }
            let img = image(t, 32);
            assert_eq!(img.rgba.len(), 32 * 32 * 4);
            let dark = img.rgba.chunks(4).filter(|p| p[3] > 200 && p[0] < 80).count();
            let light = img.rgba.chunks(4).filter(|p| p[3] > 200 && p[0] > 200).count();
            assert!(dark > 10 && light > 10, "{}: {dark} dark, {light} light pixels", t.label());
            assert!(img.hotspot.0 < 32 && img.hotspot.1 < 32);
        }
    }

    /// All the cursors on one sheet, on grey and on white, for a look: CURSOR_SHEET=out.png cargo test sheet -- --ignored
    #[test]
    #[ignore]
    fn sheet() {
        let Some(path) = std::env::var_os("CURSOR_SHEET") else { return };
        let tools: Vec<Tool> = Tool::ALL.into_iter().filter(|&t| has_cursor(t)).collect();
        let (size, cols) = (48u32, 11u32);
        let rows = (tools.len() as u32).div_ceil(cols);
        let mut sheet = image::RgbaImage::new(cols * size, rows * size * 2);
        for (k, &t) in tools.iter().enumerate() {
            let img = super::image(t, size);
            let (cx, cy) = (k as u32 % cols * size, k as u32 / cols * size);
            for half in 0..2u32 {
                let bg: [u8; 3] = if half == 0 { [150, 150, 155] } else { [255, 255, 255] };
                for y in 0..size {
                    for x in 0..size {
                        let i = ((y * size + x) * 4) as usize;
                        let a = img.rgba[i + 3] as f32 / 255.0;
                        let px: [u8; 4] =
                            std::array::from_fn(
                                |c| if c == 3 { 255 } else { (img.rgba[i + c] as f32 * a + bg[c] as f32 * (1.0 - a)) as u8 },
                            );
                        sheet.put_pixel(cx + x, cy + y + half * rows * size, image::Rgba(px));
                    }
                }
                // the hotspot, a red dot
                sheet.put_pixel(cx + img.hotspot.0 as u32, cy + img.hotspot.1 as u32 + half * rows * size, image::Rgba([255, 0, 0, 255]));
            }
        }
        sheet.save(path).unwrap();
    }
}
