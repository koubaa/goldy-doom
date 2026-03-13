use super::level_builder::LevelMeshData;
use super::vertex::{SkyVertex, SpriteVertex, StaticVertex};
use anyhow::{Context, Result};
use bytemuck::{Pod, Zeroable};
use glam::Mat4;
use goldy::types::{
    AddressMode, CompareFunction, DataAccess, DepthFormat, DepthStencilState, FilterMode,
    IndexFormat, SamplerDesc, SpatialAccess, TextureFlags, TextureFormat,
};
use goldy::{
    Buffer, BufferPool, BufferView, CommandEncoder, Device, Instance, RenderPipeline,
    RenderPipelineDesc, Sampler, ShaderModule, Surface, Texture,
};
use std::mem::size_of;
use std::sync::Arc;
use winit::window::Window;

/// Must match the `SceneUniforms` struct in doom_common.slang exactly.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct SceneUniforms {
    pub projection: [[f32; 4]; 4],
    pub modelview: [[f32; 4]; 4],
    pub atlas_size: [f32; 2],
    pub flat_atlas_size: [f32; 2],
    pub time: f32,
    pub tiled_band_size: f32,
}

/// Push constant slot indices — must match doom_common.slang PC_* constants.
const PC_SCENE: usize = 0;
const PC_LIGHTS: usize = 1;
const PC_WALL: usize = 2;
const PC_FLAT: usize = 3;
const PC_PALETTE: usize = 4;
const PC_SKY: usize = 5;
const PC_SAMPLER: usize = 6;
const NUM_PUSH_CONSTANTS: usize = 7;

struct LevelGpuResources {
    pool: BufferPool,
    static_vb: BufferView,
    static_ib: BufferView,
    static_index_count: u32,

    sky_vb: BufferView,
    sky_ib: BufferView,
    sky_index_count: u32,

    decor_vb: BufferView,
    decor_ib: BufferView,
    decor_index_count: u32,

    wall_atlas: Texture,
    flat_atlas: Texture,
    palette: Texture,
    sky_texture: Texture,

    wall_atlas_size: [f32; 2],
    flat_atlas_size: [f32; 2],
    tiled_band_size: f32,
}

pub struct Renderer {
    instance: Instance,
    device: Arc<Device>,

    surface: Option<Surface>,
    static_pipeline: Option<RenderPipeline>,
    sky_pipeline: Option<RenderPipeline>,
    sprite_pipeline: Option<RenderPipeline>,

    sampler: Sampler,
    scene_buf: Buffer,
    light_buf: Buffer,

    level: Option<LevelGpuResources>,
}

impl Renderer {
    pub fn new() -> Result<Self> {
        let instance = Instance::new().context("Failed to create goldy instance")?;
        let device = Arc::new(
            instance
                .create_device(goldy::DeviceType::DiscreteGpu)
                .context("Failed to create goldy device")?,
        );

        let sampler = Sampler::new(
            &device,
            &SamplerDesc {
                mag_filter: FilterMode::Nearest,
                min_filter: FilterMode::Nearest,
                mipmap_filter: FilterMode::Nearest,
                address_mode_u: AddressMode::Repeat,
                address_mode_v: AddressMode::ClampToEdge,
                ..Default::default()
            },
        )
        .context("Failed to create sampler")?;

        let scene_uniforms = SceneUniforms::zeroed();
        let scene_buf = Buffer::with_data(&device, &[scene_uniforms], DataAccess::Broadcast)
            .context("Failed to create scene uniform buffer")?;

        let initial_lights: Vec<f32> = vec![1.0; 256];
        let light_buf =
            Buffer::with_data(&device, &initial_lights, DataAccess::Scattered)
                .context("Failed to create light buffer")?;

        Ok(Self {
            instance,
            device,
            surface: None,
            static_pipeline: None,
            sky_pipeline: None,
            sprite_pipeline: None,
            sampler,
            scene_buf,
            light_buf,
            level: None,
        })
    }

