use super::lights::Lights;
use super::vertex::{SkyVertex, SpriteVertex, StaticVertex};
use crate::wad::tex::{Bounds, BoundsLookup};
use crate::wad::visitor::ObjectId;
use crate::wad::{
    Decor, LevelVisitor, LightInfo, Marker, SkyPoly, SkyQuad, StaticPoly, StaticQuad,
};
use glam::Vec2;
use glam::Vec3;
use indexmap::IndexMap;
use log::warn;

pub struct LevelMeshData {
    pub static_vertices: Vec<StaticVertex>,
    pub sky_vertices: Vec<SkyVertex>,
    pub decor_vertices: Vec<SpriteVertex>,
    pub static_indices: Vec<u32>,
    pub sky_indices: Vec<u32>,
    pub decor_indices: Vec<u32>,
    pub start_pos: Vec3,
    pub start_yaw: f32,
    pub lights: Lights,
}

pub struct LevelBuilder {
    wall_bounds: BoundsLookup,
    flat_bounds: BoundsLookup,
    decor_bounds: BoundsLookup,

    pub lights: Lights,
    pub start_pos: Vec3,
    pub start_yaw: f32,

    pub static_vertices: Vec<StaticVertex>,
    pub sky_vertices: Vec<SkyVertex>,
    pub decor_vertices: Vec<SpriteVertex>,

    pub static_indices: Vec<u32>,
    pub sky_indices: Vec<u32>,
    pub decor_indices: Vec<u32>,

    num_wall_quads: usize,
    num_floor_polys: usize,
    num_ceil_polys: usize,
    num_decors: usize,
}

impl LevelBuilder {
    pub fn new(
        wall_bounds: BoundsLookup,
        flat_bounds: BoundsLookup,
        decor_bounds: BoundsLookup,
    ) -> Self {
        Self {
            wall_bounds,
            flat_bounds,
            decor_bounds,
            lights: Lights::new(),
            start_pos: Vec3::ZERO,
            start_yaw: 0.0,
            static_vertices: Vec::with_capacity(16_384),
            sky_vertices: Vec::with_capacity(16_384),
            decor_vertices: Vec::with_capacity(16_384),
            static_indices: Vec::with_capacity(65_536),
            sky_indices: Vec::with_capacity(16_384),
            decor_indices: Vec::with_capacity(4_096),
            num_wall_quads: 0,
            num_floor_polys: 0,
            num_ceil_polys: 0,
            num_decors: 0,
        }
    }

    pub fn finish(self) -> LevelMeshData {
        log::info!(
            "Level mesh: {} wall quads, {} floor polys, {} ceil polys, {} decors",
            self.num_wall_quads, self.num_floor_polys, self.num_ceil_polys, self.num_decors,
        );
        log::info!(
            "  static verts={}, sky verts={}, sprite verts={}",
            self.static_vertices.len(), self.sky_vertices.len(), self.decor_vertices.len(),
        );
        log::info!(
            "  static idx={}, sky idx={}, decor idx={}",
            self.static_indices.len(), self.sky_indices.len(), self.decor_indices.len(),
        );
        LevelMeshData {
            static_vertices: self.static_vertices,
            sky_vertices: self.sky_vertices,
            decor_vertices: self.decor_vertices,
            static_indices: self.static_indices,
            sky_indices: self.sky_indices,
            decor_indices: self.decor_indices,
            start_pos: self.start_pos,
            start_yaw: self.start_yaw,
            lights: self.lights,
        }
    }

    fn add_light_info(&mut self, light_info: &LightInfo) -> u8 {
        self.lights.push(light_info)
    }

    fn push_static_quad(&mut self) {
        let n = self.static_vertices.len() as u32;
        let v0 = n - 4;
        self.static_indices.extend_from_slice(&[v0, v0 + 1, v0 + 3, v0 + 1, v0 + 2, v0 + 3]);
    }

    fn push_static_poly(&mut self, poly_len: usize) {
        let n = self.static_vertices.len() as u32;
        let v0 = n - poly_len as u32;
        for i in 1..(poly_len as u32 - 1) {
            self.static_indices.extend_from_slice(&[v0, v0 + i, v0 + i + 1]);
        }
    }

    fn push_sky_quad(&mut self) {
        let n = self.sky_vertices.len() as u32;
        let v0 = n - 4;
        self.sky_indices.extend_from_slice(&[v0, v0 + 1, v0 + 3, v0 + 1, v0 + 2, v0 + 3]);
    }

    fn push_sky_poly(&mut self, poly_len: usize) {
        let n = self.sky_vertices.len() as u32;
        let v0 = n - poly_len as u32;
        for i in 1..(poly_len as u32 - 1) {
            self.sky_indices.extend_from_slice(&[v0, v0 + i, v0 + i + 1]);
        }
    }

    fn push_decor_quad(&mut self) {
        let n = self.decor_vertices.len() as u32;
        let v0 = n - 4;
        self.decor_indices.extend_from_slice(&[v0, v0 + 1, v0 + 3, v0 + 1, v0 + 2, v0 + 3]);
    }
}

