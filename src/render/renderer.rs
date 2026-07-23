use super::level_builder::LevelMeshData;
use super::vertex::{SkyVertex, SpriteVertex, StaticVertex};
use anyhow::{Context, Result};
use bytemuck::{Pod, Zeroable};
use glam::Mat4;
use goldy::types::{
    AddressMode, BufferKind, DepthFormat, DepthStencilState, FilterMode, IndexFormat, SamplerDesc,
    SurfaceConfig, TargetLoad, TextureFlags, TextureFormat, TextureKind,
};
use goldy::{
    Buffer, Context as GpuContext, Device, Instance, LayoutCheckable, Lease, LeaseRenderTarget,
    MemoryExchange, NodeAccess, Parcel, RenderPipeline, RenderPipelineDesc, RetainedPool, Sampler, Scheme,
    ShaderLibrary, ShaderModule, ShaderResourceSlot, StructuredBufferElement, SurfaceExchange, Texture,
    Transaction, Init, ordinal,
};

/// Upload CPU bytes into a retained buffer parcel via a property-only micro-scheme dispatch.
fn upload_parcel(ctx: &GpuContext, parcel: &Parcel, offset: u64, data: &[u8]) -> Result<()> {
    let mut upload = Scheme::new(ctx);
    let deposit = MemoryExchange::new(ctx)
        .bind_deposit_buffer_at(&mut upload, parcel, offset, data.len() as u64)
        .context("bind_deposit_buffer_at")?;
    deposit
        .write(&mut upload, 0, data)
        .context("deposit write")?;
    upload.submit().context("upload scheme submit")?;
    Ok(())
}
use std::sync::Arc;
use winit::window::Window;

/// Must match the `SceneUniforms` struct in doom_common.slang exactly.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable, LayoutCheckable)]
pub struct SceneUniforms {
    pub projection: [[f32; 4]; 4],
    pub modelview: [[f32; 4]; 4],
    pub atlas_size: [f32; 2],
    pub flat_atlas_size: [f32; 2],
    pub time: f32,
    pub tiled_band_size: f32,
}

impl StructuredBufferElement for SceneUniforms {}

struct LevelGpuResources {
    geometry: Buffer,
    static_vb: usize,
    static_ib: usize,
    static_index_count: u32,

    sky_vb: usize,
    sky_ib: usize,
    sky_index_count: u32,

    decor_vb: usize,
    decor_ib: usize,
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
    context: Option<GpuContext>,

    surface: Option<SurfaceExchange>,
    present: Option<Transaction>,
    static_pipeline: Option<RenderPipeline>,
    sky_pipeline: Option<RenderPipeline>,
    sprite_pipeline: Option<RenderPipeline>,

    sampler: Sampler,
    scene_buf: Buffer,
    light_buf: Buffer,

    level: Option<LevelGpuResources>,
    retained_pool: RetainedPool,

    scene_rt: Option<Lease<LeaseRenderTarget>>,
    scheme: Option<Scheme>,
}