    /// Called once the window exists. Creates the surface and compiles pipelines.
    pub fn init_surface(&mut self, window: &Window) -> Result<()> {
        let surface = Surface::new_with_depth(
            &self.device,
            window,
            Some(DepthFormat::Depth24Plus),
        )
        .context("Failed to create surface")?;
        let target_format = surface.format();

        let shader_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("assets")
            .join("shaders");
        let shader_path = shader_dir.to_string_lossy().to_string();

        let static_src = std::fs::read_to_string(shader_dir.join("doom_static.slang"))
            .context("Failed to read doom_static.slang")?;
        let sky_src = std::fs::read_to_string(shader_dir.join("doom_sky.slang"))
            .context("Failed to read doom_sky.slang")?;
        let sprite_src = std::fs::read_to_string(shader_dir.join("doom_sprite.slang"))
            .context("Failed to read doom_sprite.slang")?;

        let static_shader =
            ShaderModule::from_slang_with_paths(&self.device, &static_src, &[&shader_path])
                .context("Failed to compile doom_static shader")?;
        let sky_shader =
            ShaderModule::from_slang_with_paths(&self.device, &sky_src, &[&shader_path])
                .context("Failed to compile doom_sky shader")?;
        let sprite_shader =
            ShaderModule::from_slang_with_paths(&self.device, &sprite_src, &[&shader_path])
                .context("Failed to compile doom_sprite shader")?;

        let static_depth = Some(DepthStencilState::default());

        let sky_depth = Some(DepthStencilState {
            depth_write_enabled: false,
            ..DepthStencilState::default()
        });

        let sprite_depth = Some(DepthStencilState {
            depth_write_enabled: false,
            ..DepthStencilState::default()
        });

        let static_pipeline = RenderPipeline::new(
            &self.device,
            &static_shader,
            &static_shader,
            &RenderPipelineDesc {
                vertex_layout: StaticVertex::layout(),
                target_format,
                depth_stencil: static_depth,
                ..Default::default()
            },
        )
        .context("Failed to create static pipeline")?;

        let sky_pipeline = RenderPipeline::new(
            &self.device,
            &sky_shader,
            &sky_shader,
            &RenderPipelineDesc {
                vertex_layout: SkyVertex::layout(),
                target_format,
                depth_stencil: sky_depth,
                ..Default::default()
            },
        )
        .context("Failed to create sky pipeline")?;

        let sprite_pipeline = RenderPipeline::new(
            &self.device,
            &sprite_shader,
            &sprite_shader,
            &RenderPipelineDesc {
                vertex_layout: SpriteVertex::layout(),
                target_format,
                depth_stencil: sprite_depth,
                ..Default::default()
            },
        )
        .context("Failed to create sprite pipeline")?;

        self.surface = Some(surface);
        self.static_pipeline = Some(static_pipeline);
        self.sky_pipeline = Some(sky_pipeline);
        self.sprite_pipeline = Some(sprite_pipeline);

        log::info!("Renderer: surface + pipelines initialized (format: {:?})", target_format);
        Ok(())
    }

