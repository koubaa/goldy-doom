#![allow(unused_imports, dead_code, mismatched_lifetime_syntaxes)]

mod player;
mod render;
mod wad;

use anyhow::{Context, Result};
use clap::Parser;
use log::info;
use player::Player;
use render::level_builder::LevelBuilder;
use render::renderer::Renderer;
use std::path::PathBuf;
use std::time::Instant;
use wad::tex::TextureDirectory;
use wad::{Archive, Level, LevelAnalysis, LevelWalker};
use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, DeviceId, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::PhysicalKey;
use winit::window::{Window, WindowId};

#[derive(Parser)]
#[command(name = "goldy-doom", about = "DOOM on goldy")]
struct Args {
    /// Path to the DOOM WAD file (e.g. doom.wad or doom2.wad)
    #[arg(short, long)]
    wad: PathBuf,

    /// Path to the metadata TOML file
    #[arg(short, long, default_value = "assets/meta/doom.toml")]
    meta: PathBuf,

    /// Level index to load (0-based)
    #[arg(short, long, default_value_t = 0)]
    level: usize,
}

fn main() -> Result<()> {
    env_logger::init();
    let args = Args::parse();

    info!("goldy-doom starting up");
    info!("WAD: {:?}", args.wad);
    info!("Meta: {:?}", args.meta);
    info!("Level: {}", args.level);

    let archive = Archive::open(&args.wad, &args.meta)
        .context("Failed to open WAD archive")?;

    info!("WAD has {} levels", archive.num_levels());
    anyhow::ensure!(args.level < archive.num_levels(), "Level {} out of range", args.level);

    let tex = TextureDirectory::from_archive(&archive)
        .context("Failed to read texture directory")?;
    let level = Level::from_archive(&archive, args.level)
        .context("Failed to load level")?;
    let analysis = LevelAnalysis::new(&level, archive.metadata());

    let level_name = archive.level_lump(args.level)?.name();
    info!("Walking level {}...", level_name);

    let wall_names: Vec<_> = level.sidedefs.iter()
        .flat_map(|s| [s.upper_texture, s.middle_texture, s.lower_texture])
        .filter(|n| !wad::util::is_untextured(*n))
        .collect();
    let flat_names: Vec<_> = level.sectors.iter()
        .flat_map(|s| [s.floor_texture, s.ceiling_texture])
        .filter(|n| !wad::util::is_sky_flat(*n))
        .collect();

    let (wall_atlas, wall_bounds) = tex.build_texture_atlas(wall_names.into_iter());
    let (flat_atlas, flat_bounds) = tex.build_flat_atlas(flat_names.into_iter());
    let decor_bounds = wall_bounds.clone();

    let mut builder = LevelBuilder::new(wall_bounds, flat_bounds, decor_bounds);
    {
        let mut walker = LevelWalker::new(&level, &analysis, &tex, archive.metadata(), &mut builder);
        walker.walk();
    }
    let mesh_data = builder.finish();
    let start_pos = mesh_data.start_pos;
    let start_yaw = mesh_data.start_yaw;

    info!("Start position: {:?}, yaw: {:.1}°", start_pos, start_yaw.to_degrees());

    let palette = tex.build_palette_texture(0, 0, tex.num_colormaps());

    // Sky texture from WAD metadata (texture_name + tiled_band_size per level)
    let (sky_texture, tiled_band_size) = archive
        .metadata()
        .sky_for(level_name)
        .and_then(|sky_meta| {
            tex.texture(sky_meta.texture_name).map(|img| {
                let size = img.size();
                let r8: Vec<u8> = img.pixels().iter().map(|p| (p & 0xFF) as u8).collect();
                info!("Sky texture: {} ({}x{}), tiled_band_size={}", sky_meta.texture_name, size[0], size[1], sky_meta.tiled_band_size);
                ((r8, size), sky_meta.tiled_band_size)
            })
        })
        .map(|((r8, size), tbs)| (Some((r8, size)), tbs))
        .unwrap_or_else(|| {
            info!("No sky texture found, using fallback (black)");
            (None, 0.125)
        });

    let event_loop = EventLoop::new().context("Failed to create event loop")?;
    let mut app = App {
        window: None,
        renderer: Renderer::new()?,
        player: Player::new(start_pos, start_yaw),
        last_frame: Instant::now(),
        time: 0.0,
        mesh_data: Some(mesh_data),
        palette_pixels: palette.pixels,
        wall_atlas: Some((wall_atlas.pixels, wall_atlas.size)),
        flat_atlas: Some((flat_atlas.pixels, flat_atlas.size)),
        sky_texture,
        tiled_band_size,
        light_buffer: vec![1.0f32; 256],
    };

    event_loop.run_app(&mut app).context("Event loop error")?;
    Ok(())
}

