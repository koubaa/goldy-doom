use bytemuck::{Pod, Zeroable};
use goldy::types::{VertexAttribute, VertexBufferLayout, VertexFormat};
use goldy::StructuredBufferElement;

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable, goldy::GpuType)]
pub struct StaticVertex {
    pub pos: [f32; 3],
    /// Matches HLSL float3 register padding so BufRO/StructuredBuffer stride == size_of.
    #[gpu(padding)]
    pub _pad: f32,
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
        // The storage ABI contains a pad word at offset 12. It is not a raster
        // attribute and must not consume TEXCOORD0 in Goldy's DX12 semantic map.
        VertexBufferLayout {
            stride: std::mem::size_of::<Self>() as u32,
            attributes: vec![
                VertexAttribute {
                    location: 0,
                    format: VertexFormat::Float32x3,
                    offset: 0,
                },
                VertexAttribute {
                    location: 1,
                    format: VertexFormat::Float32x2,
                    offset: 16,
                },
                VertexAttribute {
                    location: 2,
                    format: VertexFormat::Float32x2,
                    offset: 24,
                },
                VertexAttribute {
                    location: 3,
                    format: VertexFormat::Float32x2,
                    offset: 32,
                },
                VertexAttribute {
                    location: 4,
                    format: VertexFormat::Float32,
                    offset: 40,
                },
                VertexAttribute {
                    location: 5,
                    format: VertexFormat::Float32,
                    offset: 44,
                },
                VertexAttribute {
                    location: 6,
                    format: VertexFormat::Uint32,
                    offset: 48,
                },
                VertexAttribute {
                    location: 7,
                    format: VertexFormat::Uint32,
                    offset: 52,
                },
                VertexAttribute {
                    location: 8,
                    format: VertexFormat::Uint32,
                    offset: 56,
                },
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_vertex_gpu_schema_hides_transport_padding() {
        assert_eq!(std::mem::size_of::<StaticVertex>(), 60);
        assert_eq!(
            StaticVertex::GPU_TYPE
                .fields
                .iter()
                .map(|field| (field.name, field.offset))
                .collect::<Vec<_>>(),
            vec![
                ("pos", 0),
                ("atlas_uv", 16),
                ("tile_uv", 24),
                ("tile_size", 32),
                ("scroll_rate", 40),
                ("row_height", 44),
                ("num_frames", 48),
                ("light", 52),
                ("use_flat_atlas", 56),
            ]
        );
    }

    #[test]
    fn static_vertex_raster_layout_skips_transport_padding() {
        let layout = StaticVertex::layout();
        assert_eq!(layout.stride, 60);
        assert_eq!(
            layout
                .attributes
                .iter()
                .map(|attribute| (attribute.location, attribute.offset))
                .collect::<Vec<_>>(),
            vec![
                (0, 0),
                (1, 16),
                (2, 24),
                (3, 32),
                (4, 40),
                (5, 44),
                (6, 48),
                (7, 52),
                (8, 56),
            ]
        );
    }
}