    pub fn load_level(
        &mut self,
        mesh: LevelMeshData,
        palette: Vec<u8>,
        wall_atlas: (Vec<u16>, [usize; 2]),
        flat_atlas: (Vec<u8>, [usize; 2]),
        sky_texture: Option<(Vec<u8>, [usize; 2])>,
        tiled_band_size: f32,
    ) -> Result<()> {
        let device = &self.device;

        // Single BufferPool for all level geometry — one GPU allocation, six views.
        let total = BufferPool::padded_size(&[
            (mesh.static_vertices.len(), size_of::<StaticVertex>()),
            (mesh.static_indices.len(), size_of::<u32>()),
            (mesh.sky_vertices.len(), size_of::<SkyVertex>()),
            (mesh.sky_indices.len(), size_of::<u32>()),
            (mesh.decor_vertices.len().max(1), size_of::<SpriteVertex>()),
            (mesh.decor_indices.len().max(1), size_of::<u32>()),
        ]);

        let mut pool =
            BufferPool::new(device, total).context("level geometry buffer pool")?;

        let static_vb = pool.alloc_with_data(&mesh.static_vertices)?;
        let static_ib = pool.alloc_with_data(&mesh.static_indices)?;
        let sky_vb = pool.alloc_with_data(&mesh.sky_vertices)?;
        let sky_ib = pool.alloc_with_data(&mesh.sky_indices)?;

        let decor_vb = if mesh.decor_vertices.is_empty() {
            pool.alloc_with_data(&[SpriteVertex::zeroed()])?
        } else {
            pool.alloc_with_data(&mesh.decor_vertices)?
        };
        let decor_ib = if mesh.decor_indices.is_empty() {
            pool.alloc_with_data(&[0u32])?
        } else {
            pool.alloc_with_data(&mesh.decor_indices)?
        };

        // Wall atlas: u16 raw bytes → Rg8Unorm (R=palette_idx, G=transparency).
        let (wall_w, wall_h) = (wall_atlas.1[0] as u32, wall_atlas.1[1] as u32);
        let wall_tex = Texture::with_data(
            device,
            bytemuck::cast_slice::<u16, u8>(&wall_atlas.0),
            wall_w,
            wall_h,
            TextureFormat::Rg8Unorm,
            SpatialAccess::Interpolated,
            TextureFlags::COPY_DST,
        )
        .context("wall atlas texture")?;

        // Flat atlas: u8 raw → R8Unorm (palette index per pixel).
        let (flat_w, flat_h) = (flat_atlas.1[0] as u32, flat_atlas.1[1] as u32);
        let flat_tex = Texture::with_data(
            device,
            &flat_atlas.0,
            flat_w,
            flat_h,
            TextureFormat::R8Unorm,
            SpatialAccess::Interpolated,
            TextureFlags::COPY_DST,
        )
        .context("flat atlas texture")?;

        // Palette: RGB triplets → RGBA8. Dimensions: 256 x num_colormaps.
        let num_colors = palette.len() / 3;
        let palette_h = (num_colors / 256).max(1) as u32;
        let palette_rgba = palette_to_rgba8(&palette);
        let palette_tex = Texture::with_data(
            device,
            &palette_rgba,
            256,
            palette_h,
            TextureFormat::Rgba8Unorm,
            SpatialAccess::Interpolated,
            TextureFlags::COPY_DST,
        )
        .context("palette texture")?;

        // Sky texture: u8 raw → R8Unorm (palette index per pixel). Fallback 1x1 black.
        let sky_tex = match sky_texture {
            Some((data, [w, h])) => Texture::with_data(
                device,
                &data,
                w as u32,
                h as u32,
                TextureFormat::R8Unorm,
                SpatialAccess::Interpolated,
                TextureFlags::COPY_DST,
            )
            .context("sky texture")?,
            None => Texture::with_data(
                device,
                &[0u8],
                1,
                1,
                TextureFormat::R8Unorm,
                SpatialAccess::Interpolated,
                TextureFlags::COPY_DST,
            )
            .context("sky fallback texture")?,
        };

        log::info!(
            "Renderer: level loaded ({} static tris, {} sky tris, {} sprite tris)",
            mesh.static_indices.len() / 3,
            mesh.sky_indices.len() / 3,
            mesh.decor_indices.len() / 3,
        );
        log::info!(
            "  wall atlas: {}x{}, flat atlas: {}x{}, palette: 256x{}",
            wall_w, wall_h, flat_w, flat_h, palette_h,
        );

        self.level = Some(LevelGpuResources {
            pool,
            static_vb,
            static_ib,
            static_index_count: mesh.static_indices.len() as u32,
            sky_vb,
            sky_ib,
            sky_index_count: mesh.sky_indices.len() as u32,
            decor_vb,
            decor_ib,
            decor_index_count: mesh.decor_indices.len() as u32,
            wall_atlas: wall_tex,
            flat_atlas: flat_tex,
            palette: palette_tex,
            sky_texture: sky_tex,
            wall_atlas_size: [wall_w as f32, wall_h as f32],
            flat_atlas_size: [flat_w as f32, flat_h as f32],
            tiled_band_size,
        });

        Ok(())
    }

