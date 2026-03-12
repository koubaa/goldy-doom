use glam::{Mat4, Quat, Vec3};
use winit::event::ElementState;
use winit::keyboard::KeyCode;
use std::collections::HashSet;

pub struct Player {
    pub position: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub velocity: Vec3,

    keys_held: HashSet<KeyCode>,
    mouse_dx: f32,
    mouse_dy: f32,
}

impl Player {
    pub fn new(start_pos: Vec3, start_yaw: f32) -> Self {
        Self {
            position: start_pos,
            yaw: start_yaw,
            pitch: 0.0,
            velocity: Vec3::ZERO,
            keys_held: HashSet::new(),
            mouse_dx: 0.0,
            mouse_dy: 0.0,
        }
    }

    pub fn on_key(&mut self, key: KeyCode, state: ElementState) {
        match state {
            ElementState::Pressed => { self.keys_held.insert(key); }
            ElementState::Released => { self.keys_held.remove(&key); }
        }
    }

    pub fn on_mouse_motion(&mut self, dx: f64, dy: f64) {
        self.mouse_dx += dx as f32;
        self.mouse_dy += dy as f32;
    }

    pub fn update(&mut self, dt: f32) {
        const MOUSE_SENSITIVITY: f32 = 0.002;
        const MOVE_SPEED: f32 = 3.0;
        const HALF_PI: f32 = std::f32::consts::FRAC_PI_2 - 0.01;

        self.yaw -= self.mouse_dx * MOUSE_SENSITIVITY;
        self.pitch = (self.pitch - self.mouse_dy * MOUSE_SENSITIVITY).clamp(-HALF_PI, HALF_PI);
        self.mouse_dx = 0.0;
        self.mouse_dy = 0.0;

        let forward = Vec3::new(-self.yaw.sin(), 0.0, -self.yaw.cos());
        let right = Vec3::new(-forward.z, 0.0, forward.x);

        let mut move_dir = Vec3::ZERO;
        if self.keys_held.contains(&KeyCode::KeyW) { move_dir += forward; }
        if self.keys_held.contains(&KeyCode::KeyS) { move_dir -= forward; }
        if self.keys_held.contains(&KeyCode::KeyD) { move_dir += right; }
        if self.keys_held.contains(&KeyCode::KeyA) { move_dir -= right; }
        if self.keys_held.contains(&KeyCode::Space) { move_dir.y += 1.0; }
        if self.keys_held.contains(&KeyCode::ShiftLeft) { move_dir.y -= 1.0; }

        if move_dir.length_squared() > 0.0 {
            move_dir = move_dir.normalize();
        }

        let speed = if self.keys_held.contains(&KeyCode::ControlLeft) {
            MOVE_SPEED * 3.0
        } else {
            MOVE_SPEED
        };

        self.position += move_dir * speed * dt;
    }

    pub fn view_matrix(&self) -> Mat4 {
        let rot = Quat::from_rotation_y(self.yaw) * Quat::from_rotation_x(self.pitch);
        let forward = rot * (-Vec3::Z);
        Mat4::look_at_rh(self.position, self.position + forward, Vec3::Y)
    }

    pub fn projection_matrix(&self, aspect: f32) -> Mat4 {
        Mat4::perspective_rh(65.0f32.to_radians(), aspect, 0.01, 100.0)
    }
}
