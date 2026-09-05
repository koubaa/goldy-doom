use bytemuck::{Pod, Zeroable};

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

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable, goldy::GpuType)]
pub struct SpriteVertex {
    pub pos: [f32; 3],
    pub atlas_uv: [f32; 2],
    pub tile_uv: [f32; 2],
    pub tile_size: [f32; 2],
    pub local_x: f32,
    pub num_frames: u32,
    pub light: u32,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable, goldy::GpuType)]
pub struct SkyVertex {
    pub pos: [f32; 3],
}
