use glam::{Mat4, Vec3};

/// Orbit camera, Z-up, looking at `target`.
pub struct Camera {
    pub target: Vec3,
    pub distance: f32,
    pub yaw: f32,
    pub pitch: f32,
}

impl Camera {
    /// Frames a bounding box.
    pub fn fit(min: Vec3, max: Vec3) -> Self {
        let target = (min + max) * 0.5;
        let radius = ((max - min).length() * 0.5).max(1e-3);
        Camera { target, distance: radius * 2.8, yaw: -0.8, pitch: 0.5 }
    }

    pub fn eye(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        self.target + self.distance * Vec3::new(cp * cy, cp * sy, sp)
    }

    pub fn orbit(&mut self, dx: f32, dy: f32) {
        self.yaw -= dx * 0.01;
        self.pitch = (self.pitch + dy * 0.01).clamp(-1.54, 1.54);
    }

    pub fn pan(&mut self, dx: f32, dy: f32) {
        let forward = (self.target - self.eye()).normalize();
        let right = forward.cross(Vec3::Z).normalize_or_zero();
        let up = right.cross(forward);
        let scale = self.distance * 0.0015;
        self.target += (-right * dx + up * dy) * scale;
    }

    pub fn zoom(&mut self, amount: f32) {
        self.distance = (self.distance * (1.0 - amount * 0.1)).max(1e-3);
    }

    /// World-space ray through pixel (x, y) of a w x h viewport: (origin, unit direction).
    pub fn ray(&self, x: f32, y: f32, w: f32, h: f32) -> (Vec3, Vec3) {
        let inv = self.view_proj(w / h).inverse();
        let (nx, ny) = (2.0 * x / w - 1.0, 1.0 - 2.0 * y / h);
        let near = inv.project_point3(Vec3::new(nx, ny, 0.0));
        let far = inv.project_point3(Vec3::new(nx, ny, 1.0));
        (near, (far - near).normalize())
    }

    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        let view = Mat4::look_at_rh(self.eye(), self.target, Vec3::Z);
        let proj = Mat4::perspective_rh(45f32.to_radians(), aspect, self.distance * 0.01, self.distance * 100.0);
        proj * view
    }
}