    pub fn render_frame(
        &mut self,
        view: Mat4,
        proj: Mat4,
        time: f32,
        light_levels: &[f32],
    ) -> Result<()> {
        let surface = match &self.surface {
            Some(s) => s,
            None => return Ok(()),
        };
        let level = match &self.level {
            Some(l) => l,
            None => return Ok(()),
        };
        let static_pipeline = self.static_pipeline.as_ref().unwrap();
        let sky_pipeline = self.sky_pipeline.as_ref().unwrap();

        let uniforms = SceneUniforms {
            projection: proj.transpose().to_cols_array_2d(),
            modelview: view.transpose().to_cols_array_2d(),
            atlas_size: level.wall_atlas_size,
            flat_atlas_size: level.flat_atlas_size,
            time,
            tiled_band_size: level.tiled_band_size,
        };
        self.scene_buf
            .write(0, bytemuck::bytes_of(&uniforms))?;

        if light_levels.len() >= 256 {
            self.light_buf
                .write(0, bytemuck::cast_slice(&light_levels[..256]))?;
        }

        let scene_idx = self.scene_buf.bindless_index().unwrap_or(0);
        let light_idx = self.light_buf.bindless_index().unwrap_or(0);
        let wall_idx = level.wall_atlas.bindless_index().unwrap_or(0);
        let flat_idx = level.flat_atlas.bindless_index().unwrap_or(0);
        let palette_idx = level.palette.bindless_index().unwrap_or(0);
        let sky_idx = level.sky_texture.bindless_index().unwrap_or(0);
        let sampler_idx = self.sampler.bindless_index().unwrap_or(0);

        let push_constants = [
            scene_idx,
            light_idx,
            wall_idx,
            flat_idx,
            palette_idx,
            sky_idx,
            sampler_idx,
        ];

        let frame = surface.acquire()?;

        let mut encoder = CommandEncoder::new();
        {
            let mut pass = encoder.begin_render_pass();
            pass.clear(goldy::Color::BLACK);
            pass.clear_depth(1.0);

            // Push constants must be set AFTER set_pipeline (root signature must be bound first in D3D12,
            // and SetGraphicsRootSignature invalidates all root arguments).

            // Sky first (background), then static (walls/floors), then decor
            if level.sky_index_count > 0 {
                pass.set_pipeline(sky_pipeline);
                pass.set_push_constants_raw(&push_constants);
                pass.set_vertex_buffer(0, &level.sky_vb);
                pass.set_index_buffer(&level.sky_ib, IndexFormat::Uint32);
                pass.draw_indexed(0..level.sky_index_count, 0, 0..1);
            }

            if level.static_index_count > 0 {
                pass.set_pipeline(static_pipeline);
                pass.set_push_constants_raw(&push_constants);
                pass.set_vertex_buffer(0, &level.static_vb);
                pass.set_index_buffer(&level.static_ib, IndexFormat::Uint32);
                pass.draw_indexed(0..level.static_index_count, 0, 0..1);
            }

            if level.decor_index_count > 0 {
                let sprite_pipeline = self.sprite_pipeline.as_ref().unwrap();
                pass.set_pipeline(sprite_pipeline);
                pass.set_push_constants_raw(&push_constants);
                pass.set_vertex_buffer(0, &level.decor_vb);
                pass.set_index_buffer(&level.decor_ib, IndexFormat::Uint32);
                pass.draw_indexed(0..level.decor_index_count, 0, 0..1);
            }
        }

        frame.render(encoder)?;
        surface.present(frame)?;
        Ok(())
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if let Some(surface) = &mut self.surface {
            let _ = surface.resize(width, height);
        }
    }

    pub fn has_level(&self) -> bool {
        self.level.is_some()
    }
}

// ============================================================================
// Texture format conversion helpers
// ============================================================================

/// Palette: RGB triplets → RGBA8.
fn palette_to_rgba8(rgb: &[u8]) -> Vec<u8> {
    let mut rgba = Vec::with_capacity(rgb.len() / 3 * 4);
    for chunk in rgb.chunks_exact(3) {
        rgba.push(chunk[0]);
        rgba.push(chunk[1]);
        rgba.push(chunk[2]);
        rgba.push(255);
    }
    rgba
}
