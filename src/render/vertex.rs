use bytemuck::{Pod, Zeroable};
use goldy::types::VertexBufferLayout;
use goldy::StructuredBufferElement;

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable, goldy::GpuType)]
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
        Self::GPU_TYPE
            .vertex_buffer_layout()
            .expect("StaticVertex raster layout")
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
            goldy::types::VertexFormat::Float32x3, // pos
            goldy::types::VertexFormat::Float32x2, // atlas_uv
            goldy::types::VertexFormat::Float32x2, // tile_uv
            goldy::types::VertexFormat::Float32x2, // tile_size
            goldy::types::VertexFormat::Float32,   // local_x
            goldy::types::VertexFormat::Uint32,    // num_frames
            goldy::types::VertexFormat::Uint32,    // light
            goldy::types::VertexFormat::Uint32,    // _pad
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
            goldy::types::VertexFormat::Float32x3, // pos
            goldy::types::VertexFormat::Float32,   // _pad
        ])
    }
}

impl StructuredBufferElement for SpriteVertex {}
impl StructuredBufferElement for SkyVertex {}
