use glam::{Mat4, Vec3};
use std::f32::consts::{FRAC_PI_2, PI};

const FOV_Y: f32 = 45.0 * std::f32::consts::PI / 180.0;

/// Orbit camera, Z-up, looking at `target`. `ortho` switches to parallel
/// projection (Camera > Parallel Projection in SketchUp). Copied whole for
/// Camera > Previous.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    pub target: Vec3,
    pub distance: f32,
    pub yaw: f32,
    pub pitch: f32,
    pub ortho: bool,
}

impl Camera {
    /// Frames a bounding box from the default iso direction.
    pub fn fit(min: Vec3, max: Vec3) -> Self {
        let mut c = Camera { target: Vec3::ZERO, distance: 1.0, yaw: 0.0, pitch: 0.0, ortho: false };
        c.iso();
        c.zoom_extents(min, max);
        c
    }

    /// Keeps the view direction and frames a bounding box.
    pub fn zoom_extents(&mut self, min: Vec3, max: Vec3) {
        self.target = (min + max) * 0.5;
        let radius = ((max - min).length() * 0.5).max(1e-3);
        self.distance = radius * 2.8;
    }

    /// Plan view: looking straight down with +Y up the screen.
    pub fn top(&mut self) {
        self.yaw = -FRAC_PI_2;
        self.pitch = FRAC_PI_2;
    }

    /// The standard views: looking at the model from the front (toward +Y), the back, the right (toward -X) or the left.
    pub fn front(&mut self) {
        self.yaw = -FRAC_PI_2;
        self.pitch = 0.0;
    }

    pub fn back(&mut self) {
        self.yaw = FRAC_PI_2;
        self.pitch = 0.0;
    }

    pub fn right_side(&mut self) {
        self.yaw = 0.0;
        self.pitch = 0.0;
    }

    pub fn left_side(&mut self) {
        self.yaw = PI;
        self.pitch = 0.0;
    }

    /// Unit vector from the target toward the eye.
    fn back_dir(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        Vec3::new(cp * cy, cp * sy, sp)
    }

    /// Look Around: turns the view about the eye, which stays where it is.
    pub fn look_around(&mut self, dx: f32, dy: f32) {
        let eye = self.eye();
        self.yaw += dx * 0.004;
        self.pitch = (self.pitch + dy * 0.004).clamp(-FRAC_PI_2 + 0.01, FRAC_PI_2 - 0.01);
        self.target = eye - self.back_dir() * self.distance;
    }

    /// Walk: moves the eye forward (positive) or back along the ground, keeping its height.
    pub fn walk(&mut self, forward: f32) {
        let (sy, cy) = self.yaw.sin_cos();
        self.target -= Vec3::new(cy, sy, 0.0) * forward;
    }

    /// Position Camera: the eye at `height` above `p`, looking level in the current direction.
    pub fn stand_at(&mut self, p: Vec3, height: f32) {
        self.ortho = false;
        self.pitch = 0.0;
        self.distance = 2.0_f32.max(height);
        let eye = p + Vec3::Z * height;
        self.target = eye - self.back_dir() * self.distance;
    }

    /// Zoom Window: frames the screen rectangle from (x0, y0) to (x1, y1) of a w x h viewport.
    pub fn zoom_window(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, w: f32, h: f32) {
        let (rw, rh) = ((x1 - x0).abs(), (y1 - y0).abs());
        if rw < 4.0 || rh < 4.0 {
            return;
        }
        // the window's centre, at the target's depth, becomes the new target
        let (o, d) = self.ray((x0 + x1) * 0.5, (y0 + y1) * 0.5, w, h);
        let n = self.back_dir();
        let denom = d.dot(n);
        if denom.abs() > 1e-6 {
            let t = (self.target - o).dot(n) / denom;
            self.target = o + d * t;
        }
        self.distance = (self.distance * (rw / w).max(rh / h)).max(1e-3);
    }

    pub fn iso(&mut self) {
        self.yaw = -0.8;
        self.pitch = 0.5;
    }

    pub fn eye(&self) -> Vec3 {
        self.target + self.distance * self.back_dir()
    }

    /// Screen-up direction; stays defined when looking straight down.
    fn up(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        Vec3::new(-sp * cy, -sp * sy, cp)
    }

    fn right(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        Vec3::new(-sy, cy, 0.0)
    }

    pub fn orbit(&mut self, dx: f32, dy: f32) {
        self.yaw -= dx * 0.01;
        self.pitch = (self.pitch + dy * 0.01).clamp(-FRAC_PI_2, FRAC_PI_2);
    }

    pub fn pan(&mut self, dx: f32, dy: f32) {
        let scale = self.distance * 0.0015;
        self.target += (-self.right() * dx + self.up() * dy) * scale;
    }

    /// Zooms by `amount` wheel steps, keeping `focus` (the point under the
    /// cursor, if any) fixed on screen.
    pub fn zoom(&mut self, amount: f32, focus: Option<Vec3>) {
        let old = self.distance;
        self.distance = (old * (1.0 - amount * 0.1)).max(1e-3);
        if let Some(f) = focus {
            self.target = f + (self.target - f) * (self.distance / old);
        }
    }

    /// World-space ray through pixel (x, y) of a w x h viewport: (origin, unit direction).
    pub fn ray(&self, x: f32, y: f32, w: f32, h: f32) -> (Vec3, Vec3) {
        let inv = self.view_proj(w / h).inverse();
        let (nx, ny) = (2.0 * x / w - 1.0, 1.0 - 2.0 * y / h);
        let near = inv.project_point3(Vec3::new(nx, ny, 0.0));
        let far = inv.project_point3(Vec3::new(nx, ny, 1.0));
        (near, (far - near).normalize())
    }

