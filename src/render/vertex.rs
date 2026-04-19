use bytemuck::{Pod, Zeroable};
use goldy::types::{VertexBufferLayout, VertexFormat};
use goldy::StructuredBufferElement;

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
        VertexBufferLayout::from_formats::<Self>(&[
            VertexFormat::Float32x3, // pos
            VertexFormat::Float32x2, // atlas_uv
            VertexFormat::Float32x2, // tile_uv
            VertexFormat::Float32x2, // tile_size
            VertexFormat::Float32,   // scroll_rate
            VertexFormat::Float32,   // row_height
            VertexFormat::Uint32,    // num_frames
            VertexFormat::Uint32,    // light
            VertexFormat::Uint32,    // use_flat_atlas
        ])
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
        VertexBufferLayout::from_formats::<Self>(&[
            VertexFormat::Float32x3, // pos
            VertexFormat::Float32x2, // atlas_uv
            VertexFormat::Float32x2, // tile_uv
            VertexFormat::Float32x2, // tile_size
            VertexFormat::Float32,   // local_x
            VertexFormat::Uint32,    // num_frames
            VertexFormat::Uint32,    // light
            VertexFormat::Uint32,    // _pad
        ])
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
        VertexBufferLayout::from_formats::<Self>(&[
            VertexFormat::Float32x3, // pos
            VertexFormat::Float32,   // _pad
        ])
    }
}

impl StructuredBufferElement for StaticVertex {}
impl StructuredBufferElement for SpriteVertex {}
impl StructuredBufferElement for SkyVertex {}
