//! The window's own frame on Linux. Many Wayland compositors (Hyprland among them) draw no title
//! bar or border, so a window that relies on them cannot be moved or resized with the mouse. On
//! Linux Daedalus asks for no system frame and draws its own: the menu bar is the title bar
//! (ui.rs, `title_bar`), and the outer few pixels of the window resize it. On Windows the system
//! frame stays.

use winit::window::ResizeDirection;

/// Whether the window draws its own title bar and resize edges.
pub const OWN_FRAME: bool = cfg!(target_os = "linux");

/// Width of the invisible resize handles along the window's edges, in logical pixels.
const EDGE: f64 = 6.0;

/// The resize handle under the cursor, if any: corners first, then sides. `cursor` and `size` are
/// physical pixels; `scale` is the window's scale factor.
pub fn edge(cursor: (f32, f32), size: (u32, u32), scale: f64) -> Option<ResizeDirection> {
    let band = EDGE * scale;
    let (x, y) = (cursor.0 as f64, cursor.1 as f64);
    let (w, h) = (size.0 as f64, size.1 as f64);
    if x < 0.0 || y < 0.0 || x > w || y > h {
        return None;
    }
    let (l, r, t, b) = (x < band, x > w - band, y < band, y > h - band);
    Some(match (l, r, t, b) {
        (true, _, true, _) => ResizeDirection::NorthWest,
        (_, true, true, _) => ResizeDirection::NorthEast,
        (true, _, _, true) => ResizeDirection::SouthWest,
        (_, true, _, true) => ResizeDirection::SouthEast,
        (true, _, _, _) => ResizeDirection::West,
        (_, true, _, _) => ResizeDirection::East,
        (_, _, true, _) => ResizeDirection::North,
        (_, _, _, true) => ResizeDirection::South,
        _ => return None,
    })
}

/// Whether a window of this size fills a screen of that size, allowing for a panel or bar along one
/// side (a maximized window leaves room for it).
pub fn fills(w: u32, h: u32, screen_w: u32, screen_h: u32) -> bool {
    w + 160 >= screen_w && h + 160 >= screen_h
}

/// The pointer shape for a resize handle.
pub fn cursor(d: ResizeDirection) -> egui::CursorIcon {
    match d {
        ResizeDirection::NorthWest => egui::CursorIcon::ResizeNorthWest,
        ResizeDirection::NorthEast => egui::CursorIcon::ResizeNorthEast,
        ResizeDirection::SouthWest => egui::CursorIcon::ResizeSouthWest,
        ResizeDirection::SouthEast => egui::CursorIcon::ResizeSouthEast,
        ResizeDirection::West => egui::CursorIcon::ResizeWest,
        ResizeDirection::East => egui::CursorIcon::ResizeEast,
        ResizeDirection::North => egui::CursorIcon::ResizeNorth,
        ResizeDirection::South => egui::CursorIcon::ResizeSouth,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edges_and_corners() {
        let size = (1000, 800);
        assert!(matches!(edge((3.0, 400.0), size, 1.0), Some(ResizeDirection::West)));
        assert!(matches!(edge((997.0, 400.0), size, 1.0), Some(ResizeDirection::East)));
        assert!(matches!(edge((500.0, 2.0), size, 1.0), Some(ResizeDirection::North)));
        assert!(matches!(edge((500.0, 799.0), size, 1.0), Some(ResizeDirection::South)));
        assert!(matches!(edge((2.0, 2.0), size, 1.0), Some(ResizeDirection::NorthWest)));
        assert!(matches!(edge((998.0, 798.0), size, 1.0), Some(ResizeDirection::SouthEast)));
        assert!(edge((500.0, 400.0), size, 1.0).is_none());
        assert!(edge((-1.0, 400.0), size, 1.0).is_none());
    }

    #[test]
    fn a_floating_window_does_not_fill_the_screen() {
        assert!(!fills(1280, 800, 3440, 1440));
        assert!(fills(3440, 1392, 3440, 1440)); // maximized next to a 48 px panel
        assert!(fills(1920, 1080, 1920, 1080));
    }

    #[test]
    fn the_band_grows_with_the_scale() {
        // 9 px in from the left is inside the handle at scale 2 (12 px), not at scale 1 (6 px)
        assert!(edge((9.0, 400.0), (1000, 800), 1.0).is_none());
        assert!(matches!(edge((9.0, 400.0), (1000, 800), 2.0), Some(ResizeDirection::West)));
    }
}