struct App {
    window: Option<Window>,
    renderer: Renderer,
    player: Player,
    last_frame: Instant,
    time: f32,
    mesh_data: Option<render::level_builder::LevelMeshData>,
    palette_pixels: Vec<u8>,
    wall_atlas: Option<(Vec<u16>, [usize; 2])>,
    flat_atlas: Option<(Vec<u8>, [usize; 2])>,
    sky_texture: Option<(Vec<u8>, [usize; 2])>,
    tiled_band_size: f32,
    light_buffer: Vec<f32>,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none() {
            let attrs = Window::default_attributes()
                .with_title("goldy-doom")
                .with_inner_size(winit::dpi::LogicalSize::new(1280, 720));
            match event_loop.create_window(attrs) {
                Ok(window) => {
                    info!("Window created: {}x{}", window.inner_size().width, window.inner_size().height);
                    window.set_cursor_visible(false);

                    if let Err(e) = self.renderer.init_surface(&window) {
                        log::error!("Failed to init GPU surface: {:#}", e);
                        event_loop.exit();
                        return;
                    }

                    if let Some(mesh) = self.mesh_data.take() {
                        let wall = self.wall_atlas.take().unwrap();
                        let flat = self.flat_atlas.take().unwrap();
                        let sky = self.sky_texture.take();
                        let tbs = self.tiled_band_size;
                        if let Err(e) = self.renderer.load_level(
                            mesh,
                            self.palette_pixels.clone(),
                            wall,
                            flat,
                            sky,
                            tbs,
                        ) {
                            log::error!("Failed to load level GPU resources: {:#}", e);
                            event_loop.exit();
                            return;
                        }
                    }

                    self.window = Some(window);
                }
                Err(e) => {
                    log::error!("Failed to create window: {}", e);
                    event_loop.exit();
                }
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(key) = event.physical_key {
                    if key == winit::keyboard::KeyCode::Escape {
                        event_loop.exit();
                        return;
                    }
                    self.player.on_key(key, event.state);
                }
            }
            WindowEvent::Resized(size) => {
                self.renderer.resize(size.width, size.height);
            }
            WindowEvent::RedrawRequested => {
                let now = Instant::now();
                let dt = (now - self.last_frame).as_secs_f32().min(0.1);
                self.last_frame = now;
                self.time += dt;

                self.player.update(dt);

                let window = self.window.as_ref().unwrap();
                let size = window.inner_size();
                let aspect = size.width as f32 / size.height.max(1) as f32;

                let view = self.player.view_matrix();
                let proj = self.player.projection_matrix(aspect);

                if let Err(e) = self.renderer.render_frame(view, proj, self.time, &self.light_buffer) {
                    log::error!("Render error: {}", e);
                }

                window.request_redraw();
            }
            _ => {}
        }
    }

    fn device_event(&mut self, _event_loop: &ActiveEventLoop, _device_id: DeviceId, event: DeviceEvent) {
        if let DeviceEvent::MouseMotion { delta } = event {
            self.player.on_mouse_motion(delta.0, delta.1);
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}
