use super::level_builder::LevelMeshData;
use super::vertex::{SkyVertex, SpriteVertex, StaticVertex};
use anyhow::{bail, Context, Result};
use bytemuck::{Pod, Zeroable};
use clap::ValueEnum;
use glam::Mat4;
use goldy::types::{
    AddressMode, BackendType, BufferFlags, BufferKind, DepthFormat, DepthStencilState, FilterMode,
    IndexFormat, SamplerDesc, SurfaceConfig, TargetLoad, TextureFlags, TextureFormat, TextureKind,
};
use goldy::{
    ordinal, AccelInstance, AccelerationStructure, Buffer, ComputePipeline, Context as GpuContext,
    Device, Init, Instance, Lease, LeaseRenderTarget, MemoryExchange, MeshPipeline,
    MeshPipelineDesc, NodeAccess, Parcel, RenderPipeline, RenderPipelineDesc, RetainedPool,
    Sampler, Scheme, ShaderLibrary, ShaderModule, ShaderResourceSlot, StructuredBufferElement,
    SurfaceExchange, Texture, Transaction,
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

fn register_doom_common(device: &Device, shader_dir: &std::path::Path) -> Result<()> {
    let doom_common_src = std::fs::read_to_string(shader_dir.join("doom_common.slang"))
        .context("Failed to read doom_common.slang")?;
    let doom_common_lib = ShaderLibrary::from_source_with_gpu_types(
        "doom_common",
        &doom_common_src,
        &[SceneUniforms::GPU_TYPE],
    )
    .context("Failed to build doom_common shader library")?;
    device
        .register_library(doom_common_lib)
        .context("Failed to register doom_common shader library")?;
    ShaderModule::validate_existing_gpu_types(
        device,
        r#"
import doom_common;
import goldy_exp;

[goldy_compute]
[numthreads(1, 1, 1)]
void cs_main(SceneUniforms scene, ThreadId id) {
    SceneUniforms s = get_scene(scene);
}
"#,
        &[SceneUniforms::GPU_TYPE],
    )
    .context("SceneUniforms generated layout validation failed")?;
    Ok(())
}

/// Scene/frame uniforms. Slang declaration is generated from this Rust type.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable, goldy::GpuType)]
pub struct SceneUniforms {
    pub projection: [[f32; 4]; 4],
    pub modelview: [[f32; 4]; 4],
    pub atlas_size: [f32; 2],
    pub flat_atlas_size: [f32; 2],
    pub time: f32,
    pub tiled_band_size: f32,
}

impl StructuredBufferElement for SceneUniforms {}

/// Mutually exclusive GPU path selected by `--render`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
pub enum RenderMode {
    /// Classic VS/FS sky + static + sprites.
    #[default]
    Raster,
    /// Compute primary rays (BLAS/TLAS + RayQuery). Hard-fails without `ray_query`.
    #[value(name = "ray-query")]
    RayQuery,
    /// Sky/sprites VS/FS; static via mesh shaders. Hard-fails without `mesh_shaders`.
    Mesh,
}

/// Per-frame uniforms for `--render ray-query`. Slang declaration is generated from this type.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable, goldy::GpuType)]
struct RtUniforms {
    width: u32,
    height: u32,
    time: f32,
    tiled_band_size: f32,
    camera_pos: [f32; 3],
    #[gpu(padding)]
    _pad2: f32,
    inv_view_proj: [[f32; 4]; 4],
    atlas_size: [f32; 2],
    flat_atlas_size: [f32; 2],
    sky_rot: [f32; 2],
    #[gpu(padding)]
    _pad3: [f32; 2],
}

impl StructuredBufferElement for RtUniforms {}

struct LevelGpuResources {
    /// Mosaic sky/sprite/(raster-static) geometry. `None` in ray-query mode.
    geometry: Option<Buffer>,
    static_vb: usize,
    static_ib: usize,
    static_index_count: u32,

    sky_vb: usize,
    sky_ib: usize,
    sky_index_count: u32,

    decor_vb: usize,
    decor_ib: usize,
    decor_index_count: u32,

    wall_atlas: Option<Texture>,
    flat_atlas: Option<Texture>,
    palette: Option<Texture>,
    sky_texture: Option<Texture>,

    wall_atlas_size: [f32; 2],
    flat_atlas_size: [f32; 2],
    tiled_band_size: f32,

    accel_positions: Option<Buffer>,
    accel_indices: Option<Buffer>,
    accel_vertex_count: u32,
    accel_index_count: u32,
    blas: Option<AccelerationStructure>,
    tlas: Option<AccelerationStructure>,

    mesh_verts: Option<Buffer>,
    mesh_indices: Option<Buffer>,
    mesh_tri_count: u32,
}

pub struct Renderer {
    mode: RenderMode,
    instance: Instance,
    device: Arc<Device>,
    context: Option<GpuContext>,

    surface: Option<SurfaceExchange>,
    present: Option<Transaction>,
    static_pipeline: Option<RenderPipeline>,
    sky_pipeline: Option<RenderPipeline>,
    sprite_pipeline: Option<RenderPipeline>,
    mesh_pipeline: Option<MeshPipeline>,
    ray_pipeline: Option<ComputePipeline>,

    sampler: Sampler,
    scene_buf: Buffer,
    light_buf: Buffer,
    rt_buf: Option<Buffer>,

    level: Option<LevelGpuResources>,
    retained_pool: RetainedPool,

    scene_rt: Option<Lease<LeaseRenderTarget>>,
    scheme: Option<Scheme>,
}

impl Renderer {
    pub fn new(mode: RenderMode) -> Result<Self> {
        let instance = Instance::new().context("Failed to create goldy instance")?;
        let device = Arc::new(
            instance
                .request_adapter(&goldy::RequestAdapterOptions::default())
                .context("Failed to request goldy adapter")?
                .request_device(&goldy::DeviceDescriptor::default())
                .context("Failed to create goldy device")?,
        );

        match mode {
            RenderMode::Raster => {}
            RenderMode::RayQuery => {
                if !device.capabilities().ray_query {
                    bail!(
                        "--render ray-query requires DeviceCapabilities::ray_query (false on this adapter)"
                    );
                }
                if device.backend_type() == BackendType::WebGpu {
                    bail!(
                        "--render ray-query is not supported on WebGPU (no TraceRayInline in Slang WGSL)"
                    );
                }
            }
            RenderMode::Mesh => {
                if !device.capabilities().mesh_shaders {
                    bail!(
                        "--render mesh requires DeviceCapabilities::mesh_shaders (false on this adapter)"
                    );
                }
            }
        }

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

        let scene_uniforms = SceneUniforms::zeroed();
        let scene_bytes = bytemuck::bytes_of(&scene_uniforms);
        let scene_buf = retained_pool
            .acquire_buffer(
                scene_bytes.len() as u64,
                BufferKind::Broadcast,
                Some(std::mem::size_of::<SceneUniforms>() as u32),
                BufferFlags::empty(),
                Some(scene_bytes),
            )
            .context("Failed to create scene uniform buffer")?;

        let initial_lights: Vec<f32> = vec![1.0; 256];
        let light_buf = retained_pool
            .acquire_buffer(
                (initial_lights.len() * std::mem::size_of::<f32>()) as u64,
                BufferKind::Scattered,
                Some(std::mem::size_of::<f32>() as u32),
                BufferFlags::empty(),
                Some(bytemuck::cast_slice(&initial_lights)),
            )
            .context("Failed to create light buffer")?;

        let rt_buf = if mode == RenderMode::RayQuery {
            let zero = RtUniforms {
                width: 1,
                height: 1,
                time: 0.0,
                tiled_band_size: 0.0,
                camera_pos: [0.0; 3],
                _pad2: 0.0,
                inv_view_proj: Mat4::IDENTITY.to_cols_array_2d(),
                atlas_size: [1.0, 1.0],
                flat_atlas_size: [1.0, 1.0],
                sky_rot: [0.0, 0.0],
                _pad3: [0.0, 0.0],
            };
            Some(
                retained_pool
                    .acquire_buffer_with_data_and_flags(
                        &[zero],
                        BufferKind::Scattered,
                        BufferFlags::empty(),
                    )
                    .context("Failed to create ray-query uniform buffer")?,
            )
        } else {
            None
        };

        Ok(Self {
            mode,
            instance,
            device,
            context: None,
            surface: None,
            present: None,
            static_pipeline: None,
            sky_pipeline: None,
            sprite_pipeline: None,
            mesh_pipeline: None,
            ray_pipeline: None,
            sampler,
            scene_buf,
            light_buf,
            rt_buf,
            level: None,
            retained_pool,
            scene_rt: None,
            scheme: None,
        })
    }

    fn record_raster_scheme(
        scheme: &mut Scheme,
        surface: &SurfaceExchange,
        static_pipeline: Option<&RenderPipeline>,
        mesh_pipeline: Option<&MeshPipeline>,
        sky_pipeline: &RenderPipeline,
        sprite_pipeline: &RenderPipeline,
        scene_buf: &Buffer,
        light_buf: &Buffer,
        sampler: &Sampler,
        level: &LevelGpuResources,
        scene_rt: &Lease<LeaseRenderTarget>,
        mesh_mode: bool,
    ) -> Result<Transaction> {
        let geometry = level.geometry.as_ref().context("geometry mosaic missing")?;
        let wall = level.wall_atlas.as_ref().context("wall atlas")?;
        let flat = level.flat_atlas.as_ref().context("flat atlas")?;
        let palette = level.palette.as_ref().context("palette")?;
        let sky_tex = level.sky_texture.as_ref().context("sky texture")?;

        let mut slots = vec![
            ShaderResourceSlot::Parcel {
                parcel: &*scene_buf,
                access: NodeAccess::Read,
            },
            ShaderResourceSlot::Parcel {
                parcel: &*light_buf,
                access: NodeAccess::Read,
            },
            ShaderResourceSlot::Parcel {
                parcel: &*wall,
                access: NodeAccess::Read,
            },
            ShaderResourceSlot::Parcel {
                parcel: &*flat,
                access: NodeAccess::Read,
            },
            ShaderResourceSlot::Parcel {
                parcel: &*palette,
                access: NodeAccess::Read,
            },
            ShaderResourceSlot::Parcel {
                parcel: &*sky_tex,
                access: NodeAccess::Read,
            },
            ShaderResourceSlot::Sampler(sampler),
        ];

        if mesh_mode {
            let mv = level.mesh_verts.as_ref().context("mesh verts")?;
            let mi = level.mesh_indices.as_ref().context("mesh indices")?;
            slots.push(ShaderResourceSlot::Parcel {
                parcel: &*mv,
                access: NodeAccess::Read,
            });
            slots.push(ShaderResourceSlot::Parcel {
                parcel: &*mi,
                access: NodeAccess::Read,
            });
        }

        let mut pass = scheme.render_pass("doom", scene_rt, TargetLoad::Clear(goldy::Color::BLACK));
        pass.with_shader_resources(&slots);
        pass.with_buffer_dependency(geometry, NodeAccess::Read);
        pass.clear_depth(1.0);

        if level.sky_index_count > 0 {
            pass.set_pipeline(sky_pipeline);
            pass.set_vertex_buffer(0, &geometry[level.sky_vb]);
            pass.set_index_buffer(&geometry[level.sky_ib], IndexFormat::Uint32);
            pass.draw_indexed(0..level.sky_index_count, 0, 0..1);
        }

        if mesh_mode {
            if level.mesh_tri_count > 0 {
                let mesh_pipeline = mesh_pipeline.context("mesh pipeline")?;
                let groups = level.mesh_tri_count.div_ceil(64);
                pass.set_mesh_pipeline(mesh_pipeline);
                pass.dispatch_mesh(groups, 1, 1);
            }
        } else if level.static_index_count > 0 {
            let static_pipeline = static_pipeline.context("static pipeline")?;
            pass.set_pipeline(static_pipeline);
            pass.set_vertex_buffer(0, &geometry[level.static_vb]);
            pass.set_index_buffer(&geometry[level.static_ib], IndexFormat::Uint32);
            pass.draw_indexed(0..level.static_index_count, 0, 0..1);
        }

        if level.decor_index_count > 0 {
            pass.set_pipeline(sprite_pipeline);
            pass.set_vertex_buffer(0, &geometry[level.decor_vb]);
            pass.set_index_buffer(&geometry[level.decor_ib], IndexFormat::Uint32);
            pass.draw_indexed(0..level.decor_index_count, 0, 0..1);
        }

        pass.finish();
        surface
            .bind_render_target(scheme, scene_rt)
            .map_err(Into::into)
    }

    fn build_accel_once(&self) -> Result<()> {
        let ctx = self.context.as_ref().context("context not initialized")?;
        let level = self.level.as_ref().context("level not loaded")?;
        let positions = level.accel_positions.as_ref().context("accel positions")?;
        let blas = level.blas.as_ref().context("BLAS")?;
        let tlas = level.tlas.as_ref().context("TLAS")?;

        let indices = level.accel_indices.as_ref();
        let index_arg = indices.map(|ib| (ib.whole(), level.accel_index_count));
        let mut scheme = Scheme::new(ctx);
        // float3 positions only — stride-60 StaticVertex AS previously device-removed.
        let vert_stride = 12u32;
        scheme.build_blas(
            blas,
            positions.whole(),
            level.accel_vertex_count,
            vert_stride,
            index_arg,
        )?;
        let identity = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];
        scheme.build_tlas(
            tlas,
            &[AccelInstance {
                blas,
                transform: identity,
                mask: 0xFF,
                custom_index: 0,
            }],
        )?;
        let submission = scheme.submit().context("AS once submit")?;
        submission.wait_until_settled().context("AS once wait")?;
        Ok(())
    }

    fn record_ray_query_scheme(
        scheme: &mut Scheme,
        surface: &SurfaceExchange,
        pipeline: &ComputePipeline,
        rt_buf: &Buffer,
        light_buf: &Buffer,
        sampler: &Sampler,
        level: &LevelGpuResources,
        width: u32,
        height: u32,
    ) -> Result<Transaction> {
        // AS is built once in load_level; per-frame scheme only traces + shades.
        let tlas = level.tlas.as_ref().context("TLAS")?;
        // Shading attrs live in mesh_verts (StaticVertex); AS uses float3 accel_positions.
        let verts = level.mesh_verts.as_ref().context("attr verts")?;
        let indices = level.accel_indices.as_ref().context("accel indices")?;
        let wall = level.wall_atlas.as_ref().context("wall atlas")?;
        let flat = level.flat_atlas.as_ref().context("flat atlas")?;
        let palette = level.palette.as_ref().context("palette")?;
        let sky_tex = level.sky_texture.as_ref().context("sky texture")?;
        let (lease, present_tx) = surface.bind_destination(scheme)?;
        scheme
            .node("rays", pipeline)
            .with_parcel(rt_buf, NodeAccess::Read)
            .with_parcel(tlas, NodeAccess::Read)
            .with_parcel(verts, NodeAccess::Read)
            .with_parcel(indices, NodeAccess::Read)
            .with_parcel(light_buf, NodeAccess::Read)
            .with_parcel(wall, NodeAccess::Read)
            .with_parcel(flat, NodeAccess::Read)
            .with_parcel(palette, NodeAccess::Read)
            .with_parcel(sky_tex, NodeAccess::Read)
            .with_parcel(sampler, NodeAccess::Read)
            .with_present(&lease)
            .dispatch(width.div_ceil(8), height.div_ceil(8), 1);
        Ok(present_tx)
    }

    fn rerecord_scheme(&mut self) -> Result<()> {
        let surface = self.surface.as_ref().context("surface not initialized")?;
        let context = self.context.as_ref().context("context not initialized")?;
        let level = self.level.as_ref().context("level not loaded")?;
        let (width, height) = surface.size();
        let mut scheme = Scheme::new(context);

        let present = match self.mode {
            RenderMode::RayQuery => {
                self.scene_rt = None;
                Self::record_ray_query_scheme(
                    &mut scheme,
                    surface,
                    self.ray_pipeline.as_ref().context("ray pipeline")?,
                    self.rt_buf.as_ref().context("rt uniforms")?,
                    &self.light_buf,
                    &self.sampler,
                    level,
                    width.max(1),
                    height.max(1),
                )?
            }
            RenderMode::Raster | RenderMode::Mesh => {
                let scene_rt = scheme
                    .lease_render_target(
                        width.max(1),
                        height.max(1),
                        surface.format(),
                        Some(DepthFormat::Depth24Plus),
                    )
                    .context("Failed to lease offscreen scene render target")?;
                self.scene_rt = Some(scene_rt);
                let scene_rt = self
                    .scene_rt
                    .as_ref()
                    .context("scene render target lease missing")?;
                Self::record_raster_scheme(
                    &mut scheme,
                    surface,
                    self.static_pipeline.as_ref(),
                    self.mesh_pipeline.as_ref(),
                    self.sky_pipeline.as_ref().context("sky pipeline")?,
                    self.sprite_pipeline.as_ref().context("sprite pipeline")?,
                    &self.scene_buf,
                    &self.light_buf,
                    &self.sampler,
                    level,
                    scene_rt,
                    self.mode == RenderMode::Mesh,
                )?
            }
        };

        self.present = Some(present);
        self.scheme = Some(scheme);
        Ok(())
    }

    pub fn init_surface(&mut self, window: &Window) -> Result<()> {
        let context = self
            .device
            .create_context()
            .context("Failed to create submission context")?;
        let surface = SurfaceExchange::new(&context, window, SurfaceConfig::default())
            .context("Failed to create surface exchange")?;
        let target_format = surface.format();

        let shader_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("assets")
            .join("shaders");

        match self.mode {
            RenderMode::RayQuery => {
                register_doom_common(&self.device, &shader_dir)?;

                let ray_src = std::fs::read_to_string(shader_dir.join("doom_ray_query.slang"))
                    .context("Failed to read doom_ray_query.slang")?;
                let ray_shader = ShaderModule::from_slang_with_gpu_types(
                    &self.device,
                    &ray_src,
                    &[StaticVertex::GPU_TYPE, RtUniforms::GPU_TYPE],
                )
                .context("Failed to compile doom_ray_query shader")?;
                self.ray_pipeline = Some(
                    ComputePipeline::new(&self.device, &ray_shader)
                        .context("Failed to create ray-query compute pipeline")?,
                );
                self.static_pipeline = None;
                self.sky_pipeline = None;
                self.sprite_pipeline = None;
                self.mesh_pipeline = None;
            }
            RenderMode::Raster | RenderMode::Mesh => {
                register_doom_common(&self.device, &shader_dir)?;

                let sky_src = std::fs::read_to_string(shader_dir.join("doom_sky.slang"))
                    .context("Failed to read doom_sky.slang")?;
                let sprite_src = std::fs::read_to_string(shader_dir.join("doom_sprite.slang"))
                    .context("Failed to read doom_sprite.slang")?;
                let sky_shader = ShaderModule::from_slang(&self.device, &sky_src)
                    .context("Failed to compile doom_sky shader")?;
                let sprite_shader = ShaderModule::from_slang(&self.device, &sprite_src)
                    .context("Failed to compile doom_sprite shader")?;

                let sky_depth = Some(DepthStencilState {
                    depth_write_enabled: false,
                    ..DepthStencilState::default()
                });
                let sprite_depth = Some(DepthStencilState {
                    depth_write_enabled: false,
                    ..DepthStencilState::default()
                });

                self.sky_pipeline = Some(
                    RenderPipeline::new(
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
                    .context("Failed to create sky pipeline")?,
                );
                self.sprite_pipeline = Some(
                    RenderPipeline::new(
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
                    .context("Failed to create sprite pipeline")?,
                );

                if self.mode == RenderMode::Mesh {
                    let mesh_src =
                        std::fs::read_to_string(shader_dir.join("doom_static_mesh.slang"))
                            .context("Failed to read doom_static_mesh.slang")?;
                    let mesh_shader = ShaderModule::from_slang_with_gpu_types(
                        &self.device,
                        &mesh_src,
                        &[StaticVertex::GPU_TYPE],
                    )
                    .context("Failed to compile doom_static_mesh shader")?;
                    self.mesh_pipeline = Some(
                        MeshPipeline::new(
                            &self.device,
                            &MeshPipelineDesc {
                                mesh: &mesh_shader,
                                fragment: &mesh_shader,
                                amplification: None,
                                target_format,
                                depth_stencil: Some(DepthStencilState::default()),
                            },
                        )
                        .context("Failed to create static mesh pipeline")?,
                    );
                    self.static_pipeline = None;
                } else {
                    let static_src = std::fs::read_to_string(shader_dir.join("doom_static.slang"))
                        .context("Failed to read doom_static.slang")?;
                    let static_shader = ShaderModule::from_slang(&self.device, &static_src)
                        .context("Failed to compile doom_static shader")?;
                    self.static_pipeline = Some(
                        RenderPipeline::new(
                            &self.device,
                            &static_shader,
                            &static_shader,
                            &RenderPipelineDesc {
                                vertex_layout: StaticVertex::layout(),
                                target_format,
                                depth_stencil: Some(DepthStencilState::default()),
                                ..Default::default()
                            },
                        )
                        .context("Failed to create static pipeline")?,
                    );
                    self.mesh_pipeline = None;
                }
                self.ray_pipeline = None;
            }
        }

        self.context = Some(context);
        self.surface = Some(surface);
        self.scheme = Some(Scheme::new(
            self.context.as_ref().context("context not initialized")?,
        ));
        self.scene_rt = None;

        log::info!(
            "Renderer: surface + pipelines initialized (mode={:?}, format={:?})",
            self.mode,
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
        let static_tri_count = (mesh.static_indices.len() / 3) as u32;

        let (
            geometry,
            static_vb,
            static_ib,
            sky_vb,
            sky_ib,
            decor_vb,
            decor_ib,
            accel_positions,
            accel_indices,
            accel_vertex_count,
            accel_index_count,
            blas,
            tlas,
            mesh_verts,
            mesh_indices,
            mesh_tri_count,
            wall_atlas_tex,
            flat_atlas_tex,
            palette_tex,
            sky_tex,
            wall_atlas_size,
            flat_atlas_size,
        ) = match self.mode {
            RenderMode::RayQuery => {
                anyhow::ensure!(
                    !mesh.static_indices.is_empty(),
                    "ray-query mode requires static level geometry"
                );
                let positions: Vec<[f32; 3]> = mesh.static_vertices.iter().map(|v| v.pos).collect();
                let accel_positions = self
                    .retained_pool
                    .acquire_buffer_with_data_and_flags(
                        &positions,
                        BufferKind::Scattered,
                        BufferFlags::ACCEL_INPUT,
                    )
                    .context("accel float3 position buffer")?;
                let accel_indices = self
                    .retained_pool
                    .acquire_buffer_with_data_and_flags(
                        &mesh.static_indices,
                        BufferKind::Scattered,
                        BufferFlags::ACCEL_INPUT,
                    )
                    .context("accel index buffer")?;
                let mesh_verts = self
                    .retained_pool
                    .acquire_buffer_with_data_and_flags(
                        &mesh.static_vertices,
                        BufferKind::Scattered,
                        BufferFlags::empty(),
                    )
                    .context("StaticVertex attr buffer")?;
                let vertex_count = positions.len() as u32;
                let index_count = mesh.static_indices.len() as u32;
                let blas = AccelerationStructure::blas_triangles(
                    &self.device,
                    static_tri_count.max(1),
                    vertex_count.max(3),
                    12,
                )
                .context("create BLAS")?;
                let tlas = AccelerationStructure::tlas(&self.device, 1).context("create TLAS")?;

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

                (
                    None,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    Some(accel_positions),
                    Some(accel_indices),
                    vertex_count,
                    index_count,
                    Some(blas),
                    Some(tlas),
                    Some(mesh_verts),
                    None,
                    0,
                    Some(wall_tex),
                    Some(flat_tex),
                    Some(palette_tex),
                    Some(sky_tex),
                    [wall_w as f32, wall_h as f32],
                    [flat_w as f32, flat_h as f32],
                )
            }
            RenderMode::Mesh => {
                let static_placeholder = [StaticVertex::zeroed()];
                let static_ib_placeholder = [0u32];
                let decor_v_placeholder = [SpriteVertex::zeroed()];
                let decor_i_placeholder = [0u32];
                let geometry = self.retained_pool.acquire_record([
                    ordinal(Init::data(&static_placeholder)),
                    ordinal(Init::data(&static_ib_placeholder)),
                    ordinal(Init::data(&mesh.sky_vertices)),
                    ordinal(Init::data(&mesh.sky_indices)),
                    ordinal(Init::data(if mesh.decor_vertices.is_empty() {
                        &decor_v_placeholder[..]
                    } else {
                        &mesh.decor_vertices[..]
                    })),
                    ordinal(Init::data(if mesh.decor_indices.is_empty() {
                        &decor_i_placeholder[..]
                    } else {
                        &mesh.decor_indices[..]
                    })),
                ])?;

                let mut padded = mesh.static_indices.clone();
                let tri_count = padded.len() / 3;
                let padded_tris = tri_count.next_multiple_of(64);
                if padded_tris > tri_count {
                    let last = if tri_count == 0 {
                        [0u32, 0, 0]
                    } else {
                        [
                            padded[(tri_count - 1) * 3],
                            padded[(tri_count - 1) * 3 + 1],
                            padded[(tri_count - 1) * 3 + 2],
                        ]
                    };
                    for _ in tri_count..padded_tris {
                        padded.extend_from_slice(&last);
                    }
                }
                let verts = if mesh.static_vertices.is_empty() {
                    vec![StaticVertex::zeroed()]
                } else {
                    mesh.static_vertices.clone()
                };
                let mesh_verts = self
                    .retained_pool
                    .acquire_buffer_with_data_and_flags(
                        &verts,
                        BufferKind::Scattered,
                        BufferFlags::empty(),
                    )
                    .context("mesh vertex buffer")?;
                let mesh_indices = self
                    .retained_pool
                    .acquire_buffer_with_data_and_flags(
                        &padded,
                        BufferKind::Scattered,
                        BufferFlags::empty(),
                    )
                    .context("mesh index buffer")?;

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

                (
                    Some(geometry),
                    0,
                    1,
                    2,
                    3,
                    4,
                    5,
                    None,
                    None,
                    0,
                    0,
                    None,
                    None,
                    Some(mesh_verts),
                    Some(mesh_indices),
                    padded_tris as u32,
                    Some(wall_tex),
                    Some(flat_tex),
                    Some(palette_tex),
                    Some(sky_tex),
                    [wall_w as f32, wall_h as f32],
                    [flat_w as f32, flat_h as f32],
                )
            }
            RenderMode::Raster => {
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

                (
                    Some(geometry),
                    0,
                    1,
                    2,
                    3,
                    4,
                    5,
                    None,
                    None,
                    0,
                    0,
                    None,
                    None,
                    None,
                    None,
                    0,
                    Some(wall_tex),
                    Some(flat_tex),
                    Some(palette_tex),
                    Some(sky_tex),
                    [wall_w as f32, wall_h as f32],
                    [flat_w as f32, flat_h as f32],
                )
            }
        };

        log::info!(
            "Renderer: level loaded mode={:?} ({} static tris, {} sky tris, {} sprite tris)",
            self.mode,
            mesh.static_indices.len() / 3,
            mesh.sky_indices.len() / 3,
            mesh.decor_indices.len() / 3,
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
            wall_atlas: wall_atlas_tex,
            flat_atlas: flat_atlas_tex,
            palette: palette_tex,
            sky_texture: sky_tex,
            wall_atlas_size,
            flat_atlas_size,
            tiled_band_size,
            accel_positions,
            accel_indices,
            accel_vertex_count,
            accel_index_count,
            blas,
            tlas,
            mesh_verts,
            mesh_indices,
            mesh_tri_count,
        });

        if self.mode == RenderMode::RayQuery {
            self.build_accel_once()?;
        }
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

        let ctx = self.context.as_ref().context("context not initialized")?;

        match self.mode {
            RenderMode::RayQuery => {
                let surface = self.surface.as_ref().context("surface")?;
                let (width, height) = surface.size();
                let inv_view_proj = (proj * view).inverse();
                let camera_pos = view.inverse().col(3).truncate();
                let transform = proj * view;
                let forward = transform.col(2);
                let sky_rot = [
                    forward.x.atan2(forward.z),
                    if forward.w.abs() > 1e-8 {
                        forward.y / forward.w
                    } else {
                        0.0
                    },
                ];
                let uniforms = RtUniforms {
                    width: width.max(1),
                    height: height.max(1),
                    time,
                    tiled_band_size: level.tiled_band_size,
                    camera_pos: camera_pos.to_array(),
                    _pad2: 0.0,
                    inv_view_proj: inv_view_proj.to_cols_array_2d(),
                    atlas_size: level.wall_atlas_size,
                    flat_atlas_size: level.flat_atlas_size,
                    sky_rot,
                    _pad3: [0.0, 0.0],
                };
                let rt_buf = self.rt_buf.as_ref().context("rt uniforms")?;
                upload_parcel(ctx, &*rt_buf, 0, bytemuck::bytes_of(&uniforms))?;
                if light_levels.len() >= 256 {
                    upload_parcel(
                        ctx,
                        &*self.light_buf,
                        0,
                        bytemuck::cast_slice(&light_levels[..256]),
                    )?;
                }
            }
            RenderMode::Raster | RenderMode::Mesh => {
                let uniforms = SceneUniforms {
                    projection: proj.to_cols_array_2d(),
                    modelview: view.to_cols_array_2d(),
                    atlas_size: level.wall_atlas_size,
                    flat_atlas_size: level.flat_atlas_size,
                    time,
                    tiled_band_size: level.tiled_band_size,
                };
                upload_parcel(ctx, &*self.scene_buf, 0, bytemuck::bytes_of(&uniforms))?;
                if light_levels.len() >= 256 {
                    upload_parcel(
                        ctx,
                        &*self.light_buf,
                        0,
                        bytemuck::cast_slice(&light_levels[..256]),
                    )?;
                }
            }
        }

        let scheme = self.scheme.as_mut().context("scheme not initialized")?;
        let present_tx = self
            .present
            .as_ref()
            .context("present transaction not recorded")?;
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