impl LevelVisitor for LevelBuilder {
    fn visit_wall_quad(&mut self, quad: &StaticQuad) {
        self.num_wall_quads += 1;
        let tex_name = match quad.tex_name {
            Some(n) => n,
            None => return,
        };
        let bounds = match self.wall_bounds.get(&tex_name) {
            Some(b) => *b,
            None => { warn!("No wall texture {}", tex_name); return; }
        };
        let light = self.add_light_info(quad.light_info) as u32;
        let (v1, v2) = quad.vertices;
        let (low, high) = quad.height_range;
        let (s1, t1) = quad.tex_start;
        let (s2, t2) = quad.tex_end;
        let scroll = quad.scroll;

        let mk = |xz: Vec2, y: f32, u: f32, v: f32| StaticVertex {
            pos: [xz.x, y, xz.y],
            atlas_uv: bounds.pos,
            tile_uv: [u, v],
            tile_size: bounds.size,
            scroll_rate: scroll,
            row_height: bounds.row_height as f32,
            num_frames: bounds.num_frames as u32,
            light,
            use_flat_atlas: 0,
        };
        self.static_vertices.push(mk(v1, low, s1, t1));
        self.static_vertices.push(mk(v2, low, s2, t1));
        self.static_vertices.push(mk(v2, high, s2, t2));
        self.static_vertices.push(mk(v1, high, s1, t2));
        self.push_static_quad();
    }

    fn visit_floor_poly(&mut self, poly: &StaticPoly) {
        self.num_floor_polys += 1;
        let bounds = match self.flat_bounds.get(&poly.tex_name) {
            Some(b) => *b,
            None => {
                warn!("No floor texture {}", poly.tex_name); return;
            }
        };
        let light = self.add_light_info(poly.light_info) as u32;
        for &v in poly.vertices {
            self.static_vertices.push(StaticVertex {
                pos: [v.x, poly.height, v.y],
                atlas_uv: bounds.pos,
                tile_uv: [-v.x * 100.0, -v.y * 100.0],
                tile_size: bounds.size,
                scroll_rate: 0.0,
                row_height: bounds.row_height as f32,
                num_frames: bounds.num_frames as u32,
                light,
                use_flat_atlas: 1,
            });
        }
        self.push_static_poly(poly.vertices.len());
    }

    fn visit_ceil_poly(&mut self, poly: &StaticPoly) {
        self.num_ceil_polys += 1;
        let bounds = match self.flat_bounds.get(&poly.tex_name) {
            Some(b) => *b,
            None => {
                warn!("No ceiling texture {}", poly.tex_name); return;
            }
        };
        let light = self.add_light_info(poly.light_info) as u32;
        for &v in poly.vertices.iter().rev() {
            self.static_vertices.push(StaticVertex {
                pos: [v.x, poly.height, v.y],
                atlas_uv: bounds.pos,
                tile_uv: [-v.x * 100.0, -v.y * 100.0],
                tile_size: bounds.size,
                scroll_rate: 0.0,
                row_height: bounds.row_height as f32,
                num_frames: bounds.num_frames as u32,
                light,
                use_flat_atlas: 1,
            });
        }
        self.push_static_poly(poly.vertices.len());
    }

    fn visit_floor_sky_poly(&mut self, poly: &SkyPoly) {
        for &v in poly.vertices {
            self.sky_vertices.push(SkyVertex { pos: [v.x, poly.height, v.y], _pad: 0.0 });
        }
        self.push_sky_poly(poly.vertices.len());
    }

    fn visit_ceil_sky_poly(&mut self, poly: &SkyPoly) {
        for &v in poly.vertices.iter().rev() {
            self.sky_vertices.push(SkyVertex { pos: [v.x, poly.height, v.y], _pad: 0.0 });
        }
        self.push_sky_poly(poly.vertices.len());
    }

    fn visit_sky_quad(&mut self, quad: &SkyQuad) {
        let (v1, v2) = quad.vertices;
        let (low, high) = quad.height_range;
        self.sky_vertices.push(SkyVertex { pos: [v1.x, low, v1.y], _pad: 0.0 });
        self.sky_vertices.push(SkyVertex { pos: [v2.x, low, v2.y], _pad: 0.0 });
        self.sky_vertices.push(SkyVertex { pos: [v2.x, high, v2.y], _pad: 0.0 });
        self.sky_vertices.push(SkyVertex { pos: [v1.x, high, v1.y], _pad: 0.0 });
        self.push_sky_quad();
    }

    fn visit_marker(&mut self, pos: [f32; 3], yaw: f32, marker: Marker) {
        if let Marker::StartPos { player: 0 } = marker {
            self.start_pos = Vec3::new(pos[0], pos[1] + 0.5, pos[2]);
            self.start_yaw = yaw;
        }
    }

    fn visit_decor(&mut self, decor: &Decor) {
        self.num_decors += 1;
        let light = self.add_light_info(decor.light_info) as u32;
        let bounds = match self.decor_bounds.get(&decor.tex_name) {
            Some(b) => *b,
            None => { warn!("No decor texture {}", decor.tex_name); return; }
        };
        let hw = decor.half_width;
        let mk = |p: [f32; 3], lx: f32, u: f32, v: f32| SpriteVertex {
            pos: p, local_x: lx,
            atlas_uv: bounds.pos, tile_uv: [u, v], tile_size: bounds.size,
            num_frames: 1, light, _pad: 0,
        };
        self.decor_vertices.push(mk(decor.low, -hw, 0.0, bounds.size[1]));
        self.decor_vertices.push(mk(decor.low, hw, bounds.size[0], bounds.size[1]));
        self.decor_vertices.push(mk(decor.high, hw, bounds.size[0], 0.0));
        self.decor_vertices.push(mk(decor.high, -hw, 0.0, 0.0));
        self.push_decor_quad();
    }
}
