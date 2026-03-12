use bytemuck::{Pod, Zeroable};
use goldy::types::{VertexAttribute, VertexBufferLayout, VertexFormat};

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct StaticVertex {
    pub pos: [f32; 3],
    pub atlas_uv: [f32; 2],
    pub tile_uv: [f32; 2],
    pub tile_size: [f32; 2],
    pub scroll_rate: f32,
    pub row_height: f32,
    pub num_frames: u32,
    pub light: u32,
    /// 0 = wall atlas, 1 = flat atlas (floors/ceilings)
    pub use_flat_atlas: u32,
}

impl StaticVertex {
    pub fn layout() -> VertexBufferLayout {
        VertexBufferLayout {
            stride: std::mem::size_of::<Self>() as u32,
            attributes: vec![
                VertexAttribute { location: 0, format: VertexFormat::Float32x3, offset: 0 },   // pos
                VertexAttribute { location: 1, format: VertexFormat::Float32x2, offset: 12 },  // atlas_uv
                VertexAttribute { location: 2, format: VertexFormat::Float32x2, offset: 20 }, // tile_uv
                VertexAttribute { location: 3, format: VertexFormat::Float32x2, offset: 28 }, // tile_size
                VertexAttribute { location: 4, format: VertexFormat::Float32,   offset: 36 }, // scroll_rate
                VertexAttribute { location: 5, format: VertexFormat::Float32,   offset: 40 },  // row_height
                VertexAttribute { location: 6, format: VertexFormat::Uint32,    offset: 44 }, // num_frames
                VertexAttribute { location: 7, format: VertexFormat::Uint32,    offset: 48 }, // light
                VertexAttribute { location: 8, format: VertexFormat::Uint32,    offset: 52 }, // use_flat_atlas
            ],
        }
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct SpriteVertex {
    pub pos: [f32; 3],
    pub atlas_uv: [f32; 2],
    pub tile_uv: [f32; 2],
    pub tile_size: [f32; 2],
    pub local_x: f32,
    pub num_frames: u32,
    pub light: u32,
    pub _pad: u32,
}

impl SpriteVertex {
    pub fn layout() -> VertexBufferLayout {
        VertexBufferLayout {
            stride: std::mem::size_of::<Self>() as u32,
            attributes: vec![
                VertexAttribute { location: 0, format: VertexFormat::Float32x3, offset: 0 },  // pos
                VertexAttribute { location: 1, format: VertexFormat::Float32x2, offset: 12 }, // atlas_uv
                VertexAttribute { location: 2, format: VertexFormat::Float32x2, offset: 20 }, // tile_uv
                VertexAttribute { location: 3, format: VertexFormat::Float32x2, offset: 28 }, // tile_size
                VertexAttribute { location: 4, format: VertexFormat::Float32,   offset: 36 }, // local_x
                VertexAttribute { location: 5, format: VertexFormat::Uint32,    offset: 40 }, // num_frames
                VertexAttribute { location: 6, format: VertexFormat::Uint32,    offset: 44 }, // light
                VertexAttribute { location: 7, format: VertexFormat::Uint32,    offset: 48 }, // _pad
            ],
        }
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct SkyVertex {
    pub pos: [f32; 3],
    pub _pad: f32,
}

impl SkyVertex {
    pub fn layout() -> VertexBufferLayout {
        VertexBufferLayout {
            stride: std::mem::size_of::<Self>() as u32,
            attributes: vec![
                VertexAttribute { location: 0, format: VertexFormat::Float32x3, offset: 0 },  // pos
                VertexAttribute { location: 1, format: VertexFormat::Float32,   offset: 12 }, // _pad
            ],
        }
    }
}