    /// Pixel position of a world point, or None when it is behind the camera.
    pub fn project(&self, p: Vec3, w: f32, h: f32) -> Option<(f32, f32)> {
        let clip = self.view_proj(w / h) * p.extend(1.0);
        if clip.w <= 1e-6 {
            return None;
        }
        let (nx, ny) = (clip.x / clip.w, clip.y / clip.w);
        Some(((nx + 1.0) * 0.5 * w, (1.0 - ny) * 0.5 * h))
    }

    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        let view = Mat4::look_at_rh(self.eye(), self.target, self.up());
        let d = self.distance;
        let proj = if self.ortho {
            // Same framing as the perspective view at the target distance.
            let half_h = d * (FOV_Y * 0.5).tan();
            Mat4::orthographic_rh(-half_h * aspect, half_h * aspect, -half_h, half_h, -d * 100.0, d * 100.0)
        } else {
            Mat4::perspective_rh(FOV_Y, aspect, d * 0.01, d * 100.0)
        };
        proj * view
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn top_view_looks_down_with_y_up_the_screen() {
        let mut c = Camera::fit(Vec3::ZERO, Vec3::new(2.0, 2.0, 0.0));
        c.top();
        for ortho in [false, true] {
            c.ortho = ortho;
            let (x0, y0) = c.project(Vec3::new(1.0, 1.0, 0.0), 800.0, 600.0).unwrap();
            assert!((x0 - 400.0).abs() < 0.5 && (y0 - 300.0).abs() < 0.5, "target in the centre");
            let (x1, y1) = c.project(Vec3::new(2.0, 1.0, 0.0), 800.0, 600.0).unwrap();
            let (x2, y2) = c.project(Vec3::new(1.0, 2.0, 0.0), 800.0, 600.0).unwrap();
            assert!(x1 > x0 + 1.0 && (y1 - y0).abs() < 0.5, "+X goes right");
            assert!(y2 < y0 - 1.0 && (x2 - x0).abs() < 0.5, "+Y goes up");
        }
    }

    #[test]
    fn ray_round_trips_through_project() {
        for ortho in [false, true] {
            let mut c = Camera::fit(Vec3::ZERO, Vec3::splat(3.0));
            c.ortho = ortho;
            let p = Vec3::new(0.5, 2.0, 1.0);
            let (x, y) = c.project(p, 640.0, 480.0).unwrap();
            let (o, d) = c.ray(x, y, 640.0, 480.0);
            let closest = o + d * (p - o).dot(d);
            assert!((closest - p).length() < 1e-3, "ortho {ortho}: {closest} vs {p}");
        }
    }

    #[test]
    fn the_standard_views_look_along_the_axes() {
        let mut c = Camera::fit(Vec3::ZERO, Vec3::splat(2.0));
        let look = |c: &Camera| (c.target - c.eye()).normalize();
        c.front();
        assert!((look(&c) - Vec3::Y).length() < 1e-5, "front looks toward +Y");
        c.back();
        assert!((look(&c) + Vec3::Y).length() < 1e-5);
        c.right_side();
        assert!((look(&c) + Vec3::X).length() < 1e-5, "right looks toward -X");
        c.left_side();
        assert!((look(&c) - Vec3::X).length() < 1e-5);
    }

    #[test]
    fn look_around_keeps_the_eye_and_walk_keeps_the_height() {
        let mut c = Camera::fit(Vec3::ZERO, Vec3::splat(2.0));
        c.stand_at(Vec3::new(1.0, -3.0, 0.0), 1.676);
        let eye = c.eye();
        assert!((eye - Vec3::new(1.0, -3.0, 1.676)).length() < 1e-4);
        c.look_around(40.0, -25.0);
        assert!((c.eye() - eye).length() < 1e-4, "the eye stays put");
        let before = c.eye();
        c.walk(1.0);
        let moved = c.eye() - before;
        assert!(moved.z.abs() < 1e-5 && (moved.length() - 1.0).abs() < 1e-4, "one metre along the ground");
        assert!(moved.dot(c.target - before) > 0.0, "forward is the way it looks");
    }

    #[test]
    fn zoom_window_frames_the_rectangle() {
        let mut c = Camera::fit(Vec3::ZERO, Vec3::splat(2.0));
        let p = Vec3::new(0.3, 1.5, 0.4);
        let (x, y) = c.project(p, 800.0, 600.0).unwrap();
        let d = c.distance;
        c.zoom_window(x - 40.0, y - 30.0, x + 40.0, y + 30.0, 800.0, 600.0);
        assert!((c.distance - d * 0.1).abs() < 1e-4);
        let (x2, y2) = c.project(p, 800.0, 600.0).unwrap();
        assert!((x2 - 400.0).abs() < 25.0 && (y2 - 300.0).abs() < 25.0, "the point is near the centre now: {x2},{y2}");
    }

    #[test]
    fn zoom_keeps_the_focus_point_fixed() {
        let mut c = Camera::fit(Vec3::ZERO, Vec3::splat(2.0));
        let f = Vec3::new(2.0, 0.0, 0.0);
        let before = c.project(f, 800.0, 600.0).unwrap();
        c.zoom(2.0, Some(f));
        let after = c.project(f, 800.0, 600.0).unwrap();
        assert!((before.0 - after.0).abs() < 0.5 && (before.1 - after.1).abs() < 0.5);
    }
}