impl Renderer {
    pub fn new() -> Result<Self> {
        let instance = Instance::new().context("Failed to create goldy instance")?;
        let device = Arc::new(
            instance
                .request_adapter(&goldy::RequestAdapterOptions::default())
                .context("Failed to request goldy adapter")?
                .request_device(&goldy::DeviceDescriptor::default())
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

        let mut retained_pool = RetainedPool::new(device.clone());

        // Scene + light buffers are retained single-backed allocations: one allocation each,
        // kept across frames with a stable bindless identity, rewritten in place every frame
        // via [`upload_parcel`] (property-only micro-scheme dispatch).
        let scene_uniforms = SceneUniforms::zeroed();
        let scene_bytes = bytemuck::bytes_of(&scene_uniforms);
        let scene_buf = retained_pool
            .acquire_buffer(
                scene_bytes.len() as u64,
                BufferKind::Broadcast,
                Some(std::mem::size_of::<SceneUniforms>() as u32),
                goldy::types::BufferFlags::empty(),
                Some(scene_bytes),
            )
            .context("Failed to create scene uniform buffer")?;

        let initial_lights: Vec<f32> = vec![1.0; 256];
        let light_buf = retained_pool
            .acquire_buffer(
                (initial_lights.len() * std::mem::size_of::<f32>()) as u64,
                BufferKind::Scattered,
                Some(std::mem::size_of::<f32>() as u32),
                goldy::types::BufferFlags::empty(),
                Some(bytemuck::cast_slice(&initial_lights)),
            )
            .context("Failed to create light buffer")?;

        Ok(Self {
            instance,
            device,
            context: None,
            surface: None,
            present: None,
            static_pipeline: None,
            sky_pipeline: None,
            sprite_pipeline: None,
            sampler,
            scene_buf,
            light_buf,
            level: None,
            retained_pool,
            scene_rt: None,
            scheme: None,
        })
    }

    fn record_scheme(
        scheme: &mut Scheme,
        surface: &SurfaceExchange,
        static_pipeline: &RenderPipeline,
        sky_pipeline: &RenderPipeline,
        sprite_pipeline: &RenderPipeline,
        scene_buf: &Buffer,
        light_buf: &Buffer,
        sampler: &Sampler,
        level: &LevelGpuResources,
        scene_rt: &Lease<LeaseRenderTarget>,
    ) -> Result<Transaction> {
        let shader_resources = [
            ShaderResourceSlot::Parcel {
                parcel: &*scene_buf,
                access: NodeAccess::Read,
            },
            ShaderResourceSlot::Parcel {
                parcel: &*light_buf,
                access: NodeAccess::Read,
            },
            ShaderResourceSlot::Parcel {
                parcel: &*level.wall_atlas,
                access: NodeAccess::Read,
            },
            ShaderResourceSlot::Parcel {
                parcel: &*level.flat_atlas,
                access: NodeAccess::Read,
            },
            ShaderResourceSlot::Parcel {
                parcel: &*level.palette,
                access: NodeAccess::Read,
            },
            ShaderResourceSlot::Parcel {
                parcel: &*level.sky_texture,
                access: NodeAccess::Read,
            },
            ShaderResourceSlot::Sampler(sampler),
        ];

        let mut pass = scheme.render_pass("doom", scene_rt, TargetLoad::Clear(goldy::Color::BLACK));
        pass.with_shader_resources(&shader_resources);
        pass.with_buffer_dependency(&level.geometry, NodeAccess::Read);
        pass.clear_depth(1.0);

        if level.sky_index_count > 0 {
            pass.set_pipeline(sky_pipeline);
            pass.set_vertex_buffer(0, &level.geometry[level.sky_vb]);
            pass.set_index_buffer(&level.geometry[level.sky_ib], IndexFormat::Uint32);
            pass.draw_indexed(0..level.sky_index_count, 0, 0..1);
        }

        if level.static_index_count > 0 {
            pass.set_pipeline(static_pipeline);
            pass.set_vertex_buffer(0, &level.geometry[level.static_vb]);
            pass.set_index_buffer(&level.geometry[level.static_ib], IndexFormat::Uint32);
            pass.draw_indexed(0..level.static_index_count, 0, 0..1);
        }

        if level.decor_index_count > 0 {
            pass.set_pipeline(sprite_pipeline);
            pass.set_vertex_buffer(0, &level.geometry[level.decor_vb]);
            pass.set_index_buffer(&level.geometry[level.decor_ib], IndexFormat::Uint32);
            pass.draw_indexed(0..level.decor_index_count, 0, 0..1);
        }

        pass.finish();
        surface
            .bind_render_target(scheme, scene_rt)
            .map_err(Into::into)
    }

    fn rerecord_scheme(&mut self) -> Result<()> {
        let surface = self.surface.as_ref().context("surface not initialized")?;
        let context = self.context.as_ref().context("context not initialized")?;
        let level = self.level.as_ref().context("level not loaded")?;

        let mut scheme = Scheme::new(context);
        let (width, height) = surface.size();
        let scene_rt = scheme
            .lease_render_target(
                width.max(1),
                height.max(1),
                surface.format(),
                Some(DepthFormat::Depth24Plus),
            )
            .context("Failed to lease offscreen scene render target")?;
        self.scene_rt = Some(scene_rt);

        let scene_rt = self.scene_rt.as_ref().context("scene render target lease missing")?;
        let present = Self::record_scheme(
            &mut scheme,
            surface,
            self.static_pipeline.as_ref().context("static pipeline")?,
            self.sky_pipeline.as_ref().context("sky pipeline")?,
            self.sprite_pipeline.as_ref().context("sprite pipeline")?,
            &self.scene_buf,
            &self.light_buf,
            &self.sampler,
            level,
            scene_rt,
        )?;
        self.present = Some(present);
        self.scheme = Some(scheme);
        Ok(())
    }

    /// Called once the window exists. Creates the swapchain and compiles pipelines.
    pub fn init_surface(&mut self, window: &Window) -> Result<()> {
        let context = self
            .device
            .create_context()
            .context("Failed to create submission context")?;
        let surface = SurfaceExchange::new(&context, window, SurfaceConfig::default())
            .context("Failed to create surface exchange")?;
        let target_format = surface.format();

        // Register doom_common as a shader library so import doom_common resolves via the library system
        let shader_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("assets")
            .join("shaders");
        let doom_common_src = std::fs::read_to_string(shader_dir.join("doom_common.slang"))
            .context("Failed to read doom_common.slang")?;
        let doom_common_lib = ShaderLibrary::from_source("doom_common", &doom_common_src);
        self.device
            .register_library(doom_common_lib)
            .context("Failed to register doom_common shader library")?;

        let static_src = std::fs::read_to_string(shader_dir.join("doom_static.slang"))
            .context("Failed to read doom_static.slang")?;

        let sky_src = std::fs::read_to_string(shader_dir.join("doom_sky.slang"))
            .context("Failed to read doom_sky.slang")?;
        let sprite_src = std::fs::read_to_string(shader_dir.join("doom_sprite.slang"))
            .context("Failed to read doom_sprite.slang")?;

        let static_shader = ShaderModule::from_slang_with_options(
            &self.device,
            &static_src,
            &[],
            &[],
            Default::default(),
            &[SceneUniforms::LAYOUT_CHECK],
        )
        .context("Failed to compile doom_static shader")?;

        if goldy::layout_validation_enabled() {
            log::info!("SceneUniforms layout validated (GOLDY_VALIDATE_LAYOUTS=1)");
        }
        let sky_shader = ShaderModule::from_slang(&self.device, &sky_src)
            .context("Failed to compile doom_sky shader")?;
        let sprite_shader = ShaderModule::from_slang(&self.device, &sprite_src)
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

        self.context = Some(context);
        self.surface = Some(surface);
        self.scheme = Some(Scheme::new(
            self.context.as_ref().context("context not initialized")?,
        ));
        self.scene_rt = None;
        self.static_pipeline = Some(static_pipeline);
        self.sky_pipeline = Some(sky_pipeline);
        self.sprite_pipeline = Some(sprite_pipeline);

        log::info!(
            "Renderer: surface + pipelines initialized (format: {:?})",
            target_format
        );
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
        // Single record buffer for all level geometry — one GPU allocation, six parcels.
        let geometry = self.retained_pool.acquire_record([
            ordinal(Init::data(&mesh.static_vertices)),
            ordinal(Init::data(&mesh.static_indices)),
            ordinal(Init::data(&mesh.sky_vertices)),
            ordinal(Init::data(&mesh.sky_indices)),
            ordinal(if mesh.decor_vertices.is_empty() {
                Init::data(&[SpriteVertex::zeroed()])
            } else {
                Init::data(&mesh.decor_vertices)
            }),
            ordinal(if mesh.decor_indices.is_empty() {
                Init::data(&[0u32])
            } else {
                Init::data(&mesh.decor_indices)
            }),
        ])?;
        let static_vb = 0;
        let static_ib = 1;
        let sky_vb = 2;
        let sky_ib = 3;
        let decor_vb = 4;
        let decor_ib = 5;

        // Wall atlas: u16 raw bytes → Rg8Unorm (R=palette_idx, G=transparency).
        let (wall_w, wall_h) = (wall_atlas.1[0] as u32, wall_atlas.1[1] as u32);
        let wall_tex = self
            .retained_pool
            .acquire_texture(
                wall_w,
                wall_h,
                TextureFormat::Rg8Unorm,
                TextureKind::Interpolated,
                TextureFlags::COPY_DST,
                Some(bytemuck::cast_slice::<u16, u8>(&wall_atlas.0)),
            )
            .context("wall atlas texture")?;

        // Flat atlas: u8 raw → R8Unorm (palette index per pixel).
        let (flat_w, flat_h) = (flat_atlas.1[0] as u32, flat_atlas.1[1] as u32);
        let flat_tex = self
            .retained_pool
            .acquire_texture(
                flat_w,
                flat_h,
                TextureFormat::R8Unorm,
                TextureKind::Interpolated,
                TextureFlags::COPY_DST,
                Some(&flat_atlas.0),
            )
            .context("flat atlas texture")?;

        // Palette: RGB triplets → RGBA8. Dimensions: 256 x num_colormaps.
        let num_colors = palette.len() / 3;
        let palette_h = (num_colors / 256).max(1) as u32;
        let palette_rgba = palette_to_rgba8(&palette);
        let palette_tex = self
            .retained_pool
            .acquire_texture(
                256,
                palette_h,
                TextureFormat::Rgba8Unorm,
                TextureKind::Interpolated,
                TextureFlags::COPY_DST,
                Some(&palette_rgba),
            )
            .context("palette texture")?;

        // Sky texture: u8 raw → R8Unorm (palette index per pixel). Fallback 1x1 black.
        let sky_tex = match sky_texture {
            Some((data, [w, h])) => self
                .retained_pool
                .acquire_texture(
                    w as u32,
                    h as u32,
                    TextureFormat::R8Unorm,
                    TextureKind::Interpolated,
                    TextureFlags::COPY_DST,
                    Some(&data),
                )
                .context("sky texture")?,
            None => self
                .retained_pool
                .acquire_texture(
                    1,
                    1,
                    TextureFormat::R8Unorm,
                    TextureKind::Interpolated,
                    TextureFlags::COPY_DST,
                    Some(&[0u8]),
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
            wall_w,
            wall_h,
            flat_w,
            flat_h,
            palette_h,
        );

        self.level = Some(LevelGpuResources {
            geometry,
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

        self.rerecord_scheme()?;

        Ok(())
    }

    pub fn render_frame(
        &mut self,
        view: Mat4,
        proj: Mat4,
        time: f32,
        light_levels: &[f32],
    ) -> Result<()> {
        if self.surface.is_none() {
            return Ok(());
        }
        let level = match &self.level {
            Some(l) => l,
            None => return Ok(()),
        };

        let uniforms = SceneUniforms {
            projection: proj.to_cols_array_2d(),
            modelview: view.to_cols_array_2d(),
            atlas_size: level.wall_atlas_size,
            flat_atlas_size: level.flat_atlas_size,
            time,
            tiled_band_size: level.tiled_band_size,
        };

        let ctx = self.context.as_ref().context("context not initialized")?;
        upload_parcel(ctx, &*self.scene_buf, 0, bytemuck::bytes_of(&uniforms))?;
        if light_levels.len() >= 256 {
            upload_parcel(
                ctx,
                &self.light_buf,
                0,
                bytemuck::cast_slice(&light_levels[..256]),
            )?;
        }

        let scheme = self.scheme.as_mut().context("scheme not initialized")?;
        let present_tx = self.present.as_ref().context("present transaction not recorded")?;
        let mut submission = scheme.submit()?;
        present_tx.claim(&mut submission)?.consume()?;

        Ok(())
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        if let Some(surface) = &self.surface {
            if let Err(e) = surface.resize(width, height) {
                log::error!("Failed to resize surface: {e}");
                return;
            }
            if let Err(e) = self.rerecord_scheme() {
                log::error!("Failed to rerecord scheme after resize: {e}");
            }
        }
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
